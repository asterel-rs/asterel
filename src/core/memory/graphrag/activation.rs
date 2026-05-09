//! Graph activation spreading via Personalized `PageRank` (PPR).
//!
//! The companion's knowledge graph is a directed, weighted graph where nodes
//! are graph entities (persons, slots, events, concepts) and edges are typed
//! relations (`has_slot`, `recorded_event`, `supersedes`, etc.).
//!
//! During recall, the top-5 scoring retrieval units seed a PPR computation
//! that spreads activation through the graph. The resulting per-node scores
//! are blended back into the retrieval ranking (see
//! [`crate::core::memory::reranking::blend_with_ppr`]).
//!
//! ## Key types
//!
//! - [`GraphSnapshot`] — CSR-formatted immutable view of the knowledge graph,
//!   loaded from `PostgreSQL` once and cached per entity.
//! - [`PprQuery`] — parameters for a single PPR run (seeds, damping factor α,
//!   convergence threshold ε, iteration cap).
//! - [`PprResult`] — per-entity activation score after convergence.
//! - [`GraphActivationCache`] — process-global `Arc<RwLock<…>>` cache of
//!   snapshots; invalidated by writes via `invalidate`.
//!
//! ## PPR algorithm
//!
//! At each iteration:
//! ```text
//! activation[t+1] = α · seed + (1 − α) · M · activation[t]
//! ```
//! where `M` is the column-stochastic transition matrix (row-normalised by
//! out-edge weight sum). Convergence is declared when the L∞ delta < ε.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::contracts::ids::EntityId;

/// Immutable CSR-format snapshot of a single entity's knowledge graph.
///
/// Loaded once from `PostgreSQL` and cached in [`GraphActivationCache`].
/// The adjacency is stored as compressed sparse rows (`offsets` +
/// `neighbors` + `weights`) for cache-friendly PPR iteration.
#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    node_index: HashMap<EntityId, u32>,
    node_ids: Vec<EntityId>,
    offsets: Vec<u32>,
    neighbors: Vec<u32>,
    weights: Vec<f32>,
    loaded_at: DateTime<Utc>,
}

/// Parameters for a single PPR computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PprQuery {
    /// Seed nodes with initial activation weights (unnormalized).
    pub seeds: Vec<(EntityId, f32)>,
    /// Restart / teleport probability (damping factor). Default: 0.15.
    pub alpha: f32,
    /// L∞ convergence threshold. Default: 1e-6.
    pub epsilon: f32,
    /// Maximum power-iteration steps before forced termination. Default: 100.
    pub max_iters: usize,
    /// Maximum number of results to return.
    pub top_k: usize,
}

/// A single entity's PPR activation score after convergence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PprResult {
    pub entity_id: EntityId,
    /// Final steady-state activation (proportional to the entity's
    /// structural proximity to the seed set).
    pub activation_score: f32,
}

/// Process-global cache of per-entity graph snapshots.
///
/// Snapshots are loaded lazily on first recall and invalidated whenever
/// a write touches the owner entity's graph projection (see
/// `repository_write::append_event_impl`).
#[derive(Debug, Default)]
pub struct GraphActivationCache {
    snapshots: Arc<RwLock<HashMap<EntityId, Arc<GraphSnapshot>>>>,
}

impl Default for PprQuery {
    fn default() -> Self {
        Self {
            seeds: Vec::new(),
            alpha: 0.15,
            epsilon: 1e-6,
            max_iters: 100,
            top_k: 10,
        }
    }
}

impl GraphActivationCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "postgres")]
    #[allow(clippy::missing_errors_doc)]
    pub async fn get_or_load(
        &self,
        owner_id: &EntityId,
        pool: &sqlx_core::pool::Pool<sqlx_postgres::Postgres>,
    ) -> Result<Arc<GraphSnapshot>> {
        if let Some(snapshot) = self.snapshots.read().await.get(owner_id).cloned() {
            return Ok(snapshot);
        }

        let snapshot = Arc::new(GraphSnapshot::load_from_db(owner_id, pool).await?);
        let mut guard = self.snapshots.write().await;
        let cached = guard
            .entry(owner_id.clone())
            .or_insert_with(|| Arc::clone(&snapshot));
        Ok(Arc::clone(cached))
    }

    pub async fn invalidate(&self, owner_id: &EntityId) {
        self.snapshots.write().await.remove(owner_id);
    }
}

