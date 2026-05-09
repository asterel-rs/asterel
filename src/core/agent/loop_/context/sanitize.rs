use crate::security::external_content::{
    ExternalAction, PersistedExternalSummary, decide_action, detect_injection,
    sanitize_marker_collision, wrap_content,
};

pub(crate) fn sanitize_external_fragment_for_context(slot_key: &str, value: &str) -> String {
    if !PersistedExternalSummary::is_memory_summary_value(value) {
        return "[external payload omitted by replay-ban policy]".to_string();
    }

    let signals = detect_injection(value);
    let action = decide_action(&signals);
    match action {
        ExternalAction::Allow => wrap_content(slot_key, value),
        ExternalAction::Sanitize => {
            let sanitized = sanitize_marker_collision(value);
            wrap_content(slot_key, &sanitized)
        }
        ExternalAction::Block => {
            "[external summary blocked by policy during context replay]".to_string()
        }
    }
}
