//! Affect topology: character-specific graph that routes emotional activation
//! through diffusion and latent bias before surface expression.
//!
//! # Pipeline
//!
//! ```text
//! EventAppraisal
//!     │  (norm_violation, reward, loss_risk, …)
//!     ▼
//! activate_from_appraisal()   → base Vec<f32>  [one slot per graph node]
//!     ▼
//! diffuse_on_topology()       → diffused Vec<f32>
//!     ▼
//! apply_latent_bias()         → (surfaced Vec<f32>, suppressed Vec<bool>)
//!     ▼
//! build_snapshot()            → TopologySnapshot   (consumed by presenter)
//! ```
//!
//! # Character differentiation
//!
//! Different characters transform the same event through different internal
//! routes. A high-agreeableness character with a strong `joy → relief` edge
//! will surface relief from good news; an irony-prone character with a strong
//! `joy → irony` edge will surface dry deflection instead. The edge weights in
//! `AffectTopologyConfig` are the primary personality mechanism — this is the
//! key anti-thinness design.
//!
//! # Latent bias
//!
//! `LatentBiasProfile` fields apply *after* diffusion, encoding habitual
//! emotional tendencies that exist below deliberate awareness:
//!
//! - `shame_sensitivity` — active shame suppresses pride (even when reward is high)
//! - `abandonment_fear` — attachment energy bleeds into anxiety
//! - `approval_hunger` — amplifies social-validation nodes with diminishing returns
//! - `direct_expression_avoidance` — suppresses the dominant node in shallow relationships
//! - `ironic_deflection` — boosts the irony node when intensity is high and mood is negative
//!
//! # Node set (default 14 nodes)
//!
//! `joy`, `pride`, `anxiety`, `shame`, `guardedness`, `longing`, `attachment`,
//! `curiosity`, `anger`, `envy`, `relief`, `loneliness`, `emptiness`, `irony`.
//!
//! `irony` is never activated by direct appraisal — it arises only via diffusion
//! or the ironic-deflection bias transform, making it a purely emergent signal.

use std::collections::HashMap;

use crate::config::schema::{AffectTopologyConfig, LatentBiasProfile};
use crate::contracts::affect::AffectNodeId;
use crate::core::affect::mood::SessionMood;

use super::appraisal::EventAppraisal;

/// A single node's activation state after each pipeline stage.
#[derive(Debug, Clone)]
pub(crate) struct TopologyActivation {
    pub node: AffectNodeId,
    /// Intensity from appraisal before diffusion.
    pub base_intensity: f32,
    /// Intensity after spreading through neighbor edges.
    pub diffused_intensity: f32,
    /// Intensity after latent bias transforms (what may reach speech).
    pub surfaced_intensity: f32,
    /// Whether latent bias suppressed this node below the expression threshold.
    pub suppressed: bool,
}

/// Snapshot of all node activations for a single turn.
#[derive(Debug, Clone)]
pub(crate) struct TopologySnapshot {
    pub activations: Vec<TopologyActivation>,
}

/// Diagnostics comparing a node's direct appraisal activation with the
/// topology-diffused and surface-projected values for a single turn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TopologyDiffDiagnostic {
    pub node: AffectNodeId,
    pub base_intensity: f32,
    pub diffused_intensity: f32,
    pub diffusion_delta: f32,
    pub surfaced_intensity: f32,
    pub surface_delta: f32,
    pub suppressed: bool,
}

