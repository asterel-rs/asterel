use std::fmt::Write;

use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

use super::{EXAMPLE_LIMIT, finalize_compaction, truncate_preview};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GitStatusFormatter;

impl SemanticFormatter for GitStatusFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        if let Some(summary) = compact_short_status(raw) {
            return finalize_compaction(raw, summary);
        }
        if let Some(summary) = compact_long_status(raw) {
            return finalize_compaction(raw, summary);
        }
        SemanticCompactionOutcome::FallbackRaw
    }
}

#[derive(Debug, Default)]
struct StatusBuckets {
    branch: Option<String>,
    ahead_behind: Option<String>,
    staged: StatusBucket,
    modified: StatusBucket,
    untracked: StatusBucket,
    unmerged: StatusBucket,
}

#[derive(Debug, Default)]
struct StatusBucket {
    count: usize,
    examples: Vec<String>,
}

fn compact_short_status(raw: &str) -> Option<String> {
    let mut buckets = StatusBuckets::default();
    let mut parsed_any = false;

    for line in raw.lines() {
        if let Some(branch_line) = line.strip_prefix("## ") {
            parsed_any = true;
            let branch = branch_line
                .split("...")
                .next()
                .unwrap_or(branch_line)
                .trim()
                .to_string();
            buckets.branch = Some(branch);

            if let Some(details) = branch_line
                .split_once('[')
                .and_then(|(_, suffix)| suffix.strip_suffix(']'))
            {
                buckets.ahead_behind = Some(details.trim().to_string());
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("?? ") {
            parsed_any = true;
            push_example(&mut buckets.untracked, path);
            continue;
        }

        let bytes = line.as_bytes();
        if bytes.len() < 3 || bytes[2] != b' ' {
            return None;
        }

        let index = bytes[0] as char;
        let worktree = bytes[1] as char;
        if !is_valid_short_status_code(index) || !is_valid_short_status_code(worktree) {
            return None;
        }
        let path = line[3..].trim();
        if path.is_empty() {
            return None;
        }

        parsed_any = true;
        if index == 'U' || worktree == 'U' {
            push_example(&mut buckets.unmerged, path);
            continue;
        }
        if index != ' ' {
            push_example(&mut buckets.staged, path);
        }
        if worktree != ' ' {
            push_example(&mut buckets.modified, path);
        }
    }

    parsed_any.then(|| render_status_summary("git status", &buckets))
}

fn compact_long_status(raw: &str) -> Option<String> {
    let mut buckets = StatusBuckets::default();
    let mut section = "";
    let mut parsed_any = false;

    for line in raw.lines() {
        let trimmed = line.trim_end();
        if let Some(branch) = trimmed.strip_prefix("On branch ") {
            buckets.branch = Some(branch.trim().to_string());
            parsed_any = true;
            continue;
        }
        if trimmed.starts_with("Your branch is ") {
            buckets.ahead_behind = Some(trimmed.to_string());
            parsed_any = true;
            continue;
        }
        match trimmed {
            "Changes to be committed:" => {
                section = "staged";
                parsed_any = true;
                continue;
            }
            "Changes not staged for commit:" => {
                section = "modified";
                parsed_any = true;
                continue;
            }
            "Untracked files:" => {
                section = "untracked";
                parsed_any = true;
                continue;
            }
            "Unmerged paths:" => {
                section = "unmerged";
                parsed_any = true;
                continue;
            }
            _ => {}
        }

        if !section.is_empty() {
            if trimmed.is_empty() {
                continue;
            }

            if line.chars().next().is_some_and(char::is_whitespace) {
                match extract_long_status_path(section, trimmed) {
                    Some(path) => {
                        parsed_any = true;
                        match section {
                            "staged" => push_example(&mut buckets.staged, path),
                            "modified" => push_example(&mut buckets.modified, path),
                            "untracked" => push_example(&mut buckets.untracked, path),
                            "unmerged" => push_example(&mut buckets.unmerged, path),
                            _ => {}
                        }
                    }
                    None if trimmed.starts_with('(') || trimmed.is_empty() => {}
                    None => return None,
                }
                continue;
            }

            if is_known_long_status_footer(trimmed) {
                section = "";
                continue;
            }

            return None;
        }
    }

    parsed_any.then(|| render_status_summary("git status", &buckets))
}

fn is_known_long_status_footer(line: &str) -> bool {
    line.starts_with("no changes added to commit")
        || line.starts_with("nothing added to commit but untracked files present")
}

fn extract_long_status_path<'a>(section: &str, line: &'a str) -> Option<&'a str> {
    let trimmed = line.trim();
    for prefix in [
        "modified:",
        "new file:",
        "deleted:",
        "renamed:",
        "both modified:",
        "both added:",
        "added by us:",
        "deleted by them:",
        "deleted by us:",
        "both deleted:",
    ] {
        if let Some(path) = trimmed.strip_prefix(prefix) {
            return Some(path.trim());
        }
    }
    if section == "untracked"
        && !trimmed.starts_with('(')
        && !trimmed.is_empty()
        && !trimmed.ends_with(':')
    {
        return Some(trimmed);
    }
    None
}

