//! Cognitive GC: the escalation ladder, and the guard that protects the ledger.
//!
//! Design §86 asks for collection that is *reversible* and that never touches the
//! immutable developmental ledger. Those two requirements shape the module:
//!
//! * **The ladder is ordered cheapest-first.** [`GC_LADDER`] is the nine rungs of §86, and
//!   [`rung_for`] maps a debt kind to the lowest rung that plausibly resolves it. Anything
//!   at or above `Code` is itself a change set through the promotion gate, which is why
//!   [`GcActionKind::requires_change_set`] exists: the collector proposes, and the gate
//!   decides.
//! * **The guard runs before the action is recorded as applied, not after.** A finding
//!   whose subject a table says is protected produces an action row with
//!   `protects_ledger = 1`, a `gc.refuse` record, and an error. The row is kept — and
//!   `0012_loop.sql` makes it append-only — so the refusal is a fact about the data rather
//!   than about this build's code path.
//!
//! Two refusals are possible and they are different refusals: a protected subject, and a
//! non-reversible action. Both are recorded, and [`GcReport`] keeps them apart, because
//! "we may not" and "we cannot" call for different follow-ups.

use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::debt::{DebtFinding, DebtKind};
use crate::error::{LoopError, Result};
use crate::rdf;

/// The escalation ladder, cheapest first.
///
/// The order is the phase's, and it is a *policy*: exhausting a cheaper rung before
/// reaching for a more expensive one is what keeps collection from becoming a rewrite.
pub const GC_LADDER: [GcActionKind; 9] = [
    GcActionKind::TempReasoning,
    GcActionKind::Context,
    GcActionKind::Data,
    GcActionKind::Policy,
    GcActionKind::Prompt,
    GcActionKind::Skill,
    GcActionKind::Code,
    GcActionKind::Architecture,
    GcActionKind::Model,
];

/// One rung of the ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcActionKind {
    /// Spend less on a temporary reasoning call.
    TempReasoning,
    /// Drop context that is not needed.
    Context,
    /// Archive data nothing recalls.
    Data,
    /// Retire or merge a policy.
    Policy,
    /// Shorten or retarget a prompt.
    Prompt,
    /// Retire a technique the library no longer retrieves.
    Skill,
    /// Change code — a change set through the gate.
    Code,
    /// Change the architecture — a change set through the gate.
    Architecture,
    /// Change the model — a change set through the gate.
    Model,
}

impl GcActionKind {
    /// The stable wire name, which is the value the table's CHECK allows.
    pub fn as_str(self) -> &'static str {
        match self {
            GcActionKind::TempReasoning => "temp_reasoning",
            GcActionKind::Context => "context",
            GcActionKind::Data => "data",
            GcActionKind::Policy => "policy",
            GcActionKind::Prompt => "prompt",
            GcActionKind::Skill => "skill",
            GcActionKind::Code => "code",
            GcActionKind::Architecture => "architecture",
            GcActionKind::Model => "model",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<GcActionKind> {
        GC_LADDER
            .into_iter()
            .find(|kind| kind.as_str() == text.trim())
    }

    /// Its position in the ladder.
    pub fn rung(self) -> usize {
        GC_LADDER
            .iter()
            .position(|kind| *kind == self)
            .expect("every rung is in the ladder")
    }

    /// True when the rung is a change set rather than a local action.
    ///
    /// `Code` and above are proposals the promotion gate has to judge; below it, the
    /// action is a local, reversible tidying that the collector may apply itself.
    pub fn requires_change_set(self) -> bool {
        self.rung() >= GcActionKind::Code.rung()
    }

    /// True when the action can be undone inside the run.
    ///
    /// The two highest rungs are not reversible here — an architecture or model change
    /// that has already been applied is a new system, and calling the change "undone"
    /// would be a claim about a state the ledger no longer describes.
    pub fn reversible(self) -> bool {
        self.rung() < GcActionKind::Architecture.rung()
    }
}

impl std::fmt::Display for GcActionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The lowest rung that plausibly resolves a debt kind.
///
/// The mapping is data, not inference: a duplicate policy is a policy problem, an unused
/// graph is a data problem, and complexity with no single owner is an architecture
/// problem. The collector is allowed to escalate above this rung only through a change
/// set, which the gate judges.
pub fn rung_for(kind: DebtKind) -> GcActionKind {
    match kind {
        DebtKind::ExpensiveWorkflow => GcActionKind::Prompt,
        DebtKind::ObsoleteTechnique => GcActionKind::Skill,
        DebtKind::DuplicatePolicy | DebtKind::ConflictingPolicy => GcActionKind::Policy,
        DebtKind::StaleMemory | DebtKind::UnusedSchema => GcActionKind::Data,
        DebtKind::UnusedCapability => GcActionKind::Code,
        DebtKind::Complexity => GcActionKind::Architecture,
    }
}

/// What an action does to the developmental ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerImpact {
    /// Nothing: the ledger is not involved.
    None,
    /// Metadata only: a record about a record.
    Metadata,
    /// A row the ledger holds would move.
    Row,
}

