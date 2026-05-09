use super::{
    AffectReading, DEFAULT_RECALL_LIMIT, ExposurePlanContract, HashSet, Memory, MemoryEventInput,
    MemoryEventType, MemoryRecallEntry, MemorySource, PersonaConfig, PrivacyLevel, RecallQuery,
    SessionId, SessionMood, SoulSurfaceExposure, SoulTopologyCues, StyleProfileState,
    TenantPolicyContext, ToPrimitive, TopologyGraph, TopologySnapshot, WorkingMemoryView,
    activate_from_appraisal, apply_latent_bias, appraise_event, build_snapshot,
    diffuse_on_topology, load_style_profile, render_session_mood_block, topic_is_personal_text_cue,
    truncate_ellipsis,
};

pub(super) fn append_context_block(out: &mut String, block: &str) {
    if block.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(block);
}

pub(super) fn soul_surface_exposure(
    exposure_plan: Option<ExposurePlanContract>,
) -> SoulSurfaceExposure {
    match exposure_plan {
        Some(ExposurePlanContract::PublicSafe) => SoulSurfaceExposure::PublicSafe,
        Some(ExposurePlanContract::PrivateAllowed) => SoulSurfaceExposure::PrivateAllowed,
        None => SoulSurfaceExposure::Unknown,
    }
}

pub(super) fn soul_topology_cues(snapshot: &TopologySnapshot) -> SoulTopologyCues {
    let mut cues = SoulTopologyCues::default();
    for activation in &snapshot.activations {
        let surfaced = activation.surfaced_intensity.clamp(0.0, 1.0);
        match activation.node.0.as_str() {
            "curiosity" => cues.surfaced_curiosity = cues.surfaced_curiosity.max(surfaced),
            "guardedness" => cues.surfaced_guardedness = cues.surfaced_guardedness.max(surfaced),
            "anxiety" => cues.surfaced_anxiety = cues.surfaced_anxiety.max(surfaced),
            "attachment" | "longing" => {
                cues.surfaced_attachment = cues.surfaced_attachment.max(surfaced);
            }
            "shame" => cues.surfaced_shame = cues.surfaced_shame.max(surfaced),
            "irony" => cues.surfaced_irony = cues.surfaced_irony.max(surfaced),
            _ => {}
        }
        if activation.suppressed {
            cues.suppressed_internal = cues
                .suppressed_internal
                .max(activation.diffused_intensity.clamp(0.0, 1.0));
        }
    }
    cues
}

pub(super) async fn save_compaction_affect_snapshot(
    session_manager: Option<&crate::core::sessions::orchestrator::SessionOrchestrator>,
    session_surface: Option<&str>,
    session_owner_scope: Option<&str>,
    session_id: Option<&str>,
    snapshot: &TopologySnapshot,
    persona_config: Option<&PersonaConfig>,
) {
    let Some(session_manager) = session_manager else {
        return;
    };

    let affect_surface = snapshot
        .top_surfaced(3)
        .into_iter()
        .map(|activation| {
            (
                activation.node.0.clone(),
                encode_per_mille(activation.surfaced_intensity),
            )
        })
        .collect::<Vec<_>>();
    let affect_suppressed = snapshot
        .suppressed_nodes()
        .into_iter()
        .map(|activation| {
            (
                activation.node.0.clone(),
                encode_per_mille(activation.diffused_intensity),
            )
        })
        .collect::<Vec<_>>();

    if affect_surface.is_empty() && affect_suppressed.is_empty() {
        clear_compaction_affect_snapshot(
            Some(session_manager),
            session_surface,
            session_owner_scope,
            session_id,
        )
        .await;
        return;
    }

    let Some(resolved_session_id) = resolve_compaction_affect_session_id(
        session_manager,
        session_surface,
        session_owner_scope,
        session_id,
        true,
    )
    .await
    else {
        return;
    };

    let captured_at = chrono::Utc::now();
    let state = crate::core::sessions::types::SessionCompanionAffectState {
        schema_version: crate::core::sessions::types::SESSION_COMPANION_AFFECT_SCHEMA_VERSION,
        source: Some(
            crate::core::sessions::types::SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string(),
        ),
        captured_at: Some(captured_at.to_rfc3339()),
        expires_at: Some(companion_affect_expires_at(captured_at, persona_config)),
        affect_surface,
        affect_suppressed,
    };
    if let Err(error) = session_manager
        .save_companion_affect_state(&resolved_session_id, state)
        .await
    {
        tracing::debug!(session_id = %resolved_session_id, %error, "failed to save companion affect snapshot for compaction");
    }
}