fn is_valid_short_status_code(code: char) -> bool {
    matches!(code, ' ' | 'M' | 'T' | 'A' | 'D' | 'R' | 'C' | 'U')
}

fn render_status_summary(header: &str, buckets: &StatusBuckets) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{header}");
    if let Some(branch) = &buckets.branch {
        let _ = writeln!(out, "branch: {branch}");
    }
    if let Some(ahead_behind) = &buckets.ahead_behind {
        let _ = writeln!(out, "branch-state: {ahead_behind}");
    }
    write_bucket(&mut out, "staged", &buckets.staged);
    write_bucket(&mut out, "modified", &buckets.modified);
    write_bucket(&mut out, "untracked", &buckets.untracked);
    write_bucket(&mut out, "unmerged", &buckets.unmerged);
    out.trim_end().to_string()
}

fn write_bucket(out: &mut String, label: &str, bucket: &StatusBucket) {
    let _ = writeln!(out, "{label}: {}", bucket.count);
    if !bucket.examples.is_empty() {
        let rendered = bucket
            .examples
            .iter()
            .map(|entry| truncate_preview(entry, 120))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "{label}-examples: {rendered}");
    }
}

fn push_example(bucket: &mut StatusBucket, path: &str) {
    bucket.count += 1;
    if bucket.examples.len() < EXAMPLE_LIMIT {
        bucket.examples.push(path.trim().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::GitStatusFormatter;
    use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
    use crate::core::tools::traits::ToolResultSemanticMetadata;

    #[test]
    fn git_status_formatter_compacts_porcelain_status() {
        let mut raw = String::from("## main...origin/main [ahead 2]\nM  src/main.rs\n");
        for index in 0..128 {
            raw.push_str(&format!(
                " M src/components/{index}/{}\n",
                "very_long_status_entry".repeat(6)
            ));
        }
        raw.push_str("?? notes.txt\n");
        let outcome = GitStatusFormatter.compact(&raw, &ToolResultSemanticMetadata::default());

        match outcome {
            SemanticCompactionOutcome::Compacted { content, .. } => {
                assert!(content.contains("branch: main"));
                assert!(content.contains("branch-state: ahead 2"));
                assert!(content.contains("staged: 1"));
                assert!(content.contains("modified: 1"));
                assert!(content.contains("untracked: 1"));
            }
            other => panic!("expected compacted outcome, got {other:?}"),
        }
    }

    #[test]
    fn git_status_formatter_falls_back_on_unstructured_input() {
        let outcome = GitStatusFormatter.compact(
            "plain text without git status structure",
            &ToolResultSemanticMetadata::default(),
        );
        assert!(matches!(outcome, SemanticCompactionOutcome::FallbackRaw));
    }

    #[test]
    fn git_status_formatter_falls_back_on_invalid_porcelain_codes() {
        let raw = "@@ src/lib.rs\n";
        let outcome = GitStatusFormatter.compact(raw, &ToolResultSemanticMetadata::default());
        assert!(matches!(outcome, SemanticCompactionOutcome::FallbackRaw));
    }

    #[test]
    fn git_status_formatter_ignores_long_status_footer_text() {
        let mut raw = String::from("On branch main\nChanges not staged for commit:\n");
        for index in 0..160 {
            raw.push_str(&format!("  modified: src/lib_{index}.rs\n"));
        }
        raw.push_str("\nno changes added to commit (use \"git add\" and/or \"git commit -a\")\n");

        let outcome = GitStatusFormatter.compact(&raw, &ToolResultSemanticMetadata::default());
        let SemanticCompactionOutcome::Compacted { content, .. } = outcome else {
            panic!("expected compacted output");
        };

        assert!(content.contains("modified: 160"));
        assert!(!content.contains("no changes added to commit"));
    }

    #[test]
    fn git_status_formatter_falls_back_on_unknown_long_status_footer_text() {
        let raw = r#"On branch main
Changes not staged for commit:
  modified: src/lib.rs

malformed footer that should not be accepted
"#;

        let outcome = GitStatusFormatter.compact(raw, &ToolResultSemanticMetadata::default());
        assert!(matches!(outcome, SemanticCompactionOutcome::FallbackRaw));
    }

    #[test]
    fn git_status_formatter_declines_low_savings() {
        let raw = "## main\n?? a\n";
        let outcome = GitStatusFormatter.compact(raw, &ToolResultSemanticMetadata::default());
        assert!(matches!(outcome, SemanticCompactionOutcome::Passthrough));
    }
}
