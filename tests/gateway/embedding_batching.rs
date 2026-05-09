use asterel::core::memory::embeddings::{EmbeddingProvider, OpenAiEmbedding};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn openai_embedder_batches_into_single_http_request() {
    let server = MockServer::start().await;

    let model = "text-embedding-3-small";
    let inputs = ["hello", "world"];
    let expected_body = json!({
        "model": model,
        "input": inputs,
    });

    let response_body = json!({
        "object": "list",
        "model": model,
        "data": [
            {"object": "embedding", "index": 0, "embedding": [0.1, 0.2, 0.3]},
            {"object": "embedding", "index": 1, "embedding": [0.4, 0.5, 0.6]}
        ],
        "usage": {"prompt_tokens": 2, "total_tokens": 2}
    });

    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .expect(1)
        .mount(&server)
        .await;

    let embedder = OpenAiEmbedding::new(&server.uri(), "test-key", model, 3);
    let vectors = embedder.embed(&inputs).await.unwrap();

    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0], vec![0.1_f32, 0.2_f32, 0.3_f32]);
    assert_eq!(vectors[1], vec![0.4_f32, 0.5_f32, 0.6_f32]);

    let received = server
        .received_requests()
        .await
        .expect("mock server should record received requests");
    assert_eq!(received.len(), 1);
    server.verify().await;
}
