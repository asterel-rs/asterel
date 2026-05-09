//! Channel-aware style profiles: adapts tone, length, and format
//! constraints so that the same persona feels natural across
//! different communication media.
//!
//! References: [ZEROSTYLUS] Wu & Deng, 2025 — hierarchical template
//!   acquisition for zero-shot style transfer across media.
//! See the public research reference index in the docs site.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTone {
    Casual,
    Natural,
    SemiFormal,
    Formal,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ChannelStyleProfile {
    pub tone: ChannelTone,
    pub max_sentences: u8,
    pub max_chars: usize,
    pub allow_emoji: bool,
    pub max_emoji: u8,
    pub prefer_paragraphs: bool,
    pub allow_markdown: bool,
    pub split_long_messages: bool,
}

impl Default for ChannelStyleProfile {
    fn default() -> Self {
        Self {
            tone: ChannelTone::Natural,
            max_sentences: 6,
            max_chars: 2_000,
            allow_emoji: true,
            max_emoji: 2,
            prefer_paragraphs: false,
            allow_markdown: true,
            split_long_messages: false,
        }
    }
}

#[must_use]
pub fn profile_for_channel(channel_name: &str) -> ChannelStyleProfile {
    match channel_name.to_ascii_lowercase().as_str() {
        "cli" => ChannelStyleProfile {
            tone: ChannelTone::Direct,
            max_sentences: 10,
            max_chars: 4_000,
            allow_emoji: false,
            max_emoji: 0,
            prefer_paragraphs: true,
            allow_markdown: true,
            split_long_messages: false,
        },
        "discord" => ChannelStyleProfile {
            tone: ChannelTone::Casual,
            max_sentences: 3,
            max_chars: 1_800,
            allow_emoji: true,
            max_emoji: 2,
            prefer_paragraphs: false,
            allow_markdown: true,
            split_long_messages: true,
        },
        "telegram" => ChannelStyleProfile {
            tone: ChannelTone::Natural,
            max_sentences: 4,
            max_chars: 2_000,
            allow_emoji: true,
            max_emoji: 1,
            prefer_paragraphs: false,
            allow_markdown: true,
            split_long_messages: true,
        },
        "slack" => ChannelStyleProfile {
            tone: ChannelTone::SemiFormal,
            max_sentences: 6,
            max_chars: 3_000,
            allow_emoji: true,
            max_emoji: 1,
            prefer_paragraphs: true,
            allow_markdown: true,
            split_long_messages: false,
        },
        "email" => ChannelStyleProfile {
            tone: ChannelTone::Formal,
            max_sentences: 12,
            max_chars: 5_000,
            allow_emoji: false,
            max_emoji: 0,
            prefer_paragraphs: true,
            allow_markdown: false,
            split_long_messages: false,
        },
        "matrix" => ChannelStyleProfile {
            tone: ChannelTone::Natural,
            max_sentences: 5,
            max_chars: 2_500,
            allow_emoji: true,
            max_emoji: 1,
            prefer_paragraphs: false,
            allow_markdown: true,
            split_long_messages: false,
        },
        "irc" => ChannelStyleProfile {
            tone: ChannelTone::Direct,
            max_sentences: 3,
            max_chars: 400,
            allow_emoji: false,
            max_emoji: 0,
            prefer_paragraphs: false,
            allow_markdown: false,
            split_long_messages: true,
        },
        "imessage" => ChannelStyleProfile {
            tone: ChannelTone::Casual,
            max_sentences: 3,
            max_chars: 1_200,
            allow_emoji: true,
            max_emoji: 2,
            prefer_paragraphs: false,
            allow_markdown: false,
            split_long_messages: true,
        },
        "whatsapp" => ChannelStyleProfile {
            tone: ChannelTone::Casual,
            max_sentences: 4,
            max_chars: 1_500,
            allow_emoji: true,
            max_emoji: 2,
            prefer_paragraphs: false,
            allow_markdown: true,
            split_long_messages: true,
        },
        _ => ChannelStyleProfile::default(),
    }
}

#[must_use]
pub fn render_channel_style_block(profile: &ChannelStyleProfile) -> String {
    use std::fmt::Write;

    let mut block = String::from("[Channel Style]\n");
    let _ = writeln!(block, "Tone: {:?}", profile.tone);
    let _ = writeln!(block, "Length: max {} sentences", profile.max_sentences);

    if profile.allow_emoji {
        let _ = writeln!(block, "Emoji: max {}", profile.max_emoji);
    } else {
        block.push_str("Emoji: none\n");
    }

    if profile.prefer_paragraphs {
        block.push_str("Format: structured paragraphs\n");
    } else {
        block.push_str("Format: short messages\n");
    }

    if !profile.allow_markdown {
        block.push_str("Markdown: disabled\n");
    }

    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_profile_disables_emoji() {
        let profile = profile_for_channel("cli");
        assert!(!profile.allow_emoji);
        assert_eq!(profile.max_emoji, 0);
        assert_eq!(profile.tone, ChannelTone::Direct);
    }

    #[test]
    fn discord_profile_is_casual() {
        let profile = profile_for_channel("discord");
        assert_eq!(profile.tone, ChannelTone::Casual);
        assert!(profile.split_long_messages);
        assert!(profile.max_sentences <= 4);
    }

    #[test]
    fn email_profile_is_formal() {
        let profile = profile_for_channel("email");
        assert_eq!(profile.tone, ChannelTone::Formal);
        assert!(!profile.allow_emoji);
        assert!(!profile.allow_markdown);
        assert!(profile.prefer_paragraphs);
    }

    #[test]
    fn irc_profile_is_terse() {
        let profile = profile_for_channel("irc");
        assert!(profile.max_chars <= 500);
        assert!(!profile.allow_emoji);
        assert!(!profile.allow_markdown);
    }

    #[test]
    fn unknown_channel_gets_default() {
        let profile = profile_for_channel("unknown-platform");
        let default = ChannelStyleProfile::default();
        assert_eq!(profile.tone, default.tone);
        assert_eq!(profile.max_sentences, default.max_sentences);
    }

    #[test]
    fn case_insensitive_lookup() {
        let upper = profile_for_channel("TELEGRAM");
        let lower = profile_for_channel("telegram");
        assert_eq!(upper.tone, lower.tone);
        assert_eq!(upper.max_sentences, lower.max_sentences);
    }

    #[test]
    fn render_block_includes_tone_and_length() {
        let profile = profile_for_channel("discord");
        let block = render_channel_style_block(&profile);
        assert!(block.contains("[Channel Style]"));
        assert!(block.contains("Casual"));
        assert!(block.contains("max 3 sentences"));
    }

    #[test]
    fn render_block_shows_no_emoji_for_cli() {
        let profile = profile_for_channel("cli");
        let block = render_channel_style_block(&profile);
        assert!(block.contains("Emoji: none"));
    }
}
