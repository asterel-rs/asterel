use std::fmt::Write as _;

use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

const MAX_HEADERS_PER_FILE: usize = 5;
const MIN_ABSOLUTE_SAVINGS_BYTES: usize = 1_024;
const MIN_RELATIVE_SAVINGS_PERCENT: usize = 15;
const CONFIDENCE: f32 = 0.96;

#[derive(Debug, Default)]
pub struct GitDiffFormatter;

impl SemanticFormatter for GitDiffFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Ok(parsed) = parse_git_diff(raw) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };

        let compacted = render_git_diff(&parsed);
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
struct ParsedGitDiff {
    files: Vec<DiffFile>,
}

#[derive(Debug, Default)]
struct DiffFile {
    identity: String,
    headers: Vec<String>,
    added: usize,
    removed: usize,
    hunks: usize,
}

fn parse_git_diff(raw: &str) -> Result<ParsedGitDiff, ()> {
    let mut parsed = ParsedGitDiff::default();
    let mut current: Option<DiffFile> = None;
    let mut in_hunk = false;
    let mut saw_file = false;

    for raw_line in raw.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }

        if let Some(identity) = parse_diff_header(line) {
            if let Some(file) = current.take() {
                parsed.files.push(file);
            }
            current = Some(DiffFile {
                identity,
                headers: vec![line.to_string()],
                ..DiffFile::default()
            });
            in_hunk = false;
            saw_file = true;
            continue;
        }

        let Some(file) = current.as_mut() else {
            return Err(());
        };

        if is_header_line(line) && !in_hunk {
            push_header(file, line);
            continue;
        }

        if line.starts_with("@@ ") {
            file.hunks += 1;
            push_header(file, line);
            in_hunk = true;
            continue;
        }

        if line.starts_with('+') && !line.starts_with("+++") {
            file.added += 1;
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            file.removed += 1;
            continue;
        }
        if line.starts_with('\\') {
            continue;
        }

        if line.starts_with("Binary files ")
            || line.starts_with("GIT binary patch")
            || line.starts_with("literal ")
            || line.starts_with("delta ")
        {
            push_header(file, line);
            continue;
        }

        if in_hunk {
            if line.starts_with(' ') {
                continue;
            }
            return Err(());
        }

        return Err(());
    }

    if let Some(file) = current.take() {
        parsed.files.push(file);
    }

    if !saw_file || parsed.files.is_empty() {
        return Err(());
    }

    Ok(parsed)
}

fn parse_diff_header(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    match (parts.next()?, parts.next()?, parts.next()?, parts.next()?) {
        ("diff", "--git", a_path, b_path) if parts.next().is_none() => {
            let left = normalize_diff_path(a_path);
            let right = normalize_diff_path(b_path);
            Some(match (left, right) {
                (Some(left), Some(right)) if left != right => format!("{left} -> {right}"),
                (_, Some(right)) => right,
                (Some(left), None) => left,
                _ => b_path.to_string(),
            })
        }
        _ => None,
    }
}

fn normalize_diff_path(path: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix("a/") {
        return Some(rest.to_string());
    }
    if let Some(rest) = path.strip_prefix("b/") {
        return Some(rest.to_string());
    }
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn is_header_line(line: &str) -> bool {
    line.starts_with("index ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("similarity index ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("copy from ")
        || line.starts_with("copy to ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
}

fn push_header(file: &mut DiffFile, line: &str) {
    if file.headers.len() < MAX_HEADERS_PER_FILE
        && file.headers.last().is_none_or(|last| last != line)
    {
        file.headers.push(line.to_string());
    }
}

fn render_git_diff(parsed: &ParsedGitDiff) -> String {
    let mut rendered = String::new();
    writeln!(&mut rendered, "git diff").unwrap();
    writeln!(&mut rendered, "files: {}", parsed.files.len()).unwrap();

    for file in &parsed.files {
        writeln!(&mut rendered, "file: {}", file.identity).unwrap();
        writeln!(
            &mut rendered,
            "counts: +{} -{} hunks={}",
            file.added, file.removed, file.hunks
        )
        .unwrap();
        if !file.headers.is_empty() {
            writeln!(&mut rendered, "headers: {}", file.headers.join(" | ")).unwrap();
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
    fn compacts_noisy_valid_git_diff_output() {
        let mut raw = String::new();
        for file_idx in 0..12 {
            let suffix = "x".repeat(320);
            raw.push_str(&format!(
                "diff --git a/src/file_{file_idx}/{suffix} b/src/file_{file_idx}/{suffix}\n"
            ));
            raw.push_str("index 1111111..2222222 100644\n");
            raw.push_str(&format!("--- a/src/file_{file_idx}/{suffix}\n"));
            raw.push_str(&format!("+++ b/src/file_{file_idx}/{suffix}\n"));
            raw.push_str("@@ -1,4 +1,6 @@\n");
            raw.push_str("-old_line()\n");
            raw.push_str("+new_line()\n");
            raw.push_str(" context line that repeats to inflate the raw diff payload\n");
            raw.push_str("@@ -30,2 +30,3 @@\n");
            raw.push_str("-line_a()\n");
            raw.push_str("+line_b()\n");
            raw.push_str("+line_c()\n");
            raw.push_str(" another repeated context line to keep the unified diff realistic\n");
            for _ in 0..24 {
                raw.push_str(" context line that keeps the hunk body realistic and verbose\n");
            }
        }

        let outcome = GitDiffFormatter.compact(&raw, &ToolResultSemanticMetadata::default());
        let SemanticCompactionOutcome::Compacted {
            content,
            confidence,
        } = outcome
        else {
            panic!("expected compacted output");
        };

        assert!(confidence > 0.9);
        assert!(content.contains("files: 12"));
        assert!(content.contains("file: src/file_0/"));
        assert!(content.contains("counts: +3 -2 hunks=2"));
        assert!(content.contains("headers: diff --git a/src/file_0/"));
    }

    #[test]
    fn falls_back_raw_on_malformed_git_diff_output() {
        let raw = "this is not a diff";

        assert!(matches!(
            GitDiffFormatter.compact(raw, &ToolResultSemanticMetadata::default()),
            SemanticCompactionOutcome::FallbackRaw
        ));
    }

    #[test]
    fn falls_back_raw_on_malformed_hunk_body() {
        let raw = r#"diff --git a/src/lib.rs b/src/lib.rs
index deadbeef..feedface 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-old
+new
not valid unified diff context
"#;

        assert!(matches!(
            GitDiffFormatter.compact(raw, &ToolResultSemanticMetadata::default()),
            SemanticCompactionOutcome::FallbackRaw
        ));
    }

    #[test]
    fn declines_low_savings_git_diff_output() {
        let mut raw = String::new();
        for i in 0..3 {
            let suffix = "z".repeat(1_900);
            raw.push_str(&format!(
                "diff --git a/deep/path/{i}/{suffix} b/deep/path/{i}/{suffix}\n"
            ));
            raw.push_str(&format!(
                "index deadbeef..feedface 100644\n--- a/deep/path/{i}/{suffix}\n+++ b/deep/path/{i}/{suffix}\n@@ -1,2 +1,2 @@\n-context line\n+replacement line\n"
            ));
        }

        assert!(matches!(
            GitDiffFormatter.compact(&raw, &ToolResultSemanticMetadata::default()),
            SemanticCompactionOutcome::Passthrough
        ));
    }
}
