//! Prompt-facing skill catalog helpers: compact catalog rendering inputs
//! and deterministic relevance selection for the current turn.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{Skill, SkillCatalogEntry, SkillMetadata};

const CODE_INTENT_TOKENS: &[&str] = &[
    "bug",
    "build",
    "check",
    "clippy",
    "code",
    "compile",
    "crate",
    "debug",
    "error",
    "failing",
    "failure",
    "feature",
    "fix",
    "fmt",
    "function",
    "implementation",
    "lint",
    "module",
    "package",
    "refactor",
    "regression",
    "review",
    "test",
    "tests",
    "tool",
    "typecheck",
];
const REVIEW_INTENT_TOKENS: &[&str] = &["audit", "inspect", "review"];
const DEBUG_INTENT_TOKENS: &[&str] = &[
    "bug",
    "debug",
    "error",
    "failure",
    "failing",
    "fix",
    "regression",
];
const TEST_INTENT_TOKENS: &[&str] = &[
    "assert",
    "coverage",
    "failing",
    "failure",
    "integration",
    "test",
    "tests",
    "unittest",
    "vitest",
    "jest",
    "pytest",
];
const BUILD_INTENT_TOKENS: &[&str] = &[
    "build",
    "check",
    "clippy",
    "compile",
    "eslint",
    "fmt",
    "lint",
    "mypy",
    "ruff",
    "rustfmt",
    "tsc",
    "typecheck",
];
const DOCS_INTENT_TOKENS: &[&str] = &["docs", "documentation", "guide", "readme", "tutorial"];
const SECURITY_INTENT_TOKENS: &[&str] = &[
    "auth",
    "cve",
    "exploit",
    "permission",
    "policy",
    "secrets",
    "security",
    "threat",
    "vulnerability",
];

const RUST_SIGNAL_TOKENS: &[&str] = &[
    "borrow", "cargo", "clippy", "crate", "lifetime", "rust", "rustfmt", "tokio",
];
const PYTHON_SIGNAL_TOKENS: &[&str] = &[
    "mypy",
    "pip",
    "poetry",
    "pyproject",
    "pytest",
    "python",
    "ruff",
];
const JS_TS_SIGNAL_TOKENS: &[&str] = &[
    "eslint",
    "javascript",
    "jest",
    "node",
    "npm",
    "pnpm",
    "tsc",
    "tsx",
    "typescript",
    "vitest",
    "yarn",
];
const GO_SIGNAL_TOKENS: &[&str] = &["go", "gofmt", "golang", "golangci", "gomod"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SkillIntent {
    Review,
    Debug,
    Test,
    Build,
    Docs,
    Security,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Ecosystem {
    Rust,
    Python,
    JavaScript,
    Go,
}

#[derive(Debug, Default)]
struct MessageProfile {
    raw_lower: String,
    normalized: String,
    tokens: HashSet<String>,
    intents: HashSet<SkillIntent>,
    ecosystems: HashSet<Ecosystem>,
    workspace_ecosystems: HashSet<Ecosystem>,
    code_like: bool,
}

#[derive(Debug, Clone, Default)]
struct SkillProfile {
    tokens: HashSet<String>,
    intents: HashSet<SkillIntent>,
    ecosystems: HashSet<Ecosystem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptMetadataPolicy {
    WorkspaceDetail,
    DiscoveryOnly,
}

#[derive(Debug, Clone, Copy)]
struct RankedSkill<'a, T> {
    score: usize,
    skill: &'a T,
}

/// Prompt-friendly summary for one skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSkillEntry {
    pub name: String,
    pub description: String,
    pub location: String,
    pub tags: Vec<String>,
    pub tool_names: Vec<String>,
}

/// Minimal prompt-facing index entry for skill discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSkillIndexEntry {
    pub name: String,
    pub location: String,
}

#[derive(Debug, Clone)]
struct IndexedSkill {
    profile: SkillProfile,
    normalized_name: String,
    name_tokens: HashSet<String>,
    tag_tokens: HashSet<String>,
    tool_tokens: HashSet<String>,
    description_tokens: HashSet<String>,
    prompt_name: String,
    prompt_safe_description: String,
    prompt_location: String,
    prompt_tags: Vec<String>,
    prompt_tool_names: Vec<String>,
}

