//! The engine: where a candidate becomes a claim, and a claim changes status.
//!
//! One protocol, in one order, every time:
//!
//! **validate → append → mutate → mirror → commit.**
//!
//! * *validate* — the deterministic [`crate::guard::can_promote`] decides a status
//!   change, and [`crate::validate::ValidationBarrier`] decides admission to
//!   `/world`. Neither consults a model.
//! * *append* — the transition is written to `epistemic_transitions` and audited
//!   before the row changes, so a crash between the two leaves a record of an
//!   attempt, not a silent inconsistency.
//! * *mutate* — the claim's status moves in place.
//! * *mirror* — `/epistemic` and `/provenance` are rewritten from the resulting
//!   snapshot, and `/world` is rewritten through the barrier.
//! * *commit* — the audit record is the commit point.
//!
//! A retraction ([`EpistemicEngine::invalidate`]) is the mirror image: the
//! justification graph gives the whole support set, every dependent is lowered in
//! one deterministic pass, and the cascade is audited as one event.

use std::sync::Arc;

use mm_core::Ulid;
use mm_store_graph::GraphHandle;
use serde_json::json;

use crate::assumption::Assumption;
use crate::claim::{Claim, Evidence};
use crate::contradiction::{Contradiction, IndexedContradictionDetector};
use crate::error::{EpistemicError, Result};
use crate::guard::can_promote;
use crate::justification::JustificationGraph;
use crate::rdf::{self, EpistemicSnapshot};
use crate::status::EpistemicStatus;
use crate::store::{SqliteEpistemicStore, Transition};
use crate::validate::ValidationBarrier;
use crate::TARGET;

/// The engine the CLI drives.
pub struct EpistemicEngine {
    store: Arc<SqliteEpistemicStore>,
    graph: GraphHandle,
}

impl std::fmt::Debug for EpistemicEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpistemicEngine")
            .field("path", &self.store.sqlite().path())
            .finish_non_exhaustive()
    }
}

impl EpistemicEngine {
    /// Build an engine over a store and a graph handle.
    pub fn new(store: Arc<SqliteEpistemicStore>, graph: GraphHandle) -> Self {
        EpistemicEngine { store, graph }
    }

    /// The store.
    pub fn store(&self) -> &Arc<SqliteEpistemicStore> {
        &self.store
    }

    /// The graph handle.
    pub fn graph(&self) -> &GraphHandle {
        &self.graph
    }

    /// Ingest a candidate claim and its evidence.
    ///
    /// A claim that is already world-admissible is written to `/world` through the
    /// barrier and audited; every other claim stays in `/epistemic` only.
    pub async fn ingest(&self, claim: &Claim, evidence: &[Evidence]) -> Result<Ulid> {
        claim.validate()?;
        self.store.insert_claim(claim, evidence).await?;
        if claim.is_world_admissible() {
            ValidationBarrier::check(claim)?;
            self.store
                .audit(
                    mm_log::codes::EPISTEMIC_WORLD_WRITE,
                    claim.id,
                    json!({
                        "claim_id": mm_core::ulid_string(&claim.id),
                        "status": claim.status.as_str(),
                        "allowed": true,
                        "graph": rdf::WORLD_GRAPH,
                    }),
                )
                .await?;
        }
        self.store
            .audit(
                mm_log::codes::EPISTEMIC_CLAIM_INGEST,
                claim.id,
                json!({
                    "claim_id": mm_core::ulid_string(&claim.id),
                    "kind": claim.kind.as_str(),
                    "status": claim.status.as_str(),
                    "confidence": claim.confidence,
                    "graph": rdf::EPISTEMIC_GRAPH,
                }),
            )
            .await?;
        for item in evidence {
            self.store
                .audit(
                    mm_log::codes::EPISTEMIC_EVIDENCE_ATTACH,
                    claim.id,
                    json!({
                        "claim_id": mm_core::ulid_string(&claim.id),
                        "evidence_id": mm_core::ulid_string(&item.id),
                        "content_hash": item.content_hash_hex(),
                        "reliability": item.reliability,
                    }),
                )
                .await?;
        }
        self.mirror().await?;
        Ok(claim.id)
    }

