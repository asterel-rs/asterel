//! Human-in-the-loop approval broker subsystem.
//!
//! When a tool invocation requires operator sign-off (e.g. the `PolicyEngine`
//! returns `PolicyDecisionKind::RequireApproval`, or `AutonomyLevel::Supervised`
//! cannot auto-approve a write tool), the call is handed to an [`ApprovalBroker`]
//! that asks a human operator and returns an [`ApprovalDecision`].
//!
//! # Why this exists
//!
//! `AutonomyLevel::Supervised` lets the agent act on read-only tools without
//! approval but holds write and high-risk tools until a human says yes. Without
//! a broker, that pause has nowhere to go. The broker pattern keeps the decision
//! mechanism pluggable: the same security middleware code path works whether the
//! operator is at a terminal, monitoring a Slack channel, or away (auto-deny).
//!
//! # Broker pattern
//!
//! [`ApprovalBroker`] is an async trait (`request_approval → ApprovalDecision`).
//! Concrete implementations live in sub-modules:
//!
//! | Broker | When used |
//! |--------|-----------|
//! | [`CliApprovalBroker`] | interactive terminal; prompts stdin with a timeout |
//! | [`SlackApprovalBroker`] | polls Slack channel for `approve`/`deny` replies |
//! | [`TelegramApprover`] | polls Telegram chat for `approve`/`deny` replies |
//! | [`DiscordApprover`] | polls Discord channel (feature-gated) |
//! | [`MatrixApprovalBroker`] | polls Matrix room (feature-gated) |
//! | [`AutoDenyBroker`] | rejects all requests; used in non-interactive contexts |
//! | [`TextReplyApprovalBroker`] | stub auto-deny for unimplemented channel types |
//!
//! [`broker_for_channel`] selects the right broker given a channel name and
//! [`ChannelApprovalCtx`].  When credentials are missing, it falls back to
//! [`CliApprovalBroker`] (if `operator_fallback = true`) or
//! [`TextReplyApprovalBroker`] (auto-deny).
//!
//! # Approval flow
//!
//! ```text
//! Tool call arrives
//!   └─ SecurityMiddleware / PolicyEngine
//!        └─ PolicyDecisionKind::RequireApproval
//!             └─ ApprovalBroker::request_approval(ApprovalRequest)
//!                  ├─ Approved           → tool executes
//!                  ├─ ApprovedWithGrant  → tool executes + PermissionGrant persisted
//!                  └─ Denied { reason }  → tool blocked, reason surfaced to caller
//! ```
//!
//! # Risk classification
//!
//! Before constructing an [`ApprovalRequest`], callers use [`classify_risk`]
//! (or [`classify_risk_args`] for path-aware classification) to assign a
//! [`RiskLevel`] that brokers display to the operator.  Argument summaries are
//! produced by [`summarize_args`] with secrets scrubbed before display.
//!
//! # Grant caching (session / permanent)
//!
//! A broker may return [`ApprovalDecision::ApprovedWithGrant`] carrying a
//! [`PermissionGrant`] scoped to [`GrantScope::Session`] or
//! [`GrantScope::Permanent`].  The caller is responsible for persisting and
//! consulting the grant cache so that repeated approvals for the same pattern
//! are not required.
//!
//! # Integration note
//!
//! Broker results are routed through the shared governance grammar before the
//! final approval resolution is returned, and each approval decision emits a
//! `GovernanceAuditRecord` through the runtime audit/logging path.

pub mod channel;
pub mod cli;
#[cfg(feature = "discord")]
pub mod discord;
pub mod inline;
#[cfg(feature = "matrix")]
pub mod matrix;
pub mod slack;
pub mod telegram;
mod types;

pub use crate::security::governance::RiskLevel;
pub use channel::{ChannelApprovalCtx, TextReplyApprovalBroker, broker_for_channel};
pub use cli::CliApprovalBroker;
#[cfg(feature = "discord")]
pub use discord::DiscordApprover;
pub use inline::{run_inline_approval, timed_out_decision};
#[cfg(feature = "matrix")]
pub use matrix::MatrixApprovalBroker;
pub use slack::SlackApprovalBroker;
pub use telegram::TelegramApprover;

pub use self::types::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, AutoDenyBroker, GrantScope, PermissionGrant,
    classify_risk, classify_risk_args, format_approval_text, summarize_args,
};
