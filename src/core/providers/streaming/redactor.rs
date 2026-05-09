const REASONING_OPEN_TAGS: [&str; 2] = ["<think>", "<reasoning>"];
const REASONING_CLOSE_TAGS: [&str; 2] = ["</think>", "</reasoning>"];
const REDACTION_CONTROL_TOKENS: [&str; 5] = [
    "```",
    REASONING_OPEN_TAGS[0],
    REASONING_CLOSE_TAGS[0],
    REASONING_OPEN_TAGS[1],
    REASONING_CLOSE_TAGS[1],
];

#[derive(Default)]
enum CodeContext {
    #[default]
    Plain,
    CodeFence,
    InlineCode,
}

impl CodeContext {
    fn in_code_fence(&self) -> bool {
        matches!(self, Self::CodeFence)
    }

    fn in_inline_code(&self) -> bool {
        matches!(self, Self::InlineCode)
    }

    fn toggle_code_fence(&mut self) {
        *self = if self.in_code_fence() {
            Self::Plain
        } else {
            Self::CodeFence
        };
    }

    fn toggle_inline_code(&mut self) {
        *self = if self.in_inline_code() {
            Self::Plain
        } else {
            Self::InlineCode
        };
    }
}

#[derive(Default)]
pub(super) struct ReasoningStreamRedactor {
    show_reasoning: bool,
    pending: String,
    redaction_depth: usize,
    code_context: CodeContext,
    internal_block_active: bool,
    internal_line_candidate: String,
    buffering_internal_header: bool,
}

impl ReasoningStreamRedactor {
    pub(super) fn new(show_reasoning: bool) -> Self {
        Self {
            show_reasoning,
            pending: String::new(),
            redaction_depth: 0,
            code_context: CodeContext::Plain,
            internal_block_active: false,
            internal_line_candidate: String::new(),
            buffering_internal_header: false,
        }
    }

    pub(super) fn visible_delta(&mut self, incoming_delta: &str) -> String {
        let visible = if self.show_reasoning {
            incoming_delta.to_string()
        } else {
            self.pending.push_str(incoming_delta);
            self.process_pending(false)
        };
        self.redact_internal_prompt_blocks(&visible, false)
    }

    pub(super) fn finish_visible(&mut self) -> String {
        let tail = if self.show_reasoning {
            String::new()
        } else {
            self.process_pending(true)
        };
        self.redact_internal_prompt_blocks(&tail, true)
    }

    pub(super) fn should_hold_output(&self) -> bool {
        (!self.show_reasoning && (self.redaction_depth > 0 || !self.pending.is_empty()))
            || self.internal_block_active
            || self.buffering_internal_header
    }

    fn process_pending(&mut self, flush_all: bool) -> String {
        let mut output = String::new();
        let safe_limit = if flush_all {
            self.pending.len()
        } else {
            self.pending_safe_limit()
        };
        let mut cursor = 0usize;

        while cursor < safe_limit {
            if !self.code_context.in_inline_code()
                && self.pending[cursor..].starts_with("```")
                && cursor + 3 > safe_limit
                && !flush_all
            {
                break;
            }

            if let Some(consumed) = self.consume_code_fence(cursor, &mut output) {
                cursor += consumed;
                continue;
            }

            if let Some(consumed) = self.consume_inline_code(cursor, &mut output) {
                cursor += consumed;
                continue;
            }

            if !self.code_context.in_code_fence() && !self.code_context.in_inline_code() {
                match self.consume_reasoning_tag(cursor, flush_all) {
                    TagMatch::Open(tag_len) | TagMatch::Close(tag_len) => {
                        cursor += tag_len;
                        continue;
                    }
                    TagMatch::Incomplete => break,
                    TagMatch::None => {}
                }
            }

            let Some(next_char) = self.pending[cursor..].chars().next() else {
                break;
            };
            if self.redaction_depth == 0 {
                output.push(next_char);
            }
            cursor += next_char.len_utf8();
        }

        self.pending.drain(..cursor);
        output
    }

    fn consume_code_fence(&mut self, cursor: usize, output: &mut String) -> Option<usize> {
        let rest = &self.pending[cursor..];
        if self.code_context.in_inline_code() || !rest.starts_with("```") {
            return None;
        }
        if self.redaction_depth == 0 {
            output.push_str("```");
        }
        self.code_context.toggle_code_fence();
        Some(3)
    }

    fn consume_inline_code(&mut self, cursor: usize, output: &mut String) -> Option<usize> {
        let rest = &self.pending[cursor..];
        if self.code_context.in_code_fence() || !rest.starts_with('`') {
            return None;
        }
        if self.redaction_depth == 0 {
            output.push('`');
        }
        self.code_context.toggle_inline_code();
        Some(1)
    }

    fn consume_reasoning_tag(&mut self, cursor: usize, flush_all: bool) -> TagMatch {
        let rest = &self.pending[cursor..];
        match match_reasoning_tag(rest, flush_all, REASONING_OPEN_TAGS, REASONING_CLOSE_TAGS) {
            TagMatch::Open(tag_len) => {
                self.redaction_depth = self.redaction_depth.saturating_add(1);
                TagMatch::Open(tag_len)
            }
            TagMatch::Close(tag_len) => {
                self.redaction_depth = self.redaction_depth.saturating_sub(1);
                TagMatch::Close(tag_len)
            }
            other => other,
        }
    }