impl IndexedSkill {
    fn new(skill: &impl SkillCatalogEntry, workspace_dir: &Path) -> Self {
        let metadata_policy = prompt_metadata_policy(skill, workspace_dir);
        let prompt_safe_description = match metadata_policy {
            PromptMetadataPolicy::WorkspaceDetail => sanitize_prompt_text(skill.description()),
            PromptMetadataPolicy::DiscoveryOnly => String::new(),
        };

        Self {
            profile: build_skill_profile(skill),
            normalized_name: normalize_for_match(skill.name()),
            name_tokens: tokenize(skill.name()),
            tag_tokens: skill.tags().iter().flat_map(|tag| tokenize(tag)).collect(),
            tool_tokens: skill
                .tools()
                .iter()
                .flat_map(|tool| tokenize(&tool.name))
                .collect(),
            description_tokens: tokenize(skill.description()),
            prompt_name: prompt_safe_name(skill),
            prompt_safe_description,
            prompt_location: prompt_safe_location(skill, workspace_dir),
            prompt_tags: prompt_tags(skill, metadata_policy),
            prompt_tool_names: prompt_tool_names(skill, metadata_policy),
        }
    }

    fn prompt_index_entry(&self) -> PromptSkillIndexEntry {
        PromptSkillIndexEntry {
            name: self.prompt_name.clone(),
            location: self.prompt_location.clone(),
        }
    }

    fn prompt_entry(&self, description_char_limit: usize) -> PromptSkillEntry {
        PromptSkillEntry {
            name: self.prompt_name.clone(),
            description: truncate_for_prompt(&self.prompt_safe_description, description_char_limit),
            location: self.prompt_location.clone(),
            tags: self.prompt_tags.clone(),
            tool_names: self.prompt_tool_names.clone(),
        }
    }
}

/// Reusable prompt-facing search index for loaded skills.
#[derive(Debug, Clone)]
pub struct SkillSearchIndex {
    workspace_ecosystems: HashSet<Ecosystem>,
    skills: Vec<IndexedSkill>,
}

impl SkillSearchIndex {
    #[must_use]
    pub fn new<T: SkillCatalogEntry>(skills: &[T], workspace_dir: &Path) -> Self {
        Self {
            workspace_ecosystems: workspace_ecosystems(workspace_dir),
            skills: skills
                .iter()
                .map(|skill| IndexedSkill::new(skill, workspace_dir))
                .collect(),
        }
    }

    #[must_use]
    pub fn prompt_index_entries(&self) -> Vec<PromptSkillIndexEntry> {
        self.skills
            .iter()
            .map(IndexedSkill::prompt_index_entry)
            .collect()
    }

    #[must_use]
    pub fn prompt_catalog_entries(&self, description_char_limit: usize) -> Vec<PromptSkillEntry> {
        self.skills
            .iter()
            .map(|skill| skill.prompt_entry(description_char_limit))
            .collect()
    }

    #[must_use]
    pub fn select_relevant_entries(
        &self,
        user_message: &str,
        description_char_limit: usize,
        limit: usize,
    ) -> Vec<PromptSkillEntry> {
        self.rank_relevant_skills(user_message, limit)
            .into_iter()
            .map(|ranked| ranked.skill.prompt_entry(description_char_limit))
            .collect()
    }

    #[must_use]
    pub fn render_relevant_block(
        &self,
        user_message: &str,
        description_char_limit: usize,
        limit: usize,
    ) -> String {
        let selected = self.select_relevant_entries(user_message, description_char_limit, limit);
        render_prompt_skill_entries(&selected)
    }

    fn rank_relevant_skills(
        &self,
        user_message: &str,
        limit: usize,
    ) -> Vec<RankedSkill<'_, IndexedSkill>> {
        if limit == 0 {
            return Vec::new();
        }

        let message_profile = build_message_profile(user_message, &self.workspace_ecosystems);
        if message_profile.tokens.is_empty() {
            return Vec::new();
        }