impl GraphSnapshot {
    #[cfg(feature = "postgres")]
    #[allow(clippy::missing_errors_doc, clippy::cast_possible_truncation)]
    pub async fn load_from_db(
        owner_id: &EntityId,
        pool: &sqlx_core::pool::Pool<sqlx_postgres::Postgres>,
    ) -> Result<Self> {
        use sqlx_core::query::query;
        use sqlx_core::row::Row;

        let node_rows = query(
            "SELECT graph_entity_id
             FROM graph_entities
             WHERE owner_entity_id = $1;",
        )
        .bind(owner_id.as_str())
        .fetch_all(pool)
        .await?;

        let mut node_ids: Vec<EntityId> = node_rows
            .into_iter()
            .map(|row| EntityId::new(row.get::<String, _>("graph_entity_id")))
            .collect();

        let edge_rows = query(
            "SELECT from_entity_id, to_entity_id, COALESCE(confidence, weight, 1.0) AS edge_weight
             FROM graph_edges
             WHERE owner_entity_id = $1
               AND valid_until IS NULL
               AND (valid_from IS NULL OR valid_from <= now());",
        )
        .bind(owner_id.as_str())
        .fetch_all(pool)
        .await?;

        let mut seen: HashSet<EntityId> = node_ids.iter().cloned().collect();
        let mut edges = Vec::with_capacity(edge_rows.len());
        for row in edge_rows {
            let from_entity_id = EntityId::new(row.get::<String, _>("from_entity_id"));
            let to_entity_id = EntityId::new(row.get::<String, _>("to_entity_id"));
            let edge_weight = row.get::<f64, _>("edge_weight") as f32;

            if seen.insert(from_entity_id.clone()) {
                node_ids.push(from_entity_id.clone());
            }
            if seen.insert(to_entity_id.clone()) {
                node_ids.push(to_entity_id.clone());
            }
            edges.push((from_entity_id, to_entity_id, edge_weight));
        }

        Ok(Self::from_edge_list(node_ids, &edges))
    }

    #[must_use]
    pub fn run_ppr(&self, query: &PprQuery) -> Vec<PprResult> {
        let node_count = self.node_ids.len();
        if node_count == 0 || query.top_k == 0 || query.seeds.is_empty() {
            return Vec::new();
        }

        let Some(seed) = self.normalize_seeds(query, node_count) else {
            return Vec::new();
        };

        let activation = self.iterate_ppr(&seed, query, node_count);
        self.collect_top_k_results(activation, query.top_k)
    }

    /// Builds and normalises the seed vector from the query's weighted seed entities.
    ///
    /// Returns `None` when no seed has positive weight or all seeds are unknown nodes,
    /// which signals that PPR should short-circuit with an empty result.
    fn normalize_seeds(&self, query: &PprQuery, node_count: usize) -> Option<Vec<f32>> {
        let mut seed = vec![0.0_f32; node_count];
        let mut seed_sum = 0.0_f32;
        for (entity_id, weight) in &query.seeds {
            if *weight <= 0.0 {
                continue;
            }
            let Some(index) = self.node_idx(entity_id) else {
                continue;
            };
            seed[index as usize] += *weight;
            seed_sum += *weight;
        }

        if seed_sum <= 0.0 {
            return None;
        }

        for value in &mut seed {
            *value /= seed_sum;
        }

        Some(seed)
    }

    /// Runs the PPR iteration loop until convergence or the iteration cap is reached.
    ///
    /// Returns the final per-node activation scores.
    fn iterate_ppr(&self, seed: &[f32], query: &PprQuery, node_count: usize) -> Vec<f32> {
        let alpha = query.alpha.clamp(0.0, 1.0);
        let epsilon = query.epsilon.max(0.0);
        let max_iters = query.max_iters.max(1);

        let mut activation = seed.to_vec();
        let mut next = vec![0.0_f32; node_count];

        for _ in 0..max_iters {
            next.copy_from_slice(seed);
            for value in &mut next {
                *value *= alpha;
            }

            for (source, source_activation) in
                activation.iter().copied().enumerate().take(node_count)
            {
                let start = self.offsets[source] as usize;
                let end = self.offsets[source + 1] as usize;
                if start == end {
                    continue;
                }

                let weight_sum: f32 = self.weights[start..end].iter().copied().sum();
                if weight_sum <= 0.0 {
                    continue;
                }

                let propagated = (1.0 - alpha) * source_activation;
                if propagated == 0.0 {
                    continue;
                }

                for edge_idx in start..end {
                    let neighbor = self.neighbors[edge_idx] as usize;
                    let normalized_weight = self.weights[edge_idx] / weight_sum;
                    next[neighbor] += propagated * normalized_weight;
                }
            }

            let delta = activation
                .iter()
                .zip(&next)
                .map(|(old, new)| (new - old).abs())
                .fold(0.0_f32, f32::max);
            activation.copy_from_slice(&next);

            if delta < epsilon {
                break;
            }
        }

        activation
    }

