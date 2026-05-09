//! Persistent user knowledge graph with knowledge triplets and
//! temporal decay. Accumulates cross-conversation knowledge about
//! individual users (topics, expertise, communication preferences)
//! to enable genuinely personalized responses.
#![allow(clippy::cast_precision_loss)]

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::contracts::scores::Confidence;
use crate::contracts::strings::data_model::{
    SOURCE_PERSONA_USER_KNOWLEDGE_UPDATE, SOURCE_PERSONA_USER_KNOWLEDGE_WRITEBACK,
    SUFFIX_USER_KNOWLEDGE_SLOT_V1,
};
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::{person_entity_id, sanitize_person_id};

const MAX_TRIPLETS: usize = 100;
const DECAY_HALF_LIFE_DAYS: f64 = 30.0;

/// A knowledge triplet: (subject, relation, object) with metadata.
///
/// Examples:
///   ("user", "`expert_in`", "Rust async")
///   ("user", "prefers", "concise responses")
///   ("user", "`working_on`", "web scraper project")
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct KnowledgeTriplet {
    pub subject: String,
    pub relation: String,
    pub object: String,
    /// Confidence in this knowledge (0.0–1.0).
    pub confidence: Confidence,
    /// How many times this knowledge has been reinforced.
    pub reinforcement_count: u32,
    /// When this triplet was first observed.
    pub created_at: String,
    /// When this triplet was last reinforced.
    pub last_seen: String,
}

impl KnowledgeTriplet {
    fn new(subject: &str, relation: &str, object: &str, confidence: f64) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            subject: subject.to_string(),
            relation: relation.to_string(),
            object: object.to_string(),
            confidence: Confidence::new(confidence.max(0.1)),
            reinforcement_count: 1,
            created_at: now.clone(),
            last_seen: now,
        }
    }

    /// Apply temporal decay based on time since last seen.
    pub(crate) fn decayed_confidence(&self) -> f64 {
        let last_seen = chrono::DateTime::parse_from_rfc3339(&self.last_seen)
            .ok()
            .map(|dt| dt.with_timezone(&Utc));
        let days_since = last_seen.map_or(0.0, |ls| (Utc::now() - ls).num_hours() as f64 / 24.0);
        let decay = (-days_since * (2.0_f64.ln()) / DECAY_HALF_LIFE_DAYS).exp();
        (self.confidence.get() * decay).clamp(0.0, 1.0)
    }
}

/// Persistent user knowledge graph for one user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct UserKnowledgeGraph {
    pub triplets: Vec<KnowledgeTriplet>,
    /// Inferred expertise domains with confidence.
    pub expertise_areas: Vec<ExpertiseArea>,
    /// Communication preferences extracted over time.
    pub communication_preferences: CommunicationPreferences,
    /// Topics the user has discussed, with recency weighting.
    pub topic_history: Vec<TopicEntry>,
}

/// An expertise area inferred from interaction patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExpertiseArea {
    pub domain: String,
    pub level: ExpertiseLevel,
    pub evidence_count: u32,
    pub last_updated: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExpertiseLevel {
    Novice,
    Intermediate,
    Advanced,
    Expert,
}

/// Accumulated communication preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CommunicationPreferences {
    /// Average message length (proxy for verbosity preference).
    pub avg_message_length: f64,
    pub message_count: u32,
    /// Whether user tends to use code blocks.
    pub uses_code_blocks: bool,
    /// Preferred response style keywords extracted over time.
    pub style_keywords: Vec<String>,
}

/// A topic the user has discussed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TopicEntry {
    pub topic: String,
    pub mention_count: u32,
    pub last_mentioned: String,
}