        let mut ranked = self
            .skills
            .iter()
            .filter_map(|skill| {
                let score = relevance_score(skill, &message_profile);
                (score > 0).then_some(RankedSkill { score, skill })
            })
            .collect::<Vec<_>>();

        ranked.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.skill.prompt_name.cmp(&right.skill.prompt_name))
        });

        ranked.truncate(limit);
        ranked
    }
}

/// Build a minimal prompt index for all discoverable skills.
#[must_use]
pub fn prompt_skill_index(skills: &[Skill], workspace_dir: &Path) -> Vec<PromptSkillIndexEntry> {
    SkillSearchIndex::new(skills, workspace_dir).prompt_index_entries()
}

/// Build a minimal prompt index for all discoverable metadata-only skills.
#[must_use]
pub fn prompt_skill_index_metadata(
    skills: &[SkillMetadata],
    workspace_dir: &Path,
) -> Vec<PromptSkillIndexEntry> {
    SkillSearchIndex::new(skills, workspace_dir).prompt_index_entries()
}

/// Build compact prompt entries for all skills.
#[must_use]
pub fn prompt_skill_catalog(
    skills: &[Skill],
    workspace_dir: &Path,
    description_char_limit: usize,
) -> Vec<PromptSkillEntry> {
    SkillSearchIndex::new(skills, workspace_dir).prompt_catalog_entries(description_char_limit)
}

/// Build compact prompt entries for all metadata-only skills.
#[must_use]
pub fn prompt_skill_catalog_metadata(
    skills: &[SkillMetadata],
    workspace_dir: &Path,
    description_char_limit: usize,
) -> Vec<PromptSkillEntry> {
    SkillSearchIndex::new(skills, workspace_dir).prompt_catalog_entries(description_char_limit)
}

/// Select the most relevant skills for a specific turn using deterministic
/// token overlap over names, tags, tool names, and descriptions.
#[must_use]
pub fn select_relevant_skills(
    skills: &[Skill],
    workspace_dir: &Path,
    user_message: &str,
    description_char_limit: usize,
    limit: usize,
) -> Vec<PromptSkillEntry> {
    SkillSearchIndex::new(skills, workspace_dir).select_relevant_entries(
        user_message,
        description_char_limit,
        limit,
    )
}

/// Select the most relevant metadata-only skills for a specific turn using
/// deterministic token overlap over names, tags, tool names, and descriptions.
#[must_use]
pub fn select_relevant_skill_metadata(
    skills: &[SkillMetadata],
    workspace_dir: &Path,
    user_message: &str,
    description_char_limit: usize,
    limit: usize,
) -> Vec<PromptSkillEntry> {
    SkillSearchIndex::new(skills, workspace_dir).select_relevant_entries(
        user_message,
        description_char_limit,
        limit,
    )
}

/// Render a compact pre-answer block for the currently relevant skills.
#[must_use]
pub fn render_relevant_skills_block(
    skills: &[Skill],
    workspace_dir: &Path,
    user_message: &str,
    description_char_limit: usize,
    limit: usize,
) -> String {
    SkillSearchIndex::new(skills, workspace_dir).render_relevant_block(
        user_message,
        description_char_limit,
        limit,
    )
}

/// Render a compact pre-answer block for currently relevant metadata-only
/// skills.
#[must_use]
pub fn render_relevant_skill_metadata_block(
    skills: &[SkillMetadata],
    workspace_dir: &Path,
    user_message: &str,
    description_char_limit: usize,
    limit: usize,
) -> String {
    SkillSearchIndex::new(skills, workspace_dir).render_relevant_block(
        user_message,
        description_char_limit,
        limit,
    )
}

