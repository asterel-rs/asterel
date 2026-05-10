use std::collections::HashMap;
use std::fmt::Write as _;

use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

const MAX_PREVIEW_LINES_PER_FILE: usize = 3;
const MIN_ABSOLUTE_SAVINGS_BYTES: usize = 1_024;
const MIN_RELATIVE_SAVINGS_PERCENT: usize = 15;
const CONFIDENCE: f32 = 0.97;

#[derive(Debug, Default)]
pub struct RipgrepFormatter;

impl SemanticFormatter for RipgrepFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Ok(parsed) = parse_ripgrep(raw) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };

        let compacted = render_ripgrep(&parsed);
        if !savings_are_meaningful(raw, &compacted) {
            return SemanticCompactionOutcome::Passthrough;
        }

        SemanticCompactionOutcome::Compacted {
            content: compacted,
            confidence: CONFIDENCE,
        }
    }
}

#[derive(Debug, Default)]
struct ParsedRipgrep {
    total_matches: usize,
    files: Vec<RgFileGroup>,
}

#[derive(Debug, Default)]
struct RgFileGroup {
    file: String,
    matches: usize,
    previews: Vec<String>,
}

#[derive(Debug)]
enum ParsedLine {
    Match { file: String, preview: String },
    Count { file: String, count: usize },
    PreviewOnly { file: String, preview: String },
}

fn parse_ripgrep(raw: &str) -> Result<ParsedRipgrep, ()> {
    let mut parsed = ParsedRipgrep::default();
    let mut file_index = HashMap::<String, usize>::new();

    for raw_line in raw.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        let Some(record) = parse_line(trimmed) else {
            return Err(());
        };

        match record {
            ParsedLine::Count { file, count } => {
                parsed.total_matches += count;
                let group = ensure_group(&mut parsed, &mut file_index, file);
                group.matches += count;
            }
            ParsedLine::Match { file, preview } => {
                parsed.total_matches += 1;
                let group = ensure_group(&mut parsed, &mut file_index, file);
                group.matches += 1;
                push_preview(group, preview);
            }
            ParsedLine::PreviewOnly { file, preview } => {
                let group = ensure_group(&mut parsed, &mut file_index, file);
                push_preview(group, preview);
            }
        }
    }

    if parsed.total_matches == 0 || parsed.files.is_empty() {
        return Err(());
    }

    Ok(parsed)
}

fn parse_line(line: &str) -> Option<ParsedLine> {
    if let Some(rest) = line.strip_prefix("Binary file ") {
        let file = rest.strip_suffix(" matches")?.trim();
        if file.is_empty() {
            return None;
        }
        return Some(ParsedLine::Match {
            file: file.to_string(),
            preview: "binary file matches".to_string(),
        });
    }

    if let Some(record) = parse_colon_record(line) {
        return Some(record);
    }

    if let Some(record) = parse_dash_preview(line) {
        return Some(record);
    }

    None
}

fn parse_colon_record(line: &str) -> Option<ParsedLine> {
    let (file, rest) = line.split_once(':')?;
    let file = file.trim();
    let rest = rest.trim_start();
    if file.is_empty() || rest.is_empty() || !looks_like_ripgrep_path(file) {
        return None;
    }

    if rest.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(ParsedLine::Count {
            file: file.to_string(),
            count: rest.parse().ok()?,
        });
    }

    let mut parts = rest.splitn(3, ':');
    let first = parts.next()?;
    if first.chars().all(|ch| ch.is_ascii_digit()) {
        match parts.next() {
            Some(second) if second.chars().all(|ch| ch.is_ascii_digit()) => {
                let preview = parts.next().unwrap_or("");
                return Some(ParsedLine::Match {
                    file: file.to_string(),
                    preview: format!("{first}:{second}: {preview}"),
                });
            }
            Some(text) => {
                return Some(ParsedLine::Match {
                    file: file.to_string(),
                    preview: format!("{first}: {text}"),
                });
            }
            None => {
                return Some(ParsedLine::Match {
                    file: file.to_string(),
                    preview: first.to_string(),
                });
            }
        }
    }

    None
}

fn parse_dash_preview(line: &str) -> Option<ParsedLine> {
    let mut parts = line.rsplitn(3, '-');
    let preview = parts.next()?.trim();
    let line_no = parts.next()?.trim();
    let file = parts.next()?.trim();

    if file.is_empty() || preview.is_empty() || !line_no.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    Some(ParsedLine::PreviewOnly {
        file: file.to_string(),
        preview: format!("{line_no}: {preview}"),
    })
}

fn looks_like_ripgrep_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(char::is_whitespace)
        && path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\'))
        && (path.contains('/')
            || path.contains('\\')
            || path.contains('.')
            || path == path.to_ascii_uppercase())
}