impl UserKnowledgeGraph {
    /// Update the knowledge graph with signals from a new turn.
    pub(crate) fn update_from_turn(
        &mut self,
        user_message: &str,
        _assistant_answer: &str,
        success_score: f64,
    ) {
        self.update_communication_prefs(user_message);
        // Single lowercase per turn — shared across all extractors. Previously
        // each of the three `extract_*` methods allocated its own copy.
        let user_lower = user_message.to_lowercase();
        self.extract_expertise_signals(&user_lower);
        self.extract_topic_signals(user_message, &user_lower);
        self.extract_knowledge_triplets(user_message, &user_lower, success_score);
        self.prune_stale_triplets();
    }

    /// Query triplets matching a subject or object.
    ///
    /// `keyword_lower` must already be lowercased by the caller — amortising
    /// the lowercase cost across many `query()` calls in a tight loop. The
    /// stored `subject` / `object` fields are not pre-lowercased (they may
    /// carry original-case user input) so we still lowercase each comparand.
    #[cfg(test)]
    pub(crate) fn query(&self, keyword_lower: &str) -> Vec<&KnowledgeTriplet> {
        self.triplets
            .iter()
            .filter(|t| {
                t.subject.to_lowercase().contains(keyword_lower)
                    || t.object.to_lowercase().contains(keyword_lower)
            })
            .filter(|t| t.decayed_confidence() > 0.1)
            .collect()
    }

    fn update_communication_prefs(&mut self, message: &str) {
        let prefs = &mut self.communication_preferences;
        let msg_len = message.len() as f64;
        let count = f64::from(prefs.message_count);
        prefs.avg_message_length = (prefs.avg_message_length * count + msg_len) / (count + 1.0);
        prefs.message_count = prefs.message_count.saturating_add(1);
        if message.contains("```") {
            prefs.uses_code_blocks = true;
        }
    }

    fn extract_expertise_signals(&mut self, lower: &str) {
        let domain_keywords: &[(&str, &[&str])] = &[
            (
                "Rust",
                &[
                    "rust",
                    "cargo",
                    "tokio",
                    "async",
                    "borrow checker",
                    "lifetime",
                ],
            ),
            (
                "Python",
                &["python", "pip", "pandas", "numpy", "django", "flask"],
            ),
            (
                "DevOps",
                &["docker", "kubernetes", "ci/cd", "terraform", "ansible"],
            ),
            (
                "Web",
                &["html", "css", "javascript", "react", "vue", "frontend"],
            ),
            (
                "Database",
                &["sql", "postgres", "mysql", "mongodb", "redis"],
            ),
        ];

        for (domain, keywords) in domain_keywords {
            let hits = keywords.iter().filter(|kw| lower.contains(*kw)).count();
            if hits >= 2 {
                let level = if hits >= 4 {
                    ExpertiseLevel::Expert
                } else if hits >= 3 {
                    ExpertiseLevel::Advanced
                } else {
                    ExpertiseLevel::Intermediate
                };

                if let Some(existing) = self
                    .expertise_areas
                    .iter_mut()
                    .find(|e| e.domain == *domain)
                {
                    existing.evidence_count = existing.evidence_count.saturating_add(1);
                    existing.last_updated = Utc::now().to_rfc3339();
                    // Only upgrade level, never downgrade.
                    if (level as u8) > (existing.level as u8) {
                        existing.level = level;
                    }
                } else {
                    self.expertise_areas.push(ExpertiseArea {
                        domain: (*domain).to_string(),
                        level,
                        evidence_count: 1,
                        last_updated: Utc::now().to_rfc3339(),
                    });
                }
            }
        }
    }

    fn extract_topic_signals(&mut self, _message: &str, lower: &str) {
        // Extract significant words as topic candidates. Iterate the shared
        // lowercased buffer directly — no per-word `.to_lowercase()` clone.
        // Note: stored `topic` entries are always lowercased (see push below),
        // so equality checks use `t.topic == word` without re-lowercasing.
        for word in lower.split_whitespace().filter(|w| w.len() > 5).take(5) {
            // Skip common words.
            if matches!(
                word,
                "please"
                    | "thanks"
                    | "should"
                    | "would"
                    | "could"
                    | "about"
                    | "there"
                    | "which"
                    | "these"
                    | "those"
                    | "their"
            ) {
                continue;
            }

            if let Some(existing) = self.topic_history.iter_mut().find(|t| t.topic == word) {
                existing.mention_count = existing.mention_count.saturating_add(1);
                existing.last_mentioned = Utc::now().to_rfc3339();
            } else if self.topic_history.len() < 50 {
                self.topic_history.push(TopicEntry {
                    topic: word.to_string(),
                    mention_count: 1,
                    last_mentioned: Utc::now().to_rfc3339(),
                });
            }
        }
    }

