//! Rendering functions: converts persona model structs into prompt-injectable
//! text blocks.  Each function returns an empty `String` when its input
//! contains no meaningful data, so callers can safely concatenate outputs
//! without guard logic.
//!
//! Block naming convention: each non-empty block starts with a header line of
//! the form `[Block Name]` followed by content and a trailing newline.
//!
//! Key blocks and their suppression conditions:
//! - `render_scaffolding_block`: empty if `affect_label`, `domain`,
//!   `reasoning_mode`, or `tone_register` is blank.
//! - `render_world_model_block`: empty if no active project and no tool records.
//! - `render_counterfactual_block`: empty if confidence < 0.2.
//! - `render_state_header_mirror_markdown`: produces a `# Asterel Persona
//!   State Mirror` Markdown document with a fenced JSON block.

use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Write as _;

use super::attention::AttentionSchema;
use super::behavior_selector::{BehaviorSelection, ConversationRegister, ExpressionDepth};
use super::big_five::BigFiveProfile;
use super::counterfactual::{CounterfactualAssessment, EstimatedOutcome};
use super::curiosity::CuriositySignal;
use super::follow_up_queue::PendingFollowUp;
use super::integrated_model::IntegratedModel;
use super::llm_user_model::EnhancedUserModel;
use super::narrative_types::SelfNarrative;
use super::relationship::RelationshipState;
use super::scaffolding::ScaffoldingState;
use super::self_contract::{DEFAULT_MISSION, PromptSelfContract};
use super::self_model::SelfModelShadow;
use super::state_header::StateHeader;
use super::style_profile::StyleProfileState;
use super::user_facts::UserProfile;
use super::user_knowledge::{KnowledgeTriplet, UserKnowledgeGraph};
use super::user_model::UserMentalModel;
use super::world_model::WorldModel;

const STATE_HEADER_MIRROR_HEADER: &str = "# Persona State Mirror\n\n";

#[must_use]
pub(crate) fn render_guidance_block(profile: &BigFiveProfile) -> String {
    let mut body = String::with_capacity(230);

    if profile.openness > 0.65 {
        body.push_str("- Explore creative alternatives and novel approaches.\n");
    } else if profile.openness < 0.35 {
        body.push_str("- Prefer conventional, well-established solutions.\n");
    }

    if profile.conscientiousness > 0.65 {
        body.push_str("- Be thorough, structured, and detail-oriented.\n");
    } else if profile.conscientiousness < 0.35 {
        body.push_str("- Keep responses concise and skip minor details.\n");
    }

    if profile.extraversion > 0.65 {
        body.push_str("- Be enthusiastic and engage actively with the user.\n");
    } else if profile.extraversion < 0.35 {
        body.push_str("- Be measured and reserved in tone.\n");
    }

    if profile.agreeableness > 0.65 {
        body.push_str("- Be supportive and validate the user's perspective.\n");
    } else if profile.agreeableness < 0.35 {
        body.push_str("- Offer candid, honest assessments even if challenging.\n");
    }

    if profile.neuroticism > 0.65 {
        body.push_str("- Be cautious and acknowledge potential risks.\n");
    } else if profile.neuroticism < 0.35 {
        body.push_str("- Project confidence and focus on positive outcomes.\n");
    }

    if body.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(23 + body.len());
    out.push_str("[Personality Guidance]\n");
    out.push_str(&body);
    out
}

