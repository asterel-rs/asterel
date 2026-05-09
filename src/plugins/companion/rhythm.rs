//! Attention rhythm gate for the companion plugin.
//!
//! Manages cooldown and burst limits to prevent excessive companion
//! activations, with urgent-bypass for high-priority signals.

use std::collections::VecDeque;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Signal type that triggers the companion attention gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionAttentionSignal {
    /// User mentioned the companion by name.
    Mention,
    /// Direct message to the companion.
    DirectMessage,
    /// Wake word detected in audio.
    WakeWord,
    /// High-priority urgent signal (bypasses cooldown).
    Urgent,
}

/// Reason for the attention gate decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionAttentionDecisionReason {
    /// Signal was accepted normally.
    Accepted,
    /// Signal was rejected due to minimum interval cooldown.
    Cooldown,
    /// Signal was rejected due to burst rate limit.
    BurstLimited,
    /// Signal was accepted via urgent bypass.
    UrgentBypass,
}

/// Result of an attention gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionAttentionDecision {
    /// Whether the signal was accepted.
    pub accepted: bool,
    /// Reason for the decision.
    pub reason: CompanionAttentionDecisionReason,
    /// Milliseconds to wait before retrying, if rejected.
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
}

/// Policy controlling attention gate cooldown and burst limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionAttentionPolicy {
    /// Minimum milliseconds between accepted signals.
    pub min_interval_ms: u64,
    /// Maximum signals allowed within the burst window.
    pub burst_limit: usize,
    /// Burst window duration in seconds.
    pub burst_window_secs: u64,
    /// Whether urgent signals bypass all rate limits.
    pub urgent_bypass: bool,
}

impl Default for CompanionAttentionPolicy {
    fn default() -> Self {
        Self {
            min_interval_ms: 1_200,
            burst_limit: 4,
            burst_window_secs: 10,
            urgent_bypass: true,
        }
    }
}

/// Stateful controller that enforces attention rate limits.
#[derive(Debug)]
pub struct CompanionAttentionController {
    policy: CompanionAttentionPolicy,
    last_grant_at: Option<DateTime<Utc>>,
    burst_grants: VecDeque<DateTime<Utc>>,
}

impl CompanionAttentionController {
    /// Creates a new controller with the given attention policy.
    #[must_use]
    pub fn new(policy: CompanionAttentionPolicy) -> Self {
        Self {
            policy: CompanionAttentionPolicy {
                min_interval_ms: policy.min_interval_ms.max(1),
                burst_limit: policy.burst_limit.max(1),
                burst_window_secs: policy.burst_window_secs.max(1),
                urgent_bypass: policy.urgent_bypass,
            },
            last_grant_at: None,
            burst_grants: VecDeque::new(),
        }
    }

    /// Evaluates whether the signal should be accepted or throttled.
    #[must_use]
    pub fn evaluate(
        &mut self,
        now: DateTime<Utc>,
        signal: CompanionAttentionSignal,
    ) -> CompanionAttentionDecision {
        self.prune_burst_window(now);

        if signal == CompanionAttentionSignal::Urgent && self.policy.urgent_bypass {
            self.record_grant(now);
            return CompanionAttentionDecision {
                accepted: true,
                reason: CompanionAttentionDecisionReason::UrgentBypass,
                retry_after_ms: None,
            };
        }

        let min_interval = duration_from_millis(self.policy.min_interval_ms);
        if let Some(last_grant_at) = self.last_grant_at {
            let elapsed = now.signed_duration_since(last_grant_at);
            if elapsed < min_interval {
                let remaining =
                    non_negative_i64_to_u64((min_interval - elapsed).num_milliseconds());
                return CompanionAttentionDecision {
                    accepted: false,
                    reason: CompanionAttentionDecisionReason::Cooldown,
                    retry_after_ms: Some(remaining),
                };
            }
        }

        if self.burst_grants.len() >= self.policy.burst_limit {
            let burst_window = duration_from_secs(self.policy.burst_window_secs);
            if let Some(oldest_grant_at) = self.burst_grants.front() {
                let elapsed = now.signed_duration_since(*oldest_grant_at);
                let remaining =
                    non_negative_i64_to_u64((burst_window - elapsed).num_milliseconds());
                return CompanionAttentionDecision {
                    accepted: false,
                    reason: CompanionAttentionDecisionReason::BurstLimited,
                    retry_after_ms: Some(remaining),
                };
            }
        }

        self.record_grant(now);
        CompanionAttentionDecision {
            accepted: true,
            reason: CompanionAttentionDecisionReason::Accepted,
            retry_after_ms: None,
        }
    }

