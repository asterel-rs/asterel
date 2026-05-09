//! Embedded `OpenAPI` 3.1 contract for the gateway's admin HTTP surface.

use serde_json::Value;

/// Returns the embedded admin `OpenAPI` 3.1 specification as a JSON value.
pub(super) fn admin_openapi_contract_json() -> serde_json::Result<Value> {
    serde_json::from_str(include_str!("admin_contract.json"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::admin_openapi_contract_json;

    #[test]
    fn admin_openapi_contract_contains_required_paths() {
        let spec = admin_openapi_contract_json().expect("embedded admin contract should parse");
        assert_eq!(spec["openapi"], "3.1.0");
        assert!(spec["paths"]["/admin/v1/openapi.json"].is_object());
        assert!(spec["paths"]["/admin/v1/runtime"].is_object());
        assert!(spec["paths"]["/admin/v1/sessions"].is_object());
        assert!(spec["paths"]["/admin/v1/governance"].is_object());
        assert!(spec["paths"]["/admin/v1/settings"].is_object());
        assert!(spec["paths"]["/admin/v1/companions"].is_object());
        assert!(spec["components"]["schemas"]["RuntimeStatus"].is_object());
        assert!(spec["components"]["schemas"]["Session"].is_object());
        assert!(spec["components"]["schemas"]["GovernanceSummary"].is_object());
        assert!(spec["components"]["schemas"]["TenantRegistryRow"].is_object());
        assert!(spec["components"]["schemas"]["TenantContextResponse"].is_object());
        assert!(spec["components"]["schemas"]["TenantContextUpdateResponse"].is_object());
    }

    #[test]
    fn admin_openapi_contract_has_stable_path_set() {
        let spec = admin_openapi_contract_json().expect("embedded admin contract should parse");
        let paths = spec["paths"]
            .as_object()
            .expect("admin paths should be an object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();

        let expected = BTreeSet::from([
            "/admin/v1/openapi.json".to_string(),
            "/admin/v1/runtime".to_string(),
            "/admin/v1/usage".to_string(),
            "/admin/v1/mood".to_string(),
            "/admin/v1/activity".to_string(),
            "/admin/v1/agents".to_string(),
            "/admin/v1/gateway/restart".to_string(),
            "/admin/v1/sessions".to_string(),
            "/admin/v1/sessions/{session_id}".to_string(),
            "/admin/v1/sessions/{session_id}/messages".to_string(),
            "/admin/v1/governance".to_string(),
            "/admin/v1/auth/profiles".to_string(),
            "/admin/v1/auth/profiles/{id}".to_string(),
            "/admin/v1/providers".to_string(),
            "/admin/v1/providers/{id}".to_string(),
            "/admin/v1/settings".to_string(),
            "/admin/v1/channels".to_string(),
            "/admin/v1/channels/{channel_id}".to_string(),
            "/admin/v1/channels/{channel_id}/actions".to_string(),
            "/admin/v1/skills".to_string(),
            "/admin/v1/skills/install".to_string(),
            "/admin/v1/skills/{skill_id}".to_string(),
            "/admin/v1/cron/jobs".to_string(),
            "/admin/v1/cron/jobs/{job_id}".to_string(),
            "/admin/v1/cron/jobs/{job_id}/run".to_string(),
            "/admin/v1/memory/entities".to_string(),
            "/admin/v1/memory/consolidation".to_string(),
            "/admin/v1/memory/exposure".to_string(),
            "/admin/v1/memory/self-amendments".to_string(),
            "/admin/v1/memory/self-amendments/approve".to_string(),
            "/admin/v1/memory/entities/{entity_id}/slots".to_string(),
            "/admin/v1/memory/correct".to_string(),
            "/admin/v1/memory/forget".to_string(),
            "/admin/v1/memory/checkpoint".to_string(),
            "/admin/v1/companions".to_string(),
            "/admin/v1/companions/{scope}/captions".to_string(),
            "/admin/v1/companions/{scope}/widgets".to_string(),
            "/admin/v1/companions/{scope}/windows".to_string(),
            "/admin/v1/companions/{scope}/windows/{window_id}/confirm".to_string(),
            "/admin/v1/companions/{scope}/windows/{window_id}/cancel".to_string(),
            "/admin/v1/companions/{scope}/ingress".to_string(),
            "/admin/v1/tenants".to_string(),
            "/admin/v1/tenant-context".to_string(),
            "/admin/v1/uploads".to_string(),
        ]);

        assert_eq!(paths, expected);
    }

    #[test]
    fn memory_exposure_contract_redacts_sensitive_counts() {
        let spec = admin_openapi_contract_json().expect("embedded admin contract should parse");
        let schema = &spec["components"]["schemas"]["MemoryExposureStatusResponse"];

        assert_eq!(
            schema["required"],
            serde_json::json!(["observed_builds", "sensitive_counts_redacted"])
        );
        assert!(schema["properties"]["observed_builds"].is_object());
        assert!(schema["properties"]["sensitive_counts_redacted"].is_object());
        assert!(schema["properties"]["private_internal_total"].is_null());
        assert!(schema["properties"]["secret_suppressed_total"].is_null());
        assert!(schema["properties"]["last_projection"].is_null());
    }

    #[test]
    fn self_amendment_review_contract_marks_payloads_redacted() {
        let spec = admin_openapi_contract_json().expect("embedded admin contract should parse");
        let schema = &spec["components"]["schemas"]["SelfAmendmentReviewResponse"];
        let item_schema = &spec["components"]["schemas"]["SelfAmendmentCandidateReview"];

        assert_eq!(
            schema["required"],
            serde_json::json!([
                "count",
                "items",
                "raw_payloads_redacted",
                "durable_writes_enabled"
            ])
        );
        assert!(item_schema["properties"]["raw_payload"].is_null());
        assert!(item_schema["properties"]["user_message"].is_null());
        assert!(item_schema["properties"]["assistant_response"].is_null());
        assert!(item_schema["properties"]["raw_payload_redacted"].is_object());
        assert!(spec["components"]["schemas"]["SelfAmendmentApproveRequest"].is_object());
        assert!(spec["components"]["schemas"]["SelfAmendmentApprovalResponse"].is_object());
    }
}
