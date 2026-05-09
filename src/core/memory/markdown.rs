//! `Markdown`-based memory backend.
//!
//! A lightweight, file-system-only alternative to the `PostgreSQL` backend.
//! Suitable for zero-dependency deployments where vector search and graph
//! activation are not required.
//!
//! ## File layout
//!
//! ```text
//! workspace/
//!   MEMORY.md              — curated long-term memory (core beliefs)
//!   memory/YYYY-MM-DD.md   — daily session logs (append-only)
//! ```
//!
//! ## Limitations
//!
//! - **Append-only**: forget requests are reported through `ForgetOutcome`, but
//!   this backend does not remove or tombstone projection entries.
//! - **No vector search**: recall uses keyword matching over projected text,
//!   with an explicit slot-prefix lookup path for typed recall; cosine
//!   similarity and `pgvector` HNSW are unavailable.
//! - **No graph activation**: PPR reranking and graph entity resolution are
//!   skipped; results are ordered by the local projection scoring path.
//! - **No integrity chain**: SHA-256 event hashing is `PostgreSQL`-only.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context;
use chrono::Local;
use tokio::fs;
use tokio::sync::Mutex;

use super::traits::{
    BeliefSlot, ForgetArtifact, ForgetArtifactCheck, ForgetMode, ForgetObservation, ForgetOutcome,
    ForgetRequirement, MemoryCategory, MemoryEntry, MemoryEvent, MemoryEventInput, MemoryEventType,
    MemoryGovernance, MemoryLayer, MemoryProvenance, MemoryReader, MemoryRecallEntry, MemorySource,
    MemoryWriter, PrivacyLevel, RecallQuery,
};
use crate::contracts::ids::EventId;
use crate::contracts::memory_error::MemoryError;
use crate::contracts::scores::{Confidence, Importance};

/// Markdown-based memory — plain files as source of truth
///
/// Layout:
///   workspace/MEMORY.md          — curated long-term memory (core)
///   workspace/memory/YYYY-MM-DD.md — daily logs (append-only)
pub struct MarkdownMemory {
    workspace_dir: PathBuf,
    append_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone)]
struct ParsedMarkdownLine {
    key: String,
    content: String,
    layer: Option<MemoryLayer>,
    provenance: Option<MemoryProvenance>,
}

#[derive(Debug, Clone)]
struct TimestampedParsedLine {
    timestamp: String,
    entry: ParsedMarkdownLine,
}

#[allow(
    clippy::unused_self,
    clippy::unused_async,
    clippy::trivially_copy_pass_by_ref
)]
impl MarkdownMemory {
    /// Create a new markdown memory backed by the given workspace
    /// directory.
    #[must_use]
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            append_lock: Arc::new(Mutex::new(())),
        }
    }

    fn memory_dir(&self) -> PathBuf {
        self.workspace_dir.join("memory")
    }

    fn core_path(&self) -> PathBuf {
        self.workspace_dir.join("MEMORY.md")
    }

    fn daily_path(&self) -> PathBuf {
        let date = Local::now().format("%Y-%m-%d").to_string();
        self.memory_dir().join(format!("{date}.md"))
    }

    async fn append_to_file(&self, path: &Path, content: &str) -> anyhow::Result<()> {
        let _guard = self.append_lock.lock().await;
        let memory_dir = self.memory_dir();
        let core_path = self.core_path();
        let path = path.to_path_buf();
        let content = content.to_string();

        tokio::task::spawn_blocking(move || {
            append_to_file_sync(&memory_dir, &core_path, &path, &content)
        })
        .await
        .context("join markdown memory append")?
    }
}

