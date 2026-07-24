use super::turn_enrichment_io::{
    append_context_block, append_violation_reanchor_notice, build_transport_topology_snapshot,
    build_working_memory_focus_block, clear_compaction_affect_snapshot,
    load_and_update_session_control_block, load_session_mood_block, load_turn_style_profile,
    recall_items, save_compaction_affect_snapshot, soul_surface_exposure, soul_topology_cues,
};
use super::{
    AffectLabel, AffectReading, DEFAULT_RECALL_MIN_CONFIDENCE, ExposurePlanContract, JudgmentCore,
    PersonaConfig, PersonaContextInput, PreTurnEnrichment, PreTurnInput, RuleBasedDetector,
    SoulIdentityCues, SoulPressureInput, SoulRecallExposure, ToPrimitive, affect_to_style_delta,
    build_companion_grounding_augmentation_with_privacy, build_prompt_self_contract,
    compile_turn_contract, derive_soul_pressure, derive_soul_pressure_with_topology,
    infer_user_model, load_relationship_for_entity, load_user_profile_for_entity, person_entity_id,
    render_behavior_selection_block, render_judgment_core_turn_block,
    render_relationship_context_block, render_response_style_block, render_self_contract_block,
    render_soul_pressure_block, render_style_guidance, render_system_prompt_from_contract,
    render_tone_guidance, render_topology_block, render_user_model_block,
    render_user_profile_block,
};

/// Extract the affect confidence as a `f32` intensity value clamped to [0, 1].
///
/// Falls back to `1.0` if the numeric conversion fails.
#[must_use]
pub fn affect_intensity(affect: &AffectReading) -> f32 {
    affect
        .confidence
        .get()
        .clamp(0.0, 1.0)
        .to_f32()
        .unwrap_or(1.0)
}

/// Enrich the system prompt and derive the adjusted temperature for a turn.
///
/// Steps performed in order:
/// 1. Detect affect from the user message.
/// 2. Apply the affect-to-style temperature delta.
/// 3. Build the judgment core block from the workspace config.
/// 4. Assemble the persona context (relationship, recall, user profile, tone
///    guidance, response style) if any relevant data is available.
/// 5. Concatenate everything into the final system prompt.
pub async fn enrich_pre_turn(input: &PreTurnInput<'_>) -> PreTurnEnrichment {
    let affect = RuleBasedDetector::new().detect(input.user_message);
    let style_delta = affect_to_style_delta(affect.label);
    let temperature = (input.base_temperature + style_delta.temperature_delta).clamp(0.0, 2.0);
    let decision_core_block = render_judgment_core_turn_block(
        &JudgmentCore::from_workspace(input.workspace_dir),
        input.user_message,
    );

    if affect.label != AffectLabel::Neutral {
        tracing::debug!(
            entity_id = input.entity_id,
            person_id = input.person_id,
            label = ?affect.label,
            confidence = affect.confidence.get(),
            temperature_delta = style_delta.temperature_delta,
            "turn affect detected"
        );
    }

    let persona_context = build_persona_context(PersonaContextInput {
        mem: input.mem,
        entity_id: input.entity_id,
        person_id: input.person_id,
        user_message: input.user_message,
        affect: &affect,
        policy_context: input.policy_context,
        recall_min_confidence: input.recall_min_confidence,
        persona_config: input.persona_config,
        session_manager: input.session_manager,
        session_surface: input.session_surface,
        is_direct_address: input.is_direct_address,
        session_owner_scope: input.session_owner_scope,
        session_id: input.session_id,
        working_memory: input.working_memory,
        exposure_plan: input.exposure_plan,
    })
    .await;

    let contract = compile_turn_contract(
        input.base_prompt,
        input.policy_section,
        persona_context.as_deref(),
        &decision_core_block,
        temperature,
    );
    let system_prompt = render_system_prompt_from_contract(&contract);

    PreTurnEnrichment {
        contract,
        system_prompt,
        temperature,
        affect,
    }
}

