use std::sync::Arc;

use super::{SemanticCompactionOutcome, SemanticFormatter, SemanticFormatterRegistry};

mod browser_find;
mod browser_snapshot;
mod cargo_clippy;
mod cargo_test;
mod channel_history;
mod git_diff;
mod git_status;
mod memory_recall;
mod ripgrep;
mod web_scrape;
mod web_search;

pub(crate) use browser_find::BrowserFindFormatter;
pub(crate) use browser_snapshot::BrowserSnapshotFormatter;
pub(crate) use cargo_clippy::CargoClippyFormatter;
pub(crate) use cargo_test::CargoTestFormatter;
pub(crate) use channel_history::ChannelHistoryFormatter;
pub(crate) use git_diff::GitDiffFormatter;
pub(crate) use git_status::GitStatusFormatter;
pub(crate) use memory_recall::MemoryRecallFormatter;
pub(crate) use ripgrep::RipgrepFormatter;
pub(crate) use web_scrape::WebScrapeFormatter;
pub(crate) use web_search::WebSearchFormatter;

pub(crate) const EXAMPLE_LIMIT: usize = 5;
pub(crate) const COMPACTION_CONFIDENCE: f32 = 0.95;
const MIN_SAVED_CHARS: usize = 32;

pub(crate) fn builtin_formatter_registry() -> SemanticFormatterRegistry {
    SemanticFormatterRegistry::from_formatters([
        (
            "shell.git_status",
            Arc::new(GitStatusFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "shell.git_diff",
            Arc::new(GitDiffFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "shell.ripgrep",
            Arc::new(RipgrepFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "shell.cargo_test",
            Arc::new(CargoTestFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "shell.cargo_clippy",
            Arc::new(CargoClippyFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "browser.snapshot",
            Arc::new(BrowserSnapshotFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "browser.find",
            Arc::new(BrowserFindFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "web_search",
            Arc::new(WebSearchFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "web_scrape",
            Arc::new(WebScrapeFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "memory.recall",
            Arc::new(MemoryRecallFormatter) as Arc<dyn SemanticFormatter>,
        ),
        (
            "channel.history",
            Arc::new(ChannelHistoryFormatter) as Arc<dyn SemanticFormatter>,
        ),
    ])
}

pub(crate) fn finalize_compaction(raw: &str, compacted: String) -> SemanticCompactionOutcome {
    let raw_chars = raw.chars().count();
    let compacted_chars = compacted.chars().count();
    if compacted.trim().is_empty() {
        SemanticCompactionOutcome::FallbackRaw
    } else if compacted_chars + MIN_SAVED_CHARS >= raw_chars {
        SemanticCompactionOutcome::Passthrough
    } else {
        SemanticCompactionOutcome::Compacted {
            content: compacted,
            confidence: COMPACTION_CONFIDENCE,
        }
    }
}

pub(crate) fn truncate_preview(text: &str, limit: usize) -> String {
    let mut truncated = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= limit {
            truncated.push_str("...");
            break;
        }
        truncated.push(ch);
    }
    truncated
}