    fn extract_knowledge_triplets(&mut self, _message: &str, lower: &str, success_score: f64) {
        // Extract "working on" patterns.
        if let Some(project) = extract_after_phrase(lower, "working on") {
            self.upsert_triplet("user", "working_on", &project, 0.7);
        }

        // Extract preference statements.
        let preference_patterns = [
            ("i prefer", "prefers"),
            ("i like", "prefers"),
            ("i want", "wants"),
            ("i need", "needs"),
        ];
        for (pattern, relation) in &preference_patterns {
            if let Some(object) = extract_after_phrase(lower, pattern) {
                self.upsert_triplet("user", relation, &object, 0.6);
            }
        }

        // If the turn was about a specific topic and was successful,
        // record implicit knowledge. Use `.next()` on the iterator instead
        // of collecting a `Vec<&str>` just to grab the first element.
        if success_score > 0.7
            && lower.len() > 50
            && let Some(topic) = lower.split_whitespace().find(|w| w.len() > 6)
        {
            self.upsert_triplet("user", "discussed_successfully", topic, 0.4);
        }
    }

    fn upsert_triplet(&mut self, subject: &str, relation: &str, object: &str, confidence: f64) {
        if let Some(existing) = self
            .triplets
            .iter_mut()
            .find(|t| t.subject == subject && t.relation == relation && t.object == object)
        {
            existing.reinforcement_count = existing.reinforcement_count.saturating_add(1);
            existing.confidence =
                Confidence::new((existing.confidence.get() + confidence * 0.1).clamp(0.1, 1.0));
            existing.last_seen = Utc::now().to_rfc3339();
        } else if self.triplets.len() < MAX_TRIPLETS {
            self.triplets
                .push(KnowledgeTriplet::new(subject, relation, object, confidence));
        }
    }

    fn prune_stale_triplets(&mut self) {
        self.triplets.retain(|t| t.decayed_confidence() > 0.05);
    }
}

/// Extract the first few words after a phrase in text.
fn extract_after_phrase(text: &str, phrase: &str) -> Option<String> {
    let idx = text.find(phrase)?;
    let after = &text[idx + phrase.len()..];
    let words: Vec<&str> = after.split_whitespace().take(4).collect();
    if words.is_empty() {
        return None;
    }
    Some(words.join(" "))
}

// ── Persistence ─────────────────────────────────────────────────

fn knowledge_slot_key(person_id: &str) -> String {
    format!(
        "persona/{}{}",
        sanitize_person_id(person_id),
        SUFFIX_USER_KNOWLEDGE_SLOT_V1
    )
}

