//! Big Five (OCEAN) personality trait model.
//!
//! Adds continuous personality dimensions (Openness, Conscientiousness,
//! Extraversion, Agreeableness, Neuroticism) that influence response
//! style. Trait values live in [0.0, 1.0] with bounded updates of
//! +/-0.05 per interaction to prevent drastic personality shifts.
//!
//! References: [BIG-FIVE] Costa & `McCrae`, 1992; [LLM-PERSONALITY]
//! Serapio-García et al., 2023. See the public research reference index in the
//! docs site.

use serde::{Deserialize, Serialize};

use crate::core::persona::person_identity::{person_entity_id, sanitize_person_id};

/// OCEAN personality profile with bounded trait values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BigFiveProfile {
    /// Openness to experience: [0.0, 1.0]. High → creative, exploratory.
    pub openness: f64,
    /// Conscientiousness: [0.0, 1.0]. High → thorough, structured.
    pub conscientiousness: f64,
    /// Extraversion: [0.0, 1.0]. High → enthusiastic, engaging.
    pub extraversion: f64,
    /// Agreeableness: [0.0, 1.0]. High → supportive, validating.
    pub agreeableness: f64,
    /// Neuroticism: [0.0, 1.0]. High → cautious, risk-aware.
    pub neuroticism: f64,
}

impl Default for BigFiveProfile {
    fn default() -> Self {
        Self::from_character_config(&crate::config::schema::PersonaConfig::default())
    }
}

impl BigFiveProfile {
    pub(crate) fn from_character_config(config: &crate::config::schema::PersonaConfig) -> Self {
        let identity = &config.character.identity;
        Self {
            openness: clamp_trait(identity.openness),
            conscientiousness: clamp_trait(identity.conscientiousness),
            extraversion: clamp_trait(identity.extraversion),
            agreeableness: clamp_trait(identity.agreeableness),
            neuroticism: clamp_trait(identity.neuroticism),
        }
    }

    fn sanitized(self) -> Self {
        Self {
            openness: clamp_trait(self.openness),
            conscientiousness: clamp_trait(self.conscientiousness),
            extraversion: clamp_trait(self.extraversion),
            agreeableness: clamp_trait(self.agreeableness),
            neuroticism: clamp_trait(self.neuroticism),
        }
    }
}

