//! Sub-agent orchestration subsystem.
//!
//! Provides runtime management, role-based dispatch, coordination
//! sessions, and inline/background execution of isolated agents.

pub mod coordination;
pub mod dispatch;
pub mod roles;
mod runtime;
pub(crate) mod spawn_limits;
mod traits;

#[cfg(test)]
pub(crate) use runtime::TEST_RUNTIME_LOCK;
#[cfg(test)]
pub(crate) use runtime::{
    SubagentConfig, SubagentRunStatus, cancel, configure_runtime, get, list, run_inline, spawn,
};
pub(crate) use runtime::{
    SubagentDefaultRuntimeSpec, SubagentDelegationConfig, SubagentHandoffEnvelope,
    SubagentOrchestrator, SubagentRunOptions, cancel_scoped, get_scoped, is_configured,
    list_scoped, run_inline_with_options, spawn_with_options,
};
pub use traits::{
    AgentExtensionProfile, EmptySkillMetadataSnapshot, ExtensionLoader, NoopExtensionLoader,
    NoopSkillMetadataProvider, SkillMetadataProvider, SkillMetadataSnapshotView,
};

#[cfg(test)]
mod tests;