    /// Filters zero-score nodes, maps indices back to entity IDs, sorts by descending
    /// activation score (ties broken by entity ID), and truncates to `top_k`.
    fn collect_top_k_results(&self, activation: Vec<f32>, top_k: usize) -> Vec<PprResult> {
        let mut results: Vec<PprResult> = activation
            .into_iter()
            .enumerate()
            .filter(|(_, score)| *score > 0.0)
            .map(|(index, activation_score)| PprResult {
                entity_id: self.node_ids[index].clone(),
                activation_score,
            })
            .collect();

        results.sort_by(|left, right| {
            match right
                .activation_score
                .partial_cmp(&left.activation_score)
                .unwrap_or(Ordering::Equal)
            {
                Ordering::Equal => left.entity_id.cmp(&right.entity_id),
                ordering => ordering,
            }
        });
        results.truncate(top_k);
        results
    }

    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn from_edge_list(
        mut node_ids: Vec<EntityId>,
        edges: &[(EntityId, EntityId, f32)],
    ) -> Self {
        let mut known_nodes: HashSet<EntityId> = node_ids.iter().cloned().collect();
        for (from_entity_id, to_entity_id, _) in edges {
            if known_nodes.insert(from_entity_id.clone()) {
                node_ids.push(from_entity_id.clone());
            }
            if known_nodes.insert(to_entity_id.clone()) {
                node_ids.push(to_entity_id.clone());
            }
        }

        let node_index: HashMap<EntityId, u32> = node_ids
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, entity_id)| (entity_id, index as u32))
            .collect();

        let mut adjacency: Vec<Vec<(u32, f32)>> = vec![Vec::new(); node_ids.len()];
        for (from_entity_id, to_entity_id, weight) in edges {
            let Some(&from_index) = node_index.get(from_entity_id) else {
                continue;
            };
            let Some(&to_index) = node_index.get(to_entity_id) else {
                continue;
            };
            adjacency[from_index as usize].push((to_index, (*weight).max(0.0)));
        }

        let mut offsets = Vec::with_capacity(node_ids.len() + 1);
        let mut neighbors = Vec::new();
        let mut weights = Vec::new();
        offsets.push(0);
        let mut running = 0_u32;
        for edges in adjacency {
            running = running.saturating_add(edges.len() as u32);
            for (neighbor, weight) in edges {
                neighbors.push(neighbor);
                weights.push(weight);
            }
            offsets.push(running);
        }

        Self {
            node_index,
            node_ids,
            offsets,
            neighbors,
            weights,
            loaded_at: Utc::now(),
        }
    }

    #[must_use]
    pub(crate) fn node_idx(&self, entity_id: &EntityId) -> Option<u32> {
        self.node_index.get(entity_id).copied()
    }

    #[must_use]
    pub(crate) fn neighbor_indices(&self, index: u32) -> &[u32] {
        let i = index as usize;
        if i + 1 >= self.offsets.len() {
            return &[];
        }
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        &self.neighbors[start..end]
    }

    #[must_use]
    pub fn loaded_at(&self) -> DateTime<Utc> {
        self.loaded_at
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn sample_snapshot() -> GraphSnapshot {
        GraphSnapshot::from_edge_list(
            vec![
                EntityId::new("a"),
                EntityId::new("b"),
                EntityId::new("c"),
                EntityId::new("d"),
            ],
            &[
                (EntityId::new("a"), EntityId::new("b"), 1.0),
                (EntityId::new("b"), EntityId::new("c"), 1.0),
            ],
        )
    }

    #[test]
    fn ppr_ranks_seed_neighbor_highest() {
        let snapshot = sample_snapshot();
        let result = snapshot.run_ppr(&PprQuery {
            seeds: vec![(EntityId::new("a"), 1.0)],
            top_k: 3,
            ..PprQuery::default()
        });

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].entity_id, EntityId::new("a"));
        assert_eq!(result[1].entity_id, EntityId::new("b"));
        assert!(result[0].activation_score > result[1].activation_score);
        assert!(result[1].activation_score > result[2].activation_score);
    }

    #[test]
    fn ppr_disconnected_nodes_stay_zero() {
        let snapshot = sample_snapshot();
        let result = snapshot.run_ppr(&PprQuery {
            seeds: vec![(EntityId::new("a"), 1.0)],
            top_k: 10,
            ..PprQuery::default()
        });

        assert!(
            result
                .iter()
                .all(|entry| entry.entity_id != EntityId::new("d"))
        );
    }

    #[tokio::test]
    async fn activation_cache_rebuilds_after_invalidation() {
        let cache = GraphActivationCache::new();
        let owner_id = EntityId::new("owner");
        let first = Arc::new(sample_snapshot());
        cache
            .snapshots
            .write()
            .await
            .insert(owner_id.clone(), first);

        cache.invalidate(&owner_id).await;
        assert!(!cache.snapshots.read().await.contains_key(&owner_id));

        let replacement = Arc::new(sample_snapshot());
        cache
            .snapshots
            .write()
            .await
            .insert(owner_id.clone(), Arc::clone(&replacement));

        let cached = cache.snapshots.read().await.get(&owner_id).cloned();
        assert!(
            cached
                .as_ref()
                .is_some_and(|entry| Arc::ptr_eq(entry, &replacement))
        );
    }
}