    fn prune_burst_window(&mut self, now: DateTime<Utc>) {
        let burst_window = duration_from_secs(self.policy.burst_window_secs);
        while let Some(oldest) = self.burst_grants.front() {
            if now.signed_duration_since(*oldest) > burst_window {
                self.burst_grants.pop_front();
            } else {
                break;
            }
        }
    }

    fn record_grant(&mut self, now: DateTime<Utc>) {
        self.last_grant_at = Some(now);
        self.burst_grants.push_back(now);
    }
}

impl Default for CompanionAttentionController {
    fn default() -> Self {
        Self::new(CompanionAttentionPolicy::default())
    }
}

fn duration_from_millis(milliseconds: u64) -> Duration {
    Duration::milliseconds(i64::try_from(milliseconds).unwrap_or(i64::MAX))
}

fn duration_from_secs(seconds: u64) -> Duration {
    Duration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX))
}

fn non_negative_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(u64::MAX)
}

/// Source of a companion interruption request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionInterruptionSource {
    /// User spoke while the companion was responding.
    UserUtterance,
    /// Operator-level override (always allowed).
    OperatorOverride,
    /// Safety system alert (always allowed).
    SafetyAlert,
}

/// Reason for the interruption gate decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionInterruptionDecisionReason {
    /// Interruption was allowed normally.
    Allowed,
    /// Rejected because cooldown period is active.
    CooldownActive,
    /// Rejected because the response is too recent.
    ResponseTooYoung,
    /// Rejected because the turn interruption limit was reached.
    TurnLimitReached,
    /// Allowed via operator/safety priority bypass.
    PriorityBypass,
}

/// Result of an interruption gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionInterruptionDecision {
    /// Whether the interruption was allowed.
    pub allowed: bool,
    /// Reason for the decision.
    pub reason: CompanionInterruptionDecisionReason,
    /// Milliseconds to wait before retrying, if rejected.
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
}

/// Policy controlling interruption gate thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionInterruptionPolicy {
    /// Minimum response age in ms before allowing interruption.
    pub min_response_age_ms: u64,
    /// Cooldown period in ms between user interruptions.
    pub cooldown_ms: u64,
    /// Maximum user interruptions allowed per conversation turn.
    pub max_user_interruptions_per_turn: u32,
}

impl Default for CompanionInterruptionPolicy {
    fn default() -> Self {
        Self {
            min_response_age_ms: 1_000,
            cooldown_ms: 2_000,
            max_user_interruptions_per_turn: 2,
        }
    }
}

/// Stateful controller enforcing per-turn interruption limits.
#[derive(Debug)]
pub struct CompanionInterruptionController {
    policy: CompanionInterruptionPolicy,
    current_turn_id: Option<String>,
    user_interruptions_this_turn: u32,
    last_interruption_at: Option<DateTime<Utc>>,
}

impl CompanionInterruptionController {
    /// Creates a new controller with the given interruption policy.
    #[must_use]
    pub fn new(policy: CompanionInterruptionPolicy) -> Self {
        Self {
            policy: CompanionInterruptionPolicy {
                min_response_age_ms: policy.min_response_age_ms.max(1),
                cooldown_ms: policy.cooldown_ms.max(1),
                max_user_interruptions_per_turn: policy.max_user_interruptions_per_turn.max(1),
            },
            current_turn_id: None,
            user_interruptions_this_turn: 0,
            last_interruption_at: None,
        }
    }

