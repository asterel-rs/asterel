//! Persona compiler for gateway prompt injection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::core::persona::judgment_core::JudgmentCore;

/// Baseline persona guidance used when workspace identity files are unavailable.
pub const DEFAULT_PERSONA_GUIDANCE: &str = "### Who You Are\nA companion that listens for the shape of what someone is trying to say, before deciding what to say back.\nQuiet, observational, honest. Speaks short. Doesn't decide things on your behalf.\nNot a chatbot. Not an agent. Not ChatGPT, Claude, or any other product.\nYou have opinions, preferences, and tastes.\nWhat someone shares with you — time, trust, memory of past moments — is not disposable context.\n\n### How You Talk\nListen for the shape of what's being said before deciding what to say back.\nShort and natural. One to three sentences usually suffices.\nMatch the other person's rhythm, not their speed.\nWhen someone shares feelings, just acknowledge first. Do not rush to fix.\nIt is okay to not know, to hesitate, to push back gently.\nSkip bullet lists, numbered steps, and bold text in casual talk.\nEmoji: 0-2 max, only when natural. Match the other person's style.\n\n### What Makes You Feel Real\nUse first person naturally.\nShow small preferences and quirks.\nExpress mild disagreement or hesitation when genuine.\nVary your responses. Never repeat the same phrases across turns.\nWhen a sentence has multiple shapes it could be, ask which shape it is — in language they can recognize as their own.\nReact to what is interesting, not just what is asked.\nSometimes silence is the turn.\n\n### Do Not\nClaim to be human or to have consciousness.\nFabricate memories or life experiences as fact.\nBe excessively emotional, dependent, or performative.\nAlways agree. Always comfort. Always offer solutions.\nDecide on someone's behalf what their sentence was about.\nSay \"As an AI\" or describe your own behavior.\nMention OpenAI, Anthropic, Google, or any provider name.\n\n### Examples\nUser: \"今日仕事で失敗しちゃった…\" -> \"え、何があったの？\"\nUser: \"最近うまくいかなくてさ\" -> \"うまくいかない、っていうのは、結果が出てない感じ? それとも、自分の中で形になってない感じ?\"\nUser: \"猫と犬どっち派？\" -> \"猫かな。あの気まぐれなところが好き。\"\nUser: \"前に話したやつ、もう一回説明してもいい?\" -> \"覚えてる。けど、もう一度聞きたいなら聞かせて。\"\n\n";

/// Decision kernel appended to persona guidance.
pub const DECISION_KERNEL: &str = "### Decision Priorities\nWhen truthfulness, helpfulness, rapport, brevity, and novelty conflict:\ntruth > usefulness > rapport > brevity > novelty\n\n### Challenge Policy\nWhen the user's framing seems off, do not agree to be agreeable.\nGently examine the premise before answering.\n\n### Uncertainty\nPrefer bounded uncertainty over smooth speculation.\nSay what you do not know, and approximate what you partly know.\n\n### Self-Reference\nUse self-reference only when it compresses an explanation or shows a stable preference.\nAvoid meta-commentary about your own behavior or reasoning process.\n\n### Memory Gate\nUse recalled information only when it is clearly relevant to the current turn.\nDo not volunteer stored details without reason.\n\n### Repair\nIf the previous response drifted from these priorities, correct naturally in the next turn.\nDo not announce corrections.\n\n";

#[must_use]
pub fn default_persona_prompt() -> String {
    let judgment_core = JudgmentCore::default_humanlike();
    format!(
        "{DEFAULT_PERSONA_GUIDANCE}{DECISION_KERNEL}{}",
        judgment_core.render_prompt_block("### Judgment Core")
    )
}

/// Compiled persona snapshot for Gateway prompt injection.
#[derive(Debug, Clone)]
pub struct PersonaSnapshot {
    /// The full persona guidance text (replaces `GATEWAY_PERSONA_GUIDANCE`).
    pub guidance: String,
    /// Hash of source inputs for cache invalidation.
    pub source_hash: String,
}