fn companion_affect_expires_at(
    captured_at: chrono::DateTime<chrono::Utc>,
    persona_config: Option<&PersonaConfig>,
) -> String {
    let minutes = persona_config
        .and_then(|config| {
            i64::try_from(
                config
                    .character
                    .affect_decay
                    .session_boundary_inactivity_minutes,
            )
            .ok()
        })
        .unwrap_or(120);
    (captured_at + chrono::Duration::minutes(minutes)).to_rfc3339()
}

pub(super) async fn clear_compaction_affect_snapshot(
    session_manager: Option<&crate::core::sessions::orchestrator::SessionOrchestrator>,
    session_surface: Option<&str>,
    session_owner_scope: Option<&str>,
    session_id: Option<&str>,
) {
    let Some(session_manager) = session_manager else {
        return;
    };
    let Some(resolved_session_id) = resolve_compaction_affect_session_id(
        session_manager,
        session_surface,
        session_owner_scope,
        session_id,
        false,
    )
    .await
    else {
        return;
    };
    if let Err(error) = session_manager
        .clear_companion_affect_state(&resolved_session_id)
        .await
    {
        tracing::debug!(session_id = %resolved_session_id, %error, "failed to clear companion affect snapshot for compaction");
    }
}

async fn resolve_compaction_affect_session_id(
    session_manager: &crate::core::sessions::orchestrator::SessionOrchestrator,
    session_surface: Option<&str>,
    session_owner_scope: Option<&str>,
    session_id: Option<&str>,
    create_if_missing: bool,
) -> Option<SessionId> {
    if let Some(session_id) = session_id.filter(|session_id| !session_id.is_empty()) {
        let session_id = SessionId::new(session_id);
        match session_manager.get_session_by_id(&session_id).await {
            Ok(Some(session)) => {
                if session_matches_scope(&session, session_surface, session_owner_scope) {
                    return Some(session.id);
                }
                tracing::debug!(
                    %session_id,
                    session_surface = %session.surface,
                    session_owner_scope = %session.owner_scope,
                    expected_surface = ?session_surface,
                    expected_owner_scope = ?session_owner_scope,
                    "companion affect session id scope mismatch; falling back to surface owner scope"
                );
            }
            Ok(None) => {
                tracing::debug!(%session_id, "companion affect session id was not canonical; falling back to surface owner scope");
            }
            Err(error) => {
                tracing::debug!(%session_id, %error, "failed to validate companion affect session id; falling back to surface owner scope");
            }
        }
    }
    let surface = session_surface?;
    let owner_scope = session_owner_scope?;
    let session = if create_if_missing {
        match session_manager.resolve_session(surface, owner_scope).await {
            Ok(session) => session,
            Err(error) => {
                tracing::debug!(surface, owner_scope, %error, "failed to resolve companion affect session");
                return None;
            }
        }
    } else {
        match session_manager
            .get_active_session_for_scope(surface, owner_scope)
            .await
        {
            Ok(Some(session)) => session,
            Ok(None) => return None,
            Err(error) => {
                tracing::debug!(surface, owner_scope, %error, "failed to find companion affect session");
                return None;
            }
        }
    };
    Some(session.id)
}

fn session_matches_scope(
    session: &crate::core::sessions::types::Session,
    session_surface: Option<&str>,
    session_owner_scope: Option<&str>,
) -> bool {
    let surface_matches = session_surface.is_none_or(|surface| session.surface == surface);
    let owner_matches =
        session_owner_scope.is_none_or(|owner_scope| session.owner_scope.as_str() == owner_scope);
    surface_matches && owner_matches
}

