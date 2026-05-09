#![cfg(feature = "taste")]

use std::collections::BTreeMap;

use asterel::config::TasteConfig;
use asterel::core::taste::{
    Artifact, Axis, AxisScores, Domain, PairComparison, Priority, Suggestion, TasteContext,
    TasteOwnerScope, TasteReport, TextFormat, TextOp, UiOp, Winner,
};

fn all_axes_scores(coherence: f64, hierarchy: f64, intentionality: f64) -> AxisScores {
    let mut scores = BTreeMap::new();
    scores.insert(Axis::Coherence, coherence);
    scores.insert(Axis::Hierarchy, hierarchy);
    scores.insert(Axis::Intentionality, intentionality);
    scores
}

mod type_roundtrips {
    use super::*;

    #[test]
    fn artifact_text_roundtrip() {
        let artifact = Artifact::Text {
            content: "hello world".into(),
            format: Some(TextFormat::Markdown),
        };
        let json = serde_json::to_string(&artifact).unwrap();
        let recovered: Artifact = serde_json::from_str(&json).unwrap();
        if let Artifact::Text { content, format } = recovered {
            assert_eq!(content, "hello world");
            assert_eq!(format, Some(TextFormat::Markdown));
        } else {
            panic!("expected Artifact::Text");
        }
    }

    #[test]
    fn artifact_ui_roundtrip() {
        let artifact = Artifact::Ui {
            description: "a dashboard".into(),
            metadata: Some(serde_json::json!({"columns": 3})),
        };
        let json = serde_json::to_string(&artifact).unwrap();
        let recovered: Artifact = serde_json::from_str(&json).unwrap();
        if let Artifact::Ui {
            description,
            metadata,
        } = recovered
        {
            assert_eq!(description, "a dashboard");
            assert!(metadata.is_some());
        } else {
            panic!("expected Artifact::Ui");
        }
    }

    #[test]
    fn artifact_text_tagged_kind() {
        let artifact = Artifact::Text {
            content: "x".into(),
            format: None,
        };
        let json = serde_json::to_string(&artifact).unwrap();
        assert!(json.contains("\"kind\":\"text\""));
    }

    #[test]
    fn artifact_ui_tagged_kind() {
        let artifact = Artifact::Ui {
            description: "x".into(),
            metadata: None,
        };
        let json = serde_json::to_string(&artifact).unwrap();
        assert!(json.contains("\"kind\":\"ui\""));
    }

