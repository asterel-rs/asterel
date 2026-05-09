//! Data-model contract strings used across memory/entity boundaries.

pub(crate) const SLOT_CONVERSATION_STATE_V1: &str = "conversation.state.v1";
pub(crate) const SLOT_CONVERSATION_LEDGER_V1: &str = "conversation.ledger.v1";
pub(crate) const SLOT_VERIFY_REPAIR_ESCALATION: &str = "autonomy.verify_repair.escalation";
pub(crate) const SLOT_EMBODIED_STATE_V1: &str = "persona.writeback.embodied_state.v1";
pub(crate) const SLOT_ROLLBACK_DRILL_LATEST: &str = "persona.writeback.rollback_drill.latest";
pub(crate) const SLOT_METACOGNITION_CALIBRATION_V1: &str =
    "persona.writeback.metacognition.calibration.v1";
pub(crate) const SLOT_METACOGNITION_TURN_PREFIX: &str = "persona.writeback.metacognition.turn";
pub(crate) const SLOT_STYLE_PROFILE_ADAPTATION: &str = "persona.writeback.style_profile.adaptation";
pub(crate) const PREFIX_SELF_AMENDMENT_SLOT: &str = "persona.writeback.self_amendment.";
pub(crate) const SLOT_CONSOLIDATION_SEMANTIC_LATEST: &str = "consolidation.semantic.latest";
pub(crate) const SLOT_MILESTONES_V1: &str = "persona.milestones.v1";
pub(crate) const SLOT_CONVERSATION_USER_MSG: &str = "conversation.user_msg";
pub(crate) const SLOT_CONVERSATION_ASSISTANT_RESP: &str = "conversation.assistant_resp";
pub(crate) const SLOT_PERSONA_AFFECT_ARC_V1: &str = "persona.affect_arc/v1";
pub(crate) const SLOT_PERSONA_SESSION_MOOD_V1: &str = "persona.session_mood/v1";
pub(crate) const SLOT_PERSONA_EMOTIONAL_MEMORY_V1: &str = "persona.emotional_memory/v1";
pub(crate) const SLOT_PERSONA_EMOTIONAL_IDENTITY_V1: &str = "persona.emotional_identity/v1";
pub(crate) const SLOT_EXTERNAL_GATEWAY_WEBHOOK: &str = "external.gateway.webhook";
pub(crate) const SLOT_USER_FACT_NAME_SUFFIX: &str = "name";
pub(crate) const SLOT_USER_FACT_LANGUAGE_SUFFIX: &str = "language";
pub(crate) const SLOT_USER_FACT_STYLE_PREF_SUFFIX: &str = "response_style";
pub(crate) const SLOT_USER_FACT_PROJECTS_SUFFIX: &str = "ongoing_projects";

pub(crate) const SOURCE_REF_USER_FACT_UPDATE: &str = "persona.user_fact.update";
pub(crate) const SOURCE_REF_USER_FACT_WRITEBACK: &str = "persona.user_fact.writeback";

pub(crate) const SOURCE_REF_CONVERSATION_STATE_UPDATE: &str = "conversation.state.update";
pub(crate) const SOURCE_REF_CONVERSATION_LEDGER_UPDATE: &str = "conversation.ledger.update";
#[cfg(feature = "taste")]
pub(crate) const SOURCE_REF_TASTE_VALUE_PROFILE_UPDATE: &str = "taste.value_profile.update";
pub(crate) const SOURCE_REF_EXPERIENCE_INGEST: &str = "experience.ingest";
pub(crate) const SOURCE_REF_EXPERIENCE_DISTILL: &str = "experience.distill";
pub(crate) const SOURCE_REF_AGENT_AUTOSAVE_USER_MSG: &str = "agent.autosave.user_msg";
pub(crate) const SOURCE_REF_AGENT_AUTOSAVE_ASSISTANT_RESP: &str = "agent.autosave.assistant_resp";
pub(crate) const SOURCE_REF_AUGMENT_OUTCOME_RECORD: &str = "augment.outcome_record";
pub(crate) const SOURCE_REF_PERSONA_REFLECT_MEMORY_INFERENCE: &str =
    "persona.reflect.memory_inference";