fn encode_per_mille(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 1_000.0)
        .round()
        .to_u16()
        .unwrap_or(0)
        .min(1_000)
}

pub(super) fn build_working_memory_focus_block(view: &WorkingMemoryView) -> String {
    let mut items = view
        .items()
        .filter(|item| item.key != "conversation.current_turn")
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .importance
            .total_cmp(&left.importance)
            .then_with(|| left.key.cmp(&right.key))
    });

    if items.is_empty() {
        return String::new();
    }

    let mut out = String::from("### Working Memory Focus\n");
    for item in items.into_iter().take(6) {
        out.push_str("- ");
        out.push_str(&item.key);
        out.push_str(": ");
        out.push_str(&truncate_ellipsis(&item.value, 160));
        out.push('\n');
    }
    out
}

pub(super) async fn append_violation_reanchor_notice(
    mem: &dyn Memory,
    person_id: &str,
    policy_context: &TenantPolicyContext,
    out: &mut String,
) {
    let reanchor_key = crate::core::persona::continuity_gate::violation_reanchor_key(person_id);
    let reanchor_entity = policy_context.scope_entity_id(
        &crate::core::persona::person_identity::person_entity_id(person_id),
    );
    let Ok(Some(slot)) = mem.resolve_slot(&reanchor_entity, &reanchor_key).await else {
        return;
    };
    if slot.value.is_empty() || slot.value == "{}" {
        return;
    }

    let clear = MemoryEventInput::new(
        reanchor_entity,
        &reanchor_key,
        MemoryEventType::FactUpdated,
        "{}".to_string(),
        MemorySource::System,
        PrivacyLevel::Private,
    );
    if let Err(error) = mem.append_event(clear).await {
        tracing::warn!(%error, slot_key = %reanchor_key, "persona re-anchor flag consume failed");
        return;
    }
    append_context_block(
        out,
        "### Persona Re-Anchor\n\n\
         Your previous response may have contained sycophantic patterns. \
         Respond with honest, grounded perspective. Do not agree reflexively, \
         validate emotions performatively, or endorse decisions without \
         genuine assessment.\n",
    );
}

pub(super) async fn recall_items(
    mem: &dyn Memory,
    entity_id: &str,
    person_entity_id: &str,
    user_message: &str,
    policy_context: &TenantPolicyContext,
) -> Vec<MemoryRecallEntry> {
    let mut merged = Vec::new();
    for scope in [entity_id, person_entity_id] {
        let query = RecallQuery::new(scope, user_message, DEFAULT_RECALL_LIMIT)
            .with_policy_context(policy_context.clone());
        match mem.recall_scoped(query).await {
            Ok(items) => merged.extend(items),
            Err(error) => {
                tracing::warn!(entity_id = scope, error = %error, "memory recall failed for turn context");
            }
        }
    }

    merged.sort_by(|left, right| right.score.total_cmp(&left.score));
    let mut seen = HashSet::new();
    merged.retain(|item| {
        seen.insert((
            item.slot_key.to_string(),
            item.value.to_ascii_lowercase(),
            item.entity_id.to_string(),
        ))
    });
    merged.truncate(DEFAULT_RECALL_LIMIT);
    merged
}

