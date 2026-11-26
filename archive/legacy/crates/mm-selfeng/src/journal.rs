//! The evolution journal: the lineage of the self.
//!
//! An accepted change is not only a new capability; it is a new *version* of the
//! system, and the plan asks for the record to be immutable and append-only so the
//! question "which version was I, and why" has an answer that cannot be edited
//! afterwards. Two mechanisms carry that, and both are load-bearing:
//!
//! * **The database refuses UPDATE and DELETE** on `evolution_journal` (migration
//!   `0011_self_engineering.sql`). A journal that the same process could rewrite is a
//!   journal that proves nothing about that process.
//! * **`self_version` is derived, not assigned.** It is a function of the journal
//!   itself — the number of entries and the content hash of the accepted change sets —
//!   so a version is the lineage rather than a build number someone typed. Two systems
//!   with the same history read the same version, and a history that is edited cannot
//!   quietly keep its version.
//!
//! The `/selfeng` mirror at the end of this module is a projection of the same rows for
//! an operator or a SHACL gate, and it uses only the predicates declared in
//! `ontology/selfeng.ttl`. One link the plan implies is deliberately *absent*: there is
//! no predicate joining an `mm:EvolutionEvent` to its `mm:ChangeSet`, and rather than
//! invent an undeclared one the event carries the decision, the reason and the version,
//! which is what a reader of the lineage actually asks for. The join itself is the
//! `changeset_ulid` column.

use std::sync::Arc;

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::changeset::ChangeSetStatus;
use crate::error::{Result, SelfEngError};

/// The named graph the lineage is mirrored into.
///
/// Phase 11's own graph rather than the plan's `/provenance`, because `/provenance` is
/// the epistemic mirror's scratch graph and is rewritten on every claim mutation; the
/// reasoning is recorded in `mm-core::iri` next to the graph's name.
pub const SELFENG_GRAPH: &str = "selfeng";

/// One entry of the journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvolutionEvent {
    /// Its ULID.
    pub id: Ulid,
    /// The day it was written, so the lineage reads as a history.
    pub date: String,
    /// Why the change existed.
    pub reason: String,
    /// The evidence the decision was made on, as JSON.
    pub evidence_json: String,
    /// What the change tested.
    pub hypothesis: String,
    /// The change set it judged, when there was one.
    pub changeset: Option<Ulid>,
    /// What happened.
    pub outcome: String,
    /// `promote` or `reject`.
    pub decision: String,
    /// The version the decision produced.
    pub self_version: String,
}

impl EvolutionEvent {
    /// An entry with the identifier minted by the factory.
    pub fn new(
        ids: &UlidFactory,
        reason: impl Into<String>,
        hypothesis: impl Into<String>,
        outcome: impl Into<String>,
        decision: impl Into<String>,
    ) -> Self {
        EvolutionEvent {
            id: ids.next(),
            date: today(),
            reason: reason.into(),
            evidence_json: "{}".to_string(),
            hypothesis: hypothesis.into(),
            changeset: None,
            outcome: outcome.into(),
            decision: decision.into(),
            self_version: String::new(),
        }
    }

    /// Attach the evidence bundle's JSON.
    pub fn with_evidence(mut self, evidence: impl Into<String>) -> Self {
        self.evidence_json = evidence.into();
        self
    }

    /// Attach the change set it judges.
    pub fn with_changeset(mut self, id: Ulid) -> Self {
        self.changeset = Some(id);
        self
    }
}

/// The version a lineage of `count` entries with `accepted` change sets reads as.
fn version_of(count: i64, accepted: &[String]) -> String {
    let digest = mm_core::content_hash(accepted.join(",").as_bytes());
    format!(
        "self-{count}-{}",
        digest.chars().take(12).collect::<String>()
    )
}

/// The day, which is what `date` records.
pub fn today() -> String {
    let rfc3339 = Timestamp::now().to_rfc3339();
    rfc3339.chars().take(10).collect()
}

/// The journal over the `evolution_journal` table.
pub struct EvolutionJournal {
    store: mm_store_sqlite::SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
}

impl EvolutionJournal {
    /// A journal over the kernel's tabular store.
    pub fn new(
        store: mm_store_sqlite::SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
    ) -> Self {
        EvolutionJournal { store, logger, ids }
    }

    /// The store the journal writes to, so a caller already holding a journal does not
    /// have to be handed the same store twice.
    pub fn store(&self) -> &mm_store_sqlite::SqliteStore {
        &self.store
    }

    /// The journal's logger, for the same reason.
    pub fn logger(&self) -> &Logger {
        &self.logger
    }