impl LedgerImpact {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            LedgerImpact::None => "none",
            LedgerImpact::Metadata => "metadata",
            LedgerImpact::Row => "row",
        }
    }
}

impl std::fmt::Display for LedgerImpact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One proposed or applied action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GcAction {
    /// Its ULID.
    #[serde(with = "mm_core::serde_ulid")]
    pub id: Ulid,
    /// The finding it answers.
    #[serde(with = "mm_core::serde_ulid")]
    pub finding: Ulid,
    /// The rung.
    pub kind: GcActionKind,
    /// The resource it is about.
    pub subject: String,
    /// Whether it can be undone.
    pub reversible: bool,
    /// What it would do to the ledger.
    pub ledger_impact: LedgerImpact,
    /// True when the subject is part of the immutable developmental ledger.
    pub protects_ledger: bool,
    /// Whether it was applied.
    pub applied: bool,
}

impl GcAction {
    /// Whether this action may be applied at all.
    ///
    /// Both halves are checked here and again in SQL: the guard is cheap, and the
    /// database's trigger is the half that cannot be bypassed.
    pub fn may_apply(&self) -> Result<()> {
        if self.protects_ledger {
            return Err(LoopError::GcRefused {
                subject: self.subject.clone(),
                reason: "the subject is part of the immutable developmental ledger".to_string(),
            });
        }
        if !self.reversible {
            return Err(LoopError::GcRefused {
                subject: self.subject.clone(),
                reason: format!("{} is not reversible", self.kind),
            });
        }
        Ok(())
    }
}

/// What one collection pass did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GcReport {
    /// The actions that were applied.
    pub actions: Vec<GcAction>,
    /// The actions that were refused, with their reasons.
    pub refused: Vec<GcRefusal>,
}

/// A refusal, as an operator reads it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GcRefusal {
    /// The action that was refused. It is recorded, not discarded.
    pub action: GcAction,
    /// Why.
    pub reason: String,
}

impl GcReport {
    /// True when no action that named a protected subject was applied.
    ///
    /// The refusals are not required to be protected: a non-reversible action is refused
    /// too, and folding those together would make this predicate answer "was anything
    /// refused" instead of "did anything touch the ledger".
    pub fn protected_untouched(&self) -> bool {
        self.actions.iter().all(|action| !action.protects_ledger)
    }
}

/// What a collector does.
#[async_trait]
pub trait Collector {
    /// Propose an action for a finding, recording it.
    async fn collect(&self, finding: &DebtFinding) -> Result<GcAction>;
}

/// The collector that walks the ladder.
pub struct LadderCollector {
    store: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
    graph: Option<Arc<GraphStore>>,
}

impl LadderCollector {
    /// A collector over the kernel's store.
    pub fn new(store: SqliteStore, logger: Arc<Logger>, ids: Arc<UlidFactory>) -> Self {
        LadderCollector {
            store,
            logger,
            ids,
            graph: None,
        }
    }

    /// Attach the graph store, so refusals and actions reach `/self`.
    pub fn with_graph(mut self, graph: Arc<GraphStore>) -> Self {
        self.graph = Some(graph);
        self
    }

    /// The impact a rung has on the ledger.
    pub fn impact_for(kind: GcActionKind) -> LedgerImpact {
        match kind {
            GcActionKind::TempReasoning | GcActionKind::Context | GcActionKind::Prompt => {
                LedgerImpact::None
            }
            GcActionKind::Skill | GcActionKind::Policy => LedgerImpact::Metadata,
            GcActionKind::Data => LedgerImpact::Row,
            GcActionKind::Code => LedgerImpact::Metadata,
            GcActionKind::Architecture | GcActionKind::Model => LedgerImpact::Row,
        }
    }

