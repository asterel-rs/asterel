//! Lightweight theory of mind: rule-based inference of user
//! intent, knowledge level, and emotional need from the current
//! message and affect state.

use crate::contracts::affect::AffectLabel;
use crate::core::memory::MemoryRecallEntry;

/// High-level intent classification for the current user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserIntent {
    /// User is troubleshooting an error or bug.
    Debug,
    /// User is seeking to understand a concept.
    Learn,
    /// User is giving a direct command or instruction.
    Instruct,
    /// User is brainstorming or exploring possibilities.
    Explore,
    /// User is expressing frustration without a clear request.
    Vent,
}

/// Estimated knowledge level of the user in the current domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnowledgeLevel {
    /// Basic or beginner-level understanding.
    Novice,
    /// Moderate familiarity with the domain.
    Intermediate,
    /// Strong technical knowledge.
    Advanced,
    /// Deep domain expertise.
    Expert,
}

/// The emotional need the user likely seeks from the interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmotionalNeed {
    /// User wants affirmation or confirmation.
    Validation,
    /// User wants a concrete fix or answer.
    Solution,
    /// User wants open-ended discussion.
    Exploration,
    /// User wants emotional support.
    Empathy,
    /// User wants fast, minimal-friction execution.
    Speed,
}

/// Inferred mental model of the user for this turn.
#[derive(Debug, Clone)]
pub(crate) struct UserMentalModel {
    /// Classified intent for the current message.
    pub inferred_intent: UserIntent,
    /// Estimated domain expertise of the user.
    pub knowledge_level: KnowledgeLevel,
    /// Primary emotional need driving the interaction.
    pub emotional_need: EmotionalNeed,
    /// Situational constraints (e.g. "time pressure").
    pub active_constraints: Vec<String>,
}

/// Infer a user mental model from the message, affect, and memory.
pub(crate) fn infer_user_model(
    user_message: &str,
    affect: &crate::core::affect::AffectReading,
    user_memories: &[MemoryRecallEntry],
) -> UserMentalModel {
    let intent = infer_intent(user_message, affect.label);
    let knowledge_level = infer_knowledge_level(user_message, user_memories);
    let emotional_need = infer_emotional_need(intent, affect.label);
    let active_constraints = infer_constraints(user_message);

    UserMentalModel {
        inferred_intent: intent,
        knowledge_level,
        emotional_need,
        active_constraints,
    }
}

// ── Intent inference ────────────────────────────────────────────

fn infer_intent(message: &str, affect: AffectLabel) -> UserIntent {
    let lower = message.to_lowercase();
    let has_code_block = message.contains("```") || message.contains("    ");

    // Debug: error keywords + code
    if has_code_block
        && contains_any(
            &lower,
            &[
                "error",
                "bug",
                "crash",
                "not working",
                "fails",
                "broken",
                "exception",
            ],
        )
    {
        return UserIntent::Debug;
    }

    // Learn: question patterns
    if contains_any(
        &lower,
        &[
            "how",
            "why",
            "explain",
            "what is",
            "what are",
            "teach",
            "understand",
        ],
    ) && (lower.contains('?') || is_question_form(&lower))
    {
        return UserIntent::Learn;
    }

    // Explore: open-ended
    if contains_any(
        &lower,
        &[
            "what if",
            "could we",
            "ideas",
            "possibilities",
            "brainstorm",
            "consider",
        ],
    ) {
        return UserIntent::Explore;
    }

    // Vent: strong emotion without clear request
    if matches!(
        affect,
        AffectLabel::Frustrated | AffectLabel::Angry | AffectLabel::Sad
    ) && !lower.contains('?')
        && !is_imperative(&lower)
    {
        return UserIntent::Vent;
    }

    // Instruct: imperative / short directive
    if is_imperative(&lower) || word_count(message) <= 8 {
        return UserIntent::Instruct;
    }

    UserIntent::Instruct
}

fn infer_knowledge_level(message: &str, memories: &[MemoryRecallEntry]) -> KnowledgeLevel {
    // Check user memories for expertise signals
    for mem in memories {
        if mem.slot_key.as_str().contains("expertise") {
            let val = mem.value.to_lowercase();
            if val.contains("expert") {
                return KnowledgeLevel::Expert;
            }
            if val.contains("advanced") {
                return KnowledgeLevel::Advanced;
            }
            if val.contains("intermediate") {
                return KnowledgeLevel::Intermediate;
            }
            if val.contains("novice") || val.contains("beginner") {
                return KnowledgeLevel::Novice;
            }
        }
    }

    // Heuristic from message content
    let lower = message.to_lowercase();
    let has_technical_terms = contains_any(
        &lower,
        &[
            "async",
            "mutex",
            "trait",
            "lifetime",
            "borrow",
            "deadlock",
            "regex",
            "pipeline",
            "middleware",
            "dependency injection",
        ],
    );
    let has_basic_questions = contains_any(
        &lower,
        &["what is", "how do i", "beginner", "new to", "first time"],
    );

    if has_basic_questions {
        KnowledgeLevel::Novice
    } else if has_technical_terms {
        KnowledgeLevel::Advanced
    } else {
        KnowledgeLevel::Intermediate
    }
}