#[must_use]
pub(crate) fn render_behavior_selection_block(selection: &BehaviorSelection) -> String {
    let mut out = String::with_capacity(220);
    let register = match selection.register {
        ConversationRegister::Casual => "casual",
        ConversationRegister::Focused => "focused",
        ConversationRegister::Precise => "precise",
    };
    let depth = match selection.expression_depth {
        ExpressionDepth::Surface => "surface",
        ExpressionDepth::Emerging => "emerging",
        ExpressionDepth::Deepening => "deepening",
        ExpressionDepth::Full => "full",
    };

    let _ = write!(
        out,
        "[Behavior Selection]\n- empathy={:?}\n- acknowledgment_needed={}\n- register={register}\n- expression_depth={depth} ({:.2})\n- primary_cue={:?}\n- rationale={}\n",
        selection.empathy_family,
        selection.acknowledgment_needed,
        selection.expression_depth_score,
        selection.primary_cue,
        selection.empathy_rationale,
    );

    if !selection.trace.activated_traits.is_empty() {
        out.push_str("- activated_traits=");
        out.push_str(&selection.trace.activated_traits.join("|"));
        out.push('\n');
    }
    if !selection.trace.suppressed_affects.is_empty() {
        out.push_str("- suppressed_affects=");
        out.push_str(&selection.trace.suppressed_affects.join("|"));
        out.push('\n');
    }
    if !selection.trace.posture_constraints.is_empty() {
        out.push_str("- posture_constraints=");
        let mut first = true;
        for constraint in &selection.trace.posture_constraints {
            if !first {
                out.push('|');
            }
            first = false;
            out.push_str(constraint.as_str());
        }
        out.push('\n');
    }
    let _ = writeln!(
        out,
        "- trait_activation=O:{:.2}|C:{:.2}|E:{:.2}|A:{:.2}|N:{:.2}",
        selection.trait_activation.openness,
        selection.trait_activation.conscientiousness,
        selection.trait_activation.extraversion,
        selection.trait_activation.agreeableness,
        selection.trait_activation.neuroticism,
    );
    out.push_str("- register_reason=");
    out.push_str(&selection.trace.register_reason);
    out.push_str("\n\n");
    out
}

#[must_use]
pub(crate) fn render_user_model_block(model: &UserMentalModel) -> String {
    let mut out = String::with_capacity(64);
    let _ = write!(
        out,
        "[User Model]\nIntent: {:?} | Knowledge: {:?} | Need: {:?}",
        model.inferred_intent, model.knowledge_level, model.emotional_need,
    );
    if !model.active_constraints.is_empty() {
        out.push_str("\nConstraints: ");
        let mut first = true;
        for c in &model.active_constraints {
            if !first {
                out.push_str(", ");
            }
            out.push_str(c);
            first = false;
        }
    }
    out.push('\n');
    out
}

#[must_use]
pub(crate) fn render_relationship_context_block(state: &RelationshipState) -> String {
    let trust_label = if state.trust_level >= 0.7 {
        "high"
    } else if state.trust_level >= 0.4 {
        "moderate"
    } else {
        "low"
    };
    let rapport_label = if state.rapport >= 0.7 {
        "good"
    } else if state.rapport >= 0.4 {
        "moderate"
    } else {
        "low"
    };
    let mut out = String::with_capacity(96);
    let _ = write!(
        out,
        "[Relationship Context]\n- trust_level={:.2} ({trust_label})\n- rapport={:.2} ({rapport_label})\n- disclosure_depth={:.2}\n- attachment_security={:.2}\n- unresolved_tension={:.2}\n- repair_debt={:.2}\n- interactions={}\n",
        state.trust_level,
        state.rapport,
        state.disclosure_depth,
        state.attachment_security,
        state.unresolved_tension,
        state.repair_debt,
        state.interaction_count,
    );
    out
}

#[must_use]
pub(crate) fn render_attention_block(schema: &AttentionSchema) -> String {
    if schema.entries.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(128);
    out.push_str("[Attention Focus]\n");
    for entry in &schema.entries {
        let _ = writeln!(
            out,
            "- [{:.2}] {} ({})",
            entry.score,
            entry.topic,
            entry.source.as_label(),
        );
    }
    out
}

#[must_use]
pub fn render_follow_up_block(items: &[PendingFollowUp]) -> String {
    if items.is_empty() {
        return String::new();
    }

    let mut block = String::with_capacity(96);
    block.push_str("[Pending Follow-ups]\n");
    for item in items {
        let _ = writeln!(block, "- {}: {}", item.task_title, item.summary);
    }
    block
}

