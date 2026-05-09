//! Verdict strings — policy decisions, security blocks, URL validation,
//! payload validation, and writeback guard messages shared across modules.

pub(crate) const SECURITY_POLICY_BLOCK_PREFIX: &str = "blocked by security policy: ";

pub(crate) const WRITE_POLICY_REQUIRES_SOURCE_REF: &str = "write policy requires source_ref";
pub(crate) const WRITE_POLICY_SOURCE_REF_MUST_NOT_BE_EMPTY: &str =
    "write policy source_ref must not be empty";
pub(crate) const WRITE_POLICY_REQUIRES_PROVENANCE: &str = "write policy requires provenance";
pub(crate) const WRITE_POLICY_REQUIRES_PROVENANCE_SOURCE_CLASS_MATCH_SOURCE: &str =
    "write policy requires provenance.source_class to match source";
pub(crate) const WRITE_POLICY_REQUIRES_PROVENANCE_REFERENCE: &str =
    "write policy requires provenance.reference";

pub(crate) const PERSONA_WB_REQUIRES_SOURCE_SYSTEM: &str =
    "persona writeback policy requires source=system";
pub(crate) const PERSONA_WB_REQUIRES_PRIVACY_PRIVATE: &str =
    "persona writeback policy requires privacy_level=private";
pub(crate) const PERSONA_WB_REQUIRES_SOURCE_KIND_MANUAL: &str =
    "persona writeback policy requires source_kind=manual";
pub(crate) const PERSONA_WB_REQUIRES_PROVENANCE_SOURCE_SYSTEM: &str =
    "persona writeback policy requires provenance.source_class=system";
pub(crate) const PERSONA_WB_ENTITY_ID_MISMATCH: &str =
    "persona writeback policy entity_id mismatch";
pub(crate) const PERSONA_WB_WRITEBACK_REQUIRES_SUMMARY_COMPACTED: &str =
    "persona writeback entries must use event_type=summary_compacted";
pub(crate) const PERSONA_WB_CANONICAL_REQUIRES_FACT_UPDATED: &str =
    "persona canonical state writes must use event_type=fact_updated";
pub(crate) const PERSONA_WB_STYLE_PROFILE_REQUIRES_FACT_UPDATED: &str =
    "persona style profile writes must use event_type=fact_updated";
pub(crate) const PERSONA_WB_RELATIONSHIP_REQUIRES_FACT_UPDATED: &str =
    "persona relationship writes must use event_type=fact_updated";
pub(crate) const PERSONA_WB_WORLD_MODEL_REQUIRES_FACT_UPDATED: &str =
    "persona world_model writes must use event_type=fact_updated";
pub(crate) const PERSONA_WB_INFERRED_REQUIRES_INFERRED_CLAIM: &str =
    "persona inferred writes must use event_type=inferred_claim";
pub(crate) const PERSONA_WB_REJECTED_PROTECTED_SELF_EDIT: &str =
    "persona writeback policy rejects protected self-edit slot";
pub(crate) const PERSONA_WB_REJECTED_SLOT_KEY: &str = "persona writeback policy rejected slot_key";

pub(crate) const TOOL_WB_REJECTS_PRIVACY_SECRET: &str =
    "tool memory write policy rejects privacy_level=secret";
pub(crate) const TOOL_WB_REQUIRES_SOURCE_KIND_MANUAL: &str =
    "tool memory write policy requires source_kind=manual";

pub(crate) const EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_EXPLICIT_USER: &str =
    "external autosave policy requires source=explicit_user";
pub(crate) const EXTERNAL_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE: &str =
    "external autosave policy requires privacy_level=private";
pub(crate) const EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_KIND: &str =
    "external autosave policy requires source_kind";
pub(crate) const EXTERNAL_AUTOSAVE_REJECTED_SOURCE_KIND: &str =
    "external autosave policy rejected source_kind";

pub(crate) const AGENT_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE: &str =
    "agent autosave policy requires privacy_level=private";
pub(crate) const AGENT_AUTOSAVE_REQUIRES_SOURCE_KIND_CONVERSATION: &str =
    "agent autosave policy requires source_kind=conversation";
pub(crate) const AGENT_AUTOSAVE_REQUIRES_EVENT_TYPE_FACT_ADDED: &str =
    "agent autosave policy requires event_type=fact_added";
