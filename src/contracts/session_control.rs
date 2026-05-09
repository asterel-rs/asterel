//! Session control data types: conversation mode, response density, and
//! avoidance behaviours shared across L1 (sessions) and L3 (agent).

use serde::{Deserialize, Serialize};

/// The current conversational mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMode {
    /// Casual back-and-forth, low stakes.
    Chitchat,
    /// Empathetic listening, emotional support.
    Empathy,
    /// Focused work, task execution.
    Task,
    /// Deep exploration of a topic.
    DeepDive,
}

/// Expected response density for this turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedDensity {
    /// Keep it short: one or two sentences.
    Brief,
    /// Normal conversational length.
    Normal,
    /// User expects some elaboration.
    Expanded,
}

/// Behaviours the companion should avoid on this turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvoidBehavior {
    /// Don't over-explain or lecture.
    Overexplain,
    /// Don't sound preachy or moralistic.
    Preachy,
    /// Don't suddenly switch to organizing/structuring mode.
    SuddenOrganize,
    /// Don't lead with analysis when the user wants empathy.
    AnalysisBeforeEmpathy,
}

/// Thin session-scoped state that guides per-turn companion behaviour.
///
/// Updated at the start of each turn from the user message and affect reading.
/// Consumed by the prompt assembly pipeline to inject constraints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionControlState {
    pub mode: ConversationMode,
    pub density: ExpectedDensity,
    pub avoid: Vec<AvoidBehavior>,
    /// Number of turns in the current mode (resets on mode change).
    pub mode_turns: u32,
}

impl Default for SessionControlState {
    fn default() -> Self {
        Self {
            mode: ConversationMode::Chitchat,
            density: ExpectedDensity::Normal,
            avoid: Vec::new(),
            mode_turns: 0,
        }
    }
}
