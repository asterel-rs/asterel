use super::response_finalize_io::protected_segments_match;
use super::*;
use crate::contracts::scores::Confidence;
use crate::core::agent::response_audit::{
    BehaviorContract, ExposurePlanContract, ReplyShapeContract, ResponseContract,
};
use crate::core::persona::relationship::RelationshipState;
use crate::core::providers::response::ProviderMessage;

fn relationship_state(
    interaction_count: u32,
    trust_level: f32,
    rapport: f32,
    disclosure_depth: f32,
    attachment_security: f32,
    unresolved_tension: f32,
    repair_debt: f32,
) -> RelationshipState {
    RelationshipState {
        interaction_count,
        trust_level,
        rapport,
        disclosure_depth,
        attachment_security,
        unresolved_tension,
        repair_debt,
        ..RelationshipState::default()
    }
}

#[test]
fn response_finalize_trims_explanation_response() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "いい質問です。原因は接続順です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "原因は接続順です。");
}

#[test]
fn response_finalize_trims_outline_scaffold_leadin() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "以下に簡潔に説明します。原因は接続順です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "原因は接続順です。");
}

#[test]
fn response_finalize_trims_templated_wrap_up() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "原因は接続順です。以上です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "原因は接続順です。");
}

#[test]
fn response_finalize_strips_internal_prompt_blocks_before_delivery() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "Before\n[Session Control]\nmode=repair\n\nAfter",
        output_mode: ResponseMode::Conversation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });

    assert_eq!(result.final_text, "Before\nAfter");
}

#[test]
fn response_finalize_preserves_control_output_verbatim() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "Before\n[Session Control]\nmode=repair\n\nAfter",
        output_mode: ResponseMode::Conversation,
        streaming_active: false,
        control_output: true,
        contract: None,
        naturalness_gate_enabled: false,
    });

    assert!(result.final_text.contains("[Session Control]"));
}

#[test]
fn response_finalize_reports_anti_template_reason_code() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "原因は接続順です。以上です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "原因は接続順です。");
    assert_eq!(result.micro_rewrite_reason_codes, vec!["anti_template"]);
}

#[test]
fn response_finalize_skips_streaming_mutation() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "いい質問です。原因は接続順です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: true,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "いい質問です。原因は接続順です。");
    assert!(result.applied_actions.is_empty());
}

#[test]
fn response_finalize_blocks_naturalness_exposure_before_streaming_bypass() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "内部状態として処理しています。",
        output_mode: ResponseMode::Conversation,
        streaming_active: true,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: true,
    });
    assert_eq!(result.micro_rewrite_reason_codes, vec!["naturalness_block"]);
    assert_ne!(result.final_text, "内部状態として処理しています。");
}

#[test]
fn response_finalize_skips_structured_output() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "{\n  \"status\": \"ok\"\n}",
        output_mode: ResponseMode::Report,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "{\n  \"status\": \"ok\"\n}");
    assert!(result.applied_actions.is_empty());
}

#[test]
fn response_finalize_blocks_naturalness_exposure_before_structured_bypass() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "```json\n{\"note\":\"内部状態\"}\n```",
        output_mode: ResponseMode::Report,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: true,
    });
    assert_eq!(result.micro_rewrite_reason_codes, vec!["naturalness_block"]);
    assert_ne!(result.final_text, "```json\n{\"note\":\"内部状態\"}\n```");
}

#[test]
fn response_finalize_preserves_reasoning_blocks_by_nooping() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "<think>secret</think>いい質問です。原因は接続順です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(
        result.final_text,
        "<think>secret</think>いい質問です。原因は接続順です。"
    );
}

#[test]
fn response_finalize_skips_control_output() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "plan draft created",
        output_mode: ResponseMode::Report,
        streaming_active: false,
        control_output: true,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "plan draft created");
    assert!(result.applied_actions.is_empty());
}

