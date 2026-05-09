#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl TextSpan {
    #[must_use]
    pub(crate) const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub(crate) const fn offset(self, start: usize, end: usize) -> Self {
        Self {
            start: self.start + start,
            end: self.start + end,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Paragraph,
    ListItem,
    List,
    Heading { level: u8 },
    CodeBlock,
    Quote,
    Table,
    Blank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block<'a> {
    pub(crate) kind: BlockKind,
    pub(crate) text: &'a str,
    pub(crate) span: TextSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sentence<'a> {
    pub(crate) text: &'a str,
    pub(crate) span: TextSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0];
        for (idx, ch) in source.char_indices() {
            if ch == '\n' {
                starts.push(idx + 1);
            }
        }
        Self { starts }
    }

    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn line_for_offset(&self, offset: usize) -> usize {
        self.starts.partition_point(|start| *start <= offset)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Document<'a> {
    pub(crate) source: &'a str,
    pub(crate) blocks: Vec<Block<'a>>,
    pub(crate) sentences: Vec<Sentence<'a>>,
    pub(crate) line_index: LineIndex,
}

impl<'a> Document<'a> {
    #[must_use]
    pub(crate) fn parse(source: &'a str) -> Self {
        let line_index = LineIndex::new(source);
        let blocks = scan_blocks(source);
        let sentences = scan_sentences(source, &blocks);
        Self {
            source,
            blocks,
            sentences,
            line_index,
        }
    }
}

fn scan_blocks(source: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    let mut in_fence = false;
    let mut fence_start = 0;

    for raw_line in source.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line_start = offset;
        let line_end = line_start + line.len();
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if in_fence {
                blocks.push(Block {
                    kind: BlockKind::CodeBlock,
                    text: &source[fence_start..line_end],
                    span: TextSpan::new(fence_start, line_end),
                });
                in_fence = false;
            } else {
                in_fence = true;
                fence_start = line_start;
            }
            offset += raw_line.len();
            continue;
        }

        if in_fence {
            offset += raw_line.len();
            continue;
        }

        let kind = classify_line(line);
        blocks.push(Block {
            kind,
            text: line,
            span: TextSpan::new(line_start, line_end),
        });
        offset += raw_line.len();
    }

    if in_fence {
        blocks.push(Block {
            kind: BlockKind::CodeBlock,
            text: &source[fence_start..],
            span: TextSpan::new(fence_start, source.len()),
        });
    }

    blocks
}

fn classify_line(line: &str) -> BlockKind {
    if line.starts_with("    ") || line.starts_with('\t') {
        return BlockKind::CodeBlock;
    }

    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return BlockKind::Blank;
    }
    if let Some(level) = heading_level(trimmed) {
        return BlockKind::Heading { level };
    }
    if is_list_item(trimmed) {
        return BlockKind::ListItem;
    }
    if trimmed.starts_with('>') {
        return BlockKind::Quote;
    }
    if looks_like_table(line) {
        return BlockKind::Table;
    }
    BlockKind::Paragraph
}

fn heading_level(trimmed: &str) -> Option<u8> {
    let count = trimmed.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&count)
        && trimmed
            .as_bytes()
            .get(count)
            .is_some_and(u8::is_ascii_whitespace)
    {
        u8::try_from(count).ok()
    } else {
        None
    }
}

fn is_list_item(trimmed: &str) -> bool {
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .is_some()
        || numbered_item_body(trimmed).is_some()
}

fn numbered_item_body(trimmed: &str) -> Option<&str> {
    let dot = trimmed.find('.')?;
    if dot == 0 || dot > 3 || !trimmed[..dot].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    trimmed.get(dot + 1..)?.strip_prefix(' ')
}

fn looks_like_table(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn scan_sentences<'a>(source: &'a str, blocks: &[Block<'a>]) -> Vec<Sentence<'a>> {
    let mut sentences = Vec::new();
    for block in blocks {
        if matches!(block.kind, BlockKind::CodeBlock | BlockKind::Blank) {
            continue;
        }
        let mut start = 0;
        for (idx, ch) in block.text.char_indices() {
            if matches!(ch, '。' | '！' | '？' | '.' | '!' | '?') {
                let end = idx + ch.len_utf8();
                push_sentence(block, start, end, &mut sentences);
                start = end;
            }
        }
        if start < block.text.len() {
            push_sentence(block, start, block.text.len(), &mut sentences);
        }
    }
    sentences.retain(|sentence| {
        !sentence.text.trim().is_empty() && source.is_char_boundary(sentence.span.start)
    });
    sentences
}

fn push_sentence<'a>(
    block: &Block<'a>,
    start: usize,
    end: usize,
    sentences: &mut Vec<Sentence<'a>>,
) {
    let text = &block.text[start..end];
    let trimmed_start = text.len() - text.trim_start().len();
    let trimmed_end = text.trim_end().len();
    if trimmed_start >= trimmed_end {
        return;
    }
    sentences.push(Sentence {
        text: &text[trimmed_start..trimmed_end],
        span: block
            .span
            .offset(start + trimmed_start, start + trimmed_end),
    });
}

pub(crate) fn list_item_body(line: &str) -> &str {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .or_else(|| numbered_item_body(trimmed))
        .unwrap_or(trimmed)
}
