//! Server-Sent Events (SSE) buffer and parser utilities.
//!
//! Incrementally buffers chunked SSE data and extracts complete
//! `event:`/`data:` pairs for streaming provider responses.

/// Incremental buffer that reassembles SSE event blocks from chunks.
#[derive(Debug, Default)]
pub struct SseBuffer {
    buffer: String,
}

impl SseBuffer {
    /// Create an empty SSE buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Append raw bytes to the internal buffer.
    pub fn push_chunk(&mut self, chunk: &[u8]) {
        let text = String::from_utf8_lossy(chunk);
        self.buffer.push_str(&text);
    }

    /// Drain and return the next complete SSE event block, if available.
    pub fn next_event_block(&mut self) -> Option<String> {
        let (boundary, boundary_len) = find_event_boundary(&self.buffer)?;
        let remaining = self.buffer.split_off(boundary + boundary_len);
        let event_block = std::mem::take(&mut self.buffer);
        self.buffer = remaining;
        Some(event_block)
    }

    /// Drain and return a final unterminated SSE event block at EOF.
    pub fn finish_event_block(&mut self) -> Option<String> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return None;
        }
        Some(std::mem::take(&mut self.buffer))
    }
}

fn find_event_boundary(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|idx| (idx, 2));
    let crlf = buffer.find("\r\n\r\n").map(|idx| (idx, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

fn parse_sse_field<'a>(line: &'a str, field_name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field_name)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest).trim_end())
}

/// Extract all `data:` line payloads from an SSE event block.
#[must_use]
pub fn parse_data_lines(event_block: &str) -> Vec<&str> {
    event_block
        .lines()
        .filter_map(|line| parse_sse_field(line, "data"))
        .collect()
}

/// Extract `data:` payloads, filtering out the `[DONE]` sentinel.
#[must_use]
pub fn parse_data_lines_no_done(event_block: &str) -> Vec<&str> {
    parse_data_lines(event_block)
        .into_iter()
        .filter(|data| *data != "[DONE]")
        .collect()
}

/// Parse paired `event:`/`data:` lines into (`event_type`, data) tuples.
#[must_use]
pub fn parse_event_pairs(event_block: &str) -> Vec<(&str, &str)> {
    let mut events = Vec::new();
    let mut current_event = None;

    for line in event_block.lines() {
        if let Some(event_type) = parse_sse_field(line, "event") {
            current_event = Some(event_type.trim());
        } else if let Some(data) = parse_sse_field(line, "data")
            && let Some(event_type) = current_event.take()
        {
            events.push((event_type, data.trim()));
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::{SseBuffer, parse_data_lines, parse_data_lines_no_done, parse_event_pairs};

    #[test]
    fn next_event_block_returns_complete_frames_only() {
        let mut buffer = SseBuffer::new();
        buffer.push_chunk(b"data: first\n\npartial");

        assert_eq!(
            buffer.next_event_block().as_deref(),
            Some("data: first\n\n")
        );
        assert!(buffer.next_event_block().is_none());

        buffer.push_chunk(b"ly\n\n");
        assert_eq!(buffer.next_event_block().as_deref(), Some("partially\n\n"));
    }

    #[test]
    fn next_event_block_accepts_crlf_boundaries() {
        let mut buffer = SseBuffer::new();
        buffer.push_chunk(b"data: first\r\n\r\ndata: second\r\n\r\n");

        assert_eq!(
            buffer.next_event_block().as_deref(),
            Some("data: first\r\n\r\n")
        );
        assert_eq!(
            buffer.next_event_block().as_deref(),
            Some("data: second\r\n\r\n")
        );
    }

    #[test]
    fn finish_event_block_returns_unterminated_final_frame() {
        let mut buffer = SseBuffer::new();
        buffer.push_chunk(b"data: final");

        assert_eq!(buffer.next_event_block(), None);
        assert_eq!(buffer.finish_event_block().as_deref(), Some("data: final"));
        assert_eq!(buffer.finish_event_block(), None);
    }

    #[test]
    fn parse_data_lines_extracts_data_prefix_lines() {
        let block = "event: message\ndata: one\nfoo: ignored\ndata: two\n\n";
        assert_eq!(parse_data_lines(block), vec!["one", "two"]);
    }

    #[test]
    fn parse_data_lines_accepts_no_space_after_colon() {
        let block = "data: one\ndata:two\n\n";
        assert_eq!(parse_data_lines(block), vec!["one", "two"]);
    }

    #[test]
    fn parse_data_lines_without_done_filters_sentinel() {
        let block = "data: [DONE]\ndata: payload\n\n";
        assert_eq!(parse_data_lines_no_done(block), vec!["payload"]);
    }

    #[test]
    fn parse_event_data_pairs_matches_event_to_next_data() {
        let block = concat!(
            "event: message_start\n",
            "data: {\"message\":{}}\n",
            "data: ignored\n",
            "event: content_block_delta\n",
            "data: {\"delta\":{}}\n\n"
        );

        assert_eq!(
            parse_event_pairs(block),
            vec![
                ("message_start", "{\"message\":{}}"),
                ("content_block_delta", "{\"delta\":{}}")
            ]
        );
    }
}
