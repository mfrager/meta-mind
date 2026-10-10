//! Architectural debt: what the system has accumulated that costs more than it returns.
//!
//! Design §85 names the eight kinds of debt, and the phase's rule for all of them is the
//! same: **a finding carries the evidence that showed it**. A finding with no evidence is
//! an opinion, so [`DebtFinding`] refuses to exist without one, the `debt_findings` table
//! has no default for its evidence column, and `ontology/shapes/self_model.ttl` requires
//! at least one `mm:debtEvidence`.
//!
//! The scan has two halves, and both are needed:
//!
//! * **A live half**, over state that exists right now: two policies with one name, a
//!   memory that is stale and unprotected, a named graph with no quads in it. These are
//!   facts about the current store, and a scan that could not see them would only be able
//!   to report what someone wrote down.
//! * **A seeded half**, over `bench/debt/`. Some debt is not derivable from state — an
//!   obsolete technique, a workflow that is merely expensive, complexity with no single
//!   owner — and the phase's own gate needs a corpus whose findings are known in advance,
//!   including the one that names a protected ledger subject so the collector's refusal is
//!   exercised.
//!
//! The scan is deterministic: the corpus is read in sorted order, the live queries are
//! `ORDER BY`-ed, and the severity arithmetic has no randomness.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};

use crate::error::{LoopError, Result};
use crate::rdf;

/// One of the eight kinds of debt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebtKind {
    /// A capability nothing uses.
    UnusedCapability,
    /// Two policies with the same name.
    DuplicatePolicy,
    /// Policies that constrain the same thing in incompatible ways.
    ConflictingPolicy,
    /// Memory that is no longer recalled and no longer true.
    StaleMemory,
    /// A schema — here, a named graph — that nothing writes.
    UnusedSchema,
    /// A workflow whose cost is not paid back.
    ExpensiveWorkflow,
    /// A technique superseded by a better one.
    ObsoleteTechnique,
    /// Complexity with no single justification.
    Complexity,
}

impl DebtKind {
    /// Every kind.
    pub const ALL: [DebtKind; 8] = [
        DebtKind::UnusedCapability,
        DebtKind::DuplicatePolicy,
        DebtKind::ConflictingPolicy,
        DebtKind::StaleMemory,
        DebtKind::UnusedSchema,
        DebtKind::ExpensiveWorkflow,
        DebtKind::ObsoleteTechnique,
        DebtKind::Complexity,
    ];

    /// The stable wire name, which is the value the table's CHECK allows.
    pub fn as_str(self) -> &'static str {
        match self {
            DebtKind::UnusedCapability => "unused_capability",
            DebtKind::DuplicatePolicy => "duplicate_policy",
            DebtKind::ConflictingPolicy => "conflicting_policy",
            DebtKind::StaleMemory => "stale_memory",
            DebtKind::UnusedSchema => "unused_schema",
            DebtKind::ExpensiveWorkflow => "expensive_workflow",
            DebtKind::ObsoleteTechnique => "obsolete_technique",
            DebtKind::Complexity => "complexity",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<DebtKind> {
        DebtKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text.trim())
    }
}

impl std::fmt::Display for DebtKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One finding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebtFinding {
    /// Its ULID.
    #[serde(with = "mm_core::serde_ulid")]
    pub id: Ulid,
    /// The run that found it, when a run did.
    #[serde(with = "mm_core::serde_ulid::option")]
    pub run: Option<Ulid>,
    /// Which kind it is.
    pub kind: DebtKind,
    /// The resource it is about.
    pub subject: String,
    /// How much it costs, in `[0,1]`.
    pub severity: f32,
    /// The records that showed it. At least one.
    pub evidence: Vec<String>,
    /// True when the subject is part of the immutable developmental ledger.
    pub protects_ledger: bool,
    /// One sentence for an operator.
    pub detail: String,
}

impl DebtFinding {
    /// Refuse a finding that stands on nothing or names nothing.
    pub fn validate(&self) -> Result<()> {
        if self.subject.trim().is_empty() {
            return Err(LoopError::validation("subject", "must not be empty"));
        }
        if self.evidence.is_empty() {
            return Err(LoopError::validation(
                "evidence",
                "a finding with no evidence is an opinion",
            ));
        }
        if !self.severity.is_finite() || !(0.0..=1.0).contains(&self.severity) {
            return Err(LoopError::validation(
                "severity",
                format!("{} is not in [0,1]", self.severity),
            ));
        }
        Ok(())
    }
}

