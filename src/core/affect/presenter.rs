//! Prompt rendering: converts topology, desire, and mood state into
//! LLM-visible guidance blocks.
//!
//! # Purpose
//!
//! All upstream pipeline stages (detection → appraisal → topology → style)
//! produce structured data. This module is the **last step before the LLM**:
//! it serialises that data into human-readable text blocks that the model can
//! act on. Each function returns either a non-empty guidance string or an empty
//! string when the signal is not strong enough to warrant injection.
//!
//! # Block types
//!
//! | Function | When non-empty | What it injects |
//! |----------|---------------|-----------------|
//! | [`render_affect_block`] | Label is not Neutral | Brief behavioural guidance based on affect label |
//! | [`render_desire_block`] | Intensity ≥ 0.3 and prefix is set | Objective prefix guiding agent priority |
//! | [`render_session_mood_block`] | Mood deviates from neutral baseline | Overall tone and energy summary |
//! | [`render_topology_block`] | ≥1 surfaced node above 0.01 | Surface tone, held-back nodes, thin-response warning |
//!
//! Empty returns are intentional: injecting empty blocks into the prompt
//! adds token cost with no benefit. Callers should filter them out.

use std::fmt::Write as FmtWrite;

use super::desire::DesireState;
use super::mood::SessionMood;
use super::topology::TopologySnapshot;
use crate::contracts::affect::AffectLabel;

/// Render a brief affect guidance block for the LLM.
///
/// Returns an empty string for `Neutral` (no injection needed) or when the
/// affect label would produce no actionable guidance. Otherwise returns a
/// single-line `[Affect Guidance (confidence: X.XX)]` header followed by
/// a one-sentence behavioural directive.
#[must_use]
pub(crate) fn render_affect_block(label: AffectLabel, confidence: f64) -> String {
    if label == AffectLabel::Neutral {
        return String::new();
    }

    let guidance = match label {
        AffectLabel::Confused => "User seems confused. Provide clearer explanations with examples.",
        AffectLabel::Frustrated => {
            "User seems frustrated. Be direct, solution-focused, and concise."
        }
        AffectLabel::Anxious => {
            "User seems anxious. Be reassuring and thorough in your explanation."
        }
        AffectLabel::Sad => "User seems down. Be warm and supportive in tone.",
        AffectLabel::Angry => {
            "User seems upset. Stay professional, acknowledge the issue, and provide a clear solution."
        }
        AffectLabel::Excited => "User is excited. Match their enthusiasm appropriately.",
        AffectLabel::Grateful => "User is grateful. Acknowledge warmly and continue being helpful.",
        AffectLabel::Curious => "User is curious. Provide depth and invite further exploration.",
        AffectLabel::Overwhelmed => {
            "User feels overwhelmed. Simplify, prioritize, and break things down."
        }
        AffectLabel::Neutral => return String::new(),
    };

    let mut out = String::with_capacity(32 + guidance.len());
    let _ = writeln!(out, "[Affect Guidance (confidence: {confidence:.2})]");
    out.push_str(guidance);
    out.push('\n');
    out
}

/// Render the desire-objective block for the LLM.
///
/// Returns an empty string when intensity < 0.3 or when the desire has no
/// objective prefix (i.e., neutral affect). Otherwise returns a
/// `[Desire Objective (intensity: X.XX)]` block containing the objective
/// prefix that should reshape the agent's response priority.
#[must_use]
pub(crate) fn render_desire_block(desire: &DesireState) -> String {
    if desire.objective_prefix.is_empty() || desire.intensity < 0.3 {
        return String::new();
    }

    let mut out = String::with_capacity(32 + desire.objective_prefix.len());
    let _ = writeln!(
        out,
        "[Desire Objective (intensity: {:.2})]",
        desire.intensity
    );
    out.push_str(&desire.objective_prefix);
    out.push('\n');
    out
}

/// Render the session mood block for the LLM.
///
/// Delegates to [`SessionMood::render_block`]. Returns an empty string when
/// the mood is at or near the neutral baseline (pleasure ≈ 0, arousal ≈ 0.3,
/// dominance ≈ 0); otherwise returns a `[Current Mood]` summary.
#[must_use]
pub(crate) fn render_session_mood_block(mood: &SessionMood) -> String {
    mood.render_block()
}

/// Render the affect topology snapshot as a prompt guidance block.
///
/// Shows which internal emotions are surfaced, which are suppressed,
/// and the dominant surface tone — guiding the model to express the
/// character's internal state rather than defaulting to a flat label.
#[must_use]
pub(crate) fn render_topology_block(snapshot: &TopologySnapshot) -> String {
    let surfaced = snapshot.top_surfaced(3);
    if surfaced.is_empty() {
        return String::new();
    }

    // Surfaced nodes: what the character is feeling on the surface
    let mut out = String::with_capacity(160);
    out.push_str("[Affect Topology]\nSurface tone: ");
    for (i, a) in surfaced.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{} ({:.0}%)", a.node.0, a.surfaced_intensity * 100.0);
    }

    // Suppressed nodes: what the character feels but does not express
    let suppressed = snapshot.suppressed_nodes();
    if !suppressed.is_empty() {
        out.push_str("\nHeld back: ");
        for (i, a) in suppressed.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let _ = write!(
                out,
                "{} ({:.0}% internal)",
                a.node.0,
                a.diffused_intensity * 100.0
            );
        }
    }

    if snapshot.is_thin_response() {
        out.push_str(
            "\nWarning: only one emotion active with no suppression. \
             Add mixed tone or indirect expression.",
        );
    }

    out.push('\n');
    out
}