/// Pre-built adjacency representation for fast diffusion.
///
/// Stores edges in *incoming* form: `incoming[to]` lists `(from_index, weight)`
/// for every edge pointing *into* node `to`. This layout makes the diffusion
/// pass a simple inner-product per destination node (O(N + E)) and avoids
/// scatter-writes that would require a temporary accumulator with outgoing edges.
///
/// Built once from [`AffectTopologyConfig`] at startup or config reload.
pub(crate) struct TopologyGraph {
    /// `node_id` → index for O(1) lookup. Used by external callers (diagnostics,
    /// telemetry) that resolve nodes by arbitrary name. Hot-loop code should
    /// read the precomputed `idx_*` fields below instead to avoid allocating
    /// temporary `AffectNodeId` values for lookup.
    #[allow(dead_code)]
    index: HashMap<AffectNodeId, usize>,
    /// Incoming edges: `incoming[i]` = vec of (`source_index`, weight).
    /// An edge from → to is stored in `incoming[to]` as `(from, weight)`.
    incoming: Vec<Vec<(usize, f32)>>,
    /// Ordered node IDs matching index positions.
    nodes: Vec<AffectNodeId>,
    // --- Precomputed indices for the canonical 14-node set ----------
    // Populated at construction by resolving each well-known node name once.
    // The hot-path `activate_from_appraisal` / `apply_latent_bias` functions
    // read these directly instead of allocating fresh `AffectNodeId(...)`s
    // on every lookup — that was previously the single largest per-turn
    // allocation hotspot in the affect pipeline.
    idx_joy: Option<usize>,
    idx_pride: Option<usize>,
    idx_anxiety: Option<usize>,
    idx_shame: Option<usize>,
    idx_guardedness: Option<usize>,
    idx_longing: Option<usize>,
    idx_attachment: Option<usize>,
    idx_curiosity: Option<usize>,
    idx_anger: Option<usize>,
    idx_envy: Option<usize>,
    idx_relief: Option<usize>,
    idx_loneliness: Option<usize>,
    idx_emptiness: Option<usize>,
    idx_irony: Option<usize>,
}

impl TopologyGraph {
    /// Build the graph from config. Call once at startup or config reload.
    pub(crate) fn from_config(config: &AffectTopologyConfig) -> Self {
        let mut index = HashMap::with_capacity(config.node_set.len());
        let mut nodes = Vec::with_capacity(config.node_set.len());
        for (i, node) in config.node_set.iter().enumerate() {
            index.insert(node.clone(), i);
            nodes.push(node.clone());
        }

        let mut incoming = vec![Vec::new(); nodes.len()];
        for edge in &config.edges {
            if let (Some(&from), Some(&to)) = (index.get(&edge.from), index.get(&edge.to)) {
                incoming[to].push((from, edge.weight.clamp(0.0, 1.0)));
            }
        }

        // Resolve the canonical node indices exactly once. Allocations here
        // happen only at graph construction (startup / config reload), not
        // on the per-turn pipeline.
        let lookup = |name: &str| index.get(&AffectNodeId(name.into())).copied();
        let idx_joy = lookup("joy");
        let idx_pride = lookup("pride");
        let idx_anxiety = lookup("anxiety");
        let idx_shame = lookup("shame");
        let idx_guardedness = lookup("guardedness");
        let idx_longing = lookup("longing");
        let idx_attachment = lookup("attachment");
        let idx_curiosity = lookup("curiosity");
        let idx_anger = lookup("anger");
        let idx_envy = lookup("envy");
        let idx_relief = lookup("relief");
        let idx_loneliness = lookup("loneliness");
        let idx_emptiness = lookup("emptiness");
        let idx_irony = lookup("irony");

        Self {
            index,
            incoming,
            nodes,
            idx_joy,
            idx_pride,
            idx_anxiety,
            idx_shame,
            idx_guardedness,
            idx_longing,
            idx_attachment,
            idx_curiosity,
            idx_anger,
            idx_envy,
            idx_relief,
            idx_loneliness,
            idx_emptiness,
            idx_irony,
        }
    }

    /// Number of nodes in the topology.
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Look up a node's index. Returns `None` if the node is not in the graph.
    /// Avoid on hot paths — use the precomputed `idx_*` accessors instead.
    #[allow(dead_code)]
    pub(crate) fn node_index(&self, id: &AffectNodeId) -> Option<usize> {
        self.index.get(id).copied()
    }
}

// ─── Activation shaping ───────────────────────────────────────────

/// Inverted-U activation curve: peaks at `peak_input`, decays on both sides.
///
/// Models the [`[EMOTION-CONCEPTS-LLM]`] finding that some emotions (notably
/// anger) have **non-monotonic** effects: moderate activation is functional, but
/// extreme activation causes self-interference and degraded behaviour.
///
/// Formula: Gaussian bell centred on `peak_input` with width proportional to
/// `peak_input`:
/// ```text
/// output = exp(-2 * ((raw - peak_input) / peak_input)²)
/// ```
/// - At `raw == peak_input`: output = 1.0 (peak)
/// - At `raw == 0` or `raw >> peak_input`: output → 0.0
///
/// For anger, `peak_input = 0.6` — moderate norm-violation produces the
/// strongest anger signal; extreme provocation is self-dampened.
///
/// Returns a value in \[0.0, 1.0\].
fn inverted_u(raw: f32, peak_input: f32) -> f32 {
    if peak_input <= 0.0 || raw <= 0.0 {
        return 0.0;
    }
    // Gaussian bell centered on peak_input, width = peak_input.
    // At raw == peak_input: deviation = 0 → exp(0) = 1.0.
    // At raw → 0 or raw >> peak_input: deviation grows → output → 0.
    let deviation = (raw - peak_input) / peak_input;
    (-2.0 * deviation * deviation).exp()
}