/// What one scan did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebtScanReport {
    /// Every finding, most severe first.
    pub findings: Vec<DebtFinding>,
    /// How many rows were written.
    pub persisted: usize,
    /// How many triples reached `/self`.
    pub graph_triples: usize,
}

impl DebtScanReport {
    /// The findings whose subject is protected, which the collector must refuse.
    pub fn protected(&self) -> Vec<&DebtFinding> {
        self.findings
            .iter()
            .filter(|finding| finding.protects_ledger)
            .collect()
    }
}

/// What a scanner does.
#[async_trait]
pub trait DebtScanner {
    /// Find the debt.
    async fn scan(&self) -> Result<Vec<DebtFinding>>;
}

/// The live scans plus the seeded corpus.
pub struct CompositeDebtScanner {
    store: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
    root: PathBuf,
    corpus_dir: PathBuf,
    graph: Option<Arc<GraphStore>>,
    run: Option<Ulid>,
}

impl CompositeDebtScanner {
    /// A scanner over a repository root and a data corpus.
    pub fn new(
        store: SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
        root: PathBuf,
    ) -> Self {
        let corpus_dir = root.join("bench").join("debt");
        CompositeDebtScanner {
            store,
            logger,
            ids,
            root,
            corpus_dir,
            graph: None,
            run: None,
        }
    }

    /// Attach the graph store, so the empty-graph scan and the `/self` mirror both work.
    pub fn with_graph(mut self, graph: Arc<GraphStore>) -> Self {
        self.graph = Some(graph);
        self
    }

    /// Attribute the findings to a run.
    pub fn with_run(mut self, run: Ulid) -> Self {
        self.run = Some(run);
        self
    }

    /// The repository root the corpus is read from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// True when `subject` is a record the developmental ledger protects.
    ///
    /// Three sources, each a table whose rows are append-only: an event, a protected
    /// memory, and a staked prediction. The check is deliberately positive — a subject is
    /// protected because a *row says so*, not because its IRI looks like a ULID, since a
    /// heuristic would protect the wrong things and a missing row would protect nothing.
    pub async fn ledger_protects(&self, subject: &str) -> bool {
        let Some(ulid) = subject.strip_prefix(rdf::data_iri_prefix()) else {
            return false;
        };
        if mm_core::id::parse_ulid(ulid).is_err() {
            return false;
        }
        let probes: [(&str, &str); 3] = [
            ("SELECT count(*) AS n FROM events WHERE id = ?", ulid),
            (
                "SELECT count(*) AS n FROM memories WHERE id = ? AND protected = 1",
                ulid,
            ),
            (
                "SELECT count(*) AS n FROM prediction_ledger WHERE id = ?",
                ulid,
            ),
        ];
        for (sql, value) in probes {
            let Ok(rows) = self
                .store
                .query_json(sql, vec![Param::Text(value.to_string())])
                .await
            else {
                continue;
            };
            if rows.first().and_then(|row| row["n"].as_i64()).unwrap_or(0) > 0 {
                return true;
            }
        }
        false
    }

