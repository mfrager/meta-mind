//! Hybrid retrieval: lexical ▸ vector ▸ graph ▸ rerank.
//!
//! The score is the plan's fixed linear combination
//! `0.35·lexical + 0.30·vector + 0.20·graph + 0.10·recency + 0.05·importance`,
//! multiplied by `0.5 + 0.5·confidence`. Every channel is normalized into `[0,1]`
//! before it is weighted, because adding a raw `bm25` rank to a cosine in `[-1,1]`
//! would let whichever channel happens to have the larger range decide the ranking.
//!
//! Determinism is a property of the whole pipeline, not of one line: the weights
//! are fixed unless a caller supplies others, every channel's candidate list is
//! bounded, and the final sort breaks ties on ULID. If two memories score the
//! same, the older id wins, always.
//!
//! [`RetrievalEngine::recall`] also *records* what it did — an access row and a
//! `memory.recall` record — because a retrieval the being cannot audit is a
//! retrieval it cannot reason about later.

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, LogRecord, Logger};
use serde_json::json;

use crate::error::Result;
use crate::graph::EntityGraph;
use crate::index::MemoryIndex;
use crate::model::{Channel, Memory, MemoryFilter, RecallHit, RecallQuery, ScorePart};
use crate::store::SqliteMemoryStore;
use crate::utility::{confidence_gate, normalize_bm25, normalize_cosine, recency_score, weight_of};

/// How many candidates one channel may contribute, and how many memories one
/// recall may consider in total.
pub const CHANNEL_CANDIDATES: usize = 64;
/// The candidate ceiling, so a recall over a huge store stays bounded.
pub const MAX_CANDIDATES: usize = 4096;
/// How many seed memories contribute entities to the graph walk.
pub const GRAPH_SEEDS: usize = 5;

/// The channel weights, which a caller may override from `thresholds.toml`.
///
/// They must sum to one; [`RetrievalWeights::validate`] is what refuses a
/// configuration that does not, rather than letting a mistyped weight silently
/// rebalance retrieval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrievalWeights {
    /// Lexical (FTS5/BM25) weight.
    pub lexical: f64,
    /// Vector (cosine) weight.
    pub vector: f64,
    /// Graph (PPR) weight.
    pub graph: f64,
    /// Recency weight.
    pub recency: f64,
    /// Importance weight.
    pub importance: f64,
}

impl Default for RetrievalWeights {
    fn default() -> Self {
        RetrievalWeights {
            lexical: weight_of(Channel::Lexical),
            vector: weight_of(Channel::Vector),
            graph: weight_of(Channel::Graph),
            recency: weight_of(Channel::Recency),
            importance: weight_of(Channel::Importance),
        }
    }
}

impl RetrievalWeights {
    /// The weight of one channel.
    pub fn of(&self, channel: Channel) -> f64 {
        match channel {
            Channel::Lexical => self.lexical,
            Channel::Vector => self.vector,
            Channel::Graph => self.graph,
            Channel::Recency => self.recency,
            Channel::Importance => self.importance,
        }
    }

    /// Refuse weights that do not sum to one.
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("lexical", self.lexical),
            ("vector", self.vector),
            ("graph", self.graph),
            ("recency", self.recency),
            ("importance", self.importance),
        ] {
            if !(0.0..=1.0).contains(&value) || !value.is_finite() {
                return Err(crate::error::MemoryError::Config(format!(
                    "weight {name} must be in [0,1], got {value}"
                )));
            }
        }
        let sum = self.lexical + self.vector + self.graph + self.recency + self.importance;
        if (sum - 1.0).abs() > 1e-9 {
            return Err(crate::error::MemoryError::Config(format!(
                "retrieval weights must sum to 1, got {sum}"
            )));
        }
        Ok(())
    }

    /// Read weights from `(channel, weight)` pairs, defaulting the rest.
    pub fn from_pairs(pairs: &[(String, f64)]) -> Self {
        let mut weights = RetrievalWeights::default();
        for (name, value) in pairs {
            match name.as_str() {
                "lexical" => weights.lexical = *value,
                "vector" => weights.vector = *value,
                "graph" => weights.graph = *value,
                "recency" => weights.recency = *value,
                "importance" => weights.importance = *value,
                _ => {}
            }
        }
        weights
    }

    /// The weights as a JSON object, for `memory.rerank`.
    pub fn as_json(&self) -> serde_json::Value {
        json!({
            "lexical": self.lexical,
            "vector": self.vector,
            "graph": self.graph,
            "recency": self.recency,
            "importance": self.importance,
        })
    }
}

