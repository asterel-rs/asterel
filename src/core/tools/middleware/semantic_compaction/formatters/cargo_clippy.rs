use crate::core::tools::traits::ToolResultSemanticMetadata;

use super::super::{SemanticCompactionOutcome, SemanticFormatter};

const CARGO_CLIPPY_CONFIDENCE: f32 = 0.97;
const MIN_REQUIRED_SAVED_CHARS: usize = 48;

#[derive(Debug, Default)]
pub(crate) struct CargoClippyFormatter;

impl SemanticFormatter for CargoClippyFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let normalized = normalize_for_parsing(raw);
        match parse_clippy_report(&normalized) {
            ParseResult::Unsupported => SemanticCompactionOutcome::Passthrough,
            ParseResult::Malformed => SemanticCompactionOutcome::FallbackRaw,
            ParseResult::Parsed(report) => {
                let compacted = report.render();
                if compacted.is_empty() || low_savings(&normalized, &compacted) {
                    return SemanticCompactionOutcome::Passthrough;
                }

                SemanticCompactionOutcome::Compacted {
                    content: compacted,
                    confidence: CARGO_CLIPPY_CONFIDENCE,
                }
            }
        }
    }
}

#[derive(Debug)]
enum ParseResult {
    Parsed(ClippyReport),
    Unsupported,
    Malformed,
}

#[derive(Debug, Default)]
struct ClippyReport {
    events: Vec<ClippyEvent>,
}

impl ClippyReport {
    fn render(&self) -> String {
        self.events
            .iter()
            .map(ClippyEvent::render)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Debug)]
enum ClippyEvent {
    Diagnostic(DiagnosticBlock),
    Summary(String),
}

impl ClippyEvent {
    fn render(&self) -> String {
        match self {
            Self::Diagnostic(block) => block.render(),
            Self::Summary(line) => line.clone(),
        }
    }
}

#[derive(Debug)]
struct DiagnosticBlock {
    header: String,
    anchor: String,
    snippet: Vec<String>,
    annotations: Vec<String>,
}

impl DiagnosticBlock {
    fn render(&self) -> String {
        let mut rendered = String::new();
        rendered.push_str(&self.header);
        rendered.push('\n');
        rendered.push_str(&self.anchor);

        for line in &self.snippet {
            rendered.push('\n');
            rendered.push_str(line);
        }

        for line in &self.annotations {
            rendered.push('\n');
            rendered.push_str(line);
        }

        rendered
    }
}

fn parse_clippy_report(raw: &str) -> ParseResult {
    let lines = raw.lines().collect::<Vec<_>>();
    let mut report = ClippyReport::default();
    let mut saw_structural_cue = false;
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            index += 1;
            continue;
        }

        if is_summary_line(trimmed) {
            report
                .events
                .push(ClippyEvent::Summary(trimmed.to_string()));
            saw_structural_cue = true;
            index += 1;
            continue;
        }

        if is_diagnostic_header(trimmed) {
            match parse_diagnostic_block(&lines, index) {
                Some((block, next_index)) => {
                    report.events.push(ClippyEvent::Diagnostic(block));
                    saw_structural_cue = true;
                    index = next_index;
                }
                None if is_plain_diagnostic_header(trimmed) => {
                    report
                        .events
                        .push(ClippyEvent::Summary(trimmed.to_string()));
                    saw_structural_cue = true;
                    index += 1;
                }
                None => return ParseResult::Malformed,
            }
            continue;
        }

        if saw_structural_cue {
            return ParseResult::Malformed;
        }

        index += 1;
    }

    if !saw_structural_cue {
        return ParseResult::Unsupported;
    }

    ParseResult::Parsed(report)
}

fn parse_diagnostic_block(lines: &[&str], start_index: usize) -> Option<(DiagnosticBlock, usize)> {
    let header = lines[start_index].trim_start().to_string();
    let mut index = start_index + 1;
    while index < lines.len() && lines[index].trim().is_empty() {
        index += 1;
    }

    let anchor_line = lines.get(index)?.trim_start();
    if !is_anchor_line(anchor_line) {
        return None;
    }
    let anchor = anchor_line.to_string();
    index += 1;

    let mut block_lines = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if !block_lines.is_empty()
            && lines[index - 1].trim().is_empty()
            && (is_diagnostic_header(trimmed) || is_summary_line(trimmed))
        {
            break;
        }
        block_lines.push(line.trim_end().to_string());
        index += 1;
    }

    let snippet = select_snippet_lines(&block_lines);
    let annotations = select_annotation_lines(&block_lines);
    if snippet.is_empty() && annotations.is_empty() {
        return None;
    }
    if block_lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .any(|line| !is_valid_diagnostic_detail_line(line))
    {
        return None;
    }

    Some((
        DiagnosticBlock {
            header,
            anchor,
            snippet,
            annotations,
        },
        index,
    ))
}