    /// Attach evidence and promote.
    pub async fn verify(&self, id: &Ulid, evidence: &Evidence, to: EpistemicStatus) -> Result<()> {
        let Some(claim) = self.store.fetch_claim(id).await? else {
            return Err(EpistemicError::NotFound(mm_core::ulid_string(id)));
        };
        self.store.insert_evidence(id, evidence).await?;
        let mut held = self.store.evidence_for(id).await?;
        if !held.iter().any(|e| e.id == evidence.id) {
            held.push(evidence.clone());
        }
        if let Err(denied) = can_promote(claim.status, to, &held) {
            self.store
                .audit(
                    mm_log::codes::EPISTEMIC_PROMOTION_REJECT,
                    *id,
                    json!({
                        "claim_id": mm_core::ulid_string(id),
                        "from": denied.from.as_str(),
                        "to": denied.to.as_str(),
                        "reason": denied.reason,
                    }),
                )
                .await?;
            return Err(EpistemicError::Promotion(denied));
        }
        self.transition(id, claim.status, to, "evidence attached", Some(evidence.id))
            .await
    }

    /// Change a claim's status through the guard.
    pub async fn set_status(&self, id: &Ulid, to: EpistemicStatus, reason: &str) -> Result<()> {
        let Some(claim) = self.store.fetch_claim(id).await? else {
            return Err(EpistemicError::NotFound(mm_core::ulid_string(id)));
        };
        let held = self.store.evidence_for(id).await?;
        if let Err(denied) = can_promote(claim.status, to, &held) {
            self.store
                .audit(
                    mm_log::codes::EPISTEMIC_PROMOTION_REJECT,
                    *id,
                    json!({
                        "claim_id": mm_core::ulid_string(id),
                        "from": denied.from.as_str(),
                        "to": denied.to.as_str(),
                        "reason": denied.reason,
                    }),
                )
                .await?;
            return Err(EpistemicError::Promotion(denied));
        }
        self.transition(id, claim.status, to, reason, None).await
    }

    async fn transition(
        &self,
        id: &Ulid,
        from: EpistemicStatus,
        to: EpistemicStatus,
        reason: &str,
        evidence: Option<Ulid>,
    ) -> Result<()> {
        // Append before mutating: a crash leaves a recorded attempt, not a silent
        // change.
        let activity = self.store.next_id();
        let transition = Transition {
            id: self.store.next_id(),
            subject: *id,
            from_status: Some(from),
            to_status: Some(to),
            reason: reason.to_string(),
            evidence,
            prov_activity: Some(activity),
        };
        self.store.insert_transition(&transition).await?;
        self.store.update_claim_status(id, to).await?;
        self.store
            .audit(
                mm_log::codes::EPISTEMIC_STATUS_TRANSITION,
                transition.id,
                json!({
                    "subject": mm_core::ulid_string(id),
                    "from_status": from.as_str(),
                    "to_status": to.as_str(),
                    "evidence_id": evidence.map(|e| mm_core::ulid_string(&e)),
                    "prov_activity": mm_core::ulid_string(&activity),
                }),
            )
            .await?;
        self.mirror().await?;
        Ok(())
    }

    /// Withdraw a claim's support and retract everything that rested on it.
    ///
    /// The affected set is the dependency-directed retraction set from the
    /// justification graph, in ULID order. Each affected claim is set `UNKNOWN`
    /// and audited; the cascade is one audit event naming the set.
    pub async fn invalidate(&self, id: &Ulid, reason: &str) -> Result<Vec<Ulid>> {
        let edges = self.store.list_justifications().await?;
        let graph = JustificationGraph::from_edges(edges)
            .map_err(|e| EpistemicError::Internal(format!("justification graph: {e}")))?;
        let cascade = graph.cascade(id);
        let mut affected = Vec::new();
        for node in cascade.all() {
            let Some(claim) = self.store.fetch_claim(&node).await? else {
                continue;
            };
            let transition = Transition {
                id: self.store.next_id(),
                subject: node,
                from_status: Some(claim.status),
                to_status: Some(EpistemicStatus::Unknown),
                reason: reason.to_string(),
                evidence: None,
                prov_activity: Some(self.store.next_id()),
            };
            self.store.insert_transition(&transition).await?;
            self.store
                .update_claim_status(&node, EpistemicStatus::Unknown)
                .await?;
            affected.push(node);
        }
        self.store
            .audit(
                mm_log::codes::EPISTEMIC_CASCADE,
                *id,
                json!({
                    "root": mm_core::ulid_string(id),
                    "affected_ids": affected
                        .iter()
                        .map(mm_core::ulid_string)
                        .collect::<Vec<_>>(),
                    "count": affected.len(),
                }),
            )
            .await?;
        self.mirror().await?;
        Ok(affected)
    }