    #[test]
    fn taste_report_roundtrip() {
        let report = TasteReport {
            axis: all_axes_scores(0.8, 0.6, 0.9),
            domain: Domain::Text,
            suggestions: vec![Suggestion::General {
                title: "improve clarity".into(),
                rationale: "too dense".into(),
                priority: Priority::Medium,
            }],
            raw_critique: Some("good overall".into()),
        };
        let json = serde_json::to_string(&report).unwrap();
        let recovered: TasteReport = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered.axis.len(), 3);
        assert_eq!(recovered.domain, Domain::Text);
        assert_eq!(recovered.suggestions.len(), 1);
        assert_eq!(recovered.raw_critique.as_deref(), Some("good overall"));
    }

    #[test]
    fn taste_report_empty_suggestions() {
        let report = TasteReport {
            axis: all_axes_scores(1.0, 1.0, 1.0),
            domain: Domain::Ui,
            suggestions: vec![],
            raw_critique: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        let recovered: TasteReport = serde_json::from_str(&json).unwrap();
        assert!(recovered.suggestions.is_empty());
        assert!(recovered.raw_critique.is_none());
    }

    #[test]
    fn pair_comparison_roundtrip() {
        let pc = PairComparison {
            owner: TasteOwnerScope::new(Some("tenant-test"), "person-test", Some("session-test")),
            domain: Domain::Text,
            ctx: TasteContext::default(),
            left_id: "artifact_a".into(),
            right_id: "artifact_b".into(),
            winner: Winner::Left,
            rationale: Some("clearer argument structure".into()),
            created_at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&pc).unwrap();
        let recovered: PairComparison = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered.left_id, "artifact_a");
        assert_eq!(recovered.right_id, "artifact_b");
        assert_eq!(recovered.winner, Winner::Left);
        assert_eq!(recovered.created_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn pair_comparison_all_winners() {
        for winner in [Winner::Left, Winner::Right, Winner::Tie, Winner::Abstain] {
            let pc = PairComparison {
                owner: TasteOwnerScope::new(
                    Some("tenant-test"),
                    "person-test",
                    Some("session-test"),
                ),
                domain: Domain::General,
                ctx: TasteContext::default(),
                left_id: "l".into(),
                right_id: "r".into(),
                winner: winner.clone(),
                rationale: None,
                created_at_ms: 0,
            };
            let json = serde_json::to_string(&pc).unwrap();
            let recovered: PairComparison = serde_json::from_str(&json).unwrap();
            assert_eq!(recovered.winner, winner);
        }
    }

    #[test]
    fn suggestion_text_roundtrip() {
        let s = Suggestion::Text {
            op: TextOp::UnifyStyle,
            rationale: "inconsistent tone".into(),
            priority: Priority::High,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"kind\":\"text\""));
        let recovered: Suggestion = serde_json::from_str(&json).unwrap();
        if let Suggestion::Text { op, priority, .. } = recovered {
            assert_eq!(op, TextOp::UnifyStyle);
            assert_eq!(priority, Priority::High);
        } else {
            panic!("expected Suggestion::Text");
        }
    }

    #[test]
    fn suggestion_ui_roundtrip() {
        let s = Suggestion::Ui {
            op: UiOp::ImproveHierarchy,
            rationale: "flat layout".into(),
            priority: Priority::Low,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"kind\":\"ui\""));
        let recovered: Suggestion = serde_json::from_str(&json).unwrap();
        if let Suggestion::Ui { op, priority, .. } = recovered {
            assert_eq!(op, UiOp::ImproveHierarchy);
            assert_eq!(priority, Priority::Low);
        } else {
            panic!("expected Suggestion::Ui");
        }
    }

    #[test]
    fn suggestion_general_roundtrip() {
        let s = Suggestion::General {
            title: "add introduction".into(),
            rationale: "missing context".into(),
            priority: Priority::Medium,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"kind\":\"general\""));
        let recovered: Suggestion = serde_json::from_str(&json).unwrap();
        if let Suggestion::General {
            title, priority, ..
        } = recovered
        {
            assert_eq!(title, "add introduction");
            assert_eq!(priority, Priority::Medium);
        } else {
            panic!("expected Suggestion::General");
        }
    }

    #[test]
    fn taste_context_roundtrip() {
        let ctx = TasteContext {
            domain: Domain::Text,
            genre: Some("technical".into()),
            purpose: Some("documentation".into()),
            audience: Some("developers".into()),
            constraints: vec!["max 500 words".into()],
            extra: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let recovered: TasteContext = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered.domain, Domain::Text);
        assert_eq!(recovered.genre.as_deref(), Some("technical"));
        assert_eq!(recovered.constraints.len(), 1);
    }

    #[test]
    fn taste_context_default_deserializes_from_empty_json() {
        let ctx: TasteContext = serde_json::from_str("{}").unwrap();
        assert_eq!(ctx.domain, Domain::General);
        assert!(ctx.genre.is_none());
        assert!(ctx.constraints.is_empty());
    }

    #[test]
    fn axis_scores_stable_ordering() {
        let scores = all_axes_scores(0.1, 0.2, 0.3);
        let keys: Vec<&Axis> = scores.keys().collect();

        assert_eq!(*keys[0], Axis::Coherence);
        assert_eq!(*keys[1], Axis::Hierarchy);
        assert_eq!(*keys[2], Axis::Intentionality);
    }
}

mod config_defaults {
    use super::*;

    #[test]
    fn default_config_enabled() {
        let cfg = TasteConfig::default();
        assert!(cfg.enabled, "taste should be enabled by default");
    }

