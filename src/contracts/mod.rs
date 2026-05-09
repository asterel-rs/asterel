//! Cross-module data contracts.
//!
//! Thin module containing only data types and traits shared across
//! subsystem boundaries. No business logic lives here.

pub mod a2a;
pub mod affect;
pub mod channels;
pub(crate) mod experience;
pub mod ids;
pub mod inference;
pub mod media;
pub mod memory;
pub mod memory_domain;
pub mod memory_error;
pub mod memory_forget;
pub mod memory_traits;
pub mod network;
pub mod observability;
pub mod person_identity;
pub(crate) mod policy;
pub mod provider;
pub mod providers;
pub mod quality;
pub mod scores;
pub mod security;
pub mod session_control;
pub mod strings;
pub mod tenant;
pub mod tools;
