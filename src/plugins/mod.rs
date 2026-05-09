//! Plugins subsystem: skills, MCP bridge, companion protocol, extensions, and integrations.
//!
//! ## Sub-modules
//!
//! - **[`skills`]** — Skill loading, hot-reload, and catalog rendering. Skills are Markdown
//!   prompt files that inject domain knowledge or tool instructions into the agent's context.
//! - **[`mcp`]** — [`MCP`] (Model Context Protocol) client/server bridge. Exposes local tools to
//!   MCP-aware clients and forwards MCP tool calls into the agent's tool loop.
//! - **[`companion`]** — Companion protocol: surface adapters, multimodal context, and rhythm
//!   scheduling for the ambient companion persona.
//! - **[`integrations`]** — Third-party service integrations (e.g. calendar, search, external
//!   APIs) surfaced as agent tools.
//! - **[`extensions`]** — Runtime extension points for loading custom tool providers.

pub mod companion;
pub mod extensions;
pub mod integrations;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod skills;