fn append_to_file_sync(
    memory_dir: &Path,
    core_path: &Path,
    path: &Path,
    content: &str,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(memory_dir).context("create memory directory")?;
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).context("create memory file parent directory")?;
    let _lock_file = acquire_append_lock(memory_dir)?;

    let existing = if path.exists() {
        std::fs::read_to_string(path).context("read existing memory file before append")?
    } else {
        String::new()
    };

    let updated = if existing.is_empty() {
        let header = if path == core_path {
            "# Long-Term Memory\n\n".to_string()
        } else {
            let date = Local::now().format("%Y-%m-%d").to_string();
            format!("# Daily Log — {date}\n\n")
        };
        format!("{header}{content}\n")
    } else {
        format!("{existing}\n{content}\n")
    };

    // Atomic write: temp file + rename prevents corruption on crash. The
    // append lock serializes the read/modify/rename sequence across processes.
    let tmp_path = parent.join(format!(
        ".tmp-{}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().as_hyphenated(),
        path.file_name().and_then(|n| n.to_str()).unwrap_or("mem")
    ));
    let mut tmp_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .context("create temporary memory file")?;
    if let Err(error) = tmp_file.write_all(updated.as_bytes()) {
        drop(tmp_file);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error).context("write temporary memory file");
    }
    if let Err(error) = tmp_file.sync_all() {
        drop(tmp_file);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error).context("sync temporary memory file");
    }
    drop(tmp_file);
    if let Err(error) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error).context("rename temporary memory file");
    }
    sync_directory(parent, "memory file parent directory")?;
    Ok(())
}

#[cfg(unix)]
fn acquire_append_lock(memory_dir: &Path) -> anyhow::Result<std::fs::File> {
    let lock_path = memory_dir.join(".append.lock");
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("open memory append lock {}", lock_path.display()))?;
    rustix::fs::flock(&lock_file, rustix::fs::FlockOperation::LockExclusive)
        .with_context(|| format!("lock memory append file {}", lock_path.display()))?;
    Ok(lock_file)
}

#[cfg(not(unix))]
fn acquire_append_lock(memory_dir: &Path) -> anyhow::Result<std::fs::File> {
    let lock_path = memory_dir.join(".append.lock");
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("open memory append lock {}", lock_path.display()))
}

#[cfg(unix)]
fn sync_directory(path: &Path, label: &str) -> anyhow::Result<()> {
    let dir = std::fs::File::open(path).with_context(|| format!("open {label} for sync"))?;
    dir.sync_all().with_context(|| format!("sync {label}"))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path, _label: &str) -> anyhow::Result<()> {
    Ok(())
}

