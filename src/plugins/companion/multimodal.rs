//! Multimodal media handling for the companion plugin.
//!
//! Processes photo and sticker media events with emotional impact
//! scoring and converts them into memory signal envelopes.

use std::fmt::Write as FmtWrite;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contracts::ids::EntityId;
use crate::contracts::scores::Confidence;
use crate::core::memory::{SignalEnvelope, SignalTier, SourceKind};

/// Kind of media processed by the companion multimodal pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionMediaKind {
    /// Photographic image.
    Photo,
    /// Sticker or emoji image.
    Sticker,
}

impl CompanionMediaKind {
    /// Returns the `snake_case` string label for this media kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Sticker => "sticker",
        }
    }
}

/// Emotional impact scores for a companion media event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionEmotionalImpact {
    /// Positive/negative sentiment (-1.0 to 1.0).
    pub valence: f64,
    /// Intensity of the emotion (0.0 to 1.0).
    pub arousal: f64,
    /// Confidence in the emotional assessment (0.0 to 1.0).
    pub confidence: Confidence,
    /// Free-form emotion tags (e.g. `"joy"`, `"calm"`).
    #[serde(default)]
    pub tags: Vec<String>,
}

impl CompanionEmotionalImpact {
    /// # Errors
    ///
    /// Returns an error when values are non-finite or tags are invalid.
    pub fn normalize(mut self) -> Result<Self> {
        if !self.valence.is_finite() {
            anyhow::bail!("emotional_impact.valence must be finite");
        }
        if !self.arousal.is_finite() {
            anyhow::bail!("emotional_impact.arousal must be finite");
        }
        if !self.confidence.get().is_finite() {
            anyhow::bail!("emotional_impact.confidence must be finite");
        }

        self.valence = self.valence.clamp(-1.0, 1.0);
        self.arousal = self.arousal.clamp(0.0, 1.0);
        self.confidence = Confidence::new(self.confidence.get());
        self.tags = self
            .tags
            .into_iter()
            .map(|tag| normalize_tag(&tag))
            .collect::<Result<Vec<_>>>()?;

        Ok(self)
    }
}

/// A multimodal media memory record with descriptors and emotion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionMultimodalMemoryRecord {
    /// Unique record identifier.
    pub record_id: String,
    /// Entity this record is associated with.
    pub entity_id: EntityId,
    /// Media source reference (e.g. `"camera/frame_001"`).
    pub source_ref: String,
    /// Type of media captured.
    pub media_kind: CompanionMediaKind,
    /// Short textual descriptors of the media content.
    pub descriptors: Vec<String>,
    /// Optional speech transcript from the media.
    #[serde(default)]
    pub transcript: Option<String>,
    /// Optional emotional impact assessment.
    #[serde(default)]
    pub emotional_impact: Option<CompanionEmotionalImpact>,
    /// RFC 3339 capture timestamp.
    pub captured_at: String,
}

impl CompanionMultimodalMemoryRecord {
    /// # Errors
    ///
    /// Returns an error when entity/source/descriptors are invalid.
    pub fn new(
        entity_id: impl Into<EntityId>,
        source_ref: impl Into<String>,
        media_kind: CompanionMediaKind,
        descriptors: Vec<String>,
    ) -> Result<Self> {
        let record = Self {
            record_id: Uuid::new_v4().to_string(),
            entity_id: entity_id.into(),
            source_ref: source_ref.into(),
            media_kind,
            descriptors,
            transcript: None,
            emotional_impact: None,
            captured_at: chrono::Utc::now().to_rfc3339(),
        };
        record.validate_contract()?;
        Ok(record)
    }

    /// Attaches a speech transcript to this record.
    #[must_use]
    pub fn with_transcript(mut self, transcript: impl Into<String>) -> Self {
        self.transcript = Some(transcript.into());
        self
    }

    /// Attaches an emotional impact assessment to this record.
    #[must_use]
    pub fn with_emotional_impact(mut self, emotional_impact: CompanionEmotionalImpact) -> Self {
        self.emotional_impact = Some(emotional_impact);
        self
    }

