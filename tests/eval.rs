pub(crate) use asterel::core::eval::behavioral;

#[path = "eval/preference_coherence.rs"]
mod preference_coherence;

#[path = "eval/adversarial_personality.rs"]
mod adversarial_personality;

#[path = "eval/identity_continuity.rs"]
mod identity_continuity;

#[path = "eval/counterfactual_quality.rs"]
mod counterfactual_quality;

#[path = "eval/tom_levels.rs"]
mod tom_levels;

#[path = "eval/character_runtime.rs"]
mod character_runtime;

#[path = "eval/harness_ablation.rs"]
mod harness_ablation;
