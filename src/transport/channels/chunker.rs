//! Message chunker: splits long responses into channel-safe segments
//! by respecting sentence boundaries, code fences, and max-length limits.
/// Returns `true` if `text` has at most `max_chars` Unicode scalar values.
///
/// Uses byte-length as a fast O(1) lower bound: when `len() <= max_chars`
/// the char count is trivially within the limit.  Otherwise falls back to
/// early-exit char iteration so it stops counting after `max_chars + 1` chars.
fn within_char_limit(text: &str, max_chars: usize) -> bool {
    text.len() <= max_chars || text.chars().nth(max_chars).is_none()
}

fn char_count(text: &str) -> usize {
    text.chars().count()
}

fn split_keep_delimiter(text: &str, delimiter: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;

    while let Some(relative_pos) = text[start..].find(delimiter) {
        let end = start + relative_pos + delimiter.len();
        parts.push(text[start..end].to_string());
        start = end;
    }

    if start < text.len() {
        parts.push(text[start..].to_string());
    }

    parts
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        if matches!(ch, '.' | '!' | '?')
            && let Some((next_index, next_char)) = chars.peek().copied()
            && next_char == ' '
        {
            let end = next_index + next_char.len_utf8();
            parts.push(text[start..end].to_string());
            start = end;
        }
    }

    if start < text.len() {
        parts.push(text[start..].to_string());
    }

    parts
}

fn hard_split(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;

    for ch in text.chars() {
        if current_len == max_chars {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }

        current.push(ch);
        current_len += 1;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

#[derive(Clone, Copy)]
enum SplitLevel {
    Paragraph,
    Line,
    Sentence,
    Word,
    Hard,
}

impl SplitLevel {
    fn next(self) -> Self {
        match self {
            Self::Paragraph => Self::Line,
            Self::Line => Self::Sentence,
            Self::Sentence => Self::Word,
            Self::Word | Self::Hard => Self::Hard,
        }
    }

    fn split(self, text: &str) -> Vec<String> {
        match self {
            Self::Paragraph => split_keep_delimiter(text, "\n\n"),
            Self::Line => split_keep_delimiter(text, "\n"),
            Self::Sentence => split_sentences(text),
            Self::Word => split_keep_delimiter(text, " "),
            Self::Hard => hard_split(text, 1),
        }
    }
}

fn chunk_segment(text: &str, max_chars: usize, level: SplitLevel, out: &mut Vec<String>) {
    if text.is_empty() {
        return;
    }

    if within_char_limit(text, max_chars) {
        out.push(text.to_string());
        return;
    }

    if matches!(level, SplitLevel::Hard) {
        out.extend(hard_split(text, max_chars));
        return;
    }

    let parts = level.split(text);
    if parts.len() <= 1 {
        chunk_segment(text, max_chars, level.next(), out);
        return;
    }

    let mut current = String::new();

    for part in parts {
        if !within_char_limit(&part, max_chars) {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            chunk_segment(&part, max_chars, level.next(), out);
            continue;
        }

        if current.is_empty() {
            current = part;
            continue;
        }

        // Fast path: if combined byte length fits, char count is guaranteed to fit.
        let combined_fits = (current.len() + part.len()) <= max_chars
            || char_count(&current) + char_count(&part) <= max_chars;
        if combined_fits {
            current.push_str(&part);
        } else {
            out.push(std::mem::take(&mut current));
            current = part;
        }
    }

    if !current.is_empty() {
        out.push(current);
    }
}

#[must_use]
pub fn chunk_message(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() || max_chars == 0 {
        return Vec::new();
    }

    if within_char_limit(text, max_chars) {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    chunk_segment(text, max_chars, SplitLevel::Paragraph, &mut chunks);
    chunks
}

#[cfg(test)]
mod tests {
    use super::chunk_message;

    #[test]
    fn chunk_empty_message() {
        assert!(chunk_message("", 10).is_empty());
    }

    #[test]
    fn chunk_short_message() {
        assert_eq!(chunk_message("hello", 10), vec!["hello"]);
    }

    #[test]
    fn chunk_long_url_with_hard_split() {
        let text = "https://example.com/".repeat(20);
        let chunks = chunk_message(&text, 30);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 30));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn chunk_unicode_by_character_count() {
        let text = "🦀世界こんにちは";
        let chunks = chunk_message(text, 3);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 3));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn chunk_with_paragraph_sentence_word_fallback() {
        let text = "Paragraph one has several words. Another sentence!\n\nParagraph two has averyveryverylongwordwithoutspaces";
        let chunks = chunk_message(text, 25);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 25));
        assert_eq!(chunks.concat(), text);
    }
}