    /// Append one entry.
    pub async fn append(&self, event: &EvolutionEvent) -> Result<Ulid> {
        if event.reason.trim().is_empty() {
            return Err(SelfEngError::validation("reason", "must not be empty"));
        }
        // The journal records a decision, and the phase has exactly two. A third
        // spelling would produce an entry the gate's own reader could not classify.
        let decision_code = match event.decision.as_str() {
            "promote" => codes::PROMOTE_COMMIT,
            "reject" => codes::PROMOTE_REJECT,
            other => {
                return Err(SelfEngError::validation(
                    "decision",
                    format!("must be promote or reject, got {other:?}"),
                ))
            }
        };
        if event.self_version.trim().is_empty() {
            return Err(SelfEngError::validation(
                "self_version",
                "an entry that cannot name the version it produced is not a lineage",
            ));
        }
        // A caller that has no identifier to hand (the gate builds the entry it is
        // about to append) gets one here rather than being forced to mint it first.
        let id = if event.id.is_nil() {
            self.ids.next()
        } else {
            event.id
        };
        sqlx::query(
            "INSERT INTO evolution_journal \
             (id, date, reason, evidence_json, hypothesis, changeset_ulid, outcome, decision, self_version) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(mm_core::ulid_string(&id))
        .bind(&event.date)
        .bind(&event.reason)
        .bind(&event.evidence_json)
        .bind(&event.hypothesis)
        .bind(event.changeset.map(|id| mm_core::ulid_string(&id)))
        .bind(&event.outcome)
        .bind(&event.decision)
        .bind(&event.self_version)
        .execute(self.store.pool())
        .await
        .map_err(|e| SelfEngError::Store(format!("cannot append to the journal: {e}")))?;
        // The journal append is what carries the version the decision produced, which
        // is why `promote.commit` / `promote.reject` are recorded here rather than in
        // `promotion::record_decision` — that one records `gate.decide`, and the two
        // records must not both claim to be the decision.
        self.logger
            .audit(
                Level::Info,
                decision_code,
                crate::TARGET,
                Some(id),
                json!({
                    "journal_id": mm_core::ulid_string(&id),
                    "change_set_id": event
                        .changeset
                        .map(|id| mm_core::ulid_string(&id))
                        .unwrap_or_else(|| "-".to_string()),
                    "decision": event.decision,
                    "reason": event.reason,
                    "self_version": event.self_version,
                }),
            )
            .await?;
        Ok(id)
    }

    /// The version the lineage currently reads as.
    ///
    /// `self-{entries}-{hash}`: the count so a version moves when anything is appended,
    /// and the hash of the accepted change sets so two histories with the same length
    /// are still different versions. Deterministic and derived — nothing here reads a
    /// clock or a counter that lives outside the journal.
    pub async fn self_version(&self) -> Result<String> {
        let count = self.count().await?;
        let accepted = self.accepted().await?;
        Ok(version_of(count, &accepted))
    }

    /// The version the *next* decision would produce.
    ///
    /// Needed because an entry must carry the version it produced, and the version
    /// depends on the entry: computing it first and appending afterwards is what breaks
    /// the circle, and it is well defined because the journal is append-only. A
    /// `promote` adds its change set to the accepted set; a `reject` does not.
    pub async fn next_version(&self, decision: &str, change_set: Ulid) -> Result<String> {
        let count = self.count().await?;
        let mut accepted = self.accepted().await?;
        if decision == "promote" {
            let id = mm_core::ulid_string(&change_set);
            if !accepted.contains(&id) {
                accepted.push(id);
                accepted.sort();
            }
        }
        Ok(version_of(count + 1, &accepted))
    }

    /// How many entries the journal holds.
    async fn count(&self) -> Result<i64> {
        sqlx::query_scalar("SELECT count(*) FROM evolution_journal")
            .fetch_one(self.store.pool())
            .await
            .map_err(|e| SelfEngError::Store(format!("cannot count the journal: {e}")))
    }

    /// The change sets the lineage accepted, sorted.
    async fn accepted(&self) -> Result<Vec<String>> {
        sqlx::query_scalar(
            "SELECT changeset_ulid FROM evolution_journal \
             WHERE decision = 'promote' AND changeset_ulid IS NOT NULL \
             ORDER BY changeset_ulid",
        )
        .fetch_all(self.store.pool())
        .await
        .map_err(|e| SelfEngError::Store(format!("cannot read the accepted changes: {e}")))
    }

    /// Every entry, in the order it was written.
    pub async fn events(&self) -> Result<Vec<EvolutionEvent>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            String,
            String,
        )> = sqlx::query_as(
            "SELECT id, date, reason, evidence_json, hypothesis, changeset_ulid, outcome, \
                 decision, self_version FROM evolution_journal ORDER BY date, id",
        )
        .fetch_all(self.store.pool())
        .await
        .map_err(|e| SelfEngError::Store(format!("cannot read the journal: {e}")))?;
        rows.into_iter()
            .map(|row| {
                let id = mm_core::id::parse_ulid(&row.0)
                    .map_err(|e| SelfEngError::Store(format!("journal id: {e}")))?;
                let changeset = match row.5 {
                    None => None,
                    Some(text) => Some(
                        mm_core::id::parse_ulid(&text)
                            .map_err(|e| SelfEngError::Store(format!("journal change set: {e}")))?,
                    ),
                };
                Ok(EvolutionEvent {
                    id,
                    date: row.1,
                    reason: row.2,
                    evidence_json: row.3,
                    hypothesis: row.4,
                    changeset,
                    outcome: row.6,
                    decision: row.7,
                    self_version: row.8,
                })
            })
            .collect()
    }

    /// Write the lineage into `/selfeng`, returning how many triples landed.
    pub async fn mirror(&self, graph: &mm_store_graph::GraphStore) -> Result<usize> {
        let change_sets = self.change_sets_for_mirror().await?;
        let events = self.events().await?;
        let turtle = mirror_turtle(&change_sets, &events)?;
        let inserted = graph
            .handle()
            .insert_turtle(SELFENG_GRAPH, &turtle)
            .await
            .map_err(|e| SelfEngError::Graph(e.to_string()))?;
        Ok(inserted)
    }

    /// The change sets and statuses the mirror emits, oldest first.
    async fn change_sets_for_mirror(
        &self,
    ) -> Result<Vec<(String, String, String, String, String)>> {
        let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, reason, hypothesis, rollback_json, status FROM change_sets \
             ORDER BY created_at, id",
        )
        .fetch_all(self.store.pool())
        .await
        .map_err(|e| SelfEngError::Store(format!("cannot read the change sets: {e}")))?;
        Ok(rows)
    }
}

