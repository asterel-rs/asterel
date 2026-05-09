//! Two-layer skill structure: Core (built-in) and Community (user-installed).
//!
//! Skills are classified into layers that determine loading priority and
//! override semantics. Core skills always take precedence unless explicitly
//! marked as overridable.

use serde::{Deserialize, Serialize};

use crate::config::SkillSource;

use super::types::Skill;

// ---------------------------------------------------------------------------
// SkillLayer enum
// ---------------------------------------------------------------------------

/// Classifies a skill into one of two priority layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SkillLayer {
    /// Built-in skills that are always loaded with highest priority.
    Core,
    /// User-installed or community-provided skills with lower priority.
    Community,
}

// ---------------------------------------------------------------------------
// LayeredSkill wrapper
// ---------------------------------------------------------------------------

/// A [`Skill`] annotated with its layer and override policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayeredSkill {
    /// The underlying skill definition.
    pub skill: Skill,
    /// Which layer this skill belongs to.
    pub layer: SkillLayer,
    /// Whether a community skill may shadow this entry.
    /// Always `false` for core skills by default.
    pub override_allowed: bool,
}

// ---------------------------------------------------------------------------
// SkillLayerRegistry
// ---------------------------------------------------------------------------

/// Registry that maintains core and community skill lists and resolves
/// conflicts using layer priority.
#[derive(Debug, Default)]
pub struct SkillLayerRegistry {
    core_skills: Vec<LayeredSkill>,
    community_skills: Vec<LayeredSkill>,
}

impl SkillLayerRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a skill in the **Core** layer (`override_allowed = false`).
    pub fn register_core(&mut self, skill: Skill) {
        self.core_skills.push(LayeredSkill {
            skill,
            layer: SkillLayer::Core,
            override_allowed: false,
        });
    }

    /// Register a skill in the **Community** layer (`override_allowed = true`).
    pub fn register_community(&mut self, skill: Skill) {
        self.community_skills.push(LayeredSkill {
            skill,
            layer: SkillLayer::Community,
            override_allowed: true,
        });
    }

    /// Register a pre-built [`LayeredSkill`] directly, routing it to the
    /// appropriate internal list based on its layer.
    pub fn register_layered(&mut self, layered: LayeredSkill) {
        match layered.layer {
            SkillLayer::Core => self.core_skills.push(layered),
            SkillLayer::Community => self.community_skills.push(layered),
        }
    }

    /// Resolve the full skill list with core-first ordering.
    ///
    /// When a community skill has the same name as a core skill, the community
    /// entry is hidden **unless** the core skill has `override_allowed = true`.
    #[must_use]
    pub fn resolve(&self) -> Vec<&LayeredSkill> {
        let mut result: Vec<&LayeredSkill> = Vec::new();

        // Core skills always appear first.
        for ls in &self.core_skills {
            result.push(ls);
        }

        // Community skills are included only when they don't collide with a
        // non-overridable core skill.
        for ls in &self.community_skills {
            let dominated = self
                .core_skills
                .iter()
                .any(|core| core.skill.name == ls.skill.name && !core.override_allowed);
            if !dominated {
                result.push(ls);
            }
        }

        result
    }

    /// Return the core layer slice.
    #[must_use]
    pub fn core_skills(&self) -> &[LayeredSkill] {
        &self.core_skills
    }

    /// Return the community layer slice.
    #[must_use]
    pub fn community_skills(&self) -> &[LayeredSkill] {
        &self.community_skills
    }

    /// Look up a skill by name. Core layer is checked first.
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<&LayeredSkill> {
        self.core_skills
            .iter()
            .find(|ls| ls.skill.name == name)
            .or_else(|| {
                self.community_skills
                    .iter()
                    .find(|ls| ls.skill.name == name)
            })
    }

    /// Remove a community skill by name.
    ///
    /// Returns `true` if a skill was removed. Core skills cannot be removed
    /// through this method.
    pub fn remove_community(&mut self, name: &str) -> bool {
        let before = self.community_skills.len();
        self.community_skills.retain(|ls| ls.skill.name != name);
        self.community_skills.len() < before
    }

    /// Check whether a skill with the given name exists in the core layer.
    #[must_use]
    pub fn is_core(&self, name: &str) -> bool {
        self.core_skills.iter().any(|ls| ls.skill.name == name)
    }

    /// Total number of skills across both layers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.core_skills.len() + self.community_skills.len()
    }

    /// Whether the registry contains no skills.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.core_skills.is_empty() && self.community_skills.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Source classification helper
// ---------------------------------------------------------------------------