    /// # Errors
    ///
    /// Returns an error when record fields violate multimodal memory contract.
    pub fn validate_contract(&self) -> Result<()> {
        let entity_id = normalize_identifier("entity_id", self.entity_id.as_str(), false)?;
        if entity_id.len() > 128 {
            anyhow::bail!("entity_id must be <= 128 chars");
        }

        let source_ref = normalize_identifier("source_ref", &self.source_ref, true)?;
        if source_ref.len() > 256 {
            anyhow::bail!("source_ref must be <= 256 chars");
        }

        if self.descriptors.is_empty() {
            anyhow::bail!("descriptors must contain at least one value");
        }
        for descriptor in &self.descriptors {
            let normalized = descriptor.trim();
            if normalized.is_empty() {
                anyhow::bail!("descriptor must not be empty");
            }
            if normalized.len() > 120 {
                anyhow::bail!("descriptor must be <= 120 chars");
            }
        }

        if let Some(transcript) = &self.transcript
            && transcript.trim().is_empty()
        {
            anyhow::bail!("transcript must not be empty");
        }

        if let Some(emotional_impact) = &self.emotional_impact {
            emotional_impact.clone().normalize()?;
        }

        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when record normalization fails.
    pub fn to_signal_envelope(&self) -> Result<SignalEnvelope> {
        self.validate_contract()?;
        let mut content = format!(
            "media={} descriptors={}",
            self.media_kind.as_str(),
            self.descriptors.join(", ")
        );
        if let Some(transcript) = self.transcript.as_deref() {
            let _ = write!(content, " | transcript={}", transcript.trim());
        }
        if let Some(emotional_impact) = &self.emotional_impact {
            let normalized = emotional_impact.clone().normalize()?;
            let _ = write!(
                content,
                " | emotion=valence:{:.3},arousal:{:.3},confidence:{:.3}",
                normalized.valence,
                normalized.arousal,
                normalized.confidence.get()
            );
            if !normalized.tags.is_empty() {
                let _ = write!(content, " | emotion_tags={}", normalized.tags.join("|"));
            }
        }

        let mut envelope = SignalEnvelope::new(
            SourceKind::Conversation,
            format!(
                "companion/{}/{}",
                self.media_kind.as_str(),
                self.source_ref.trim()
            ),
            content,
            self.entity_id.as_str().trim(),
        )
        .with_signal_tier(SignalTier::Belief)
        .with_metadata("companion_media_kind", self.media_kind.as_str())
        .with_metadata("descriptor_count", self.descriptors.len().to_string())
        .with_metadata("captured_at", &self.captured_at);

        if let Some(transcript) = self.transcript.as_deref() {
            envelope = envelope
                .with_metadata("has_transcript", "true")
                .with_metadata(
                    "transcript_length",
                    transcript.trim().chars().count().to_string(),
                );
        }
        if let Some(emotional_impact) = &self.emotional_impact {
            let normalized = emotional_impact.clone().normalize()?;
            envelope = envelope
                .with_metadata("emotion_valence", format!("{:.3}", normalized.valence))
                .with_metadata("emotion_arousal", format!("{:.3}", normalized.arousal))
                .with_metadata(
                    "emotion_confidence",
                    format!("{:.3}", normalized.confidence.get()),
                );
            if !normalized.tags.is_empty() {
                envelope = envelope.with_metadata("emotion_tags", normalized.tags.join("|"));
            }
        }

        envelope.normalize().map_err(Into::into)
    }
}

/// A single voice activity detection frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CompanionVadFrame {
    /// Timestamp in milliseconds from stream start.
    pub at_ms: u64,
    /// Probability that this frame contains speech (0.0 to 1.0).
    pub speech_probability: f64,
}

/// A contiguous speech segment detected by VAD segmentation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanionSpeechSegment {
    /// Unique segment identifier.
    pub segment_id: String,
    /// Start timestamp in milliseconds.
    pub start_ms: u64,
    /// End timestamp in milliseconds.
    pub end_ms: u64,
    /// Average speech probability across all frames.
    pub avg_speech_probability: f64,
    /// Number of VAD frames in this segment.
    pub frame_count: usize,
}

