//! IRC message formatting: style prefix for plain-text-only output and
//! line splitting that respects the IRC protocol's byte-length limits.
/// Style instruction prepended to every IRC message before it reaches the LLM.
/// IRC clients render plain text only — no markdown, no HTML, no XML.
pub(super) const IRC_STYLE_PREFIX: &str = "\
[context: you are responding over IRC. \
Plain text only. No markdown, no tables, no XML/HTML tags. \
Never use triple backtick code fences. Use a single blank line to separate blocks instead. \
Be terse and concise. \
Use short lines. Avoid walls of text.]\n";

/// Reserved bytes for the server-prepended sender prefix (`:nick!user@host `).
pub(super) const SENDER_PREFIX_RESERVE: usize = 64;

/// Split a message into lines safe for IRC transmission.
///
/// IRC is a line-based protocol — `\r\n` terminates each command, so any
/// newline inside a PRIVMSG payload would truncate the message and turn the
/// remainder into garbled/invalid IRC commands.
///
/// This function:
/// 1. Splits on `\n` (and strips `\r`) so each logical line becomes its own PRIVMSG.
/// 2. Splits any line that exceeds `max_bytes` at a safe UTF-8 boundary.
/// 3. Skips empty lines to avoid sending blank PRIVMSGs.
pub(super) fn split_message(message: &str, max_bytes: usize) -> Vec<String> {
    let mut chunks = Vec::new();

    // Guard against max_bytes == 0 to prevent infinite loop
    if max_bytes == 0 {
        let mut full = String::new();
        for l in message
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| !l.is_empty())
        {
            if !full.is_empty() {
                full.push(' ');
            }
            full.push_str(l);
        }
        if full.is_empty() {
            chunks.push(String::new());
        } else {
            chunks.push(full);
        }
        return chunks;
    }

    for line in message.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }

        if line.len() <= max_bytes {
            chunks.push(line.to_string());
            continue;
        }

        // Line exceeds max_bytes — split at safe UTF-8 boundaries
        let mut remaining = line;
        while !remaining.is_empty() {
            if remaining.len() <= max_bytes {
                chunks.push(remaining.to_string());
                break;
            }

            let mut split_at = max_bytes;
            while split_at > 0 && !remaining.is_char_boundary(split_at) {
                split_at -= 1;
            }
            if split_at == 0 {
                // No valid boundary found going backward — advance forward instead
                split_at = max_bytes;
                while split_at < remaining.len() && !remaining.is_char_boundary(split_at) {
                    split_at += 1;
                }
            }

            chunks.push(remaining[..split_at].to_string());
            remaining = &remaining[split_at..];
        }
    }

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    chunks
}