    #[test]
    fn default_config_backend() {
        let cfg = TasteConfig::default();
        assert_eq!(cfg.backend, asterel::config::TasteBackend::Llm);
    }

    #[test]
    fn default_config_has_three_axes() {
        let cfg = TasteConfig::default();
        assert_eq!(cfg.axes.len(), 3);
        assert!(cfg.axes.contains(&"coherence".to_string()));
        assert!(cfg.axes.contains(&"hierarchy".to_string()));
        assert!(cfg.axes.contains(&"intentionality".to_string()));
    }

    #[test]
    fn default_config_domains_enabled() {
        let cfg = TasteConfig::default();
        assert!(cfg.text_enabled);
        assert!(cfg.ui_enabled);
    }

    #[test]
    fn config_json_roundtrip() {
        let cfg = TasteConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let recovered: TasteConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered.backend, cfg.backend);
        assert_eq!(recovered.axes, cfg.axes);
        assert_eq!(recovered.enabled, cfg.enabled);
    }
}

mod domain_tests {
    use super::*;

    #[test]
    fn domain_default_is_general() {
        assert_eq!(Domain::default(), Domain::General);
    }

    #[test]
    fn domain_display_snake_case() {
        assert_eq!(Domain::Text.to_string(), "text");
        assert_eq!(Domain::Ui.to_string(), "ui");
        assert_eq!(Domain::General.to_string(), "general");
    }

    #[test]
    fn axis_display_snake_case() {
        assert_eq!(Axis::Coherence.to_string(), "coherence");
        assert_eq!(Axis::Hierarchy.to_string(), "hierarchy");
        assert_eq!(Axis::Intentionality.to_string(), "intentionality");
    }

    #[test]
    fn axis_exactly_three_variants() {
        let axes = [Axis::Coherence, Axis::Hierarchy, Axis::Intentionality];
        assert_eq!(axes.len(), 3);
    }
}

mod pipeline_tests {
    use super::*;