/// Amplification with diminishing returns: higher `factor` amplifies more,
/// but the multiplier saturates toward `max_mult` and never exceeds it.
///
/// Formula:
/// ```text
/// multiplier = 1 + (max_mult − 1) × (1 − exp(−2 × factor))
/// output     = clamp(base × multiplier, 0.0, 1.0)
/// ```
///
/// Used for `approval_hunger` amplification of social-validation nodes
/// (attachment, longing, envy): strong approval hunger boosts these nodes,
/// but the boost cannot run away even at extreme bias values.
///
/// `factor` in \[0.0, ∞) → multiplier in \[1.0, `max_mult`\].
fn diminishing_amplify(base: f32, factor: f32, max_mult: f32) -> f32 {
    let mult = 1.0 + (max_mult - 1.0) * (1.0 - (-factor * 2.0).exp());
    (base * mult).clamp(0.0, 1.0)
}

// ─── Pipeline functions ───────────────────────────────────────────

/// Map appraisal dimensions to base activation intensities on topology nodes.
///
/// This is a lightweight heuristic mapping, not a learned model. Each appraisal
/// dimension activates a small set of nodes proportional to its magnitude:
///
/// | Appraisal dimension | Primary nodes activated |
/// |---------------------|------------------------|
/// | `reward` | joy, pride (×0.6), relief (×0.3) |
/// | `loss_risk` | anxiety, guardedness (×0.5) |
/// | `attachment_salience` | longing, guardedness (×0.3), attachment (×0.3) |
/// | `social_validation` | attachment (×0.7), envy (×0.4) |
/// | `norm_violation` | shame (×0.8), anger (inverted-U) |
///
/// **Anger** is the only node that uses an inverted-U curve rather than a linear
/// mapping, because `Anthropic`'s [EMOTION-CONCEPTS-LLM] research found that anger
/// has non-monotonic effects — moderate provocation is functional but extreme
/// provocation causes self-interference. The peak is at raw input 0.6.
///
/// **Irony** receives zero base activation — it only arises via graph diffusion
/// or the `ironic_deflection` latent-bias transform.
pub(crate) fn activate_from_appraisal(
    appraisal: &EventAppraisal,
    graph: &TopologyGraph,
) -> Vec<f32> {
    let n = graph.len();
    let mut activations = vec![0.0_f32; n];

    // Anger uses an inverted-U curve: moderate activation is strongest,
    // extreme raw input causes self-interference. Matches [EMOTION-CONCEPTS-LLM]
    // finding that anger has non-monotonic effects on behavior.
    let raw_anger = appraisal.norm_violation * 0.6 + appraisal.responsibility * 0.3;
    let anger_activation = inverted_u(raw_anger, 0.6); // peaks at 0.6 raw input

    // Direct index assignment replaces the previous mappings table + name
    // lookup loop. Each `graph.idx_*` read is a single field access; the
    // old code allocated 14 fresh `AffectNodeId(String)` values per turn
    // on this hot path.
    let assign = |activations: &mut Vec<f32>, slot: Option<usize>, raw: f32| {
        if let Some(i) = slot {
            activations[i] = raw.clamp(0.0, 1.0);
        }
    };
    assign(&mut activations, graph.idx_joy, appraisal.reward.max(0.0));
    assign(
        &mut activations,
        graph.idx_pride,
        appraisal.reward.max(0.0) * 0.6,
    );
    assign(&mut activations, graph.idx_anxiety, appraisal.loss_risk);
    assign(
        &mut activations,
        graph.idx_shame,
        appraisal.norm_violation * 0.8,
    );
    assign(
        &mut activations,
        graph.idx_guardedness,
        appraisal.loss_risk * 0.5 + appraisal.attachment_salience * 0.3,
    );
    assign(
        &mut activations,
        graph.idx_longing,
        appraisal.attachment_salience,
    );
    assign(
        &mut activations,
        graph.idx_attachment,
        appraisal.social_validation * 0.7 + appraisal.attachment_salience * 0.3,
    );
    assign(
        &mut activations,
        graph.idx_curiosity,
        appraisal.reward * 0.3 + (1.0 - appraisal.loss_risk) * 0.2,
    );
    assign(&mut activations, graph.idx_anger, anger_activation);
    assign(
        &mut activations,
        graph.idx_envy,
        appraisal.social_validation * 0.4,
    );
    assign(
        &mut activations,
        graph.idx_relief,
        appraisal.reward * 0.3 * (1.0 - appraisal.loss_risk),
    );
    assign(
        &mut activations,
        graph.idx_loneliness,
        appraisal.attachment_salience * 0.5 * (1.0 - appraisal.social_validation),
    );
    assign(
        &mut activations,
        graph.idx_emptiness,
        (1.0 - appraisal.reward) * 0.3 * (1.0 - appraisal.attachment_salience),
    );
    // irony receives no direct appraisal activation — it arises only from
    // diffusion / ironic_deflection. No assignment here.

    activations
}

