//! Taste engine orchestrator: wires the critic, learner, store,
//! and domain adapters into the `TasteEngine` trait for artifact
//! evaluation and preference comparison.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::Context;

use super::adapter::{DomainAdapter, TextAdapter, UiAdapter};
use super::critic::{LlmCritic, UniversalCritic};
use super::learner::{BradleyTerryLearner, TasteLearner};
use super::store::{ItemRating, PostgresTasteStore, TasteStore};
use super::types::{
    Artifact, Domain, PairComparison, TasteContext, TasteOwnerScope, TasteReport, Winner,
};
use crate::config::TasteConfig;
use crate::core::providers::Provider;

/// Trait for evaluating artifacts and comparing preferences.
pub trait TasteEngine: Send + Sync {
    /// Evaluate an artifact and return a taste report with scores.
    fn evaluate<'a>(
        &'a self,
        artifact: &'a Artifact,
        ctx: &'a TasteContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<TasteReport>> + Send + 'a>>;

    /// Record a pairwise preference comparison.
    fn compare<'a>(
        &'a self,
        comparison: &'a PairComparison,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    /// Whether the taste engine is enabled in configuration.
    fn enabled(&self) -> bool;
}

async fn refresh_item_cache(
    store: &dyn TasteStore,
    item_id: &str,
    domain: &Domain,
    owner: &TasteOwnerScope,
    label: &str,
) {
    if let Err(error) = store.get_comparisons_for_item(item_id, domain, owner).await {
        tracing::warn!(
            error = %error,
            item_id = %item_id,
            domain = ?domain,
            side = label,
            "failed to refresh taste comparisons after update"
        );
    }
    if let Err(error) = store.get_rating(item_id, domain, owner).await {
        tracing::warn!(
            error = %error,
            item_id = %item_id,
            domain = ?domain,
            side = label,
            "failed to refresh taste rating after update"
        );
    }
}

/// Default implementation wiring critic, adapters, store, and learner.
pub struct DefaultTasteEngine {
    /// Taste engine configuration.
    pub config: TasteConfig,
    /// LLM-based aesthetic critic for scoring artifacts.
    pub(crate) critic: Arc<dyn UniversalCritic>,
    /// Domain-specific suggestion adapters keyed by domain.
    pub(crate) adapters: HashMap<Domain, Arc<dyn DomainAdapter>>,
    /// Optional persistent store for comparisons and ratings.
    pub(crate) store: Option<Arc<dyn TasteStore>>,
    /// Optional Bradley-Terry preference learners partitioned by owner scope.
    pub(crate) learner: Option<Arc<Mutex<HashMap<TasteOwnerScope, BradleyTerryLearner>>>>,
}

impl TasteEngine for DefaultTasteEngine {
    fn evaluate<'a>(
        &'a self,
        artifact: &'a Artifact,
        ctx: &'a TasteContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<TasteReport>> + Send + 'a>> {
        Box::pin(async move {
            let critique = self.critic.critique(artifact, ctx).await?;
            tracing::debug!(
                confidence = critique.confidence,
                "taste critic produced critique confidence"
            );

            let domain = match artifact {
                Artifact::Text { .. } => Domain::Text,
                Artifact::Ui { .. } => Domain::Ui,
            };

            let suggestions = self
                .adapters
                .get(&domain)
                .map(|adapter| adapter.suggest(&critique, ctx))
                .unwrap_or_default();

            Ok(TasteReport {
                axis: critique.axis_scores,
                domain,
                suggestions,
                raw_critique: Some(critique.raw_response),
            })
        })
    }

