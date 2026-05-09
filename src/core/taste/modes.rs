//! Render mode enum mapping user preference ratings to output
//! verbosity levels (concise bullets, structured outline, etc.).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Output verbosity mode derived from user preference ratings.
pub(crate) enum RenderMode {
    /// Terse bullet points for users preferring minimal output.
    ConciseBullets,
    /// Headed outline with sub-points for structured thinkers.
    StructuredOutline,
    /// Compact prose paragraphs (default neutral mode).
    ConciseProse,
    /// Step-by-step walkthrough with full explanations.
    DetailedWalkthrough,
}

impl RenderMode {
    #[must_use]
    /// Map a numeric preference rating to a render mode.
    pub(crate) fn from_rating(rating: f64) -> Self {
        if rating > 1.5 {
            Self::DetailedWalkthrough
        } else if rating > 0.5 {
            Self::StructuredOutline
        } else if rating > -0.5 {
            Self::ConciseProse
        } else {
            Self::ConciseBullets
        }
    }

    #[must_use]
    /// Return a human-readable instruction string for this mode.
    pub(crate) fn as_instruction(self) -> &'static str {
        match self {
            Self::ConciseBullets => "Use concise bullet points. Be brief and direct.",
            Self::StructuredOutline => {
                "Use a structured outline with clear headers and sub-points."
            }
            Self::ConciseProse => "Write in compact prose paragraphs. Be clear but not verbose.",
            Self::DetailedWalkthrough => {
                "Provide a detailed step-by-step walkthrough with explanations."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RenderMode;

    #[test]
    fn from_rating_maps_to_expected_modes() {
        assert_eq!(
            RenderMode::from_rating(2.0),
            RenderMode::DetailedWalkthrough
        );
        assert_eq!(RenderMode::from_rating(1.0), RenderMode::StructuredOutline);
        assert_eq!(RenderMode::from_rating(0.0), RenderMode::ConciseProse);
        assert_eq!(RenderMode::from_rating(-1.0), RenderMode::ConciseBullets);
    }

    #[test]
    fn as_instruction_is_non_empty_for_all_variants() {
        let modes = [
            RenderMode::ConciseBullets,
            RenderMode::StructuredOutline,
            RenderMode::ConciseProse,
            RenderMode::DetailedWalkthrough,
        ];

        for mode in modes {
            assert!(!mode.as_instruction().trim().is_empty());
        }
    }
}
