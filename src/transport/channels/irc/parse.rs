//! Lightweight IRC protocol parser: extracts prefix, command, and params
//! from raw IRC lines per RFC 1459.
/// A parsed IRC message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IrcMessage {
    pub(super) prefix: Option<String>,
    pub(super) command: String,
    pub(super) params: Vec<String>,
}

impl IrcMessage {
    /// Parse a raw IRC line into an `IrcMessage`.
    ///
    /// IRC format: `[:<prefix>] <command> [<params>] [:<trailing>]`
    pub(super) fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return None;
        }

        let (prefix, rest) = if let Some(stripped) = line.strip_prefix(':') {
            let space = stripped.find(' ')?;
            (Some(stripped[..space].to_string()), &stripped[space + 1..])
        } else {
            (None, line)
        };

        // Split at trailing (first `:` after command/params)
        let (params_part, trailing) = if let Some(colon_pos) = rest.find(" :") {
            (&rest[..colon_pos], Some(&rest[colon_pos + 2..]))
        } else {
            (rest, None)
        };

        let mut parts: Vec<&str> = params_part.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let command = parts.remove(0).to_uppercase();
        let mut params: Vec<String> = parts.iter().map(std::string::ToString::to_string).collect();
        if let Some(t) = trailing {
            params.push(t.to_string());
        }

        Some(IrcMessage {
            prefix,
            command,
            params,
        })
    }

    /// Extract the nickname from the prefix (nick!user@host → nick).
    pub(super) fn nick(&self) -> Option<&str> {
        self.prefix.as_ref().and_then(|p| {
            let end = p.find('!').unwrap_or(p.len());
            let nick = &p[..end];
            if nick.is_empty() { None } else { Some(nick) }
        })
    }
}
