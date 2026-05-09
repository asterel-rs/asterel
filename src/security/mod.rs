//! Security subsystem facade.
//!
//! Provides approval workflows, policy enforcement, secret storage,
//! pairing guards, URL/process protections, and permission persistence.

/// Log and recover from a poisoned mutex lock.
///
/// Expands to a closure suitable for `unwrap_or_else` that logs the
/// poisoning event at `error!` level (with module path and line number)
/// before recovering the inner guard.
macro_rules! poison_recover {
    () => {
        |poison| {
            tracing::error!(
                module = module_path!(),
                line = line!(),
                "mutex poisoned, recovering with potentially inconsistent state"
            );
            poison.into_inner()
        }
    };
}

pub(crate) use poison_recover;

/// Affect → governance feedback bridge: emotional distress reduces autonomy.
pub mod affect_governance;
pub mod approval;
pub mod auth;
/// Capability-based security for tool execution.
pub mod capability;
/// Dynamic per-domain trust tracker with time decay and autonomy demotion.
pub mod domain_trust;
/// External content validation and sanitization.
pub mod external_content;
pub mod governance;
/// ML-based intent classification for injection detection.
pub mod intent_classifier;
/// Argon2id key derivation for password-based secret encryption.
pub mod kdf;
pub mod pairing;
pub mod path_boundary;
pub mod permissions;
pub mod policy;
pub(crate) mod private_file_permissions;
pub mod process_spawn;
/// Secret scrubbing utilities for logs, errors, and user-facing output.
pub mod scrub;
pub mod secrets;
/// Taint tracking for data contamination through tool executions.
pub mod taint;
/// Tool execution policy engine with loadable rules and precedence chain.
pub mod tool_policy;
/// URL validation and SSRF protections.
pub mod url_validation;
pub mod writeback_guard;

pub use approval::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, AutoDenyBroker, GrantScope, PermissionGrant,
};
pub(crate) use approval::{ChannelApprovalCtx, broker_for_channel, classify_risk, summarize_args};
pub use governance::{
    AutonomyVerdict, GovernanceAuditContext, GovernanceAuditRecord, GovernanceDecision,
    GovernanceTrustState, RiskLevel, TrustLevel, evaluate_governance,
};
pub use path_boundary::{
    RootBoundPathKind, canonicalize_path_within_root, resolve_relative_path_within_root,
};
pub use permissions::PermissionStore;
pub use policy::{
    ActionPolicyVerdict, AutonomyLevel, EntityRateLimiter, ExternalActionExecution, SecurityPolicy,
};
pub(crate) use process_spawn::{
    ProcessSpawnClass, enforce_process_spawn_policy_with_args, enforce_spawn_policy,
};
pub(crate) use secrets::SecretStore;
pub use tool_policy::{PolicyDecisionKind, PolicyEngine, PolicyEvaluation, is_read_only_tool};
pub(crate) use url_validation::{resolve_public_fetch_addrs, validate_fetch_url, validate_no_ssrf};
