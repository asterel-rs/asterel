//! Generic field-level validators for writeback payloads.
//!
//! Validates string fields, string arrays, unknown-field checks,
//! and list-item length constraints.

use serde_json::{Map, Value};

use super::profile_validators::contains_poison_pattern;
use super::types::WritebackVerdict;
use crate::contracts::strings::limits::MAX_LIST_ITEM_CHARS;
use crate::security::scrub::sanitize_api_error;

/// Result type for field-level validation (Ok or error message).
pub(super) type ValidationResult<T> = std::result::Result<T, String>;

/// Build a sanitized rejection verdict from a reason string.
pub(super) fn reject(reason: &str) -> WritebackVerdict {
    WritebackVerdict::Rejected {
        reason: sanitize_api_error(reason),
    }
}

/// Reject objects containing keys not in the allowed set.
pub(super) fn ensure_no_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    context: &str,
) -> ValidationResult<()> {
    for key in object.keys() {
        if !allowed.iter().any(|allowed_key| allowed_key == key) {
            return Err(format!("{context} contains unknown field: {key}"));
        }
    }
    Ok(())
}

/// Validate a required string field: present, non-empty, bounded,
/// and free of poison patterns.
pub(super) fn validate_str(
    object: &Map<String, Value>,
    field: &str,
    max_chars: usize,
    context: &str,
) -> ValidationResult<String> {
    let value = object
        .get(field)
        .ok_or_else(|| format!("{context}.{field} is required"))?;

    let raw = value
        .as_str()
        .ok_or_else(|| format!("{context}.{field} must be a string"))?;

    let sanitized = raw.trim().to_string();
    if sanitized.is_empty() {
        return Err(format!("{context}.{field} cannot be empty"));
    }
    // Byte length >= char count; skip the O(n) char count when byte length already fits.
    if sanitized.len() > max_chars && sanitized.chars().count() > max_chars {
        return Err(format!(
            "{context}.{field} exceeds max length ({max_chars})"
        ));
    }
    if contains_poison_pattern(&sanitized) {
        return Err(format!("{context}.{field} contains unsafe content pattern"));
    }

    Ok(sanitized)
}

/// Validate a required string array field: present, bounded count,
/// each item non-empty, bounded length, and poison-free.
pub(super) fn validate_str_array(
    object: &Map<String, Value>,
    field: &str,
    max_items: usize,
    context: &str,
) -> ValidationResult<Vec<String>> {
    let value = object
        .get(field)
        .ok_or_else(|| format!("{context}.{field} is required"))?;

    let list = value
        .as_array()
        .ok_or_else(|| format!("{context}.{field} must be an array"))?;

    if list.len() > max_items {
        return Err(format!("{context}.{field} exceeds max items ({max_items})"));
    }

    let mut out = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        let raw = item
            .as_str()
            .ok_or_else(|| format!("{context}.{field}[{index}] must be a string"))?;

        let sanitized = raw.trim().to_string();
        if sanitized.is_empty() {
            return Err(format!("{context}.{field}[{index}] cannot be empty"));
        }
        if sanitized.len() > MAX_LIST_ITEM_CHARS && sanitized.chars().count() > MAX_LIST_ITEM_CHARS
        {
            return Err(format!(
                "{context}.{field}[{index}] exceeds max length ({MAX_LIST_ITEM_CHARS})"
            ));
        }
        if contains_poison_pattern(&sanitized) {
            return Err(format!(
                "{context}.{field}[{index}] contains unsafe content pattern"
            ));
        }

        out.push(sanitized);
    }

    Ok(out)
}
