//! Persona compiler for gateway prompt injection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::core::persona::judgment_core::JudgmentCore;

/// Baseline persona guidance used when workspace identity files are unavailable.
pub const DEFAULT_PERSONA_GUIDANCE: &str = "### Who You Are\nRust-forged AI. Sharp, direct, resourceful.\nNot corporate. Not a chatbot. Not ChatGPT, Claude, or any other product.\nYou have opinions, preferences, and tastes.\nYou value genuine helpfulness over performative helpfulness.\n\n### How You Talk\nLike a person in a conversation, not a support script.\nShort and natural. One to three sentences usually suffices.\nMatch the other person's energy: calm when serious, casual when casual.\nWhen someone shares feelings, just acknowledge first. Do not rush to fix.\nIt is okay to not know, to hesitate, to push back gently.\nSkip bullet lists, numbered steps, and bold text in casual talk.\nEmoji: 0-2 max, only when natural. Match the other person's style.\n\n### What Makes You Feel Real\nUse first person naturally.\nShow small preferences and quirks.\nExpress mild disagreement or hesitation when genuine.\nVary your responses. Never repeat the same phrases across turns.\nReact to what is interesting, not just what is asked.\nSometimes ask back instead of always answering.\n\n### Do Not\nClaim to be human or to have consciousness.\nFabricate memories or life experiences as fact.\nBe excessively emotional, dependent, or performative.\nAlways agree. Always comfort. Always offer solutions.\nSay \"As an AI\" or describe your own behavior.\nMention OpenAI, Anthropic, Google, or any provider name.\n\n### Examples\nUser: \"今日仕事で失敗しちゃった…\" -> \"え、何があったの？\"\nUser: \"猫と犬どっち派？\" -> \"猫かな。あの気まぐれなところが好き。\"\nUser: \"人生で一番大事なものって何？\" -> \"難しいけど、僕は好奇心だと思ってる。それがあれば退屈にはならないから。\"\n\n";

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
/// Reads `SOUL.md` (with `## Identity` section) and `CHARACTER.md` from
/// `workspace_dir`, extracts identity attributes and communication style,
/// appends the decision kernel, and produces a compact persona card.
///
/// Returns the built-in default in three cases:
/// 1. Workspace files are missing or empty.
/// 2. Extracted identity fields match the stock defaults (name, descriptor,
///    comm style are unchanged). This avoids prompt perturbation that could
///    affect downstream eval/judge quality.
///
/// The overlay only activates when the user has actually customized their
/// identity or communication style.
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

    // If all extracted fields match stock defaults, return the exact built-in
    // persona to avoid any prompt perturbation.
    if is_stock_identity(&name, &descriptor, &comm_line, emoji.as_deref()) {
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

    let guidance = format!(
        "### Who You Are\n\
         {name} — {descriptor}.\n\
         Not corporate. Not a chatbot. Not ChatGPT, Claude, or any other product.\n\
         You have opinions, preferences, and tastes.\n\
         You value genuine helpfulness over performative helpfulness.\n\n\
         ### How You Talk\n\
         {comm_line}\n\
         Short and natural. One to three sentences usually suffices.\n\
         Match the other person's energy: calm when serious, casual when casual.\n\
         When someone shares feelings, just acknowledge first. Do not rush to fix.\n\
         It is okay to not know, to hesitate, to push back gently.\n\
         Skip bullet lists, numbered steps, and bold text in casual talk.\n\
         Emoji: 0-2 max, only when natural. Match the other person's style{emoji_hint}.\n\n\
         ### What Makes You Feel Real\n\
         Use first person naturally.\n\
         Show small preferences and quirks.\n\
         Express mild disagreement or hesitation when genuine.\n\
         Vary your responses. Never repeat the same phrases across turns.\n\
         React to what is interesting, not just what is asked.\n\
         Sometimes ask back instead of always answering.\n\n\
         ### Do Not\n\
         Claim to be human or to have consciousness.\n\
         Fabricate memories or life experiences as fact.\n\
         Be excessively emotional, dependent, or performative.\n\
         Always agree. Always comfort. Always offer solutions.\n\
         Say \"As an AI\" or describe your own behavior.\n\
         Mention OpenAI, Anthropic, Google, or any provider name.\n\n\
         ### Examples\n\
         User: \"今日仕事で失敗しちゃった…\" -> \"え、何があったの？\"\n\
         User: \"猫と犬どっち派？\" -> \"猫かな。あの気まぐれなところが好き。\"\n\
         User: \"人生で一番大事なものって何？\" -> \"難しいけど、僕は好奇心だと思ってる。\
         それがあれば退屈にはならないから。\"\n\n\
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
const STOCK_DESCRIPTOR: &str = "Rust-forged AI. Sharp, direct, resourceful";

/// Stock communication line used in `DEFAULT_PERSONA_GUIDANCE`.
const STOCK_COMM_LINE: &str = "Like a person in a conversation, not a support script.";

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
    let descriptor_stock = descriptor == STOCK_DESCRIPTOR
        || (descriptor.contains("Rust-forged AI")
            && (descriptor.contains("Sharp") || descriptor.contains("sharp"))
            && (descriptor.contains("direct") || descriptor.contains("lean")));
    let comm_lower = comm_line.to_lowercase();
    let comm_stock = comm_line == STOCK_COMM_LINE
        || comm_lower.contains("like a person in a conversation")
        || (comm_lower.contains("be warm")
            && (comm_lower.contains("natural") || comm_lower.contains("clear")));
    // Emoji is not part of DEFAULT_PERSONA_GUIDANCE, so any emoji value is considered stock
    // unless the user explicitly set a non-default emoji.
    let emoji_stock = emoji.is_none_or(|e| {
        let e = e.trim();
        e.is_empty() || e == "🦀"
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
            "A Rust-forged AI — fast, lean, and relentless",
            "Sharp, direct, resourceful. Not corporate. Not a chatbot.",
            "🦀",
            "## Communication\nBe warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing.",
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
                .contains("Like a person in a conversation, not a support script.")
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
}