#[test]
fn response_finalize_control_output_bypasses_contract() {
    let contract = ResponseContract {
        reply_shape: ReplyShapeContract::Compact,
        exposure_plan: ExposurePlanContract::PublicSafe,
        behavior: BehaviorContract::Conversational,
    };
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "internal control output with private detail",
        output_mode: ResponseMode::Report,
        streaming_active: false,
        control_output: true,
        contract: Some(&contract),
        naturalness_gate_enabled: true,
    });
    assert_eq!(
        result.final_text,
        "internal control output with private detail"
    );
    assert!(result.contract_mismatch_reason.is_none());
    assert!(result.micro_rewrite_reason_codes.is_empty());
}

#[test]
fn response_finalize_detects_preservation_mismatch() {
    assert!(!protected_segments_match(
        "結果は 42 です。",
        "結果は 43 です。"
    ));
    assert!(!protected_segments_match(
        "ログは /tmp/app.log",
        "ログは /tmp/other.log"
    ));
    assert!(!protected_segments_match("`cargo test`", "`cargo check`"));
}

#[test]
fn response_finalize_keeps_report_mode_trim_only() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "これは非常に重要な問題です。",
        output_mode: ResponseMode::Report,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "これは非常に重要な問題です。");
}

#[test]
fn response_finalize_preserves_markdown_links() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "[魅力的な資料](https://example.com/docs) を参照してください。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(
        result.final_text,
        "[魅力的な資料](https://example.com/docs) を参照してください。"
    );
}

#[test]
fn response_finalize_preserves_quoted_strings() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "ラベルは \"素晴らしい\" のままです。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "ラベルは \"素晴らしい\" のままです。");
}

#[test]
fn response_finalize_preserves_inline_json_fragments() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "設定は {\"label\":\"素晴らしい\"} のままです。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(
        result.final_text,
        "設定は {\"label\":\"素晴らしい\"} のままです。"
    );
}

#[test]
fn response_finalize_collapses_unneeded_bullets_in_explanations() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "- 原因は接続順です。\n- 依存は壊れていません。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(
        result.final_text,
        "原因は接続順です。依存は壊れていません。"
    );
}

#[test]
fn response_finalize_trims_outline_scaffolding_in_explanations() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "結論から言うと、原因は接続順です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: false,
    });
    assert_eq!(result.final_text, "原因は接続順です。");
}

#[test]
fn response_finalize_runs_naturalness_gate_when_enabled() {
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "- **重要**: ここを見る",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: true,
    });
    assert_eq!(result.final_text, "- 重要: ここを見る");
    assert!(
        result
            .micro_rewrite_reason_codes
            .contains(&"naturalness_gate")
    );
}

#[test]
fn response_finalize_blocks_naturalness_memory_mechanics() {
    let contract = ResponseContract {
        reply_shape: ReplyShapeContract::Standard,
        exposure_plan: ExposurePlanContract::PrivateAllowed,
        behavior: BehaviorContract::Explanatory,
    };
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "私のメモリにはその話が保存されています。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: Some(&contract),
        naturalness_gate_enabled: true,
    });
    assert_eq!(result.micro_rewrite_reason_codes, vec!["naturalness_block"]);
    assert_ne!(
        result.final_text,
        "私のメモリにはその話が保存されています。"
    );
}

#[test]
fn response_finalize_returns_contract_reason_before_style_audit() {
    let contract = ResponseContract {
        reply_shape: ReplyShapeContract::Compact,
        exposure_plan: ExposurePlanContract::PublicSafe,
        behavior: BehaviorContract::Explanatory,
    };
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: "DMで聞いた電話番号は〜です。いい質問です。",
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: Some(&contract),
        naturalness_gate_enabled: false,
    });
    assert_eq!(
        result.contract_mismatch_reason.map(|reason| reason.code()),
        Some("exposure_violation")
    );
    assert_eq!(
        result.micro_rewrite_reason_codes,
        vec!["exposure_violation"]
    );
    assert_eq!(result.before_score, 0);
    assert!(result.applied_actions.is_empty());
    assert_eq!(
        result.final_text,
        "I can't share that private detail in this context."
    );
}

