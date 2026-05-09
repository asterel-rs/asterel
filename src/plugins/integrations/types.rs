//! Type definitions for integrations: status, category, and
//! catalog entry with a config-dependent status function.

use crate::config::Config;

/// Integration status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    /// Fully implemented and ready to use
    Available,
    /// Configured and active
    Active,
}

/// Integration category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationCategory {
    /// Chat and messaging platforms.
    Chat,
    /// AI model providers.
    AiModel,
    /// Productivity and workflow tools.
    Productivity,
    /// Smart home and `IoT` devices.
    SmartHome,
    /// Developer tools and automation.
    ToolsAutomation,
    /// Media creation and editing.
    MediaCreative,
    /// Social media platforms.
    Social,
    /// Platform-level integrations.
    Platform,
}

impl IntegrationCategory {
    /// Returns the human-readable display label for this category.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat Providers",
            Self::AiModel => "AI Models",
            Self::Productivity => "Productivity",
            Self::SmartHome => "Smart Home",
            Self::ToolsAutomation => "Tools & Automation",
            Self::MediaCreative => "Media & Creative",
            Self::Social => "Social",
            Self::Platform => "Platforms",
        }
    }

    /// Returns all category variants in display order.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Chat,
            Self::AiModel,
            Self::Productivity,
            Self::SmartHome,
            Self::ToolsAutomation,
            Self::MediaCreative,
            Self::Social,
            Self::Platform,
        ]
    }
}

/// A registered integration
pub struct IntegrationEntry {
    /// Display name of the integration.
    pub name: &'static str,
    /// Short description of the integration.
    pub description: &'static str,
    /// Category this integration belongs to.
    pub category: IntegrationCategory,
    /// Function that determines runtime status from config.
    pub status_fn: fn(&Config) -> IntegrationStatus,
}