#[allow(
    clippy::unused_self,
    clippy::unused_async,
    clippy::trivially_copy_pass_by_ref
)]
impl MarkdownMemory {
    fn parse_entries_from_file(
        path: &Path,
        content: &str,
        category: &MemoryCategory,
    ) -> Vec<MemoryEntry> {
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        content
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && !trimmed.starts_with('#')
            })
            .enumerate()
            .filter_map(|(i, line)| Self::parse_markdown_entry_line(line).map(|entry| (i, entry)))
            .map(|(i, entry)| MemoryEntry {
                // Layer-aware prior score improves recall ordering even before keyword match.
                score: entry.layer.as_ref().map(|layer| match layer {
                    MemoryLayer::Identity => 0.95,
                    MemoryLayer::Procedural => 0.85,
                    MemoryLayer::Semantic => 0.75,
                    MemoryLayer::Episodic => 0.65,
                    MemoryLayer::Working => 0.55,
                }),
                id: format!("{filename}:{i}"),
                key: entry.key,
                content: entry.content,
                category: category.clone(),
                timestamp: filename.to_string(),
                session_id: None,
                source: entry
                    .provenance
                    .map(|p| p.source_class)
                    .or_else(|| entry.layer.as_ref().map(|_| MemorySource::System)),
                layer: entry.layer,
            })
            .collect()
    }

    fn encode_tag_value(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            match ch {
                '%' => out.push_str("%25"),
                ';' => out.push_str("%3B"),
                '=' => out.push_str("%3D"),
                '&' => out.push_str("%26"),
                '\\' => out.push_str("%5C"),
                '[' => out.push_str("%5B"),
                ']' => out.push_str("%5D"),
                _ => out.push(ch),
            }
        }
        out
    }

    fn decode_tag_value(value: &str) -> String {
        let mut decoded = String::new();
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                decoded.push(ch);
                continue;
            }

            // Accumulate consecutive percent-encoded bytes to handle
            // multi-byte UTF-8 sequences (e.g. %C3%A9 → é).
            let mut byte_buf = Vec::new();
            let Some(a) = chars.next() else {
                tracing::warn!(value, "truncated percent-encoding in tag value");
                decoded.push('%');
                break;
            };
            let Some(b) = chars.next() else {
                tracing::warn!(value, "truncated percent-encoding in tag value");
                decoded.push('%');
                decoded.push(a);
                break;
            };
            let hex = format!("{a}{b}");
            let Ok(byte) = u8::from_str_radix(&hex, 16) else {
                tracing::warn!(hex, value, "invalid hex pair in tag percent-encoding");
                decoded.push('%');
                decoded.push(a);
                decoded.push(b);
                continue;
            };
            byte_buf.push(byte);

            while chars.peek() == Some(&'%') {
                chars.next(); // consume '%'
                let Some(a2) = chars.next() else { break };
                let Some(b2) = chars.next() else {
                    decoded.push('%');
                    decoded.push(a2);
                    break;
                };
                let hex2 = format!("{a2}{b2}");
                if let Ok(b2_val) = u8::from_str_radix(&hex2, 16) {
                    byte_buf.push(b2_val);
                } else {
                    // Flush accumulated bytes, then push the bad sequence as-is.
                    decoded.push_str(&String::from_utf8_lossy(&byte_buf));
                    byte_buf.clear();
                    decoded.push('%');
                    decoded.push(a2);
                    decoded.push(b2);
                    break;
                }
            }

            decoded.push_str(&String::from_utf8_lossy(&byte_buf));
        }
        decoded
    }

    fn parse_markdown_tags(raw: &str) -> HashMap<String, String> {
        raw.split(';')
            .filter_map(|chunk| {
                let (k, v) = chunk.split_once('=')?;
                if k.is_empty() || v.is_empty() {
                    return None;
                }
                let value = Self::decode_tag_value(v);
                Some((k.to_string(), value))
            })
            .collect()
    }

    fn memory_layer_to_str(layer: &MemoryLayer) -> &'static str {
        match layer {
            MemoryLayer::Working => "working",
            MemoryLayer::Episodic => "episodic",
            MemoryLayer::Semantic => "semantic",
            MemoryLayer::Procedural => "procedural",
            MemoryLayer::Identity => "identity",
        }
    }

    fn parse_memory_layer(raw: &str) -> Option<MemoryLayer> {
        match raw {
            "working" => Some(MemoryLayer::Working),
            "episodic" => Some(MemoryLayer::Episodic),
            "semantic" => Some(MemoryLayer::Semantic),
            "procedural" => Some(MemoryLayer::Procedural),
            "identity" => Some(MemoryLayer::Identity),
            _ => None,
        }
    }

    fn memory_source_to_str(source: &MemorySource) -> &'static str {
        match source {
            MemorySource::ExplicitUser => "explicit_user",
            MemorySource::ToolVerified => "tool_verified",
            MemorySource::System => "system",
            MemorySource::Inferred => "inferred",
            MemorySource::ExternalPrimary => "external_primary",
            MemorySource::ExternalSecondary => "external_secondary",
        }
    }

    fn parse_memory_source(raw: &str) -> Option<MemorySource> {
        match raw {
            "explicit_user" => Some(MemorySource::ExplicitUser),
            "tool_verified" => Some(MemorySource::ToolVerified),
            "system" => Some(MemorySource::System),
            "inferred" => Some(MemorySource::Inferred),
            "external_primary" => Some(MemorySource::ExternalPrimary),
            "external_secondary" => Some(MemorySource::ExternalSecondary),
            _ => None,
        }
    }

    fn format_tagged_line(
        key: &str,
        value: &str,
        layer: &MemoryLayer,
        provenance: Option<&MemoryProvenance>,
    ) -> String {
        let mut tag_fields = vec![format!("layer={}", Self::memory_layer_to_str(layer))];

        if let Some(provenance) = provenance {
            tag_fields.push(format!(
                "provenance_source_class={}",
                Self::memory_source_to_str(&provenance.source_class)
            ));
            tag_fields.push(format!(
                "provenance_reference={}",
                Self::encode_tag_value(&provenance.reference)
            ));

            if let Some(uri) = &provenance.evidence_uri {
                tag_fields.push(format!(
                    "provenance_evidence_uri={}",
                    Self::encode_tag_value(uri)
                ));
            }
        }

        let tag_total: usize = tag_fields.iter().map(|f| f.len() + 1).sum();
        let mut out = String::with_capacity(8 + tag_total + 8 + key.len() + 2 + value.len());
        out.push_str("- **");
        out.push_str(key);
        out.push_str("** [md:");
        let mut first = true;
        for field in &tag_fields {
            if !first {
                out.push(';');
            }
            out.push_str(field);
            first = false;
        }
        out.push_str("]: ");
        out.push_str(value);
        out
    }

    fn parse_markdown_entry_line(line: &str) -> Option<ParsedMarkdownLine> {
        let line = line.trim();
        let without_bullet = line.strip_prefix("- ")?;
        let without_key = without_bullet.strip_prefix("**")?;
        let end_key = without_key.find("**")?;
        let (key, rest) = without_key.split_at(end_key);
        let rest = rest.strip_prefix("**").unwrap_or("").trim_start();

        if let Some(content) = rest.strip_prefix(": ") {
            return Some(ParsedMarkdownLine {
                key: key.to_string(),
                content: content.to_string(),
                layer: None,
                provenance: None,
            });
        }

        if let Some(rest_after_marker) = rest.strip_prefix("[md:") {
            let tag_end = rest_after_marker.find("]: ");
            let Some(tag_end) = tag_end else {
                return Some(ParsedMarkdownLine {
                    key: key.to_string(),
                    content: format!("[md:{rest_after_marker}"),
                    layer: None,
                    provenance: None,
                });
            };

            let raw_tags = &rest_after_marker[..tag_end];
            let content = &rest_after_marker[(tag_end + 3)..];
            let tags = Self::parse_markdown_tags(raw_tags);

            let layer = tags
                .get("layer")
                .and_then(|value| Self::parse_memory_layer(value))
                .unwrap_or(MemoryLayer::Working);

            let provenance = tags.get("provenance_source_class").and_then(|source_raw| {
                let source = Self::parse_memory_source(source_raw)?;
                let reference = tags.get("provenance_reference")?.clone();
                Some(MemoryProvenance {
                    source_class: source,
                    reference,
                    evidence_uri: tags
                        .get("provenance_evidence_uri")
                        .map(std::string::ToString::to_string),
                })
            });

            return Some(ParsedMarkdownLine {
                key: key.to_string(),
                content: content.to_string(),
                layer: Some(layer),
                provenance,
            });
        }

        None
    }

    async fn read_all_entries(&self) -> anyhow::Result<Vec<MemoryEntry>> {
        let mut entries = Vec::new();

        // Read MEMORY.md (core)
        let core_path = self.core_path();
        if core_path.exists() {
            let content = fs::read_to_string(&core_path)
                .await
                .context("read core memory file")?;
            entries.extend(Self::parse_entries_from_file(
                &core_path,
                &content,
                &MemoryCategory::Core,
            ));
        }

        let mem_dir = self.memory_dir();
        if mem_dir.exists() {
            let mut dir = fs::read_dir(&mem_dir)
                .await
                .context("read memory directory")?;
            while let Some(entry) = dir
                .next_entry()
                .await
                .context("read memory directory entry")?
            {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    let content = match fs::read_to_string(&path).await {
                        Ok(content) => content,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            // Another task may rotate or prune the log between
                            // readdir and file read; skip disappeared files.
                            tracing::debug!(
                                path = %path.display(),
                                "daily memory log disappeared during read"
                            );
                            continue;
                        }
                        Err(error) => return Err(error).context("read daily memory log"),
                    };
                    entries.extend(Self::parse_entries_from_file(
                        &path,
                        &content,
                        &MemoryCategory::Daily,
                    ));
                }
            }
        }

        entries.sort_by(|a, b| Self::projection_timestamp_cmp(&b.timestamp, &a.timestamp));
        Ok(entries)
    }

    async fn read_all_parsed_lines(&self) -> anyhow::Result<Vec<TimestampedParsedLine>> {
        let mut entries = Vec::new();

        let core_path = self.core_path();
        if core_path.exists() {
            let content = fs::read_to_string(&core_path)
                .await
                .context("read core memory file")?;
            let timestamp = core_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            entries.extend(
                content
                    .lines()
                    .filter(|line| {
                        let trimmed = line.trim();
                        !trimmed.is_empty() && !trimmed.starts_with('#')
                    })
                    .filter_map(Self::parse_markdown_entry_line)
                    .map(|entry| TimestampedParsedLine {
                        timestamp: timestamp.clone(),
                        entry,
                    }),
            );
        }

        let mem_dir = self.memory_dir();
        if mem_dir.exists() {
            let mut dir = fs::read_dir(&mem_dir)
                .await
                .context("read memory directory")?;
            while let Some(entry) = dir
                .next_entry()
                .await
                .context("read memory directory entry")?
            {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let content = match fs::read_to_string(&path).await {
                    Ok(content) => content,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        tracing::debug!(path = %path.display(), "daily memory log disappeared during read");
                        continue;
                    }
                    Err(error) => return Err(error).context("read daily memory log"),
                };
                let timestamp = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                entries.extend(
                    content
                        .lines()
                        .filter(|line| {
                            let trimmed = line.trim();
                            !trimmed.is_empty() && !trimmed.starts_with('#')
                        })
                        .filter_map(Self::parse_markdown_entry_line)
                        .map(|entry| TimestampedParsedLine {
                            timestamp: timestamp.clone(),
                            entry,
                        }),
                );
            }
        }

        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(entries)
    }
}

