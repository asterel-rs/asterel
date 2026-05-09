//! Shared runtime service bootstrap and thin surface entrypoints.
//!
//! This module centralizes the common startup work needed by the
//! agent, gateway, and daemon surfaces so CLI dispatch can stay thin
//! and future remote clients can target a stable runtime API.

mod a2a_task;
mod admin_state;
mod bootstrap;
mod companion_policy;
mod companion_settings;
mod companion_turn;
mod companion_turn_contract;
mod governance_queries;
mod management;
mod memory_review;
mod operator_scope;
mod operator_sessions;
mod operator_tenant_context;
mod plugins;
mod provider_discovery;
mod provider_factory;
mod runtime_operational;
mod runtime_queries;
mod self_amendment_review;
mod surface;
mod surface_plans;
mod tenant_binding;

pub use a2a_task::{
    A2aTaskStore, cancel_task as cancel_a2a_task, complete_task as complete_a2a_task,
    evict_stale_tasks as evict_a2a_tasks, fail_task as fail_a2a_task, new_a2a_task_store,
    register_task as register_a2a_task,
};
pub use admin_state::{
    AdminStateError, load_admin_auth_profile, load_admin_auth_profiles,
    load_admin_runtime_config_snapshot, save_admin_auth_profiles, save_admin_runtime_config,
    set_admin_auth_profile_disabled, set_admin_provider_auth_profile,
    set_admin_provider_default_model, set_admin_provider_enabled,
    update_admin_active_provider_selection,
};
pub use bootstrap::{
    RuntimeModelSelection, RuntimeServiceBootstrapOptions, SharedRuntimeServices,
    bootstrap_runtime_memory, bootstrap_runtime_services,
};
pub(crate) use companion_policy::{
    PolicyAssemblyInput, build_policy_section, render_baseline_safety_section,
    render_prompt_confidentiality_section,
};
pub use companion_settings::{
    companion_settings_path, load_companion_admin_settings, save_companion_admin_settings,
};
pub(crate) use companion_turn::{
    CompanionTransportTurnRequest, CompanionTurnRuntimeDeps, run_transport_companion_turn,
};
pub use companion_turn_contract::{
    CompanionTurnContractExclusionRules, CompanionTurnContractFixture,
    CompanionTurnContractSemantics, derive_contract_semantics, semantics_match_with_exclusions,
};
pub use governance_queries::{
    GovernanceSummarySnapshot, PendingWindowSnapshot, load_admin_governance_summary,
    load_admin_governance_summary_with_runtime_trust,
};
pub use management::{
    ChannelActionResult, ChannelMutationResult, ManagedChannelInventory, ManagedChannelRecord,
    ManagedRuntimeOwner, ManagedSkillRecord, RuntimeApplyMode, SkillMutationResult,
    create_admin_channel, install_admin_skill, list_admin_channels, list_admin_skills,
    remove_admin_skill, run_admin_channel_action, update_admin_channel, update_admin_skill,
};
pub use memory_review::{
    correct_admin_memory_slot, forget_admin_memory_slot, list_admin_memory_entities,
    load_admin_memory_consolidation_statuses, load_admin_memory_exposure_status,
    load_admin_memory_slots,
};
pub use operator_scope::OperatorScope;
pub use operator_sessions::{
    OperatorSessionAccess, PagedOperatorSessionListReadModel,
    PagedOperatorSessionMessageListReadModel, append_operator_session_message,
    build_operator_session_message_read_model, build_operator_session_summary_read_model,
    create_operator_session, delete_operator_session, list_operator_session_messages,
    list_operator_sessions, load_operator_session, session_matches_operator_scope,
};
pub use operator_tenant_context::{
    list_operator_tenants, load_operator_tenant_context_read_model,
    load_operator_tenant_context_view, load_operator_tenant_inventory_read_model,
    update_operator_tenant_context_view,
};
pub use plugins::{runtime_mcp_tool_provider, runtime_skill_metadata_provider};
pub use provider_discovery::{
    DiscoveredModel, DiscoveredModelCapabilityHints, ProviderDiscoveryCache,
    ProviderDiscoveryEntry, ProviderDiscoveryRequest, ProviderDiscoveryResult,
    ProviderDiscoverySource, load_provider_discovery_cache, provider_discovery_cache_path,
    resolve_provider_discovery, save_provider_discovery_cache,
};
pub use provider_factory::{
    build_tool_registry, create_provider_box, create_resilient_provider,
    create_resilient_provider_box, create_resilient_provider_box_with_credential_provider,
    create_resilient_provider_with_credential_provider, provider_selector_with_api_base,
};
pub use runtime_operational::{
    GatewayReadinessAssessment, GatewayReadinessProfile, RuntimeCapabilityState,
    RuntimeCapabilityStatus, RuntimeChannelSurfaceState, RuntimeOperationalSnapshot,
    load_gateway_readiness_assessment, load_runtime_operational_snapshot,
    runtime_boot_requires_onboarding, runtime_boot_requires_onboarding_for_provider,
};
pub use runtime_queries::{
    GatewayRestartReadModel, MoodReadModel, RuntimeStatusSnapshot, load_admin_mood,
    load_admin_runtime_status, request_gateway_restart,
};
pub use self_amendment_review::{
    SelfAmendmentCandidateReviewStore, approve_self_amendment_candidate,
    load_self_amendment_candidate_review, load_self_amendment_candidate_review_for_tenant,
};
pub use surface::RuntimeSurfaceResources;
pub use surface_plans::{
    ChannelsSurfacePlan, GatewaySurfacePlan, RuntimeBindAddress, prepare_channels_surface_plan,
    prepare_gateway_surface_plan, run_agent_surface, run_channels_surface, run_daemon_surface,
    run_gateway_surface,
};
#[cfg(test)]
pub use tenant_binding::new_empty_tenant_binding_store;
pub use tenant_binding::{
    TenantBindingInventory, TenantBindingRecord, TenantBindingStore, TenantContextUpdate,
    TenantContextView, list_operator_tenant_inventory, load_operator_tenant_context,
    new_tenant_binding_store, resolve_selected_tenant_for_principal,
    update_operator_tenant_context,
};
