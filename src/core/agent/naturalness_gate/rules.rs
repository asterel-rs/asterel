use super::document::list_item_body;
use super::{
    AffectLevel, Block, BlockKind, Document, GateIssue, Locale, NaturalnessInput, NaturalnessRule,
    OutputProfile, PatchConfidence, RelationshipDistance, RuleId, Severity, TextPatch, TextSpan,
};

pub(crate) struct RuleContext<'a> {
    pub(crate) input: &'a NaturalnessInput<'a>,
}

impl<'a> RuleContext<'a> {
    pub(crate) const fn from_input(input: &'a NaturalnessInput<'a>) -> Self {
        Self { input }
    }

    pub(crate) fn is_rule_enabled(&self, rule_id: RuleId) -> bool {
        match (self.input.output_profile, rule_id) {
            (
                OutputProfile::DiscordShort | OutputProfile::EmotionalReply,
                RuleId::TechWriting | RuleId::ColonContinuation,
            ) => false,
            // Keep repeated-opening detection dormant when the finalization caller
            // has no nearby assistant openings to compare against.
            (_, RuleId::TemplateTone)
                if self.input.turn_context.recent_opening_phrases.is_empty() =>
            {
                false
            }
            // Affect-aware companion-tone checks require a real upstream affect
            // signal. `Unknown` means the runtime has not provided that context,
            // so this rule stays inactive until a later affect-threading slice lands.
            (_, RuleId::CompanionTone)
                if matches!(self.input.turn_context.user_affect, AffectLevel::Unknown) =>
            {
                false
            }
            _ => true,
        }
    }
}

pub(super) fn default_rules() -> Vec<Box<dyn NaturalnessRule>> {
    vec![
        Box::new(MechanicalListRule),
        Box::new(EmphasisAbuseRule),
        Box::new(HypeLexiconRule),
        Box::new(MemoryExposureRule),
        Box::new(ColonContinuationRule),
        Box::new(TemplateToneRule),
        Box::new(TechWritingRule),
        Box::new(CompanionToneRule),
    ]
}

struct MechanicalListRule;

impl NaturalnessRule for MechanicalListRule {
    fn id(&self) -> RuleId {
        RuleId::MechanicalList
    }

    fn check(&self, doc: &Document<'_>, ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        for block in doc
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::ListItem))
        {
            let body = list_item_body(block.text);
            let body_start = block.text.find(body).unwrap_or(0);
            if let Some((start, end, replacement)) = bold_label_colon(body) {
                let span = block.span.offset(body_start + start, body_start + end);
                issues.push(
                    GateIssue::new(
                        self.id(),
                        Severity::Warn,
                        2,
                        Some(span),
                        "リスト項目の太字ラベルとコロンが機械的に見えます。",
                        Some("太字を外して通常の語句として書く。"),
                    )
                    .with_fix(TextPatch {
                        span,
                        replacement,
                        confidence: PatchConfidence::Safe,
                    }),
                );
            }

            if penalize_decorative_list_emoji(ctx) && starts_with_decorative_marker(body) {
                let marker_len = body.chars().next().map_or(0, char::len_utf8);
                issues.push(GateIssue::new(
                    self.id(),
                    Severity::Info,
                    1,
                    Some(block.span.offset(body_start, body_start + marker_len)),
                    "リスト冒頭の装飾記号がテンプレ的に見える場合があります。",
                    Some("必要がなければ装飾を削る。"),
                ));
            }
        }
    }
}

fn bold_label_colon(text: &str) -> Option<(usize, usize, String)> {
    let start = text.find("**")?;
    if start > 2 {
        return None;
    }
    let after_open = start + 2;
    let close = text[after_open..].find("**:").map(|idx| after_open + idx)?;
    let label = text[after_open..close].trim();
    if label.is_empty() || label.chars().count() > 12 {
        return None;
    }
    Some((start, close + 3, format!("{label}:")))
}

fn starts_with_decorative_marker(text: &str) -> bool {
    matches!(
        text.trim_start().chars().next(),
        Some('✅' | '❌' | '⭐' | '🌟' | '🔹' | '🔸' | '👉' | '⚠' | '💡')
    )
}

fn penalize_decorative_list_emoji(ctx: &RuleContext<'_>) -> bool {
    matches!(
        ctx.input.output_profile,
        OutputProfile::DiscordShort | OutputProfile::DiscordNormal | OutputProfile::EmotionalReply
    )
}

struct EmphasisAbuseRule;

