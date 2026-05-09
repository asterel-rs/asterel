//! Embedded `OpenAPI` 3.1 contract for the gateway's public HTTP surface.
use serde_json::Value;

/// Returns the embedded `OpenAPI` 3.1 specification as a JSON value.
pub(super) fn openapi_contract_json() -> serde_json::Result<Value> {
    serde_json::from_str(include_str!("contract.json"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::openapi_contract_json;

    #[test]
    fn openapi_contract_contains_required_paths() {
        let spec = openapi_contract_json().expect("embedded OpenAPI contract should parse");
        assert_eq!(spec["openapi"], "3.1.0");
        assert!(spec["paths"]["/health"].is_object());
        assert!(spec["paths"]["/healthz"].is_object());
        assert!(spec["paths"]["/ready"].is_object());
        assert!(spec["paths"]["/readyz"].is_object());
        assert!(spec["paths"]["/.well-known/agent.json"].is_object());
        assert!(spec["paths"]["/a2a/v1/messages"].is_object());
        assert!(spec["paths"]["/a2a/v1/tasks"].is_object());
        assert!(spec["paths"]["/a2a/v1/tasks/{task_id}"].is_object());
        assert!(spec["paths"]["/a2a/v1/tasks/{task_id}/cancel"].is_object());
        assert!(spec["paths"]["/pair"].is_object());
        assert!(spec["paths"]["/webhook"].is_object());
        assert!(spec["paths"]["/companion/context/ingest"].is_object());
        assert!(spec["paths"]["/companion/multimodal/ingest"].is_object());
        assert!(spec["paths"]["/companion/surface/caption"].is_object());
        assert!(spec["paths"]["/companion/surface/widget"].is_object());
        assert!(spec["paths"]["/companion/surface/request-window/open"].is_object());
        assert!(spec["paths"]["/companion/surface/request-window/{window_id}"].is_object());
        assert!(spec["paths"]["/companion/surface/request-window/{window_id}/confirm"].is_object());
        assert!(spec["paths"]["/companion/surface/request-window/{window_id}/cancel"].is_object());
        assert!(spec["paths"]["/ws"].is_object());
        assert!(spec["paths"]["/whatsapp"].is_object());
        assert!(spec["components"]["schemas"]["ProblemDetails"].is_object());
        assert!(spec["components"]["schemas"]["A2aAgentCard"].is_object());
        assert!(spec["components"]["schemas"]["A2aMessageRequest"].is_object());
        assert!(spec["components"]["schemas"]["A2aMessageResponse"].is_object());
        assert!(spec["components"]["schemas"]["CompanionContextIngestRequest"].is_object());
        assert!(spec["components"]["schemas"]["CompanionMultimodalIngestRequest"].is_object());
        assert!(spec["components"]["schemas"]["CompanionSurfaceCaptionRequest"].is_object());
        assert!(spec["components"]["schemas"]["CompanionSurfaceWidgetCommand"].is_object());
        assert!(spec["components"]["schemas"]["CompanionSurfaceRequestWindowOpen"].is_object());
    }

    #[test]
    fn openapi_contract_has_stable_path_set() {
        let spec = openapi_contract_json().expect("embedded OpenAPI contract should parse");
        let paths = spec["paths"]
            .as_object()
            .expect("paths should be an object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();

        let expected = BTreeSet::from([
            "/health".to_string(),
            "/healthz".to_string(),
            "/ready".to_string(),
            "/readyz".to_string(),
            "/.well-known/agent.json".to_string(),
            "/a2a/v1/messages".to_string(),
            "/a2a/v1/tasks".to_string(),
            "/a2a/v1/tasks/{task_id}".to_string(),
            "/a2a/v1/tasks/{task_id}/cancel".to_string(),
            "/openapi/v1.json".to_string(),
            "/pair".to_string(),
            "/companion/context/ingest".to_string(),
            "/companion/multimodal/ingest".to_string(),
            "/companion/surface/caption".to_string(),
            "/companion/surface/widget".to_string(),
            "/companion/surface/request-window/open".to_string(),
            "/companion/surface/request-window/{window_id}".to_string(),
            "/companion/surface/request-window/{window_id}/confirm".to_string(),
            "/companion/surface/request-window/{window_id}/cancel".to_string(),
            "/webhook".to_string(),
            "/whatsapp".to_string(),
            "/ws".to_string(),
        ]);

        assert_eq!(paths, expected);
    }

    #[test]
    fn openapi_contract_documents_trust_headers_for_webhook_and_a2a_ingress() {
        fn parameter_refs(spec: &serde_json::Value, path: &str) -> BTreeSet<String> {
            spec["paths"][path]["post"]["parameters"]
                .as_array()
                .expect("post parameters should be array")
                .iter()
                .filter_map(|entry| entry.get("$ref").and_then(serde_json::Value::as_str))
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
        }

        let spec = openapi_contract_json().expect("embedded OpenAPI contract should parse");
        let expected_shared_refs = BTreeSet::from([
            "#/components/parameters/SignatureVerifiedHeader".to_string(),
            "#/components/parameters/SignatureStatusHeader".to_string(),
            "#/components/parameters/WebhookSignatureStatusHeader".to_string(),
            "#/components/parameters/SourceUrlHeader".to_string(),
            "#/components/parameters/ExternalSourceUrlHeader".to_string(),
            "#/components/parameters/ForwardedProtoHeader".to_string(),
            "#/components/parameters/OriginHeader".to_string(),
            "#/components/parameters/RefererHeader".to_string(),
        ]);
        let expected_webhook_refs = expected_shared_refs
            .iter()
            .cloned()
            .chain(std::iter::once(
                "#/components/parameters/WebhookSourceHeader".to_string(),
            ))
            .collect::<BTreeSet<_>>();

        assert_eq!(
            parameter_refs(&spec, "/webhook"),
            expected_webhook_refs,
            "webhook trust headers must stay documented"
        );
        assert_eq!(
            parameter_refs(&spec, "/a2a/v1/messages"),
            expected_shared_refs,
            "a2a trust headers must stay documented"
        );
    }
}
