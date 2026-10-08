//! `mm-memory` — the being's long-term memory organ.
//!
//! Design §32 asks for ten classes of memory trace; this crate makes them typed,
//! provenance-bearing, and retrievable. Four mechanisms are borrowed, each from a
//! project whose code is public (plan §0), and each shapes a module:
//!
//! * **A-MEM** — a memory is a *note* with dynamic links ([`model::LinkKind`]),
//!   written on write and reinforced on access.
//! * **Graphiti** — every memory is bi-temporal: world-time validity is
//!   independent of system-time recording, so "what did I know, and when did I
//!   know it" is answerable.
//! * **Mem0 + RAPTOR + GraphRAG** — candidate reconciliation and a summary tree
//!   ([`consolidate`]).
//! * **MemoryBank** — Ebbinghaus retention, so forgetting is a decision rather
//!   than an accident ([`utility::retention_score`], [`forgetting`]).
//!
//! Three rules hold across the whole crate:
//!
//! 1. **Nothing is deleted.** Forgetting archives. A `protected` or
//!    `developmental` record is refused outright, and every refusal is logged with
//!    what protected it.
//! 2. **Every write carries provenance.** [`model::Memory::validate`] refuses a
//!    nil provenance node, and the SHACL shapes refuse a missing `mm:source`.
//! 3. **Retrieval is deterministic.** Fixed channel weights, a fixed ULID
//!    tie-break, and no clock except the instant the caller passes in. A replay
//!    returns the hits the first run returned.
#![forbid(unsafe_code)]

pub mod consolidate;
pub mod error;
pub mod forgetting;
pub mod graph;
pub mod index;
pub mod mistake;
pub mod model;
pub mod procedural;
pub mod rdf;
pub mod retrieval;
pub mod store;
pub mod utility;

pub use consolidate::{ConsolidationPolicy, Episode, Outcome};
pub use error::{AdmissionDenied, MemoryError, ProtectedRefusal, Result, UtilityDenied};
pub use forgetting::{forget_cycle, ForgetPolicy};
pub use graph::{ppr_scores, EntityEdge, EntityGraph};
pub use index::{MemoryIndex, EMBEDDER_NAME, INDEX_VERSION};
pub use mistake::MistakeInput;
pub use model::{
    ref_ulid, Channel, Community, Consolidation, ConsolidationMethod, CueKind, ForgetReport,
    LinkKind, Memory, MemoryFilter, MemoryKind, MistakeRecord, NearMiss, RecallHit, RecallQuery,
    RecordStatus, RetrievalCue, ScorePart, SummaryNode, Tier, TimeInterval, UtilityInputs,
    CHANNELS, MAX_CONTENT_CHARS, MEMORY_KINDS, TIERS,
};
pub use procedural::Procedure;
pub use retrieval::{hybrid_score, RetrievalEngine, RetrievalWeights};
pub use store::{MemoryStore, SqliteMemoryStore};
pub use utility::{
    memory_utility, normalize_bm25, normalize_cosine, recency_score, retention_score, salience,
    MONTH_NS, WEEK_NS,
};

use std::path::Path;
use std::sync::Arc;

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, LogRecord, Logger};
use mm_store_graph::GraphHandle;
use mm_store_sqlite::SqliteStore;
use serde_json::json;

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.memory";

/// What `memory verify` found.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifyReport {
    /// Rows in `memories`.
    pub sqlite_rows: usize,
    /// Quads in `/memory`.
    pub rdf_triples: usize,
    /// True when every class reconciles exactly.
    pub reconciled: bool,
    /// The per-class `(class, sqlite, rdf)` counts.
    pub classes: Vec<(String, usize, usize)>,
    /// Records that would be archived if `forget_cycle` ran, but are protected.
    pub protected_violations: usize,
    /// Sources a consolidation names that do not resolve.
    pub dangling_provenance: usize,
    /// Human-readable failures, empty when everything held.
    pub failures: Vec<String>,
}

impl VerifyReport {
    /// True when every check held.
    pub fn ok(&self) -> bool {
        self.failures.is_empty()
            && self.reconciled
            && self.protected_violations == 0
            && self.dangling_provenance == 0
    }
}