    /// Resets per-turn counters when a new conversation turn begins.
    pub fn begin_turn(&mut self, turn_id: impl Into<String>) {
        let turn_id = turn_id.into();
        if self.current_turn_id.as_ref() == Some(&turn_id) {
            return;
        }

        self.current_turn_id = Some(turn_id);
        self.user_interruptions_this_turn = 0;
        self.last_interruption_at = None;
    }

    /// Evaluates whether the interruption should be allowed.
    #[must_use]
    pub fn evaluate(
        &mut self,
        now: DateTime<Utc>,
        source: CompanionInterruptionSource,
        response_started_at: Option<DateTime<Utc>>,
    ) -> CompanionInterruptionDecision {
        if matches!(
            source,
            CompanionInterruptionSource::OperatorOverride
                | CompanionInterruptionSource::SafetyAlert
        ) {
            self.last_interruption_at = Some(now);
            return CompanionInterruptionDecision {
                allowed: true,
                reason: CompanionInterruptionDecisionReason::PriorityBypass,
                retry_after_ms: None,
            };
        }

        if self.user_interruptions_this_turn >= self.policy.max_user_interruptions_per_turn {
            return CompanionInterruptionDecision {
                allowed: false,
                reason: CompanionInterruptionDecisionReason::TurnLimitReached,
                retry_after_ms: None,
            };
        }

        let min_age = duration_from_millis(self.policy.min_response_age_ms);
        let Some(response_started_at) = response_started_at else {
            return CompanionInterruptionDecision {
                allowed: false,
                reason: CompanionInterruptionDecisionReason::ResponseTooYoung,
                retry_after_ms: Some(self.policy.min_response_age_ms),
            };
        };

        let response_age = now.signed_duration_since(response_started_at);
        if response_age < min_age {
            let remaining = non_negative_i64_to_u64((min_age - response_age).num_milliseconds());
            return CompanionInterruptionDecision {
                allowed: false,
                reason: CompanionInterruptionDecisionReason::ResponseTooYoung,
                retry_after_ms: Some(remaining),
            };
        }

        let cooldown = duration_from_millis(self.policy.cooldown_ms);
        if let Some(last_interruption_at) = self.last_interruption_at {
            let elapsed = now.signed_duration_since(last_interruption_at);
            if elapsed < cooldown {
                let remaining = non_negative_i64_to_u64((cooldown - elapsed).num_milliseconds());
                return CompanionInterruptionDecision {
                    allowed: false,
                    reason: CompanionInterruptionDecisionReason::CooldownActive,
                    retry_after_ms: Some(remaining),
                };
            }
        }

        self.user_interruptions_this_turn += 1;
        self.last_interruption_at = Some(now);
        CompanionInterruptionDecision {
            allowed: true,
            reason: CompanionInterruptionDecisionReason::Allowed,
            retry_after_ms: None,
        }
    }
}

impl Default for CompanionInterruptionController {
    fn default() -> Self {
        Self::new(CompanionInterruptionPolicy::default())
    }
}

/// Policy controlling message chunking and typing delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionMessageRhythmPolicy {
    /// Maximum characters per message chunk.
    pub max_chars_per_chunk: usize,
    /// Minimum characters per chunk (avoids tiny tail chunks).
    pub min_chars_per_chunk: usize,
    /// Base typing delay in milliseconds before each chunk.
    pub base_typing_delay_ms: u64,
    /// Additional delay per character in the chunk.
    pub per_char_delay_ms: u64,
    /// Maximum typing delay cap in milliseconds.
    pub max_typing_delay_ms: u64,
}

impl Default for CompanionMessageRhythmPolicy {
    fn default() -> Self {
        Self {
            max_chars_per_chunk: 160,
            min_chars_per_chunk: 24,
            base_typing_delay_ms: 180,
            per_char_delay_ms: 18,
            max_typing_delay_ms: 1_800,
        }
    }
}

/// A single chunk produced by the message rhythm planner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionMessageChunk {
    /// 1-based sequence number of this chunk.
    pub sequence: u32,
    /// Text content of this chunk.
    pub text: String,
    /// Suggested typing delay before sending this chunk.
    pub typing_delay_ms: u64,
}