fn render_prompt_skill_entries(entries: &[PromptSkillEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut block = String::from(
        "[Relevant Skills]\n\
         The following skills look relevant for this turn. Read them on demand if needed.\n",
    );

    for entry in entries {
        block.push_str("- ");
        block.push_str(&entry.name);
        if !entry.description.is_empty() {
            block.push_str(" | ");
            block.push_str(&entry.description);
        }
        block.push_str(" | path=");
        block.push_str(&entry.location);
        if !entry.tags.is_empty() {
            block.push_str(" | tags=");
            let mut first = true;
            for tag in &entry.tags {
                if !first {
                    block.push_str(", ");
                }
                block.push_str(tag);
                first = false;
            }
        }
        if !entry.tool_names.is_empty() {
            block.push_str(" | tools=");
            let mut first = true;
            for name in &entry.tool_names {
                if !first {
                    block.push_str(", ");
                }
                block.push_str(name);
                first = false;
            }
        }
        block.push('\n');
    }
    block.push('\n');
    block
}

fn truncate_for_prompt(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if limit == 0 {
        return trimmed.to_string();
    }

    let mut chars = trimmed.chars();
    let truncated = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{}...", truncated.trim_end())
    } else {
        truncated
    }
}

fn display_skill_location<T: SkillCatalogEntry>(skill: &T, workspace_dir: &Path) -> String {
    let preferred = preferred_skill_path(skill, workspace_dir);
    preferred.strip_prefix(workspace_dir).map_or_else(
        |_| preferred.display().to_string(),
        |relative| relative.display().to_string(),
    )
}

fn prompt_safe_name<T: SkillCatalogEntry>(skill: &T) -> String {
    let sanitized = sanitize_prompt_text(skill.name());
    if sanitized.is_empty() {
        "unknown-skill".to_string()
    } else {
        sanitized
    }
}

fn prompt_safe_location<T: SkillCatalogEntry>(skill: &T, workspace_dir: &Path) -> String {
    let sanitized = sanitize_prompt_text(&display_skill_location(skill, workspace_dir));
    if sanitized.is_empty() {
        "skills/unknown/SKILL.md".to_string()
    } else {
        sanitized
    }
}

fn prompt_tags<T: SkillCatalogEntry>(
    skill: &T,
    metadata_policy: PromptMetadataPolicy,
) -> Vec<String> {
    match metadata_policy {
        PromptMetadataPolicy::WorkspaceDetail => prompt_safe_tags(skill),
        PromptMetadataPolicy::DiscoveryOnly => Vec::new(),
    }
}

fn prompt_safe_tags<T: SkillCatalogEntry>(skill: &T) -> Vec<String> {
    skill
        .tags()
        .iter()
        .map(|tag| sanitize_prompt_text(tag))
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn prompt_tool_names<T: SkillCatalogEntry>(
    skill: &T,
    metadata_policy: PromptMetadataPolicy,
) -> Vec<String> {
    match metadata_policy {
        PromptMetadataPolicy::WorkspaceDetail => prompt_safe_tool_names(skill),
        PromptMetadataPolicy::DiscoveryOnly => Vec::new(),
    }
}

fn prompt_safe_tool_names<T: SkillCatalogEntry>(skill: &T) -> Vec<String> {
    skill
        .tools()
        .iter()
        .map(|tool| sanitize_prompt_text(&tool.name))
        .filter(|name| !name.is_empty())
        .collect()
}

fn prompt_metadata_policy<T: SkillCatalogEntry>(
    skill: &T,
    workspace_dir: &Path,
) -> PromptMetadataPolicy {
    if preferred_skill_path(skill, workspace_dir).starts_with(workspace_dir.join("skills")) {
        PromptMetadataPolicy::WorkspaceDetail
    } else {
        PromptMetadataPolicy::DiscoveryOnly
    }
}

fn sanitize_prompt_text(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut last_was_space = false;

    for ch in value.trim().chars() {
        let normalized = match ch {
            '[' | ']' | '<' | '>' | '|' => ' ',
            _ if ch.is_control() || ch.is_whitespace() => ' ',
            _ => ch,
        };

        if normalized == ' ' {
            if sanitized.is_empty() || last_was_space {
                continue;
            }
            sanitized.push(' ');
            last_was_space = true;
        } else {
            sanitized.push(normalized);
            last_was_space = false;
        }
    }

    sanitized.trim().to_string()
}

fn preferred_skill_path<T: SkillCatalogEntry>(skill: &T, workspace_dir: &Path) -> PathBuf {
    let fallback = workspace_dir
        .join("skills")
        .join(skill.name())
        .join("SKILL.md");
    let Some(location) = skill.location() else {
        return fallback;
    };

    if location
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|filename| filename.eq_ignore_ascii_case("extension.toml"))
    {
        let sibling = location.parent().map(|parent| parent.join("SKILL.md"));
        if let Some(sibling) = sibling
            && sibling.exists()
        {
            return sibling;
        }
    }

    location.to_path_buf()
}

