use asterel::core::memory::{
    MemoryEventInput, MemoryEventType, MemoryInferenceEvent, MemoryLayer, MemorySource,
    PrivacyLevel,
};

#[test]
fn memory_layer_serde_roundtrip() {
    let input = MemoryEventInput::new(
        "entity-1",
        "profile.locale",
        MemoryEventType::FactAdded,
        "en-US",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Identity)
    .with_confidence(0.91)
    .with_importance(0.64)
    .with_occurred_at("2026-02-18T00:00:00Z");

    let input_json = serde_json::to_value(&input).expect("memory event input should serialize");
    assert_eq!(input_json["layer"], "identity");

    let input_roundtrip: MemoryEventInput =
        serde_json::from_value(input_json).expect("memory event input should deserialize");
    assert_eq!(input_roundtrip.layer, MemoryLayer::Identity);

    let inference = MemoryInferenceEvent::inferred_claim(
        "entity-1",
        "skills.rust",
        "prefers cargo over manual linking",
    )
    .with_layer(MemoryLayer::Procedural)
    .with_occurred_at("2026-02-18T00:00:00Z");

    let inference_json =
        serde_json::to_value(&inference).expect("inference event should serialize");
    assert_eq!(inference_json["layer"], "procedural");

    let inference_roundtrip: MemoryInferenceEvent =
        serde_json::from_value(inference_json).expect("inference event should deserialize");

    match &inference_roundtrip {
        MemoryInferenceEvent::InferredClaim { layer, .. } => {
            assert_eq!(*layer, MemoryLayer::Procedural);
        }
        _ => panic!("expected inferred claim variant"),
    }

    let projected = inference_roundtrip.into_memory_event_input();
    assert_eq!(projected.layer, MemoryLayer::Procedural);
}

#[test]
fn memory_layer_rejects_payload_without_layer() {
    let missing_layer_input = serde_json::json!({
        "entity_id": "entity-sample",
        "slot_key": "profile.timezone",
        "event_type": "fact_added",
        "value": "UTC",
        "source": "explicit_user",
        "confidence": 0.8,
        "importance": 0.5,
        "privacy_level": "private",
        "occurred_at": "2026-02-18T00:00:00Z"
    });

    let input_error = serde_json::from_value::<MemoryEventInput>(missing_layer_input)
        .expect_err("event input without layer must fail");
    assert!(input_error.to_string().contains("layer"));

    let missing_layer_inferred = serde_json::json!({
        "kind": "inferred_claim",
        "entity_id": "entity-sample",
        "slot_key": "preferences.editor",
        "value": "neovim",
        "confidence": 0.7,
        "importance": 0.5,
        "privacy_level": "private",
        "occurred_at": "2026-02-18T00:00:00Z"
    });

    let inferred_error = serde_json::from_value::<MemoryInferenceEvent>(missing_layer_inferred)
        .expect_err("inferred claim without layer must fail");
    assert!(inferred_error.to_string().contains("layer"));

    let missing_layer_contradiction = serde_json::json!({
        "kind": "contradiction_event",
        "entity_id": "entity-sample",
        "slot_key": "profile.timezone",
        "value": "conflict",
        "confidence": 0.85,
        "importance": 0.8,
        "privacy_level": "private",
        "occurred_at": "2026-02-18T00:00:00Z"
    });

    let contradiction_error =
        serde_json::from_value::<MemoryInferenceEvent>(missing_layer_contradiction)
            .expect_err("contradiction event without layer must fail");
    assert!(contradiction_error.to_string().contains("layer"));
}