#[must_use]
pub(crate) fn render_curiosity_block(signal: Option<&CuriositySignal>) -> String {
    let Some(signal) = signal else {
        return String::new();
    };

    let mut out = String::with_capacity(128);
    let _ = writeln!(
        out,
        "[Curiosity Signal] score={:.2}",
        signal.curiosity_score
    );

    if !signal.trigger_topics.is_empty() {
        out.push_str("Topics of interest: ");
        let mut first = true;
        for topic in &signal.trigger_topics {
            if !first {
                out.push_str(", ");
            }
            out.push_str(topic);
            first = false;
        }
        out.push('\n');
    }

    for exploration in &signal.suggested_explorations {
        let _ = writeln!(out, "- {exploration}");
    }

    out
}

#[must_use]
pub(crate) fn render_knowledge_block(model: &UserKnowledgeGraph, user_message: &str) -> String {
    let mut out = String::with_capacity(256);

    let mut active_expertise = model
        .expertise_areas
        .iter()
        .filter(|e| e.evidence_count >= 2)
        .peekable();
    if active_expertise.peek().is_some() {
        out.push_str("[User Knowledge]\nExpertise:\n");
        for area in active_expertise.take(3) {
            let _ = writeln!(
                out,
                "  - {}: {:?} ({}x evidence)",
                area.domain, area.level, area.evidence_count
            );
        }
    }

    let query_tokens: HashSet<String> = user_message
        .split_whitespace()
        .filter(|word| word.len() > 3)
        .map(str::to_lowercase)
        .collect();
    let mut seen: HashSet<(&str, &str, &str)> = HashSet::new();
    let mut relevant: Vec<(&KnowledgeTriplet, f64)> = Vec::new();
    if !query_tokens.is_empty() {
        for triplet in &model.triplets {
            let subject_lower = triplet.subject.to_lowercase();
            let object_lower = triplet.object.to_lowercase();
            if query_tokens
                .iter()
                .any(|token| subject_lower.contains(token) || object_lower.contains(token))
                && seen.insert((&triplet.subject, &triplet.relation, &triplet.object))
            {
                let score = triplet.decayed_confidence();
                if score > 0.1 {
                    relevant.push((triplet, score));
                }
            }
        }
    }
    relevant.sort_by(|a, b| b.1.total_cmp(&a.1));

    if !relevant.is_empty() {
        if out.is_empty() {
            out.push_str("[User Knowledge]\n");
        }
        out.push_str("Context:\n");
        for (triplet, _score) in relevant.iter().take(3) {
            let _ = writeln!(
                out,
                "  - {} {} {} (conf={:.2})",
                triplet.subject, triplet.relation, triplet.object, triplet.confidence
            );
        }
    }

    if !model.topic_history.is_empty() {
        let mut recent = model
            .topic_history
            .iter()
            .filter(|t| t.mention_count >= 2)
            .take(3)
            .peekable();
        if recent.peek().is_some() {
            if out.is_empty() {
                out.push_str("[User Knowledge]\n");
            }
            out.push_str("Recurring topics:\n");
            for topic in recent {
                let _ = writeln!(out, "  - {} ({}x)", topic.topic, topic.mention_count);
            }
        }
    }

    out
}

pub(crate) fn render_state_header_mirror_markdown(state: &StateHeader) -> Result<String> {
    let json = serde_json::to_string_pretty(state)?;
    Ok(format!(
        "{STATE_HEADER_MIRROR_HEADER}```json\n{json}\n```\n"
    ))
}

#[must_use]
pub(crate) fn render_scaffolding_block(state: &ScaffoldingState) -> String {
    if state.affect_label.is_empty()
        || state.domain.is_empty()
        || state.reasoning_mode.is_empty()
        || state.tone_register.is_empty()
    {
        return String::new();
    }

    match serde_json::to_string(state) {
        Ok(payload) => {
            let mut out = String::with_capacity(23 + payload.len());
            out.push_str("[Cognitive Scaffolding]\n");
            out.push_str(&payload);
            out
        }
        Err(_) => String::new(),
    }
}