pub(crate) const SOURCE_REF_PERSONA_REFLECT_USER_INFERENCE: &str = "persona.reflect.user_inference";

pub(crate) const SOURCE_PERSONA_WORLD_MODEL_UPDATE: &str = "persona.world_model.update";
pub(crate) const SOURCE_PERSONA_WORLD_MODEL_WRITEBACK: &str = "persona.world_model.writeback";
pub(crate) const SOURCE_PERSONA_USER_KNOWLEDGE_UPDATE: &str = "persona.user_knowledge.update";
pub(crate) const SOURCE_PERSONA_USER_KNOWLEDGE_WRITEBACK: &str = "persona.user_knowledge.writeback";
pub(crate) const SOURCE_PERSONA_RELATIONSHIP_UPDATE: &str = "persona.relationship.update";
pub(crate) const SOURCE_PERSONA_RELATIONSHIP_WRITEBACK: &str = "persona.relationship.writeback";
pub(crate) const SOURCE_PERSONA_STYLE_PROFILE_WRITEBACK: &str = "persona.style_profile.writeback";
pub(crate) const SOURCE_PERSONA_STYLE_PROFILE_ADAPTATION: &str = "persona.style_profile.adaptation";
pub(crate) const SOURCE_EXPERIENCE_AGGREGATE: &str = "experience.aggregate";
pub(crate) const SOURCE_EXPERIENCE_ATOM_INGESTION: &str = "experience.atom.ingestion";
pub(crate) const SOURCE_EXPERIENCE_DISTILL_PRINCIPLE: &str = "experience.distill.principle";

pub(crate) const PREFIX_PRINCIPLE_SLOT: &str = "principle.";
pub(crate) const PREFIX_EXPERIENCE_SLOT: &str = "experience.";
pub(crate) const PREFIX_PERSONA_WRITEBACK: &str = "persona.writeback.";
pub(crate) const PREFIX_PERSONA_SLASH: &str = "persona/";
pub(crate) const SUFFIX_USER_KNOWLEDGE_SLOT_V1: &str = "/user_knowledge/v1";
pub(crate) const PREFIX_CONVERSATION: &str = "conversation.";
pub(crate) const PREFIX_OUTCOME_RECORD_SLOT: &str = "turn_outcome.";
pub(crate) const PREFIX_EXTERNAL: &str = "external.";
pub(crate) const PREFIX_USER_SLOT: &str = "user.";

pub(crate) const SUFFIX_NARRATIVE_V1: &str = "/narrative/v1";

pub(crate) const ENTITY_PREFIX_PERSON: &str = "person:";
pub(crate) const ENTITY_PREFIX_USER: &str = "user:";

pub(crate) const PERSONA_ROLLBACK_LATEST_SLOT_GLOB: &str = "persona/%/state_header/rollback/latest";

pub(crate) const PREFIX_AUTONOMY: &str = "autonomy.";
pub(crate) const PREFIX_SYSTEM: &str = "system.";
pub(crate) const PREFIX_SECURITY: &str = "security.";