pub(crate) const AGENT_AUTOSAVE_REJECTED_SLOT_KEY: &str = "agent autosave policy rejected slot_key";
pub(crate) const AGENT_AUTOSAVE_REJECTED_SOURCE: &str = "agent autosave policy rejected source";

pub(crate) const CONVERSATION_STATE_REQUIRES_SOURCE_SYSTEM: &str =
    "conversation state write policy requires source=system";
pub(crate) const CONVERSATION_STATE_REQUIRES_PRIVACY_PRIVATE: &str =
    "conversation state write policy requires privacy_level=private";
pub(crate) const CONVERSATION_STATE_REQUIRES_SOURCE_KIND_CONVERSATION: &str =
    "conversation state write policy requires source_kind=conversation";
pub(crate) const CONVERSATION_STATE_REQUIRES_EVENT_TYPE_FACT_UPDATED: &str =
    "conversation state write policy requires event_type=fact_updated";
pub(crate) const CONVERSATION_STATE_REJECTED_SLOT_KEY: &str =
    "conversation state write policy rejected slot_key";

pub(crate) const INFERENCE_WB_REQUIRES_PRIVACY_PRIVATE: &str =
    "inference write policy requires privacy_level=private";
pub(crate) const INFERENCE_WB_REQUIRES_SOURCE_KIND_CONVERSATION: &str =
    "inference write policy requires source_kind=conversation";
pub(crate) const INFERENCE_WB_REJECTED_SOURCE: &str = "inference write policy rejected source";
pub(crate) const INFERENCE_WB_REJECTED_EVENT_TYPE: &str =
    "inference write policy rejected event_type";

pub(crate) const USER_INFERENCE_REQUIRES_SOURCE_SYSTEM: &str =
    "user inference write policy requires source=system";
pub(crate) const USER_INFERENCE_REQUIRES_PRIVACY_PRIVATE: &str =
    "user inference write policy requires privacy_level=private";
pub(crate) const USER_INFERENCE_REQUIRES_SOURCE_KIND_MANUAL: &str =
    "user inference write policy requires source_kind=manual";
pub(crate) const USER_INFERENCE_REQUIRES_EVENT_TYPE_INFERRED_CLAIM: &str =
    "user inference write policy requires event_type=inferred_claim";
pub(crate) const USER_INFERENCE_ENTITY_ID_MISMATCH: &str =
    "user inference write policy entity_id mismatch";
pub(crate) const USER_INFERENCE_REQUIRES_USER_PREFIX: &str =
    "user inference write policy requires slot_key prefix user.";
pub(crate) const USER_INFERENCE_REJECTED_SLOT_KEY_FORMAT: &str =
    "user inference write policy rejected slot_key format";
pub(crate) const USER_INFERENCE_REJECTED_RESERVED_SLOT_KEY: &str =
    "user inference write policy rejected reserved slot_key";

pub(crate) const VERIFY_REPAIR_REQUIRES_SOURCE_SYSTEM: &str =
    "verify-repair write policy requires source=system";
pub(crate) const VERIFY_REPAIR_REQUIRES_PRIVACY_PRIVATE: &str =
    "verify-repair write policy requires privacy_level=private";
pub(crate) const VERIFY_REPAIR_REQUIRES_SOURCE_KIND_MANUAL: &str =
    "verify-repair write policy requires source_kind=manual";
pub(crate) const VERIFY_REPAIR_REJECTED_SLOT_KEY: &str =
    "verify-repair write policy rejected slot_key";
pub(crate) const VERIFY_REPAIR_REQUIRES_EVENT_TYPE_SUMMARY_COMPACTED: &str =
    "verify-repair write policy requires event_type=summary_compacted";

pub(crate) const INGESTION_REQUIRES_EVENT_TYPE_FACT_ADDED: &str =
    "ingestion write policy requires event_type=fact_added";
pub(crate) const INGESTION_REJECTED_SOURCE: &str = "ingestion write policy rejected source";
pub(crate) const INGESTION_REQUIRES_SOURCE_KIND: &str =
    "ingestion write policy requires source_kind";
pub(crate) const INGESTION_REQUIRES_EXTERNAL_SLOT_KEY_PREFIX: &str =
    "ingestion write policy requires external slot_key prefix";