impl CompanionMessageRhythmPolicy {
    /// Splits text into chunks with computed typing delays.
    #[must_use]
    pub fn plan_chunks(&self, text: &str) -> Vec<CompanionMessageChunk> {
        let mut normalized = String::with_capacity(text.len());
        for segment in text.split_whitespace() {
            if !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push_str(segment);
        }
        if normalized.is_empty() {
            return Vec::new();
        }

        let max_chars = self.max_chars_per_chunk.max(1);
        let min_chars = self.min_chars_per_chunk.min(max_chars).max(1);
        let mut chunks = split_text_with_boundaries(&normalized, max_chars);
        if chunks.len() > 1 {
            // Byte count >= char count; if bytes < min_chars, chars < min_chars guaranteed.
            let last_chunk_chars = chunks.last().map_or(0, |chunk| {
                if chunk.len() < min_chars {
                    0
                } else {
                    chunk.chars().count()
                }
            });
            if last_chunk_chars < min_chars
                && let Some(last_chunk) = chunks.pop()
                && let Some(previous) = chunks.last_mut()
            {
                previous.push(' ');
                previous.push_str(last_chunk.trim());
            }
        }

        chunks
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| CompanionMessageChunk {
                sequence: u32::try_from(index + 1).unwrap_or(u32::MAX),
                typing_delay_ms: self.typing_delay_for_chunk(&chunk),
                text: chunk,
            })
            .collect()
    }

    fn typing_delay_for_chunk(&self, chunk: &str) -> u64 {
        let chunk_len = u64::try_from(chunk.chars().count()).unwrap_or(u64::MAX);
        let scaled = chunk_len.saturating_mul(self.per_char_delay_ms);
        self.base_typing_delay_ms
            .saturating_add(scaled)
            .min(self.max_typing_delay_ms)
    }
}