fn select_snippet_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty() && line.contains('|'))
        .take(4)
        .cloned()
        .collect()
}

fn select_annotation_lines(lines: &[String]) -> Vec<String> {
    let mut annotations = Vec::new();

    for line in lines {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("= note:") && !trimmed.starts_with("= help:") {
            continue;
        }

        let annotation = if let Some(lint) = extract_lint_name(trimmed) {
            format!("= lint: {lint}")
        } else {
            trimmed.to_string()
        };

        if annotations.last() != Some(&annotation) {
            annotations.push(annotation);
        }
    }

    annotations
}

fn is_valid_diagnostic_detail_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.contains('|') || trimmed.starts_with("= note:") || trimmed.starts_with("= help:")
}

fn extract_lint_name(line: &str) -> Option<String> {
    if let Some(start) = line.find("#[warn(") {
        return extract_parenthesized_ident(&line[start + "#[warn(".len()..]);
    }
    if let Some(start) = line.find("#[deny(") {
        return extract_parenthesized_ident(&line[start + "#[deny(".len()..]);
    }
    if let Some(position) = line.find("clippy::") {
        return Some(
            line[position..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '-' | '_'))
                .collect(),
        );
    }
    if let Some(fragment) = line.split('#').next_back()
        && fragment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        && !fragment.is_empty()
    {
        return Some(fragment.to_string());
    }
    None
}

fn extract_parenthesized_ident(raw: &str) -> Option<String> {
    let end = raw.find(')')?;
    let ident = &raw[..end];
    if ident.is_empty() {
        return None;
    }
    Some(ident.to_string())
}

fn is_diagnostic_header(line: &str) -> bool {
    (line.starts_with("warning: ") || line.starts_with("error: ")) && !is_summary_line(line)
}

fn is_plain_diagnostic_header(line: &str) -> bool {
    let rest = line
        .strip_prefix("warning: ")
        .or_else(|| line.strip_prefix("error: "));
    let Some(rest) = rest else {
        return false;
    };
    let Some((candidate_path, _)) = rest.split_once(": ") else {
        return false;
    };

    let candidate_path = candidate_path.trim();
    !candidate_path.is_empty()
        && !candidate_path.contains(char::is_whitespace)
        && candidate_path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\'))
        && (candidate_path.contains('/')
            || candidate_path.contains('\\')
            || candidate_path.contains('.'))
}

fn is_summary_line(line: &str) -> bool {
    (line.starts_with("warning: `") && line.contains(" generated "))
        || (line.starts_with("warning: ") && line.contains(" warning emitted"))
        || (line.starts_with("warning: ") && line.contains(" warnings emitted"))
        || line.starts_with("error: could not compile `")
        || line.starts_with("error: aborting due to ")
        || line.starts_with("warning: build failed")
}

fn is_anchor_line(line: &str) -> bool {
    line.starts_with("--> ") || line.starts_with("::: ")
}

fn low_savings(raw: &str, compacted: &str) -> bool {
    let raw_chars = raw.chars().count();
    let compacted_chars = compacted.chars().count();
    raw_chars <= compacted_chars
        || raw_chars.saturating_sub(compacted_chars) < MIN_REQUIRED_SAVED_CHARS
        || compacted_chars * 10 >= raw_chars * 9
}