pub(crate) const URL_REQUIRES_HTTPS: &str = "only https:// URLs are allowed";
pub(crate) const URL_REQUIRES_HTTP_OR_HTTPS: &str = "only http:// and https:// URLs are allowed";
pub(crate) const URL_USERINFO_NOT_ALLOWED: &str = "URL userinfo is not allowed";
pub(crate) const URL_HAS_NO_HOST: &str = "URL has no host";
pub(crate) const SSRF_BLOCK_PREFIX: &str = "SSRF blocked:";

pub(crate) const SECURITY_BLOCK_GLOBAL_ACTION_LIMIT_EXCEEDED: &str =
    "blocked by security policy: global action limit exceeded";
pub(crate) const SECURITY_BLOCK_AUTONOMY_READ_ONLY: &str =
    "blocked by security policy: autonomy is read-only";
pub(crate) const TOOL_OUTPUT_BLOCKED_BY_EXTERNAL_CONTENT_POLICY: &str =
    "tool output blocked by external-content policy";
pub(crate) const TOOL_ERROR_BLOCKED_BY_EXTERNAL_CONTENT_POLICY: &str =
    "tool error blocked by external-content policy";
pub(crate) const EXTERNAL_CONTENT_BLOCKED_BY_SAFETY_POLICY: &str =
    "External content blocked by safety policy";

pub(crate) const ACTION_LIMIT_EXCEEDED_ERROR: &str =
    "blocked by security policy: action limit exceeded";
pub(crate) const COST_LIMIT_EXCEEDED_ERROR: &str =
    "blocked by security policy: daily cost limit exceeded";
pub(crate) const TENANT_RECALL_SCOPE_MISMATCH: &str =
    "blocked by security policy: tenant recall scope mismatch";
pub(crate) const TENANT_DEFAULT_RECALL_FORBIDDEN: &str =
    "blocked by security policy: tenant mode forbids default recall scope";

pub(crate) const PAYLOAD_STATE_HEADER_REQUIRED: &str = "payload.state_header is required";
pub(crate) const PAYLOAD_STATE_HEADER_MUST_BE_OBJECT: &str =
    "payload.state_header must be an object";
pub(crate) const PAYLOAD_MEMORY_APPEND_MUST_BE_ARRAY: &str =
    "payload.memory_append must be an array";
pub(crate) const PAYLOAD_SELF_TASKS_MUST_BE_ARRAY: &str = "payload.self_tasks must be an array";
pub(crate) const PAYLOAD_MUST_BE_JSON_OBJECT: &str = "payload must be a JSON object";
pub(crate) const PAYLOAD_LAST_UPDATED_AT_MUST_BE_RFC3339: &str =
    "payload.state_header.last_updated_at must be RFC3339";

pub(crate) const PAYLOAD_STYLE_PROFILE_MUST_BE_OBJECT: &str =
    "payload.style_profile must be an object";
pub(crate) const PAYLOAD_STYLE_PROFILE_FORMALITY_MUST_BE_INTEGER: &str =
    "payload.style_profile.formality must be an integer";
pub(crate) const PAYLOAD_STYLE_PROFILE_VERBOSITY_MUST_BE_INTEGER: &str =
    "payload.style_profile.verbosity must be an integer";
pub(crate) const PAYLOAD_STYLE_PROFILE_TEMPERATURE_MUST_BE_NUMBER: &str =
    "payload.style_profile.temperature must be a number";
pub(crate) const PAYLOAD_STYLE_PROFILE_FORMALITY_OUT_OF_RANGE: &str =
    "payload.style_profile.formality must be in safe range";
pub(crate) const PAYLOAD_STYLE_PROFILE_VERBOSITY_OUT_OF_RANGE: &str =
    "payload.style_profile.verbosity must be in safe range";
pub(crate) const PAYLOAD_STYLE_PROFILE_TEMPERATURE_OUT_OF_RANGE: &str =
    "payload.style_profile.temperature must be in safe range";
pub(crate) const PAYLOAD_STATE_HEADER_IDENTITY_HASH_MUST_BE_STRING: &str =
    "payload.state_header.identity_principles_hash must be a string";
pub(crate) const IMMUTABLE_MISMATCH_IDENTITY_HASH: &str =
    "immutable field mismatch: payload.state_header.identity_principles_hash";
pub(crate) const PAYLOAD_STATE_HEADER_SAFETY_POSTURE_MUST_BE_STRING: &str =
    "payload.state_header.safety_posture must be a string";