    /// Detect and record every contradiction among the stored claims.
    pub async fn detect_contradictions(&self) -> Result<Vec<Contradiction>> {
        let claims = self.store.list_all_claims().await?;
        let detector = IndexedContradictionDetector::new();
        let mut index = crate::contradiction::ConflictIndex::new();
        let mut created = Vec::new();
        for claim in &claims {
            for contradiction in detector.check_indexed(claim, &index) {
                if self.store.insert_contradiction(&contradiction).await? {
                    self.store
                        .audit(
                            mm_log::codes::EPISTEMIC_CONTRADICTION_DETECT,
                            contradiction.id,
                            json!({
                                "contradiction_id": mm_core::ulid_string(&contradiction.id),
                                "claim_a": mm_core::ulid_string(&contradiction.claim_a),
                                "claim_b": mm_core::ulid_string(&contradiction.claim_b),
                                "reason": contradiction.reason,
                                "formal_check": "unavailable",
                            }),
                        )
                        .await?;
                    created.push(contradiction);
                }
            }
            index.insert(claim.clone());
        }
        self.mirror().await?;
        Ok(created)
    }

    /// Store an assumption and audit it.
    pub async fn assume(&self, assumption: &Assumption, p_false: f64) -> Result<()> {
        self.store.insert_assumption(assumption).await?;
        self.store
            .audit(
                mm_log::codes::EPISTEMIC_ASSUMPTION_CREATE,
                assumption.id,
                json!({
                    "assumption_id": mm_core::ulid_string(&assumption.id),
                    "consequence_if_false": assumption.consequence_if_false.as_str(),
                    "verification_cost": assumption.verification_cost,
                }),
            )
            .await?;
        let priority = crate::assumption::verification_priority(assumption, p_false);
        self.store
            .emit(
                mm_log::codes::EPISTEMIC_ASSUMPTION_PRIORITY,
                assumption.id,
                json!({
                    "assumption_id": mm_core::ulid_string(&assumption.id),
                    "p_false": p_false,
                    "risk": assumption.consequence_if_false.weight(),
                    "decision_dependence": assumption.decision_dependence,
                    "priority": priority,
                }),
            )
            .await?;
        self.mirror().await?;
        Ok(())
    }

    /// Build the current snapshot.
    pub async fn snapshot(&self) -> Result<EpistemicSnapshot> {
        let claims = self.store.list_all_claims().await?;
        let mut evidence = Vec::new();
        for claim in &claims {
            for item in self.store.evidence_for(&claim.id).await? {
                evidence.push((claim.id, item));
            }
        }
        Ok(EpistemicSnapshot {
            claims,
            evidence,
            assumptions: self.store.list_assumptions().await?,
            predictions: self.store.list_predictions().await?,
            contradictions: self.store.list_contradictions().await?,
            justifications: self.store.list_justifications().await?,
            transitions: self.store.list_transitions().await?,
        })
    }