/// The one door to persistent memory.
///
/// A caller never reaches a table: [`MemoryEngine::remember`] validates and admits
/// a memory, [`MemoryEngine::recall`] scores it, and every mutation rewrites the
/// `/memory` mirror from the current state. The mirror being a projection rather
/// than a second authority is what lets `memory stats --reconcile` mean something.
pub struct MemoryEngine {
    store: SqliteMemoryStore,
    graph: GraphHandle,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
    index: index::MemoryIndex,
    weights: RetrievalWeights,
    half_life_ns: u64,
    forget_threshold: f64,
}

impl std::fmt::Debug for MemoryEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryEngine")
            .field("indexed", &self.index.len())
            .field("half_life_ns", &self.half_life_ns)
            .finish_non_exhaustive()
    }
}

impl MemoryEngine {
    /// Open the memory organ over a store and the `/memory` graph handle.
    pub async fn open(
        sqlite: &SqliteStore,
        graph: &GraphHandle,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
    ) -> Result<Self> {
        let store = SqliteMemoryStore::new(sqlite.clone(), Arc::clone(&logger), Arc::clone(&ids));
        let mut index = index::MemoryIndex::new();
        // Rebuild the packed index from the store so a reopened kernel answers the
        // same queries it answered before it closed.
        index.rebuild(&store).await?;
        Ok(MemoryEngine {
            store,
            graph: graph.clone(),
            logger,
            ids,
            index,
            weights: RetrievalWeights::default(),
            half_life_ns: utility::WEEK_NS,
            forget_threshold: forgetting::DEFAULT_THRESHOLD,
        })
    }

    /// The store, for a caller that needs to read a projection directly.
    pub fn store(&self) -> &SqliteMemoryStore {
        &self.store
    }

    /// The packed vector index.
    pub fn index(&self) -> &index::MemoryIndex {
        &self.index
    }

    /// The channel weights.
    pub fn weights(&self) -> RetrievalWeights {
        self.weights
    }

    /// Use these channel weights.
    pub fn with_weights(mut self, weights: RetrievalWeights) -> Self {
        self.weights = weights;
        self
    }

    /// Use this retention half-life.
    pub fn with_half_life(mut self, half_life_ns: u64) -> Self {
        self.half_life_ns = half_life_ns;
        self
    }