/// Policy controlling VAD-based speech segmentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompanionVadSegmentationPolicy {
    /// Minimum probability to consider a frame as speech.
    pub speech_threshold: f64,
    /// Milliseconds of silence that split segments.
    pub silence_gap_ms: u64,
    /// Minimum segment duration to keep (milliseconds).
    pub min_segment_ms: u64,
    /// Maximum segment duration before forced split.
    pub max_segment_ms: u64,
}

impl Default for CompanionVadSegmentationPolicy {
    fn default() -> Self {
        Self {
            speech_threshold: 0.55,
            silence_gap_ms: 450,
            min_segment_ms: 250,
            max_segment_ms: 15_000,
        }
    }
}

impl CompanionVadSegmentationPolicy {
    /// Segments sorted VAD frames into contiguous speech regions.
    #[must_use]
    pub fn segment(&self, frames: &[CompanionVadFrame]) -> Vec<CompanionSpeechSegment> {
        if frames.is_empty() {
            return Vec::new();
        }

        let mut sorted = frames.to_vec();
        sorted.sort_by_key(|frame| frame.at_ms);

        let mut segments = Vec::new();
        let mut active: Option<SegmentAccumulator> = None;

        for frame in sorted {
            let probability = frame.speech_probability.clamp(0.0, 1.0);
            let is_speech = probability >= self.speech_threshold;

            match (active.as_mut(), is_speech) {
                (None, true) => {
                    active = Some(SegmentAccumulator::new(frame.at_ms, probability));
                }
                (Some(current), true) => {
                    if frame.at_ms.saturating_sub(current.last_ms) > self.silence_gap_ms {
                        if let Some(done) = current.finalize(self.min_segment_ms) {
                            segments.push(done);
                        }
                        active = Some(SegmentAccumulator::new(frame.at_ms, probability));
                        continue;
                    }

                    current.push(frame.at_ms, probability);
                    if current.duration_ms() >= self.max_segment_ms {
                        if let Some(done) = current.finalize(self.min_segment_ms) {
                            segments.push(done);
                        }
                        active = None;
                    }
                }
                (Some(current), false) => {
                    if frame.at_ms.saturating_sub(current.last_ms) > self.silence_gap_ms {
                        if let Some(done) = current.finalize(self.min_segment_ms) {
                            segments.push(done);
                        }
                        active = None;
                    }
                }
                (None, false) => {}
            }
        }

        if let Some(current) = active
            && let Some(done) = current.finalize(self.min_segment_ms)
        {
            segments.push(done);
        }

        segments
    }
}

#[derive(Debug, Clone, Copy)]
struct SegmentAccumulator {
    start_ms: u64,
    last_ms: u64,
    total_probability: f64,
    frame_count: usize,
}

impl SegmentAccumulator {
    fn new(at_ms: u64, probability: f64) -> Self {
        Self {
            start_ms: at_ms,
            last_ms: at_ms,
            total_probability: probability,
            frame_count: 1,
        }
    }

    fn push(&mut self, at_ms: u64, probability: f64) {
        self.last_ms = at_ms;
        self.total_probability += probability;
        self.frame_count += 1;
    }

    fn duration_ms(&self) -> u64 {
        self.last_ms.saturating_sub(self.start_ms)
    }

    fn finalize(self, min_segment_ms: u64) -> Option<CompanionSpeechSegment> {
        if self.duration_ms() < min_segment_ms {
            return None;
        }

        let frame_count_u32 = u32::try_from(self.frame_count).unwrap_or(u32::MAX);
        Some(CompanionSpeechSegment {
            segment_id: Uuid::new_v4().to_string(),
            start_ms: self.start_ms,
            end_ms: self.last_ms,
            avg_speech_probability: self.total_probability / f64::from(frame_count_u32),
            frame_count: self.frame_count,
        })
    }
}

fn normalize_identifier(field: &str, raw: &str, allow_slash: bool) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    if !value.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(ch, '.' | '_' | '-' | ':')
            || (allow_slash && ch == '/')
    }) {
        anyhow::bail!("{field} contains invalid characters");
    }
    Ok(value.to_string())
}