/// Score one memory from its per-channel raw values.
///
/// Returns the gated total and the auditable parts. The parts sum to the total
/// *before* the gate, which is what makes a mismatch between them a bug rather
/// than a rounding question.
pub fn hybrid_score(
    weights: &RetrievalWeights,
    raws: &[(Channel, f64)],
    confidence: f32,
) -> (f64, Vec<ScorePart>) {
    let mut parts = Vec::with_capacity(raws.len());
    let mut total = 0.0;
    for (channel, raw) in raws {
        let weight = weights.of(*channel);
        let contribution = raw.clamp(0.0, 1.0) * weight;
        total += contribution;
        parts.push(ScorePart {
            channel: *channel,
            raw: *raw,
            weight,
            contribution,
        });
    }
    (total * confidence_gate(confidence), parts)
}

/// A recall pipeline over a store, an index, and an entity graph.
pub struct RetrievalEngine<'a> {
    weights: RetrievalWeights,
    half_life_ns: u64,
    index: Option<&'a MemoryIndex>,
}

impl std::fmt::Debug for RetrievalEngine<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetrievalEngine")
            .field("weights", &self.weights)
            .field("half_life_ns", &self.half_life_ns)
            .field("indexed", &self.index.map(MemoryIndex::len))
            .finish()
    }
}

impl<'a> RetrievalEngine<'a> {
    /// An engine with these weights and this retention half-life.
    pub fn new(weights: RetrievalWeights, half_life_ns: u64) -> Self {
        RetrievalEngine {
            weights,
            half_life_ns,
            index: None,
        }
    }

    /// Consult a packed vector index for the vector channel.
    pub fn with_index(mut self, index: &'a MemoryIndex) -> Self {
        self.index = Some(index);
        self
    }