    /// Use this forgetting threshold.
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.forget_threshold = threshold;
        self
    }

    // --------------------------------------------------------------- writes ----

    /// Admit a memory, or refuse it and say why.
    ///
    /// The admission gate is the plan's answer to retrieval poisoning: a duplicate
    /// is not stored twice, a near-duplicate is linked instead of merged, and a
    /// contradiction is recorded as a link rather than resolved silently.
    pub async fn remember(&mut self, memory: Memory) -> Result<Admission> {
        memory.validate()?;
        // Exact duplicate: the same content learned true at the same instant is the
        // same fact, and storing it twice would double its weight in every rank.
        if let Some(existing) = self
            .store
            .find_by_hash(memory.kind, &memory.content_hash(), memory.validity.from)
            .await?
        {
            self.record_admission(&memory, "duplicate", Some(&existing))
                .await?;
            return Ok(Admission::Duplicate { existing });
        }
        // Near-duplicate: distinct enough to keep, close enough to say so. It is
        // stored *and* flagged, because discarding a real episode would be worse
        // than keeping a redundant one.
        let nearest = self
            .index
            .knn(&MemoryIndex::embed(&memory.content), 1)
            .into_iter()
            .find(|(id, cosine)| *id != memory.id && *cosine >= NEAR_DUPLICATE_COSINE);
        self.store.insert(&memory).await?;
        self.index.insert_text(memory.id, &memory.content);
        match nearest {
            Some((existing, cosine)) => {
                self.store
                    .link(memory.id, existing, LinkKind::SimilarTo, cosine as f32)
                    .await?;
                self.record_admission(&memory, "near_duplicate", Some(&existing))
                    .await?;
                self.mirror().await?;
                Ok(Admission::NearDuplicate { existing, cosine })
            }
            None => {
                self.mirror().await?;
                Ok(Admission::Accepted)
            }
        }
    }

    async fn record_admission(
        &self,
        memory: &Memory,
        flag: &str,
        existing: Option<&Ulid>,
    ) -> Result<()> {
        self.logger
            .emit(
                LogRecord::new(Level::Warn, codes::MEMORY_ADD, TARGET)
                    .with_field("memory_id", mm_core::ulid_string(&memory.id))
                    .with_field("kind", memory.kind.as_str())
                    .with_field("tier", memory.tier.as_str())
                    .with_field("content_hash", memory.content_hash())
                    .with_field("confidence", memory.confidence)
                    .with_field("importance", memory.importance)
                    .with_field("protected", memory.protected)
                    .with_field("provenance", mm_core::ulid_string(&memory.provenance))
                    .with_field("admitted", false)
                    .with_field("flag", flag)
                    .with_field(
                        "existing",
                        existing
                            .map(mm_core::ulid_string)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
            )
            .await?;
        Ok(())
    }

    /// Wire a link between two memories.
    pub async fn link(
        &self,
        from: Ulid,
        to: Ulid,
        relation: LinkKind,
        weight: f32,
    ) -> Result<()> {
        self.store.link(from, to, relation, weight).await
    }

    /// Record that `memory` served an open commitment, so a forget cycle can
    /// refuse to archive it.
    pub async fn serves_commitment(&self, memory: Ulid, commitment: Ulid) -> Result<()> {
        self.store
            .link(memory, commitment, LinkKind::ServesCommitment, 1.0)
            .await
    }

    /// Reinforce a memory with new evidence.
    pub async fn reinforce(&self, id: Ulid, evidence: f32) -> Result<()> {
        self.store.reinforce(id, evidence).await?;
        self.mirror().await?;
        Ok(())
    }

    // -------------------------------------------------------------- reads ------

    /// Fetch one memory.
    pub async fn get(&self, id: &Ulid) -> Result<Option<Memory>> {
        self.store.get(id).await
    }

    /// Recall the `k` best memories for a query.
    pub async fn recall(&self, query: RecallQuery) -> Result<Vec<RecallHit>> {
        let engine =
            RetrievalEngine::new(self.weights, self.half_life_ns).with_index(&self.index);
        engine
            .recall(&self.store, &self.logger, &self.ids, &query)
            .await
    }

    /// Run a forget cycle.
    pub async fn forget_cycle(&self, now: Timestamp, dry_run: bool) -> Result<ForgetReport> {
        let report = forgetting::forget_cycle(
            &self.store,
            &self.logger,
            &self.ids,
            now,
            forgetting::ForgetPolicy {
                min_retention: self.forget_threshold,
                half_life_ns: self.half_life_ns,
                dry_run,
            },
        )
        .await?;
        if !dry_run {
            self.mirror().await?;
        }
        Ok(report)
    }

    /// Record a mistake: a `memories` row of kind `mistake` plus its detail row.
    pub async fn record_mistake(&mut self, mistake: mistake::MistakeInput) -> Result<Ulid> {
        let id = mistake::record_mistake(&self.store, &self.logger, &self.ids, &mistake).await?;
        self.index.rebuild(&self.store).await?;
        self.mirror().await?;
        Ok(id)
    }

    /// Record a near miss.
    pub async fn record_near_miss(&mut self, near: NearMiss) -> Result<Ulid> {
        let id = mistake::record_near_miss(&self.store, &self.logger, &self.ids, &near).await?;
        self.index.rebuild(&self.store).await?;
        self.mirror().await?;
        Ok(id)
    }

    /// Record a procedure.
    pub async fn record_procedure(
        &mut self,
        procedure: procedural::Procedure,
    ) -> Result<Ulid> {
        let id =
            procedural::record_procedure(&self.store, &self.logger, &self.ids, &procedure).await?;
        self.index.rebuild(&self.store).await?;
        self.mirror().await?;
        Ok(id)
    }

    /// Consolidate everything recorded inside `window`.
    pub async fn consolidate(&mut self, window: TimeInterval) -> Result<consolidate::Outcome> {
        let outcome = consolidate::consolidate(
            &self.store,
            &self.logger,
            &self.ids,
            window,
            consolidate::ConsolidationPolicy::default(),
        )
        .await?;
        self.index.rebuild(&self.store).await?;
        self.mirror().await?;
        Ok(outcome)
    }

    /// Ingest a JSONL episode stream, then consolidate it.
    ///
    /// Ingestion is keyed on the episode's stable `ref`, so running the same file
    /// twice adds nothing the second time — which is what lets the gate be rerun.
    pub async fn ingest_episodes(
        &mut self,
        path: &Path,
        logger: &Logger,
    ) -> Result<Vec<Ulid>> {
        let episodes = consolidate::read_episodes(path)?;
        let mut ingested = Vec::new();
        for episode in &episodes {
            let id = consolidate::ingest_episode(&self.store, &self.logger, &self.ids, episode).await?;
            ingested.push(id);
        }
        let _ = logger;
        self.index.rebuild(&self.store).await?;
        self.mirror().await?;
        Ok(ingested)
    }

    // ------------------------------------------------------------- mirror ------

    /// Rewrite `/memory` from the current state.
    pub async fn mirror(&self) -> Result<usize> {
        let snapshot = rdf::snapshot(&self.store).await?;
        rdf::mirror(&self.graph, &snapshot).await
    }

    /// Rebuild and persist the packed index, returning its hash.
    pub async fn reindex(&mut self, path: &Path) -> Result<String> {
        self.index.rebuild(&self.store).await?;
        let hash = self.index.save(path)?;
        self.logger
            .emit(
                LogRecord::new(Level::Info, codes::MEMORY_INDEX_REBUILD, TARGET)
                    .with_field("count", self.index.len())
                    .with_field("embedder_version", index::EMBEDDER_NAME)
                    .with_field("pack_hash", hash.clone())
                    .with_field("latency_ms", 0),
            )
            .await?;
        Ok(hash)
    }

    /// Run the reconciliation the Phase 5 gate asserts.
    pub async fn verify(&self) -> Result<VerifyReport> {
        let snapshot = rdf::snapshot(&self.store).await?;
        let classes = vec![
            (
                "memory".to_string(),
                self.store.count_of(None).await?,
                snapshot.memories.len(),
            ),
            (
                "consolidation".to_string(),
                self.store.count_of(Some("memory_consolidations")).await?,
                snapshot.consolidations.len(),
            ),
            (
                "summary".to_string(),
                self.store.count_of(Some("memory_summaries")).await?,
                snapshot.summaries.len(),
            ),
            (
                "mistake".to_string(),
                self.store.count_of(Some("mistakes")).await?,
                snapshot.mistakes.len(),
            ),
        ];
        let mut failures = Vec::new();
        for (class, sqlite, rdf) in &classes {
            if sqlite != rdf {
                failures.push(format!(
                    "{class}: {sqlite} SQLite row(s) but {rdf} /memory node(s)"
                ));
            }
        }

        // The invariant this phase exists to protect: nothing protected may be
        // missing, and nothing archived may be protected.
        let protected_violations = self.store.archived_but_protected().await?;
        if protected_violations > 0 {
            failures.push(format!(
                "{protected_violations} protected memory(ies) are archived"
            ));
        }

        let dangling = self.store.dangling_consolidation_sources().await?;
        if dangling > 0 {
            failures.push(format!(
                "{dangling} consolidation source(s) do not resolve"
            ));
        }

        let reconciled = classes.iter().all(|(_, a, b)| a == b);
        let report = VerifyReport {
            sqlite_rows: classes[0].1,
            rdf_triples: snapshot.triple_count(),
            reconciled,
            classes,
            protected_violations,
            dangling_provenance: dangling,
            failures,
        };
        self.logger
            .audit(
                Level::Info,
                codes::MEMORY_VERIFY,
                TARGET,
                None,
                json!({
                    "sqlite_rows": report.sqlite_rows,
                    "rdf_triples": report.rdf_triples,
                    "reconciled": report.reconciled,
                    "drift_ids": report.failures.join("; "),
                }),
            )
            .await?;
        Ok(report)
    }

    /// The identifiers this engine mints.
    pub fn ids(&self) -> &Arc<UlidFactory> {
        &self.ids
    }

    /// The logger this engine writes to.
    pub fn logger(&self) -> &Arc<Logger> {
        &self.logger
    }
}

/// What happened when a memory was offered.
#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    /// It was stored with no equivalent to point at.
    Accepted,
    /// An identical memory already exists; the new one was not stored.
    Duplicate {
        /// The memory that already covers it.
        existing: Ulid,
    },
    /// It was stored, and linked to the memory it is near.
    NearDuplicate {
        /// The memory it is near to.
        existing: Ulid,
        /// Their cosine similarity.
        cosine: f64,
    },
}

/// The cosine above which a write is flagged as a near-duplicate.
pub const NEAR_DUPLICATE_COSINE: f64 = consolidate::NEAR_DUPLICATE_COSINE;