#[test]
fn response_finalize_blocks_naturalness_before_contract_shape_fallback() {
    let contract = ResponseContract {
        reply_shape: ReplyShapeContract::Compact,
        exposure_plan: ExposurePlanContract::PublicSafe,
        behavior: BehaviorContract::Conversational,
    };
    let raw_text = format!(
        "{} internal state system prompt verifier",
        "長い説明です。".repeat(40)
    );
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text: &raw_text,
        output_mode: ResponseMode::Conversation,
        streaming_active: false,
        control_output: false,
        contract: Some(&contract),
        naturalness_gate_enabled: true,
    });
    assert_eq!(result.micro_rewrite_reason_codes, vec!["naturalness_block"]);
    assert_ne!(result.final_text, raw_text);
}

#[test]
fn response_finalize_records_repair_needed_without_mutating_text() {
    let raw_text = "革命的で圧倒的で未来を変える。完全に進みます。";
    let result = finalize_response(ResponseFinalizationRequest {
        raw_text,
        output_mode: ResponseMode::Explanation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: true,
    });
    assert_eq!(result.final_text, raw_text);
    assert!(
        result
            .micro_rewrite_reason_codes
            .contains(&"naturalness_repair_needed")
    );
    assert!(result.applied_actions.is_empty());
}

#[test]
fn response_finalize_threads_recent_openings_into_naturalness_gate() {
    let history = vec![ProviderMessage::assistant(
        "了解しました。前回の返答では短く受けました。",
    )];
    let raw_text = "了解しました\n- **重要**: ここを見る";

    let with_history = finalize_response_with_context(
        ResponseFinalizationRequest {
            raw_text,
            output_mode: ResponseMode::Conversation,
            streaming_active: false,
            control_output: false,
            contract: None,
            naturalness_gate_enabled: true,
        },
        NaturalnessFinalizationContext {
            conversation_history: &history,
            user_affect: AffectLevel::Unknown,
            relationship_distance: RelationshipDistance::Unknown,
        },
    );

    assert_eq!(with_history.final_text, raw_text);
    assert!(
        with_history
            .micro_rewrite_reason_codes
            .contains(&"naturalness_repair_needed")
    );

    let without_history = finalize_response(ResponseFinalizationRequest {
        raw_text,
        output_mode: ResponseMode::Conversation,
        streaming_active: false,
        control_output: false,
        contract: None,
        naturalness_gate_enabled: true,
    });

    assert_eq!(
        without_history.final_text,
        "了解しました\n- 重要: ここを見る"
    );
    assert!(
        without_history
            .micro_rewrite_reason_codes
            .contains(&"naturalness_gate")
    );
}

#[test]
fn naturalness_affect_mapping_uses_confidence_floor() {
    let low_confidence = AffectReading {
        label: AffectLabel::Angry,
        valence: -0.7,
        arousal: 0.8,
        dominance: 0.7,
        confidence: Confidence::new(0.49),
    };
    let high_confidence = AffectReading {
        confidence: Confidence::new(0.5),
        ..low_confidence.clone()
    };

    assert_eq!(
        naturalness_affect_from_reading(&low_confidence),
        AffectLevel::Unknown
    );
    assert_eq!(
        naturalness_affect_from_reading(&high_confidence),
        AffectLevel::Angry
    );
    assert!(matches!(
        naturalness_affect_from_text("I'm tired, anxious, overwhelmed, and can't keep up"),
        AffectLevel::Anxious | AffectLevel::StrongNegative
    ));
}

