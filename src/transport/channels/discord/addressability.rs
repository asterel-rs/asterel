//! Message addressability classification for Discord.
//!
//! Determines whether the bot should respond to a message based on
//! channel context (DM, mention, thread continuation, passive).

use crate::config::DiscordPickupMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressabilityMode {
    Direct,
    Continuation,
    AmbientCandidate,
    Passive,
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct AddressabilityContext {
    pub is_dm: bool,
    pub mentions_bot: bool,
    pub is_thread_with_bot: bool,
    pub is_reply_to_bot: bool,
}

impl AddressabilityContext {
    #[must_use]
    pub fn classify(&self) -> AddressabilityMode {
        self.classify_with_pickup_policy(DiscordPickupMode::DirectOnly)
    }

    #[must_use]
    pub fn classify_with_pickup_policy(
        &self,
        pickup_mode: DiscordPickupMode,
    ) -> AddressabilityMode {
        if self.is_dm || self.mentions_bot || self.is_reply_to_bot {
            return AddressabilityMode::Direct;
        }
        if self.is_thread_with_bot {
            return AddressabilityMode::Continuation;
        }
        if pickup_mode == DiscordPickupMode::SparseAmbient {
            return AddressabilityMode::AmbientCandidate;
        }
        AddressabilityMode::Passive
    }
}

#[must_use]
pub fn detect_bot_mention(content: &str, bot_user_id: Option<&str>) -> bool {
    let Some(bot_id) = bot_user_id else {
        return false;
    };
    let mention_plain = format!("<@{bot_id}>");
    let mention_nick = format!("<@!{bot_id}>");
    content.contains(&mention_plain) || content.contains(&mention_nick)
}

#[must_use]
pub fn strip_bot_mention(content: &str, bot_user_id: Option<&str>) -> String {
    let Some(bot_id) = bot_user_id else {
        return content.to_string();
    };
    let mention_plain = format!("<@{bot_id}>");
    let mention_nick = format!("<@!{bot_id}>");
    content
        .replace(&mention_plain, "")
        .replace(&mention_nick, "")
        .trim()
        .to_string()
}

#[must_use]
pub fn looks_like_ambient_pickup_candidate(content: &str) -> bool {
    let normalized = content.trim().to_lowercase();
    if normalized.len() < 8 {
        return false;
    }

    if normalized.starts_with("help ") || normalized.contains("help me") {
        return true;
    }

    let asks_room_for_help = normalized.contains("anyone know")
        || normalized.contains("does anyone")
        || normalized.contains("can anyone")
        || normalized.contains("can someone")
        || normalized.contains("someone know");
    let has_problem_signal = [
        "broken", "bug", "debug", "error", "explain", "fail", "fix", "issue", "problem", "stuck",
        "trace", "why",
    ]
    .iter()
    .any(|signal| normalized.contains(signal));

    if asks_room_for_help && has_problem_signal {
        return true;
    }

    let starts_as_problem_question = ["how ", "why ", "what "]
        .iter()
        .any(|prefix| normalized.starts_with(prefix));
    starts_as_problem_question && has_problem_signal
}

#[must_use]
pub fn channel_context_hint(mode: AddressabilityMode, is_dm: bool) -> Option<&'static str> {
    match (mode, is_dm) {
        (AddressabilityMode::Direct, true) => {
            Some("[Channel Context: DM — conversational tone, natural length]")
        }
        (AddressabilityMode::Direct, false) => {
            Some("[Channel Context: Direct mention — concise, relevant to channel topic]")
        }
        (AddressabilityMode::Continuation, _) => {
            Some("[Channel Context: Thread continuation — stay on topic, build on prior context]")
        }
        (AddressabilityMode::AmbientCandidate, _) => {
            Some("[Channel Context: Ambient pickup — brief, useful, and easy to ignore]")
        }
        (AddressabilityMode::Passive, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dm_is_always_direct() {
        let ctx = AddressabilityContext {
            is_dm: true,
            mentions_bot: false,
            is_thread_with_bot: false,
            is_reply_to_bot: false,
        };
        assert_eq!(ctx.classify(), AddressabilityMode::Direct);
    }

    #[test]
    fn mention_is_direct() {
        let ctx = AddressabilityContext {
            is_dm: false,
            mentions_bot: true,
            is_thread_with_bot: false,
            is_reply_to_bot: false,
        };
        assert_eq!(ctx.classify(), AddressabilityMode::Direct);
    }

    #[test]
    fn reply_to_bot_is_direct() {
        let ctx = AddressabilityContext {
            is_dm: false,
            mentions_bot: false,
            is_thread_with_bot: false,
            is_reply_to_bot: true,
        };
        assert_eq!(ctx.classify(), AddressabilityMode::Direct);
    }

    #[test]
    fn thread_with_bot_is_continuation() {
        let ctx = AddressabilityContext {
            is_dm: false,
            mentions_bot: false,
            is_thread_with_bot: true,
            is_reply_to_bot: false,
        };
        assert_eq!(ctx.classify(), AddressabilityMode::Continuation);
    }

    #[test]
    fn guild_message_no_mention_is_passive() {
        let ctx = AddressabilityContext {
            is_dm: false,
            mentions_bot: false,
            is_thread_with_bot: false,
            is_reply_to_bot: false,
        };
        assert_eq!(ctx.classify(), AddressabilityMode::Passive);
    }

    #[test]
    fn guild_message_can_become_ambient_candidate_in_sparse_mode() {
        let ctx = AddressabilityContext {
            is_dm: false,
            mentions_bot: false,
            is_thread_with_bot: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            ctx.classify_with_pickup_policy(DiscordPickupMode::SparseAmbient),
            AddressabilityMode::AmbientCandidate
        );
    }

    #[test]
    fn ambient_pickup_heuristic_prefers_question_like_messages() {
        assert!(looks_like_ambient_pickup_candidate(
            "anyone know why this broke?"
        ));
        assert!(looks_like_ambient_pickup_candidate("how do I fix this"));
        assert!(looks_like_ambient_pickup_candidate(
            "can someone help me untangle this error?"
        ));
        assert!(!looks_like_ambient_pickup_candidate("lol"));
    }

    #[test]
    fn ambient_pickup_heuristic_rejects_room_chatter() {
        assert!(!looks_like_ambient_pickup_candidate(
            "what are we ordering for lunch?"
        ));
        assert!(!looks_like_ambient_pickup_candidate(
            "anyone up for a game later?"
        ));
        assert!(!looks_like_ambient_pickup_candidate("this is fine, right?"));
    }

    #[test]
    fn detect_plain_mention() {
        assert!(detect_bot_mention("hey <@123456> help", Some("123456")));
    }

    #[test]
    fn detect_nick_mention() {
        assert!(detect_bot_mention("hey <@!123456>", Some("123456")));
    }

    #[test]
    fn no_mention_no_match() {
        assert!(!detect_bot_mention("hello world", Some("123456")));
    }

    #[test]
    fn no_bot_id_no_match() {
        assert!(!detect_bot_mention("hey <@123456>", None));
    }

    #[test]
    fn strip_removes_mention_and_trims() {
        assert_eq!(
            strip_bot_mention("<@123> hello there", Some("123")),
            "hello there"
        );
    }

    #[test]
    fn strip_nick_mention() {
        assert_eq!(strip_bot_mention("<@!123> help me", Some("123")), "help me");
    }

    #[test]
    fn context_hint_dm_returns_conversational() {
        let hint = channel_context_hint(AddressabilityMode::Direct, true);
        assert!(hint.unwrap().contains("DM"));
    }

    #[test]
    fn context_hint_mention_returns_concise() {
        let hint = channel_context_hint(AddressabilityMode::Direct, false);
        assert!(hint.unwrap().contains("concise"));
    }

    #[test]
    fn context_hint_continuation_returns_thread() {
        let hint = channel_context_hint(AddressabilityMode::Continuation, false);
        assert!(hint.unwrap().contains("Thread"));
    }

    #[test]
    fn context_hint_passive_returns_none() {
        assert!(channel_context_hint(AddressabilityMode::Passive, false).is_none());
    }
}