pub(crate) const RESERVED_SLOT_PREFIXES: [&str; 5] = [
    PREFIX_CONVERSATION,
    PREFIX_AUTONOMY,
    PREFIX_PERSONA_SLASH,
    PREFIX_SYSTEM,
    PREFIX_SECURITY,
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn prefix_constants_end_with_separator() {
        let prefixes = [
            PREFIX_PRINCIPLE_SLOT,
            PREFIX_EXPERIENCE_SLOT,
            PREFIX_PERSONA_WRITEBACK,
            PREFIX_SELF_AMENDMENT_SLOT,
            PREFIX_PERSONA_SLASH,
            PREFIX_CONVERSATION,
            PREFIX_OUTCOME_RECORD_SLOT,
            PREFIX_EXTERNAL,
            PREFIX_USER_SLOT,
            PREFIX_AUTONOMY,
            PREFIX_SYSTEM,
            PREFIX_SECURITY,
        ];

        for prefix in prefixes {
            assert!(
                prefix.ends_with('.') || prefix.ends_with('/'),
                "prefix must end with '.' or '/': {prefix}"
            );
        }
    }

    #[test]
    fn slot_constants_have_no_duplicate_values() {
        let slots = [
            SLOT_CONVERSATION_STATE_V1,
            SLOT_CONVERSATION_LEDGER_V1,
            SLOT_VERIFY_REPAIR_ESCALATION,
            SLOT_EMBODIED_STATE_V1,
            SLOT_ROLLBACK_DRILL_LATEST,
            SLOT_METACOGNITION_CALIBRATION_V1,
            SLOT_METACOGNITION_TURN_PREFIX,
            SLOT_STYLE_PROFILE_ADAPTATION,
            PREFIX_SELF_AMENDMENT_SLOT,
            SLOT_CONSOLIDATION_SEMANTIC_LATEST,
            SLOT_MILESTONES_V1,
            SLOT_CONVERSATION_USER_MSG,
            SLOT_CONVERSATION_ASSISTANT_RESP,
            SLOT_PERSONA_AFFECT_ARC_V1,
            SLOT_PERSONA_SESSION_MOOD_V1,
            SLOT_PERSONA_EMOTIONAL_MEMORY_V1,
            SLOT_PERSONA_EMOTIONAL_IDENTITY_V1,
            SLOT_EXTERNAL_GATEWAY_WEBHOOK,
            SLOT_USER_FACT_NAME_SUFFIX,
            SLOT_USER_FACT_LANGUAGE_SUFFIX,
            SLOT_USER_FACT_STYLE_PREF_SUFFIX,
            SLOT_USER_FACT_PROJECTS_SUFFIX,
        ];

        let unique: HashSet<&str> = slots.into_iter().collect();
        assert_eq!(unique.len(), slots.len());
    }

    #[test]
    fn reserved_slot_prefixes_end_with_separator() {
        for prefix in RESERVED_SLOT_PREFIXES {
            assert!(
                prefix.ends_with('.') || prefix.ends_with('/'),
                "reserved prefix must end with '.' or '/': {prefix}"
            );
        }
    }

    #[test]
    fn user_knowledge_slot_suffix_uses_slash_namespace() {
        assert!(SUFFIX_USER_KNOWLEDGE_SLOT_V1.starts_with('/'));
        assert!(SUFFIX_USER_KNOWLEDGE_SLOT_V1.ends_with("/v1"));
    }

    #[test]
    fn entity_prefix_constants_end_with_colon() {
        assert!(ENTITY_PREFIX_PERSON.ends_with(':'));
        assert!(ENTITY_PREFIX_USER.ends_with(':'));
    }

    #[test]
    fn rollback_latest_slot_glob_uses_sql_wildcard() {
        assert!(PERSONA_ROLLBACK_LATEST_SLOT_GLOB.contains('%'));
    }

    #[test]
    fn source_ref_constants_use_dot_notation() {
        let source_refs = [
            SOURCE_REF_CONVERSATION_STATE_UPDATE,
            SOURCE_REF_CONVERSATION_LEDGER_UPDATE,
            SOURCE_REF_EXPERIENCE_INGEST,
            SOURCE_REF_EXPERIENCE_DISTILL,
            SOURCE_REF_AGENT_AUTOSAVE_USER_MSG,
            SOURCE_REF_AGENT_AUTOSAVE_ASSISTANT_RESP,
            SOURCE_REF_AUGMENT_OUTCOME_RECORD,
            SOURCE_REF_PERSONA_REFLECT_MEMORY_INFERENCE,
            SOURCE_REF_PERSONA_REFLECT_USER_INFERENCE,
            SOURCE_REF_USER_FACT_UPDATE,
            SOURCE_REF_USER_FACT_WRITEBACK,
        ];

        for source_ref in source_refs {
            assert!(
                source_ref.contains('.'),
                "source_ref must use dot notation: {source_ref}"
            );
        }

        #[cfg(feature = "taste")]
        assert!(SOURCE_REF_TASTE_VALUE_PROFILE_UPDATE.contains('.'));
    }

    #[test]
    fn suffix_constant_starts_with_slash() {
        assert!(
            SUFFIX_NARRATIVE_V1.starts_with('/'),
            "suffix must start with '/': {SUFFIX_NARRATIVE_V1}"
        );
    }
}