    /// Full TasteReport with all 3 axes, all suggestion types (General/Text/Ui),
    /// and raw_critique populated — verifies thorough serialize/deserialize fidelity.
    #[test]
    fn taste_report_full_pipeline_roundtrip() {
        let report = TasteReport {
            axis: all_axes_scores(0.8, 0.7, 0.9),
            domain: Domain::Text,
            suggestions: vec![
                Suggestion::General {
                    title: "add introduction".into(),
                    rationale: "missing context for the reader".into(),
                    priority: Priority::High,
                },
                Suggestion::Text {
                    op: TextOp::RestructureArgument,
                    rationale: "argument flow is non-linear".into(),
                    priority: Priority::Medium,
                },
                Suggestion::Ui {
                    op: UiOp::AddContrast,
                    rationale: "low contrast between heading and body".into(),
                    priority: Priority::Low,
                },
            ],
            raw_critique: Some("Overall good but needs structural improvements.".into()),
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        let recovered: TasteReport = serde_json::from_str(&json).unwrap();

        // All 3 axes present with correct values
        assert_eq!(recovered.axis.len(), 3);
        assert!((recovered.axis[&Axis::Coherence] - 0.8).abs() < f64::EPSILON);
        assert!((recovered.axis[&Axis::Hierarchy] - 0.7).abs() < f64::EPSILON);
        assert!((recovered.axis[&Axis::Intentionality] - 0.9).abs() < f64::EPSILON);

        // All axis values in [0,1]
        for &score in recovered.axis.values() {
            assert!((0.0..=1.0).contains(&score), "score {score} not in [0,1]");
        }

        // Domain preserved
        assert_eq!(recovered.domain, Domain::Text);

        // 3 suggestion variants
        assert_eq!(recovered.suggestions.len(), 3);
        assert!(matches!(
            &recovered.suggestions[0],
            Suggestion::General {
                priority: Priority::High,
                ..
            }
        ));
        assert!(matches!(
            &recovered.suggestions[1],
            Suggestion::Text {
                op: TextOp::RestructureArgument,
                ..
            }
        ));
        assert!(matches!(
            &recovered.suggestions[2],
            Suggestion::Ui {
                op: UiOp::AddContrast,
                ..
            }
        ));

        // raw_critique preserved
        assert_eq!(
            recovered.raw_critique.as_deref(),
            Some("Overall good but needs structural improvements.")
        );
    }

    /// PairComparison with Winner::Tie and Winner::Abstain round-trips through JSON
    /// with fully populated TasteContext — verifies nested context fidelity.
    #[test]
    fn pair_comparison_tie_and_abstain_full_context() {
        let rich_ctx = TasteContext {
            domain: Domain::Ui,
            genre: Some("dashboard".into()),
            purpose: Some("executive summary".into()),
            audience: Some("C-suite executives".into()),
            constraints: vec!["max 3 columns".into(), "mobile-first".into()],
            extra: {
                let mut m = serde_json::Map::new();
                m.insert("brand".into(), serde_json::json!("acme"));
                m.insert("version".into(), serde_json::json!(2));
                m
            },
        };

        for winner in [Winner::Tie, Winner::Abstain] {
            let pc = PairComparison {
                owner: TasteOwnerScope::new(
                    Some("tenant-test"),
                    "person-test",
                    Some("session-test"),
                ),
                domain: Domain::Ui,
                ctx: rich_ctx.clone(),
                left_id: "dashboard_v1".into(),
                right_id: "dashboard_v2".into(),
                winner: winner.clone(),
                rationale: Some("equally compelling layouts".into()),
                created_at_ms: 1_700_000_000_000,
            };

            let json = serde_json::to_string(&pc).unwrap();
            let recovered: PairComparison = serde_json::from_str(&json).unwrap();

            assert_eq!(recovered.winner, winner);
            assert_eq!(recovered.left_id, "dashboard_v1");
            assert_eq!(recovered.right_id, "dashboard_v2");
            assert_eq!(recovered.domain, Domain::Ui);
            assert_eq!(recovered.created_at_ms, 1_700_000_000_000);
            assert_eq!(
                recovered.rationale.as_deref(),
                Some("equally compelling layouts")
            );

            // Nested context fields
            assert_eq!(recovered.ctx.domain, Domain::Ui);
            assert_eq!(recovered.ctx.genre.as_deref(), Some("dashboard"));
            assert_eq!(recovered.ctx.purpose.as_deref(), Some("executive summary"));
            assert_eq!(
                recovered.ctx.audience.as_deref(),
                Some("C-suite executives")
            );
            assert_eq!(recovered.ctx.constraints.len(), 2);
            assert_eq!(recovered.ctx.constraints[0], "max 3 columns");
            assert_eq!(recovered.ctx.constraints[1], "mobile-first");
            assert_eq!(recovered.ctx.extra.len(), 2);
            assert_eq!(recovered.ctx.extra["brand"], serde_json::json!("acme"));
            assert_eq!(recovered.ctx.extra["version"], serde_json::json!(2));
        }
    }

    /// TasteContext with every optional field populated and extra map entries
    /// round-trips through JSON — verifies no fields are silently dropped.
    #[test]
    fn taste_context_all_fields_populated_roundtrip() {
        let ctx = TasteContext {
            domain: Domain::Text,
            genre: Some("technical blog post".into()),
            purpose: Some("explain async Rust patterns".into()),
            audience: Some("intermediate Rust developers".into()),
            constraints: vec![
                "under 2000 words".into(),
                "include code examples".into(),
                "no unsafe".into(),
            ],
            extra: {
                let mut m = serde_json::Map::new();
                m.insert("tone".into(), serde_json::json!("approachable"));
                m.insert("code_ratio".into(), serde_json::json!(0.3));
                m.insert("tags".into(), serde_json::json!(["rust", "async", "tokio"]));
                m
            },
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let recovered: TasteContext = serde_json::from_str(&json).unwrap();

        assert_eq!(recovered.domain, Domain::Text);
        assert_eq!(recovered.genre.as_deref(), Some("technical blog post"));
        assert_eq!(
            recovered.purpose.as_deref(),
            Some("explain async Rust patterns")
        );
        assert_eq!(
            recovered.audience.as_deref(),
            Some("intermediate Rust developers")
        );
        assert_eq!(recovered.constraints.len(), 3);
        assert_eq!(recovered.constraints[2], "no unsafe");

        // Extra map preserves all types: string, number, array
        assert_eq!(recovered.extra.len(), 3);
        assert_eq!(recovered.extra["tone"], serde_json::json!("approachable"));
        assert!((recovered.extra["code_ratio"].as_f64().unwrap() - 0.3).abs() < f64::EPSILON);
        assert_eq!(
            recovered.extra["tags"],
            serde_json::json!(["rust", "async", "tokio"])
        );
    }

    /// Suggestion::General with each Priority variant (High, Medium, Low)
    /// round-trips correctly — verifies tagged-enum + priority serde.
    #[test]
    fn suggestion_general_all_priority_levels() {
        let priorities = [
            (Priority::High, "high"),
            (Priority::Medium, "medium"),
            (Priority::Low, "low"),
        ];

        for (priority, expected_str) in priorities {
            let suggestion = Suggestion::General {
                title: format!("suggestion at {expected_str} priority"),
                rationale: format!("rationale for {expected_str}"),
                priority: priority.clone(),
            };

            let json = serde_json::to_string(&suggestion).unwrap();

            // Verify JSON contains correct tagged kind and priority
            assert!(json.contains("\"kind\":\"general\""), "missing kind tag");
            assert!(
                json.contains(&format!("\"priority\":\"{expected_str}\"")),
                "priority {expected_str} not found in JSON: {json}"
            );

            let recovered: Suggestion = serde_json::from_str(&json).unwrap();
            if let Suggestion::General {
                title,
                rationale,
                priority: recovered_priority,
            } = recovered
            {
                assert_eq!(title, format!("suggestion at {expected_str} priority"));
                assert_eq!(rationale, format!("rationale for {expected_str}"));
                assert_eq!(recovered_priority, priority);
            } else {
                panic!("expected Suggestion::General, got different variant");
            }
        }
    }

    /// AxisScores BTreeMap ordering remains stable (Coherence < Hierarchy < Intentionality)
    /// after JSON round-trip — verifies BTreeMap key ordering survives serialization.
    #[test]
    fn axis_scores_ordering_stable_after_json_roundtrip() {
        let scores = all_axes_scores(0.1, 0.5, 0.9);

        let json = serde_json::to_string(&scores).unwrap();
        let recovered: AxisScores = serde_json::from_str(&json).unwrap();

        // BTreeMap maintains Ord-based ordering after deserialization
        let keys: Vec<&Axis> = recovered.keys().collect();
        assert_eq!(keys.len(), 3);
        assert_eq!(*keys[0], Axis::Coherence);
        assert_eq!(*keys[1], Axis::Hierarchy);
        assert_eq!(*keys[2], Axis::Intentionality);

        // Values preserved at correct positions
        assert!((recovered[&Axis::Coherence] - 0.1).abs() < f64::EPSILON);
        assert!((recovered[&Axis::Hierarchy] - 0.5).abs() < f64::EPSILON);
        assert!((recovered[&Axis::Intentionality] - 0.9).abs() < f64::EPSILON);

        // Verify JSON key order matches (serde_json serializes BTreeMap in key order)
        let coherence_pos = json.find("coherence").unwrap();
        let hierarchy_pos = json.find("hierarchy").unwrap();
        let intentionality_pos = json.find("intentionality").unwrap();
        assert!(
            coherence_pos < hierarchy_pos && hierarchy_pos < intentionality_pos,
            "JSON key order not alphabetical: C@{coherence_pos} H@{hierarchy_pos} I@{intentionality_pos}"
        );
    }
}