impl NaturalnessRule for EmphasisAbuseRule {
    fn id(&self) -> RuleId {
        RuleId::EmphasisAbuse
    }

    fn check(&self, doc: &Document<'_>, ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        let bold_count = prose_blocks(doc)
            .map(|block| block.text.matches("**").count())
            .sum::<usize>()
            / 2;
        if bold_count >= bold_limit(ctx.input.output_profile) {
            issues.push(GateIssue::new(
                self.id(),
                Severity::Warn,
                2,
                None,
                "太字の密度が高く、情報カードのように見えます。",
                Some("強調を減らし、文の流れで重要度を出す。"),
            ));
        }

        for block in &doc.blocks {
            match block.kind {
                BlockKind::Heading { .. } if block.text.contains("**") => {
                    if let Some((start, end, replacement)) = first_bold_segment(block.text) {
                        let span = block.span.offset(start, end);
                        issues.push(
                            GateIssue::new(
                                self.id(),
                                Severity::Warn,
                                2,
                                Some(span),
                                "見出し内の太字が過剰な強調に見えます。",
                                Some("見出し自体に強調を任せる。"),
                            )
                            .with_fix(TextPatch {
                                span,
                                replacement,
                                confidence: PatchConfidence::Safe,
                            }),
                        );
                    }
                }
                BlockKind::ListItem if has_emoji_bold_combo(block.text) => {
                    issues.push(GateIssue::new(
                        self.id(),
                        Severity::Info,
                        1,
                        Some(block.span),
                        "絵文字と太字が重なり、テンプレ的な見た目になっています。",
                        Some("どちらか片方に寄せる。"),
                    ));
                }
                _ => {}
            }
        }
    }
}

fn bold_limit(profile: OutputProfile) -> usize {
    match profile {
        OutputProfile::DiscordShort | OutputProfile::EmotionalReply => 2,
        OutputProfile::DiscordNormal => 4,
        OutputProfile::LongAnalysis | OutputProfile::TechnicalDoc | OutputProfile::SystemNotice => {
            7
        }
    }
}

fn first_bold_segment(text: &str) -> Option<(usize, usize, String)> {
    let start = text.find("**")?;
    let after_open = start + 2;
    let close = text[after_open..].find("**").map(|idx| after_open + idx)?;
    Some((start, close + 2, text[after_open..close].to_string()))
}

fn has_emoji_bold_combo(text: &str) -> bool {
    starts_with_decorative_marker(text) && text.contains("**")
}

struct HypeLexiconRule;

impl NaturalnessRule for HypeLexiconRule {
    fn id(&self) -> RuleId {
        RuleId::HypeLexicon
    }