    fn pending_safe_limit(&self) -> usize {
        let max_tail_bytes = REDACTION_CONTROL_TOKENS
            .iter()
            .map(|token| token.len().saturating_sub(1))
            .max()
            .unwrap_or_default();
        let search_start = crate::utils::text::floor_char_boundary(
            &self.pending,
            self.pending.len().saturating_sub(max_tail_bytes),
        );
        let mut hold_from = self.pending.len();

        for (start, _) in self.pending.char_indices() {
            if start < search_start {
                continue;
            }
            let suffix = &self.pending[start..];
            if REDACTION_CONTROL_TOKENS
                .iter()
                .any(|token| has_incomplete_control_suffix(token, suffix))
            {
                hold_from = hold_from.min(start);
            }
        }

        hold_from
    }

    fn redact_internal_prompt_blocks(&mut self, input: &str, flush_all: bool) -> String {
        let mut output = String::new();

        for ch in input.chars() {
            if self.internal_block_active {
                self.internal_line_candidate.push(ch);
                if ch == '\n' {
                    if self.internal_line_candidate.trim().is_empty() {
                        self.internal_block_active = false;
                    }
                    self.internal_line_candidate.clear();
                }
                continue;
            }

            if self.buffering_internal_header {
                self.internal_line_candidate.push(ch);
                if ch == '\n' {
                    let candidate = std::mem::take(&mut self.internal_line_candidate);
                    if crate::utils::text::is_internal_prompt_block_header(candidate.trim()) {
                        self.internal_block_active = true;
                    } else {
                        output.push_str(&candidate);
                    }
                    self.buffering_internal_header = false;
                }
                continue;
            }

            if ch == '[' {
                self.buffering_internal_header = true;
                self.internal_line_candidate.push(ch);
                continue;
            }

            output.push(ch);
        }

        if flush_all && self.buffering_internal_header {
            let candidate = std::mem::take(&mut self.internal_line_candidate);
            if !crate::utils::text::is_internal_prompt_block_header(candidate.trim()) {
                output.push_str(&candidate);
            }
            self.buffering_internal_header = false;
        }

        if flush_all && self.internal_block_active {
            self.internal_line_candidate.clear();
            self.internal_block_active = false;
        }

        output
    }
}

enum TagMatch {
    Open(usize),
    Close(usize),
    Incomplete,
    None,
}

fn has_incomplete_control_suffix(control: &str, suffix: &str) -> bool {
    !suffix.is_empty()
        && suffix.len() < control.len()
        && control.as_bytes()[..suffix.len()].eq_ignore_ascii_case(suffix.as_bytes())
}

fn match_reasoning_tag<const N: usize>(
    rest: &str,
    flush_all: bool,
    open_tags: [&str; N],
    close_tags: [&str; N],
) -> TagMatch {
    for tag in open_tags {
        if rest.len() >= tag.len()
            && rest.as_bytes()[..tag.len()].eq_ignore_ascii_case(tag.as_bytes())
        {
            return TagMatch::Open(tag.len());
        }
        if !flush_all
            && rest.len() < tag.len()
            && tag.as_bytes()[..rest.len()].eq_ignore_ascii_case(rest.as_bytes())
        {
            return TagMatch::Incomplete;
        }
    }

    for tag in close_tags {
        if rest.len() >= tag.len()
            && rest.as_bytes()[..tag.len()].eq_ignore_ascii_case(tag.as_bytes())
        {
            return TagMatch::Close(tag.len());
        }
        if !flush_all
            && rest.len() < tag.len()
            && tag.as_bytes()[..rest.len()].eq_ignore_ascii_case(rest.as_bytes())
        {
            return TagMatch::Incomplete;
        }
    }

    TagMatch::None
}

#[cfg(test)]
mod tests {
    use super::ReasoningStreamRedactor;

    #[test]
    fn visible_delta_redacts_internal_prompt_blocks() {
        let mut redactor = ReasoningStreamRedactor::new(false);
        let mut visible = String::new();

        visible.push_str(&redactor.visible_delta("[Integrated Model]\n"));
        visible.push_str(&redactor.visible_delta("- situational_awareness=0.54\n"));
        visible.push_str(&redactor.visible_delta("\n"));
        visible.push_str(&redactor.visible_delta("夜は静かだけど、"));
        visible.push_str(&redactor.visible_delta("気持ちまで静かとは限らない。\n"));
        visible.push_str(&redactor.finish_visible());

        assert_eq!(visible, "夜は静かだけど、気持ちまで静かとは限らない。\n");
    }

    #[test]
    fn visible_delta_keeps_non_internal_bracket_lines() {
        let mut redactor = ReasoningStreamRedactor::new(false);
        let mut visible = String::new();

        visible.push_str(&redactor.visible_delta("[Playlist]\n"));
        visible.push_str(&redactor.visible_delta("- dawn\n"));
        visible.push_str(&redactor.finish_visible());

        assert_eq!(visible, "[Playlist]\n- dawn\n");
    }
}