fn infer_emotional_need(intent: UserIntent, affect: AffectLabel) -> EmotionalNeed {
    match (intent, affect) {
        (UserIntent::Vent, _) => EmotionalNeed::Empathy,
        (_, AffectLabel::Grateful) => EmotionalNeed::Validation,
        (UserIntent::Learn | UserIntent::Explore, _) => EmotionalNeed::Exploration,
        (UserIntent::Instruct, AffectLabel::Excited) => EmotionalNeed::Speed,
        (UserIntent::Debug | UserIntent::Instruct, _) => EmotionalNeed::Solution,
    }
}

fn infer_constraints(message: &str) -> Vec<String> {
    let lower = message.to_lowercase();
    let mut constraints = Vec::new();

    if contains_any(&lower, &["urgent", "asap", "quickly", "hurry", "deadline"]) {
        constraints.push("time pressure".to_string());
    }
    if contains_any(&lower, &["simple", "easy", "basic", "minimal"]) {
        constraints.push("simplicity preferred".to_string());
    }
    if contains_any(&lower, &["production", "deploy", "release", "live"]) {
        constraints.push("production context".to_string());
    }
    constraints
}

// ── Helpers ─────────────────────────────────────────────────────

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

fn is_question_form(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("how")
        || trimmed.starts_with("why")
        || trimmed.starts_with("what")
        || trimmed.starts_with("where")
        || trimmed.starts_with("when")
        || trimmed.starts_with("can")
        || trimmed.starts_with("could")
        || trimmed.starts_with("do")
        || trimmed.starts_with("does")
        || trimmed.starts_with("is")
}

fn is_imperative(text: &str) -> bool {
    let first_word = text.split_whitespace().next().unwrap_or("");
    matches!(
        first_word,
        "do" | "make"
            | "create"
            | "add"
            | "fix"
            | "update"
            | "remove"
            | "delete"
            | "run"
            | "build"
            | "implement"
            | "write"
            | "show"
            | "list"
            | "change"
            | "set"
            | "move"
            | "copy"
    )
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::affect::AffectReading;

    fn neutral_reading() -> AffectReading {
        AffectReading::neutral()
    }

    fn frustrated_reading() -> AffectReading {
        AffectReading {
            label: AffectLabel::Frustrated,
            valence: -0.6,
            arousal: 0.7,
            dominance: 0.4,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        }
    }

    #[test]
    fn debug_intent_from_error_with_code() {
        let model = infer_user_model(
            "```rust\nfn main() {}\n```\nI'm getting an error here",
            &neutral_reading(),
            &[],
        );
        assert_eq!(model.inferred_intent, UserIntent::Debug);
    }

    #[test]
    fn learn_intent_from_question() {
        let model = infer_user_model("How does the borrow checker work?", &neutral_reading(), &[]);
        assert_eq!(model.inferred_intent, UserIntent::Learn);
    }

    #[test]
    fn instruct_intent_from_imperative() {
        let model = infer_user_model("Add a new endpoint", &neutral_reading(), &[]);
        assert_eq!(model.inferred_intent, UserIntent::Instruct);
    }

    #[test]
    fn explore_intent_from_open_ended() {
        let model = infer_user_model(
            "What if we used a different approach?",
            &neutral_reading(),
            &[],
        );
        assert_eq!(model.inferred_intent, UserIntent::Explore);
    }

    #[test]
    fn vent_intent_from_frustration() {
        let model = infer_user_model(
            "This is so annoying, nothing works",
            &frustrated_reading(),
            &[],
        );
        assert_eq!(model.inferred_intent, UserIntent::Vent);
        assert_eq!(model.emotional_need, EmotionalNeed::Empathy);
    }

    #[test]
    fn novice_from_basic_question() {
        let model = infer_user_model(
            "What is a trait in Rust? I'm new to this",
            &neutral_reading(),
            &[],
        );
        assert_eq!(model.knowledge_level, KnowledgeLevel::Novice);
    }

    #[test]
    fn advanced_from_technical_terms() {
        let model = infer_user_model(
            "The async mutex deadlock in this pipeline",
            &neutral_reading(),
            &[],
        );
        assert_eq!(model.knowledge_level, KnowledgeLevel::Advanced);
    }

    #[test]
    fn time_pressure_constraint_detected() {
        let model = infer_user_model(
            "Fix this urgently, we need to deploy asap",
            &neutral_reading(),
            &[],
        );
        assert!(
            model
                .active_constraints
                .iter()
                .any(|c| c.contains("time pressure"))
        );
    }

    #[test]
    fn render_produces_block() {
        let model = UserMentalModel {
            inferred_intent: UserIntent::Debug,
            knowledge_level: KnowledgeLevel::Advanced,
            emotional_need: EmotionalNeed::Solution,
            active_constraints: vec!["production context".to_string()],
        };
        let block = crate::core::persona::presenter::render_user_model_block(&model);
        assert!(block.contains("[User Model]"));
        assert!(block.contains("Debug"));
        assert!(block.contains("Advanced"));
        assert!(block.contains("production context"));
    }
}
