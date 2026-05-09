use super::{GateIssue, PatchConfidence, TextPatch};

pub(super) fn apply_safe_patches(text: &str, issues: &[GateIssue]) -> Option<String> {
    let mut patches = issues
        .iter()
        .filter_map(|issue| issue.deterministic_fix.as_ref())
        .filter(|patch| matches!(patch.confidence, PatchConfidence::Safe))
        .collect::<Vec<_>>();

    if patches.is_empty() || has_overlaps(&patches) {
        return None;
    }

    patches.sort_by_key(|patch| patch.span.start);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for TextPatch {
        span, replacement, ..
    } in patches
    {
        if !text.is_char_boundary(span.start)
            || !text.is_char_boundary(span.end)
            || span.start < cursor
        {
            return None;
        }
        out.push_str(&text[cursor..span.start]);
        out.push_str(replacement);
        cursor = span.end;
    }
    out.push_str(&text[cursor..]);
    (out != text).then_some(out)
}

fn has_overlaps(patches: &[&TextPatch]) -> bool {
    let mut ranges = patches
        .iter()
        .map(|patch| (patch.span.start, patch.span.end))
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
}