/// The Turtle document for the lineage.
///
/// One statement per property, so the text is a function of the rows alone: the same
/// rows render byte-identically, which is what makes re-ingesting a cycle a no-op diff
/// rather than a new version of the graph.
pub fn mirror_turtle(
    change_sets: &[(String, String, String, String, String)],
    events: &[EvolutionEvent],
) -> Result<String> {
    let mut out = String::from(
        "@prefix mm:   <https://metamind.dev/ontology#> .\n\
         @prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .\n\n",
    );
    for (id, reason, hypothesis, rollback_json, status) in change_sets {
        let status = ChangeSetStatus::parse(status)
            .ok_or_else(|| SelfEngError::Graph(format!("unknown change-set status {status:?}")))?
            .as_str()
            .to_string();
        // The rollback plan is stored as JSON; the graph carries the plan in words, so
        // a reader of the graph sees the same steps a reader of `promote` would.
        let steps: Vec<String> =
            serde_json::from_str::<crate::changeset::RollbackPlan>(rollback_json)
                .map(|plan| plan.steps)
                .unwrap_or_default();
        let rollback_text = if steps.is_empty() {
            "no steps recorded".to_string()
        } else {
            steps.join("; ")
        };
        let subject = format!("<{}>", mm_core::iri::data(&parse_ulid(id)?).into_string());
        out.push_str(&format!("{subject} a mm:ChangeSet ;\n"));
        out.push_str(&format!("    mm:changesetReason {} ;\n", literal(reason)));
        out.push_str(&format!("    mm:hypothesis {} ;\n", literal(hypothesis)));
        out.push_str(&format!(
            "    mm:rollbackPlan {} ;\n",
            literal(&rollback_text)
        ));
        out.push_str(&format!(
            "    mm:changesetStatus {} .\n\n",
            literal(&status)
        ));
    }
    for event in events {
        let subject = format!("<{}>", mm_core::iri::data(&event.id).into_string());
        out.push_str(&format!("{subject} a mm:EvolutionEvent ;\n"));
        out.push_str(&format!(
            "    mm:promotionDecision {} ;\n",
            literal(&event.decision)
        ));
        out.push_str(&format!(
            "    mm:promotionReason {} ;\n",
            literal(&event.reason)
        ));
        out.push_str(&format!(
            "    mm:selfVersion {} ;\n",
            literal(&event.self_version)
        ));
        out.push_str(&format!("    mm:journalDate {} ;\n", literal(&event.date)));
        out.push_str(&format!(
            "    mm:journalOutcome {} .\n\n",
            literal(&event.outcome)
        ));
    }
    Ok(out)
}

