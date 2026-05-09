use super::http_providers::{
    build_embedding_http_client, parse_named_array_embeddings, parse_openai_like_embeddings,
    send_json_request,
};
use super::*;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn noop_name() {
    let provider = NoopEmbedding;
    assert_eq!(provider.name(), "none");
    assert_eq!(provider.dimensions(), 0);
}

#[tokio::test]
async fn noop_embed_returns_empty() {
    let provider = NoopEmbedding;
    assert!(provider.embed(&["hello"]).await.unwrap().is_empty());
}

#[test]
fn factory_none() {
    let provider =
        create_embedding_provider(&crate::config::EmbeddingProvider::None, None, "model", 0)
            .unwrap();
    assert_eq!(provider.name(), "none");
}

#[test]
fn factory_openai_uses_model_default_dimensions() {
    let provider = create_embedding_provider(
        &crate::config::EmbeddingProvider::OpenAi,
        Some("key"),
        "text-embedding-3-large",
        0,
    )
    .unwrap();
    assert_eq!(provider.name(), "openai");
    assert_eq!(provider.dimensions(), 3072);
}

#[test]
fn factory_openai_requires_api_key() {
    let error = create_embedding_provider(
        &crate::config::EmbeddingProvider::OpenAi,
        None,
        "text-embedding-3-small",
        0,
    )
    .err()
    .expect("openai embeddings should require an API key");
    assert!(error.to_string().contains("OpenAI embedding API key"));
}

#[test]
fn factory_custom_url_requires_valid_https_url() {
    let error = create_embedding_provider(
        &crate::config::EmbeddingProvider::OpenAiCompatible("https://localhost".into()),
        Some("key"),
        "model",
        0,
    )
    .err()
    .expect("blocked custom URL should return an error");
    assert!(error.to_string().contains("blocked"));
}

#[test]
fn custom_url_blocks_private_ipv4_ranges() {
    for url in [
        "https://10.0.0.1",
        "https://172.16.0.1",
        "https://192.168.1.1",
        "https://169.254.0.1",
        "https://127.0.0.1",
    ] {
        let outcome = validate_custom_base_url(url, CustomBaseUrlPolicy { allow_http: true });
        assert!(outcome.is_err(), "expected blocked URL: {url}");
    }
}

#[test]
fn custom_url_blocks_ipv6_loopback_and_link_local() {
    for url in ["https://[::1]", "https://[fe80::1]"] {
        let outcome = validate_custom_base_url(url, CustomBaseUrlPolicy { allow_http: true });
        assert!(outcome.is_err(), "expected blocked URL: {url}");
    }
}

#[test]
fn custom_url_blocks_metadata_host() {
    let outcome = validate_custom_base_url(
        "https://metadata.google.internal",
        CustomBaseUrlPolicy { allow_http: true },
    );
    assert!(outcome.is_err());
}

#[test]
fn openai_trailing_slash_stripped() {
    let provider = OpenAiEmbedding::new("https://api.openai.com/", "key", "model", 1536);
    assert_eq!(
        provider.embeddings_url,
        "https://api.openai.com/v1/embeddings"
    );
}

#[tokio::test]
async fn deterministic_embedder_is_stable_and_dimensional() {
    let provider = DeterministicEmbedding::with_seed(8, 42);

    let a1 = provider.embed_one("hello").await.unwrap();
    let a2 = provider.embed_one("hello").await.unwrap();
    let b = provider.embed_one("world").await.unwrap();

    assert_eq!(a1.len(), 8);
    assert_eq!(a2.len(), 8);
    assert_eq!(b.len(), 8);
    assert_eq!(a1, a2);
    assert_ne!(a1, b);

    let batch = provider.embed(&["a", "b"]).await.unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].len(), 8);
    assert_eq!(batch[1].len(), 8);
}

#[tokio::test]
async fn openai_provider_sends_dimensions_and_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header("Authorization", "Bearer key"))
        .and(body_json(json!({
            "model": "text-embedding-3-small",
            "input": ["hello"],
            "dimensions": 1536,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "embedding": [0.1, 0.2, 0.3] }]
        })))
        .mount(&server)
        .await;

    let provider = OpenAiEmbedding::new_with_options(
        "openai",
        &server.uri(),
        "key",
        "text-embedding-3-small",
        1536,
        Some("dimensions"),
    );

    let embeddings = provider.embed(&["hello"]).await.unwrap();
    assert_eq!(embeddings, vec![vec![0.1, 0.2, 0.3]]);
}