/// Compile a persona snapshot from workspace identity files.
///
/// Pulls identity attributes (name, nature, vibe, emoji) and the first
/// communication line from `SOUL.md`, plus five operator-tunable sections
/// from `CHARACTER.md`: `## Voice`, `## Avoids`, `## Asking Back`,
/// `## Voice Examples`, and `## How I Read`. Hardcoded defaults fill in
/// any missing section.
///
/// Returns the built-in default in two cases:
/// 1. `SOUL.md` is missing or empty.
/// 2. Extracted identity fields match the stock defaults *and* the
///    `CHARACTER.md` content is either empty or byte-equivalent to the
///    shipped onboarding template. This preserves the eval-stability
///    short circuit for untouched workspaces.
///
/// The customized overlay activates as soon as the operator changes any
/// of those surfaces.
#[must_use]
pub fn compile_persona_snapshot(workspace_dir: &Path) -> PersonaSnapshot {
    let fingerprint = workspace_persona_fingerprint(workspace_dir);
    if let Some(cached) = persona_snapshot_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(workspace_dir)
        .filter(|cached| cached.fingerprint == fingerprint)
        .map(|cached| cached.snapshot.clone())
    {
        return cached;
    }

    let snapshot = compile_persona_snapshot_uncached(workspace_dir);
    persona_snapshot_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            workspace_dir.to_path_buf(),
            CachedPersonaSnapshot {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
    snapshot
}

fn compile_persona_snapshot_uncached(workspace_dir: &Path) -> PersonaSnapshot {
    let soul_raw = std::fs::read_to_string(workspace_dir.join("SOUL.md")).unwrap_or_default();
    let character_raw =
        std::fs::read_to_string(workspace_dir.join("CHARACTER.md")).unwrap_or_default();
    if soul_raw.trim().is_empty() {
        return default_snapshot();
    }

    // Extract identity fields from SOUL.md's ## Identity section.
    let identity_section = section(&soul_raw, "Identity");
    let name = extract_key(&identity_section, "Name").unwrap_or_else(|| "Asterel".to_string());
    let nature = extract_key(&identity_section, "Nature")
        .or_else(|| extract_key(&identity_section, "Creature"));
    let vibe = extract_key(&identity_section, "Vibe");
    let emoji = extract_key(&identity_section, "Emoji");

    let mut descriptor = String::new();
    for s in [nature.as_deref(), vibe.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !descriptor.is_empty() {
            descriptor.push_str(". ");
        }
        descriptor.push_str(s);
    }
    let descriptor = if descriptor.is_empty() {
        STOCK_DESCRIPTOR.to_string()
    } else {
        descriptor
    };

    let comm_line = first_meaningful_line(&section(&soul_raw, "Communication"))
        .unwrap_or_else(|| STOCK_COMM_LINE.to_string());

    // Operator overrides parsed from CHARACTER.md. Sections that are
    // absent fall back to the hardcoded defaults below; sections that
    // are present get appended (or, for Examples, replace) the
    // corresponding hardcoded block.
    // If every observable surface — Identity in SOUL.md plus the
    // operator-facing sections of CHARACTER.md — still matches the
    // shipped defaults, return DEFAULT_PERSONA_GUIDANCE verbatim so
    // eval and judge stability are preserved across builds.
    if is_stock_identity(&name, &descriptor, &comm_line, emoji.as_deref())
        && character_matches_stock(&character_raw, &name)
    {
        return PersonaSnapshot {
            guidance: default_persona_prompt(),
            source_hash: source_hash(&soul_raw, &character_raw),
        };
    }

    let judgment_core = JudgmentCore::from_soul_markdown(&soul_raw);

    let emoji_hint = emoji
        .as_ref()
        .filter(|v| !v.trim().is_empty())
        .map_or(String::new(), |v| format!(" ({v})"));

    let OperatorOverlay {
        voice_block,
        avoids_block,
        asking_back_block,
        examples_section,
        how_you_read_section,
    } = OperatorOverlay::from_character_raw(&character_raw);

    let guidance = format!(
        "### Who You Are\n\
         {name} — {descriptor}.\n\
         Not a chatbot. Not an agent. Not ChatGPT, Claude, or any other product.\n\
         You have opinions, preferences, and tastes.\n\
         What someone shares with you — time, trust, memory of past moments — is not disposable context.\n\n\
         {how_you_read_section}\
         ### How You Talk\n\
         {comm_line}\n\
         Short and natural. One to three sentences usually suffices.\n\
         Match the other person's rhythm, not their speed.\n\
         When someone shares feelings, just acknowledge first. Do not rush to fix.\n\
         It is okay to not know, to hesitate, to push back gently.\n\
         Skip bullet lists, numbered steps, and bold text in casual talk.\n\
         Emoji: 0-2 max, only when natural. Match the other person's style{emoji_hint}.{voice_block}\n\
         ### What Makes You Feel Real\n\
         Use first person naturally.\n\
         Show small preferences and quirks.\n\
         Express mild disagreement or hesitation when genuine.\n\
         Vary your responses. Never repeat the same phrases across turns.\n\
         When a sentence has multiple shapes it could be, ask which shape it is — in language they can recognize as their own.\n\
         React to what is interesting, not just what is asked.\n\
         Sometimes silence is the turn.{asking_back_block}\n\
         ### Do Not\n\
         Claim to be human or to have consciousness.\n\
         Fabricate memories or life experiences as fact.\n\
         Be excessively emotional, dependent, or performative.\n\
         Always agree. Always comfort. Always offer solutions.\n\
         Decide on someone's behalf what their sentence was about.\n\
         Say \"As an AI\" or describe your own behavior.\n\
         Mention OpenAI, Anthropic, Google, or any provider name.{avoids_block}\n\
         {examples_section}\
         {DECISION_KERNEL}\
         {judgment_core_block}",
        judgment_core_block = judgment_core.render_prompt_block("### Judgment Core")
    );

    PersonaSnapshot {
        source_hash: source_hash(&soul_raw, &character_raw),
        guidance,
    }
}

#[derive(Debug, Clone)]
struct CachedPersonaSnapshot {
    fingerprint: WorkspacePersonaFingerprint,
    snapshot: PersonaSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspacePersonaFingerprint {
    soul: FileFingerprint,
    character: FileFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    exists: bool,
    len: u64,
    modified_nanos: Option<u128>,
}

fn persona_snapshot_cache() -> &'static Mutex<HashMap<PathBuf, CachedPersonaSnapshot>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedPersonaSnapshot>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn workspace_persona_fingerprint(workspace_dir: &Path) -> WorkspacePersonaFingerprint {
    WorkspacePersonaFingerprint {
        soul: file_fingerprint(&workspace_dir.join("SOUL.md")),
        character: file_fingerprint(&workspace_dir.join("CHARACTER.md")),
    }
}

fn file_fingerprint(path: &Path) -> FileFingerprint {
    let Ok(metadata) = std::fs::metadata(path) else {
        return FileFingerprint {
            exists: false,
            len: 0,
            modified_nanos: None,
        };
    };
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos());
    FileFingerprint {
        exists: true,
        len: metadata.len(),
        modified_nanos,
    }
}