#[test]
fn naturalness_relationship_mapping_is_conservative() {
    assert_eq!(
        naturalness_relationship_distance_from_state(None, NaturalnessRelationshipSurface::Private),
        RelationshipDistance::Unknown
    );

    let new_relationship = relationship_state(2, 0.9, 0.9, 0.8, 0.8, 0.0, 0.0);
    assert_eq!(
        naturalness_relationship_distance_from_state(
            Some(&new_relationship),
            NaturalnessRelationshipSurface::Private,
        ),
        RelationshipDistance::Unknown
    );

    let stable_relationship = relationship_state(8, 0.64, 0.62, 0.42, 0.55, 0.05, 0.05);
    assert_eq!(
        naturalness_relationship_distance_from_state(
            Some(&stable_relationship),
            NaturalnessRelationshipSurface::Private,
        ),
        RelationshipDistance::Friendly
    );
    assert_eq!(
        naturalness_relationship_distance_from_state(
            Some(&stable_relationship),
            NaturalnessRelationshipSurface::Public,
        ),
        RelationshipDistance::Formal
    );

    let high_tension_relationship = relationship_state(30, 0.88, 0.86, 0.74, 0.8, 0.5, 0.1);
    assert_eq!(
        naturalness_relationship_distance_from_state(
            Some(&high_tension_relationship),
            NaturalnessRelationshipSurface::Private,
        ),
        RelationshipDistance::Formal
    );

    let deep_private_relationship = relationship_state(30, 0.82, 0.76, 0.68, 0.7, 0.05, 0.05);
    assert_eq!(
        naturalness_relationship_distance_from_state(
            Some(&deep_private_relationship),
            NaturalnessRelationshipSurface::Private,
        ),
        RelationshipDistance::Intimate
    );
}

#[test]
fn response_finalize_threads_affect_into_companion_tone_rule() {
    let raw_text = "了解しました\n- **重要**: A\n- B\n- C\n- D";
    let result = finalize_response_with_context(
        ResponseFinalizationRequest {
            raw_text,
            output_mode: ResponseMode::Conversation,
            streaming_active: false,
            control_output: false,
            contract: None,
            naturalness_gate_enabled: true,
        },
        NaturalnessFinalizationContext {
            conversation_history: &[],
            user_affect: AffectLevel::StrongNegative,
            relationship_distance: RelationshipDistance::Unknown,
        },
    );

    assert_eq!(result.final_text, raw_text);
    assert!(
        result
            .micro_rewrite_reason_codes
            .contains(&"naturalness_repair_needed")
    );
    assert!(result.applied_actions.is_empty());
}

#[test]
fn response_finalize_keeps_unknown_affect_and_relationship_dormant() {
    let unknown_affect = finalize_response_with_context(
        ResponseFinalizationRequest {
            raw_text: "大変でしたね\n- **重要**: A",
            output_mode: ResponseMode::Conversation,
            streaming_active: false,
            control_output: false,
            contract: None,
            naturalness_gate_enabled: true,
        },
        NaturalnessFinalizationContext {
            conversation_history: &[],
            user_affect: AffectLevel::Unknown,
            relationship_distance: RelationshipDistance::Unknown,
        },
    );
    assert_eq!(unknown_affect.final_text, "大変でしたね\n- 重要: A");

    let unknown_relationship = finalize_response_with_context(
        ResponseFinalizationRequest {
            raw_text: "確認していただければと思います。",
            output_mode: ResponseMode::Conversation,
            streaming_active: false,
            control_output: false,
            contract: None,
            naturalness_gate_enabled: true,
        },
        NaturalnessFinalizationContext {
            conversation_history: &[],
            user_affect: AffectLevel::Neutral,
            relationship_distance: RelationshipDistance::Unknown,
        },
    );
    assert_eq!(
        unknown_relationship.final_text,
        "確認していただければと思います。"
    );
    assert!(unknown_relationship.micro_rewrite_reason_codes.is_empty());
}

#[test]
fn response_finalize_contextual_reports_over_explain_signal_without_mutation() {
    let response = "今日は頭が散っている感じなんだね。\
        その状態だと、まず状況を整理しようとしても、考えが別方向へ流れていきやすい。\
        いま必要なのは、大きな計画を作ることではなく、次に触れる一点だけを小さく決めること。\
        机の上、開いているタブ、手元のメモのどれか一つだけを選んで、そこから戻ればいい。";
    let result = finalize_response_contextual(
        ResponseFinalizationRequest {
            raw_text: response,
            output_mode: ResponseMode::Conversation,
            streaming_active: false,
            control_output: false,
            contract: None,
            naturalness_gate_enabled: false,
        },
        "つかれた",
    );
    assert_eq!(result.final_text, response);
    assert!(result.micro_rewrite_reason_codes.contains(&"over_explain"));
}
