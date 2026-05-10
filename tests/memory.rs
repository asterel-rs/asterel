#[path = "support/memory_harness.rs"]
mod memory_harness;
#[path = "support/test_env.rs"]
mod test_env;

#[path = "memory/backend_compatibility.rs"]
mod backend_compatibility;
#[path = "memory/backend_parity.rs"]
mod backend_parity;
#[path = "memory/capability_contract.rs"]
mod capability_contract;
#[path = "memory/comparison.rs"]
mod comparison;
#[path = "memory/consolidation_orchestrator.rs"]
mod consolidation_orchestrator;
#[path = "memory/delete_contract.rs"]
mod delete_contract;
#[path = "memory/governance.rs"]
mod governance;
#[path = "memory/layer_schema.rs"]
mod layer_schema;
#[path = "memory/markdown_tagged.rs"]
mod markdown_tagged;
#[cfg(feature = "postgres")]
#[path = "memory/postgres_integration.rs"]
mod postgres_integration;
#[path = "memory/provenance_validation.rs"]
mod provenance_validation;
#[path = "memory/revocation_gate.rs"]
mod revocation_gate;
#[path = "memory/tenant_recall.rs"]
mod tenant_recall;
#[path = "memory/tool_contract.rs"]
mod tool_contract;
