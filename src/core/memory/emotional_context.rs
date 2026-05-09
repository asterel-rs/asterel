//! Lightweight rule-based emotional context detection.
//!
//! Scans message text for keyword patterns to infer an
//! [`EmotionalContext`] label with `valence` (positive/negative,
//! −1.0–1.0) and `arousal` (calm/excited, 0.0–1.0) dimensions.
//! Falls back to a neutral context when no keywords match.

#[derive(Debug, Clone, PartialEq)]
pub struct EmotionalContext {
    pub label: String,
    pub valence: f64,
    pub arousal: f64,
    pub confidence: f64,
}

struct EmotionRule {
    label: &'static str,
    valence: f64,
    arousal: f64,
    keywords: &'static [&'static str],
}

const EMOTION_RULES: [EmotionRule; 5] = [
    EmotionRule {
        label: "joy",
        valence: 0.8,
        arousal: 0.6,
        keywords: &[
            "joy",
            "happy",
            "love",
            "excited",
            "grateful",
            "wonderful",
            "嬉しい",
            "楽しい",
        ],
    },
    EmotionRule {
        label: "sadness",
        valence: -0.6,
        arousal: 0.3,
        keywords: &[
            "sad",
            "disappointed",
            "sorry",
            "miss",
            "lonely",
            "悲しい",
            "寂しい",
        ],
    },
    EmotionRule {
        label: "anger",
        valence: -0.7,
        arousal: 0.8,
        keywords: &[
            "angry",
            "frustrated",
            "annoyed",
            "furious",
            "怒り",
            "腹立つ",
        ],
    },
    EmotionRule {
        label: "fear",
        valence: -0.5,
        arousal: 0.7,
        keywords: &[
            "worried", "anxious", "scared", "afraid", "nervous", "心配", "不安",
        ],
    },
    EmotionRule {
        label: "surprise",
        valence: 0.2,
        arousal: 0.8,
        keywords: &[
            "surprised",
            "unexpected",
            "amazed",
            "shocked",
            "驚き",
            "びっくり",
        ],
    },
];

#[must_use]
pub fn infer_emotion_from_text(text: &str) -> Option<EmotionalContext> {
    let normalized = text.to_lowercase();

    let mut best_index = None;
    let mut best_count = 0usize;
    for (index, rule) in EMOTION_RULES.iter().enumerate() {
        let count = rule
            .keywords
            .iter()
            .filter(|keyword| normalized.contains(**keyword))
            .count();
        if count > best_count {
            best_count = count;
            best_index = Some(index);
        }
    }

    let index = best_index?;
    if best_count == 0 {
        return None;
    }

    let rule = &EMOTION_RULES[index];
    let confidence = if best_count == 1 {
        0.5
    } else if best_count == 2 {
        0.7
    } else {
        0.85
    };

    Some(EmotionalContext {
        label: rule.label.to_string(),
        valence: rule.valence,
        arousal: rule.arousal,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::{EmotionalContext, infer_emotion_from_text};

    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn detects_joy_in_english_with_single_keyword() {
        let ctx = infer_emotion_from_text("I feel happy today").expect("emotion expected");
        assert_eq!(
            ctx,
            EmotionalContext {
                label: "joy".to_string(),
                valence: 0.8,
                arousal: 0.6,
                confidence: 0.5,
            }
        );
    }

    #[test]
    fn detects_anger_in_english_with_multiple_keywords() {
        let ctx = infer_emotion_from_text("I am angry and frustrated").expect("emotion expected");
        assert_eq!(ctx.label, "anger");
        assert_f64_eq(ctx.confidence, 0.7);
        assert_f64_eq(ctx.valence, -0.7);
        assert_f64_eq(ctx.arousal, 0.8);
    }

    #[test]
    fn detects_japanese_emotion_keywords() {
        let ctx = infer_emotion_from_text("本当に嬉しいし楽しい").expect("emotion expected");
        assert_eq!(ctx.label, "joy");
        assert_f64_eq(ctx.confidence, 0.7);
    }

    #[test]
    fn confidence_is_high_for_three_or_more_keywords() {
        let ctx = infer_emotion_from_text("I am worried anxious and scared").expect("emotion");
        assert_eq!(ctx.label, "fear");
        assert_f64_eq(ctx.confidence, 0.85);
    }

    #[test]
    fn returns_none_when_no_keywords_match() {
        assert!(infer_emotion_from_text("The server started on port 3000").is_none());
    }
}