    /// Recall the `k` best memories for `query`, recording the access.
    pub async fn recall(
        &self,
        store: &SqliteMemoryStore,
        logger: &Logger,
        ids: &UlidFactory,
        query: &RecallQuery,
    ) -> Result<Vec<RecallHit>> {
        self.weights.validate()?;
        let started = std::time::Instant::now();
        let query_hash = mm_core::hash_fields(&["recall", &query.text]);

        // --- candidates -------------------------------------------------------
        let filter = MemoryFilter {
            kinds: query.kinds.clone(),
            tiers: query.tiers.clone(),
            status: None,
            limit: Some(MAX_CANDIDATES),
        };
        let candidates = store.list(&filter).await?;
        let eligible: std::collections::BTreeMap<Ulid, Memory> =
            candidates.into_iter().map(|m| (m.id, m)).collect();

        // --- [1] lexical ------------------------------------------------------
        let lexical = store
            .lexical_search(&query.text, CHANNEL_CANDIDATES)
            .await?;
        let lexical_n = lexical.len();
        let mut lexical_by_id: std::collections::BTreeMap<Ulid, f64> = lexical
            .into_iter()
            .filter(|(id, _)| eligible.contains_key(id))
            .collect();

        // --- [2] vector -------------------------------------------------------
        let mut vector_by_id: std::collections::BTreeMap<Ulid, f64> = Default::default();
        if let Some(index) = self.index {
            let vector = index.knn(&MemoryIndex::embed(&query.text), CHANNEL_CANDIDATES);
            for (id, cosine) in vector {
                if eligible.contains_key(&id) {
                    vector_by_id.insert(id, cosine);
                }
            }
        }
        let vector_n = vector_by_id.len();

        // A memory that both channels missed can still be reached: the graph walk
        // and the recency/importance terms rank over the whole eligible set. This
        // is the plan's `fallback` flag.
        let fallback = lexical_n + vector_n == 0;

        // --- [3] graph --------------------------------------------------------
        let graph = EntityGraph::load(store, Some(query.now)).await?;
        let ppr = self
            .graph_scores(store, &graph, &eligible, &lexical_by_id, &vector_by_id)
            .await?;
        let graph_n = ppr.values().filter(|v| **v > 0.0).count();
        let ppr_max = ppr.values().copied().fold(0.0f64, f64::max);

        // --- [4] rerank -------------------------------------------------------
        let mut hits: Vec<RecallHit> = Vec::with_capacity(eligible.len());
        for (id, memory) in &eligible {
            let age_ns = query
                .now
                .as_nanos()
                .saturating_sub(memory.recorded_at.as_nanos()) as u64;
            let graph_raw = if ppr_max > 0.0 {
                ppr.get(id).copied().unwrap_or(0.0) / ppr_max
            } else {
                0.0
            };
            let raws = [
                (
                    Channel::Lexical,
                    lexical_by_id.remove(id).map(normalize_bm25).unwrap_or(0.0),
                ),
                (
                    Channel::Vector,
                    vector_by_id.remove(id).map(normalize_cosine).unwrap_or(0.0),
                ),
                (Channel::Graph, graph_raw),
                (Channel::Recency, recency_score(age_ns, self.half_life_ns)),
                (Channel::Importance, f64::from(memory.importance)),
            ];
            let (score, parts) = hybrid_score(&self.weights, &raws, memory.confidence);
            hits.push(RecallHit {
                memory: memory.clone(),
                score,
                parts,
            });
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.memory.id.cmp(&b.memory.id))
        });
        hits.truncate(query.k);

        // --- [5] record -------------------------------------------------------
        let trace = ids.next();
        let mut result_ids = Vec::with_capacity(hits.len());
        let mut scores = Vec::with_capacity(hits.len());
        for hit in &hits {
            store
                .insert_access(hit.memory.id, hit.score, &query_hash, trace)
                .await?;
            result_ids.push(mm_core::ulid_string(&hit.memory.id));
            scores.push(hit.score);
        }
        let kinds: Vec<&str> = query.kinds.iter().map(|k| k.as_str()).collect();
        logger
            .emit(
                LogRecord::new(Level::Info, codes::MEMORY_RECALL, crate::TARGET)
                    .with_trace(trace)
                    .with_field("query_hash", query_hash.clone())
                    .with_field("k", query.k)
                    .with_field("kinds", kinds.join(","))
                    .with_field("lexical_n", lexical_n)
                    .with_field("vector_n", vector_n)
                    .with_field("graph_n", graph_n)
                    .with_field("fallback", fallback)
                    .with_field("result_ids", json!(result_ids))
                    .with_field("scores", json!(scores))
                    .with_field("latency_ms", started.elapsed().as_millis() as u64),
            )
            .await?;
        logger
            .emit(
                LogRecord::new(Level::Debug, codes::MEMORY_RERANK, crate::TARGET)
                    .with_trace(trace)
                    .with_field("result_ids", json!(result_ids))
                    .with_field("weights", self.weights.as_json())
                    .with_field(
                        "parts_summary",
                        json!(hits
                            .iter()
                            .map(|hit| hit
                                .parts
                                .iter()
                                .map(|part| format!("{}={:.4}", part.channel, part.contribution))
                                .collect::<Vec<_>>()
                                .join(","))
                            .collect::<Vec<_>>()),
                    ),
            )
            .await?;
        Ok(hits)
    }

    /// Personal PageRank over the entities the top lexical/vector candidates
    /// mention, attributed back onto the memories that mention an entity.
    async fn graph_scores(
        &self,
        store: &SqliteMemoryStore,
        graph: &EntityGraph,
        eligible: &std::collections::BTreeMap<Ulid, Memory>,
        lexical: &std::collections::BTreeMap<Ulid, f64>,
        vector: &std::collections::BTreeMap<Ulid, f64>,
    ) -> Result<std::collections::BTreeMap<Ulid, f64>> {
        let mut seeds: Vec<Ulid> = Vec::new();
        let mut ordered: Vec<Ulid> = lexical.keys().copied().collect();
        ordered.extend(vector.keys().copied());
        ordered.sort();
        ordered.dedup();
        for id in ordered.into_iter().take(GRAPH_SEEDS) {
            if let Some(memory) = eligible.get(&id) {
                seeds.extend(memory.entities.iter().copied());
            }
        }
        // When nothing matched lexically or vectorially, seed the walk from the
        // entities of the most recent candidates so the graph channel can still
        // say something.
        if seeds.is_empty() {
            let mut by_recency: Vec<&Memory> = eligible.values().collect();
            by_recency.sort_by(|a, b| {
                b.recorded_at
                    .cmp(&a.recorded_at)
                    .then_with(|| a.id.cmp(&b.id))
            });
            for memory in by_recency.into_iter().take(GRAPH_SEEDS) {
                seeds.extend(memory.entities.iter().copied());
            }
        }
        seeds.sort();
        seeds.dedup();
        if seeds.is_empty() || graph.is_empty() {
            return Ok(Default::default());
        }
        let entity_scores = graph.ppr(&seeds, crate::graph::MAX_HOPS, crate::graph::DAMPING);
        // A memory's graph score is the best mass any of its entities carries.
        let by_entity = store.entities_by_entity().await?;
        let mut scores: std::collections::BTreeMap<Ulid, f64> = Default::default();
        for (entity, memories) in by_entity {
            let Some(mass) = entity_scores.get(&entity).copied() else {
                continue;
            };
            if mass <= 0.0 {
                continue;
            }
            for memory in memories {
                scores
                    .entry(memory)
                    .and_modify(|value: &mut f64| *value = value.max(mass))
                    .or_insert(mass);
            }
        }
        Ok(scores)
    }
}

