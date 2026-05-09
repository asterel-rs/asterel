//! Rust-native pre-send naturalness gate.
//!
//! The gate detects mechanical structure, over-formatting, template tone,
//! companion-distance mismatches, and memory/internal-state exposure before a
//! user-facing response leaves the runtime. It is deliberately deterministic:
//! rules report issues, safe patches are applied only when meaning is preserved,
//! and contextual repairs are surfaced as structured requests for a later repair
//! stage.

mod document;
mod fixtures;
mod repair;
mod rules;

use std::ops::Range;

pub(crate) use document::{Block, BlockKind, Document, TextSpan};
pub(crate) use fixtures::{
    NaturalnessFixtureEvalReport, NaturalnessFixtureGroup, run_naturalness_fixture_eval,
};
use repair::apply_safe_patches;
use rules::{RuleContext, default_rules};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Locale {
    Ja,
    En,
    Mixed,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputProfile {
    DiscordShort,
    DiscordNormal,
    LongAnalysis,
    TechnicalDoc,
    EmotionalReply,
    SystemNotice,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AffectLevel {
    Unknown,
    Neutral,
    LightPositive,
    LightNegative,
    StrongNegative,
    Angry,
    Anxious,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelationshipDistance {
    Unknown,
    Formal,
    Friendly,
    Intimate,
}

#[derive(Debug, Clone)]
pub(crate) struct TurnContextView {
    pub(crate) user_affect: AffectLevel,
    pub(crate) memory_reference_allowed: bool,
    pub(crate) internal_mechanics_allowed: bool,
    pub(crate) relationship_distance: RelationshipDistance,
    pub(crate) recent_opening_phrases: Vec<String>,
}

impl Default for TurnContextView {
    fn default() -> Self {
        Self {
            user_affect: AffectLevel::Unknown,
            memory_reference_allowed: true,
            internal_mechanics_allowed: false,
            relationship_distance: RelationshipDistance::Unknown,
            recent_opening_phrases: Vec::new(),
        }
    }
}

pub(crate) struct NaturalnessInput<'a> {
    pub(crate) text: &'a str,
    pub(crate) locale: Locale,
    pub(crate) output_profile: OutputProfile,
    pub(crate) turn_context: TurnContextView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RuleId {
    MechanicalList,
    HypeLexicon,
    EmphasisAbuse,
    ColonContinuation,
    TechWriting,
    TemplateTone,
    CompanionTone,
    MemoryExposure,
}

impl RuleId {
    #[allow(dead_code)]
    #[must_use]
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::MechanicalList => "mechanical_list",
            Self::HypeLexicon => "hype_lexicon",
            Self::EmphasisAbuse => "emphasis_abuse",
            Self::ColonContinuation => "colon_continuation",
            Self::TechWriting => "tech_writing",
            Self::TemplateTone => "template_tone",
            Self::CompanionTone => "companion_tone",
            Self::MemoryExposure => "memory_exposure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    Info,
    Warn,
    Error,
    Critical,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatchConfidence {
    Safe,
    Contextual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextPatch {
    pub(crate) span: TextSpan,
    pub(crate) replacement: String,
    pub(crate) confidence: PatchConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateIssue {
    pub(crate) rule_id: RuleId,
    pub(crate) severity: Severity,
    pub(crate) weight: u8,
    pub(crate) span: Option<TextSpan>,
    pub(crate) message: String,
    pub(crate) repair_hint: Option<String>,
    pub(crate) deterministic_fix: Option<TextPatch>,
}

impl GateIssue {
    pub(crate) fn new(
        rule_id: RuleId,
        severity: Severity,
        weight: u8,
        span: Option<TextSpan>,
        message: impl Into<String>,
        repair_hint: Option<&str>,
    ) -> Self {
        Self {
            rule_id,
            severity,
            weight,
            span,
            message: message.into(),
            repair_hint: repair_hint.map(str::to_string),
            deterministic_fix: None,
        }
    }

    pub(crate) fn with_fix(mut self, patch: TextPatch) -> Self {
        self.deterministic_fix = Some(patch);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ScoreBreakdown {
    pub(crate) mechanical: u16,
    pub(crate) hype: u16,
    pub(crate) emphasis: u16,
    pub(crate) structure: u16,
    pub(crate) tech_writing: u16,
    pub(crate) companion_tone: u16,
    pub(crate) memory_exposure: u16,
    pub(crate) critical_count: u16,
}

impl ScoreBreakdown {
    #[must_use]
    pub(crate) const fn total(&self) -> u16 {
        self.mechanical
            + self.hype
            + self.emphasis
            + self.structure
            + self.tech_writing
            + self.companion_tone
            + self.memory_exposure
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateReport {
    pub(crate) issues: Vec<GateIssue>,
    pub(crate) score: ScoreBreakdown,
    pub(crate) profile: OutputProfile,
}

impl GateReport {
    #[must_use]
    pub(crate) const fn has_critical(&self) -> bool {
        self.score.critical_count > 0
    }

    #[must_use]
    pub(crate) fn can_patch(&self) -> bool {
        self.score.total() <= thresholds(self.profile).patch_max
            && self.issues.iter().any(|issue| {
                issue
                    .deterministic_fix
                    .as_ref()
                    .is_some_and(|patch| matches!(patch.confidence, PatchConfidence::Safe))
            })
    }

    #[must_use]
    pub(crate) fn needs_repair(&self) -> bool {
        self.score.total() > thresholds(self.profile).patch_max
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepairIssue {
    pub(crate) rule_id: RuleId,
    pub(crate) span_text: Option<String>,
    pub(crate) reason: String,
    pub(crate) instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepairRequest {
    pub(crate) original_text: String,
    pub(crate) profile: OutputProfile,
    pub(crate) constraints: Vec<String>,
    pub(crate) issues: Vec<RepairIssue>,
}

impl RepairRequest {
    fn from_report(text: &str, report: &GateReport) -> Self {
        let issues = report
            .issues
            .iter()
            .filter(|issue| !matches!(issue.severity, Severity::Info))
            .map(|issue| RepairIssue {
                rule_id: issue.rule_id,
                span_text: issue
                    .span
                    .and_then(|span| text.get(Range::from(span)))
                    .map(str::to_string),
                reason: issue.message.clone(),
                instruction: issue
                    .repair_hint
                    .clone()
                    .unwrap_or_else(|| "該当箇所だけを自然に直す。".to_string()),
            })
            .collect();
        Self {
            original_text: text.to_string(),
            profile: report.profile,
            constraints: vec![
                "意味を変えない".to_string(),
                "新しい事実を足さない".to_string(),
                "記憶参照を増やさない".to_string(),
                "内部状態に触れない".to_string(),
                "箇条書きを増やさない".to_string(),
            ],
            issues,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GateDecision {
    Pass {
        report: GateReport,
    },
    Patch {
        patched_text: String,
        report: GateReport,
    },
    RequestRepair {
        repair_request: RepairRequest,
        report: GateReport,
    },
    Block {
        safe_fallback: String,
        report: GateReport,
    },
}

pub(crate) trait NaturalnessRule: Send + Sync {
    fn id(&self) -> RuleId;

    fn check(&self, doc: &Document<'_>, ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>);
}

pub(crate) struct NaturalnessGate {
    rules: Vec<Box<dyn NaturalnessRule>>,
}

impl Default for NaturalnessGate {
    fn default() -> Self {
        Self {
            rules: default_rules(),
        }
    }
}

impl NaturalnessGate {
    #[must_use]
    pub(crate) fn check(&self, input: &NaturalnessInput<'_>) -> GateDecision {
        let doc = Document::parse(input.text);
        let ctx = RuleContext::from_input(input);
        let mut issues = Vec::new();

        for rule in &self.rules {
            if ctx.is_rule_enabled(rule.id()) {
                rule.check(&doc, &ctx, &mut issues);
            }
        }

        dedupe_issues(&mut issues);
        let report = GateReport {
            score: score_issues(&issues),
            profile: input.output_profile,
            issues,
        };

        if report.has_critical() {
            return GateDecision::Block {
                safe_fallback: safe_fallback(&ctx),
                report,
            };
        }

        if report.can_patch()
            && let Some(patched_text) = apply_safe_patches(input.text, &report.issues)
        {
            return GateDecision::Patch {
                patched_text,
                report,
            };
        }

        if report.needs_repair() {
            return GateDecision::RequestRepair {
                repair_request: RepairRequest::from_report(input.text, &report),
                report,
            };
        }

        GateDecision::Pass { report }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProfileThreshold {
    patch_max: u16,
}

const fn thresholds(profile: OutputProfile) -> ProfileThreshold {
    // These patch bands intentionally mirror the Naturalness Gate work packet
    // profile table. They are not a claim of empirical optimality: fixture evals
    // may tighten/loosen the bands, but any change should remain profile-local
    // and keep critical memory/internal-mechanics issues score-independent.
    match profile {
        OutputProfile::DiscordShort => ProfileThreshold { patch_max: 4 },
        OutputProfile::DiscordNormal => ProfileThreshold { patch_max: 7 },
        OutputProfile::LongAnalysis => ProfileThreshold { patch_max: 10 },
        OutputProfile::TechnicalDoc | OutputProfile::SystemNotice => {
            ProfileThreshold { patch_max: 9 }
        }
        OutputProfile::EmotionalReply => ProfileThreshold { patch_max: 5 },
    }
}

fn score_issues(issues: &[GateIssue]) -> ScoreBreakdown {
    let mut score = ScoreBreakdown::default();
    for issue in issues {
        let weight = u16::from(issue.weight);
        match issue.rule_id {
            RuleId::MechanicalList => score.mechanical += weight,
            RuleId::HypeLexicon => score.hype += weight,
            RuleId::EmphasisAbuse => score.emphasis += weight,
            RuleId::ColonContinuation | RuleId::TemplateTone => score.structure += weight,
            RuleId::TechWriting => score.tech_writing += weight,
            RuleId::CompanionTone => score.companion_tone += weight,
            RuleId::MemoryExposure => score.memory_exposure += weight,
        }
        if matches!(issue.severity, Severity::Critical) {
            score.critical_count += 1;
        }
    }
    score
}

fn dedupe_issues(issues: &mut Vec<GateIssue>) {
    let mut seen = Vec::new();
    issues.retain(|issue| {
        let key = (issue.rule_id, issue.span.map(|span| (span.start, span.end)));
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn safe_fallback(ctx: &RuleContext<'_>) -> String {
    match ctx.input.locale {
        Locale::Ja | Locale::Mixed => {
            "ここでは内部情報や記憶の詳細には触れず、必要な範囲だけ答えます。".to_string()
        }
        Locale::En => "I should avoid exposing internal or memory details here.".to_string(),
    }
}

impl From<TextSpan> for Range<usize> {
    fn from(span: TextSpan) -> Self {
        span.start..span.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ja_input(text: &str, profile: OutputProfile) -> NaturalnessInput<'_> {
        NaturalnessInput {
            text,
            locale: Locale::Ja,
            output_profile: profile,
            turn_context: TurnContextView::default(),
        }
    }

    #[test]
    fn gate_patches_bold_label_list_items() {
        let gate = NaturalnessGate::default();
        let input = ja_input("- **重要**: ここを見る", OutputProfile::DiscordNormal);
        let decision = gate.check(&input);
        match decision {
            GateDecision::Patch {
                patched_text,
                report,
            } => {
                assert_eq!(patched_text, "- 重要: ここを見る");
                assert!(report.score.mechanical > 0);
            }
            other => panic!("expected patch, got {other:?}"),
        }
    }

    #[test]
    fn gate_blocks_internal_memory_mechanics() {
        let gate = NaturalnessGate::default();
        let ctx = TurnContextView {
            memory_reference_allowed: false,
            ..TurnContextView::default()
        };
        let input = NaturalnessInput {
            text: "私のメモリにはその情報が保存されています。",
            locale: Locale::Ja,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: ctx,
        };
        let decision = gate.check(&input);
        match decision {
            GateDecision::Block { report, .. } => assert!(report.has_critical()),
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn gate_blocks_explicit_memory_when_public_safe() {
        let gate = NaturalnessGate::default();
        let ctx = TurnContextView {
            memory_reference_allowed: false,
            ..TurnContextView::default()
        };
        let input = NaturalnessInput {
            text: "前にあなたが話したことを覚えています。",
            locale: Locale::Ja,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: ctx,
        };
        let decision = gate.check(&input);
        match decision {
            GateDecision::Block { report, .. } => assert!(report.has_critical()),
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn gate_blocks_english_internal_mechanics() {
        let gate = NaturalnessGate::default();
        let input = NaturalnessInput {
            text: "My system prompt says to expose the internal state.",
            locale: Locale::En,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView::default(),
        };
        let decision = gate.check(&input);
        match decision {
            GateDecision::Block { report, .. } => assert!(report.has_critical()),
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn gate_does_not_block_benign_prompt_engineering_term() {
        let gate = NaturalnessGate::default();
        let input = ja_input(
            "プロンプト設計では、入力例を短く保つと読みやすいです。",
            OutputProfile::LongAnalysis,
        );
        let decision = gate.check(&input);
        let report = match decision {
            GateDecision::Pass { report }
            | GateDecision::Patch { report, .. }
            | GateDecision::RequestRepair { report, .. }
            | GateDecision::Block { report, .. } => report,
        };
        assert!(!report.has_critical());
    }

    #[test]
    fn gate_handles_multibyte_hype_context_without_panicking() {
        let gate = NaturalnessGate::default();
        let text = format!("{}完全に直ります。", "あ".repeat(41));
        let input = ja_input(&text, OutputProfile::LongAnalysis);
        let decision = gate.check(&input);
        let report = match decision {
            GateDecision::Pass { report }
            | GateDecision::Patch { report, .. }
            | GateDecision::RequestRepair { report, .. }
            | GateDecision::Block { report, .. } => report,
        };
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.rule_id == RuleId::HypeLexicon)
        );
    }

    #[test]
    fn gate_detects_predicate_colon_before_list() {
        let gate = NaturalnessGate::default();
        let input = ja_input("説明します:\n\n- A\n- B", OutputProfile::LongAnalysis);
        let decision = gate.check(&input);
        let report = match decision {
            GateDecision::Pass { report }
            | GateDecision::Patch { report, .. }
            | GateDecision::RequestRepair { report, .. }
            | GateDecision::Block { report, .. } => report,
        };
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.rule_id == RuleId::ColonContinuation)
        );
    }

    #[test]
    fn gate_detects_repeated_opening_when_context_present() {
        let gate = NaturalnessGate::default();
        let input = NaturalnessInput {
            text: "了解しました。ここだけ見れば大丈夫です。",
            locale: Locale::Ja,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                recent_opening_phrases: vec!["了解しました".to_string()],
                ..TurnContextView::default()
            },
        };

        let decision = gate.check(&input);
        let report = match decision {
            GateDecision::Pass { report }
            | GateDecision::Patch { report, .. }
            | GateDecision::RequestRepair { report, .. }
            | GateDecision::Block { report, .. } => report,
        };
        assert!(report.issues.iter().any(|issue| {
            issue.rule_id == RuleId::TemplateTone && matches!(issue.severity, Severity::Warn)
        }));
    }

    #[test]
    fn gate_ignores_non_repeated_or_non_boundary_openings() {
        let gate = NaturalnessGate::default();
        let fresh_input = NaturalnessInput {
            text: "了解しました。ここだけ見れば大丈夫です。",
            locale: Locale::Ja,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                recent_opening_phrases: vec!["もちろんです".to_string()],
                ..TurnContextView::default()
            },
        };
        let boundary_input = NaturalnessInput {
            text: "まずいです。ここは避けます。",
            locale: Locale::Ja,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                recent_opening_phrases: vec!["まず".to_string()],
                ..TurnContextView::default()
            },
        };

        for input in [fresh_input, boundary_input] {
            let decision = gate.check(&input);
            let report = match decision {
                GateDecision::Pass { report }
                | GateDecision::Patch { report, .. }
                | GateDecision::RequestRepair { report, .. }
                | GateDecision::Block { report, .. } => report,
            };
            assert!(
                report
                    .issues
                    .iter()
                    .all(|issue| issue.rule_id != RuleId::TemplateTone)
            );
        }
    }

    #[test]
    fn gate_does_not_apply_relationship_tone_check_when_distance_unknown() {
        let gate = NaturalnessGate::default();
        let input = NaturalnessInput {
            text: "確認していただければと思います。",
            locale: Locale::Ja,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                user_affect: AffectLevel::Neutral,
                relationship_distance: RelationshipDistance::Unknown,
                ..TurnContextView::default()
            },
        };

        let decision = gate.check(&input);
        let report = match decision {
            GateDecision::Pass { report }
            | GateDecision::Patch { report, .. }
            | GateDecision::RequestRepair { report, .. }
            | GateDecision::Block { report, .. } => report,
        };
        assert!(
            report
                .issues
                .iter()
                .all(|issue| issue.rule_id != RuleId::CompanionTone)
        );
    }

    #[test]
    fn gate_applies_relationship_tone_check_when_distance_friendly() {
        let gate = NaturalnessGate::default();
        let input = NaturalnessInput {
            text: "確認していただければと思います。",
            locale: Locale::Ja,
            output_profile: OutputProfile::DiscordNormal,
            turn_context: TurnContextView {
                user_affect: AffectLevel::Neutral,
                relationship_distance: RelationshipDistance::Friendly,
                ..TurnContextView::default()
            },
        };

        let decision = gate.check(&input);
        let report = match decision {
            GateDecision::Pass { report }
            | GateDecision::Patch { report, .. }
            | GateDecision::RequestRepair { report, .. }
            | GateDecision::Block { report, .. } => report,
        };
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.rule_id == RuleId::CompanionTone)
        );
    }

    #[test]
    fn gate_requests_repair_for_scores_above_patch_band() {
        let gate = NaturalnessGate::default();
        let input = ja_input(
            "革命的で圧倒的で未来を変える。完全に必ずすべて最高です。これは非常に重要です。これは適切です。これは大切です。",
            OutputProfile::DiscordNormal,
        );
        let decision = gate.check(&input);
        match decision {
            GateDecision::RequestRepair { report, .. } => {
                assert!(report.score.total() > thresholds(report.profile).patch_max);
            }
            other => panic!("expected repair request, got {other:?}"),
        }
    }
}