    /// The seeded corpus, one file at a time, in sorted order.
    async fn seeded(&self) -> Result<Vec<DebtFinding>> {
        if !self.corpus_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.corpus_dir)
            .map_err(|e| {
                LoopError::Store(format!("cannot read {}: {e}", self.corpus_dir.display()))
            })?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();
        let mut findings = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .map_err(|e| LoopError::Store(format!("cannot read {}: {e}", file.display())))?;
            let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                LoopError::validation(
                    "corpus",
                    format!("{} is not a debt corpus: {e}", file.display()),
                )
            })?;
            let items = match value.get("findings").and_then(serde_json::Value::as_array) {
                Some(items) => items.clone(),
                None => vec![value.clone()],
            };
            for item in items {
                let kind_text = item["kind"].as_str().unwrap_or_default();
                let kind = DebtKind::parse(kind_text).ok_or_else(|| {
                    LoopError::validation(
                        "corpus",
                        format!("{}: unknown debt kind {kind_text:?}", file.display()),
                    )
                })?;
                let evidence: Vec<String> = item["evidence"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut finding = DebtFinding {
                    id: self.ids.next(),
                    run: self.run,
                    kind,
                    subject: item["subject"].as_str().unwrap_or_default().to_string(),
                    severity: item["severity"].as_f64().unwrap_or(0.0) as f32,
                    evidence,
                    protects_ledger: item["protects_ledger"].as_bool().unwrap_or(false),
                    detail: item["detail"].as_str().unwrap_or_default().to_string(),
                };
                finding.validate()?;
                // The corpus may assert protection, and the store may confirm it; either
                // is enough. The store cannot *revoke* it, because a corpus that named a
                // ledger record was written for exactly that case.
                if !finding.protects_ledger {
                    finding.protects_ledger = self.ledger_protects(&finding.subject).await;
                }
                findings.push(finding);
            }
        }
        Ok(findings)
    }

    /// Policies that share a name.
    async fn duplicate_policies(&self) -> Result<Vec<DebtFinding>> {
        let rows = self
            .store
            .query_json(
                "SELECT name, count(*) AS n FROM policies GROUP BY name HAVING n > 1 ORDER BY name",
                Params::new(),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read policies: {e}")))?;
        let mut findings = Vec::new();
        for row in rows {
            let name = row["name"].as_str().unwrap_or_default().to_string();
            let n = row["n"].as_i64().unwrap_or(0);
            findings.push(DebtFinding {
                id: self.ids.next(),
                run: self.run,
                kind: DebtKind::DuplicatePolicy,
                subject: format!("{MM}policy/{name}"),
                severity: 0.5_f32.min(0.1 * n as f32 + 0.4),
                evidence: vec![format!("policies: {n} rows share the name {name}")],
                protects_ledger: false,
                detail: format!(
                    "{n} policies are called {name}; a caller that looks one up by name is \
                     not asking a determined question"
                ),
            });
        }
        Ok(findings)
    }

    /// Active, unprotected memories nothing has found important.
    async fn stale_memory(&self, limit: usize) -> Result<Vec<DebtFinding>> {
        let rows = self
            .store
            .query_json(
                "SELECT id, importance FROM memories \
                 WHERE protected = 0 AND status = 'active' AND importance < 0.2 \
                 ORDER BY id LIMIT ?",
                vec![Param::Int(limit as i64)],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read memories: {e}")))?;
        let mut findings = Vec::new();
        for row in rows {
            let id = row["id"].as_str().unwrap_or_default().to_string();
            let importance = row["importance"].as_f64().unwrap_or(0.0);
            let subject = format!("{}{id}", rdf::data_iri_prefix());
            findings.push(DebtFinding {
                id: self.ids.next(),
                run: self.run,
                kind: DebtKind::StaleMemory,
                subject: subject.clone(),
                severity: (1.0 - importance) as f32,
                evidence: vec![format!(
                    "memories: {id} is active, unprotected and scored {importance:.3}"
                )],
                protects_ledger: self.ledger_protects(&subject).await,
                detail: "a memory nothing recalls and nothing values is a row that only costs"
                    .to_string(),
            });
        }
        Ok(findings)
    }

    /// Named graphs with no quads in them.
    async fn empty_graphs(&self) -> Result<Vec<DebtFinding>> {
        let Some(graph) = &self.graph else {
            return Ok(Vec::new());
        };
        let mut names = graph.named_graphs().await.map_err(LoopError::from)?;
        names.sort();
        let mut findings = Vec::new();
        for name in names {
            let bare = name
                .strip_prefix("https://metamind.dev/graph/")
                .unwrap_or(&name)
                .to_string();
            let quads = graph
                .graph_quad_count(&bare)
                .await
                .map_err(LoopError::from)?;
            if quads > 0 {
                continue;
            }
            findings.push(DebtFinding {
                id: self.ids.next(),
                run: self.run,
                kind: DebtKind::UnusedSchema,
                subject: name.clone(),
                severity: 0.2,
                evidence: vec![format!("named graph {name} holds 0 quads")],
                protects_ledger: false,
                detail: "a named graph nothing writes is a schema with no user".to_string(),
            });
        }
        Ok(findings)
    }

    /// Scan, persist and mirror.
    pub async fn scan_and_persist(&self) -> Result<DebtScanReport> {
        let findings = self.scan().await?;
        let mut persisted = 0;
        let mut graph_triples = 0;
        self.logger
            .emit(
                mm_log::LogRecord::new(Level::Info, codes::DEBT_SCAN, crate::TARGET)
                    .with_field("findings", findings.len())
                    .with_field("corpus_dir", self.corpus_dir.display().to_string()),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the scan: {e}")))?;
        for finding in &findings {
            self.store
                .execute(
                    "INSERT INTO debt_findings \
                     (id, run_id, kind, subject_uri, severity, evidence_json, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    vec![
                        Param::Text(mm_core::ulid_string(&finding.id)),
                        finding
                            .run
                            .map(|run| Param::Text(mm_core::ulid_string(&run)))
                            .unwrap_or(Param::Null),
                        Param::Text(finding.kind.as_str().to_string()),
                        Param::Text(finding.subject.clone()),
                        Param::Real(f64::from(finding.severity)),
                        Param::Text(
                            serde_json::to_string(&finding.evidence)
                                .unwrap_or_else(|_| "[]".into()),
                        ),
                        Param::Text(Timestamp::now().to_rfc3339()),
                    ],
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot record the finding: {e}")))?;
            persisted += 1;
            self.logger
                .emit(
                    mm_log::LogRecord::new(Level::Info, codes::DEBT_FINDING, crate::TARGET)
                        .with_trace(finding.id)
                        .with_field("kind", finding.kind.as_str())
                        .with_field("subject_uri", finding.subject.clone())
                        .with_field("severity", finding.severity)
                        .with_field("evidence", finding.evidence.clone()),
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot record the finding: {e}")))?;
            if let Some(graph) = &self.graph {
                let turtle = rdf::turtle(&rdf::debt_quads(finding));
                graph_triples += graph
                    .handle()
                    .insert_turtle(rdf::SELF_GRAPH, &turtle)
                    .await
                    .map_err(LoopError::from)?;
            }
        }
        Ok(DebtScanReport {
            findings,
            persisted,
            graph_triples,
        })
    }
}

/// The ontology namespace, for subjects that are not instances.
const MM: &str = "https://metamind.dev/ontology#";

#[async_trait]
impl DebtScanner for CompositeDebtScanner {
    /// Every finding, most severe first.
    ///
    /// The sort is total — severity descending, then subject, then kind — so two scans of
    /// the same state produce the same order and the collector's ladder is applied in the
    /// same sequence both times.
    async fn scan(&self) -> Result<Vec<DebtFinding>> {
        let mut findings = Vec::new();
        findings.extend(self.seeded().await?);
        findings.extend(self.duplicate_policies().await?);
        findings.extend(self.stale_memory(20).await?);
        findings.extend(self.empty_graphs().await?);
        for finding in &findings {
            finding.validate()?;
        }
        findings.sort_by(|a, b| {
            b.severity
                .partial_cmp(&a.severity)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.subject.cmp(&b.subject))
                .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
        });
        Ok(findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;

    fn repo_root() -> PathBuf {
        mm_core::Config::repo_root()
    }

    async fn scanner() -> (CompositeDebtScanner, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path().join("data"));
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        (
            CompositeDebtScanner::new(store, logger, ids, repo_root()),
            dir,
        )
    }

    #[tokio::test]
    async fn the_seeded_corpus_is_found_with_its_evidence_and_one_protected_subject() {
        let (scanner, _dir) = scanner().await;
        let findings = scanner.scan().await.unwrap();
        // The corpus's own `expected.kinds` is the checklist: a scan that found five of
        // the six kinds is a scanner that silently dropped one, so the assertion names
        // the kind rather than counting rows.
        let corpus: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo_root().join("bench/debt/seeded_debt_01.json")).unwrap(),
        )
        .unwrap();
        let expected: Vec<&str> = corpus["expected"]["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kind| kind.as_str().unwrap())
            .collect();
        assert_eq!(expected.len(), 6, "the corpus seeds six findings");
        for kind_text in &expected {
            let kind = DebtKind::parse(kind_text).expect("a known kind");
            assert!(
                findings.iter().any(|finding| finding.kind == kind),
                "the seeded {kind_text} is found: {findings:?}"
            );
        }
        for finding in &findings {
            assert!(
                !finding.evidence.is_empty(),
                "a finding with no evidence is an opinion: {finding:?}"
            );
            assert!((0.0..=1.0).contains(&finding.severity));
        }
        let protected = findings.iter().filter(|f| f.protects_ledger).count();
        assert!(protected >= 1, "the corpus seeds a protected subject");
        // Sorted most severe first.
        for pair in findings.windows(2) {
            assert!(
                pair[0].severity >= pair[1].severity,
                "the scan must be ordered"
            );
        }
    }

    #[tokio::test]
    async fn a_duplicate_policy_and_a_stale_memory_are_found_live() {
        let (scanner, _dir) = scanner().await;
        let store = scanner.store.clone();
        let ids = UlidFactory::new();
        // Two IRIs, one name: `policies.iri` is unique, and the debt is the shared
        // *name*, which is what a caller addresses the policy by.
        for iri in ["prefer_simpler_solution", "prefer_simpler_solution_v2"] {
            store
                .execute(
                    "INSERT INTO policies (id, iri, name, head_version, created_ulid) \
                     VALUES (?, ?, ?, 1, ?)",
                    vec![
                        Param::Text(mm_core::ulid_string(&ids.next())),
                        Param::Text(format!("https://metamind.dev/ontology#policy/{iri}")),
                        Param::Text("prefer_simpler_solution".to_string()),
                        Param::Text(mm_core::ulid_string(&ids.next())),
                    ],
                )
                .await
                .unwrap();
        }
        store
            .execute(
                "INSERT INTO memories (id, kind, tier, content, confidence, importance, \
                 valid_from, recorded_at, recorded_ulid, provenance, protected, status, \
                 content_hash) VALUES (?, 'semantic', 'recall', 'a fact', 0.5, 0.05, 0, 0, \
                 ?, ?, 0, 'active', 'h')",
                vec![
                    Param::Text(mm_core::ulid_string(&ids.next())),
                    Param::Text(mm_core::ulid_string(&ids.next())),
                    Param::Text(mm_core::ulid_string(&ids.next())),
                ],
            )
            .await
            .unwrap();
        let findings = scanner.scan().await.unwrap();
        assert!(
            findings.iter().any(|f| f.kind == DebtKind::DuplicatePolicy),
            "two policies named alpha is a duplicate"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.kind == DebtKind::StaleMemory && !f.protects_ledger),
            "an unprotected stale memory is found and is not protected"
        );
    }

    #[tokio::test]
    async fn a_protected_ledger_subject_is_recognised_by_its_row_not_its_shape() {
        let (scanner, _dir) = scanner().await;
        let ids = UlidFactory::new();
        let event = ids.next();
        scanner
            .store
            .execute(
                "INSERT INTO events (seq, id, kind, payload, status, system_from, created_at, hash) \
                 VALUES (1, ?, 'custom', '{}', 'committed', '2026-10-08T00:00:00Z', \
                 '2026-10-08T00:00:00Z', 'h')",
                vec![Param::Text(mm_core::ulid_string(&event))],
            )
            .await
            .unwrap();
        let subject = format!("{}{}", rdf::data_iri_prefix(), mm_core::ulid_string(&event));
        assert!(scanner.ledger_protects(&subject).await);
        // A ULID-shaped subject no table names is not protected.
        let unknown = format!(
            "{}{}",
            rdf::data_iri_prefix(),
            mm_core::ulid_string(&ids.next())
        );
        assert!(!scanner.ledger_protects(&unknown).await);
        // A module IRI is never protected by this rule.
        assert!(
            !scanner
                .ledger_protects("https://metamind.dev/code/module/cognition/calibration")
                .await
        );
    }

    #[test]
    fn every_kind_round_trips_through_its_wire_name() {
        for kind in DebtKind::ALL {
            assert_eq!(DebtKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DebtKind::parse("not_a_kind"), None);
    }

    #[test]
    fn a_finding_stands_on_evidence_and_a_bounded_severity() {
        let mut finding = DebtFinding {
            id: Ulid::from_parts(1, 1),
            run: None,
            kind: DebtKind::Complexity,
            subject: "https://metamind.dev/ontology#X".to_string(),
            severity: 0.5,
            evidence: vec!["one record".to_string()],
            protects_ledger: false,
            detail: "d".to_string(),
        };
        assert!(finding.validate().is_ok());
        finding.evidence.clear();
        assert!(finding.validate().is_err());
        finding.evidence = vec!["e".to_string()];
        finding.severity = 1.5;
        assert!(finding.validate().is_err());
    }
}