    fn compare<'a>(
        &'a self,
        comparison: &'a PairComparison,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if comparison.winner == Winner::Abstain {
                if let Some(store) = &self.store {
                    store
                        .record_comparison_with_ratings(comparison, Vec::new())
                        .await?;
                }
                return Ok(());
            }

            if let Some(learner) = &self.learner {
                let (ratings, left_stable, right_stable) = {
                    let mut l = learner
                        .lock()
                        .map_err(|e| anyhow::anyhow!("learner lock poisoned: {e}"))?;

                    let owner = comparison.owner.clone();
                    let l = l
                        .entry(owner.clone())
                        .or_insert_with(BradleyTerryLearner::new);

                    let outcome = match comparison.winner {
                        Winner::Left => 1.0,
                        Winner::Right => 0.0,
                        Winner::Tie => 0.5,
                        Winner::Abstain => unreachable!("abstain handled before learner lock"),
                    };

                    l.update(&comparison.left_id, &comparison.right_id, outcome);

                    let left = l.get_rating(&comparison.left_id);
                    let right = l.get_rating(&comparison.right_id);
                    let left_stable = l.get_rating_if_sufficient(&comparison.left_id, 5).is_some();
                    let right_stable = l
                        .get_rating_if_sufficient(&comparison.right_id, 5)
                        .is_some();
                    let now = chrono::Utc::now().to_rfc3339();
                    let ratings = [
                        (comparison.left_id.clone(), left),
                        (comparison.right_id.clone(), right),
                    ]
                    .into_iter()
                    .filter_map(|(item_id, rating)| {
                        rating.map(|(rating, n_comparisons)| ItemRating {
                            owner: owner.clone(),
                            item_id,
                            domain: comparison.domain.clone(),
                            rating,
                            n_comparisons,
                            updated_at: now.clone(),
                        })
                    })
                    .collect::<Vec<_>>();
                    (ratings, left_stable, right_stable)
                };

                if let Some(store) = &self.store {
                    store
                        .record_comparison_with_ratings(comparison, ratings.clone())
                        .await?;

                    for rating in &ratings {
                        refresh_item_cache(
                            store.as_ref(),
                            &rating.item_id,
                            &rating.domain,
                            &rating.owner,
                            if rating.item_id == comparison.left_id {
                                "left"
                            } else {
                                "right"
                            },
                        )
                        .await;
                    }

                    tracing::debug!(
                        left_id = %comparison.left_id,
                        right_id = %comparison.right_id,
                        left_stable,
                        right_stable,
                        "taste comparison updated persistent ratings"
                    );
                }
            } else if let Some(store) = &self.store {
                store
                    .record_comparison_with_ratings(comparison, Vec::new())
                    .await?;
            }

            Ok(())
        })
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }
}

/// Creates a taste engine instance from configuration.
///
/// # Errors
///
/// Returns an error when fallback in-memory taste storage cannot be created or
/// taste store initialization fails.
pub fn create_taste_engine(
    config: &TasteConfig,
    provider: Arc<dyn Provider>,
    model: String,
    workspace_dir: &Path,
) -> anyhow::Result<Arc<dyn TasteEngine>> {
    let critic = LlmCritic::new(provider, model);

    let mut adapters: HashMap<Domain, Arc<dyn DomainAdapter>> = HashMap::new();
    if config.text_enabled {
        let adapter: Arc<dyn DomainAdapter> = Arc::new(TextAdapter);
        adapters.insert(adapter.domain(), adapter);
    }
    if config.ui_enabled {
        let adapter: Arc<dyn DomainAdapter> = Arc::new(UiAdapter);
        adapters.insert(adapter.domain(), adapter);
    }

    let database_url =
        crate::utils::postgres::require_postgres_url(None, Some(workspace_dir), "taste store")?;

    let bootstrap_domain = if config.text_enabled {
        Domain::Text
    } else if config.ui_enabled {
        Domain::Ui
    } else {
        Domain::General
    };

    // Connect and run the bootstrap query in a single block_on_taste call so
    // that both operations share the same tokio runtime. Splitting them across
    // two calls creates separate runtimes when called outside an async context;
    // the pool is bound to the first runtime and becomes unusable on the second.
    let learner = HashMap::new();

    let store = block_on_taste(async {
        let store = PostgresTasteStore::connect(&database_url).await?;
        store
            .get_all_ratings(&bootstrap_domain)
            .await
            .context("load bootstrap taste ratings")?;
        Ok(store)
    })?;

    let engine = DefaultTasteEngine {
        config: config.clone(),
        critic: Arc::new(critic),
        adapters,
        store: Some(Arc::new(store)),
        learner: Some(Arc::new(Mutex::new(learner))),
    };

    Ok(Arc::new(engine))
}