#[tokio::test]
async fn gemini_provider_uses_query_task_type() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-embedding-001:batchEmbedContents",
        ))
        .and(header("x-goog-api-key", "gem-key"))
        .and(wiremock::matchers::body_partial_json(json!({
            "requests": [{
                "model": "models/gemini-embedding-001"
            }]
        })))
        .and(wiremock::matchers::body_string_contains("RETRIEVAL_QUERY"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embeddings": [{ "values": [0.4, 0.5] }]
        })))
        .mount(&server)
        .await;

    let provider = GeminiEmbedding {
        client: build_embedding_http_client(),
        base_url: server.uri(),
        model: "models/gemini-embedding-001".to_string(),
        api_key: "gem-key".to_string(),
        dims: 3072,
    };
    let embeddings = provider.embed_queries(&["hello"]).await.unwrap();
    assert_eq!(embeddings, vec![vec![0.4, 0.5]]);
}

#[tokio::test]
async fn cohere_provider_uses_input_type_and_parses_float_embeddings() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/embed"))
        .and(header("Authorization", "Bearer co-key"))
        .and(wiremock::matchers::body_partial_json(json!({
            "input_type": "search_query",
            "output_dimension": 1536,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embeddings": { "float": [[0.3, 0.7]] }
        })))
        .mount(&server)
        .await;

    let provider = CohereEmbedding {
        client: build_embedding_http_client(),
        auth_header: "Bearer co-key".to_string(),
        model: "embed-v4.0".to_string(),
        dims: 1536,
    };
    let embeddings = send_json_request(
        "cohere",
        provider
            .client
            .post(format!("{}/v2/embed", server.uri()))
            .header("Authorization", &provider.auth_header)
            .json(&json!({
                "model": provider.model,
                "texts": ["hello"],
                "input_type": "search_query",
                "embedding_types": ["float"],
                "output_dimension": provider.dims,
            })),
    )
    .await
    .unwrap();
    let parsed = parse_named_array_embeddings(
        "cohere",
        embeddings.get("embeddings").unwrap().get("float").unwrap(),
        1,
    )
    .unwrap();
    assert_eq!(parsed, vec![vec![0.3, 0.7]]);
}

#[tokio::test]
async fn voyage_provider_uses_query_input_type_and_parses_data() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header("Authorization", "Bearer voy-key"))
        .and(wiremock::matchers::body_partial_json(json!({
            "input_type": "query",
            "output_dimension": 1024,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "embedding": [0.9, 0.1] }]
        })))
        .mount(&server)
        .await;

    let provider = VoyageEmbedding {
        client: build_embedding_http_client(),
        auth_header: "Bearer voy-key".to_string(),
        model: "voyage-4".to_string(),
        dims: 1024,
    };
    let json = send_json_request(
        "voyage",
        provider
            .client
            .post(format!("{}/v1/embeddings", server.uri()))
            .header("Authorization", &provider.auth_header)
            .json(&json!({
                "model": provider.model,
                "input": ["hello"],
                "input_type": "query",
                "output_dimension": provider.dims,
                "truncation": true,
            })),
    )
    .await
    .unwrap();
    let parsed = parse_openai_like_embeddings("voyage", &json, 1).unwrap();
    assert_eq!(parsed, vec![vec![0.9, 0.1]]);
}

#[tokio::test]
async fn nomic_provider_uses_task_type_and_parses_embeddings() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embedding/text"))
        .and(header("Authorization", "Bearer nom-key"))
        .and(wiremock::matchers::body_partial_json(json!({
            "task_type": "search_query",
            "dimensionality": 768,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embeddings": [[0.2, 0.8]]
        })))
        .mount(&server)
        .await;

    let provider = NomicEmbedding {
        client: build_embedding_http_client(),
        auth_header: "Bearer nom-key".to_string(),
        model: "nomic-embed-text-v1.5".to_string(),
        dims: 768,
    };
    let json = send_json_request(
        "nomic",
        provider
            .client
            .post(format!("{}/v1/embedding/text", server.uri()))
            .header("Authorization", &provider.auth_header)
            .json(&json!({
                "model": provider.model,
                "texts": ["hello"],
                "task_type": "search_query",
                "dimensionality": provider.dims,
            })),
    )
    .await
    .unwrap();
    let parsed = parse_named_array_embeddings("nomic", json.get("embeddings").unwrap(), 1).unwrap();
    assert_eq!(parsed, vec![vec![0.2, 0.8]]);
}

#[tokio::test]
async fn bedrock_provider_signs_request_and_parses_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/amazon.titan-embed-text-v2%3A0/invoke"))
        .and(wiremock::matchers::header_exists("x-amz-content-sha256"))
        .and(wiremock::matchers::header_exists("x-amz-date"))
        .and(wiremock::matchers::header_exists("Authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embedding": [0.6, 0.4]
        })))
        .mount(&server)
        .await;

    let provider = BedrockEmbedding::new_for_tests(
        &server.uri(),
        "us-east-1",
        "AKIDEXAMPLE",
        "secret",
        "amazon.titan-embed-text-v2:0",
        1024,
    )
    .unwrap();

    let embeddings = provider.embed(&["hello"]).await.unwrap();
    assert_eq!(embeddings, vec![vec![0.6, 0.4]]);
}