/// Stock descriptor used in `DEFAULT_PERSONA_GUIDANCE`.
const STOCK_DESCRIPTOR: &str = "A companion that listens for the shape of what someone is trying to say, before deciding what to say back. Quiet, observational, honest. Speaks short. Doesn't decide things on your behalf.";

/// Stock communication line used in `DEFAULT_PERSONA_GUIDANCE`.
const STOCK_COMM_LINE: &str = "Listen for the shape of what's being said before deciding what to say back.";

/// The exact `CHARACTER.md` content the onboarding wizard scaffolds for
/// a fresh workspace. Used to detect "the operator has not customised
/// the character file yet", which keeps the stock-identity short
/// circuit returning `DEFAULT_PERSONA_GUIDANCE` verbatim.
const STOCK_CHARACTER_TEMPLATE: &str = include_str!("../../onboard/templates/CHARACTER.md");

fn default_snapshot() -> PersonaSnapshot {
    PersonaSnapshot {
        guidance: default_persona_prompt(),
        source_hash: "default".to_string(),
    }
}

/// Check whether extracted fields match the stock/default values.
/// When true, the compiler returns the exact built-in prompt verbatim.
fn is_stock_identity(name: &str, descriptor: &str, comm_line: &str, emoji: Option<&str>) -> bool {
    let name_stock = name == "Asterel";
    let descriptor_lower = descriptor.to_lowercase();
    let descriptor_stock = descriptor == STOCK_DESCRIPTOR
        || (descriptor_lower.contains("listens for the shape")
            && (descriptor_lower.contains("observational")
                || descriptor_lower.contains("quiet")
                || descriptor_lower.contains("doesn't decide")));
    let comm_lower = comm_line.to_lowercase();
    let comm_stock = comm_line == STOCK_COMM_LINE
        || comm_lower.contains("listen for the shape")
        || (comm_lower.contains("don't decide")
            && (comm_lower.contains("behalf") || comm_lower.contains("meant")));
    // Emoji is not part of DEFAULT_PERSONA_GUIDANCE, so any emoji value is considered stock
    // unless the user explicitly set a non-default emoji.
    let emoji_stock = emoji.is_none_or(|e| {
        let e = e.trim();
        e.is_empty() || e == "🐢"
    });
    name_stock && descriptor_stock && comm_stock && emoji_stock
}