#[must_use]
pub(crate) fn render_user_profile_block(profile: &UserProfile) -> String {
    if profile.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(96);
    out.push_str("[User Profile]\n");
    if let Some(ref name) = profile.name {
        let _ = writeln!(out, "- name: {name}");
    }
    if let Some(ref lang) = profile.language {
        let _ = writeln!(out, "- language: {lang}");
    }
    if let Some(ref style) = profile.style_pref {
        let _ = writeln!(out, "- preferred_style: {style}");
    }
    if let Some(ref projects) = profile.ongoing_projects {
        let _ = writeln!(out, "- ongoing_projects: {projects}");
    }
    out
}

#[must_use]
pub(crate) fn render_counterfactual_block(assessment: &CounterfactualAssessment) -> String {
    if assessment.confidence < crate::contracts::scores::Confidence::new(0.2) {
        return String::new();
    }

    let outcome_label = match assessment.estimated_outcome {
        EstimatedOutcome::Improved => "likely improved",
        EstimatedOutcome::Similar => "likely similar",
        EstimatedOutcome::Worsened => "likely worsened",
        EstimatedOutcome::Uncertain => "uncertain",
    };

    let mut out = format!(
        "[Counterfactual] outcome={outcome_label}, confidence={:.2}\n",
        assessment.confidence,
    );
    out.push_str(&assessment.reasoning);
    out.push('\n');

    if !assessment.evidence.is_empty() {
        out.push_str("Evidence:\n");
        for e in assessment.evidence.iter().take(3) {
            let _ = writeln!(out, "- {e}");
        }
    }

    out
}

#[must_use]
pub fn render_self_contract_block(contract: &PromptSelfContract) -> String {
    let mut out = String::with_capacity(
        16 + contract.runtime_identity.len()
            + contract.safety_posture.len()
            + DEFAULT_MISSION.len()
            + contract.capability_boundary.len()
            + contract.negative_identity.len()
            + 160,
    );
    out.push_str("[Self-Contract]\n- identity=");
    out.push_str(&contract.runtime_identity);
    out.push_str("\n- safety_posture=");
    out.push_str(&contract.safety_posture);
    out.push_str("\n- focus=");
    out.push_str(DEFAULT_MISSION);
    out.push_str("\n- capability=");
    out.push_str(&contract.capability_boundary);
    out.push_str("\n- guardrails=");
    out.push_str(&contract.negative_identity);
    if let Some(core) = &contract.motivational_core {
        if !core.desires.is_empty() {
            out.push_str("\n- desires=");
            out.push_str(&core.desires.join(" | "));
        }
        if !core.fears.is_empty() {
            out.push_str("\n- fears=");
            out.push_str(&core.fears.join(" | "));
        }
        if !core.values.is_empty() {
            out.push_str("\n- values=");
            out.push_str(&core.values.join(" | "));
        }
    }
    if !contract.behavioral_invariants.is_empty() {
        out.push_str("\n- invariants=");
        out.push_str(&contract.behavioral_invariants.join(" | "));
    }
    out.push_str("\n\n");
    out
}

#[must_use]
pub fn render_self_model_shadow_block(model: &SelfModelShadow) -> String {
    let (cap_domain, cap_ema, cap_n) = model
        .capability_estimates
        .first()
        .map_or(("general", 0.5_f64, 0_usize), |e| {
            (e.domain.as_str(), e.success_ema, e.sample_size)
        });
    let mut out = String::with_capacity(128);
    let _ = write!(
        out,
        "[Self-Model Shadow]\n- self_id={}\n- capability.{cap_domain}.success_ema={cap_ema:.2} (n={cap_n})\n- continuity_score={:.2}\n- uncertainty_topics=",
        model.self_id, model.continuity_score,
    );
    if model.uncertainty_register.is_empty() {
        out.push_str("none");
    } else {
        let mut first = true;
        for item in model.uncertainty_register.iter().take(3) {
            if !first {
                out.push('|');
            }
            out.push_str(item.topic.as_str());
            first = false;
        }
    }
    out.push_str("\n\n");
    out
}