    /// Rewrite `/epistemic`, `/provenance`, and `/world`.
    pub async fn mirror(&self) -> Result<usize> {
        let snapshot = self.snapshot().await?;
        let written = rdf::mirror(&self.graph, &snapshot).await?;
        let admissible: Vec<Claim> = snapshot.admissible().into_iter().cloned().collect();
        ValidationBarrier::mirror_world(&self.graph, &admissible).await?;
        self.store
            .emit(
                mm_log::codes::EPISTEMIC_JUSTIFICATION_EDGE,
                Ulid::nil(),
                json!({ "target": TARGET, "written": written }),
            )
            .await
            .ok();
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::{ClaimKind, EvidenceKind};
    use crate::proposition::Proposition;
    use mm_core::{Config, UlidFactory};
    use mm_log::{Level, Logger, RedactionPolicy};
    use mm_store_sqlite::SqliteStore;

    async fn engine() -> (
        EpistemicEngine,
        mm_store_graph::GraphStore,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path());
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        sqlite.migrate().await.unwrap();
        let shapes = Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl");
        let graph = mm_store_graph::GraphStore::in_memory(&shapes)
            .await
            .unwrap();
        let logger = Arc::new(Logger::new(
            Level::Trace,
            vec![],
            Some(Arc::new(sqlite.clone())),
            RedactionPolicy::kernel_default(),
        ));
        let ids = Arc::new(UlidFactory::new().with_persist_every(1_000_000));
        let store = Arc::new(SqliteEpistemicStore::new(sqlite, logger, ids));
        let engine = EpistemicEngine::new(store, graph.handle().clone());
        (engine, graph, dir)
    }

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    #[tokio::test]
    async fn a_report_is_stored_in_epistemic_only() {
        let (engine, _graph, _dir) = engine().await;
        let claim = Claim::new(
            id(1),
            ClaimKind::Fact,
            Proposition::literal("https://x/s", "https://x/p", "v").unwrap(),
            EpistemicStatus::Reported,
            0.5,
        )
        .unwrap();
        engine.ingest(&claim, &[]).await.unwrap();
        let snapshot = engine.snapshot().await.unwrap();
        assert_eq!(snapshot.claims.len(), 1);
        assert!(snapshot.admissible().is_empty());
    }

    #[tokio::test]
    async fn a_forbidden_promotion_is_refused_and_audited() {
        let (engine, _graph, _dir) = engine().await;
        let claim = Claim::new(
            id(1),
            ClaimKind::Fact,
            Proposition::literal("https://x/s", "https://x/p", "v").unwrap(),
            EpistemicStatus::Assumed,
            0.5,
        )
        .unwrap();
        engine.ingest(&claim, &[]).await.unwrap();
        let error = engine
            .set_status(&id(1), EpistemicStatus::Observed, "wishful")
            .await
            .unwrap_err();
        assert!(error.is_promotion());
        // The status did not move.
        let held = engine.store().fetch_claim(&id(1)).await.unwrap().unwrap();
        assert_eq!(held.status, EpistemicStatus::Assumed);
    }

    #[tokio::test]
    async fn evidence_lifts_a_report_and_writes_the_world() {
        let (engine, _graph, _dir) = engine().await;
        let claim = Claim::new(
            id(1),
            ClaimKind::Fact,
            Proposition::literal("https://x/s", "https://x/p", "v").unwrap(),
            EpistemicStatus::Inferred,
            0.6,
        )
        .unwrap();
        engine.ingest(&claim, &[]).await.unwrap();
        let evidence =
            Evidence::from_content(id(9), EvidenceKind::ExternalTool, None, "proof", 0.99).unwrap();
        engine
            .verify(&id(1), &evidence, EpistemicStatus::Verified)
            .await
            .unwrap();
        let snapshot = engine.snapshot().await.unwrap();
        assert_eq!(snapshot.admissible().len(), 1);
    }

    #[tokio::test]
    async fn invalidation_retracts_the_transitive_dependents() {
        let (engine, _graph, _dir) = engine().await;
        for n in 1..=3 {
            let claim = Claim::new(
                id(n),
                ClaimKind::Inference,
                Proposition::literal("https://x/s", "https://x/p", &format!("v{n}")).unwrap(),
                EpistemicStatus::Inferred,
                0.5,
            )
            .unwrap();
            engine.ingest(&claim, &[]).await.unwrap();
        }
        // 2 rests on 1, 3 rests on 2.
        engine
            .store()
            .insert_justification(
                &crate::justification::Justification::new(
                    id(2),
                    vec![id(1)],
                    crate::justification::DepKind::Derivation,
                    0.5,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        engine
            .store()
            .insert_justification(
                &crate::justification::Justification::new(
                    id(3),
                    vec![id(2)],
                    crate::justification::DepKind::Derivation,
                    0.5,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let affected = engine.invalidate(&id(1), "source retracted").await.unwrap();
        assert_eq!(affected, vec![id(1), id(2), id(3)]);
    }
}