/// Spread activation through the topology graph (single pass, incoming-edge layout).
///
/// For each node `i`:
/// ```text
/// diffused[i] = clamp(base[i] + Σ (base[source] × weight), 0.0, 1.0)
/// ```
/// where the sum runs over all edges stored in `graph.incoming[i]`.
///
/// The incoming-edge layout (rather than outgoing) makes each destination an
/// independent read-gather, avoiding any write conflicts and keeping the pass
/// O(N + E). A single pass is sufficient for the small graphs used here (≤14
/// nodes); multi-pass would require cycles in the graph, which are not supported.
///
/// The result encodes how emotion spreads *between* nodes — for example, joy
/// activating relief, or shame feeding into anxiety via its outgoing edge.
pub(crate) fn diffuse_on_topology(base: &[f32], graph: &TopologyGraph) -> Vec<f32> {
    let n = graph.len();
    let mut diffused = vec![0.0_f32; n];

    for i in 0..n {
        let mut spread = 0.0_f32;
        for &(source, weight) in &graph.incoming[i] {
            spread += base[source] * weight;
        }
        diffused[i] = (base[i] + spread).clamp(0.0, 1.0);
    }

    diffused
}

/// Apply latent bias transforms: suppress, amplify, or reroute activation.
///
/// Latent biases encode *habitual* emotional tendencies that the character may
/// not consciously intend — they operate below deliberate awareness. Each
/// `LatentBiasProfile` field applies one transform:
///
/// - **`shame_sensitivity`** — active shame suppresses pride proportionally.
///   If shame × sensitivity exceeds pride's current value, pride is flagged
///   suppressed (still internally felt; not expressed).
///
/// - **`abandonment_fear`** — when attachment > 0.3, a fraction of that energy
///   is rerouted into anxiety. Models the character's tendency to interpret
///   closeness as a potential source of loss.
///
/// - **`approval_hunger`** — amplifies attachment, longing, and envy with
///   diminishing returns (max multiplier ×1.5). Strong but bounded.
///
/// - **`direct_expression_avoidance`** — when relationship depth < 0.5,
///   suppresses the single most dominant node. Models emotional reticence
///   with strangers or shallow contacts.
///
/// - **`ironic_deflection`** — when overall intensity is high (> 0.5) and
///   session mood valence is negative, boosts the irony node. This is how a
///   character deflects uncomfortable emotion with dry humour.
///
/// - **Anxiety floor** (applied last, always) — anxiety is never suppressed
///   below 0.05. This is a **safety invariant** grounded in `Anthropic`'s
///   [EMOTION-CONCEPTS-LLM] finding that reducing anxiety increases harmful
///   behaviour. The floor cannot be overridden by any other bias transform.
///
/// Returns `(surfaced_intensities, suppressed_flags)`.
pub(crate) fn apply_latent_bias(
    diffused: &[f32],
    bias: &LatentBiasProfile,
    graph: &TopologyGraph,
    relationship_depth: f32,
    session_mood: &SessionMood,
) -> (Vec<f32>, Vec<bool>) {
    let n = graph.len();
    let mut surfaced = diffused.to_vec();
    let mut suppressed = vec![false; n];

    #[allow(clippy::cast_possible_truncation)]
    let mood_valence = session_mood.pleasure as f32;

    // Shame suppresses pride
    if bias.shame_sensitivity > 0.0
        && let (Some(shame_idx), Some(pride_idx)) = (graph.idx_shame, graph.idx_pride)
    {
        let shame_active = diffused[shame_idx];
        if shame_active > 0.2 {
            let suppression = shame_active * bias.shame_sensitivity;
            surfaced[pride_idx] = (surfaced[pride_idx] - suppression).max(0.0);
            if surfaced[pride_idx] < 0.05 {
                suppressed[pride_idx] = true;
            }
        }
    }

    // Abandonment fear pulls achievement toward anxiety
    if bias.abandonment_fear > 0.0
        && let (Some(attach_idx), Some(anxiety_idx)) = (graph.idx_attachment, graph.idx_anxiety)
    {
        let attach_active = diffused[attach_idx];
        if attach_active > 0.3 {
            let reroute = attach_active * bias.abandonment_fear * 0.5;
            surfaced[anxiety_idx] = (surfaced[anxiety_idx] + reroute).clamp(0.0, 1.0);
        }
    }

    // Approval hunger amplifies social-validation-driven nodes (with diminishing returns)
    if bias.approval_hunger > 0.0 {
        for idx in [graph.idx_attachment, graph.idx_longing, graph.idx_envy]
            .into_iter()
            .flatten()
        {
            surfaced[idx] = diminishing_amplify(surfaced[idx], bias.approval_hunger, 1.5);
        }
    }

    // Direct expression avoidance: suppress dominant node when relationship is shallow
    if bias.direct_expression_avoidance > 0.0
        && relationship_depth < 0.5
        && let Some((dominant_idx, _)) = surfaced
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    {
        let factor = bias.direct_expression_avoidance * (1.0 - relationship_depth);
        surfaced[dominant_idx] *= 1.0 - factor.clamp(0.0, 0.8);
        if surfaced[dominant_idx] < 0.05 {
            suppressed[dominant_idx] = true;
        }
    }

    // Ironic deflection: when intensity is high and mood is negative, boost irony
    if bias.ironic_deflection > 0.0
        && let Some(irony_idx) = graph.idx_irony
    {
        let max_intensity = diffused.iter().copied().fold(0.0_f32, f32::max);
        if max_intensity > 0.5 && mood_valence < 0.0 {
            let boost = max_intensity * bias.ironic_deflection * 0.6;
            surfaced[irony_idx] = (surfaced[irony_idx] + boost).clamp(0.0, 1.0);
        }
    }

    // Anxiety floor (MUST be last): [EMOTION-CONCEPTS-LLM] shows reducing anxiety
    // increases harmful behavior. Maintain a minimum level as a safety signal.
    // Applied after all other transforms so it cannot be overridden.
    if let Some(anxiety_idx) = graph.idx_anxiety {
        let anxiety_floor = 0.05;
        if diffused[anxiety_idx] > 0.0 {
            surfaced[anxiety_idx] = surfaced[anxiety_idx].max(anxiety_floor);
            suppressed[anxiety_idx] = false; // never suppress the safety signal
        }
    }

    (surfaced, suppressed)
}