impl MarkdownMemory {
    fn backend_name() -> &'static str {
        "markdown"
    }

    fn projection_timestamp_cmp(left: &str, right: &str) -> Ordering {
        match (left == "MEMORY", right == "MEMORY") {
            (true, true) => Ordering::Equal,
            // MEMORY.md is the curated projection file, but it has no date stem.
            // For current-slot selection, any dated append-only memory log is
            // fresher than an undated core projection entry for the same key.
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => left.cmp(right),
        }
    }

    fn projection_timestamp_is_newer_or_equal(left: &str, right: &str) -> bool {
        Self::projection_timestamp_cmp(left, right) != Ordering::Less
    }

    async fn upsert_projection(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        layer: MemoryLayer,
        provenance: Option<&MemoryProvenance>,
    ) -> anyhow::Result<()> {
        let entry = Self::format_tagged_line(key, content, &layer, provenance);
        let path = match category {
            MemoryCategory::Core => self.core_path(),
            _ => self.daily_path(),
        };
        self.append_to_file(&path, &entry).await
    }

    async fn projection_category_for_event(
        &self,
        key: &str,
        input: &MemoryEventInput,
    ) -> anyhow::Result<MemoryCategory> {
        let default_category = match input.source {
            MemorySource::ExplicitUser
            | MemorySource::ToolVerified
            | MemorySource::ExternalPrimary => MemoryCategory::Core,
            MemorySource::System => MemoryCategory::Daily,
            MemorySource::Inferred | MemorySource::ExternalSecondary => {
                MemoryCategory::Conversation
            }
        };

        if matches!(input.event_type, MemoryEventType::FactUpdated)
            && let Some(current) = self.fetch_projection_entry(key).await?
        {
            return Ok(current.category);
        }

        Ok(default_category)
    }

    async fn search_projection(
        &self,
        query: &str,
        limit: usize,
        layer_filter: Option<MemoryLayer>,
        key_prefix: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let all = self.read_all_entries().await?;
        let query_lower = query.to_lowercase();
        let keywords: Vec<&str> = query_lower.split_whitespace().collect();
        // Distinguish two caller intents:
        //
        // 1. **Slot-prefix lookup** (single whitespace-free token containing
        //    '/' or ':', e.g. `"augment/outcome_record/"` or
        //    `"persona/foo/v1"`): these tokens are SLOT KEY prefixes, not
        //    natural-language content, so we match them against the stored
        //    entry's KEY rather than its content. Without this fallback,
        //    `recall_typed` (which passes the slot prefix as the query)
        //    would return no hits because the prefix never appears in the
        //    serialized JSON payload.
        //
        // 2. **Natural-language keyword search**: multiple whitespace-
        //    separated tokens. We only match against CONTENT here — matching
        //    against the key causes false positives where short tokens like
        //    "x", "on", "api" accidentally appear in path components like
        //    "external" / "person".
        let is_slot_prefix_lookup = keywords.len() == 1
            && (keywords[0].contains('/')
                || keywords[0].contains(':')
                || keywords[0].contains('.')
                || keywords[0].ends_with('_'));

        let mut scored: Vec<MemoryEntry> = all
            .into_iter()
            .filter(|entry| key_prefix.is_none_or(|prefix| entry.key.starts_with(prefix)))
            .filter(|entry| layer_filter.is_none_or(|layer| entry.layer == Some(layer)))
            .filter_map(|mut entry| {
                let content_lower = entry.content.to_lowercase();
                let key_lower = entry.key.to_lowercase();
                let matched = keywords
                    .iter()
                    .filter(|kw| {
                        if is_slot_prefix_lookup {
                            key_lower.contains(**kw) || content_lower.contains(**kw)
                        } else {
                            content_lower.contains(**kw)
                        }
                    })
                    .count();
                if matched > 0 {
                    let score =
                        bounded_count_to_f64(matched) / bounded_count_to_f64(keywords.len());
                    entry.score = Some(score);
                    Some(entry)
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    async fn fetch_projection_entry(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        let all = self.read_all_entries().await?;
        Ok(all.into_iter().filter(|entry| entry.key == key).fold(
            None,
            |latest: Option<MemoryEntry>, entry| match latest {
                Some(current)
                    if Self::projection_timestamp_cmp(&current.timestamp, &entry.timestamp)
                        == Ordering::Greater =>
                {
                    Some(current)
                }
                _ => Some(entry),
            },
        ))
    }

    // Used by unit tests in this module (e.g. markdown_list_by_category)
    #[cfg(test)]
    async fn list_projection_entries(
        &self,
        category: Option<&MemoryCategory>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let all = self.read_all_entries().await?;
        match category {
            Some(cat) => Ok(all.into_iter().filter(|e| &e.category == cat).collect()),
            None => Ok(all),
        }
    }

    #[allow(clippy::unused_async)] // Trait requires async; markdown backend is synchronous
    async fn delete_projection_entry(&self, _key: &str) -> anyhow::Result<bool> {
        // Markdown memory is append-only by design (audit trail).
        // Return false to indicate no projection entry was removed or tombstoned.
        Ok(false)
    }

    async fn count_projection_entries(&self) -> anyhow::Result<usize> {
        let all = self.read_all_entries().await?;
        Ok(all.len())
    }

    #[allow(dead_code)]
    async fn list_current_entities(&self) -> anyhow::Result<Vec<String>> {
        let all = self.read_all_entries().await?;
        let mut entities = BTreeSet::new();
        for entry in all {
            if let Some((entity_id, _)) = entry.key.rsplit_once(':') {
                entities.insert(entity_id.to_string());
            }
        }
        Ok(entities.into_iter().collect())
    }

    #[allow(dead_code)]
    async fn list_current_slots(&self, entity_id: &str) -> anyhow::Result<Vec<BeliefSlot>> {
        let all = self.read_all_entries().await?;
        let prefix = format!("{entity_id}:");
        let mut latest = BTreeMap::new();

        for entry in all {
            if !entry.key.starts_with(&prefix) {
                continue;
            }
            let Some(slot_key) = entry.key.strip_prefix(&prefix) else {
                continue;
            };
            latest
                .entry(slot_key.to_string())
                .and_modify(|current: &mut MemoryEntry| {
                    if Self::projection_timestamp_is_newer_or_equal(
                        &entry.timestamp,
                        &current.timestamp,
                    ) {
                        *current = entry.clone();
                    }
                })
                .or_insert(entry);
        }

        Ok(latest
            .into_iter()
            .map(|(slot_key, entry)| BeliefSlot {
                entity_id: crate::contracts::ids::EntityId::new(entity_id),
                slot_key: crate::contracts::ids::SlotKey::new(slot_key),
                value: entry.content,
                source: entry.source.unwrap_or(MemorySource::System),
                confidence: Confidence::new(0.5),
                importance: Importance::new(0.5),
                privacy_level: PrivacyLevel::Private,
                updated_at: entry.timestamp,
            })
            .collect())
    }

    #[allow(dead_code)]
    async fn current_slot_provenance(
        &self,
        entity_id: &str,
        slot_key: &str,
    ) -> anyhow::Result<Option<MemoryProvenance>> {
        let key = format!("{entity_id}:{slot_key}");
        let entries = self.read_all_parsed_lines().await?;
        let latest = entries
            .into_iter()
            .filter(|entry| entry.entry.key == key)
            .fold(
                None,
                |latest: Option<TimestampedParsedLine>, entry| match latest {
                    Some(current) if current.timestamp > entry.timestamp => Some(current),
                    _ => Some(entry),
                },
            );
        Ok(latest.and_then(|entry| entry.entry.provenance))
    }

    async fn append_event(&self, input: MemoryEventInput) -> anyhow::Result<MemoryEvent> {
        let input = input.normalize_for_ingress()?;
        let key = format!("{}:{}", input.entity_id, input.slot_key);
        let category = self.projection_category_for_event(&key, &input).await?;
        self.upsert_projection(
            &key,
            &input.value,
            category,
            input.layer,
            input.provenance.as_ref(),
        )
        .await?;

        Ok(MemoryEvent {
            event_id: EventId::new(uuid::Uuid::new_v4().to_string()),
            entity_id: input.entity_id,
            slot_key: input.slot_key,
            event_type: input.event_type,
            value: input.value,
            source: input.source,
            confidence: input.confidence,
            importance: input.importance,
            provenance: input.provenance,
            privacy_level: input.privacy_level,
            occurred_at: input.occurred_at,
            ingested_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn recall_scoped(&self, query: RecallQuery) -> anyhow::Result<Vec<MemoryRecallEntry>> {
        query.enforce_policy()?;

        let entity_prefix = format!("{}:", query.entity_id);
        let rows = self
            .search_projection(
                &query.query,
                query.limit,
                query.layer_filter,
                Some(&entity_prefix),
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|entry| {
                let slot_key = entry
                    .key
                    .strip_prefix(&entity_prefix)
                    .unwrap_or(&entry.key)
                    .to_string();
                MemoryRecallEntry {
                    entity_id: query.entity_id.clone(),
                    slot_key: crate::contracts::ids::SlotKey::new(slot_key),
                    value: entry.content,
                    source: entry.source.unwrap_or(MemorySource::System),
                    // MarkdownMemory does not persist confidence; use neutral default
                    confidence: Confidence::new(0.5),
                    importance: Importance::new(0.5),
                    privacy_level: PrivacyLevel::Private,
                    score: entry.score.unwrap_or(0.0),
                    occurred_at: entry.timestamp,
                }
            })
            .collect())
    }

    async fn resolve_slot(
        &self,
        entity_id: &str,
        slot_key: &str,
    ) -> anyhow::Result<Option<BeliefSlot>> {
        let key = format!("{entity_id}:{slot_key}");
        let Some(entry) = self.fetch_projection_entry(&key).await? else {
            return Ok(None);
        };

        Ok(Some(BeliefSlot {
            entity_id: crate::contracts::ids::EntityId::new(entity_id),
            slot_key: crate::contracts::ids::SlotKey::new(slot_key),
            value: entry.content,
            source: entry.source.unwrap_or(MemorySource::System),
            // MarkdownMemory does not persist confidence; use neutral default
            confidence: Confidence::new(0.5),
            importance: Importance::new(0.5),
            privacy_level: PrivacyLevel::Private,
            updated_at: entry.timestamp,
        }))
    }

    async fn forget_slot(
        &self,
        entity_id: &str,
        slot_key: &str,
        mode: ForgetMode,
        reason: &str,
    ) -> anyhow::Result<ForgetOutcome> {
        let key = format!("{entity_id}:{slot_key}");
        let _ = reason;
        let applied = self.delete_projection_entry(&key).await?;

        let slot_observed = if self.resolve_slot(entity_id, slot_key).await?.is_some() {
            ForgetObservation::PresentRetrievable
        } else {
            ForgetObservation::Absent
        };
        let projection_observed = if self.fetch_projection_entry(&key).await?.is_some() {
            ForgetObservation::PresentRetrievable
        } else {
            ForgetObservation::Absent
        };

        let slot_requirement = match mode {
            ForgetMode::Hard => ForgetRequirement::MustBeAbsent,
            ForgetMode::Soft | ForgetMode::Tombstone => ForgetRequirement::MustBeNonRetrievable,
        };
        let retrieval_docs_requirement = slot_requirement;

        let artifact_checks = vec![
            ForgetArtifactCheck::new(ForgetArtifact::Slot, slot_requirement, slot_observed),
            ForgetArtifactCheck::new(
                ForgetArtifact::RetrievalDocs,
                retrieval_docs_requirement,
                projection_observed,
            ),
            ForgetArtifactCheck::new(
                ForgetArtifact::Caches,
                ForgetRequirement::NotGoverned,
                ForgetObservation::Absent,
            ),
            ForgetArtifactCheck::new(
                ForgetArtifact::Ledger,
                ForgetRequirement::NotGoverned,
                ForgetObservation::Absent,
            ),
        ];

        Ok(ForgetOutcome::from_checks(
            entity_id,
            slot_key,
            mode,
            applied,
            true,
            artifact_checks,
        ))
    }

    async fn count_events(&self, _entity_id: Option<&str>) -> anyhow::Result<usize> {
        self.count_projection_entries().await
    }

    #[allow(dead_code)]
    async fn list_entities(&self) -> anyhow::Result<Vec<String>> {
        self.list_current_entities().await
    }

    #[allow(dead_code)]
    async fn list_slots(&self, entity_id: &str) -> anyhow::Result<Vec<BeliefSlot>> {
        self.list_current_slots(entity_id).await
    }

    #[allow(dead_code)]
    async fn slot_provenance(
        &self,
        entity_id: &str,
        slot_key: &str,
    ) -> anyhow::Result<Option<MemoryProvenance>> {
        self.current_slot_provenance(entity_id, slot_key).await
    }

    #[allow(clippy::unused_async)] // Trait requires async; existence check is synchronous
    async fn health_check(&self) -> bool {
        self.workspace_dir.exists()
    }
}

fn bounded_count_to_f64(value: usize) -> f64 {
    match u32::try_from(value) {
        Ok(value) => f64::from(value),
        Err(_) => f64::from(u32::MAX),
    }
}

fn markdown_query_error(error: anyhow::Error) -> MemoryError {
    match error.downcast::<MemoryError>() {
        Ok(error) => error,
        Err(error) => MemoryError::query(error.to_string()),
    }
}

fn markdown_write_error(error: anyhow::Error) -> MemoryError {
    match error.downcast::<MemoryError>() {
        Ok(error) => error,
        Err(error) => MemoryError::write(error.to_string()),
    }
}

impl MemoryWriter for MarkdownMemory {
    fn append_event(
        &self,
        input: MemoryEventInput,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<MemoryEvent>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            MarkdownMemory::append_event(self, input)
                .await
                .map_err(markdown_write_error)
        })
    }
}

impl MemoryReader for MarkdownMemory {
    fn recall_scoped(
        &self,
        query: RecallQuery,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = crate::contracts::memory_error::MemoryResult<Vec<MemoryRecallEntry>>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            MarkdownMemory::recall_scoped(self, query)
                .await
                .map_err(markdown_query_error)
        })
    }

    fn resolve_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<Option<BeliefSlot>>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            MarkdownMemory::resolve_slot(self, entity_id, slot_key)
                .await
                .map_err(markdown_query_error)
        })
    }
}