fn clamp_trait(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn big_five_slot_key(person_id: &str) -> String {
    format!("persona/{}/big_five/v1", sanitize_person_id(person_id))
}

/// Load a persisted Big Five profile from memory, if one exists.
pub(crate) async fn load_big_five(
    mem: &dyn crate::core::memory::MemoryReader,
    person_id: &str,
) -> Option<BigFiveProfile> {
    let entity_id = person_entity_id(person_id);
    let slot = mem
        .resolve_slot(&entity_id, &big_five_slot_key(person_id))
        .await
        .ok()
        .flatten()?;
    serde_json::from_str::<BigFiveProfile>(&slot.value)
        .ok()
        .map(BigFiveProfile::sanitized)
}

/// Persist a Big Five profile to memory.
///
/// # Errors
///
/// Returns an error if serialization or the memory write fails.
pub(crate) async fn persist_big_five(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    profile: &BigFiveProfile,
) -> anyhow::Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        big_five_slot_key(person_id),
        crate::core::memory::MemoryEventType::FactUpdated,
        serde_json::to_string(&profile.clone().sanitized())?,
        0.85,
        0.6,
        "persona.big_five.update",
        "persona.big_five.writeback",
        None,
        person_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balanced_profile() -> BigFiveProfile {
        BigFiveProfile {
            openness: 0.5,
            conscientiousness: 0.5,
            extraversion: 0.5,
            agreeableness: 0.5,
            neuroticism: 0.5,
        }
    }

    #[test]
    fn balanced_profile_all_traits_at_midpoint() {
        let profile = balanced_profile();
        assert!((profile.openness - 0.5).abs() < f64::EPSILON);
        assert!((profile.conscientiousness - 0.5).abs() < f64::EPSILON);
        assert!((profile.extraversion - 0.5).abs() < f64::EPSILON);
        assert!((profile.agreeableness - 0.5).abs() < f64::EPSILON);
        assert!((profile.neuroticism - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn big_five_slot_key_is_person_scoped() {
        assert_eq!(
            big_five_slot_key("person:test"),
            "persona/person_test__h6a651757900b/big_five/v1"
        );
    }

    #[test]
    fn default_profile_uses_character_config_seeds() {
        let profile = BigFiveProfile::default();
        assert!((profile.openness - 0.50).abs() < f64::EPSILON);
        assert!((profile.conscientiousness - 0.50).abs() < f64::EPSILON);
        assert!((profile.extraversion - 0.50).abs() < f64::EPSILON);
        assert!((profile.agreeableness - 0.50).abs() < f64::EPSILON);
        assert!((profile.neuroticism - 0.50).abs() < f64::EPSILON);
    }

    #[test]
    fn from_character_config_clamps_and_seeds_traits() {
        let mut config = crate::config::schema::PersonaConfig::default();
        config.character.identity.openness = 1.2;
        config.character.identity.conscientiousness = 0.65;
        config.character.identity.extraversion = -0.2;
        config.character.identity.agreeableness = 0.75;
        config.character.identity.neuroticism = 0.25;

        let profile = BigFiveProfile::from_character_config(&config);
        assert!((profile.openness - 1.0).abs() < f64::EPSILON);
        assert!((profile.conscientiousness - 0.65).abs() < f64::EPSILON);
        assert!((profile.extraversion - 0.0).abs() < f64::EPSILON);
        assert!((profile.agreeableness - 0.75).abs() < f64::EPSILON);
        assert!((profile.neuroticism - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn render_guidance_balanced_is_empty() {
        let profile = balanced_profile();
        assert!(
            crate::core::persona::presenter::render_guidance_block(&profile).is_empty(),
            "balanced profile should produce no guidance"
        );
    }

    #[test]
    fn render_guidance_high_traits() {
        let profile = BigFiveProfile {
            openness: 0.8,
            conscientiousness: 0.8,
            extraversion: 0.8,
            agreeableness: 0.8,
            neuroticism: 0.8,
        };
        let block = crate::core::persona::presenter::render_guidance_block(&profile);
        assert!(block.contains("[Personality Guidance]"));
        assert!(block.contains("creative"));
        assert!(block.contains("thorough"));
        assert!(block.contains("enthusiastic"));
        assert!(block.contains("supportive"));
        assert!(block.contains("cautious"));
    }

    #[test]
    fn render_guidance_low_traits() {
        let profile = BigFiveProfile {
            openness: 0.2,
            conscientiousness: 0.2,
            extraversion: 0.2,
            agreeableness: 0.2,
            neuroticism: 0.2,
        };
        let block = crate::core::persona::presenter::render_guidance_block(&profile);
        assert!(block.contains("conventional"));
        assert!(block.contains("concise"));
        assert!(block.contains("reserved"));
        assert!(block.contains("candid"));
        assert!(block.contains("confidence"));
    }

    #[test]
    fn serde_round_trip() {
        let profile = BigFiveProfile {
            openness: 0.7,
            conscientiousness: 0.3,
            extraversion: 0.9,
            agreeableness: 0.1,
            neuroticism: 0.5,
        };
        let json = serde_json::to_string(&profile).expect("serialize");
        let loaded: BigFiveProfile = serde_json::from_str(&json).expect("deserialize");
        assert!((loaded.openness - 0.7).abs() < f64::EPSILON);
        assert!((loaded.agreeableness - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn persisted_big_five_payloads_are_sanitized_after_deserialize() {
        let profile: BigFiveProfile = serde_json::from_str(
            r#"{
                "openness": 2.0,
                "conscientiousness": -1.0,
                "extraversion": 0.5,
                "agreeableness": 1.5,
                "neuroticism": 0.25
            }"#,
        )
        .unwrap();

        let profile = profile.sanitized();
        assert_eq!(profile.openness, 1.0);
        assert_eq!(profile.conscientiousness, 0.0);
        assert_eq!(profile.extraversion, 0.5);
        assert_eq!(profile.agreeableness, 1.0);
        assert_eq!(profile.neuroticism, 0.25);
    }

    #[test]
    fn big_five_nan_traits_sanitize_to_zero() {
        let profile = BigFiveProfile {
            openness: f64::NAN,
            conscientiousness: f64::INFINITY,
            extraversion: f64::NEG_INFINITY,
            agreeableness: 0.5,
            neuroticism: f64::NAN,
        }
        .sanitized();

        assert_eq!(profile.openness, 0.0);
        assert_eq!(profile.conscientiousness, 1.0);
        assert_eq!(profile.extraversion, 0.0);
        assert_eq!(profile.agreeableness, 0.5);
        assert_eq!(profile.neuroticism, 0.0);
    }
}