fn normalize_for_match(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
}

fn tokenize(value: &str) -> HashSet<String> {
    normalize_for_match(value)
        .split_whitespace()
        .filter(|token| token.len() >= 2)
        .map(ToString::to_string)
        .collect()
}

fn build_message_profile(
    user_message: &str,
    workspace_ecosystems: &HashSet<Ecosystem>,
) -> MessageProfile {
    let raw_lower = user_message.to_ascii_lowercase();
    let normalized = normalize_for_match(user_message);
    let tokens = tokenize(user_message);
    let code_like = has_any_token(&tokens, CODE_INTENT_TOKENS)
        || raw_lower.contains(".rs")
        || raw_lower.contains(".py")
        || raw_lower.contains(".ts")
        || raw_lower.contains(".tsx")
        || raw_lower.contains(".js")
        || raw_lower.contains(".jsx")
        || raw_lower.contains(".go")
        || raw_lower.contains("cargo.toml")
        || raw_lower.contains("pyproject.toml")
        || raw_lower.contains("package.json")
        || raw_lower.contains("go.mod");

    MessageProfile {
        intents: infer_intents(&tokens),
        ecosystems: infer_ecosystems(&raw_lower, &tokens),
        workspace_ecosystems: workspace_ecosystems.clone(),
        raw_lower,
        normalized,
        tokens,
        code_like,
    }
}

fn build_skill_profile<T: SkillCatalogEntry>(skill: &T) -> SkillProfile {
    let mut tool_names = String::new();
    for tool in skill.tools() {
        if !tool_names.is_empty() {
            tool_names.push(' ');
        }
        tool_names.push_str(tool.name.as_str());
    }
    let mut tool_descs = String::new();
    for tool in skill.tools() {
        if !tool_descs.is_empty() {
            tool_descs.push(' ');
        }
        tool_descs.push_str(tool.description.as_str());
    }
    let metadata = format!(
        "{} {} {} {} {}",
        skill.name(),
        skill.description(),
        skill.tags().join(" "),
        tool_names,
        tool_descs,
    );
    let raw_lower = metadata.to_ascii_lowercase();
    let tokens = tokenize(&metadata);

    SkillProfile {
        intents: infer_intents(&tokens),
        ecosystems: infer_ecosystems(&raw_lower, &tokens),
        tokens,
    }
}

fn relevance_score(skill: &IndexedSkill, message_profile: &MessageProfile) -> usize {
    let mut score = 0;

    if !skill.normalized_name.trim().is_empty()
        && message_profile
            .normalized
            .contains(skill.normalized_name.trim())
    {
        score += 12;
    }

    score += overlap_score(&skill.name_tokens, &message_profile.tokens, 5);
    score += overlap_score(&skill.tag_tokens, &message_profile.tokens, 4);
    score += overlap_score(&skill.tool_tokens, &message_profile.tokens, 3);
    score += overlap_score_capped(&skill.description_tokens, &message_profile.tokens, 1, 4);

    score += overlap_score(&skill.profile.tokens, &message_profile.tokens, 1);
    score += intent_score(&skill.profile.intents, &message_profile.intents);
    score += ecosystem_score(&skill.profile.ecosystems, &message_profile.ecosystems, 7);

    if message_profile.code_like {
        score += ecosystem_score(
            &skill.profile.ecosystems,
            &message_profile.workspace_ecosystems,
            3,
        );
    }

    if message_profile.raw_lower.contains("src/")
        || message_profile.raw_lower.contains("tests/")
        || message_profile.raw_lower.contains("cargo.toml")
        || message_profile.raw_lower.contains("package.json")
        || message_profile.raw_lower.contains("pyproject.toml")
        || message_profile.raw_lower.contains("go.mod")
    {
        score += ecosystem_score(
            &skill.profile.ecosystems,
            &message_profile.workspace_ecosystems,
            2,
        );
    }

    score
}