impl MemoryGovernance for MarkdownMemory {
    fn name(&self) -> &str {
        Self::backend_name()
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { MarkdownMemory::health_check(self).await })
    }

    fn forget_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
        mode: ForgetMode,
        reason: &'a str,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<ForgetOutcome>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            MarkdownMemory::forget_slot(self, entity_id, slot_key, mode, reason)
                .await
                .map_err(markdown_write_error)
        })
    }

    fn count_events<'a>(
        &'a self,
        entity_id: Option<&'a str>,
    ) -> Pin<
        Box<dyn Future<Output = crate::contracts::memory_error::MemoryResult<usize>> + Send + 'a>,
    > {
        Box::pin(async move {
            MarkdownMemory::count_events(self, entity_id)
                .await
                .map_err(markdown_query_error)
        })
    }

    fn list_entities(
        &self,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<Vec<String>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            MarkdownMemory::list_entities(self)
                .await
                .map_err(markdown_query_error)
        })
    }

    fn list_slots<'a>(
        &'a self,
        entity_id: &'a str,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<Vec<BeliefSlot>>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            MarkdownMemory::list_slots(self, entity_id)
                .await
                .map_err(markdown_query_error)
        })
    }

    fn slot_provenance<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = crate::contracts::memory_error::MemoryResult<Option<MemoryProvenance>>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            MarkdownMemory::slot_provenance(self, entity_id, slot_key)
                .await
                .map_err(markdown_query_error)
        })
    }
}

#[cfg(test)]
mod tests;