pub(crate) const IMMUTABLE_MISMATCH_SAFETY_POSTURE: &str =
    "immutable field mismatch: payload.state_header.safety_posture";

pub(crate) const PAYLOAD_SOURCE_IDENTITY_FORBIDDEN_SUFFIX: &str =
    "is forbidden; writeback cannot modify source identity";

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn all_constants() -> Vec<&'static str> {
        vec![
            SECURITY_POLICY_BLOCK_PREFIX,
            WRITE_POLICY_REQUIRES_SOURCE_REF,
            WRITE_POLICY_SOURCE_REF_MUST_NOT_BE_EMPTY,
            WRITE_POLICY_REQUIRES_PROVENANCE,
            WRITE_POLICY_REQUIRES_PROVENANCE_SOURCE_CLASS_MATCH_SOURCE,
            WRITE_POLICY_REQUIRES_PROVENANCE_REFERENCE,
            PERSONA_WB_REQUIRES_SOURCE_SYSTEM,
            PERSONA_WB_REQUIRES_PRIVACY_PRIVATE,
            PERSONA_WB_REQUIRES_SOURCE_KIND_MANUAL,
            PERSONA_WB_REQUIRES_PROVENANCE_SOURCE_SYSTEM,
            PERSONA_WB_ENTITY_ID_MISMATCH,
            PERSONA_WB_WRITEBACK_REQUIRES_SUMMARY_COMPACTED,
            PERSONA_WB_CANONICAL_REQUIRES_FACT_UPDATED,
            PERSONA_WB_STYLE_PROFILE_REQUIRES_FACT_UPDATED,
            PERSONA_WB_RELATIONSHIP_REQUIRES_FACT_UPDATED,
            PERSONA_WB_WORLD_MODEL_REQUIRES_FACT_UPDATED,
            PERSONA_WB_INFERRED_REQUIRES_INFERRED_CLAIM,
            PERSONA_WB_REJECTED_SLOT_KEY,
            TOOL_WB_REJECTS_PRIVACY_SECRET,
            TOOL_WB_REQUIRES_SOURCE_KIND_MANUAL,
            EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_EXPLICIT_USER,
            EXTERNAL_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE,
            EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_KIND,
            EXTERNAL_AUTOSAVE_REJECTED_SOURCE_KIND,
            AGENT_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE,
            AGENT_AUTOSAVE_REQUIRES_SOURCE_KIND_CONVERSATION,
            AGENT_AUTOSAVE_REQUIRES_EVENT_TYPE_FACT_ADDED,
            AGENT_AUTOSAVE_REJECTED_SLOT_KEY,
            AGENT_AUTOSAVE_REJECTED_SOURCE,
            CONVERSATION_STATE_REQUIRES_SOURCE_SYSTEM,
            CONVERSATION_STATE_REQUIRES_PRIVACY_PRIVATE,
            CONVERSATION_STATE_REQUIRES_SOURCE_KIND_CONVERSATION,
            CONVERSATION_STATE_REQUIRES_EVENT_TYPE_FACT_UPDATED,
            CONVERSATION_STATE_REJECTED_SLOT_KEY,
            INFERENCE_WB_REQUIRES_PRIVACY_PRIVATE,
            INFERENCE_WB_REQUIRES_SOURCE_KIND_CONVERSATION,
            INFERENCE_WB_REJECTED_SOURCE,
            INFERENCE_WB_REJECTED_EVENT_TYPE,
            USER_INFERENCE_REQUIRES_SOURCE_SYSTEM,
            USER_INFERENCE_REQUIRES_PRIVACY_PRIVATE,
            USER_INFERENCE_REQUIRES_SOURCE_KIND_MANUAL,
            USER_INFERENCE_REQUIRES_EVENT_TYPE_INFERRED_CLAIM,
            USER_INFERENCE_ENTITY_ID_MISMATCH,
            USER_INFERENCE_REQUIRES_USER_PREFIX,
            USER_INFERENCE_REJECTED_SLOT_KEY_FORMAT,
            USER_INFERENCE_REJECTED_RESERVED_SLOT_KEY,
            VERIFY_REPAIR_REQUIRES_SOURCE_SYSTEM,
            VERIFY_REPAIR_REQUIRES_PRIVACY_PRIVATE,
            VERIFY_REPAIR_REQUIRES_SOURCE_KIND_MANUAL,
            VERIFY_REPAIR_REJECTED_SLOT_KEY,
            VERIFY_REPAIR_REQUIRES_EVENT_TYPE_SUMMARY_COMPACTED,
            INGESTION_REQUIRES_EVENT_TYPE_FACT_ADDED,
            INGESTION_REJECTED_SOURCE,
            INGESTION_REQUIRES_SOURCE_KIND,
            INGESTION_REQUIRES_EXTERNAL_SLOT_KEY_PREFIX,
            URL_REQUIRES_HTTPS,
            URL_REQUIRES_HTTP_OR_HTTPS,
            URL_USERINFO_NOT_ALLOWED,
            URL_HAS_NO_HOST,
            SSRF_BLOCK_PREFIX,
            SECURITY_BLOCK_GLOBAL_ACTION_LIMIT_EXCEEDED,
            SECURITY_BLOCK_AUTONOMY_READ_ONLY,
            TOOL_OUTPUT_BLOCKED_BY_EXTERNAL_CONTENT_POLICY,
            TOOL_ERROR_BLOCKED_BY_EXTERNAL_CONTENT_POLICY,
            EXTERNAL_CONTENT_BLOCKED_BY_SAFETY_POLICY,
            ACTION_LIMIT_EXCEEDED_ERROR,
            COST_LIMIT_EXCEEDED_ERROR,
            TENANT_RECALL_SCOPE_MISMATCH,
            TENANT_DEFAULT_RECALL_FORBIDDEN,
            PAYLOAD_STATE_HEADER_REQUIRED,
            PAYLOAD_STATE_HEADER_MUST_BE_OBJECT,
            PAYLOAD_MEMORY_APPEND_MUST_BE_ARRAY,
            PAYLOAD_SELF_TASKS_MUST_BE_ARRAY,
            PAYLOAD_MUST_BE_JSON_OBJECT,
            PAYLOAD_LAST_UPDATED_AT_MUST_BE_RFC3339,
            PAYLOAD_STYLE_PROFILE_MUST_BE_OBJECT,
            PAYLOAD_STYLE_PROFILE_FORMALITY_MUST_BE_INTEGER,
            PAYLOAD_STYLE_PROFILE_VERBOSITY_MUST_BE_INTEGER,
            PAYLOAD_STYLE_PROFILE_TEMPERATURE_MUST_BE_NUMBER,
            PAYLOAD_STYLE_PROFILE_FORMALITY_OUT_OF_RANGE,
            PAYLOAD_STYLE_PROFILE_VERBOSITY_OUT_OF_RANGE,
            PAYLOAD_STYLE_PROFILE_TEMPERATURE_OUT_OF_RANGE,
            PAYLOAD_STATE_HEADER_IDENTITY_HASH_MUST_BE_STRING,
            IMMUTABLE_MISMATCH_IDENTITY_HASH,
            PAYLOAD_STATE_HEADER_SAFETY_POSTURE_MUST_BE_STRING,
            IMMUTABLE_MISMATCH_SAFETY_POSTURE,
            PAYLOAD_SOURCE_IDENTITY_FORBIDDEN_SUFFIX,
        ]
    }

    #[test]
    fn security_policy_block_prefix_ends_with_colon_space() {
        assert!(SECURITY_POLICY_BLOCK_PREFIX.ends_with(": "));
    }

    #[test]
    fn security_block_constants_start_with_security_policy_prefix() {
        let constants = [
            SECURITY_BLOCK_GLOBAL_ACTION_LIMIT_EXCEEDED,
            SECURITY_BLOCK_AUTONOMY_READ_ONLY,
        ];

        for value in constants {
            assert!(
                value.starts_with(SECURITY_POLICY_BLOCK_PREFIX),
                "constant must start with SECURITY_POLICY_BLOCK_PREFIX: {value}"
            );
        }
    }

    #[test]
    fn constants_have_no_duplicate_values() {
        let constants = all_constants();
        let unique: HashSet<&str> = constants.iter().copied().collect();
        assert_eq!(unique.len(), constants.len());
    }

    #[test]
    fn ssrf_block_prefix_ends_with_colon() {
        // SSRF prefix uses colon-only (no trailing space) because callers
        // supply the space in format strings: `"{SSRF_BLOCK_PREFIX} ..."` .
        assert!(SSRF_BLOCK_PREFIX.ends_with(':'));
        assert!(!SSRF_BLOCK_PREFIX.ends_with(": "));
    }
}
