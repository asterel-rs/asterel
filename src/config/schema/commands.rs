//! Runtime command authorization configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

fn default_allow_from() -> Vec<String> {
    vec!["*".to_string()]
}

/// Authorization controls for in-band slash commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsConfig {
    /// Global sender allowlist for commands.
    ///
    /// - `"*"` allows every sender.
    /// - Empty list denies all senders.
    ///
    /// Default: `["*"]`.
    #[serde(default = "default_allow_from")]
    pub allow_from: Vec<String>,
    /// Optional per-channel command sender allowlists.
    ///
    /// When a channel key exists here, it overrides `allow_from` for that
    /// channel.
    #[serde(default)]
    pub by_channel: BTreeMap<String, Vec<String>>,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            allow_from: default_allow_from(),
            by_channel: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandsConfig;

    #[test]
    fn defaults_allow_commands_from_any_sender() {
        let cfg = CommandsConfig::default();
        assert_eq!(cfg.allow_from, vec!["*".to_string()]);
        assert!(cfg.by_channel.is_empty());
    }

    #[test]
    fn per_channel_allowlist_round_trip() {
        let toml = r#"
allow_from = ["ops"]

[by_channel]
discord = ["123"]
telegram = ["alice"]
"#;
        let cfg: CommandsConfig = toml::from_str(toml).expect("parse commands config");
        assert_eq!(cfg.allow_from, vec!["ops".to_string()]);
        assert_eq!(
            cfg.by_channel.get("discord"),
            Some(&vec!["123".to_string()])
        );
        assert_eq!(
            cfg.by_channel.get("telegram"),
            Some(&vec!["alice".to_string()])
        );
    }
}
