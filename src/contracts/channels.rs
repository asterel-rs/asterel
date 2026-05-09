//! Channel capability contracts shared between `core` and `transport`.

use std::fmt::Write as _;

use crate::config::CompanionBehaviorConfig;

/// Persisted/runtime companion management settings.
///
/// Shared between `runtime/services` (L4) and `transport/gateway` (L5) so that
/// the persistence service does not need to import from the gateway surface layer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayCompanionSettings {
    pub caption_retention_limit: usize,
    #[serde(default)]
    pub behavior: CompanionBehaviorConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl Default for GatewayCompanionSettings {
    fn default() -> Self {
        Self {
            caption_retention_limit: 256,
            behavior: CompanionBehaviorConfig::default(),
            config: None,
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct ChannelCapabilities {
    pub can_edit_message: bool,
    pub can_delete_message: bool,
    pub can_send_media: bool,
    pub can_send_embed: bool,
    pub can_send_typing: bool,
    pub max_message_length: usize,
    pub can_create_thread: bool,
    pub can_manage_thread_members: bool,
    pub can_add_reaction: bool,
    pub can_read_reactions: bool,
    pub can_send_buttons: bool,
    pub can_send_select_menu: bool,
    pub can_send_modal: bool,
    pub can_fetch_history: bool,
    pub can_receive_reactions: bool,
    pub can_receive_edits: bool,
    pub can_receive_deletes: bool,
    pub can_receive_typing: bool,
    /// Acknowledgement deadline in milliseconds.  0 = no deadline.
    pub ack_deadline_ms: u64,
}

impl Default for ChannelCapabilities {
    fn default() -> Self {
        Self {
            can_edit_message: false,
            can_delete_message: false,
            can_send_media: false,
            can_send_embed: false,
            can_send_typing: false,
            max_message_length: usize::MAX,
            can_create_thread: false,
            can_manage_thread_members: false,
            can_add_reaction: false,
            can_read_reactions: false,
            can_send_buttons: false,
            can_send_select_menu: false,
            can_send_modal: false,
            can_fetch_history: false,
            can_receive_reactions: false,
            can_receive_edits: false,
            can_receive_deletes: false,
            can_receive_typing: false,
            ack_deadline_ms: 0,
        }
    }
}

/// Companion behavior constraints per surface (§6.4.B).
///
/// Unlike `ChannelCapabilities` (which describes what the transport *can* do),
/// this describes how the companion *should* behave on a given surface.
/// Same persona, different realization.
#[derive(Debug, Clone)]
pub struct SurfaceRealizationPolicy {
    /// Target response length in characters. 0 = no constraint.
    pub target_length: usize,
    /// Maximum warmth/intimacy level on this surface (0.0 = formal, 1.0 = full).
    pub intimacy_cap: f32,
    /// Maximum memory exposure on this surface (0.0 = none, 1.0 = full).
    pub memory_exposure_cap: f32,
    /// Expected response density ("brief", "normal", "expanded").
    pub default_density: &'static str,
    /// Whether the surface is public (affects exposure and tone).
    pub is_public: bool,
}

impl Default for SurfaceRealizationPolicy {
    fn default() -> Self {
        Self {
            target_length: 0,
            intimacy_cap: 0.7,
            memory_exposure_cap: 0.5,
            default_density: "normal",
            is_public: false,
        }
    }
}

impl SurfaceRealizationPolicy {
    /// Discord public channel: short, low intimacy, low memory exposure.
    #[must_use]
    pub fn discord_public() -> Self {
        Self {
            target_length: 400,
            intimacy_cap: 0.3,
            memory_exposure_cap: 0.1,
            default_density: "brief",
            is_public: true,
        }
    }

    /// Conservative default for channels that have not declared a narrower
    /// private/direct-message surface. This is fail-closed: public-safe until a
    /// channel opts into a more permissive private policy.
    #[must_use]
    pub fn public_channel_default() -> Self {
        Self::discord_public()
    }

    /// Discord DM: moderate length, higher intimacy.
    #[must_use]
    pub fn discord_dm() -> Self {
        Self {
            target_length: 800,
            intimacy_cap: 0.7,
            memory_exposure_cap: 0.5,
            default_density: "normal",
            is_public: false,
        }
    }

    /// CLI: longer responses allowed, full intimacy.
    #[must_use]
    pub fn cli() -> Self {
        Self {
            target_length: 0,
            intimacy_cap: 1.0,
            memory_exposure_cap: 1.0,
            default_density: "normal",
            is_public: false,
        }
    }

    /// X (Twitter) public post/reply: very short, minimal intimacy, no personal history.
    #[must_use]
    pub fn twitter_public() -> Self {
        Self {
            target_length: 280,
            intimacy_cap: 0.2,
            memory_exposure_cap: 0.05,
            default_density: "brief",
            is_public: true,
        }
    }

    /// X (Twitter) DM: moderate length, higher intimacy.
    #[must_use]
    pub fn twitter_dm() -> Self {
        Self {
            target_length: 560,
            intimacy_cap: 0.6,
            memory_exposure_cap: 0.4,
            default_density: "normal",
            is_public: false,
        }
    }

    /// Gateway WebSocket: moderate defaults.
    #[must_use]
    pub fn gateway_ws() -> Self {
        Self {
            target_length: 600,
            intimacy_cap: 0.6,
            memory_exposure_cap: 0.4,
            default_density: "normal",
            is_public: false,
        }
    }

    /// Gateway HTTP: private operator/API surface with standard response shape.
    #[must_use]
    pub fn gateway_http() -> Self {
        Self::gateway_ws()
    }

    /// Render as a prompt guidance section.
    #[must_use]
    pub fn render_guidance(&self) -> String {
        let mut body = String::new();
        if self.target_length > 0 {
            let _ = writeln!(body, "Target length: ~{} chars", self.target_length);
        }
        if self.is_public {
            body.push_str("Context: public (keep personal details minimal)\n");
        }
        if self.intimacy_cap < 0.5 {
            body.push_str("Warmth: keep distance, stay professional\n");
        }
        if self.memory_exposure_cap < 0.3 {
            body.push_str("Memory: do not reference personal history\n");
        }
        if body.is_empty() {
            return String::new();
        }
        format!("[Surface Realization]\n{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_all_flags_disabled_and_unbounded_length() {
        let caps = ChannelCapabilities::default();
        assert!(!caps.can_edit_message);
        assert!(!caps.can_delete_message);
        assert!(!caps.can_send_media);
        assert!(!caps.can_send_embed);
        assert!(!caps.can_send_typing);
        assert_eq!(caps.max_message_length, usize::MAX);
        assert!(!caps.can_create_thread);
        assert!(!caps.can_manage_thread_members);
        assert!(!caps.can_add_reaction);
        assert!(!caps.can_read_reactions);
        assert!(!caps.can_send_buttons);
        assert!(!caps.can_send_select_menu);
        assert!(!caps.can_send_modal);
        assert!(!caps.can_fetch_history);
        assert!(!caps.can_receive_reactions);
        assert!(!caps.can_receive_edits);
        assert!(!caps.can_receive_deletes);
        assert!(!caps.can_receive_typing);
        assert_eq!(caps.ack_deadline_ms, 0);
    }

    #[test]
    fn discord_public_policy_constrains_length_and_intimacy() {
        let policy = SurfaceRealizationPolicy::discord_public();
        assert_eq!(policy.target_length, 400);
        assert!(policy.intimacy_cap < 0.5);
        assert!(policy.is_public);
        let guidance = policy.render_guidance();
        assert!(guidance.contains("public"));
        assert!(guidance.contains("400"));
    }

    #[test]
    fn public_channel_default_is_conservative() {
        let policy = SurfaceRealizationPolicy::public_channel_default();
        assert!(policy.is_public);
        assert!(policy.intimacy_cap < 0.5);
        assert!(policy.memory_exposure_cap < 0.3);
    }

    #[test]
    fn cli_policy_has_no_length_constraint() {
        let policy = SurfaceRealizationPolicy::cli();
        assert_eq!(policy.target_length, 0);
        assert!((policy.intimacy_cap - 1.0).abs() < f32::EPSILON);
        let guidance = policy.render_guidance();
        assert!(
            guidance.is_empty(),
            "CLI should have no constraints to render"
        );
    }

    #[test]
    fn discord_dm_is_not_public() {
        let policy = SurfaceRealizationPolicy::discord_dm();
        assert!(!policy.is_public);
        assert!(policy.intimacy_cap > 0.5);
    }

    #[test]
    fn gateway_http_policy_is_private() {
        let policy = SurfaceRealizationPolicy::gateway_http();
        assert!(!policy.is_public);
        assert!(policy.memory_exposure_cap > 0.3);
    }
}