fn block_on_taste<T, F>(future: F) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            Err(anyhow::anyhow!(
                "taste store bootstrap requires multi-thread tokio runtime; skipping in current-thread runtime"
            ))
        }
    } else {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|error| anyhow::anyhow!("create taste runtime: {error}"))?;
        runtime.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex as StdMutex};

    use tempfile::TempDir;

    use super::*;
    use crate::core::providers::{Provider, ProviderResult};
    use crate::core::taste::critic::CritiqueResult;
    use crate::core::taste::types::{Axis, TasteOwnerScope};
    use crate::utils::test_env::EnvVarGuard;

    struct NoopProvider;

    impl Provider for NoopProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Ok("ok".to_string()) })
        }
    }

    struct NoopCritic;

    impl UniversalCritic for NoopCritic {
        fn critique<'a>(
            &'a self,
            _artifact: &'a Artifact,
            _ctx: &'a TasteContext,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<CritiqueResult>> + Send + 'a>> {
            Box::pin(async move {
                Ok(CritiqueResult {
                    axis_scores: BTreeMap::from([
                        (Axis::Coherence, 0.5),
                        (Axis::Hierarchy, 0.5),
                        (Axis::Intentionality, 0.5),
                    ]),
                    raw_response: String::new(),
                    confidence: 1.0,
                })
            })
        }
    }

    struct RecordingTasteStore {
        calls: Arc<StdMutex<Vec<(PairComparison, Vec<ItemRating>)>>>,
    }

    impl TasteStore for RecordingTasteStore {
        fn save_comparison<'a>(
            &'a self,
            _comparison: &'a PairComparison,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move { anyhow::bail!("compare should use transaction method") })
        }

        fn get_comparisons_for_item<'a>(
            &'a self,
            _item_id: &'a str,
            _domain: &'a Domain,
            _owner: &'a TasteOwnerScope,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<PairComparison>>> + Send + 'a>>
        {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_rating<'a>(
            &'a self,
            _item_id: &'a str,
            _domain: &'a Domain,
            _owner: &'a TasteOwnerScope,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<ItemRating>>> + Send + 'a>> {
            Box::pin(async move { Ok(None) })
        }

        fn update_rating(
            &self,
            _rating: ItemRating,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
            Box::pin(async move { anyhow::bail!("compare should use transaction method") })
        }

        fn record_comparison_with_ratings<'a>(
            &'a self,
            comparison: &'a PairComparison,
            ratings: Vec<ItemRating>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("recording lock")
                    .push((comparison.clone(), ratings));
                Ok(())
            })
        }

        fn get_all_ratings<'a>(
            &'a self,
            _domain: &'a Domain,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ItemRating>>> + Send + 'a>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    #[tokio::test]
    async fn compare_persists_comparison_and_ratings_atomically_by_owner() {
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let engine = DefaultTasteEngine {
            config: TasteConfig::default(),
            critic: Arc::new(NoopCritic),
            adapters: HashMap::new(),
            store: Some(Arc::new(RecordingTasteStore {
                calls: Arc::clone(&calls),
            })),
            learner: Some(Arc::new(StdMutex::new(HashMap::new()))),
        };
        let owner = TasteOwnerScope::new(Some("tenant-a"), "person-a", Some("session-a"));
        let comparison = PairComparison {
            owner: owner.clone(),
            domain: Domain::Text,
            ctx: TasteContext::default(),
            left_id: "left".to_string(),
            right_id: "right".to_string(),
            winner: Winner::Left,
            rationale: Some("clearer".to_string()),
            created_at_ms: 42,
        };

        engine.compare(&comparison).await.expect("compare succeeds");

        let calls = calls.lock().expect("recording lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.owner, owner);
        assert_eq!(calls[0].1.len(), 2);
        assert!(calls[0].1.iter().all(|rating| rating.owner == owner));
    }

    #[test]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    fn create_taste_engine_creates_persistent_store_file() {
        let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
        let temp = TempDir::new().expect("temp dir");
        let _env_guard = EnvVarGuard::require_postgres_url();
        let config = TasteConfig {
            enabled: true,
            ..TasteConfig::default()
        };
        let provider: Arc<dyn Provider> = Arc::new(NoopProvider);

        let engine = create_taste_engine(&config, provider, "test-model".to_string(), temp.path())
            .expect("taste engine should initialize");

        assert!(engine.enabled());
    }
}