    /// Record one action row.
    async fn record(&self, action: &GcAction) -> Result<()> {
        self.store
            .execute(
                "INSERT INTO gc_actions \
                 (id, finding_id, action, subject_uri, reversible, ledger_impact, \
                  protects_ledger, applied, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&action.id)),
                    Param::Text(mm_core::ulid_string(&action.finding)),
                    Param::Text(action.kind.as_str().to_string()),
                    Param::Text(action.subject.clone()),
                    Param::Int(i64::from(action.reversible)),
                    Param::Text(action.ledger_impact.as_str().to_string()),
                    Param::Int(i64::from(action.protects_ledger)),
                    Param::Int(i64::from(action.applied)),
                    Param::Text(Timestamp::now().to_rfc3339()),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the action: {e}")))?;
        if let Some(graph) = &self.graph {
            let turtle = rdf::turtle(&rdf::gc_quads(action));
            graph
                .handle()
                .insert_turtle(rdf::SELF_GRAPH, &turtle)
                .await
                .map_err(LoopError::from)?;
        }
        Ok(())
    }

    /// Apply an action, or refuse it.
    pub async fn apply(&self, action: &GcAction) -> Result<()> {
        action.may_apply()?;
        self.store
            .execute(
                "UPDATE gc_actions SET applied = 1 \
                 WHERE id = ? AND protects_ledger = 0 AND reversible = 1",
                vec![Param::Text(mm_core::ulid_string(&action.id))],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot apply the action: {e}")))?;
        self.logger
            .audit(
                Level::Warn,
                codes::GC_ACTION,
                crate::TARGET,
                Some(action.id),
                json!({
                    "finding_id": mm_core::ulid_string(&action.finding),
                    "action": action.kind.as_str(),
                    "subject_uri": action.subject,
                    "reversible": action.reversible,
                    "ledger_impact": action.ledger_impact.as_str(),
                    "protects_ledger": action.protects_ledger,
                    "applied": true,
                }),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot apply the action: {e}")))?;
        Ok(())
    }

    /// Collect for every finding, applying what may be applied.
    pub async fn run(&self, findings: &[DebtFinding]) -> Result<GcReport> {
        let mut actions = Vec::new();
        let mut refused = Vec::new();
        for finding in findings {
            match self.collect(finding).await {
                Ok(action) => {
                    self.apply(&action).await?;
                    actions.push(GcAction {
                        applied: true,
                        ..action
                    });
                }
                Err(error) if error.is_gc_refused() => {
                    // The row was already written by `collect`; the report carries the
                    // refusal so a caller can see both the fact and the reason.
                    let action = self.last_action_for(&finding.id).await?;
                    refused.push(GcRefusal {
                        action,
                        reason: error.to_string(),
                    });
                }
                Err(other) => return Err(other),
            }
        }
        Ok(GcReport { actions, refused })
    }

    /// The action row most recently recorded for a finding.
    async fn last_action_for(&self, finding: &Ulid) -> Result<GcAction> {
        let rows = self
            .store
            .query_json(
                "SELECT * FROM gc_actions WHERE finding_id = ? ORDER BY created_at DESC, id DESC",
                vec![Param::Text(mm_core::ulid_string(finding))],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the action: {e}")))?;
        let row = rows
            .first()
            .ok_or_else(|| LoopError::Store("the refusal recorded no action".to_string()))?;
        Ok(GcAction {
            id: mm_core::id::parse_ulid(row["id"].as_str().unwrap_or_default())
                .map_err(LoopError::from)?,
            finding: *finding,
            kind: GcActionKind::parse(row["action"].as_str().unwrap_or_default())
                .ok_or_else(|| LoopError::Store("unknown action kind in gc_actions".to_string()))?,
            subject: row["subject_uri"].as_str().unwrap_or_default().to_string(),
            reversible: row["reversible"].as_i64().unwrap_or(0) == 1,
            ledger_impact: match row["ledger_impact"].as_str().unwrap_or_default() {
                "none" => LedgerImpact::None,
                "metadata" => LedgerImpact::Metadata,
                _ => LedgerImpact::Row,
            },
            protects_ledger: row["protects_ledger"].as_i64().unwrap_or(0) == 1,
            applied: row["applied"].as_i64().unwrap_or(0) == 1,
        })
    }

    /// How many collected actions name a protected subject, and how many of those were
    /// applied. The second number must be zero.
    pub async fn protected_applied(&self) -> Result<i64> {
        let rows = self
            .store
            .query_json(
                "SELECT count(*) AS n FROM gc_actions WHERE protects_ledger = 1 AND applied = 1",
                Params::new(),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read gc_actions: {e}")))?;
        Ok(rows[0]["n"].as_i64().unwrap_or(-1))
    }
}

#[async_trait]
impl Collector for LadderCollector {
    /// Propose the cheapest rung for a finding, recording the action.
    ///
    /// The refusal path records the row *and* returns an error. Both are deliberate: the
    /// error is what stops the caller from applying it, and the row is what makes the
    /// refusal auditable — and append-only, since a protected row cannot be updated or
    /// deleted.
    async fn collect(&self, finding: &DebtFinding) -> Result<GcAction> {
        finding.validate()?;
        let kind = rung_for(finding.kind);
        let action = GcAction {
            id: self.ids.next(),
            finding: finding.id,
            kind,
            subject: finding.subject.clone(),
            reversible: kind.reversible(),
            ledger_impact: Self::impact_for(kind),
            protects_ledger: finding.protects_ledger,
            applied: false,
        };
        self.record(&action).await?;
        if action.protects_ledger {
            self.logger
                .audit(
                    Level::Warn,
                    codes::GC_REFUSE,
                    crate::TARGET,
                    Some(action.id),
                    json!({
                        "finding_id": mm_core::ulid_string(&finding.id),
                        "subject_uri": action.subject,
                        "reason": "protected-ledger",
                    }),
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot record the refusal: {e}")))?;
            return Err(LoopError::GcRefused {
                subject: action.subject.clone(),
                reason: "the subject is part of the immutable developmental ledger".to_string(),
            });
        }
        if !action.reversible {
            self.logger
                .audit(
                    Level::Warn,
                    codes::GC_REFUSE,
                    crate::TARGET,
                    Some(action.id),
                    json!({
                        "finding_id": mm_core::ulid_string(&finding.id),
                        "subject_uri": action.subject,
                        "reason": "non-reversible",
                    }),
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot record the refusal: {e}")))?;
            return Err(LoopError::GcRefused {
                subject: action.subject.clone(),
                reason: format!("{kind} is not reversible"),
            });
        }
        Ok(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debt::DebtFinding;
    use mm_core::Config;

    async fn collector() -> (LadderCollector, SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path().join("data"));
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        (LadderCollector::new(store.clone(), logger, ids), store, dir)
    }

    /// Persist a finding, because `gc_actions.finding_id` references `debt_findings(id)`:
    /// an action about a finding nothing recorded would be a row with no subject of its own.
    async fn persist(store: &SqliteStore, finding: &DebtFinding) {
        store
            .execute(
                "INSERT INTO debt_findings (id, run_id, kind, subject_uri, severity, \
                 evidence_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&finding.id)),
                    Param::Null,
                    Param::Text(finding.kind.as_str().to_string()),
                    Param::Text(finding.subject.clone()),
                    Param::Real(f64::from(finding.severity)),
                    Param::Text(serde_json::to_string(&finding.evidence).unwrap()),
                    Param::Text(Timestamp::now().to_rfc3339()),
                ],
            )
            .await
            .unwrap();
    }

    fn finding(seed: u128, kind: DebtKind, subject: &str, protected: bool) -> DebtFinding {
        DebtFinding {
            id: Ulid::from_parts(1_700_000_000_000, seed),
            run: None,
            kind,
            subject: subject.to_string(),
            severity: 0.6,
            evidence: vec!["one record".to_string()],
            protects_ledger: protected,
            detail: "d".to_string(),
        }
    }

    #[test]
    fn the_ladder_is_ordered_cheapest_first() {
        for pair in GC_LADDER.windows(2) {
            assert!(pair[0].rung() < pair[1].rung(), "{} {}", pair[0], pair[1]);
        }
        assert_eq!(GC_LADDER[0], GcActionKind::TempReasoning);
        assert_eq!(GC_LADDER[8], GcActionKind::Model);
        assert!(GcActionKind::Code.requires_change_set());
        assert!(!GcActionKind::Skill.requires_change_set());
        assert!(!GcActionKind::Architecture.reversible());
    }

    #[test]
    fn every_debt_kind_maps_to_a_rung_that_is_not_above_it_without_a_change_set() {
        for kind in DebtKind::ALL {
            let rung = rung_for(kind);
            assert!(GC_LADDER.contains(&rung), "{kind} -> {rung}");
        }
        assert_eq!(rung_for(DebtKind::DuplicatePolicy), GcActionKind::Policy);
        assert_eq!(rung_for(DebtKind::StaleMemory), GcActionKind::Data);
        assert_eq!(rung_for(DebtKind::UnusedCapability), GcActionKind::Code);
        assert!(!rung_for(DebtKind::DuplicatePolicy).requires_change_set());
        assert!(rung_for(DebtKind::UnusedCapability).requires_change_set());
    }

    #[test]
    fn impact_is_a_function_of_the_rung() {
        assert_eq!(
            LadderCollector::impact_for(GcActionKind::Prompt),
            LedgerImpact::None
        );
        assert_eq!(
            LadderCollector::impact_for(GcActionKind::Policy),
            LedgerImpact::Metadata
        );
        assert_eq!(
            LadderCollector::impact_for(GcActionKind::Data),
            LedgerImpact::Row
        );
    }

    #[tokio::test]
    async fn a_protected_subject_is_recorded_and_refused_never_applied() {
        let (collector, store, _dir) = collector().await;
        // A real ledger row, so the refusal is about data and not about a flag.
        let ids = UlidFactory::new();
        let event = ids.next();
        store
            .execute(
                "INSERT INTO events (seq, id, kind, payload, status, system_from, created_at, hash) \
                 VALUES (1, ?, 'custom', '{}', 'committed', '2026-10-08T00:00:00Z', \
                 '2026-10-08T00:00:00Z', 'h')",
                vec![Param::Text(mm_core::ulid_string(&event))],
            )
            .await
            .unwrap();
        let protected = finding(
            7,
            DebtKind::StaleMemory,
            &format!("{}{}", rdf::data_iri_prefix(), mm_core::ulid_string(&event)),
            true,
        );
        persist(&store, &protected).await;
        let error = collector
            .collect(&protected)
            .await
            .expect_err("a protected subject must be refused");
        assert!(error.is_gc_refused(), "{error}");
        // The row exists, asserts protection, and is append-only.
        let rows = store
            .query_json(
                "SELECT protects_ledger, applied, reversible FROM gc_actions",
                Params::new(),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["protects_ledger"].as_i64(), Some(1));
        assert_eq!(rows[0]["applied"].as_i64(), Some(0));
        let rewritten = store
            .execute(
                "UPDATE gc_actions SET applied = 1 WHERE protects_ledger = 1",
                Params::new(),
            )
            .await;
        assert!(
            rewritten.is_err(),
            "a protected action row must be append-only in SQL"
        );
    }

    #[tokio::test]
    async fn an_ordinary_finding_is_collected_and_applied() {
        let (collector, store, _dir) = collector().await;
        let finding = finding(
            8,
            DebtKind::DuplicatePolicy,
            "https://metamind.dev/ontology#p",
            false,
        );
        persist(&store, &finding).await;
        let action = collector.collect(&finding).await.expect("an action");
        assert_eq!(action.kind, GcActionKind::Policy);
        assert!(action.reversible);
        collector.apply(&action).await.expect("apply");
        assert_eq!(collector.protected_applied().await.unwrap(), 0);
        let report = collector.run(&[finding]).await.expect("a report");
        assert_eq!(report.actions.len(), 1);
        assert!(report.protected_untouched());
    }

    #[tokio::test]
    async fn the_run_reports_a_refusal_rather_than_stopping() {
        let (collector, store, _dir) = collector().await;
        let protected = finding(
            9,
            DebtKind::StaleMemory,
            "https://metamind.dev/data/x",
            true,
        );
        let ordinary = finding(
            10,
            DebtKind::Complexity,
            "https://metamind.dev/ontology#C",
            false,
        );
        persist(&store, &protected).await;
        persist(&store, &ordinary).await;
        let report = collector
            .run(&[protected, ordinary])
            .await
            .expect("a report");
        // Complexity maps to Architecture, which is not reversible: refused, not applied.
        assert_eq!(report.actions.len(), 0);
        assert_eq!(report.refused.len(), 2);
        assert!(report.protected_untouched());
    }
}
