//! Memory influence (grounding) subsystem: turning recalled memory into prompt context.
//!
//! The grounding pipeline classifies recalled [`MemoryRecallEntry`] items into
//! three confidence tiers ([`GroundingTier::Fact`] / `Hint` / `Noise`),
//! assembles a [`ContextBundle`], and renders it as a formatted block for
//! injection into system or user prompts.

mod builder;
mod render;
mod types;

pub use builder::build_context_bundle;
pub use render::{
    CompanionGroundingAugmentation, GroundingExposureMonitorSnapshot, GroundingExposureProjection,
    build_companion_grounding_augmentation, build_companion_grounding_augmentation_block,
    build_grounding_augmentation_block, grounding_exposure_monitor_snapshot,
    render_grounding_contract,
};
pub use types::{ContextBundle, GroundingEntry, GroundingTier};