/// The age of a memory at `now`, in nanoseconds.
pub fn age_ns(recorded_at: Timestamp, now: Timestamp) -> u64 {
    now.as_nanos().saturating_sub(recorded_at.as_nanos()) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_one_and_each_channel_maps() {
        let weights = RetrievalWeights::default();
        weights.validate().unwrap();
        assert_eq!(weights.of(Channel::Lexical), 0.35);
        assert_eq!(weights.of(Channel::Graph), 0.20);
    }

    #[test]
    fn a_rebalanced_weight_set_is_refused() {
        let weights = RetrievalWeights {
            lexical: 0.9,
            ..RetrievalWeights::default()
        };
        assert!(weights.validate().is_err());
    }

    #[test]
    fn the_parts_sum_to_the_gated_total() {
        let weights = RetrievalWeights::default();
        let raws = [
            (Channel::Lexical, 0.8),
            (Channel::Vector, 0.5),
            (Channel::Graph, 0.0),
            (Channel::Recency, 1.0),
            (Channel::Importance, 0.4),
        ];
        let (score, parts) = hybrid_score(&weights, &raws, 1.0);
        let expected = 0.35 * 0.8 + 0.30 * 0.5 + 0.10 * 1.0 + 0.05 * 0.4;
        assert!((score - expected).abs() < 1e-12, "{score} != {expected}");
        let sum: f64 = parts.iter().map(|p| p.contribution).sum();
        assert!((sum - score).abs() < 1e-12);
        assert_eq!(parts.len(), 5);
    }

    #[test]
    fn confidence_scales_but_never_zeroes() {
        let weights = RetrievalWeights::default();
        let raws = [(Channel::Lexical, 1.0)];
        let (high, _) = hybrid_score(&weights, &raws, 1.0);
        let (low, _) = hybrid_score(&weights, &raws, 0.0);
        assert!((high - 0.35).abs() < 1e-12);
        assert!((low - 0.175).abs() < 1e-12);
        assert!(low > 0.0, "an untrusted memory must still be reachable");
    }

    #[test]
    fn a_raw_value_outside_the_unit_interval_cannot_dominate() {
        let weights = RetrievalWeights::default();
        let (score, parts) = hybrid_score(&weights, &[(Channel::Lexical, 40.0)], 1.0);
        assert!((score - 0.35).abs() < 1e-12);
        assert_eq!(parts[0].raw, 40.0);
        assert!((parts[0].contribution - 0.35).abs() < 1e-12);
    }

    #[test]
    fn weights_can_be_read_from_threshold_pairs() {
        let weights = RetrievalWeights::from_pairs(&[
            ("lexical".to_string(), 0.5),
            ("vector".to_string(), 0.5),
            ("unknown".to_string(), 9.0),
        ]);
        assert_eq!(weights.lexical, 0.5);
        assert_eq!(weights.vector, 0.5);
        assert_eq!(weights.graph, 0.20);
    }

    #[test]
    fn age_is_saturating_not_wrapping() {
        let now = Timestamp::from_epoch_seconds(10);
        let later = Timestamp::from_epoch_seconds(20);
        assert_eq!(age_ns(later, now), 0, "a future record has no negative age");
        assert_eq!(age_ns(now, later), 10 * 1_000_000_000);
    }
}
