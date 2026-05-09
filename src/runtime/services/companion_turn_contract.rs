//! Companion-turn transport contract semantics helpers.
//!
//! Integration tests use this module to ensure gateway HTTP / gateway WS /
//! channel-handler request assembly stay aligned on the contract's semantic
//! fields even when transport-local richness (history depth, streaming shape)
//! differs.

/// Fixture that mirrors one `CompanionTransportTurnRequest` assembly path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionTurnContractFixture {
    pub transport_route: String,
    pub session_surface: Option<String>,
    pub user_message: String,
    pub policy_tenant_id: Option<String>,
    pub session_owner_scope: Option<String>,
    pub history_session_key: Option<String>,
    pub history_channel_name: String,
    pub history_richness_tokens: usize,
    pub channel_context_hint: Option<String>,
    pub image_content_present: bool,
}

/// Semantic fields that must remain transport-invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionTurnContractSemantics {
    pub mode: String,
    pub pickup: String,
    pub behavior: String,
    pub reply_shape: String,
    pub exposure_plan: String,
}

/// Fields intentionally excluded from semantic-equivalence checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionTurnContractExclusionRules {
    pub excluded_fields: Vec<&'static str>,
}

impl Default for CompanionTurnContractExclusionRules {
    fn default() -> Self {
        Self {
            excluded_fields: vec!["history_richness_tokens", "transport_route"],
        }
    }
}

#[must_use]
pub fn derive_contract_semantics(
    fixture: &CompanionTurnContractFixture,
) -> CompanionTurnContractSemantics {
    let mode = if fixture.session_surface.is_some() {
        "interactive_companion_turn"
    } else {
        "stateless_companion_turn"
    }
    .to_string();

    let pickup = if fixture.channel_context_hint.as_deref().is_some_and(|hint| {
        let normalized = hint.to_ascii_lowercase();
        normalized.contains("ambient")
            || normalized.contains("thread continuation")
            || normalized.contains("passive")
    }) {
        "ambient"
    } else {
        "direct"
    }
    .to_string();

    let behavior = if fixture.policy_tenant_id.is_some() || fixture.session_owner_scope.is_some() {
        "tenant_scoped_companion"
    } else {
        "global_companion"
    }
    .to_string();

    let reply_shape = if fixture.image_content_present {
        "multimodal_text_first"
    } else {
        "text_first"
    }
    .to_string();

    let exposure_plan = if fixture.history_session_key.is_some() {
        "session_scoped_history"
    } else {
        "ephemeral_history"
    }
    .to_string();

    CompanionTurnContractSemantics {
        mode,
        pickup,
        behavior,
        reply_shape,
        exposure_plan,
    }
}

#[must_use]
pub fn semantics_match_with_exclusions(
    left: &CompanionTurnContractFixture,
    right: &CompanionTurnContractFixture,
    _rules: &CompanionTurnContractExclusionRules,
) -> bool {
    derive_contract_semantics(left) == derive_contract_semantics(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_fixture(route: &str) -> CompanionTurnContractFixture {
        CompanionTurnContractFixture {
            transport_route: route.to_string(),
            session_surface: Some("gateway_ws".to_string()),
            user_message: "same message".to_string(),
            policy_tenant_id: Some("tenant-a".to_string()),
            session_owner_scope: Some("tenant::tenant-a::user-1".to_string()),
            history_session_key: Some("session-1".to_string()),
            history_channel_name: "gateway_ws".to_string(),
            history_richness_tokens: 2048,
            channel_context_hint: None,
            image_content_present: false,
        }
    }

    #[test]
    fn semantics_ignore_history_richness_and_route_name() {
        let mut left = base_fixture("gateway_http");
        let mut right = base_fixture("channel_handler");
        left.history_richness_tokens = 1024;
        right.history_richness_tokens = 4096;

        let rules = CompanionTurnContractExclusionRules::default();
        assert!(semantics_match_with_exclusions(&left, &right, &rules));
    }
}
