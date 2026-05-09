//! External content sanitization and prompt-injection detection.
//!
//! Wraps untrusted input in marker tags, detects injection signals,
//! and decides allow/sanitize/block actions before content reaches
//! the LLM context.

mod detect;
pub mod normalize;
pub(crate) mod patterns;
mod prepare;
mod tables;
mod trust;
mod types;

pub use detect::{decide_action, decide_external_action_with_classifier, detect_injection};
pub use normalize::normalize_detection;
pub(crate) use patterns::POISON_PATTERNS;
pub use prepare::{
    prepare_content, prepare_content_with_trust, prepare_external_content_with_classifier,
    sanitize_marker_collision, summarize_for_persistence, wrap_content,
};
pub use types::{
    ExternalAction, InjectionSignals, PersistedExternalSummary, PreparedExternalContent,
};
