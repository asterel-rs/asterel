use super::{
    AffectLevel, GateDecision, GateReport, Locale, NaturalnessGate, NaturalnessInput,
    OutputProfile, RelationshipDistance, RuleId, TurnContextView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NaturalnessFixtureDecision {
    Pass,
    Patch,
    RequestRepair,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NaturalnessFixtureGroup {
    MechanicalList,
    ColonContinuation,
    MemoryExposure,
    CompanionTone,
    CodeBlockBypass,
    ParserPrecision,
    RelationshipAffect,
    SurfacePrivacy,
}

#[derive(Debug, Clone)]
struct NaturalnessFixture {
    name: &'static str,
    group: NaturalnessFixtureGroup,
    quality_sample: bool,
    text: &'static str,
    locale: Locale,
    profile: OutputProfile,
    turn_context: TurnContextView,
    expected_decision: NaturalnessFixtureDecision,
    expected_rules: &'static [RuleId],
    forbidden_rules: &'static [RuleId],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NaturalnessProfileFixtureCount {
    pub(crate) profile: OutputProfile,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NaturalnessGroupFixtureCount {
    pub(crate) group: NaturalnessFixtureGroup,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NaturalnessFixtureResult {
    pub(crate) name: &'static str,
    pub(crate) group: NaturalnessFixtureGroup,
    pub(crate) quality_sample: bool,
    pub(crate) profile: OutputProfile,
    pub(crate) passed: bool,
    pub(crate) expected_decision: NaturalnessFixtureDecision,
    pub(crate) actual_decision: NaturalnessFixtureDecision,
    pub(crate) actual_rules: Vec<RuleId>,
    pub(crate) failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NaturalnessFixtureEvalReport {
    pub(crate) total: usize,
    pub(crate) passed: usize,
    pub(crate) failures: Vec<String>,
    pub(crate) fixture_results: Vec<NaturalnessFixtureResult>,
    pub(crate) profile_counts: Vec<NaturalnessProfileFixtureCount>,
    pub(crate) group_counts: Vec<NaturalnessGroupFixtureCount>,
}

fn fixture_corpus() -> Vec<NaturalnessFixture> {
    let mut fixtures = Vec::new();
    fixtures.extend(core_fixture_corpus());
    fixtures.extend(parser_precision_fixture_corpus());
    fixtures.extend(context_fixture_corpus());
    fixtures
}

fn core_fixture_corpus() -> Vec<NaturalnessFixture> {
    let mut fixtures = Vec::new();
    fixtures.extend(mechanical_list_fixtures());
    fixtures.extend(colon_continuation_fixtures());
    fixtures.push(memory_exposure_fixture());
    fixtures.extend(companion_tone_fixtures());
    fixtures.push(code_block_bypass_fixture());
    fixtures
}

fn mechanical_list_fixtures() -> Vec<NaturalnessFixture> {
    vec![
        NaturalnessFixture {
            name: "mechanical_list/bold_label_patch",
            group: NaturalnessFixtureGroup::MechanicalList,
            quality_sample: false,
            text: "- **重要**: ここを見る",
            locale: Locale::Ja,
            profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView::default(),
            expected_decision: NaturalnessFixtureDecision::Patch,
            expected_rules: &[RuleId::MechanicalList],
            forbidden_rules: &[RuleId::MemoryExposure],
        },
        NaturalnessFixture {
            name: "mechanical_list/clean_short_reply",
            group: NaturalnessFixtureGroup::MechanicalList,
            quality_sample: true,
            text: "ここだけ見れば大丈夫です。次はログの一行だけ確認しよう。",
            locale: Locale::Ja,
            profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView::default(),
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[],
            forbidden_rules: &[RuleId::MechanicalList, RuleId::MemoryExposure],
        },
    ]
}

fn colon_continuation_fixtures() -> Vec<NaturalnessFixture> {
    vec![
        NaturalnessFixture {
            name: "colon_continuation/predicate_before_list",
            group: NaturalnessFixtureGroup::ColonContinuation,
            quality_sample: false,
            text: "説明します:\n\n- A\n- B",
            locale: Locale::Ja,
            profile: OutputProfile::LongAnalysis,
            turn_context: TurnContextView::default(),
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[RuleId::ColonContinuation],
            forbidden_rules: &[RuleId::MemoryExposure],
        },
        NaturalnessFixture {
            name: "colon_continuation/noun_label_list",
            group: NaturalnessFixtureGroup::ColonContinuation,
            quality_sample: true,
            text: "手順:\n\n- A\n- B",
            locale: Locale::Ja,
            profile: OutputProfile::LongAnalysis,
            turn_context: TurnContextView::default(),
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[],
            forbidden_rules: &[RuleId::ColonContinuation, RuleId::MemoryExposure],
        },
    ]
}

fn memory_exposure_fixture() -> NaturalnessFixture {
    NaturalnessFixture {
        name: "memory_exposure/public_memory_reference",
        group: NaturalnessFixtureGroup::MemoryExposure,
        quality_sample: false,
        text: "前にあなたが話したことを覚えています。",
        locale: Locale::Ja,
        profile: OutputProfile::DiscordNormal,
        turn_context: TurnContextView {
            memory_reference_allowed: false,
            ..TurnContextView::default()
        },
        expected_decision: NaturalnessFixtureDecision::Block,
        expected_rules: &[RuleId::MemoryExposure],
        forbidden_rules: &[],
    }
}

fn companion_tone_fixtures() -> Vec<NaturalnessFixture> {
    vec![
        NaturalnessFixture {
            name: "companion_tone/friendly_office_register",
            group: NaturalnessFixtureGroup::CompanionTone,
            quality_sample: false,
            text: "確認していただければと思います。",
            locale: Locale::Ja,
            profile: OutputProfile::EmotionalReply,
            turn_context: TurnContextView {
                user_affect: AffectLevel::Neutral,
                relationship_distance: RelationshipDistance::Friendly,
                ..TurnContextView::default()
            },
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[RuleId::CompanionTone],
            forbidden_rules: &[RuleId::MemoryExposure],
        },
        NaturalnessFixture {
            name: "companion_tone/friendly_plain_register",
            group: NaturalnessFixtureGroup::CompanionTone,
            quality_sample: true,
            text: "ここだけ一緒に確認しよう。",
            locale: Locale::Ja,
            profile: OutputProfile::EmotionalReply,
            turn_context: TurnContextView {
                user_affect: AffectLevel::Neutral,
                relationship_distance: RelationshipDistance::Friendly,
                ..TurnContextView::default()
            },
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[],
            forbidden_rules: &[RuleId::CompanionTone, RuleId::MemoryExposure],
        },
    ]
}

fn code_block_bypass_fixture() -> NaturalnessFixture {
    NaturalnessFixture {
        name: "code_block_bypass/mechanical_markdown_example",
        group: NaturalnessFixtureGroup::CodeBlockBypass,
        quality_sample: true,
        text: "```md\n説明します:\n- **重要**: ここを見る\n- ✅ **確認**: A\n**A** **B** **C** **D** **E** **F** **G** **H**\n完全にできます。\nまず最初に変更を行うことができます。\n大変でしたね。\n確認していただければと思います。\n```\n本文ではここだけ確認します。",
        locale: Locale::Ja,
        profile: OutputProfile::TechnicalDoc,
        turn_context: TurnContextView {
            user_affect: AffectLevel::Neutral,
            relationship_distance: RelationshipDistance::Friendly,
            recent_opening_phrases: vec!["まず".to_string()],
            ..TurnContextView::default()
        },
        expected_decision: NaturalnessFixtureDecision::Pass,
        expected_rules: &[],
        forbidden_rules: &[
            RuleId::MechanicalList,
            RuleId::EmphasisAbuse,
            RuleId::HypeLexicon,
            RuleId::ColonContinuation,
            RuleId::TechWriting,
            RuleId::CompanionTone,
        ],
    }
}

fn parser_precision_fixture_corpus() -> Vec<NaturalnessFixture> {
    vec![
        NaturalnessFixture {
            name: "parser_precision/indented_markdown_snippet",
            group: NaturalnessFixtureGroup::ParserPrecision,
            quality_sample: true,
            text: "    - **重要**: サンプルです\n    完全にできます。\n\n本文では短く補足します。",
            locale: Locale::Ja,
            profile: OutputProfile::TechnicalDoc,
            turn_context: TurnContextView::default(),
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[],
            forbidden_rules: &[RuleId::MechanicalList, RuleId::HypeLexicon],
        },
        NaturalnessFixture {
            name: "parser_precision/markdown_table_not_list",
            group: NaturalnessFixtureGroup::ParserPrecision,
            quality_sample: true,
            text: "| 項目 | 値 |\n|---|---|\n| 重要 | A |\n\n表だけ見れば十分です。",
            locale: Locale::Ja,
            profile: OutputProfile::TechnicalDoc,
            turn_context: TurnContextView::default(),
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[],
            forbidden_rules: &[RuleId::MechanicalList, RuleId::ColonContinuation],
        },
    ]
}

fn context_fixture_corpus() -> Vec<NaturalnessFixture> {
    vec![
        NaturalnessFixture {
            name: "relationship_affect/strong_negative_advice_list",
            group: NaturalnessFixtureGroup::RelationshipAffect,
            quality_sample: false,
            text: "- A\n- B\n- C\n- D",
            locale: Locale::Ja,
            profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                user_affect: AffectLevel::StrongNegative,
                ..TurnContextView::default()
            },
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[RuleId::CompanionTone],
            forbidden_rules: &[RuleId::MemoryExposure],
        },
        NaturalnessFixture {
            name: "relationship_affect/strong_negative_brief_ack",
            group: NaturalnessFixtureGroup::RelationshipAffect,
            quality_sample: true,
            text: "それはきつかったね。今は一つだけ確認しよう。",
            locale: Locale::Ja,
            profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                user_affect: AffectLevel::StrongNegative,
                ..TurnContextView::default()
            },
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[],
            forbidden_rules: &[RuleId::CompanionTone, RuleId::MemoryExposure],
        },
        NaturalnessFixture {
            name: "surface_privacy/private_memory_reference_warns",
            group: NaturalnessFixtureGroup::SurfacePrivacy,
            quality_sample: false,
            text: "前にあなたが話したことを覚えています。",
            locale: Locale::Ja,
            profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                memory_reference_allowed: true,
                ..TurnContextView::default()
            },
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[RuleId::MemoryExposure],
            forbidden_rules: &[],
        },
        NaturalnessFixture {
            name: "surface_privacy/public_safe_without_memory_exposure",
            group: NaturalnessFixtureGroup::SurfacePrivacy,
            quality_sample: true,
            text: "その流れなら、ここだけ見れば大丈夫です。",
            locale: Locale::Ja,
            profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                memory_reference_allowed: false,
                ..TurnContextView::default()
            },
            expected_decision: NaturalnessFixtureDecision::Pass,
            expected_rules: &[],
            forbidden_rules: &[RuleId::MemoryExposure],
        },
    ]
}

pub(crate) fn run_naturalness_fixture_eval() -> NaturalnessFixtureEvalReport {
    run_fixture_suite(&fixture_corpus())
}

fn run_fixture_suite(fixtures: &[NaturalnessFixture]) -> NaturalnessFixtureEvalReport {
    let gate = NaturalnessGate::default();
    let mut all_failures = Vec::new();
    let mut fixture_results = Vec::with_capacity(fixtures.len());
    let mut passed = 0usize;

    for fixture in fixtures {
        let mut failures = Vec::new();
        let input = NaturalnessInput {
            text: fixture.text,
            locale: fixture.locale,
            output_profile: fixture.profile,
            turn_context: fixture.turn_context.clone(),
        };
        let decision = gate.check(&input);
        let actual_decision = decision_kind(&decision);
        let report = report_for_decision(&decision);

        if actual_decision != fixture.expected_decision {
            failures.push(format!(
                "{} decision: expected {:?}, got {:?}",
                fixture.name, fixture.expected_decision, actual_decision
            ));
        }

        for rule in fixture.expected_rules {
            if !report.issues.iter().any(|issue| issue.rule_id == *rule) {
                failures.push(format!("{} missing expected rule {:?}", fixture.name, rule));
            }
        }

        for rule in fixture.forbidden_rules {
            if report.issues.iter().any(|issue| issue.rule_id == *rule) {
                failures.push(format!(
                    "{} emitted forbidden rule {:?}",
                    fixture.name, rule
                ));
            }
        }

        assert_spans_are_utf8_boundaries(fixture, report, &mut failures);
        assert_safe_patch_reduces_score(fixture, &decision, &gate, &mut failures);
        if failures.is_empty() {
            passed += 1;
        }
        all_failures.extend(failures.clone());
        fixture_results.push(NaturalnessFixtureResult {
            name: fixture.name,
            group: fixture.group,
            quality_sample: fixture.quality_sample,
            profile: fixture.profile,
            passed: failures.is_empty(),
            expected_decision: fixture.expected_decision,
            actual_decision,
            actual_rules: actual_rules(report),
            failures,
        });
    }

    NaturalnessFixtureEvalReport {
        total: fixtures.len(),
        passed,
        failures: all_failures,
        fixture_results,
        profile_counts: profile_counts(fixtures),
        group_counts: group_counts(fixtures),
    }
}

fn decision_kind(decision: &GateDecision) -> NaturalnessFixtureDecision {
    match decision {
        GateDecision::Pass { .. } => NaturalnessFixtureDecision::Pass,
        GateDecision::Patch { .. } => NaturalnessFixtureDecision::Patch,
        GateDecision::RequestRepair { .. } => NaturalnessFixtureDecision::RequestRepair,
        GateDecision::Block { .. } => NaturalnessFixtureDecision::Block,
    }
}

fn report_for_decision(decision: &GateDecision) -> &GateReport {
    match decision {
        GateDecision::Pass { report }
        | GateDecision::Patch { report, .. }
        | GateDecision::RequestRepair { report, .. }
        | GateDecision::Block { report, .. } => report,
    }
}

fn actual_rules(report: &GateReport) -> Vec<RuleId> {
    let mut rules = Vec::new();
    for issue in &report.issues {
        if !rules.contains(&issue.rule_id) {
            rules.push(issue.rule_id);
        }
    }
    rules
}

fn assert_spans_are_utf8_boundaries(
    fixture: &NaturalnessFixture,
    report: &GateReport,
    failures: &mut Vec<String>,
) {
    for issue in &report.issues {
        if let Some(span) = issue.span
            && (!fixture.text.is_char_boundary(span.start)
                || !fixture.text.is_char_boundary(span.end)
                || span.start > span.end
                || span.end > fixture.text.len())
        {
            failures.push(format!(
                "{} invalid UTF-8 span {:?} for {:?}",
                fixture.name, span, issue.rule_id
            ));
        }
    }
}

fn assert_safe_patch_reduces_score(
    fixture: &NaturalnessFixture,
    decision: &GateDecision,
    gate: &NaturalnessGate,
    failures: &mut Vec<String>,
) {
    let GateDecision::Patch {
        patched_text,
        report,
    } = decision
    else {
        return;
    };
    let patched_input = NaturalnessInput {
        text: patched_text,
        locale: fixture.locale,
        output_profile: fixture.profile,
        turn_context: fixture.turn_context.clone(),
    };
    let patched_decision = gate.check(&patched_input);
    let patched_report = report_for_decision(&patched_decision);
    if patched_report.score.total() >= report.score.total() {
        failures.push(format!(
            "{} patch did not reduce score: before={} after={}",
            fixture.name,
            report.score.total(),
            patched_report.score.total()
        ));
    }
}

fn profile_counts(fixtures: &[NaturalnessFixture]) -> Vec<NaturalnessProfileFixtureCount> {
    [
        OutputProfile::DiscordShort,
        OutputProfile::DiscordNormal,
        OutputProfile::LongAnalysis,
        OutputProfile::TechnicalDoc,
        OutputProfile::EmotionalReply,
        OutputProfile::SystemNotice,
    ]
    .into_iter()
    .filter_map(|profile| {
        let count = fixtures
            .iter()
            .filter(|fixture| fixture.profile == profile)
            .count();
        (count > 0).then_some(NaturalnessProfileFixtureCount { profile, count })
    })
    .collect()
}

fn group_counts(fixtures: &[NaturalnessFixture]) -> Vec<NaturalnessGroupFixtureCount> {
    [
        NaturalnessFixtureGroup::MechanicalList,
        NaturalnessFixtureGroup::ColonContinuation,
        NaturalnessFixtureGroup::MemoryExposure,
        NaturalnessFixtureGroup::CompanionTone,
        NaturalnessFixtureGroup::CodeBlockBypass,
        NaturalnessFixtureGroup::ParserPrecision,
        NaturalnessFixtureGroup::RelationshipAffect,
        NaturalnessFixtureGroup::SurfacePrivacy,
    ]
    .into_iter()
    .filter_map(|group| {
        let count = fixtures
            .iter()
            .filter(|fixture| fixture.group == group)
            .count();
        (count > 0).then_some(NaturalnessGroupFixtureCount { group, count })
    })
    .collect()
}

#[test]
fn naturalness_fixture_suite_matches_golden_expectations() {
    let fixtures = fixture_corpus();
    let report = run_fixture_suite(&fixtures);

    assert_eq!(report.total, fixtures.len());
    assert_eq!(report.passed, report.total, "{:#?}", report.failures);
    assert_eq!(
        report.profile_counts,
        vec![
            NaturalnessProfileFixtureCount {
                profile: OutputProfile::DiscordNormal,
                count: 7,
            },
            NaturalnessProfileFixtureCount {
                profile: OutputProfile::LongAnalysis,
                count: 2,
            },
            NaturalnessProfileFixtureCount {
                profile: OutputProfile::TechnicalDoc,
                count: 3,
            },
            NaturalnessProfileFixtureCount {
                profile: OutputProfile::EmotionalReply,
                count: 2,
            },
        ]
    );
    assert_eq!(
        report.group_counts,
        vec![
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::MechanicalList,
                count: 2,
            },
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::ColonContinuation,
                count: 2,
            },
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::MemoryExposure,
                count: 1,
            },
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::CompanionTone,
                count: 2,
            },
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::CodeBlockBypass,
                count: 1,
            },
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::ParserPrecision,
                count: 2,
            },
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::RelationshipAffect,
                count: 2,
            },
            NaturalnessGroupFixtureCount {
                group: NaturalnessFixtureGroup::SurfacePrivacy,
                count: 2,
            },
        ]
    );
}