fn normalize_for_parsing(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek().is_some_and(|next| *next == '[') {
                chars.next();
                for marker in chars.by_ref() {
                    if ('@'..='~').contains(&marker) {
                        break;
                    }
                }
            }
            continue;
        }

        if ch != '\r' {
            normalized.push(ch);
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::CargoClippyFormatter;
    use crate::core::tools::middleware::{
        SEMANTIC_COMPACTION_CONFIDENCE_FLOOR, SemanticCompactionOutcome, SemanticFormatter,
    };
    use crate::core::tools::traits::ToolResultSemanticMetadata;

    #[test]
    fn compacts_mixed_warning_and_error_output() {
        let raw = r#"    Checking asteron v0.1.0 (/workspace)
warning: this import is unused
 --> src/lib.rs:1:5
  |
1 | use std::fmt::Debug;
  |     ^^^^^^^^^^^^^^^
  |
  = note: `#[warn(unused_imports)]` on by default

error: this expression creates a reference which is immediately dereferenced by the compiler
  --> src/main.rs:4:13
   |
4  |     let x = &String::from("hi");
   |             ^^^^^^^^^^^^^^^^^^^ help: change this to: `String::from("hi")`
   |
   = help: for further information visit https://rust-lang.github.io/rust-clippy/master/index.html#needless_borrow
   = note: `-D clippy::needless-borrow` implied by `-D warnings`

warning: `asteron` (lib) generated 1 warning
error: could not compile `asteron` (bin "asteron") due to 1 previous error; 1 warning emitted
"#;

        let formatter = CargoClippyFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        match outcome {
            SemanticCompactionOutcome::Compacted {
                content,
                confidence,
            } => {
                assert!(confidence > SEMANTIC_COMPACTION_CONFIDENCE_FLOOR);
                assert!(content.contains("warning: this import is unused"));
                assert!(content.contains("--> src/lib.rs:1:5"));
                assert!(content.contains("unused_imports"));
                assert!(content.contains("error: this expression creates a reference"));
                assert!(content.contains("--> src/main.rs:4:13"));
                assert!(content.contains("needless_borrow"));
                assert!(content.contains("warning: `asteron` (lib) generated 1 warning"));
                assert!(content.contains("error: could not compile `asteron` (bin \"asteron\")"));
                assert!(!content.contains("Checking asteron v0.1.0"));
            }
            other => panic!("expected compacted output, got {:?}", other),
        }
    }

    #[test]
    fn compacts_warning_heavy_output_but_keeps_count_summary() {
        let raw = r#"warning: variable does not need to be mutable
  --> src/lib.rs:10:9
   |
10 |     let mut value = 1;
   |         ----^^^^^
   |         |
   |         help: remove this `mut`
   |
   = note: `#[warn(unused_mut)]` on by default

warning: `asteron` (lib) generated 1 warning
"#;

        let formatter = CargoClippyFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        match outcome {
            SemanticCompactionOutcome::Compacted { content, .. } => {
                assert!(content.contains("warning: variable does not need to be mutable"));
                assert!(content.contains("--> src/lib.rs:10:9"));
                assert!(content.contains("unused_mut"));
                assert!(content.contains("warning: `asteron` (lib) generated 1 warning"));
            }
            other => panic!("expected compacted output, got {:?}", other),
        }
    }

    #[test]
    fn plain_cargo_warning_before_anchored_diagnostic_does_not_force_raw_fallback() {
        let mut raw = String::from(
            "warning: /workspace/Cargo.toml: unused manifest key: package.metadata.test\n",
        );
        raw.push_str(
            r#"warning: this import is unused
 --> src/lib.rs:1:5
  |
1 | use std::fmt::Debug;
  |     ^^^^^^^^^^^^^^^
"#,
        );
        for index in 0..80 {
            raw.push_str(&format!(
                "{:>2} | {}\n",
                index + 2,
                "very long code snippet line that should be reduced by semantic compaction"
                    .repeat(2)
            ));
        }
        raw.push_str(
            r#"  |
  = note: `#[warn(unused_imports)]` on by default

warning: `asteron` (lib) generated 1 warning
"#,
        );

        let formatter = CargoClippyFormatter;
        let outcome = formatter.compact(&raw, &ToolResultSemanticMetadata::default());

        assert_ne!(outcome, SemanticCompactionOutcome::FallbackRaw);
    }

    #[test]
    fn declines_compaction_when_savings_are_low() {
        let raw = r#"warning: field is never read
  --> src/lib.rs:3:5
   |
3  |     field: usize,
   |     ^^^^^
"#;

        let formatter = CargoClippyFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        assert_eq!(outcome, SemanticCompactionOutcome::Passthrough);
    }

    #[test]
    fn malformed_diagnostic_block_falls_back_to_raw() {
        let raw = r#"warning: this import is unused
not an anchor
"#;

        let formatter = CargoClippyFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        assert_eq!(outcome, SemanticCompactionOutcome::FallbackRaw);
    }

    #[test]
    fn stray_text_after_structural_cues_falls_back_to_raw() {
        let raw = r#"warning: this import is unused
 --> src/lib.rs:1:5
  |
1 | use std::fmt::Debug;
  |     ^^^^^^^^^^^^^^^
  |
  = note: `#[warn(unused_imports)]` on by default
mystery line that should not be ignored
warning: `asteron` (lib) generated 1 warning
"#;

        let formatter = CargoClippyFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        assert_eq!(outcome, SemanticCompactionOutcome::FallbackRaw);
    }
}