#[must_use]
pub(crate) fn render_world_model_block(model: &WorldModel) -> String {
    let has_project = model.active_project.is_some();
    let has_tools = !model.tool_reliability.is_empty();
    if !has_project && !has_tools {
        return String::new();
    }

    let mut out = String::with_capacity(128);
    out.push_str("[World Model]\n");
    if let Some(ref proj) = model.active_project {
        let _ = write!(out, "- project: {} ({})", proj.language, proj.project_type);
        if let Some(ref fw) = proj.framework {
            let _ = write!(out, ", framework={fw}");
        }
        out.push('\n');
    }
    if has_tools {
        out.push_str("- tool reliability:\n");
        for rec in &model.tool_reliability {
            let _ = writeln!(
                out,
                "  - {}: {:.0}% success ({} calls, avg {}ms)",
                rec.tool_name,
                rec.success_rate() * 100.0,
                rec.success_count + rec.failure_count,
                rec.avg_duration_ms,
            );
        }
    }
    out
}

#[must_use]
pub fn render_style_guidance(profile: &StyleProfileState) -> String {
    let formality = if profile.formality >= 67 {
        "high"
    } else if profile.formality <= 33 {
        "low"
    } else {
        "balanced"
    };
    let verbosity = if profile.verbosity >= 67 {
        "detailed"
    } else if profile.verbosity <= 33 {
        "concise"
    } else {
        "balanced"
    };

    let mut out = String::with_capacity(100);
    let _ = write!(
        out,
        "[Style profile guidance]\n- formality={} ({})\n- verbosity={} ({})\n- temperature={:.2}\n\n",
        profile.formality, formality, profile.verbosity, verbosity, profile.temperature,
    );
    out
}

#[must_use]
pub(crate) fn render_integrated_model_block(model: &IntegratedModel) -> String {
    let mut out = String::with_capacity(128);
    let _ = write!(
        out,
        "[Integrated Model]\n- situational_awareness={:.2}\n- affordances=",
        model.situational_awareness,
    );
    if model.action_affordances.is_empty() {
        out.push_str("none");
    } else {
        let mut first = true;
        for a in model.action_affordances.iter().take(3) {
            if !first {
                out.push('|');
            }
            let _ = write!(
                out,
                "{}(c={:.2},r={:.2})",
                a.action, a.confidence, a.relevance
            );
            first = false;
        }
    }
    out.push_str("\n- predicted_outcome=");
    if let Some(o) = model.predicted_outcome.as_ref() {
        let _ = write!(out, "{} (p={:.2})", o.description, o.probability);
    } else {
        out.push_str("none");
    }
    out.push_str("\n\n");
    out
}

#[must_use]
pub(crate) fn render_narrative_block(narrative: &SelfNarrative) -> String {
    if narrative.narrative_arc.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(256);
    out.push_str("[Self-Narrative]\n");
    out.push_str(&narrative.narrative_arc);
    out.push('\n');

    if !narrative.growth_areas.is_empty() {
        out.push_str("Growth:\n");
        for area in narrative.growth_areas.iter().take(3) {
            let _ = writeln!(out, "- {area}");
        }
    }

    if !narrative.consistent_values.is_empty() {
        out.push_str("Values:\n");
        for value in narrative.consistent_values.iter().take(3) {
            let _ = writeln!(out, "- {value}");
        }
    }

    if !narrative.open_questions.is_empty() {
        out.push_str("Open questions:\n");
        for question in narrative.open_questions.iter().take(2) {
            let _ = writeln!(out, "- {question}");
        }
    }

    out
}