fn ensure_group<'a>(
    parsed: &'a mut ParsedRipgrep,
    file_index: &mut HashMap<String, usize>,
    file: String,
) -> &'a mut RgFileGroup {
    if let Some(index) = file_index.get(&file).copied() {
        return &mut parsed.files[index];
    }

    let index = parsed.files.len();
    parsed.files.push(RgFileGroup {
        file: file.clone(),
        ..RgFileGroup::default()
    });
    file_index.insert(file, index);
    &mut parsed.files[index]
}

fn push_preview(group: &mut RgFileGroup, preview: String) {
    if group.previews.len() < MAX_PREVIEW_LINES_PER_FILE {
        group.previews.push(preview);
    }
}

fn render_ripgrep(parsed: &ParsedRipgrep) -> String {
    let mut rendered = String::new();
    let _ = writeln!(&mut rendered, "rg");
    let _ = writeln!(&mut rendered, "matches: {}", parsed.total_matches);
    let _ = writeln!(&mut rendered, "files: {}", parsed.files.len());

    for group in &parsed.files {
        let _ = writeln!(&mut rendered, "file: {} [{}]", group.file, group.matches);
        for preview in &group.previews {
            let _ = writeln!(&mut rendered, "  {preview}");
        }
    }

    rendered.trim_end().to_string()
}

fn savings_are_meaningful(raw: &str, compacted: &str) -> bool {
    let raw_len = raw.len();
    let compacted_len = compacted.len();
    let saved_bytes = raw_len.saturating_sub(compacted_len);
    saved_bytes >= MIN_ABSOLUTE_SAVINGS_BYTES
        && saved_bytes.saturating_mul(100) >= raw_len.saturating_mul(MIN_RELATIVE_SAVINGS_PERCENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_noisy_valid_ripgrep_output() {
        let mut raw = String::new();
        for file_idx in 0..5 {
            let suffix = "r".repeat(120);
            for line_idx in 0..6 {
                raw.push_str(&format!(
                    "src/rg_file_{file_idx}/{suffix}:{line}:match {file_idx}-{line_idx}\n",
                    line = 10 + line_idx
                ));
            }
        }
        raw.push_str("Binary file assets/blob.bin matches\n");

        let outcome = RipgrepFormatter.compact(&raw, &ToolResultSemanticMetadata::default());
        let SemanticCompactionOutcome::Compacted {
            content,
            confidence,
        } = outcome
        else {
            panic!("expected compacted output");
        };

        assert!(confidence > 0.9);
        assert!(content.contains("matches: 31"));
        assert!(content.contains("files: 6"));
        assert!(content.contains("file: src/rg_file_0/"));
        assert!(content.contains("file: assets/blob.bin [1]"));
        assert!(content.contains("binary file matches"));
    }

    #[test]
    fn falls_back_raw_on_malformed_ripgrep_output() {
        let raw = "this is not a ripgrep payload";

        assert!(matches!(
            RipgrepFormatter.compact(raw, &ToolResultSemanticMetadata::default()),
            SemanticCompactionOutcome::FallbackRaw
        ));
    }

    #[test]
    fn falls_back_raw_on_non_path_colon_record() {
        let raw = "status: definitely not a ripgrep match";

        assert!(matches!(
            RipgrepFormatter.compact(raw, &ToolResultSemanticMetadata::default()),
            SemanticCompactionOutcome::FallbackRaw
        ));
    }

    #[test]
    fn falls_back_raw_on_path_like_plain_file_preview_without_line_numbers() {
        let raw = "config.toml: plain text without ripgrep line structure";

        assert!(matches!(
            RipgrepFormatter.compact(raw, &ToolResultSemanticMetadata::default()),
            SemanticCompactionOutcome::FallbackRaw
        ));
    }

    #[test]
    fn compacts_uppercase_plain_filename_with_line_numbers() {
        let mut raw = String::new();
        for index in 0..120 {
            raw.push_str(&format!(
                "README:{}:{}\n",
                index + 1,
                "upper-case filename hit that should still compact".repeat(3)
            ));
        }

        let outcome = RipgrepFormatter.compact(&raw, &ToolResultSemanticMetadata::default());
        let SemanticCompactionOutcome::Compacted { content, .. } = outcome else {
            panic!("expected compacted output");
        };

        assert!(content.contains("file: README [120]"));
    }

    #[test]
    fn declines_low_savings_ripgrep_output() {
        let mut raw = String::new();
        for i in 0..4 {
            let suffix = "q".repeat(2_000);
            raw.push_str(&format!(
                "deep/very/long/path/{i}/{suffix}:{}:{}\n",
                100 + i,
                "x".repeat(40)
            ));
        }

        assert!(matches!(
            RipgrepFormatter.compact(&raw, &ToolResultSemanticMetadata::default()),
            SemanticCompactionOutcome::Passthrough
        ));
    }
}