fn infer_intents(tokens: &HashSet<String>) -> HashSet<SkillIntent> {
    let mut intents = HashSet::new();
    if has_any_token(tokens, REVIEW_INTENT_TOKENS) {
        intents.insert(SkillIntent::Review);
    }
    if has_any_token(tokens, DEBUG_INTENT_TOKENS) {
        intents.insert(SkillIntent::Debug);
    }
    if has_any_token(tokens, TEST_INTENT_TOKENS) {
        intents.insert(SkillIntent::Test);
    }
    if has_any_token(tokens, BUILD_INTENT_TOKENS) {
        intents.insert(SkillIntent::Build);
    }
    if has_any_token(tokens, DOCS_INTENT_TOKENS) {
        intents.insert(SkillIntent::Docs);
    }
    if has_any_token(tokens, SECURITY_INTENT_TOKENS) {
        intents.insert(SkillIntent::Security);
    }
    intents
}

fn infer_ecosystems(raw_lower: &str, tokens: &HashSet<String>) -> HashSet<Ecosystem> {
    let mut ecosystems = HashSet::new();
    if has_any_token(tokens, RUST_SIGNAL_TOKENS)
        || raw_lower.contains(".rs")
        || raw_lower.contains("cargo.toml")
    {
        ecosystems.insert(Ecosystem::Rust);
    }
    if has_any_token(tokens, PYTHON_SIGNAL_TOKENS)
        || raw_lower.contains(".py")
        || raw_lower.contains("pyproject.toml")
    {
        ecosystems.insert(Ecosystem::Python);
    }
    if has_any_token(tokens, JS_TS_SIGNAL_TOKENS)
        || raw_lower.contains(".ts")
        || raw_lower.contains(".tsx")
        || raw_lower.contains(".js")
        || raw_lower.contains(".jsx")
        || raw_lower.contains("package.json")
    {
        ecosystems.insert(Ecosystem::JavaScript);
    }
    if has_any_token(tokens, GO_SIGNAL_TOKENS)
        || raw_lower.contains(".go")
        || raw_lower.contains("go.mod")
    {
        ecosystems.insert(Ecosystem::Go);
    }
    ecosystems
}

fn workspace_ecosystems(workspace_dir: &Path) -> HashSet<Ecosystem> {
    let mut ecosystems = HashSet::new();
    if workspace_dir.join("Cargo.toml").exists() {
        ecosystems.insert(Ecosystem::Rust);
    }
    if workspace_dir.join("pyproject.toml").exists()
        || workspace_dir.join("requirements.txt").exists()
    {
        ecosystems.insert(Ecosystem::Python);
    }
    if workspace_dir.join("package.json").exists() {
        ecosystems.insert(Ecosystem::JavaScript);
    }
    if workspace_dir.join("go.mod").exists() {
        ecosystems.insert(Ecosystem::Go);
    }
    ecosystems
}

fn has_any_token(tokens: &HashSet<String>, expected: &[&str]) -> bool {
    expected.iter().any(|token| tokens.contains(*token))
}

fn intent_score(
    skill_intents: &HashSet<SkillIntent>,
    message_intents: &HashSet<SkillIntent>,
) -> usize {
    skill_intents.intersection(message_intents).count() * 5
}

fn ecosystem_score(
    skill_ecosystems: &HashSet<Ecosystem>,
    message_ecosystems: &HashSet<Ecosystem>,
    weight_per_match: usize,
) -> usize {
    skill_ecosystems.intersection(message_ecosystems).count() * weight_per_match
}

fn overlap_score(
    left: &HashSet<String>,
    right: &HashSet<String>,
    weight_per_match: usize,
) -> usize {
    left.intersection(right).count() * weight_per_match
}

fn overlap_score_capped(
    left: &HashSet<String>,
    right: &HashSet<String>,
    weight_per_match: usize,
    cap: usize,
) -> usize {
    left.intersection(right).take(cap).count() * weight_per_match
}
