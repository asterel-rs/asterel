//! Shared utility modules: filesystem paths, encoding, HTTP helpers, link parsing, and text ops.
//!
//! - **[`dirs`]** — Platform-aware data/config/cache directory resolution.
//! - **[`encoding`]** — Base64 and hex encode/decode helpers.
//! - **[`http`]** — Lightweight HTTP client wrappers (retry, timeout, header injection).
//! - **[`links`]** — URL extraction and normalisation from free-form text.
//! - **[`postgres`]** — Connection pool helpers and query utilities shared across subsystems.
//! - **[`text`]** — String manipulation: truncation, reasoning-tag stripping, token estimation.

pub(crate) mod dirs;
pub(crate) mod encoding;
pub(crate) mod http;
pub(crate) mod links;
pub(crate) mod postgres;
#[cfg(test)]
pub(crate) mod test_env;
pub(crate) mod text;

#[must_use]
pub(crate) fn truncate_u128_to_u64(value: u128) -> u64 {
    let masked = value & u128::from(u64::MAX);
    u64::try_from(masked).unwrap_or(u64::MAX)
}