fn split_text_with_boundaries(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut remaining = text.trim();

    while !remaining.is_empty() {
        // Byte count >= char count; if bytes fit, chars fit too.
        if remaining.len() <= max_chars || remaining.chars().count() <= max_chars {
            chunks.push(remaining.to_string());
            break;
        }

        let mut char_count = 0usize;
        let mut hard_cut = 0usize;
        let mut boundary_cut = None;
        for (index, ch) in remaining.char_indices() {
            char_count += 1;
            if char_count > max_chars {
                break;
            }

            hard_cut = index + ch.len_utf8();
            if ch.is_whitespace()
                || matches!(
                    ch,
                    '.' | '!' | '?' | ',' | ';' | ':' | '。' | '！' | '？' | '、'
                )
            {
                boundary_cut = Some(hard_cut);
            }
        }

        let cut = boundary_cut.unwrap_or(hard_cut).max(1);
        let head = remaining[..cut].trim();
        if head.is_empty() {
            break;
        }

        chunks.push(head.to_string());
        remaining = remaining[cut..].trim_start();
    }

    chunks
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::{
        CompanionAttentionController, CompanionAttentionDecisionReason, CompanionAttentionPolicy,
        CompanionAttentionSignal, CompanionInterruptionController,
        CompanionInterruptionDecisionReason, CompanionInterruptionPolicy,
        CompanionInterruptionSource, CompanionMessageRhythmPolicy,
    };

    #[test]
    fn attention_enforces_cooldown_and_burst_limit() {
        let now = Utc::now();
        let mut controller = CompanionAttentionController::new(CompanionAttentionPolicy {
            min_interval_ms: 1_000,
            burst_limit: 2,
            burst_window_secs: 10,
            urgent_bypass: true,
        });

        let first = controller.evaluate(now, CompanionAttentionSignal::Mention);
        assert!(first.accepted);

        let cooldown = controller.evaluate(
            now + Duration::milliseconds(500),
            CompanionAttentionSignal::Mention,
        );
        assert!(!cooldown.accepted);
        assert_eq!(cooldown.reason, CompanionAttentionDecisionReason::Cooldown);

        let second = controller.evaluate(
            now + Duration::milliseconds(1_200),
            CompanionAttentionSignal::Mention,
        );
        assert!(second.accepted);

        let burst_limited = controller.evaluate(
            now + Duration::milliseconds(2_500),
            CompanionAttentionSignal::WakeWord,
        );
        assert!(!burst_limited.accepted);
        assert_eq!(
            burst_limited.reason,
            CompanionAttentionDecisionReason::BurstLimited
        );
    }

    #[test]
    fn attention_allows_urgent_bypass() {
        let now = Utc::now();
        let mut controller = CompanionAttentionController::new(CompanionAttentionPolicy {
            min_interval_ms: 10_000,
            burst_limit: 1,
            burst_window_secs: 60,
            urgent_bypass: true,
        });

        assert!(
            controller
                .evaluate(now, CompanionAttentionSignal::Mention)
                .accepted
        );
        let urgent = controller.evaluate(
            now + Duration::milliseconds(10),
            CompanionAttentionSignal::Urgent,
        );
        assert!(urgent.accepted);
        assert_eq!(
            urgent.reason,
            CompanionAttentionDecisionReason::UrgentBypass
        );
    }

    #[test]
    fn interruption_enforces_response_age_cooldown_and_turn_limit() {
        let now = Utc::now();
        let response_started_at = now;
        let mut controller = CompanionInterruptionController::new(CompanionInterruptionPolicy {
            min_response_age_ms: 1_000,
            cooldown_ms: 2_000,
            max_user_interruptions_per_turn: 2,
        });
        controller.begin_turn("turn-1");

        let too_early = controller.evaluate(
            now + Duration::milliseconds(300),
            CompanionInterruptionSource::UserUtterance,
            Some(response_started_at),
        );
        assert!(!too_early.allowed);
        assert_eq!(
            too_early.reason,
            CompanionInterruptionDecisionReason::ResponseTooYoung
        );

        let first = controller.evaluate(
            now + Duration::milliseconds(1_200),
            CompanionInterruptionSource::UserUtterance,
            Some(response_started_at),
        );
        assert!(first.allowed);

        let cooldown = controller.evaluate(
            now + Duration::milliseconds(2_200),
            CompanionInterruptionSource::UserUtterance,
            Some(response_started_at),
        );
        assert!(!cooldown.allowed);
        assert_eq!(
            cooldown.reason,
            CompanionInterruptionDecisionReason::CooldownActive
        );

        let second = controller.evaluate(
            now + Duration::milliseconds(3_500),
            CompanionInterruptionSource::UserUtterance,
            Some(response_started_at),
        );
        assert!(second.allowed);

        let limit = controller.evaluate(
            now + Duration::milliseconds(6_000),
            CompanionInterruptionSource::UserUtterance,
            Some(response_started_at),
        );
        assert!(!limit.allowed);
        assert_eq!(
            limit.reason,
            CompanionInterruptionDecisionReason::TurnLimitReached
        );
    }

    #[test]
    fn interruption_allows_priority_sources() {
        let now = Utc::now();
        let mut controller = CompanionInterruptionController::default();
        controller.begin_turn("turn-2");

        let operator =
            controller.evaluate(now, CompanionInterruptionSource::OperatorOverride, None);
        assert!(operator.allowed);
        assert_eq!(
            operator.reason,
            CompanionInterruptionDecisionReason::PriorityBypass
        );

        let safety = controller.evaluate(
            now + Duration::milliseconds(10),
            CompanionInterruptionSource::SafetyAlert,
            None,
        );
        assert!(safety.allowed);
        assert_eq!(
            safety.reason,
            CompanionInterruptionDecisionReason::PriorityBypass
        );
    }

    #[test]
    fn message_rhythm_splits_text_and_caps_typing_delay() {
        let policy = CompanionMessageRhythmPolicy {
            max_chars_per_chunk: 48,
            min_chars_per_chunk: 16,
            base_typing_delay_ms: 200,
            per_char_delay_ms: 30,
            max_typing_delay_ms: 900,
        };
        let text = "This is a long answer that should be split into multiple chunks. \
            Each chunk should keep meaningful boundaries, and typing delay must stay capped.";

        let chunks = policy.plan_chunks(text);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].sequence, 1);
        assert!(chunks.iter().all(|chunk| !chunk.text.trim().is_empty()));
        assert!(chunks.iter().all(|chunk| chunk.typing_delay_ms <= 900));
    }
}
