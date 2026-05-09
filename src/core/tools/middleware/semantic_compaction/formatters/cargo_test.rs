use crate::core::tools::traits::ToolResultSemanticMetadata;

use super::super::{SemanticCompactionOutcome, SemanticFormatter};

const CARGO_TEST_CONFIDENCE: f32 = 0.98;
const MIN_REQUIRED_SAVED_CHARS: usize = 128;

#[derive(Debug, Default)]
pub(crate) struct CargoTestFormatter;

impl SemanticFormatter for CargoTestFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let normalized = normalize_for_parsing(raw);
        match parse_cargo_test_report(&normalized) {
            ParseResult::Unsupported => SemanticCompactionOutcome::Passthrough,
            ParseResult::Malformed => SemanticCompactionOutcome::FallbackRaw,
            ParseResult::Parsed(report) => {
                let compacted = report.render();
                if compacted.is_empty() || low_savings(&normalized, &compacted) {
                    return SemanticCompactionOutcome::Passthrough;
                }

                SemanticCompactionOutcome::Compacted {
                    content: compacted,
                    confidence: CARGO_TEST_CONFIDENCE,
                }
            }
        }
    }
}

#[derive(Debug)]
enum ParseResult {
    Parsed(CargoTestReport),
    Unsupported,
    Malformed,
}

#[derive(Debug, Default)]
struct CargoTestReport {
    events: Vec<CargoTestEvent>,
}

impl CargoTestReport {
    fn render(&self) -> String {
        self.events
            .iter()
            .map(CargoTestEvent::render)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Debug)]
enum CargoTestEvent {
    ContextLine(String),
    SummaryLine(String),
    FailureNames(Vec<String>),
    FailureBlocks(Vec<String>),
    TerminalLine(String),
}

impl CargoTestEvent {
    fn render(&self) -> String {
        match self {
            Self::ContextLine(line) | Self::SummaryLine(line) | Self::TerminalLine(line) => {
                line.clone()
            }
            Self::FailureNames(names) => {
                let mut rendered = String::from("failures:\n");
                for name in names {
                    rendered.push_str("    ");
                    rendered.push_str(name);
                    rendered.push('\n');
                }
                rendered.trim_end().to_string()
            }
            Self::FailureBlocks(blocks) => {
                let mut rendered = String::from("failures:\n\n");
                rendered.push_str(&blocks.join("\n\n"));
                rendered
            }
        }
    }
}

fn parse_cargo_test_report(raw: &str) -> ParseResult {
    let lines = raw.lines().collect::<Vec<_>>();
    let mut report = CargoTestReport::default();
    let mut saw_structural_cue = false;
    let mut saw_summary = false;
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            index += 1;
            continue;
        }

        if is_context_line(trimmed) {
            report
                .events
                .push(CargoTestEvent::ContextLine(trimmed.to_string()));
            saw_structural_cue = true;
            index += 1;
            continue;
        }

        if is_summary_line(trimmed) {
            report
                .events
                .push(CargoTestEvent::SummaryLine(trimmed.to_string()));
            saw_structural_cue = true;
            saw_summary = true;
            index += 1;
            continue;
        }

        if line == "failures:" {
            match parse_failure_section(&lines, index) {
                Some((event, next_index)) => {
                    report.events.push(event);
                    saw_structural_cue = true;
                    index = next_index;
                }
                None => return ParseResult::Malformed,
            }
            continue;
        }

        if line.starts_with("---- ") {
            return ParseResult::Malformed;
        }

        if is_terminal_error_line(trimmed) {
            report
                .events
                .push(CargoTestEvent::TerminalLine(trimmed.to_string()));
            index += 1;
            continue;
        }

        if is_test_case_line(trimmed) {
            index += 1;
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
    if !saw_summary {
        return ParseResult::Malformed;
    }

    ParseResult::Parsed(report)
}

fn parse_failure_section(lines: &[&str], start_index: usize) -> Option<(CargoTestEvent, usize)> {
    let mut index = start_index + 1;
    while index < lines.len() && lines[index].trim().is_empty() {
        index += 1;
    }

    let next_line = *lines.get(index)?;
    if next_line.starts_with("---- ") {
        let mut blocks = Vec::new();

        while index < lines.len() {
            while index < lines.len() && lines[index].trim().is_empty() {
                index += 1;
            }
            if index >= lines.len() {
                break;
            }

            let line = lines[index];
            let trimmed = line.trim_start();
            if is_summary_line(trimmed) || is_context_line(trimmed) || line == "failures:" {
                break;
            }
            if !line.starts_with("---- ") {
                return None;
            }

            let mut block_lines = vec![line.trim_end().to_string()];
            let mut saw_body = false;
            index += 1;

            while index < lines.len() {
                let current = lines[index];
                let current_trimmed = current.trim_start();
                if current.starts_with("---- ")
                    || current == "failures:"
                    || is_summary_line(current_trimmed)
                    || is_context_line(current_trimmed)
                    || is_terminal_error_line(current_trimmed)
                {
                    break;
                }
                if !current.trim().is_empty() {
                    saw_body = true;
                }
                block_lines.push(current.trim_end().to_string());
                index += 1;
            }

            while block_lines.last().is_some_and(String::is_empty) {
                block_lines.pop();
            }
            if !saw_body {
                return None;
            }
            blocks.push(block_lines.join("\n"));

            if index >= lines.len()
                || lines[index] == "failures:"
                || is_summary_line(lines[index].trim_start())
                || is_context_line(lines[index].trim_start())
                || is_terminal_error_line(lines[index].trim_start())
            {
                break;
            }
        }

        if blocks.is_empty() {
            return None;
        }
        return Some((CargoTestEvent::FailureBlocks(blocks), index));
    }

    if !is_failure_name_line(next_line) {
        return None;
    }

    let mut names = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            break;
        }
        if !is_failure_name_line(line) {
            return None;
        }
        names.push(line.trim().to_string());
        index += 1;
    }

    if names.is_empty() {
        return None;
    }

    Some((CargoTestEvent::FailureNames(names), index))
}

