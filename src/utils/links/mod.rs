//! Re-exports for the link detection and extraction subsystem.

pub(crate) mod detector;
#[cfg(feature = "link-extraction")]
pub(crate) mod extractor;
pub(crate) mod types;