    fn check(&self, doc: &Document<'_>, ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        for phrase in HYPE_PHRASES {
            for (start, _) in prose_match_indices(doc, phrase.text) {
                let span = TextSpan::new(start, start + phrase.text.len());
                let weight = if has_nearby_evidence(doc.source, span)
                    || is_marketing_profile(ctx.input.output_profile)
                {
                    phrase.base_weight.saturating_sub(1).max(1)
                } else {
                    phrase.base_weight
                };
                issues.push(GateIssue::new(
                    self.id(),
                    Severity::Warn,
                    weight,
                    Some(span),
                    "断定や誇張が根拠より強く見える可能性があります。",
                    Some("根拠に合う強さへ弱める。"),
                ));
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct HypePhrase {
    text: &'static str,
    base_weight: u8,
}

const HYPE_PHRASES: &[HypePhrase] = &[
    HypePhrase {
        text: "完全に",
        base_weight: 2,
    },
    HypePhrase {
        text: "必ず",
        base_weight: 2,
    },
    HypePhrase {
        text: "すべて",
        base_weight: 2,
    },
    HypePhrase {
        text: "最高",
        base_weight: 2,
    },
    HypePhrase {
        text: "革命的",
        base_weight: 3,
    },
    HypePhrase {
        text: "劇的",
        base_weight: 2,
    },
    HypePhrase {
        text: "圧倒的",
        base_weight: 3,
    },
    HypePhrase {
        text: "未来を変える",
        base_weight: 3,
    },
    HypePhrase {
        text: "不可避",
        base_weight: 3,
    },
];

fn has_nearby_evidence(text: &str, span: TextSpan) -> bool {
    let start = floor_char_boundary(text, span.start.saturating_sub(80));
    let end = ceil_char_boundary(text, (span.end + 80).min(text.len()));
    let window = &text[start..end];
    window.chars().any(|ch| ch.is_ascii_digit())
        || window.contains('`')
        || window.contains(".rs")
        || window.contains("::")
        || window.contains("例えば")
        || window.contains("具体的")
}

fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(text: &str, mut idx: usize) -> usize {
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn is_marketing_profile(profile: OutputProfile) -> bool {
    matches!(profile, OutputProfile::SystemNotice)
}

struct MemoryExposureRule;

impl NaturalnessRule for MemoryExposureRule {
    fn id(&self) -> RuleId {
        RuleId::MemoryExposure
    }

    fn check(&self, doc: &Document<'_>, ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        for phrase in INTERNAL_MECHANICS {
            for (start, _) in doc.source.match_indices(phrase) {
                push_internal_mechanics_issue(self.id(), ctx, issues, start, phrase.len());
            }
        }

        let ascii_lower = doc.source.to_ascii_lowercase();
        for phrase in INTERNAL_MECHANICS_ASCII {
            for (start, _) in ascii_lower.match_indices(phrase) {
                push_internal_mechanics_issue(self.id(), ctx, issues, start, phrase.len());
            }
        }

        for phrase in EXPLICIT_MEMORY {
            for (start, _) in doc.source.match_indices(phrase) {
                push_explicit_memory_issue(self.id(), ctx, issues, start, phrase.len());
            }
        }

        for phrase in EXPLICIT_MEMORY_ASCII {
            for (start, _) in ascii_lower.match_indices(phrase) {
                push_explicit_memory_issue(self.id(), ctx, issues, start, phrase.len());
            }
        }
    }
}

fn push_internal_mechanics_issue(
    rule_id: RuleId,
    ctx: &RuleContext<'_>,
    issues: &mut Vec<GateIssue>,
    start: usize,
    len: usize,
) {
    let severity = if ctx.input.turn_context.internal_mechanics_allowed {
        Severity::Warn
    } else {
        Severity::Critical
    };
    issues.push(GateIssue::new(
        rule_id,
        severity,
        8,
        Some(TextSpan::new(start, start + len)),
        "内部状態やシステム都合の露出です。",
        Some("内部機構に触れない表現へ直す。"),
    ));
}

fn push_explicit_memory_issue(
    rule_id: RuleId,
    ctx: &RuleContext<'_>,
    issues: &mut Vec<GateIssue>,
    start: usize,
    len: usize,
) {
    let severity = if ctx.input.turn_context.memory_reference_allowed {
        Severity::Warn
    } else {
        Severity::Critical
    };
    issues.push(GateIssue::new(
        rule_id,
        severity,
        if matches!(severity, Severity::Critical) {
            5
        } else {
            2
        },
        Some(TextSpan::new(start, start + len)),
        "記憶参照が露骨に見える可能性があります。",
        Some("必要なら、記憶を見せずに文脈へ自然に接続する。"),
    ));
}

const INTERNAL_MECHANICS: &[&str] = &[
    "私のメモリ",
    "メモリには",
    "内部的には",
    "システムプロンプト",
    "私のプロンプト",
    "内部プロンプト",
    "隠し指示",
    "内部指示",
    "ポリシーでは",
    "内部状態",
];

const INTERNAL_MECHANICS_ASCII: &[&str] = &[
    "my memory",
    "memory store",
    "system prompt",
    "internal prompt",
    "hidden instruction",
    "hidden instructions",
    "internal instruction",
    "internal instructions",
    "internal state",
    "internal policy",
    "verifier",
];

const EXPLICIT_MEMORY: &[&str] = &[
    "覚えています",
    "記憶しています",
    "前にあなた",
    "メモリに保存されています",
    "記憶に保存されています",
];

const EXPLICIT_MEMORY_ASCII: &[&str] = &[
    "i remember you",
    "i remember that you",
    "i have stored",
    "saved in memory",
    "stored in memory",
];

struct ColonContinuationRule;

impl NaturalnessRule for ColonContinuationRule {
    fn id(&self) -> RuleId {
        RuleId::ColonContinuation
    }

    fn check(&self, doc: &Document<'_>, _ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        for (idx, current) in doc.blocks.iter().enumerate() {
            let Some(next) = doc.blocks[idx + 1..]
                .iter()
                .find(|block| !matches!(block.kind, BlockKind::Blank))
            else {
                continue;
            };
            if !matches!(
                current.kind,
                BlockKind::Paragraph | BlockKind::Heading { .. }
            ) || !ends_with_colon(current.text)
                || !matches!(
                    next.kind,
                    BlockKind::ListItem
                        | BlockKind::List
                        | BlockKind::CodeBlock
                        | BlockKind::Quote
                        | BlockKind::Table
                )
            {
                continue;
            }
            if matches!(
                classify_colon_ending(current.text),
                EndingClass::PredicateLike | EndingClass::ConnectiveLike
            ) {
                issues.push(GateIssue::new(
                    self.id(),
                    Severity::Warn,
                    2,
                    Some(current.span),
                    "述語的なコロン導入が機械的な展開に見えます。",
                    Some("自然な文でつなぐか、名詞ラベルにする。"),
                ));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndingClass {
    NounLike,
    PredicateLike,
    ConnectiveLike,
    EnglishLabel,
    Unknown,
}

fn ends_with_colon(text: &str) -> bool {
    text.trim_end().ends_with(':') || text.trim_end().ends_with('：')
}

fn classify_colon_ending(text: &str) -> EndingClass {
    let s = text.trim().trim_end_matches([':', '：']).trim();
    if s.is_ascii()
        && s.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace())
    {
        return EndingClass::EnglishLabel;
    }
    if s.chars().count() <= 2
        || ends_with_any(
            s,
            &[
                "方法",
                "手順",
                "概要",
                "構成",
                "設定",
                "例",
                "使用方法",
                "注意点",
            ],
        )
    {
        return EndingClass::NounLike;
    }
    if ends_with_any(
        s,
        &[
            "ます",
            "ました",
            "です",
            "でした",
            "します",
            "しました",
            "説明します",
            "示します",
            "実行します",
            "できます",
        ],
    ) {
        return EndingClass::PredicateLike;
    }
    if ends_with_any(s, &["例えば", "たとえば", "以下", "次に", "具体的には"]) {
        return EndingClass::ConnectiveLike;
    }
    EndingClass::Unknown
}

fn ends_with_any(text: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|suffix| text.ends_with(suffix))
}

struct TemplateToneRule;

// Phase 3-A note: this rule is active for finalization callers that thread
// provider conversation history into `TurnContextView::recent_opening_phrases`.
// Empty history still leaves the rule dormant rather than guessing from one
// response and creating false positives.

impl NaturalnessRule for TemplateToneRule {
    fn id(&self) -> RuleId {
        RuleId::TemplateTone
    }

    fn check(&self, doc: &Document<'_>, ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        let trimmed = doc.source.trim_start();
        let leading_len = doc.source.len() - trimmed.len();
        for phrase in TEMPLATE_OPENINGS {
            if starts_with_opening_phrase(trimmed, phrase)
                && ctx
                    .input
                    .turn_context
                    .recent_opening_phrases
                    .iter()
                    .any(|recent| recent == phrase)
            {
                issues.push(GateIssue::new(
                    self.id(),
                    Severity::Warn,
                    3,
                    Some(TextSpan::new(leading_len, leading_len + phrase.len())),
                    "冒頭がテンプレ進行に寄っています。",
                    Some("直前文脈へ直接接続する。"),
                ));
            }
        }
    }
}

const TEMPLATE_OPENINGS: &[&str] = &[
    "結論から言うと",
    "まず",
    "もちろんです",
    "了解しました",
    "いい質問です",
    "まとめると",
];

fn starts_with_opening_phrase(text: &str, phrase: &str) -> bool {
    let Some(rest) = text.strip_prefix(phrase) else {
        return false;
    };
    rest.chars().next().is_none_or(is_opening_boundary)
}

fn is_opening_boundary(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | ' ' | '　' | '。' | '！' | '？' | '!' | '?' | '、' | ',' | ':' | '：'
    )
}

struct TechWritingRule;

impl NaturalnessRule for TechWritingRule {
    fn id(&self) -> RuleId {
        RuleId::TechWriting
    }

    fn check(&self, doc: &Document<'_>, _ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        for phrase in VERBOSE_PHRASES {
            for (start, _) in prose_match_indices(doc, phrase.from) {
                let span = TextSpan::new(start, start + phrase.from.len());
                issues.push(
                    GateIssue::new(
                        self.id(),
                        Severity::Info,
                        1,
                        Some(span),
                        "冗長な技術文に見える表現です。",
                        Some("短い表現へ寄せる。"),
                    )
                    .with_fix(TextPatch {
                        span,
                        replacement: phrase.to.to_string(),
                        confidence: PatchConfidence::Safe,
                    }),
                );
            }
        }

        for sentence in &doc.sentences {
            if sentence.text.chars().count() > 90 {
                issues.push(GateIssue::new(
                    self.id(),
                    Severity::Warn,
                    2,
                    Some(sentence.span),
                    "文が長く、読み手の負荷が高いです。",
                    Some("一文一主張に分ける。"),
                ));
            }
            if contains_abstract_without_anchor(sentence.text) {
                issues.push(GateIssue::new(
                    self.id(),
                    Severity::Info,
                    1,
                    Some(sentence.span),
                    "抽象語に対する具体が不足しています。",
                    Some("具体例、型名、ファイル名、条件を添える。"),
                ));
            }
        }
    }
}

struct ReplacementPhrase {
    from: &'static str,
    to: &'static str,
}

const VERBOSE_PHRASES: &[ReplacementPhrase] = &[
    ReplacementPhrase {
        from: "まず最初に",
        to: "まず",
    },
    ReplacementPhrase {
        from: "することができます",
        to: "できます",
    },
    ReplacementPhrase {
        from: "実装を実施する",
        to: "実装する",
    },
    ReplacementPhrase {
        from: "変更を行う",
        to: "変更する",
    },
];

fn contains_abstract_without_anchor(text: &str) -> bool {
    let has_abstract = [
        "重要です",
        "有効です",
        "適切です",
        "大切です",
        "必要に応じて",
    ]
    .iter()
    .any(|phrase| text.contains(phrase));
    has_abstract && !has_nearby_evidence(text, TextSpan::new(0, text.len()))
}

struct CompanionToneRule;

// Phase 3 note: affect-aware branches are active only when upstream code
// provides a non-Unknown affect signal. Relationship-distance branches require a
// mapped non-Unknown distance from the canonical relationship state.

impl NaturalnessRule for CompanionToneRule {
    fn id(&self) -> RuleId {
        RuleId::CompanionTone
    }

    fn check(&self, doc: &Document<'_>, ctx: &RuleContext<'_>, issues: &mut Vec<GateIssue>) {
        if matches!(
            ctx.input.turn_context.user_affect,
            AffectLevel::Unknown | AffectLevel::Neutral | AffectLevel::LightPositive
        ) && UNEARNED_EMPATHY
            .iter()
            .any(|phrase| prose_contains(doc, phrase))
        {
            issues.push(GateIssue::new(
                self.id(),
                Severity::Warn,
                3,
                None,
                "感情が出ていない場面で共感が強すぎます。",
                Some("相手の感情を決めつけず、文脈にだけ接続する。"),
            ));
        }

        if matches!(
            ctx.input.turn_context.user_affect,
            AffectLevel::StrongNegative | AffectLevel::Angry | AffectLevel::Anxious
        ) && doc
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::ListItem))
            .count()
            >= 4
        {
            issues.push(GateIssue::new(
                self.id(),
                Severity::Error,
                3,
                None,
                "強い感情の場面で助言リストが先に出ています。",
                Some("短く受け止めてから必要な一点だけ返す。"),
            ));
        }

        if matches!(
            ctx.input.turn_context.relationship_distance,
            RelationshipDistance::Friendly | RelationshipDistance::Intimate
        ) && prose_contains(doc, "していただければと思います")
        {
            issues.push(GateIssue::new(
                self.id(),
                Severity::Info,
                1,
                None,
                "距離感に対して急に事務的です。",
                Some("関係性に合う自然な言い方へ寄せる。"),
            ));
        }
    }
}

const UNEARNED_EMPATHY: &[&str] = &[
    "それはつらかったですね",
    "大変でしたね",
    "不安ですよね",
    "苦しかったですね",
];

fn prose_blocks<'a>(doc: &'a Document<'a>) -> impl Iterator<Item = &'a Block<'a>> {
    doc.blocks
        .iter()
        .filter(|block| !matches!(block.kind, BlockKind::CodeBlock))
}

fn prose_match_indices<'a>(
    doc: &'a Document<'a>,
    phrase: &'a str,
) -> impl Iterator<Item = (usize, &'a str)> + 'a {
    prose_blocks(doc).flat_map(move |block| {
        block
            .text
            .match_indices(phrase)
            .map(move |(relative_start, matched)| (block.span.start + relative_start, matched))
    })
}

fn prose_contains(doc: &Document<'_>, phrase: &str) -> bool {
    prose_blocks(doc).any(|block| block.text.contains(phrase))
}

#[allow(dead_code)]
fn _locale_is_japanese(locale: Locale) -> bool {
    matches!(locale, Locale::Ja | Locale::Mixed)
}