/// Load a user's knowledge graph from memory.
pub(crate) async fn load_user_knowledge(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<UserKnowledgeGraph> {
    let entity_id = person_entity_id(person_id);
    let slot_key = knowledge_slot_key(person_id);
    match mem.resolve_slot(&entity_id, &slot_key).await? {
        Some(slot) => match serde_json::from_str::<UserKnowledgeGraph>(&slot.value) {
            Ok(kg) => Ok(kg),
            Err(error) => {
                tracing::warn!(%error, "failed to parse user knowledge graph; resetting");
                Ok(UserKnowledgeGraph::default())
            }
        },
        None => Ok(UserKnowledgeGraph::default()),
    }
}

/// Persist a user's knowledge graph to memory.
pub(crate) async fn persist_user_knowledge(
    mem: &dyn Memory,
    person_id: &str,
    kg: &UserKnowledgeGraph,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        knowledge_slot_key(person_id),
        MemoryEventType::FactUpdated,
        serde_json::to_string(kg)?,
        0.85,
        0.7,
        SOURCE_PERSONA_USER_KNOWLEDGE_UPDATE,
        SOURCE_PERSONA_USER_KNOWLEDGE_WRITEBACK,
        None,
        person_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::contracts::memory_traits::MemoryReader;
    use crate::core::memory::{MarkdownMemory, Memory};

    #[test]
    fn new_knowledge_graph_is_empty() {
        let kg = UserKnowledgeGraph::default();
        assert!(kg.triplets.is_empty());
        assert!(kg.expertise_areas.is_empty());
        assert!(kg.topic_history.is_empty());
    }

    #[test]
    fn update_extracts_expertise() {
        let mut kg = UserKnowledgeGraph::default();
        kg.update_from_turn(
            "I need help with async tokio and the borrow checker in Rust",
            "Sure, here's how...",
            0.8,
        );
        assert!(!kg.expertise_areas.is_empty());
        assert_eq!(kg.expertise_areas[0].domain, "Rust");
    }

    #[test]
    fn update_extracts_preference_triplet() {
        let mut kg = UserKnowledgeGraph::default();
        kg.update_from_turn("I prefer concise and direct responses", "Got it.", 0.7);
        let prefs: Vec<_> = kg
            .triplets
            .iter()
            .filter(|t| t.relation == "prefers")
            .collect();
        assert!(!prefs.is_empty());
    }

    #[test]
    fn update_extracts_working_on() {
        let mut kg = UserKnowledgeGraph::default();
        kg.update_from_turn(
            "I'm working on a web scraper project",
            "That sounds interesting.",
            0.7,
        );
        let projects: Vec<_> = kg
            .triplets
            .iter()
            .filter(|t| t.relation == "working_on")
            .collect();
        assert!(!projects.is_empty());
        assert!(projects[0].object.contains("web scraper"));
    }

    #[test]
    fn reinforcement_increases_confidence() {
        let mut kg = UserKnowledgeGraph::default();
        kg.update_from_turn("I prefer short answers", "ok", 0.7);
        let conf1 = kg.triplets[0].confidence;

        kg.update_from_turn("I prefer short answers", "ok", 0.7);
        let conf2 = kg.triplets[0].confidence;

        assert!(
            conf2 > conf1,
            "confidence should increase with reinforcement"
        );
        assert_eq!(kg.triplets[0].reinforcement_count, 2);
    }

    #[test]
    fn query_finds_matching_triplets() {
        let mut kg = UserKnowledgeGraph::default();
        kg.update_from_turn("I'm working on a compiler project", "", 0.7);

        let results = kg.query("compiler");
        assert!(!results.is_empty());
    }

    #[test]
    fn topic_history_tracks_mentions() {
        let mut kg = UserKnowledgeGraph::default();
        kg.update_from_turn("Let's discuss authentication", "", 0.7);
        kg.update_from_turn("More about authentication", "", 0.7);

        let auth_topic = kg
            .topic_history
            .iter()
            .find(|t| t.topic.contains("authentication"));
        assert!(auth_topic.is_some());
        assert!(auth_topic.unwrap().mention_count >= 2);
    }

    #[test]
    fn communication_prefs_updated() {
        let mut kg = UserKnowledgeGraph::default();
        kg.update_from_turn("short msg", "", 0.5);
        assert_eq!(kg.communication_preferences.message_count, 1);
        assert!(kg.communication_preferences.avg_message_length > 0.0);

        kg.update_from_turn("```rust\nfn main() {}\n```", "", 0.5);
        assert!(kg.communication_preferences.uses_code_blocks);
    }

    #[test]
    fn render_knowledge_block_empty_when_no_data() {
        let kg = UserKnowledgeGraph::default();
        assert!(crate::core::persona::presenter::render_knowledge_block(&kg, "hello").is_empty());
    }

    #[test]
    fn render_knowledge_block_includes_expertise() {
        let mut kg = UserKnowledgeGraph::default();
        kg.expertise_areas.push(ExpertiseArea {
            domain: "Rust".to_string(),
            level: ExpertiseLevel::Advanced,
            evidence_count: 5,
            last_updated: Utc::now().to_rfc3339(),
        });
        let block = crate::core::persona::presenter::render_knowledge_block(&kg, "help with rust");
        assert!(block.contains("[User Knowledge]"));
        assert!(block.contains("Rust"));
        assert!(block.contains("Advanced"));
    }

    #[test]
    fn temporal_decay_reduces_confidence() {
        let mut triplet = KnowledgeTriplet::new("user", "likes", "tests", 0.8);
        // Simulate last seen 60 days ago (2 half-lives → 25% of original).
        triplet.last_seen = (Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        let decayed = triplet.decayed_confidence();
        assert!(
            decayed < 0.3,
            "after 2 half-lives, confidence should be ~25% of original, got {decayed}"
        );
    }

    #[test]
    fn extract_after_phrase_works() {
        assert_eq!(
            extract_after_phrase("i'm working on a big project now", "working on"),
            Some("a big project now".to_string())
        );
        assert_eq!(extract_after_phrase("hello world", "working on"), None);
    }

    #[tokio::test]
    async fn persist_user_knowledge_uses_person_entity_once_and_canonical_slot() {
        let temp = TempDir::new().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let kg = UserKnowledgeGraph::default();

        persist_user_knowledge(mem.as_ref(), "local-default", &kg)
            .await
            .expect("persist succeeds");

        let slot = mem
            .resolve_slot(
                "person:local-default",
                "persona/local-default/user_knowledge/v1",
            )
            .await
            .expect("resolve succeeds")
            .expect("slot exists");

        assert!(slot.value.contains("\"triplets\""));
    }

    #[cfg(feature = "postgres")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn persist_user_knowledge_round_trips_in_postgres_memory() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        use std::time::Duration;

        use crate::core::memory::embeddings::{EmbeddingFuture, EmbeddingProvider};
        use crate::core::memory::postgres::{PostgresConnectOptions, PostgresMemory};
        use crate::utils::test_env::EnvVarGuard;

        struct FailingEmbedding;

        impl EmbeddingProvider for FailingEmbedding {
            fn name(&self) -> &'static str {
                "failing_test"
            }

            fn dimensions(&self) -> usize {
                3
            }

            fn embed<'a>(&'a self, _texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
                Box::pin(async move { anyhow::bail!("synthetic embedding failure") })
            }
        }

        let _env_guard = EnvVarGuard::require_postgres_url();
        let database_url = std::env::var("ASTEREL_POSTGRES_URL").expect("postgres url");
        let mem = PostgresMemory::connect_with_options(
            &database_url,
            Arc::new(FailingEmbedding),
            PostgresConnectOptions {
                cache_max: 16,
                graph_retrieval_fusion_enabled: false,
                graph_retrieval_weight: 0.0,
                max_connections: 4,
                min_connections: 1,
                connect_timeout: Duration::from_secs(5),
                idle_timeout: Duration::from_secs(30),
                vector_weight: 0.7,
                keyword_weight: 0.3,
                max_lifetime: Duration::from_secs(60),
                hnsw_ef_search: 0,
            },
        )
        .await
        .expect("connect postgres memory");

        let person_id = format!("postgres-user-knowledge-{}", uuid::Uuid::new_v4().simple());
        let kg = UserKnowledgeGraph::default();

        persist_user_knowledge(&mem, &person_id, &kg)
            .await
            .expect("persist succeeds");

        let slot = mem
            .resolve_slot(
                &person_entity_id(&person_id),
                &knowledge_slot_key(&person_id),
            )
            .await
            .expect("resolve succeeds")
            .expect("slot exists");

        assert!(slot.value.contains("\"triplets\""));
    }
}