#[allow(clippy::too_many_lines)]
async fn build_persona_context(input: PersonaContextInput<'_>) -> Option<String> {
    let mut out = String::with_capacity(512);
    let person_entity_id = input
        .policy_context
        .scope_entity_id(&person_entity_id(input.person_id));

    let self_contract_enabled = input
        .persona_config
        .is_some_and(|cfg| cfg.enable_self_contract);
    if self_contract_enabled {
        let contract =
            build_prompt_self_contract(input.mem, input.person_id, input.persona_config).await;
        append_context_block(&mut out, &render_self_contract_block(&contract));
    }

    append_violation_reanchor_notice(input.mem, input.person_id, input.policy_context, &mut out)
        .await;

    if let Some(block) = load_and_update_session_control_block(
        input.session_manager,
        input.session_surface,
        input.session_owner_scope,
        input.session_id,
        input.user_message,
        input.affect,
        input.persona_config,
    )
    .await
    {
        append_context_block(&mut out, &block);
    }

    if let Some(mood_block) = load_session_mood_block(
        input.mem,
        input.entity_id,
        input.person_id,
        input.persona_config,
    )
    .await
    {
        append_context_block(&mut out, &mood_block);
    }

    let relationship = match load_relationship_for_entity(
        input.mem,
        &person_entity_id,
        input.person_id,
    )
    .await
    {
        Ok(Some(relationship)) => {
            append_context_block(&mut out, &render_relationship_context_block(&relationship));
            Some(relationship)
        }
        Ok(None) => None,
        Err(error) => {
            tracing::debug!(person_id = input.person_id, error = %error, "no relationship state for turn");
            None
        }
    };

    let recall_items = recall_items(
        input.mem,
        input.entity_id,
        &person_entity_id,
        input.user_message,
        input.policy_context,
    )
    .await;
    let dialogue_act =
        crate::core::persona::continuity_v2::classify_dialogue_act(input.user_message);
    let user_model = infer_user_model(input.user_message, input.affect, &recall_items);
    append_context_block(&mut out, &render_user_model_block(&user_model));

    let min_confidence = input
        .recall_min_confidence
        .unwrap_or(DEFAULT_RECALL_MIN_CONFIDENCE);
    let grounding_augmentation = build_companion_grounding_augmentation_with_privacy(
        input.user_message,
        &recall_items,
        min_confidence,
        !matches!(input.exposure_plan, Some(ExposurePlanContract::PublicSafe)),
    );
    let topology_snapshot = build_transport_topology_snapshot(
        input.mem,
        input.entity_id,
        input.person_id,
        input.user_message,
        input.affect,
        input.is_direct_address,
        relationship.as_ref(),
        input.persona_config,
    )
    .await;
    if let Some(snapshot) = &topology_snapshot {
        save_compaction_affect_snapshot(
            input.session_manager,
            input.session_surface,
            input.session_owner_scope,
            input.session_id,
            snapshot,
            input.persona_config,
        )
        .await;
    } else {
        clear_compaction_affect_snapshot(
            input.session_manager,
            input.session_surface,
            input.session_owner_scope,
            input.session_id,
        )
        .await;
    }

    let soul_pressure = if let Some(config) = input
        .persona_config
        .filter(|config| config.enable_soul_pressure)
    {
        let exposure = grounding_augmentation.exposure;
        let identity = SoulIdentityCues {
            soul_root_sentence: &config.character.identity.soul_root_sentence,
            values: &config.character.identity.values,
            negative_identity: &config.character.identity.negative_identity,
        };
        let soul_pressure_input = SoulPressureInput {
            user_message: input.user_message,
            identity,
            affect: input.affect,
            dialogue_act,
            user_model: &user_model,
            relationship: relationship.as_ref(),
            recall_exposure: SoulRecallExposure {
                public: exposure.public_visible,
                private: exposure.private_internal,
                secret: exposure.secret_suppressed,
            },
            surface_exposure: soul_surface_exposure(input.exposure_plan),
        };
        let topology_cues = topology_snapshot
            .as_ref()
            .map(soul_topology_cues)
            .filter(|cues| cues.has_signal());
        let soul_pressure = topology_cues.map_or_else(
            || derive_soul_pressure(soul_pressure_input),
            |cues| derive_soul_pressure_with_topology(soul_pressure_input, Some(cues)),
        );
        append_context_block(&mut out, &render_soul_pressure_block(&soul_pressure));
        Some(soul_pressure)
    } else {
        None
    };

    let style_profile =
        load_turn_style_profile(input.mem, input.person_id, input.persona_config).await;
    if let Some(profile) = style_profile.as_ref() {
        append_context_block(&mut out, &render_style_guidance(profile));
    }

    if input
        .persona_config
        .is_some_and(|config| config.enable_behavior_selector)
    {
        let fallback_persona = PersonaConfig::default();
        let persona_config = input.persona_config.unwrap_or(&fallback_persona);
        let big_five = crate::core::persona::big_five::load_big_five(input.mem, input.person_id)
            .await
            .unwrap_or_else(|| {
                crate::core::persona::big_five::BigFiveProfile::from_character_config(
                    persona_config,
                )
            });
        let default_tiers = crate::config::schema::RelationshipTierConfig::default();
        let default_activation = crate::config::schema::TraitActivationConfig::default();
        let relationship_ref = relationship.as_ref();
        let tiers = input.persona_config.map_or(&default_tiers, |config| {
            &config.character.relationship_tiers
        });
        let activation = input.persona_config.map_or(&default_activation, |config| {
            &config.character.trait_activation
        });
        let selection = crate::core::persona::select_behavior(
            input.affect,
            relationship_ref,
            dialogue_act,
            &big_five,
            style_profile.as_ref(),
            &user_model,
            tiers,
            activation,
            persona_config.enable_trait_activation,
            soul_pressure.as_ref(),
        );
        append_context_block(&mut out, &render_behavior_selection_block(&selection));
    }

    match load_user_profile_for_entity(input.mem, &person_entity_id, input.person_id).await {
        Ok(profile) if !profile.is_empty() => {
            append_context_block(&mut out, &render_user_profile_block(&profile));
        }
        Err(error) => {
            tracing::debug!(person_id = input.person_id, error = %error, "failed to load user profile");
        }
        _ => {}
    }

    let tone_block = render_tone_guidance(input.affect, relationship.as_ref(), input.user_message);
    if !tone_block.is_empty() {
        append_context_block(&mut out, &tone_block);
    }

    if let Some(topology_block) = topology_snapshot.as_ref().map(render_topology_block) {
        append_context_block(&mut out, &topology_block);
    }

    if !grounding_augmentation.exposure.is_empty() {
        tracing::debug!(
            public_visible = grounding_augmentation.exposure.public_visible,
            private_internal = grounding_augmentation.exposure.private_internal,
            secret_suppressed = grounding_augmentation.exposure.secret_suppressed,
            "transport pre-turn grounding exposure projected"
        );
    }
    let grounding_block = grounding_augmentation.block;
    if !grounding_block.is_empty() {
        tracing::debug!(
            entity_id = input.entity_id,
            count = recall_items.len(),
            "injected companion grounding context"
        );
        append_context_block(&mut out, &grounding_block);
    }

    if let Some(working_memory) = input.working_memory {
        append_context_block(&mut out, &build_working_memory_focus_block(working_memory));
    }

    append_context_block(&mut out, &render_response_style_block(input.user_message));

    if out.is_empty() {
        return None;
    }

    Some(out)
}