fn parse_ulid(text: &str) -> Result<Ulid> {
    mm_core::id::parse_ulid(text).map_err(|e| SelfEngError::Graph(format!("{text}: {e}")))
}

/// A Turtle string literal, escaped.
fn literal(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 2);
    escaped.push('"');
    for c in text.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;
    use mm_store_sqlite::SqliteStore;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    async fn journal(dir: &std::path::Path) -> (SqliteStore, EvolutionJournal) {
        let store = SqliteStore::open(&dir.join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir);
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let journal = EvolutionJournal::new(store.clone(), logger, Arc::new(UlidFactory::new()));
        (store, journal)
    }

    fn event(journal: &EvolutionJournal, decision: &str) -> EvolutionEvent {
        EvolutionEvent {
            id: journal.ids.next(),
            date: "2026-10-09".to_string(),
            reason: "the calibration bin drifted".to_string(),
            evidence_json: "{\"brier\":0.176200009}".to_string(),
            hypothesis: "a summary makes the drift visible".to_string(),
            changeset: Some(ulid(1)),
            outcome: "the bench moved by 0.02".to_string(),
            decision: decision.to_string(),
            self_version: "self-1-abc".to_string(),
        }
    }

    #[tokio::test]
    async fn an_appended_entry_reads_back_and_the_table_refuses_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let (store, journal) = journal(dir.path()).await;
        let entry = event(&journal, "promote");
        journal.append(&entry).await.unwrap();
        let events = journal.events().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], entry);

        // The database refuses an update and a delete: the lineage is append-only even
        // for a writer that never goes through this crate.
        let update = sqlx::query("UPDATE evolution_journal SET decision = 'reject' WHERE id = ?")
            .bind(mm_core::ulid_string(&entry.id))
            .execute(store.pool())
            .await;
        assert!(update.is_err(), "the journal must refuse an edit");
        assert!(sqlx::query("DELETE FROM evolution_journal")
            .execute(store.pool())
            .await
            .is_err());
        assert_eq!(journal.events().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_next_version_is_defined_before_the_entry_exists() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, journal) = journal(dir.path()).await;
        // A reject does not move the accepted set, so its version is the current one
        // with the count advanced.
        let next_reject = journal.next_version("reject", ulid(9)).await.unwrap();
        assert!(next_reject.starts_with("self-1-"), "{next_reject}");
        // A promote does, and two different change sets give two different versions.
        let promoted_a = journal.next_version("promote", ulid(1)).await.unwrap();
        let promoted_b = journal.next_version("promote", ulid(2)).await.unwrap();
        assert_ne!(promoted_a, promoted_b);
        // And the version the decision announced is the one a subsequent read agrees
        // with once the entry is appended.
        let mut entry = event(&journal, "promote");
        entry.changeset = Some(ulid(1));
        entry.self_version = promoted_a.clone();
        journal.append(&entry).await.unwrap();
        assert_eq!(promoted_a, journal.self_version().await.unwrap());
    }

    #[tokio::test]
    async fn the_self_version_is_derived_from_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, journal) = journal(dir.path()).await;
        let empty = journal.self_version().await.unwrap();
        assert!(empty.starts_with("self-0-"), "{empty}");

        journal.append(&event(&journal, "reject")).await.unwrap();
        let after_reject = journal.self_version().await.unwrap();
        assert!(after_reject.starts_with("self-1-"), "{after_reject}");
        assert_ne!(empty, after_reject);

        let mut accepted = event(&journal, "promote");
        accepted.changeset = Some(ulid(2));
        journal.append(&accepted).await.unwrap();
        let after_promote = journal.self_version().await.unwrap();
        assert!(after_promote.starts_with("self-2-"), "{after_promote}");
        // Deterministic: the same journal reads the same version twice.
        assert_eq!(after_promote, journal.self_version().await.unwrap());
        // And a rejected entry does not change *which* changes are accepted: the same
        // accepted set yields the same digest half.
        let digest = |version: &str| version.split('-').nth(2).unwrap().to_string();
        assert_eq!(digest(&after_promote).len(), 12);
        assert_eq!(digest(&after_reject).len(), 12);
    }

    #[tokio::test]
    async fn an_entry_with_a_decision_that_is_neither_promote_nor_reject_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, journal) = journal(dir.path()).await;
        let mut bad = event(&journal, "maybe");
        bad.self_version = "self-1-000000000000".to_string();
        let error = journal.append(&bad).await.unwrap_err();
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("promote or reject"), "{error}");
    }

    #[tokio::test]
    async fn an_entry_without_a_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, journal) = journal(dir.path()).await;
        let mut bad = event(&journal, "promote");
        bad.self_version = String::new();
        let error = journal.append(&bad).await.unwrap_err();
        assert_eq!(error.code(), "validation");
        assert!(journal.events().await.unwrap().is_empty());
    }

    #[test]
    fn the_mirror_uses_only_the_declared_predicates() {
        let change_sets = vec![(
            mm_core::ulid_string(&ulid(1)),
            "reason".to_string(),
            "hypothesis".to_string(),
            serde_json::to_string(&crate::changeset::RollbackPlan {
                steps: vec!["delete a/b.txt".to_string()],
                state_hash: "deadbeef".to_string(),
            })
            .unwrap(),
            "draft".to_string(),
        )];
        let events = vec![EvolutionEvent {
            id: ulid(2),
            date: "2026-10-09".to_string(),
            reason: "because".to_string(),
            evidence_json: "{}".to_string(),
            hypothesis: "h".to_string(),
            changeset: Some(ulid(1)),
            outcome: "nothing moved".to_string(),
            decision: "reject".to_string(),
            self_version: "self-1-000000000000".to_string(),
        }];
        let turtle = mirror_turtle(&change_sets, &events).unwrap();
        for predicate in [
            "mm:changesetReason",
            "mm:hypothesis",
            "mm:rollbackPlan",
            "mm:changesetStatus",
            "mm:promotionDecision",
            "mm:promotionReason",
            "mm:selfVersion",
            "mm:journalDate",
            "mm:journalOutcome",
        ] {
            assert!(
                turtle.contains(predicate),
                "{predicate} missing from\n{turtle}"
            );
        }
        assert!(turtle.contains("a mm:ChangeSet"));
        assert!(turtle.contains("a mm:EvolutionEvent"));
        assert!(turtle.contains("delete a/b.txt"));
        // Deterministic: the same rows render the same bytes.
        assert_eq!(turtle, mirror_turtle(&change_sets, &events).unwrap());
    }

    #[test]
    fn a_literal_with_a_quote_or_a_newline_is_escaped() {
        let change_sets = vec![(
            mm_core::ulid_string(&ulid(1)),
            "a \"quoted\" reason\nwith a newline\\".to_string(),
            "h".to_string(),
            "{}".to_string(),
            "draft".to_string(),
        )];
        let turtle = mirror_turtle(&change_sets, &[]).unwrap();
        assert!(turtle.contains("\\\"quoted\\\""), "{turtle}");
        assert!(turtle.contains("\\n"), "{turtle}");
        assert!(turtle.contains("\\\\"), "{turtle}");
    }

    #[test]
    fn an_unknown_status_is_refused_rather_than_mirrored() {
        let change_sets = vec![(
            mm_core::ulid_string(&ulid(1)),
            "r".to_string(),
            "h".to_string(),
            "{}".to_string(),
            "nonesuch".to_string(),
        )];
        let error = mirror_turtle(&change_sets, &[]).unwrap_err();
        assert_eq!(error.code(), "graph");
    }

    #[tokio::test]
    async fn mirroring_the_lineage_lands_in_the_selfeng_graph() {
        let dir = tempfile::tempdir().unwrap();
        let (store, journal) = journal(dir.path()).await;
        journal.append(&event(&journal, "promote")).await.unwrap();
        sqlx::query(
            "INSERT INTO change_sets (id, reason, hypothesis, payload_json, rollback_json, status, created_at) \
             VALUES (?, ?, ?, ?, ?, 'promoted', '2026-10-09T00:00:00.000000000Z')",
        )
        .bind(mm_core::ulid_string(&ulid(1)))
        .bind("the calibration bin drifted")
        .bind("a summary makes the drift visible")
        .bind("{}")
        .bind("{\"steps\":[\"delete x\"],\"state_hash\":\"deadbeef\"}")
        .execute(store.pool())
        .await
        .unwrap();

        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("selfeng.ttl");
        let graph = mm_store_graph::GraphStore::in_memory(&shapes)
            .await
            .unwrap();
        let inserted = journal.mirror(&graph).await.unwrap();
        assert!(inserted >= 9, "the mirror wrote {inserted} triples");
        let rows = graph
            .handle()
            .sparql(
                SELFENG_GRAPH,
                "SELECT ?s ?p ?o WHERE { GRAPH <https://metamind.dev/graph/selfeng> { ?s ?p ?o } }",
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), inserted);
    }
}