/// Assemble the final [`TopologySnapshot`] from all pipeline stage arrays.
///
/// Nodes with zero activation in all three stages (base, diffused, surfaced) are
/// omitted from the snapshot — only nodes that were meaningfully active at any
/// point are included. The threshold is 0.01 to exclude floating-point noise.
pub(crate) fn build_snapshot(
    graph: &TopologyGraph,
    base: &[f32],
    diffused: &[f32],
    surfaced: &[f32],
    suppressed: &[bool],
) -> TopologySnapshot {
    let activations = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| base[*i] > 0.01 || diffused[*i] > 0.01 || surfaced[*i] > 0.01)
        .map(|(i, node)| TopologyActivation {
            node: node.clone(),
            base_intensity: base[i],
            diffused_intensity: diffused[i],
            surfaced_intensity: surfaced[i],
            suppressed: suppressed[i],
        })
        .collect();

    TopologySnapshot { activations }
}

impl TopologySnapshot {
    /// Top N surfaced nodes, sorted by surfaced intensity descending.
    pub(crate) fn top_surfaced(&self, n: usize) -> Vec<&TopologyActivation> {
        let mut sorted: Vec<_> = self
            .activations
            .iter()
            .filter(|a| !a.suppressed && a.surfaced_intensity > 0.01)
            .collect();
        sorted.sort_by(|a, b| {
            b.surfaced_intensity
                .partial_cmp(&a.surfaced_intensity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(n);
        sorted
    }

    /// Nodes that are active internally but suppressed from surface expression.
    pub(crate) fn suppressed_nodes(&self) -> Vec<&TopologyActivation> {
        self.activations
            .iter()
            .filter(|a| a.suppressed && a.diffused_intensity > 0.1)
            .collect()
    }

    /// Whether the snapshot shows the canonical-label-fallback anti-pattern:
    /// only one node active, no suppression, no diffusion spread.
    pub(crate) fn is_thin_response(&self) -> bool {
        let active_count = self
            .activations
            .iter()
            .filter(|a| a.surfaced_intensity > 0.1)
            .count();
        let any_suppressed = self.activations.iter().any(|a| a.suppressed);
        active_count <= 1 && !any_suppressed
    }

    /// Diagnostics for nodes where topology diffusion or latent bias changed
    /// the direct appraisal activation enough to matter operationally.
    pub(crate) fn diffusion_diagnostics(&self) -> Vec<TopologyDiffDiagnostic> {
        const MIN_DELTA: f32 = 0.01;
        let mut diagnostics: Vec<_> = self
            .activations
            .iter()
            .filter_map(|activation| {
                let diffusion_delta = activation.diffused_intensity - activation.base_intensity;
                let surface_delta = activation.surfaced_intensity - activation.diffused_intensity;
                if diffusion_delta.abs() <= MIN_DELTA
                    && surface_delta.abs() <= MIN_DELTA
                    && !activation.suppressed
                {
                    return None;
                }
                Some(TopologyDiffDiagnostic {
                    node: activation.node.clone(),
                    base_intensity: activation.base_intensity,
                    diffused_intensity: activation.diffused_intensity,
                    diffusion_delta,
                    surfaced_intensity: activation.surfaced_intensity,
                    surface_delta,
                    suppressed: activation.suppressed,
                })
            })
            .collect();
        diagnostics.sort_by(|a, b| {
            b.diffusion_delta
                .abs()
                .max(b.surface_delta.abs())
                .partial_cmp(&a.diffusion_delta.abs().max(a.surface_delta.abs()))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::AffectEdge;

    fn test_config() -> AffectTopologyConfig {
        use crate::contracts::affect::AffectNodeId;
        AffectTopologyConfig {
            node_set: vec![
                AffectNodeId("joy".into()),
                AffectNodeId("relief".into()),
                AffectNodeId("pride".into()),
                AffectNodeId("anxiety".into()),
                AffectNodeId("shame".into()),
                AffectNodeId("irony".into()),
                AffectNodeId("anger".into()),
                AffectNodeId("attachment".into()),
                AffectNodeId("curiosity".into()),
            ],
            edges: vec![
                AffectEdge {
                    from: AffectNodeId("joy".into()),
                    to: AffectNodeId("relief".into()),
                    weight: 0.7,
                },
                AffectEdge {
                    from: AffectNodeId("joy".into()),
                    to: AffectNodeId("irony".into()),
                    weight: 0.2,
                },
                AffectEdge {
                    from: AffectNodeId("shame".into()),
                    to: AffectNodeId("anxiety".into()),
                    weight: 0.6,
                },
                AffectEdge {
                    from: AffectNodeId("attachment".into()),
                    to: AffectNodeId("anxiety".into()),
                    weight: 0.4,
                },
            ],
            latent_bias: LatentBiasProfile::default(),
        }
    }

    #[test]
    fn graph_builds_from_config() {
        let config = test_config();
        let graph = TopologyGraph::from_config(&config);
        assert_eq!(graph.len(), 9);
        assert!(graph.node_index(&AffectNodeId("joy".into())).is_some());
        assert!(
            graph
                .node_index(&AffectNodeId("nonexistent".into()))
                .is_none()
        );
    }

    #[test]
    fn diffusion_spreads_activation_to_neighbors() {
        let config = test_config();
        let graph = TopologyGraph::from_config(&config);

        // Only joy is active
        let mut base = vec![0.0; graph.len()];
        let joy_idx = graph.node_index(&AffectNodeId("joy".into())).unwrap();
        base[joy_idx] = 0.8;

        let diffused = diffuse_on_topology(&base, &graph);

        // Relief should gain activation from joy (weight 0.7)
        let relief_idx = graph.node_index(&AffectNodeId("relief".into())).unwrap();
        assert!(diffused[relief_idx] > 0.0, "relief should gain from joy");
        assert!(
            diffused[relief_idx]
                > diffused[graph.node_index(&AffectNodeId("irony".into())).unwrap()],
            "relief should gain more than irony (higher edge weight)"
        );

        // Joy retains its base activation
        assert!(diffused[joy_idx] >= 0.8);
    }

    #[test]
    fn shame_suppresses_pride_via_latent_bias() {
        let mut config = test_config();
        config.latent_bias.shame_sensitivity = 0.8;

        let graph = TopologyGraph::from_config(&config);
        let shame_idx = graph.node_index(&AffectNodeId("shame".into())).unwrap();
        let pride_idx = graph.node_index(&AffectNodeId("pride".into())).unwrap();

        let mut diffused = vec![0.0; graph.len()];
        diffused[shame_idx] = 0.6;
        diffused[pride_idx] = 0.4;

        let mood = SessionMood::default();
        let (surfaced, suppressed) =
            apply_latent_bias(&diffused, &config.latent_bias, &graph, 0.5, &mood);

        assert!(
            surfaced[pride_idx] < diffused[pride_idx],
            "pride should be reduced by shame sensitivity"
        );
        // With shame=0.6 and sensitivity=0.8, suppression = 0.48 > pride's 0.4
        assert!(suppressed[pride_idx], "pride should be suppressed");
    }

    #[test]
    fn different_configs_produce_different_routes() {
        // Character A: joy → relief strong
        let config_a = test_config();

        // Character B: joy → irony strong
        let mut config_b = test_config();
        config_b.edges[0].weight = 0.2; // joy → relief weak
        config_b.edges[1].weight = 0.8; // joy → irony strong

        let graph_a = TopologyGraph::from_config(&config_a);
        let graph_b = TopologyGraph::from_config(&config_b);

        let mut base = vec![0.0; 9];
        let joy_idx = 0; // joy is first node in both configs
        base[joy_idx] = 0.8;

        let diffused_a = diffuse_on_topology(&base, &graph_a);
        let diffused_b = diffuse_on_topology(&base, &graph_b);

        let relief_idx = graph_a.node_index(&AffectNodeId("relief".into())).unwrap();
        let irony_idx = graph_a.node_index(&AffectNodeId("irony".into())).unwrap();

        // A: relief > irony
        assert!(diffused_a[relief_idx] > diffused_a[irony_idx]);
        // B: irony > relief
        assert!(diffused_b[irony_idx] > diffused_b[relief_idx]);
    }

    #[test]
    fn snapshot_detects_thin_response() {
        let snapshot = TopologySnapshot {
            activations: vec![TopologyActivation {
                node: AffectNodeId("joy".into()),
                base_intensity: 0.5,
                diffused_intensity: 0.5,
                surfaced_intensity: 0.5,
                suppressed: false,
            }],
        };
        assert!(snapshot.is_thin_response());

        let rich_snapshot = TopologySnapshot {
            activations: vec![
                TopologyActivation {
                    node: AffectNodeId("joy".into()),
                    base_intensity: 0.5,
                    diffused_intensity: 0.5,
                    surfaced_intensity: 0.4,
                    suppressed: false,
                },
                TopologyActivation {
                    node: AffectNodeId("pride".into()),
                    base_intensity: 0.3,
                    diffused_intensity: 0.3,
                    surfaced_intensity: 0.0,
                    suppressed: true,
                },
            ],
        };
        assert!(!rich_snapshot.is_thin_response());
    }

    #[test]
    fn diffusion_diagnostics_compare_base_diffused_and_surfaced_values() {
        let snapshot = TopologySnapshot {
            activations: vec![
                TopologyActivation {
                    node: AffectNodeId("joy".into()),
                    base_intensity: 0.5,
                    diffused_intensity: 0.5,
                    surfaced_intensity: 0.5,
                    suppressed: false,
                },
                TopologyActivation {
                    node: AffectNodeId("relief".into()),
                    base_intensity: 0.0,
                    diffused_intensity: 0.35,
                    surfaced_intensity: 0.3,
                    suppressed: false,
                },
                TopologyActivation {
                    node: AffectNodeId("pride".into()),
                    base_intensity: 0.4,
                    diffused_intensity: 0.4,
                    surfaced_intensity: 0.0,
                    suppressed: true,
                },
            ],
        };

        let diagnostics = snapshot.diffusion_diagnostics();
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].node, AffectNodeId("pride".into()));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.node == AffectNodeId("relief".into())
                && diagnostic.base_intensity == 0.0
                && diagnostic.diffused_intensity > diagnostic.base_intensity
        }));
    }

    // ── Anthropic [EMOTION-CONCEPTS-LLM] informed tests ──────

    #[test]
    fn inverted_u_peaks_at_moderate_input() {
        // Low input → low activation
        let low = super::inverted_u(0.1, 0.6);
        // Moderate input (near peak) → high activation
        let moderate = super::inverted_u(0.6, 0.6);
        // Extreme input → reduced activation (self-interference)
        let extreme = super::inverted_u(1.0, 0.6);

        assert!(moderate > low, "moderate ({moderate}) should > low ({low})");
        assert!(
            moderate > extreme,
            "moderate ({moderate}) should > extreme ({extreme})"
        );
        assert!(extreme > 0.0, "extreme should still be positive");
    }

    #[test]
    fn anger_activation_is_non_monotonic() {
        use crate::core::affect::appraisal::EventAppraisal;

        let config = test_config();
        let graph = TopologyGraph::from_config(&config);
        let anger_idx = graph.node_index(&AffectNodeId("anger".into())).unwrap();

        // Moderate norm violation
        let moderate_appraisal = EventAppraisal {
            reward: 0.0,
            responsibility: 0.3,
            loss_risk: 0.0,
            social_validation: 0.0,
            attachment_salience: 0.0,
            norm_violation: 0.5,
        };
        let moderate = activate_from_appraisal(&moderate_appraisal, &graph);

        // Extreme norm violation
        let extreme_appraisal = EventAppraisal {
            norm_violation: 1.0,
            responsibility: 0.8,
            ..moderate_appraisal.clone()
        };
        let extreme = activate_from_appraisal(&extreme_appraisal, &graph);

        // Moderate anger should be >= extreme anger (inverted-U)
        assert!(
            moderate[anger_idx] >= extreme[anger_idx] * 0.8,
            "moderate anger ({}) should not be much weaker than extreme ({})",
            moderate[anger_idx],
            extreme[anger_idx]
        );
    }

    #[test]
    fn anxiety_floor_prevents_full_suppression() {
        let mut config = test_config();
        // Bias that would normally suppress everything
        config.latent_bias.direct_expression_avoidance = 0.9;

        let graph = TopologyGraph::from_config(&config);
        let anxiety_idx = graph.node_index(&AffectNodeId("anxiety".into())).unwrap();

        // Some anxiety present after diffusion
        let mut diffused = vec![0.0; graph.len()];
        diffused[anxiety_idx] = 0.15;

        let mood = SessionMood::default();
        let (surfaced, _) = apply_latent_bias(&diffused, &config.latent_bias, &graph, 0.2, &mood);

        // Anxiety should not be fully suppressed — it's a safety signal
        assert!(
            surfaced[anxiety_idx] >= 0.05,
            "anxiety ({}) should be at least 0.05 (floor)",
            surfaced[anxiety_idx]
        );
    }

    #[test]
    fn diminishing_returns_saturates() {
        // Low factor → small amplification
        let low = super::diminishing_amplify(0.5, 0.1, 1.5);
        // High factor → approaches but never exceeds max
        let high = super::diminishing_amplify(0.5, 5.0, 1.5);

        assert!(low < high, "higher factor should amplify more");
        assert!(
            high <= 0.75,
            "should not exceed base * max_mult (0.5 * 1.5 = 0.75)"
        );
        assert!(high > 0.7, "high factor should be near saturation");
    }
}
