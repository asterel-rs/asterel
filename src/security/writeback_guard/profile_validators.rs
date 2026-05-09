//! Profile-level validators for state headers and style profiles.
//!
//! Validates immutable invariants, state header fields, style
//! profile scores/temperature, timestamps, and poison patterns.

use chrono::{DateTime, FixedOffset};
use serde_json::{Map, Value};

use super::field_validators::{
    ValidationResult, ensure_no_unknown_fields, validate_str, validate_str_array,
};
use super::types::{ImmutableStateHeader, StateHeaderWriteback, StyleWriteback};
use crate::contracts::strings::limits::{
    ALLOWED_STATE_HEADER_FIELDS, MAX_COMMITMENTS, MAX_CURRENT_OBJECTIVE_CHARS, MAX_NEXT_ACTIONS,
    MAX_OPEN_LOOPS, MAX_RECENT_CONTEXT_SUMMARY_CHARS, STYLE_SCORE_MAX, STYLE_SCORE_MIN,
    STYLE_TEMPERATURE_MAX, STYLE_TEMPERATURE_MIN,
};
use crate::contracts::strings::verdicts::{
    IMMUTABLE_MISMATCH_IDENTITY_HASH, IMMUTABLE_MISMATCH_SAFETY_POSTURE,
    PAYLOAD_LAST_UPDATED_AT_MUST_BE_RFC3339, PAYLOAD_STATE_HEADER_IDENTITY_HASH_MUST_BE_STRING,
    PAYLOAD_STATE_HEADER_SAFETY_POSTURE_MUST_BE_STRING,
    PAYLOAD_STYLE_PROFILE_FORMALITY_MUST_BE_INTEGER, PAYLOAD_STYLE_PROFILE_FORMALITY_OUT_OF_RANGE,
    PAYLOAD_STYLE_PROFILE_MUST_BE_OBJECT, PAYLOAD_STYLE_PROFILE_TEMPERATURE_MUST_BE_NUMBER,
    PAYLOAD_STYLE_PROFILE_TEMPERATURE_OUT_OF_RANGE,
    PAYLOAD_STYLE_PROFILE_VERBOSITY_MUST_BE_INTEGER, PAYLOAD_STYLE_PROFILE_VERBOSITY_OUT_OF_RANGE,
};
use crate::security::external_content::{POISON_PATTERNS, normalize_detection};

/// Validate an optional `style_profile` sub-object if present.
pub(super) fn validate_style_profile(
    object: &Map<String, Value>,
) -> ValidationResult<Option<StyleWriteback>> {
    let Some(raw_style_profile) = object.get("style_profile") else {
        return Ok(None);
    };

    let style_profile = raw_style_profile
        .as_object()
        .ok_or_else(|| PAYLOAD_STYLE_PROFILE_MUST_BE_OBJECT.to_string())?;
    ensure_no_unknown_fields(
        style_profile,
        &["formality", "verbosity", "temperature"],
        "payload.style_profile",
    )?;

    let formality = style_profile
        .get("formality")
        .and_then(Value::as_u64)
        .ok_or_else(|| PAYLOAD_STYLE_PROFILE_FORMALITY_MUST_BE_INTEGER.to_string())?;
    if !(u64::from(STYLE_SCORE_MIN)..=u64::from(STYLE_SCORE_MAX)).contains(&formality) {
        return Err(format!(
            "{PAYLOAD_STYLE_PROFILE_FORMALITY_OUT_OF_RANGE} [{STYLE_SCORE_MIN}, {STYLE_SCORE_MAX}]"
        ));
    }

    let verbosity = style_profile
        .get("verbosity")
        .and_then(Value::as_u64)
        .ok_or_else(|| PAYLOAD_STYLE_PROFILE_VERBOSITY_MUST_BE_INTEGER.to_string())?;
    if !(u64::from(STYLE_SCORE_MIN)..=u64::from(STYLE_SCORE_MAX)).contains(&verbosity) {
        return Err(format!(
            "{PAYLOAD_STYLE_PROFILE_VERBOSITY_OUT_OF_RANGE} [{STYLE_SCORE_MIN}, {STYLE_SCORE_MAX}]"
        ));
    }

    let temperature = style_profile
        .get("temperature")
        .and_then(Value::as_f64)
        .ok_or_else(|| PAYLOAD_STYLE_PROFILE_TEMPERATURE_MUST_BE_NUMBER.to_string())?;
    if !(STYLE_TEMPERATURE_MIN..=STYLE_TEMPERATURE_MAX).contains(&temperature) {
        return Err(format!(
            "{PAYLOAD_STYLE_PROFILE_TEMPERATURE_OUT_OF_RANGE} [{STYLE_TEMPERATURE_MIN}, {STYLE_TEMPERATURE_MAX}]"
        ));
    }

    Ok(Some(StyleWriteback {
        formality: u8::try_from(formality)
            .map_err(|_| PAYLOAD_STYLE_PROFILE_FORMALITY_OUT_OF_RANGE.to_string())?,
        verbosity: u8::try_from(verbosity)
            .map_err(|_| PAYLOAD_STYLE_PROFILE_VERBOSITY_OUT_OF_RANGE.to_string())?,
        temperature,
    }))
}

