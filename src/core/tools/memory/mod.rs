//! Memory management tools for the agent's belief store.
//!
//! # Tool hierarchy
//!
//! | Tool | Purpose |
//! |------|---------|
//! | [`store::MemoryStoreTool`] | Append a new immutable event to a belief slot. |
//! | [`recall::MemoryRecallTool`] | Entity-scoped recall using the active backend's ranking path. |
//! | [`lookup::MemoryLookupTool`] | Point lookup — resolve the current value of one slot. |
//! | [`correct::MemoryCorrectTool`] | Amend an existing slot after verifying the prior value. |
//! | [`forget::MemoryForgetTool`] | Soft/hard/tombstone forget requests for a slot. |
//! | [`governance::MemoryGovernanceTool`] | Data-sovereignty actions: inspect, export, delete, verify. |
//!
//! # Middleware integration
//!
//! Every write path (`store`, `correct`) passes through
//! `security::writeback_guard::enforce_tool_memory_write_policy` before
//! the memory backend is touched. Reads (`recall`, `lookup`) enforce tenant
//! scope via `policy_context::enforce_entity_scope`, which prevents cross-tenant
//! data leakage when tenant mode is enabled on the `ExecutionContext`.
//!
//! # Privacy model
//!
//! Each slot carries a `PrivacyLevel` (`Public`, `Private`, or `Secret`).
//! The governance tool redacts `Private` and `Secret` values from inspect/export
//! responses unless `include_sensitive: true` is explicitly set by the caller.
//! Writing `secret`-level slots is rejected by the default security policy.

mod policy_context;

pub mod correct;
pub mod forget;
pub mod governance;
pub mod lookup;
pub mod recall;
pub mod store;

pub use correct::MemoryCorrectTool;
pub use forget::MemoryForgetTool;
pub use governance::MemoryGovernanceTool;
pub use lookup::MemoryLookupTool;
pub use recall::MemoryRecallTool;
pub use store::MemoryStoreTool;
