//! State mirror I/O: reads and atomically writes the persona state mirror
//! file (`state_mirror_path()`), which is a Markdown file containing the
//! canonical `StateHeader` as a fenced JSON block.
//!
//! `read_mirror_state` parses the fenced triple-backtick `json` block first;
//! falls back to treating the whole file as raw JSON if the fence is absent.
//!
//! `sync_mirror_from_backend_canonical` serialises the `StateHeader` via
//! the presenter and atomically replaces the mirror file using a
//! `write_atomic` rename sequence (`.<uuid_temp>` → target path) to
//! prevent partial writes.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::{BackendHeaderPersist, StateHeader};

pub(super) fn read_mirror_state(service: &BackendHeaderPersist) -> Result<Option<StateHeader>> {
    let mirror_path = service.state_mirror_path();
    if !mirror_path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&mirror_path)
        .with_context(|| format!("failed reading state mirror: {}", mirror_path.display()))?;
    let parsed = parse_state_header_mirror_markdown(&raw)?;
    parsed.validate(&service.persona)?;

    Ok(Some(parsed))
}

pub(super) fn sync_mirror_from_backend_canonical(
    service: &BackendHeaderPersist,
    state: &StateHeader,
) -> Result<()> {
    let mirror_path = service.state_mirror_path();
    let content = crate::core::persona::presenter::render_state_header_mirror_markdown(state)?;
    write_atomic(&mirror_path, &content)
}

fn parse_state_header_mirror_markdown(raw: &str) -> Result<StateHeader> {
    if let Some(start) = raw.find("```json") {
        let after_start = &raw[start + "```json".len()..];
        if let Some(end) = after_start.find("```") {
            let json_block = after_start[..end].trim();
            let parsed: StateHeader = serde_json::from_str(json_block)
                .context("failed parsing json block from state mirror")?;
            return Ok(parsed);
        }
    }

    let parsed: StateHeader =
        serde_json::from_str(raw.trim()).context("failed parsing raw state mirror as json")?;
    Ok(parsed)
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        anyhow::ensure!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "state mirror filename must not contain path separators or '..': {name}"
        );
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating mirror parent: {}", parent.display()))?;
    }

    let temp_name = format!(".state_tmp_{}", uuid::Uuid::new_v4().as_simple());
    let temp_path = path.with_file_name(temp_name);
    fs::write(&temp_path, content)
        .with_context(|| format!("failed writing mirror temp file: {}", temp_path.display()))?;

    if let Err(rename_error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(rename_error).with_context(|| {
            format!(
                "failed replacing mirror file atomically: {}",
                path.display()
            )
        });
    }

    Ok(())
}