fn normalize_tag(raw: &str) -> Result<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        anyhow::bail!("emotion tag must not be empty");
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        anyhow::bail!("emotion tag must use only [A-Za-z0-9._-]");
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::{
        CompanionEmotionalImpact, CompanionMediaKind, CompanionMultimodalMemoryRecord,
        CompanionVadFrame, CompanionVadSegmentationPolicy,
    };

    #[test]
    fn multimodal_record_builds_signal_envelope_with_emotion_metadata() {
        let record = CompanionMultimodalMemoryRecord::new(
            "user_1",
            "camera/frame_001",
            CompanionMediaKind::Photo,
            vec!["sunset".to_string(), "beach".to_string()],
        )
        .unwrap()
        .with_transcript("It looks peaceful.")
        .with_emotional_impact(CompanionEmotionalImpact {
            valence: 0.8,
            arousal: 0.4,
            confidence: crate::contracts::scores::Confidence::new(0.9),
            tags: vec!["joy".to_string(), "calm".to_string()],
        });

        let envelope = record.to_signal_envelope().unwrap();
        assert_eq!(
            envelope
                .metadata
                .get("companion_media_kind")
                .map(String::as_str),
            Some("photo")
        );
        assert_eq!(
            envelope
                .metadata
                .get("descriptor_count")
                .map(String::as_str),
            Some("2")
        );
        assert_eq!(
            envelope.metadata.get("emotion_tags").map(String::as_str),
            Some("joy|calm")
        );
    }

    #[test]
    fn emotional_impact_normalization_clamps_values() {
        let normalized = CompanionEmotionalImpact {
            valence: 4.2,
            arousal: -0.5,
            confidence: crate::contracts::scores::Confidence::new(3.0),
            tags: vec![" Joy ".to_string()],
        }
        .normalize()
        .unwrap();

        assert!((normalized.valence - 1.0).abs() < f64::EPSILON);
        assert!(normalized.arousal.abs() < f64::EPSILON);
        assert!((normalized.confidence.get() - 1.0).abs() < f64::EPSILON);
        assert_eq!(normalized.tags, vec!["joy".to_string()]);
    }

    #[test]
    fn vad_segmentation_splits_on_silence_gaps() {
        let policy = CompanionVadSegmentationPolicy {
            speech_threshold: 0.6,
            silence_gap_ms: 200,
            min_segment_ms: 100,
            max_segment_ms: 5_000,
        };
        let frames = vec![
            CompanionVadFrame {
                at_ms: 0,
                speech_probability: 0.8,
            },
            CompanionVadFrame {
                at_ms: 80,
                speech_probability: 0.9,
            },
            CompanionVadFrame {
                at_ms: 160,
                speech_probability: 0.75,
            },
            CompanionVadFrame {
                at_ms: 500,
                speech_probability: 0.85,
            },
            CompanionVadFrame {
                at_ms: 620,
                speech_probability: 0.7,
            },
        ];

        let segments = policy.segment(&frames);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].start_ms, 0);
        assert_eq!(segments[0].end_ms, 160);
        assert_eq!(segments[1].start_ms, 500);
        assert_eq!(segments[1].end_ms, 620);
    }

    #[test]
    fn vad_segmentation_discards_too_short_segments() {
        let policy = CompanionVadSegmentationPolicy {
            speech_threshold: 0.5,
            silence_gap_ms: 150,
            min_segment_ms: 120,
            max_segment_ms: 5_000,
        };
        let frames = vec![
            CompanionVadFrame {
                at_ms: 0,
                speech_probability: 0.9,
            },
            CompanionVadFrame {
                at_ms: 50,
                speech_probability: 0.9,
            },
            CompanionVadFrame {
                at_ms: 500,
                speech_probability: 0.95,
            },
            CompanionVadFrame {
                at_ms: 650,
                speech_probability: 0.95,
            },
        ];

        let segments = policy.segment(&frames);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms, 500);
        assert_eq!(segments[0].end_ms, 650);
    }
}