/// Check if normalized text matches any poison injection pattern.
pub(super) fn contains_poison_pattern(input: &str) -> bool {
    let normalized = normalize_detection(input);
    POISON_PATTERNS
        .iter()
        .any(|pattern| normalized.contains(pattern))
}

/// Validate that `last_updated_at` is a valid RFC 3339 timestamp.
pub(super) fn validate_last_updated_at(value: &str) -> ValidationResult<()> {
    DateTime::<FixedOffset>::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| PAYLOAD_LAST_UPDATED_AT_MUST_BE_RFC3339.to_string())
}

/// Validate the full state header: immutable fields must match,
/// mutable fields must be present and within bounds.
pub(super) fn validate_state_header(
    state_header: &Map<String, Value>,
    immutable: &ImmutableStateHeader,
) -> ValidationResult<StateHeaderWriteback> {
    ensure_no_unknown_fields(
        state_header,
        &ALLOWED_STATE_HEADER_FIELDS,
        "payload.state_header",
    )?;

    let Some(identity_hash) = state_header
        .get("identity_principles_hash")
        .and_then(Value::as_str)
    else {
        return Err(PAYLOAD_STATE_HEADER_IDENTITY_HASH_MUST_BE_STRING.to_string());
    };
    if identity_hash != immutable.identity_principles_hash {
        return Err(IMMUTABLE_MISMATCH_IDENTITY_HASH.to_string());
    }

    let Some(safety_posture) = state_header.get("safety_posture").and_then(Value::as_str) else {
        return Err(PAYLOAD_STATE_HEADER_SAFETY_POSTURE_MUST_BE_STRING.to_string());
    };
    if safety_posture != immutable.safety_posture {
        return Err(IMMUTABLE_MISMATCH_SAFETY_POSTURE.to_string());
    }

    let current_objective = validate_str(
        state_header,
        "current_objective",
        MAX_CURRENT_OBJECTIVE_CHARS,
        "payload.state_header",
    )?;
    let open_loops = validate_str_array(
        state_header,
        "open_loops",
        MAX_OPEN_LOOPS,
        "payload.state_header",
    )?;
    let next_actions = validate_str_array(
        state_header,
        "next_actions",
        MAX_NEXT_ACTIONS,
        "payload.state_header",
    )?;
    let commitments = validate_str_array(
        state_header,
        "commitments",
        MAX_COMMITMENTS,
        "payload.state_header",
    )?;
    let recent_context_summary = validate_str(
        state_header,
        "recent_context_summary",
        MAX_RECENT_CONTEXT_SUMMARY_CHARS,
        "payload.state_header",
    )?;
    let last_updated_at =
        validate_str(state_header, "last_updated_at", 64, "payload.state_header")?;
    validate_last_updated_at(&last_updated_at)?;

    Ok(StateHeaderWriteback {
        current_objective,
        open_loops,
        next_actions,
        commitments,
        recent_context_summary,
        last_updated_at,
    })
}
