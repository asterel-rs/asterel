//! Centralized numeric limits and allowlists for writeback payload contracts.

/// Max length for `state_header.current_objective`.
pub(crate) const MAX_CURRENT_OBJECTIVE_CHARS: usize = 280;
/// Max length for `state_header.recent_context_summary`.
pub(crate) const MAX_RECENT_CONTEXT_SUMMARY_CHARS: usize = 1200;
/// Max length of each list item in summary arrays.
pub(crate) const MAX_LIST_ITEM_CHARS: usize = 240;
/// Max number of `memory_append` entries accepted per writeback.
pub(crate) const MAX_MEMORY_APPEND_ITEMS: usize = 8;
/// Max length of each `memory_append` item value.
pub(crate) const MAX_MEMORY_APPEND_ITEM_CHARS: usize = 240;
/// Max number of `self_tasks` items accepted.
pub(crate) const MAX_SELF_TASKS: usize = 5;
/// Max title length for each self-task.
pub(crate) const MAX_SELF_TASK_TITLE_CHARS: usize = 120;
/// Max instructions length for each self-task.
pub(crate) const MAX_SELF_TASK_INSTRUCTIONS_CHARS: usize = 240;
/// Max allowed future expiry offset for self-tasks.
pub(crate) const MAX_SELF_TASK_EXPIRY_HOURS: i64 = 72;
/// Minimum style score accepted by validators.
pub(crate) const STYLE_SCORE_MIN: u8 = 0;
/// Maximum style score accepted by validators.
pub(crate) const STYLE_SCORE_MAX: u8 = 100;
/// Minimum style temperature accepted by validators.
pub(crate) const STYLE_TEMPERATURE_MIN: f64 = 0.0;
/// Maximum style temperature accepted by validators.
pub(crate) const STYLE_TEMPERATURE_MAX: f64 = 1.0;

/// Maximum count of `open_loops` entries.
pub(crate) const MAX_OPEN_LOOPS: usize = 7;
/// Maximum count of `next_actions` entries.
pub(crate) const MAX_NEXT_ACTIONS: usize = 5;
/// Maximum count of `commitments` entries.
pub(crate) const MAX_COMMITMENTS: usize = 5;

/// Allowed top-level JSON fields in writeback payloads.
pub(crate) const ALLOWED_TOP_LEVEL_FIELDS: [&str; 6] = [
    "state_header",
    "memory_append",
    "self_tasks",
    "style_profile",
    "memory_inferences",
    "user_inferences",
];
/// Top-level source fields explicitly rejected.
pub(crate) const FORBIDDEN_TOP_LEVEL_SOURCE_FIELDS: [&str; 2] = ["source_kind", "source_ref"];
/// Allowed keys under `state_header`.
pub(crate) const ALLOWED_STATE_HEADER_FIELDS: [&str; 8] = [
    "identity_principles_hash",
    "safety_posture",
    "current_objective",
    "open_loops",
    "next_actions",
    "commitments",
    "recent_context_summary",
    "last_updated_at",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlists_contain_required_fields() {
        assert!(ALLOWED_TOP_LEVEL_FIELDS.contains(&"state_header"));
        assert!(ALLOWED_STATE_HEADER_FIELDS.contains(&"identity_principles_hash"));
    }

    #[test]
    fn allowed_top_level_fields_has_no_duplicates() {
        let set: std::collections::HashSet<&&str> = ALLOWED_TOP_LEVEL_FIELDS.iter().collect();
        assert_eq!(set.len(), ALLOWED_TOP_LEVEL_FIELDS.len());
    }

    #[test]
    fn allowed_state_header_fields_has_no_duplicates() {
        let set: std::collections::HashSet<&&str> = ALLOWED_STATE_HEADER_FIELDS.iter().collect();
        assert_eq!(set.len(), ALLOWED_STATE_HEADER_FIELDS.len());
    }

    #[test]
    fn forbidden_source_fields_has_no_duplicates() {
        let set: std::collections::HashSet<&&str> =
            FORBIDDEN_TOP_LEVEL_SOURCE_FIELDS.iter().collect();
        assert_eq!(set.len(), FORBIDDEN_TOP_LEVEL_SOURCE_FIELDS.len());
    }

    #[test]
    fn allowed_top_level_fields_exact_membership() {
        let expected = [
            "state_header",
            "memory_append",
            "self_tasks",
            "style_profile",
            "memory_inferences",
            "user_inferences",
        ];
        assert_eq!(ALLOWED_TOP_LEVEL_FIELDS.len(), expected.len());
        for field in &expected {
            assert!(
                ALLOWED_TOP_LEVEL_FIELDS.contains(field),
                "missing expected field: {field}"
            );
        }
    }

    #[test]
    fn allowed_state_header_fields_exact_membership() {
        let expected = [
            "identity_principles_hash",
            "safety_posture",
            "current_objective",
            "open_loops",
            "next_actions",
            "commitments",
            "recent_context_summary",
            "last_updated_at",
        ];
        assert_eq!(ALLOWED_STATE_HEADER_FIELDS.len(), expected.len());
        for field in &expected {
            assert!(
                ALLOWED_STATE_HEADER_FIELDS.contains(field),
                "missing expected field: {field}"
            );
        }
    }

    #[test]
    fn forbidden_source_fields_exact_membership() {
        let expected = ["source_kind", "source_ref"];
        assert_eq!(FORBIDDEN_TOP_LEVEL_SOURCE_FIELDS.len(), expected.len());
        for field in &expected {
            assert!(
                FORBIDDEN_TOP_LEVEL_SOURCE_FIELDS.contains(field),
                "missing expected field: {field}"
            );
        }
    }
}