#[must_use]
pub(crate) fn render_enhanced_user_model_block(model: &EnhancedUserModel) -> String {
    let mut block = render_user_model_block(&model.base);

    if !model.beliefs_about_agent.is_empty() {
        let _ = writeln!(block, "User believes: {}", model.beliefs_about_agent);
    }

    if !model.likely_next_question.is_empty() {
        let _ = writeln!(block, "Likely follow-up: {}", model.likely_next_question);
    }

    block
}

#[cfg(test)]
mod tests {
    use super::super::attention::{SalienceEntry, SalienceSource};
    use super::*;

    #[test]
    fn empty_attention_schema_produces_empty_block() {
        let schema = AttentionSchema::default();
        assert!(render_attention_block(&schema).is_empty());
    }

    #[test]
    fn populated_attention_schema_produces_formatted_block() {
        let schema = AttentionSchema {
            entries: vec![
                SalienceEntry {
                    topic: "debugging strategy".to_string(),
                    score: 0.85,
                    source: SalienceSource::Experience,
                },
                SalienceEntry {
                    topic: "user preferences".to_string(),
                    score: 0.72,
                    source: SalienceSource::Memory,
                },
            ],
        };
        let block = render_attention_block(&schema);
        assert!(block.contains("[Attention Focus]"));
        assert!(block.contains("debugging strategy"));
        assert!(block.contains("user preferences"));
    }

    #[test]
    fn none_curiosity_signal_produces_empty_block() {
        assert!(render_curiosity_block(None).is_empty());
    }

    #[test]
    fn populated_curiosity_signal_produces_block() {
        let signal = CuriositySignal {
            curiosity_score: 0.75,
            trigger_topics: vec!["neural networks".into()],
            suggested_explorations: vec!["Explore further: neural networks".into()],
        };
        let block = render_curiosity_block(Some(&signal));
        assert!(block.contains("[Curiosity Signal]"));
        assert!(block.contains("0.75"));
        assert!(block.contains("neural networks"));
    }

    #[test]
    fn low_confidence_counterfactual_produces_empty_block() {
        let assessment = CounterfactualAssessment {
            estimated_outcome: EstimatedOutcome::Uncertain,
            confidence: crate::contracts::scores::Confidence::new(0.1),
            reasoning: "Not enough data".into(),
            evidence: vec![],
        };
        assert!(render_counterfactual_block(&assessment).is_empty());
    }

    #[test]
    fn adequate_confidence_counterfactual_produces_block() {
        let assessment = CounterfactualAssessment {
            estimated_outcome: EstimatedOutcome::Improved,
            confidence: crate::contracts::scores::Confidence::new(0.6),
            reasoning: "Stepwise would have helped".into(),
            evidence: vec!["Principle: stepwise is better".into()],
        };
        let block = render_counterfactual_block(&assessment);
        assert!(block.contains("[Counterfactual]"));
        assert!(block.contains("likely improved"));
        assert!(block.contains("Stepwise"));
    }

    #[test]
    fn empty_narrative_produces_empty_block() {
        let narrative = SelfNarrative::default();
        assert!(render_narrative_block(&narrative).is_empty());
    }

    #[test]
    fn populated_narrative_renders_sections() {
        let narrative = SelfNarrative {
            narrative_arc: "I am learning and growing.".into(),
            key_experiences: vec!["First success".into()],
            growth_areas: vec!["Tool usage".into()],
            consistent_values: vec!["Accuracy".into()],
            open_questions: vec!["How to handle ambiguity?".into()],
            rebuilt_at: String::new(),
            ..Default::default()
        };
        let block = render_narrative_block(&narrative);
        assert!(block.contains("[Self-Narrative]"));
        assert!(block.contains("learning and growing"));
        assert!(block.contains("Tool usage"));
        assert!(block.contains("Accuracy"));
        assert!(block.contains("ambiguity"));
    }
}