/// Map a [`SkillSource`] to the corresponding [`SkillLayer`].
#[must_use]
pub fn classify_skill_source(source: &SkillSource) -> SkillLayer {
    match source {
        SkillSource::Workspace => SkillLayer::Core,
        SkillSource::ExtraDirs | SkillSource::OpenSkills => SkillLayer::Community,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal `Skill` with a given name.
    fn stub_skill(name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("{name} description"),
            version: "0.1.0".to_string(),
            author: None,
            tags: Vec::new(),
            tools: Vec::new(),
            prompts: Vec::new(),
            location: None,
        }
    }

    #[test]
    fn empty_registry() {
        let reg = SkillLayerRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.resolve().is_empty());
        assert!(reg.core_skills().is_empty());
        assert!(reg.community_skills().is_empty());
        assert!(reg.find_by_name("nope").is_none());
    }

    #[test]
    fn register_core_only() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_core(stub_skill("alpha"));
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
        assert_eq!(reg.core_skills().len(), 1);
        assert!(reg.community_skills().is_empty());
        assert!(reg.is_core("alpha"));
    }

    #[test]
    fn register_community_only() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_community(stub_skill("beta"));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.community_skills().len(), 1);
        assert!(reg.core_skills().is_empty());
        assert!(!reg.is_core("beta"));
    }

    #[test]
    fn resolve_core_first_ordering() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_community(stub_skill("comm"));
        reg.register_core(stub_skill("core"));

        let resolved = reg.resolve();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].skill.name, "core");
        assert_eq!(resolved[0].layer, SkillLayer::Core);
        assert_eq!(resolved[1].skill.name, "comm");
        assert_eq!(resolved[1].layer, SkillLayer::Community);
    }

    #[test]
    fn core_wins_over_community_on_name_conflict() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_core(stub_skill("shared"));
        reg.register_community(stub_skill("shared"));

        let resolved = reg.resolve();
        // Community duplicate should be hidden.
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].layer, SkillLayer::Core);
    }

    #[test]
    fn community_hidden_by_non_overridable_core() {
        let mut reg = SkillLayerRegistry::new();
        // Default core: override_allowed = false
        reg.register_core(stub_skill("x"));
        reg.register_community(stub_skill("x"));

        assert_eq!(reg.resolve().len(), 1);
        assert_eq!(reg.resolve()[0].layer, SkillLayer::Core);
    }

    #[test]
    fn community_visible_when_core_allows_override() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_layered(LayeredSkill {
            skill: stub_skill("x"),
            layer: SkillLayer::Core,
            override_allowed: true,
        });
        reg.register_community(stub_skill("x"));

        // Both should appear because the core skill allows override.
        let resolved = reg.resolve();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn remove_community_works() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_community(stub_skill("bye"));
        assert!(reg.remove_community("bye"));
        assert!(reg.is_empty());
    }

    #[test]
    fn remove_community_does_not_affect_core() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_core(stub_skill("stay"));
        // Attempting to remove from community returns false.
        assert!(!reg.remove_community("stay"));
        assert_eq!(reg.len(), 1);
        assert!(reg.is_core("stay"));
    }

    #[test]
    fn remove_community_returns_false_for_missing() {
        let mut reg = SkillLayerRegistry::new();
        assert!(!reg.remove_community("ghost"));
    }

    #[test]
    fn find_by_name_prefers_core() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_core(stub_skill("dup"));
        reg.register_community(stub_skill("dup"));

        let found = reg.find_by_name("dup").unwrap();
        assert_eq!(found.layer, SkillLayer::Core);
    }

    #[test]
    fn find_by_name_falls_back_to_community() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_community(stub_skill("only_comm"));

        let found = reg.find_by_name("only_comm").unwrap();
        assert_eq!(found.layer, SkillLayer::Community);
    }

    #[test]
    fn classify_skill_source_mapping() {
        assert_eq!(
            classify_skill_source(&SkillSource::Workspace),
            SkillLayer::Core
        );
        assert_eq!(
            classify_skill_source(&SkillSource::ExtraDirs),
            SkillLayer::Community
        );
        assert_eq!(
            classify_skill_source(&SkillSource::OpenSkills),
            SkillLayer::Community
        );
    }

    #[test]
    fn is_core_checks() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_core(stub_skill("c"));
        reg.register_community(stub_skill("d"));
        assert!(reg.is_core("c"));
        assert!(!reg.is_core("d"));
        assert!(!reg.is_core("e"));
    }

    #[test]
    fn serde_roundtrip_skill_layer() {
        let layer = SkillLayer::Core;
        let json = serde_json::to_string(&layer).unwrap();
        assert_eq!(json, "\"core\"");
        let back: SkillLayer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SkillLayer::Core);

        let layer2 = SkillLayer::Community;
        let json2 = serde_json::to_string(&layer2).unwrap();
        assert_eq!(json2, "\"community\"");
        let back2: SkillLayer = serde_json::from_str(&json2).unwrap();
        assert_eq!(back2, SkillLayer::Community);
    }

    #[test]
    fn serde_roundtrip_layered_skill() {
        let ls = LayeredSkill {
            skill: stub_skill("roundtrip"),
            layer: SkillLayer::Community,
            override_allowed: true,
        };
        let json = serde_json::to_string(&ls).unwrap();
        let back: LayeredSkill = serde_json::from_str(&json).unwrap();
        assert_eq!(back.skill.name, "roundtrip");
        assert_eq!(back.layer, SkillLayer::Community);
        assert!(back.override_allowed);
    }

    #[test]
    fn register_layered_routes_correctly() {
        let mut reg = SkillLayerRegistry::new();
        reg.register_layered(LayeredSkill {
            skill: stub_skill("a"),
            layer: SkillLayer::Core,
            override_allowed: false,
        });
        reg.register_layered(LayeredSkill {
            skill: stub_skill("b"),
            layer: SkillLayer::Community,
            override_allowed: true,
        });
        assert_eq!(reg.core_skills().len(), 1);
        assert_eq!(reg.community_skills().len(), 1);
    }

    #[test]
    fn skill_layer_ordering() {
        // Core < Community in the derived Ord
        assert!(SkillLayer::Core < SkillLayer::Community);
    }
}