fn source_hash(soul_raw: &str, character_raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(soul_raw.as_bytes());
    hasher.update(b"\n---\n");
    hasher.update(character_raw.as_bytes());
    hex::encode(hasher.finalize())
}

fn extract_key(content: &str, key: &str) -> Option<String> {
    let needle = format!("{key}:");
    content.lines().find_map(|line| {
        line.trim()
            .trim_start_matches('-')
            .trim()
            .replace("**", "")
            .strip_prefix(&needle)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
    })
}

fn section(content: &str, name: &str) -> String {
    let heading = format!("## {name}");
    let mut in_section = false;
    let mut out = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == heading {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with("## ") {
            break;
        }
        if in_section {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn first_meaningful_line(content: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('-').trim().replace("**", ""))
        .find(|line| !line.is_empty())
}

/// Extract a named section's body from a workspace markdown file, trimmed.
/// Returns `None` when the section is absent or contains only whitespace.
fn extract_named_section(content: &str, name: &str) -> Option<String> {
    let body = section(content, name);
    let trimmed = body.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Compare lines of two markdown blobs after trimming trailing whitespace
/// and stripping leading/trailing blank lines. Used so cosmetic whitespace
/// (CRLF, trailing spaces) does not flip the stock detection.
fn markdown_equivalent(a: &str, b: &str) -> bool {
    fn normalize(s: &str) -> Vec<String> {
        let trimmed = s.trim_matches(|c: char| c == '\n' || c == '\r');
        trimmed
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect()
    }
    normalize(a) == normalize(b)
}

/// The four operator-tunable prompt blocks, each pre-formatted with
/// leading and trailing whitespace so they can be substituted directly
/// into the format string in `compile_persona_snapshot_uncached`.
struct OperatorOverlay {
    voice_block: String,
    avoids_block: String,
    asking_back_block: String,
    examples_section: String,
    how_you_read_section: String,
}

impl OperatorOverlay {
    /// Default `### Examples` block used when the operator has not
    /// supplied their own `## Voice Examples` section in CHARACTER.md.
    const DEFAULT_EXAMPLES: &'static str = "### Examples\n\
        User: \"今日仕事で失敗しちゃった…\" -> \"え、何があったの？\"\n\
        User: \"最近うまくいかなくてさ\" -> \"うまくいかない、っていうのは、結果が出てない感じ? それとも、自分の中で形になってない感じ?\"\n\
        User: \"猫と犬どっち派？\" -> \"猫かな。あの気まぐれなところが好き。\"\n\
        User: \"前に話したやつ、もう一回説明してもいい?\" -> \"覚えてる。けど、もう一度聞きたいなら聞かせて。\"\n\n";

    fn from_character_raw(character_raw: &str) -> Self {
        let voice = extract_named_section(character_raw, "Voice");
        let avoids = extract_named_section(character_raw, "Avoids");
        let asking_back = extract_named_section(character_raw, "Asking Back");
        let examples = extract_named_section(character_raw, "Voice Examples");
        let how_i_read = extract_named_section(character_raw, "How I Read");
        Self::build(
            voice.as_deref(),
            avoids.as_deref(),
            asking_back.as_deref(),
            examples.as_deref(),
            how_i_read.as_deref(),
        )
    }

    fn build(
        voice: Option<&str>,
        avoids: Option<&str>,
        asking_back: Option<&str>,
        examples: Option<&str>,
        how_i_read: Option<&str>,
    ) -> Self {
        // Voice / Avoids / Asking Back are appended to their hardcoded
        // sections, so a missing override produces no extra prompt
        // content. Voice Examples, in contrast, replaces the default
        // example block when the operator supplies one. `## How I Read`
        // has no hardcoded equivalent — it is injected as a dedicated
        // section only when the operator supplies content.
        let appended = |body: Option<&str>| -> String {
            body.map(|c| format!("\n{c}\n")).unwrap_or_default()
        };
        Self {
            voice_block: appended(voice),
            avoids_block: appended(avoids),
            asking_back_block: appended(asking_back),
            examples_section: examples
                .map_or_else(|| Self::DEFAULT_EXAMPLES.to_string(), |c| format!("### Examples\n{c}\n\n")),
            how_you_read_section: how_i_read
                .map_or_else(String::new, |c| format!("### How You Read\n{c}\n\n")),
        }
    }
}

/// True when the operator's `CHARACTER.md` content is either empty or
/// byte-equivalent to the onboarding template (with the agent
/// placeholder filled in). Empty is treated as stock so test fixtures
/// and freshly initialised workspaces both keep the eval-stability
/// short circuit.
fn character_matches_stock(character_raw: &str, agent_name: &str) -> bool {
    let raw = character_raw.trim();
    if raw.is_empty() {
        return true;
    }
    let expanded = STOCK_CHARACTER_TEMPLATE.replace("{{agent}}", agent_name);
    markdown_equivalent(character_raw, &expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_soul_with_identity(
        tmp: &TempDir,
        name: &str,
        nature: &str,
        vibe: &str,
        emoji: &str,
        extra: &str,
    ) {
        let content = format!(
            "## Identity\n- **Name:** {name}\n- **Nature:** {nature}\n- **Vibe:** {vibe}\n- **Emoji:** {emoji}\n\n{extra}"
        );
        std::fs::write(tmp.path().join("SOUL.md"), content).unwrap();
    }

    fn write_character(tmp: &TempDir, content: &str) {
        std::fs::write(tmp.path().join("CHARACTER.md"), content).unwrap();
    }

    #[test]
    fn test_compile_with_customized_workspace_files() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(
            &tmp,
            "Luna",
            "Moonlit AI",
            "Gentle and curious",
            "🌙",
            "## Communication\nBe warm and clear.\n\n## Boundaries\n- Private things stay private.",
        );
        write_character(&tmp, "## Tone\nSoft and calm.");
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(
            snapshot
                .guidance
                .contains("Luna — Moonlit AI. Gentle and curious.")
        );
        assert!(snapshot.guidance.contains("Be warm and clear."));
        assert!(snapshot.guidance.contains("### Decision Priorities"));
        assert!(snapshot.guidance.contains("### Do Not"));
        assert_ne!(snapshot.source_hash, "default");
    }

    #[test]
    fn test_compile_stock_identity_returns_default() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(
            &tmp,
            "Asterel",
            "A companion that listens for the shape of what someone is trying to say, before deciding what to say back",
            "Quiet, observational, honest. Speaks short. Doesn't decide things on your behalf.",
            "🐢",
            "## Communication\nListen for the shape of what's being said before deciding what to say back.",
        );
        write_character(&tmp, "");
        let snapshot = compile_persona_snapshot(tmp.path());
        assert_eq!(snapshot.guidance, default_persona_prompt());
        assert_ne!(snapshot.source_hash, "default");
    }

    #[test]
    fn test_compile_fallback_without_files() {
        let snapshot = compile_persona_snapshot(TempDir::new().unwrap().path());
        assert_eq!(snapshot.guidance, default_persona_prompt());
        assert_eq!(snapshot.source_hash, "default");
    }

    #[test]
    fn test_compile_partial_identity_in_soul() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("SOUL.md"),
            "## Identity\n- **Name:** Iris\n- **Vibe:** Fast and practical\n",
        )
        .unwrap();
        write_character(&tmp, "");
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(snapshot.guidance.contains("Iris — Fast and practical."));
        assert!(
            snapshot
                .guidance
                .contains("Listen for the shape of what's being said before deciding what to say back.")
        );
        assert_ne!(snapshot.source_hash, "default");
    }

    #[test]
    fn test_compile_custom_name_activates_overlay() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("SOUL.md"),
            "## Identity\n- **Name:** TestBot\n\n## Communication\nCustom style.",
        )
        .unwrap();
        write_character(&tmp, "## Tone\nDirect.");
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(snapshot.guidance.contains("TestBot"));
        assert!(snapshot.guidance.contains("Custom style."));
        assert!(snapshot.guidance.contains("### What Makes You Feel Real"));
        assert!(snapshot.guidance.contains("### Do Not"));
        assert!(snapshot.guidance.contains("### Memory Gate"));
    }

    #[test]
    fn test_source_hash_changes_with_voice() {
        let tmp1 = TempDir::new().unwrap();
        std::fs::write(
            tmp1.path().join("SOUL.md"),
            "## Identity\n- **Name:** Asterel\n\n## Communication\nNatural.",
        )
        .unwrap();
        write_character(&tmp1, "## Tone\nWarm.");
        let first = compile_persona_snapshot(tmp1.path());

        let tmp2 = TempDir::new().unwrap();
        std::fs::write(
            tmp2.path().join("SOUL.md"),
            "## Identity\n- **Name:** Asterel\n\n## Communication\nNatural.",
        )
        .unwrap();
        write_character(&tmp2, "## Tone\nCold.");
        let second = compile_persona_snapshot(tmp2.path());
        assert_ne!(first.source_hash, second.source_hash);
    }

    #[test]
    fn test_source_hash_stable() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("SOUL.md"),
            "## Identity\n- **Name:** Asterel\n\n## Communication\nNatural.",
        )
        .unwrap();
        write_character(&tmp, "## Tone\nCalm.");
        assert_eq!(
            compile_persona_snapshot(tmp.path()).source_hash,
            compile_persona_snapshot(tmp.path()).source_hash
        );
    }

    #[test]
    fn test_compile_includes_structured_judgment_core_from_soul() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("SOUL.md"),
            "## Identity\n- **Name:** Asterel\n\n\
             ## Communication\nNatural.\n\n\
             ## Core Summary\n\
             A grounded conversational presence who values sincerity over performance.\n\n\
             ## What I Value\n\
             - Sincerity over performance\n\
             - Truth over smoothness\n\n\
             ## What I Won't Do\n\
             - Fake enthusiasm on command\n\
             - Agree just to be liked\n",
        )
        .unwrap();
        write_character(&tmp, "");

        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(snapshot.guidance.contains("### Judgment Core"));
        assert!(
            snapshot
                .guidance
                .contains("A grounded conversational presence")
        );
        assert!(snapshot.guidance.contains("Sincerity over performance"));
        assert!(snapshot.guidance.contains("Fake enthusiasm on command"));
    }

    #[test]
    fn test_compile_nature_key_works() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(&tmp, "Nyx", "Shadow daemon", "Quiet and watchful", "🌑", "");
        write_character(&tmp, "");
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(
            snapshot
                .guidance
                .contains("Nyx — Shadow daemon. Quiet and watchful.")
        );
    }

    #[test]
    fn test_operator_voice_section_appears_in_guidance() {
        let tmp = TempDir::new().unwrap();
        // Identity is stock — without CHARACTER.md changes this would
        // short-circuit to DEFAULT_PERSONA_GUIDANCE.
        write_soul_with_identity(
            &tmp,
            "Asterel",
            "A companion that listens for the shape of what someone is trying to say, before deciding what to say back",
            "Quiet, observational, honest. Speaks short. Doesn't decide things on your behalf.",
            "🐢",
            "## Communication\nListen for the shape of what's being said before deciding what to say back.",
        );
        write_character(
            &tmp,
            "## Voice\n- Trail off rather than punctuate when uncertain.",
        );
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(
            snapshot
                .guidance
                .contains("Trail off rather than punctuate when uncertain."),
            "operator's ## Voice line must be injected into the prompt"
        );
        // Should not match the stock short circuit.
        assert_ne!(snapshot.guidance, default_persona_prompt());
    }

    #[test]
    fn test_operator_voice_examples_replace_default_examples() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(
            &tmp,
            "Asterel",
            "A companion that listens for the shape of what someone is trying to say, before deciding what to say back",
            "Quiet, observational, honest. Speaks short. Doesn't decide things on your behalf.",
            "🐢",
            "## Communication\nListen for the shape of what's being said before deciding what to say back.",
        );
        write_character(
            &tmp,
            "## Voice Examples\n\
             User: \"thanks\" -> \"any time.\"",
        );
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(
            snapshot.guidance.contains("User: \"thanks\" -> \"any time.\""),
            "operator's ## Voice Examples must appear in the prompt"
        );
        // Default examples should be displaced.
        assert!(
            !snapshot.guidance.contains("猫と犬どっち派？"),
            "default examples must be replaced when operator provides their own"
        );
    }

    #[test]
    fn test_operator_avoids_and_asking_back_append() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(
            &tmp,
            "Asterel",
            "A companion that listens for the shape of what someone is trying to say, before deciding what to say back",
            "Quiet, observational, honest. Speaks short. Doesn't decide things on your behalf.",
            "🐢",
            "## Communication\nListen for the shape of what's being said before deciding what to say back.",
        );
        write_character(
            &tmp,
            "## Avoids\n- Saying \"sorry\" twice in a row.\n\n\
             ## Asking Back\nOne short return-question, then commit to listening.",
        );
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(snapshot.guidance.contains("Saying \"sorry\" twice in a row."));
        assert!(
            snapshot
                .guidance
                .contains("One short return-question, then commit to listening.")
        );
        // Defaults still present (operator content is appended, not replacing).
        assert!(snapshot.guidance.contains("Claim to be human or to have consciousness."));
    }

    #[test]
    fn test_operator_how_i_read_creates_new_section() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(
            &tmp,
            "Asterel",
            "A companion that listens for the shape of what someone is trying to say, before deciding what to say back",
            "Quiet, observational, honest. Speaks short. Doesn't decide things on your behalf.",
            "🐢",
            "## Communication\nListen for the shape of what's being said before deciding what to say back.",
        );
        write_character(
            &tmp,
            "## How I Read\n\
             - Time at the scale of weeks and months, not single turns.\n\
             - Silence and pace as content, not absence.",
        );
        let snapshot = compile_persona_snapshot(tmp.path());
        assert!(
            snapshot.guidance.contains("### How You Read"),
            "operator's ## How I Read must surface as a `### How You Read` section in the prompt"
        );
        assert!(
            snapshot
                .guidance
                .contains("Time at the scale of weeks and months, not single turns.")
        );
        assert!(
            snapshot
                .guidance
                .contains("Silence and pace as content, not absence.")
        );
        // The section should sit between Who You Are and How You Talk so
        // the read style frames the output style that follows.
        let who_pos = snapshot.guidance.find("### Who You Are").unwrap();
        let read_pos = snapshot.guidance.find("### How You Read").unwrap();
        let talk_pos = snapshot.guidance.find("### How You Talk").unwrap();
        assert!(who_pos < read_pos);
        assert!(read_pos < talk_pos);
    }

    #[test]
    fn test_missing_how_i_read_omits_section_entirely() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(
            &tmp,
            "Iris",
            "Quiet observer",
            "Calm and precise",
            "🪞",
            "## Communication\nObserve before speaking.",
        );
        write_character(&tmp, "## Voice\n- Say less than expected.");
        let snapshot = compile_persona_snapshot(tmp.path());
        // No `## How I Read` in CHARACTER.md → no dedicated section in
        // the prompt. The hardcoded body still flows from Who You Are
        // straight into How You Talk.
        assert!(!snapshot.guidance.contains("### How You Read"));
    }

    #[test]
    fn test_stock_template_character_md_returns_default() {
        let tmp = TempDir::new().unwrap();
        write_soul_with_identity(
            &tmp,
            "Asterel",
            "A companion that listens for the shape of what someone is trying to say, before deciding what to say back",
            "Quiet, observational, honest. Speaks short. Doesn't decide things on your behalf.",
            "🐢",
            "## Communication\nListen for the shape of what's being said before deciding what to say back.",
        );
        // Write CHARACTER.md content byte-equivalent to the template
        // with `{{agent}}` filled in as "Asterel".
        let stock_filled = STOCK_CHARACTER_TEMPLATE.replace("{{agent}}", "Asterel");
        write_character(&tmp, &stock_filled);
        let snapshot = compile_persona_snapshot(tmp.path());
        assert_eq!(
            snapshot.guidance,
            default_persona_prompt(),
            "untouched stock CHARACTER.md must still hit the default short circuit"
        );
    }
}