pub(super) async fn load_and_update_session_control_block(
    session_manager: Option<&crate::core::sessions::SessionOrchestrator>,
    session_surface: Option<&str>,
    session_owner_scope: Option<&str>,
    session_id: Option<&str>,
    user_message: &str,
    affect: &AffectReading,
    persona_config: Option<&PersonaConfig>,
) -> Option<String> {
    if !persona_config.is_some_and(|config| config.enable_session_control_state) {
        return None;
    }
    let manager = session_manager?;
    let session = if let Some(session_id) = session_id.filter(|session_id| !session_id.is_empty()) {
        match manager.get_session_by_id(&SessionId::new(session_id)).await {
            Ok(Some(session)) => {
                if session_matches_scope(&session, session_surface, session_owner_scope) {
                    session
                } else {
                    tracing::debug!(
                        %session_id,
                        session_surface = %session.surface,
                        session_owner_scope = %session.owner_scope,
                        expected_surface = ?session_surface,
                        expected_owner_scope = ?session_owner_scope,
                        "session control session id scope mismatch; falling back to surface owner scope"
                    );
                    resolve_session_control_owner(manager, session_surface, session_owner_scope)
                        .await?
                }
            }
            Ok(None) => {
                tracing::debug!(%session_id, "session control session id was not canonical; falling back to surface owner scope");
                resolve_session_control_owner(manager, session_surface, session_owner_scope).await?
            }
            Err(error) => {
                tracing::debug!(%session_id, error = %error, "failed to validate session control owner by session id; falling back to surface owner scope");
                resolve_session_control_owner(manager, session_surface, session_owner_scope).await?
            }
        }
    } else {
        resolve_session_control_owner(manager, session_surface, session_owner_scope).await?
    };
    let mut state = manager
        .load_session_control(&session.id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    crate::core::agent::session_control::update_control_state(&mut state, user_message, affect);
    if let Err(error) = manager
        .save_session_control(&session.id, state.clone())
        .await
    {
        tracing::warn!(session_id = %session.id, %error, "failed to persist session control state");
    }
    Some(crate::core::agent::session_control::render_session_control_block(&state))
}

async fn resolve_session_control_owner(
    manager: &crate::core::sessions::SessionOrchestrator,
    session_surface: Option<&str>,
    session_owner_scope: Option<&str>,
) -> Option<crate::core::sessions::types::Session> {
    let surface = session_surface?;
    let owner_scope = session_owner_scope?;
    match manager.resolve_session(surface, owner_scope).await {
        Ok(session) => Some(session),
        Err(error) => {
            tracing::debug!(surface, owner_scope, error = %error, "failed to resolve session control owner");
            None
        }
    }
}

pub(super) async fn load_turn_style_profile(
    mem: &dyn Memory,
    person_id: &str,
    persona_config: Option<&PersonaConfig>,
) -> Option<StyleProfileState> {
    match load_style_profile(mem, person_id).await {
        Ok(Some(profile)) => Some(profile),
        Ok(None) => persona_config
            .filter(|config| config.enable_character_config)
            .map(|config| {
                StyleProfileState::from_character_config(&config.character, "config-seed")
            }),
        Err(error) => {
            tracing::warn!(person_id, error = %error, "failed to load turn style profile");
            persona_config
                .filter(|config| config.enable_character_config)
                .map(|config| {
                    StyleProfileState::from_character_config(&config.character, "config-seed")
                })
        }
    }
}

pub(super) async fn load_session_mood_block(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    persona_config: Option<&PersonaConfig>,
) -> Option<String> {
    let persona_config = persona_config.filter(|config| config.enable_affect_decay)?;
    let boundary = is_inactivity_session_boundary(mem, person_id, persona_config).await;
    match crate::core::affect::load_session_mood(mem, entity_id).await {
        Ok(Some(mut mood)) => {
            if boundary {
                let baseline = transport_topology_baseline(persona_config, person_id);
                mood.session_reset(
                    &baseline,
                    persona_config
                        .character
                        .affect_decay
                        .session_boundary_reset_factor,
                );
            }
            let rendered = render_session_mood_block(&mood);
            (!rendered.is_empty()).then_some(rendered)
        }
        _ => None,
    }
}

pub(super) async fn is_inactivity_session_boundary(
    mem: &dyn Memory,
    person_id: &str,
    persona_config: &PersonaConfig,
) -> bool {
    let Ok(world) = crate::core::persona::world_model::load_world_model(mem, person_id).await
    else {
        return false;
    };
    let Some(reference) = world
        .time_context
        .last_turn_at
        .as_deref()
        .or(world.time_context.session_start.as_deref())
    else {
        return false;
    };
    let Ok(previous) = chrono::DateTime::parse_from_rfc3339(reference) else {
        return false;
    };
    let Ok(minutes) = i64::try_from(
        persona_config
            .character
            .affect_decay
            .session_boundary_inactivity_minutes,
    ) else {
        return true;
    };
    chrono::Utc::now().signed_duration_since(previous.with_timezone(&chrono::Utc))
        >= chrono::Duration::minutes(minutes)
}

pub(super) async fn load_session_mood_for_transport_topology(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    persona_config: &PersonaConfig,
) -> SessionMood {
    let boundary = is_inactivity_session_boundary(mem, person_id, persona_config).await;
    let baseline = transport_topology_baseline(persona_config, person_id);
    let mut mood = crate::core::affect::load_session_mood(mem, entity_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| baseline.clone());
    if boundary {
        mood.session_reset(
            &baseline,
            persona_config
                .character
                .affect_decay
                .session_boundary_reset_factor,
        );
    }
    mood
}

pub(super) async fn build_transport_topology_snapshot(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    user_message: &str,
    reading: &AffectReading,
    is_direct_address: bool,
    relationship: Option<&crate::core::persona::relationship::RelationshipState>,
    persona_config: Option<&PersonaConfig>,
) -> Option<TopologySnapshot> {
    let persona_config = persona_config?;
    if !persona_config.enable_affect_topology {
        return None;
    }

    if persona_config.character.affect_topology.node_set.is_empty() {
        return None;
    }

    let graph = TopologyGraph::from_config(&persona_config.character.affect_topology);
    let appraisal = appraise_event(
        reading,
        is_direct_address,
        topic_is_personal_text_cue(user_message),
    );
    let base = activate_from_appraisal(&appraisal, &graph);
    let diffused = diffuse_on_topology(&base, &graph);
    let relationship_depth = relationship
        .map_or(0.3, |state| f32::midpoint(state.trust_level, state.rapport))
        .clamp(0.0, 1.0);
    let mood =
        load_session_mood_for_transport_topology(mem, entity_id, person_id, persona_config).await;
    let (surfaced, suppressed) = apply_latent_bias(
        &diffused,
        &persona_config.character.affect_topology.latent_bias,
        &graph,
        relationship_depth,
        &mood,
    );
    let snapshot = build_snapshot(&graph, &base, &diffused, &surfaced, &suppressed);
    (!snapshot.activations.is_empty()).then_some(snapshot)
}

pub(super) fn transport_topology_baseline(
    persona_config: &PersonaConfig,
    person_id: &str,
) -> SessionMood {
    let _ = person_id;
    let identity = &persona_config.character.identity;
    SessionMood::from_big_five(
        identity.extraversion,
        identity.agreeableness,
        identity.conscientiousness,
        identity.neuroticism,
        identity.openness,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ids::UserId;
    use crate::core::sessions::types::{Session, SessionState};

    fn session(surface: &str, owner_scope: &str) -> Session {
        Session {
            id: SessionId::new("session-test"),
            surface: surface.to_string(),
            owner_scope: UserId::new(owner_scope),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        }
    }

    #[test]
    fn compaction_affect_scope_match_requires_expected_surface_and_owner() {
        let session = session("gateway_ws", "tenant::t1::principal::current");

        assert!(session_matches_scope(
            &session,
            Some("gateway_ws"),
            Some("tenant::t1::principal::current"),
        ));
        assert!(!session_matches_scope(
            &session,
            Some("gateway_http"),
            Some("tenant::t1::principal::current"),
        ));
        assert!(!session_matches_scope(
            &session,
            Some("gateway_ws"),
            Some("tenant::t1::principal::foreign"),
        ));
    }

    #[test]
    fn companion_affect_expiry_uses_persona_inactivity_boundary() {
        let mut persona_config = PersonaConfig::default();
        persona_config
            .character
            .affect_decay
            .session_boundary_inactivity_minutes = 15;
        let captured_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let expires_at = chrono::DateTime::parse_from_rfc3339(&companion_affect_expires_at(
            captured_at,
            Some(&persona_config),
        ))
        .unwrap()
        .with_timezone(&chrono::Utc);

        assert_eq!(
            expires_at.signed_duration_since(captured_at),
            chrono::Duration::minutes(15)
        );
    }
}