fn is_context_line(line: &str) -> bool {
    line.starts_with("Running ") || line.starts_with("Doc-tests ") || line.starts_with("running ")
}

fn is_summary_line(line: &str) -> bool {
    line.starts_with("test result: ")
}

fn is_terminal_error_line(line: &str) -> bool {
    line.starts_with("error: test failed") || line.starts_with("error: doctest failed")
}

fn is_failure_name_line(line: &str) -> bool {
    (line.starts_with("    ") || line.starts_with('\t')) && !line.trim().is_empty()
}

fn is_test_case_line(line: &str) -> bool {
    line.starts_with("test ")
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
    use super::CargoTestFormatter;
    use crate::core::tools::middleware::{
        SEMANTIC_COMPACTION_CONFIDENCE_FLOOR, SemanticCompactionOutcome, SemanticFormatter,
    };
    use crate::core::tools::traits::ToolResultSemanticMetadata;

    #[test]
    fn compacts_success_output_to_running_and_summary_lines() {
        let raw = r#"     Running unittests src/lib.rs (target/debug/deps/asteron-123456)
running 4 tests
test tests::alpha ... ok
test tests::beta ... ok
test tests::gamma ... ok
test tests::delta ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

   Doc-tests asteron

running 2 tests
test src/lib.rs - alpha (line 10) ... ok
test src/lib.rs - beta (line 20) ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;

        let formatter = CargoTestFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        match outcome {
            SemanticCompactionOutcome::Compacted {
                content,
                confidence,
            } => {
                assert!(confidence > SEMANTIC_COMPACTION_CONFIDENCE_FLOOR);
                assert!(content.contains("Running unittests src/lib.rs"));
                assert!(content.contains("running 4 tests"));
                assert!(content.contains("test result: ok. 4 passed; 0 failed;"));
                assert!(content.contains("Doc-tests asteron"));
                assert!(content.contains("test result: ok. 2 passed; 0 failed;"));
                assert!(!content.contains("test tests::alpha ... ok"));
            }
            other => panic!("expected compacted output, got {:?}", other),
        }
    }

    #[test]
    fn compacts_failure_output_preserving_names_blocks_and_summary() {
        let raw = r#"running 5 tests
test tests::ok_alpha ... ok
test tests::ok_beta ... ok
test tests::broken_alpha ... FAILED
test tests::broken_beta ... FAILED
test tests::ok_gamma ... ok

failures:

---- tests::broken_alpha stdout ----
thread 'tests::broken_alpha' panicked at src/lib.rs:10:5:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- tests::broken_beta stdout ----
thread 'tests::broken_beta' panicked at src/lib.rs:22:5:
explicit panic

failures:
    tests::broken_alpha
    tests::broken_beta

test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

error: test failed, to rerun pass `--lib`
"#;

        let formatter = CargoTestFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        match outcome {
            SemanticCompactionOutcome::Compacted {
                content,
                confidence,
            } => {
                assert!(confidence > SEMANTIC_COMPACTION_CONFIDENCE_FLOOR);
                assert!(content.contains("failures:"));
                assert!(content.contains("tests::broken_alpha"));
                assert!(content.contains("tests::broken_beta"));
                assert!(content.contains("---- tests::broken_alpha stdout ----"));
                assert!(content.contains("assertion `left == right` failed"));
                assert!(content.contains("---- tests::broken_beta stdout ----"));
                assert!(content.contains("explicit panic"));
                assert!(content.contains("test result: FAILED. 3 passed; 2 failed;"));
                assert!(content.contains("error: test failed, to rerun pass `--lib`"));
                assert!(!content.contains("test tests::ok_alpha ... ok"));
            }
            other => panic!("expected compacted output, got {:?}", other),
        }
    }

    #[test]
    fn declines_compaction_when_savings_are_low() {
        let raw = r#"running 1 test

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
"#;

        let formatter = CargoTestFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        assert_eq!(outcome, SemanticCompactionOutcome::Passthrough);
    }

    #[test]
    fn malformed_failure_section_falls_back_to_raw() {
        let raw = r#"running 2 tests
failures:
broken payload without structural cues
"#;

        let formatter = CargoTestFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        assert_eq!(outcome, SemanticCompactionOutcome::FallbackRaw);
    }

    #[test]
    fn stray_text_after_structural_cues_falls_back_to_raw() {
        let raw = r#"running 2 tests
test tests::alpha ... ok
mystery line that should not be silently ignored
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
"#;

        let formatter = CargoTestFormatter;
        let outcome = formatter.compact(raw, &ToolResultSemanticMetadata::default());

        assert_eq!(outcome, SemanticCompactionOutcome::FallbackRaw);
    }
}
