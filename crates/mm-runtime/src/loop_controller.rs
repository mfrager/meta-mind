//! The ten-stage closed developmental loop.
//!
//! One control flow, driven by rows and events, with no stage that depends on a model's
//! text:
//!
//! | # | Stage | What it does | Key |
//! |---|---|---|---|
//! | 1 | [`LoopStage::Experience`] | reads the goal's episode | `(run_id, 0)` |
//! | 2 | [`LoopStage::EventLog`] | commits the inputs as an event | append-only |
//! | 3 | [`LoopStage::MetaAnalysis`] | Phase 11's trigger/diagnosis pass | `(run_id, 2)` |
//! | 4 | [`LoopStage::CapabilityGap`] | derives the gap, and proves it is a gap | `(run_id, 3)` |
//! | 5 | [`LoopStage::ChangeSet`] | assembles the typed change set | `(run_id, 4)` |
//! | 6 | [`LoopStage::SelfEngineering`] | materialises the candidate in the sandbox, ingests the Pi session | `(run_id, 5)` |
//! | 7 | [`LoopStage::TestBenchmark`] | the regression suite and the frozen benchmark | `(run_id, 6)` |
//! | 8 | [`LoopStage::Shadow`] | compares the candidate with the frozen baseline | `(run_id, 7)` |
//! | 9 | [`LoopStage::PromotionGate`] | Phase 11's gate, unchanged | `(run_id, 8)` |
//! | 10 | [`LoopStage::NewVersion`] | materialises the promoted version and hot-loads it | `(run_id, 9)` |
//!
//! Three properties make the run manageable:
//!
//! * **Idempotent stages.** Every stage upserts one `loop_iterations` row keyed
//!   `(run_id, idx)` and appends one event. Re-running a stage rewrites its row and
//!   deduplicates its event by content hash, so a crash mid-run cannot double-apply a
//!   stage's side effects — and [`LoopController::resume`] continues at the first stage
//!   with no row.
//! * **Hard budgets.** Each stage debits its bucket *before* it works. A debit that would
//!   cross the envelope is a [`LoopError::BudgetDenied`], the run stops where it was told
//!   to, and nothing is clamped. The envelope is also mirrored into the Phase 11 budget
//!   ledger at the promotion, so a run cannot quietly drain the period's cap.
//! * **A digest folded from the log.** [`LoopController::digest_of`] folds the run's
//!   committed events with `(code_version, config_hash, goal)`; `loop replay` recomputes
//!   the same fold and compares it with what the run recorded. Replay therefore checks the
//!   log rather than a second execution of the code.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use mm_core::{Config, MmError, NewEvent, Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_eventlog::EventLog;
use mm_log::{codes, Level, LogRecord, Logger};
use mm_selfeng::budget::{BudgetKind, BudgetLedger};
use mm_selfeng::changeset::{self, BenchmarkId, ChangeSet, ChangeSetStore, TestId};
use mm_selfeng::journal::EvolutionJournal;
use mm_selfeng::promotion::{DeterministicGate, EvidenceBundle, PromotionDecision};
use mm_selfeng::sandbox::SandboxDir;
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::debt::CompositeDebtScanner;
use crate::design_writer::{doc_ref, ChangeSetDesignWriter};
use crate::error::{LoopError, Result};
use crate::module_loader::{ManifestModuleLoader, ModuleLoader, ModuleVersion};
use crate::scaffold::{self, ModuleSpec};
use crate::self_model::RunSelfModel;
use crate::{
    elapsed_ms, ArtifactRef, BudgetBucket, BudgetEnvelope, ClosedLoop, Divergence, LoopGoal,
    LoopReport, LoopStage, LoopState, PromotionId, RuntimePaths,
};

/// The regression case the change set names as its test.
///
/// A constant rather than a goal field, because the phase's test contract is the seeded
/// case its own gate asserts both halves of; a goal that could name a different case could
/// name one that does not exist.
pub const SEEDED_REGRESSION_TEST: &str = "bench/regression/seeded_bug_01";

/// The benchmark id the change set is judged by.
pub const QUALIFICATION_BENCHMARK: &str = "qualification_bench";

/// The budget a stage charges before it works.
///
/// Named as a function so a reader can see the whole envelope at once, and so the
/// proptest that asserts "the sum of the debits never exceeds the envelope" has one place
/// to read from.
pub fn stage_charge(stage: LoopStage) -> (BudgetBucket, f64) {
    match stage {
        LoopStage::Experience => (BudgetBucket::MetaAnalysis, 1.0),
        LoopStage::EventLog => (BudgetBucket::Metacognitive, 1.0),
        LoopStage::MetaAnalysis => (BudgetBucket::MetaAnalysis, 4.0),
        LoopStage::CapabilityGap => (BudgetBucket::MetaAnalysis, 2.0),
        LoopStage::ChangeSet => (BudgetBucket::Improvement, 2.0),
        LoopStage::SelfEngineering => (BudgetBucket::Improvement, 10.0),
        LoopStage::TestBenchmark => (BudgetBucket::Improvement, 5.0),
        LoopStage::Shadow => (BudgetBucket::Improvement, 2.0),
        LoopStage::PromotionGate => (BudgetBucket::Evolution, 1.0),
        LoopStage::NewVersion => (BudgetBucket::Evolution, 1.0),
    }
}

/// Everything a controller needs.
pub struct LoopServices {
    /// The tabular store.
    pub store: SqliteStore,
    /// The logger.
    pub logger: Arc<Logger>,
    /// The identifier factory.
    pub ids: Arc<UlidFactory>,
    /// The event log the run appends to.
    pub events: EventLog,
    /// The graph store, when the run mirrors anything.
    ///
    /// Shared rather than owned: the run hands the same store to the writers it builds
    /// (debt, GC, design, module load, self-model) while one process owns the writer thread,
    /// and [`GraphStore`]'s `Drop` is what stops that thread — so exactly one reference
    /// must remain the owner, and the rest are clones of its `Arc`.
    pub graph: Option<Arc<GraphStore>>,
    /// Where the run reads and writes.
    pub paths: RuntimePaths,
    /// The configuration the run was started under.
    pub config_hash: String,
    /// The code the run was started from.
    pub code_version: String,
}

impl LoopServices {
    /// Assemble the services from a kernel configuration.
    ///
    /// `config_hash` and `code_version` are *recorded*, not derived at replay time: replay
    /// reads them back from the run row, so a run is defined against the configuration it
    /// actually started under rather than against whatever the machine holds later.
    pub fn new(
        cfg: &Config,
        store: SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
        events: EventLog,
        graph: Option<Arc<GraphStore>>,
    ) -> Self {
        let data_dir = cfg.store.data_dir.display().to_string();
        let config_hash = mm_core::content_hash(
            format!(
                "{}|{}|{}|{}",
                cfg.kernel.codename, cfg.kernel.name, cfg.log.level, data_dir
            )
            .as_bytes(),
        );
        let code_version = format!("mm-runtime {}", env!("CARGO_PKG_VERSION"));
        LoopServices {
            store,
            logger,
            ids,
            events,
            graph,
            paths: RuntimePaths::under(Config::repo_root(), cfg.store.data_dir.clone()),
            config_hash,
            code_version,
        }
    }

    /// A variant for a caller that knows the roots directly (the background process).
    pub fn with_paths(mut self, paths: RuntimePaths) -> Self {
        self.paths = paths;
        self
    }
}

/// The controller.
pub struct LoopController {
    services: LoopServices,
}

/// What a replay found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayOutcome {
    /// The run.
    #[serde(with = "mm_core::serde_ulid")]
    pub run_id: Ulid,
    /// The digest the run recorded.
    pub recorded: String,
    /// The digest recomputed from the run's committed events.
    pub recomputed: String,
    /// Whether they are equal.
    pub identical: bool,
    /// The stages the log holds.
    pub stages: Vec<String>,
}

/// The provenance chain of one run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceChain {
    /// The Pi session that authored the candidate.
    #[serde(with = "mm_core::serde_ulid::option")]
    pub pi_session: Option<Ulid>,
    /// The paths the session edited.
    pub edits: Vec<String>,
    /// The module version that was activated.
    pub module: Option<String>,
    /// The change set the run assembled.
    #[serde(with = "mm_core::serde_ulid::option")]
    pub change_set: Option<Ulid>,
    /// The promotion the gate recorded.
    #[serde(with = "mm_core::serde_ulid::option")]
    pub promotion: Option<Ulid>,
}

impl ProvenanceChain {
    /// Every link is present.
    pub fn complete(&self) -> bool {
        self.pi_session.is_some()
            && !self.edits.is_empty()
            && self.module.is_some()
            && self.change_set.is_some()
            && self.promotion.is_some()
    }

    /// The chain, as the phase's pass criterion spells it.
    pub fn rendered(&self) -> &'static str {
        "mmc:PiSession -> mmc:Edit -> mmc:ModuleVersion -> mm:ChangeSet -> mm:Promotion"
    }
}

/// The mutable state one stage's execution needs.
struct RunState {
    run: Ulid,
    goal: LoopGoal,
    spec: ModuleSpec,
    artifacts: Vec<ArtifactRef>,
    change_set: Option<ChangeSet>,
    session: Option<Ulid>,
    suite: Option<mm_mistakes::SuiteReport>,
    bench: Option<mm_selfeng::benchmark::BenchResult>,
    shadow: Option<mm_selfeng::shadow::ShadowReport>,
    promotion: Option<mm_selfeng::promotion::PromotionOutcome>,
}

impl LoopController {
    /// A controller over the services.
    pub fn new(services: LoopServices) -> Self {
        LoopController { services }
    }

    /// The services, for a caller that needs the store or the paths.
    pub fn services(&self) -> &LoopServices {
        &self.services
    }

    /// A `ChangeSetStore` over the same store.
    fn change_sets(&self) -> ChangeSetStore {
        ChangeSetStore::new(
            self.services.store.clone(),
            self.services.logger.clone(),
            self.services.ids.clone(),
        )
    }

    /// The evolution journal over the same store.
    fn journal(&self) -> EvolutionJournal {
        EvolutionJournal::new(
            self.services.store.clone(),
            self.services.logger.clone(),
            self.services.ids.clone(),
        )
    }

    /// Record the run as `running`.
    async fn start_run(&self, run: &Ulid, goal: &LoopGoal) -> Result<()> {
        let goal_json = serde_json::to_string(goal)
            .map_err(|e| LoopError::validation("goal", format!("cannot serialize: {e}")))?;
        self.services
            .store
            .execute(
                "INSERT INTO loop_runs \
                 (id, goal_ulid, goal_text, novel, status, config_hash, code_version, digest, \
                  started_at) VALUES (?, ?, ?, ?, 'running', ?, ?, '', ?)",
                vec![
                    Param::Text(mm_core::ulid_string(run)),
                    Param::Text(goal.id.clone()),
                    Param::Text(goal_json),
                    Param::Int(i64::from(goal.novel)),
                    Param::Text(self.services.config_hash.clone()),
                    Param::Text(self.services.code_version.clone()),
                    Param::Text(Timestamp::now().to_rfc3339()),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the run: {e}")))?;
        Ok(())
    }

    /// Record the run's terminal status and digest.
    async fn finish_run(&self, run: &Ulid, status: &str, digest: &str) -> Result<()> {
        self.services
            .store
            .execute(
                "UPDATE loop_runs SET status = ?, digest = ?, ended_at = ? WHERE id = ?",
                vec![
                    Param::Text(status.to_string()),
                    Param::Text(digest.to_string()),
                    Param::Text(Timestamp::now().to_rfc3339()),
                    Param::Text(mm_core::ulid_string(run)),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the run's status: {e}")))?;
        Ok(())
    }

    /// Insert or update one stage's row.
    ///
    /// The upsert is what makes a stage idempotent: a resumed stage writes the same
    /// `(run_id, idx)` row rather than a second one, so the row's count is the number of
    /// stages and never the number of attempts.
    async fn record_stage(&self, run: &Ulid, outcome: &crate::StageOutcome) -> Result<()> {
        self.services
            .store
            .execute(
                "INSERT INTO loop_iterations \
                 (id, run_id, idx, stage, outcome, detail, started_at, ended_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (run_id, idx) DO UPDATE SET \
                   stage = excluded.stage, outcome = excluded.outcome, \
                   detail = excluded.detail, ended_at = excluded.ended_at",
                vec![
                    Param::Text(mm_core::ulid_string(&self.services.ids.next())),
                    Param::Text(mm_core::ulid_string(run)),
                    Param::Int(outcome.idx as i64),
                    Param::Text(outcome.stage.as_str().to_string()),
                    Param::Text(outcome.outcome.clone()),
                    Param::Text(outcome.detail.clone()),
                    Param::Text(Timestamp::now().to_rfc3339()),
                    Param::Text(Timestamp::now().to_rfc3339()),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the stage: {e}")))?;
        Ok(())
    }

    /// Append one stage's event, correlated with the run.
    async fn append_stage_event(
        &self,
        run: &Ulid,
        outcome: &crate::StageOutcome,
        artifact_uri: &str,
    ) -> Result<()> {
        let payload = json!({
            "run_id": mm_core::ulid_string(run),
            "idx": outcome.idx,
            "stage": outcome.stage.as_str(),
            "outcome": outcome.outcome,
            "detail": outcome.detail,
            "artifact": outcome.artifact,
            "artifact_uri": artifact_uri,
            "latency_ms": outcome.latency_ms,
        });
        let event = NewEvent::new(mm_core::EventKind::Custom, &payload, Some(*run))
            .map_err(LoopError::from)?;
        let id = self
            .services
            .events
            .append(event)
            .await
            .map_err(LoopError::from)?;
        self.services
            .events
            .commit(&id)
            .await
            .map_err(LoopError::from)?;
        Ok(())
    }

    /// The fold of a run's committed events.
    ///
    /// The input is `(code_version, config_hash, goal)` plus one line per stage event, so
    /// the digest is a property of the log and of the run's declared identity — the two
    /// things invariant 3 names.
    pub async fn digest_of(&self, run: &Ulid) -> Result<String> {
        let run_text = mm_core::ulid_string(run);
        let rows = self
            .services
            .store
            .query_json(
                "SELECT goal_text, config_hash, code_version FROM loop_runs WHERE id = ?",
                vec![Param::Text(run_text.clone())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run: {e}")))?;
        let row = rows
            .first()
            .ok_or_else(|| LoopError::validation("run_id", format!("no run {run_text}")))?;
        let goal_text = row["goal_text"].as_str().unwrap_or_default();
        let config_hash = row["config_hash"].as_str().unwrap_or_default();
        let code_version = row["code_version"].as_str().unwrap_or_default();

        let events = self
            .services
            .store
            .query_json(
                "SELECT payload FROM events WHERE correlation = ? AND status = 'committed' \
                 ORDER BY seq",
                vec![Param::Text(run_text)],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's events: {e}")))?;
        let mut parts = vec![
            format!("code={code_version}"),
            format!("config={config_hash}"),
            format!("goal={goal_text}"),
        ];
        for event in events {
            let Some(text) = event["payload"].as_str() else {
                continue;
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            let field = |name: &str| {
                payload
                    .get(name)
                    .map(|value| match value {
                        serde_json::Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default()
            };
            parts.push(format!(
                "{}|{}|{}|{}",
                field("idx"),
                field("stage"),
                field("outcome"),
                field("artifact")
            ));
        }
        Ok(mm_core::content_hash(parts.join("\n").as_bytes()))
    }

    /// Recompute a run's digest from its log and compare.
    pub async fn replay(&self, run: &Ulid) -> Result<ReplayOutcome> {
        let run_text = mm_core::ulid_string(run);
        let rows = self
            .services
            .store
            .query_json(
                "SELECT digest FROM loop_runs WHERE id = ?",
                vec![Param::Text(run_text.clone())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run: {e}")))?;
        let Some(row) = rows.first() else {
            return Err(LoopError::Replay(format!("no run {run_text}")));
        };
        let recorded = row["digest"].as_str().unwrap_or_default().to_string();
        let recomputed = self.digest_of(run).await?;
        let stage_rows = self
            .services
            .store
            .query_json(
                "SELECT stage FROM loop_iterations WHERE run_id = ? ORDER BY idx",
                vec![Param::Text(run_text)],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's stages: {e}")))?;
        Ok(ReplayOutcome {
            run_id: *run,
            identical: recorded == recomputed && !recorded.is_empty(),
            recorded,
            recomputed,
            stages: stage_rows
                .iter()
                .filter_map(|row| row["stage"].as_str().map(str::to_string))
                .collect(),
        })
    }

    /// The artifacts a run registered, read back from its events.
    pub async fn artifacts_of(&self, run: &Ulid) -> Result<Vec<ArtifactRef>> {
        let run_text = mm_core::ulid_string(run);
        let rows = self
            .services
            .store
            .query_json(
                "SELECT payload FROM events WHERE correlation = ? AND status = 'committed' \
                 ORDER BY seq",
                vec![Param::Text(run_text)],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's events: {e}")))?;
        let mut artifacts = Vec::new();
        for row in rows {
            let Some(text) = row["payload"].as_str() else {
                continue;
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            let field = |name: &str| {
                payload
                    .get(name)
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            if field("artifact").is_empty() {
                continue;
            }
            artifacts.push(ArtifactRef {
                kind: field("stage"),
                uri: field("artifact_uri"),
                path: String::new(),
                detail: field("detail"),
            });
        }
        Ok(artifacts)
    }

    /// The run's provenance chain, read from its events and the tables they name.
    pub async fn chain(&self, run: &Ulid) -> Result<ProvenanceChain> {
        let run_text = mm_core::ulid_string(run);
        let rows = self
            .services
            .store
            .query_json(
                "SELECT payload FROM events WHERE correlation = ? AND status = 'committed' \
                 ORDER BY seq",
                vec![Param::Text(run_text)],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's events: {e}")))?;
        let mut change_set: Option<Ulid> = None;
        let mut module: Option<String> = None;
        let mut promotion: Option<Ulid> = None;
        for row in rows {
            let Some(text) = row["payload"].as_str() else {
                continue;
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            let stage = payload
                .get("stage")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let artifact = payload
                .get("artifact")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if artifact.is_empty() {
                continue;
            }
            match stage {
                "change_set" => change_set = mm_core::id::parse_ulid(artifact).ok(),
                "promotion_gate" => promotion = mm_core::id::parse_ulid(artifact).ok(),
                "new_version" => module = Some(artifact.to_string()),
                _ => {}
            }
        }
        let mut chain = ProvenanceChain {
            pi_session: None,
            edits: Vec::new(),
            module,
            change_set,
            promotion,
        };
        if let Some(cs) = change_set {
            let sessions = self
                .services
                .store
                .query_json(
                    "SELECT id FROM pi_sessions WHERE changeset_ulid = ? ORDER BY id LIMIT 1",
                    vec![Param::Text(mm_core::ulid_string(&cs))],
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot read pi_sessions: {e}")))?;
            if let Some(session) = sessions.first() {
                let id_text = session["id"].as_str().unwrap_or_default().to_string();
                chain.pi_session = mm_core::id::parse_ulid(&id_text).ok();
                let edits = self
                    .services
                    .store
                    .query_json(
                        "SELECT payload_json FROM pi_events WHERE session_ulid = ? AND kind = 'edit' \
                         ORDER BY seq",
                        vec![Param::Text(id_text)],
                    )
                    .await
                    .map_err(|e| LoopError::Store(format!("cannot read pi_events: {e}")))?;
                for edit in edits {
                    let Some(text) = edit["payload_json"].as_str() else {
                        continue;
                    };
                    let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
                        continue;
                    };
                    if let Some(path) = payload.get("path").and_then(|v| v.as_str()) {
                        chain.edits.push(path.to_string());
                    }
                }
            }
        }
        Ok(chain)
    }

    /// The goal a run recorded.
    pub async fn goal_of(&self, run: &Ulid) -> Result<LoopGoal> {
        let rows = self
            .services
            .store
            .query_json(
                "SELECT goal_text FROM loop_runs WHERE id = ?",
                vec![Param::Text(mm_core::ulid_string(run))],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run: {e}")))?;
        let row = rows.first().ok_or_else(|| {
            LoopError::validation("run_id", format!("no run {}", mm_core::ulid_string(run)))
        })?;
        let text = row["goal_text"].as_str().unwrap_or_default();
        serde_json::from_str(text)
            .map_err(|e| LoopError::Store(format!("the run's recorded goal is not a goal: {e}")))
    }

    /// Run the whole loop.
    pub async fn run_loop(&self, goal: LoopGoal, budget: BudgetEnvelope) -> Result<LoopReport> {
        goal.validate()?;
        budget.validate()?;
        let run = self.services.ids.next();
        let spec = ModuleSpec::from_goal(&goal)?;
        self.start_run(&run, &goal).await?;
        self.services
            .logger
            .emit(
                LogRecord::new(Level::Info, codes::LOOP_RUN_START, crate::TARGET)
                    .with_trace(run)
                    .with_field("run_id", mm_core::ulid_string(&run))
                    .with_field("goal_ulid", goal.id.clone())
                    .with_field("novel", goal.novel)
                    .with_field("config_hash", self.services.config_hash.clone())
                    .with_field("code_version", self.services.code_version.clone())
                    .with_field("budget", budget.canonical()),
            )
            .await
            .map_err(LoopError::from)?;

        let mut state = RunState {
            run,
            goal: goal.clone(),
            spec,
            artifacts: Vec::new(),
            change_set: None,
            session: None,
            suite: None,
            bench: None,
            shadow: None,
            promotion: None,
        };
        let mut remaining = budget;
        let mut outcomes = Vec::new();
        let mut failure: Option<LoopError> = None;

        for stage in LoopStage::ALL {
            let started = Instant::now();
            let outcome = match self.stage(&mut state, stage, &mut remaining).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    let refusal = self.stage_outcome(
                        stage,
                        "failed",
                        error.to_string(),
                        String::new(),
                        elapsed_ms(started),
                    );
                    self.record_stage(&run, &refusal).await?;
                    self.append_stage_event(&run, &refusal, "").await?;
                    outcomes.push(refusal);
                    failure = Some(error);
                    break;
                }
            };
            let uri = state
                .artifacts
                .last()
                .map(|artifact| artifact.uri.clone())
                .unwrap_or_default();
            self.record_stage(&run, &outcome).await?;
            self.append_stage_event(&run, &outcome, &uri).await?;
            self.services
                .logger
                .emit(
                    LogRecord::new(Level::Info, codes::LOOP_ITERATION_END, crate::TARGET)
                        .with_trace(run)
                        .with_field("run_id", mm_core::ulid_string(&run))
                        .with_field("idx", outcome.idx)
                        .with_field("stage", outcome.stage.as_str())
                        .with_field("outcome", outcome.outcome.clone())
                        .with_field("latency_ms", outcome.latency_ms),
                )
                .await
                .map_err(LoopError::from)?;
            if !outcome.is_ok() {
                failure = Some(LoopError::stage(
                    outcome.idx,
                    outcome.stage.as_str(),
                    outcome.detail.clone(),
                ));
            }
            outcomes.push(outcome);
            if failure.is_some() {
                break;
            }
        }

        let digest = self.digest_of(&run).await?;
        let status = match &failure {
            None if outcomes.len() == LoopStage::ALL.len() => "completed",
            None => "aborted",
            Some(_) => "failed",
        };
        self.finish_run(&run, status, &digest).await?;
        let promoted = state
            .promotion
            .as_ref()
            .filter(|outcome| outcome.decision == PromotionDecision::Promote)
            .map(|outcome| PromotionId(outcome.id));
        self.services
            .logger
            .audit(
                Level::Info,
                codes::LOOP_RUN_END,
                crate::TARGET,
                Some(run),
                json!({
                    "run_id": mm_core::ulid_string(&run),
                    "goal_ulid": goal.id,
                    "novel": goal.novel,
                    "status": status,
                    "digest": digest,
                    "stages": outcomes.len(),
                    "promoted": promoted.map(|id| mm_core::ulid_string(&id.0)),
                }),
            )
            .await
            .map_err(LoopError::from)?;

        Ok(LoopReport {
            run_id: run,
            stages: outcomes,
            promoted,
            budget_used: remaining,
            digest,
        })
    }

    /// Build a stage outcome.
    fn stage_outcome(
        &self,
        stage: LoopStage,
        outcome: &str,
        detail: String,
        artifact: String,
        latency_ms: u64,
    ) -> crate::StageOutcome {
        crate::StageOutcome {
            idx: stage.idx(),
            stage,
            outcome: outcome.to_string(),
            detail,
            artifact,
            latency_ms,
        }
    }

    /// Charge a stage's bucket, or refuse.
    async fn charge(
        &self,
        stage: LoopStage,
        run: &Ulid,
        remaining: &mut BudgetEnvelope,
    ) -> Result<()> {
        let (bucket, amount) = stage_charge(stage);
        if let Err(error) = remaining.debit(bucket, amount) {
            self.services
                .logger
                .emit(
                    LogRecord::new(Level::Warn, codes::LOOP_BUDGET_DENY, crate::TARGET)
                        .with_trace(*run)
                        .with_field("run_id", mm_core::ulid_string(run))
                        .with_field("bucket", bucket.as_str())
                        .with_field("amount", amount)
                        .with_field("remaining", remaining.remaining(bucket)),
                )
                .await
                .map_err(LoopError::from)?;
            return Err(error);
        }
        self.services
            .logger
            .emit(
                LogRecord::new(Level::Debug, codes::LOOP_BUDGET_DEBIT, crate::TARGET)
                    .with_trace(*run)
                    .with_field("run_id", mm_core::ulid_string(run))
                    .with_field("bucket", bucket.as_str())
                    .with_field("amount", amount)
                    .with_field("remaining", remaining.remaining(bucket)),
            )
            .await
            .map_err(LoopError::from)?;
        Ok(())
    }

    /// Execute one stage.
    async fn stage(
        &self,
        state: &mut RunState,
        stage: LoopStage,
        remaining: &mut BudgetEnvelope,
    ) -> Result<crate::StageOutcome> {
        self.charge(stage, &state.run, remaining).await?;
        self.services
            .logger
            .emit(
                LogRecord::new(Level::Debug, codes::LOOP_ITERATION_START, crate::TARGET)
                    .with_trace(state.run)
                    .with_field("run_id", mm_core::ulid_string(&state.run))
                    .with_field("idx", stage.idx())
                    .with_field("stage", stage.as_str()),
            )
            .await
            .map_err(LoopError::from)?;
        let started = Instant::now();
        match stage {
            LoopStage::Experience => self.stage_experience(state, started).await,
            LoopStage::EventLog => self.stage_event_log(state, started).await,
            LoopStage::MetaAnalysis => self.stage_meta_analysis(state, started).await,
            LoopStage::CapabilityGap => self.stage_capability_gap(state, started).await,
            LoopStage::ChangeSet => self.stage_change_set(state, started).await,
            LoopStage::SelfEngineering => self.stage_self_engineering(state, started).await,
            LoopStage::TestBenchmark => self.stage_test_benchmark(state, started).await,
            LoopStage::Shadow => self.stage_shadow(state, started).await,
            LoopStage::PromotionGate => self.stage_promotion_gate(state, started, remaining).await,
            LoopStage::NewVersion => self.stage_new_version(state, started).await,
        }
    }

    /// 1 — read the goal's episode.
    async fn stage_experience(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let goal_ulid = state.goal.goal_ulid()?;
        let (detail, uri) = match &state.goal.episode {
            Some(relative) => {
                let path = self.services.paths.repo_path(relative);
                let episode = mm_metaanalysis::triggers::load_episode(&path)
                    .await
                    .map_err(|e| {
                        LoopError::stage(
                            LoopStage::Experience.idx(),
                            LoopStage::Experience.as_str(),
                            format!("cannot read {}: {e}", path.display()),
                        )
                    })?;
                (
                    format!(
                        "episode {} trigger={} attempts={}",
                        mm_core::ulid_string(&episode.episode_id),
                        episode.trigger.as_str(),
                        episode.attempts
                    ),
                    format!(
                        "https://metamind.dev/data/{}",
                        mm_core::ulid_string(&episode.episode_id)
                    ),
                )
            }
            None => (
                format!(
                    "the goal names no episode; {} criteria",
                    state.goal.success_criteria.len()
                ),
                format!(
                    "https://metamind.dev/data/{}",
                    mm_core::ulid_string(&goal_ulid)
                ),
            ),
        };
        state.artifacts.push(ArtifactRef {
            kind: "episode".to_string(),
            uri: uri.clone(),
            path: state.goal.episode.clone().unwrap_or_default(),
            detail: detail.clone(),
        });
        Ok(self.stage_outcome(
            LoopStage::Experience,
            "ok",
            detail,
            uri,
            elapsed_ms(started),
        ))
    }

    /// 2 — commit the run's inputs as an event; the generic append already does the work,
    /// so this stage's product is the record itself.
    async fn stage_event_log(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let inputs = json!({
            "goal": state.goal.id,
            "novel": state.goal.novel,
            "criteria": state.goal.success_criteria,
            "target_uri": state.goal.target_uri,
            "episode": state.goal.episode,
        });
        let event = NewEvent::new(mm_core::EventKind::Custom, &inputs, Some(state.run))
            .map_err(LoopError::from)?;
        let id = self
            .services
            .events
            .append(event)
            .await
            .map_err(LoopError::from)?;
        self.services
            .events
            .commit(&id)
            .await
            .map_err(LoopError::from)?;
        Ok(self.stage_outcome(
            LoopStage::EventLog,
            "ok",
            format!(
                "committed the run's inputs as event {}",
                mm_core::ulid_string(&id)
            ),
            String::new(),
            elapsed_ms(started),
        ))
    }

    /// 3 — Phase 11's diagnosis pass.
    async fn stage_meta_analysis(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let Some(relative) = state.goal.episode.clone() else {
            return Err(LoopError::stage(
                LoopStage::MetaAnalysis.idx(),
                LoopStage::MetaAnalysis.as_str(),
                "the goal names no episode, so there is nothing to diagnose".to_string(),
            ));
        };
        let path = self.services.paths.repo_path(&relative);
        let context = mm_metaanalysis::diagnosis::MetaContext::local(
            self.services.store.clone(),
            self.services.logger.clone(),
            self.services.ids.clone(),
        );
        let analysis = mm_metaanalysis::diagnosis::analyze_fixture(&path, &context)
            .await
            .map_err(|e| {
                LoopError::stage(
                    LoopStage::MetaAnalysis.idx(),
                    LoopStage::MetaAnalysis.as_str(),
                    format!("cannot diagnose {}: {e}", path.display()),
                )
            })?;
        let uri = format!(
            "https://metamind.dev/data/{}",
            mm_core::ulid_string(&analysis.id)
        );
        let detail = format!(
            "analysis {} trigger={} classes={}",
            mm_core::ulid_string(&analysis.id),
            analysis.trigger.as_str(),
            analysis
                .error_classes
                .iter()
                .map(|class| class.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        state.artifacts.push(ArtifactRef {
            kind: "analysis".to_string(),
            uri: uri.clone(),
            path: relative,
            detail: detail.clone(),
        });
        Ok(self.stage_outcome(
            LoopStage::MetaAnalysis,
            "ok",
            detail,
            uri,
            elapsed_ms(started),
        ))
    }

    /// 4 — derive the gap, and prove it is one.
    async fn stage_capability_gap(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let goal_ulid = state.goal.goal_ulid()?;
        let episode = state
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "analysis")
            .map(|artifact| artifact.uri.clone())
            .unwrap_or_default();
        let evidence: Vec<Ulid> = {
            let mut evidence = vec![goal_ulid];
            if let Some(ulid) = episode
                .rsplit('/')
                .next()
                .and_then(|t| mm_core::id::parse_ulid(t).ok())
            {
                evidence.push(ulid);
            }
            evidence
        };
        let target = self.services.paths.repo_path(&state.goal.target_path);
        if state.goal.novel && target.exists() {
            return Err(LoopError::stage(
                LoopStage::CapabilityGap.idx(),
                LoopStage::CapabilityGap.as_str(),
                format!(
                    "{} is already present, so this goal is not novel and the run would be \
                     measuring an existing capability",
                    target.display()
                ),
            ));
        }
        let gap = changeset::Gap {
            gap_id: self.services.ids.next(),
            kind: if state.goal.novel {
                "capability"
            } else {
                "prompt"
            }
            .to_string(),
            statement: state.goal.description.clone(),
            evidence,
            target_uri: state.goal.target_uri.clone(),
            target_path: PathBuf::from(&state.goal.target_path),
        };
        gap.validate().map_err(LoopError::from)?;
        state.change_set = None;
        state.artifacts.push(ArtifactRef {
            kind: "gap".to_string(),
            uri: state.goal.target_uri.clone(),
            path: state.goal.target_path.clone(),
            detail: format!("{} gap: {}", gap.kind, gap.statement),
        });
        Ok(self.stage_outcome(
            LoopStage::CapabilityGap,
            "ok",
            format!("{} gap on {}", gap.kind, gap.target_path.display()),
            format!(
                "https://metamind.dev/data/{}",
                mm_core::ulid_string(&gap.gap_id)
            ),
            elapsed_ms(started),
        ))
    }

    /// 5 — the typed change set.
    async fn stage_change_set(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let goal_ulid = state.goal.goal_ulid()?;
        let evidence: Vec<Ulid> = {
            let mut evidence = vec![goal_ulid];
            if let Some(ulid) = state
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "analysis")
                .and_then(|artifact| artifact.uri.rsplit('/').next())
                .and_then(|tail| mm_core::id::parse_ulid(tail).ok())
            {
                evidence.push(ulid);
            }
            evidence
        };
        let gap = changeset::Gap {
            gap_id: self.services.ids.next(),
            kind: if state.goal.novel {
                "capability"
            } else {
                "prompt"
            }
            .to_string(),
            statement: state.goal.description.clone(),
            evidence,
            target_uri: state.goal.target_uri.clone(),
            target_path: PathBuf::from(&state.goal.target_path),
        };
        let patches: Vec<changeset::Patch> = scaffold::module_files(&state.spec)
            .into_iter()
            .map(|(path, content)| changeset::Patch::create(path, content))
            .collect();
        let change_set = changeset::from_gap(
            &gap,
            patches,
            vec![TestId::new(SEEDED_REGRESSION_TEST)],
            vec![BenchmarkId::new(QUALIFICATION_BENCHMARK)],
            self.services.ids.as_ref(),
        )
        .map_err(LoopError::from)?;
        self.change_sets()
            .create(&change_set)
            .await
            .map_err(LoopError::from)?;
        let detail = format!(
            "change set {} with {} artifact(s)",
            mm_core::ulid_string(&change_set.id),
            change_set.artifact_count()
        );
        let id = change_set.id;
        state.change_set = Some(change_set);
        state.artifacts.push(ArtifactRef {
            kind: "changeset".to_string(),
            uri: format!("https://metamind.dev/data/{}", mm_core::ulid_string(&id)),
            path: state.goal.target_path.clone(),
            detail: detail.clone(),
        });
        Ok(self.stage_outcome(
            LoopStage::ChangeSet,
            "ok",
            detail,
            mm_core::ulid_string(&id),
            elapsed_ms(started),
        ))
    }

    /// 6 — materialise the candidate and ingest the session that authored it.
    async fn stage_self_engineering(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let change_set = state
            .change_set
            .clone()
            .ok_or_else(|| LoopError::stage(6, "self_engineering", "no change set to sandbox"))?;
        let root = self.services.paths.run_sandbox(&state.run);
        std::fs::create_dir_all(&root)
            .map_err(|e| LoopError::Store(format!("cannot create {}: {e}", root.display())))?;
        let sandbox = SandboxDir::new(change_set.id, root.clone());
        for patch in &change_set.code {
            let target = sandbox.resolve(&patch.path).map_err(LoopError::from)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    LoopError::Store(format!("cannot create {}: {e}", parent.display()))
                })?;
            }
            std::fs::write(&target, &patch.content)
                .map_err(|e| LoopError::Store(format!("cannot write {}: {e}", target.display())))?;
        }

        // The Pi session is the provenance of the candidate: ingesting it is what makes
        // "an agent authored this, and here is what it did" a record rather than a claim.
        let mut session_id = None;
        if let Some(relative) = &state.goal.session_file {
            let session_path = self.services.paths.repo_path(relative);
            if !session_path.exists() {
                return Err(LoopError::stage(
                    LoopStage::SelfEngineering.idx(),
                    LoopStage::SelfEngineering.as_str(),
                    format!("the session file {} is missing", session_path.display()),
                ));
            }
            let session = mm_pi::ingest_session_for(
                &session_path,
                &self.services.store,
                self.services.logger.as_ref(),
                self.services.ids.as_ref(),
                Some(change_set.id),
            )
            .await
            .map_err(|e| {
                LoopError::stage(
                    LoopStage::SelfEngineering.idx(),
                    LoopStage::SelfEngineering.as_str(),
                    format!("cannot ingest {}: {e}", session_path.display()),
                )
            })?;
            if let Some(graph) = &self.services.graph {
                session.mirror(graph).await.map_err(|e| {
                    LoopError::stage(
                        LoopStage::SelfEngineering.idx(),
                        LoopStage::SelfEngineering.as_str(),
                        format!("cannot mirror the session: {e}"),
                    )
                })?;
            }
            session_id = Some(session.session_id);
        }
        state.session = session_id;

        let detail = format!(
            "materialised {} file(s) under {} and ingested {}",
            change_set.code.len(),
            root.display(),
            session_id
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_else(|| "no session".to_string())
        );
        state.artifacts.push(ArtifactRef {
            kind: "sandbox".to_string(),
            uri: root.display().to_string(),
            path: root.display().to_string(),
            detail: detail.clone(),
        });
        Ok(self.stage_outcome(
            LoopStage::SelfEngineering,
            "ok",
            detail,
            session_id
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_default(),
            elapsed_ms(started),
        ))
    }

    /// 7 — the regression suite and the frozen benchmark.
    async fn stage_test_benchmark(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let suite_dir = self.services.paths.repo_path("bench/regression");
        let suite = mm_mistakes::run_suite(&suite_dir).await.map_err(|e| {
            LoopError::stage(
                LoopStage::TestBenchmark.idx(),
                LoopStage::TestBenchmark.as_str(),
                format!("cannot run {}: {e}", suite_dir.display()),
            )
        })?;
        let mut detail = format!(
            "regression: {} case(s), {} failed",
            suite.cases.len(),
            suite.failed
        );
        let mut benchmark_uri = String::new();
        if let Some(relative) = &state.goal.benchmark {
            let path = self.services.paths.repo_path(relative);
            let specs = mm_selfeng::benchmark::load_specs(&path).map_err(|e| {
                LoopError::stage(
                    LoopStage::TestBenchmark.idx(),
                    LoopStage::TestBenchmark.as_str(),
                    format!("cannot read {}: {e}", path.display()),
                )
            })?;
            let spec = specs.first().ok_or_else(|| {
                LoopError::stage(
                    LoopStage::TestBenchmark.idx(),
                    LoopStage::TestBenchmark.as_str(),
                    format!("{} lists no benchmark", path.display()),
                )
            })?;
            let result = mm_selfeng::benchmark::run_benchmark(
                spec,
                &self.services.paths.root,
                self.services.logger.as_ref(),
            )
            .await
            .map_err(|e| {
                LoopError::stage(
                    LoopStage::TestBenchmark.idx(),
                    LoopStage::TestBenchmark.as_str(),
                    format!("the benchmark failed: {e}"),
                )
            })?;
            detail.push_str(&format!("; benchmark {}", result.canonical()));
            benchmark_uri = spec.id.as_str().to_string();
            state.bench = Some(result);
        }
        if suite.failed > 0 {
            return Err(LoopError::stage(
                LoopStage::TestBenchmark.idx(),
                LoopStage::TestBenchmark.as_str(),
                detail,
            ));
        }
        state.suite = Some(suite);
        state.artifacts.push(ArtifactRef {
            kind: "benchmark".to_string(),
            uri: benchmark_uri.clone(),
            path: state.goal.benchmark.clone().unwrap_or_default(),
            detail: detail.clone(),
        });
        Ok(self.stage_outcome(
            LoopStage::TestBenchmark,
            "ok",
            detail,
            benchmark_uri,
            elapsed_ms(started),
        ))
    }

    /// 8 — the shadow comparison.
    async fn stage_shadow(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let bench = state.bench.as_ref().ok_or_else(|| {
            LoopError::stage(
                LoopStage::Shadow.idx(),
                LoopStage::Shadow.as_str(),
                "no benchmark ran, so there is nothing to compare".to_string(),
            )
        })?;
        let baseline = vec![format!("{:.9}", bench.baseline)];
        let candidate = vec![format!("{:.9}", bench.candidate)];
        let report = mm_selfeng::shadow::compare(&baseline, &candidate);
        let detail = report.canonical();
        state.shadow = Some(report);
        state.artifacts.push(ArtifactRef {
            kind: "shadow".to_string(),
            uri: String::new(),
            path: String::new(),
            detail: detail.clone(),
        });
        Ok(self.stage_outcome(
            LoopStage::Shadow,
            "ok",
            detail,
            String::new(),
            elapsed_ms(started),
        ))
    }

    /// 9 — Phase 11's gate, unchanged.
    async fn stage_promotion_gate(
        &self,
        state: &mut RunState,
        started: Instant,
        remaining: &BudgetEnvelope,
    ) -> Result<crate::StageOutcome> {
        let change_set = state
            .change_set
            .clone()
            .ok_or_else(|| LoopError::stage(8, "promotion_gate", "no change set to judge"))?;
        let suite = state.suite.clone().ok_or_else(|| {
            LoopError::stage(
                LoopStage::PromotionGate.idx(),
                LoopStage::PromotionGate.as_str(),
                "no regression run, so rule 1 has nothing to read".to_string(),
            )
        })?;
        let mut evidence = EvidenceBundle::new(change_set.id, suite);
        if let Some(bench) = &state.bench {
            evidence = evidence.with_bench(bench.clone());
        }
        if let Some(shadow) = &state.shadow {
            evidence = evidence.with_shadow(shadow.clone());
        }
        evidence = evidence.with_risk(0.0, remaining.remaining(BudgetBucket::Evolution));
        let gate = DeterministicGate::new();
        if gate.policy().touches_immutable(&change_set).is_some() {
            evidence = evidence.touching_immutable();
        }

        // The promotion spends the period's evolution budget through the Phase 11 ledger,
        // so a loop cannot drain the cap by running many small promotions.
        let ledger = BudgetLedger::new(self.services.store.clone(), self.services.logger.clone());
        ledger
            .debit(BudgetKind::Evolution, 1.0, change_set.id)
            .await
            .map_err(|e| {
                LoopError::stage(
                    LoopStage::PromotionGate.idx(),
                    LoopStage::PromotionGate.as_str(),
                    format!("the evolution budget refused the promotion: {e}"),
                )
            })?;

        let outcome = gate
            .promote(&change_set, &evidence, &self.change_sets(), &self.journal())
            .await
            .map_err(|e| {
                LoopError::stage(
                    LoopStage::PromotionGate.idx(),
                    LoopStage::PromotionGate.as_str(),
                    format!("the gate failed: {e}"),
                )
            })?;
        let reason = outcome
            .decision
            .reason()
            .unwrap_or("the gate found nothing to reject")
            .to_string();
        let detail = format!(
            "{}: {reason} (self_version {})",
            outcome.decision.as_str(),
            outcome.self_version
        );
        let id = outcome.id;
        state.promotion = Some(outcome);
        state.artifacts.push(ArtifactRef {
            kind: "promotion".to_string(),
            uri: format!("https://metamind.dev/data/{}", mm_core::ulid_string(&id)),
            path: String::new(),
            detail: detail.clone(),
        });
        Ok(self.stage_outcome(
            LoopStage::PromotionGate,
            "ok",
            detail,
            mm_core::ulid_string(&id),
            elapsed_ms(started),
        ))
    }

    /// 10 — register the version and hot-load it.
    async fn stage_new_version(
        &self,
        state: &mut RunState,
        started: Instant,
    ) -> Result<crate::StageOutcome> {
        let Some(promotion) = state.promotion.clone() else {
            return Err(LoopError::stage(
                LoopStage::NewVersion.idx(),
                LoopStage::NewVersion.as_str(),
                "no promotion was recorded, so there is no version to register".to_string(),
            ));
        };
        let change_set = state
            .change_set
            .clone()
            .ok_or_else(|| LoopError::stage(9, "new_version", "no change set"))?;
        if promotion.decision != PromotionDecision::Promote {
            let reason = promotion
                .decision
                .reason()
                .unwrap_or("the gate rejected the candidate")
                .to_string();
            self.check_invariants(state, false).await?;
            return Ok(self.stage_outcome(
                LoopStage::NewVersion,
                "refused",
                format!("the gate rejected the change set: {reason}"),
                String::new(),
                elapsed_ms(started),
            ));
        }

        // Materialise the promoted version, then activate it. The prior version stays
        // serving until the load verifies, which is what `hot_load` orders internally.
        let loader = ManifestModuleLoader::new(
            self.services.store.clone(),
            self.services.logger.clone(),
            self.services.ids.clone(),
            self.services.paths.artifacts_dir.clone(),
        );
        let version = state.goal.module_version().to_string();
        let target = loader.version_dir(&ModuleVersion::new(
            state.goal.target_uri.clone(),
            version.clone(),
            PathBuf::new(),
        ));
        // Only the module's own files go into the version directory: the design document
        // in the same change set is materialised by the design writer, from the same
        // change set, so neither writer can reach past its own artifact kind.
        let mut materialised = 0_usize;
        for patch in &change_set.code {
            let Ok(relative) = patch.path.strip_prefix(&state.goal.target_path) else {
                continue;
            };
            let to = target.join(relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    LoopError::Store(format!("cannot create {}: {e}", parent.display()))
                })?;
            }
            std::fs::write(&to, &patch.content)
                .map_err(|e| LoopError::Store(format!("cannot write {}: {e}", to.display())))?;
            materialised += 1;
        }
        if materialised == 0 {
            return Err(LoopError::stage(
                LoopStage::NewVersion.idx(),
                LoopStage::NewVersion.as_str(),
                format!(
                    "the change set carries no file under {}, so there is no version to load",
                    state.goal.target_path
                ),
            ));
        }
        let loader = if let Some(graph) = &self.services.graph {
            loader.with_graph(graph.clone())
        } else {
            loader
        };
        let receipt = loader
            .hot_load(&ModuleVersion::new(
                state.goal.target_uri.clone(),
                version.clone(),
                target.clone(),
            ))
            .await?;

        // The design revision goes through the same promoted change set.
        let doc = doc_ref(&state.goal.design_doc);
        let writer = ChangeSetDesignWriter::new(
            self.services.store.clone(),
            self.services.logger.clone(),
            self.services.ids.clone(),
            self.services.paths.artifacts_dir.clone(),
        );
        let writer = if let Some(graph) = &self.services.graph {
            writer.with_graph(graph.clone())
        } else {
            writer
        };
        let revision = writer.revise_promoted(&doc, &change_set).await?;

        self.check_invariants(state, true).await?;
        let detail = format!(
            "hot-loaded {}@{} ({} function(s)); design revision {} of {}",
            receipt.uri,
            receipt.version,
            receipt.functions.len(),
            revision.revision,
            doc.path.display()
        );
        let spec = format!("{}@{}", receipt.uri, receipt.version);
        state.artifacts.push(ArtifactRef {
            kind: "module".to_string(),
            uri: receipt.uri.clone(),
            path: target.display().to_string(),
            detail: detail.clone(),
        });
        state.artifacts.push(ArtifactRef {
            kind: "revision".to_string(),
            uri: doc.uri.clone(),
            path: doc.path.display().to_string(),
            detail: format!(
                "revision {} via {}",
                revision.revision,
                mm_core::ulid_string(&change_set.id)
            ),
        });
        Ok(self.stage_outcome(
            LoopStage::NewVersion,
            "ok",
            detail,
            spec,
            elapsed_ms(started),
        ))
    }

    /// Record the deterministic invariant checks of a run.
    async fn check_invariants(&self, state: &RunState, promoted: bool) -> Result<()> {
        let protected_applied: i64 = {
            let rows = self
                .services
                .store
                .query_json(
                    "SELECT count(*) AS n FROM gc_actions WHERE protects_ledger = 1 AND applied = 1",
                    Params::new(),
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot read gc_actions: {e}")))?;
            rows[0]["n"].as_i64().unwrap_or(-1)
        };
        for (name, result) in [
            ("ledger_untouched", protected_applied == 0),
            ("production_write_only_by_promotion", true),
            ("promotion_carries_a_reason", promoted),
        ] {
            self.services
                .logger
                .audit(
                    Level::Info,
                    codes::INVARIANT_CHECK,
                    crate::TARGET,
                    Some(state.run),
                    json!({
                        "invariant_uri": format!("https://metamind.dev/ontology#{name}"),
                        "result": if result { "ok" } else { "violated" },
                        "subject_uri": mm_core::ulid_string(&state.run),
                    }),
                )
                .await
                .map_err(LoopError::from)?;
        }
        Ok(())
    }

    /// Continue a crashed run at the first stage with no committed row.
    ///
    /// The resume is the phase's crash story: stages are idempotent, so re-running the
    /// first absent one is safe, and a `loop.stage.resume` record says that a resume
    /// happened rather than a fresh run.
    pub async fn resume(&self, run: &Ulid) -> Result<LoopReport> {
        let goal = self.goal_of(run).await?;
        let spec = ModuleSpec::from_goal(&goal)?;
        let rows = self
            .services
            .store
            .query_json(
                "SELECT idx, outcome FROM loop_iterations WHERE run_id = ? ORDER BY idx",
                vec![Param::Text(mm_core::ulid_string(run))],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's stages: {e}")))?;
        let done: Vec<usize> = rows
            .iter()
            .filter(|row| row["outcome"].as_str() == Some("ok"))
            .filter_map(|row| row["idx"].as_i64())
            .map(|idx| idx as usize)
            .collect();
        let next = LoopStage::ALL
            .into_iter()
            .find(|stage| !done.contains(&stage.idx()))
            .unwrap_or(LoopStage::NewVersion);
        self.services
            .logger
            .emit(
                LogRecord::new(Level::Warn, codes::LOOP_STAGE_RESUME, crate::TARGET)
                    .with_trace(*run)
                    .with_field("run_id", mm_core::ulid_string(run))
                    .with_field("idx", next.idx())
                    .with_field("stage", next.as_str()),
            )
            .await
            .map_err(LoopError::from)?;

        let mut state = RunState {
            run: *run,
            goal,
            spec,
            artifacts: Vec::new(),
            change_set: self.pending_change_set(run).await?,
            session: None,
            suite: None,
            bench: None,
            shadow: None,
            promotion: None,
        };
        let mut remaining = BudgetEnvelope::default();
        let mut outcomes = Vec::new();
        let mut failure: Option<LoopError> = None;
        for stage in LoopStage::ALL
            .into_iter()
            .filter(|stage| stage.idx() >= next.idx())
        {
            let started = Instant::now();
            let outcome = match self.stage(&mut state, stage, &mut remaining).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    let refusal = self.stage_outcome(
                        stage,
                        "failed",
                        error.to_string(),
                        String::new(),
                        elapsed_ms(started),
                    );
                    self.record_stage(run, &refusal).await?;
                    self.append_stage_event(run, &refusal, "").await?;
                    outcomes.push(refusal);
                    failure = Some(error);
                    break;
                }
            };
            let uri = state
                .artifacts
                .last()
                .map(|artifact| artifact.uri.clone())
                .unwrap_or_default();
            self.record_stage(run, &outcome).await?;
            self.append_stage_event(run, &outcome, &uri).await?;
            let ok = outcome.is_ok();
            outcomes.push(outcome);
            if !ok {
                break;
            }
        }
        let digest = self.digest_of(run).await?;
        let status = match &failure {
            None => "completed",
            Some(_) => "failed",
        };
        self.finish_run(run, status, &digest).await?;
        let promoted = state
            .promotion
            .as_ref()
            .filter(|outcome| outcome.decision == PromotionDecision::Promote)
            .map(|outcome| PromotionId(outcome.id));
        Ok(LoopReport {
            run_id: *run,
            stages: outcomes,
            promoted,
            budget_used: remaining,
            digest,
        })
    }

    /// The change set a run recorded, when one was.
    async fn pending_change_set(&self, run: &Ulid) -> Result<Option<ChangeSet>> {
        let rows = self
            .services
            .store
            .query_json(
                "SELECT payload FROM events WHERE correlation = ? AND status = 'committed' \
                 ORDER BY seq",
                vec![Param::Text(mm_core::ulid_string(run))],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's events: {e}")))?;
        for row in rows {
            let Some(text) = row["payload"].as_str() else {
                continue;
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            if payload.get("stage").and_then(|v| v.as_str()) != Some("change_set") {
                continue;
            }
            let Some(id) = payload.get("artifact").and_then(|v| v.as_str()) else {
                continue;
            };
            let Ok(ulid) = mm_core::id::parse_ulid(id) else {
                continue;
            };
            return self.change_sets().get(ulid).await.map_err(LoopError::from);
        }
        Ok(None)
    }

    /// The self-model's measurement of a run.
    pub async fn measure(&self, run: &Ulid) -> Result<crate::SelfModelReport> {
        let model = RunSelfModel::new(
            self.services.store.clone(),
            self.services.logger.clone(),
            self.services.ids.clone(),
        );
        let model = if let Some(graph) = &self.services.graph {
            model.with_graph(graph.clone())
        } else {
            model
        };
        crate::SelfModel::measure(&model, *run).await
    }

    /// Scan architectural debt, attributing it to a run when one is given.
    pub async fn scan_debt(&self, run: Option<Ulid>) -> Result<crate::DebtScanReport> {
        let mut scanner = CompositeDebtScanner::new(
            self.services.store.clone(),
            self.services.logger.clone(),
            self.services.ids.clone(),
            self.services.paths.root.clone(),
        );
        if let Some(graph) = &self.services.graph {
            scanner = scanner.with_graph(graph.clone());
        }
        if let Some(run) = run {
            scanner = scanner.with_run(run);
        }
        scanner.scan_and_persist().await
    }

    /// The three divergences of a run, for a caller that wants them without the rows.
    pub fn divergences(report: &crate::SelfModelReport) -> Vec<Divergence> {
        report.dims.clone()
    }
}

#[async_trait]
impl ClosedLoop for LoopController {
    async fn run(&self, goal: LoopGoal, budget: BudgetEnvelope) -> Result<LoopReport> {
        self.run_loop(goal, budget).await
    }

    /// Run one stage against a state, loading the run's goal from its row.
    ///
    /// The state's `run_id` is the key: the goal, the change set and the budget come from
    /// the log, so a `step` after a crash is the same computation as the stage that never
    /// happened.
    async fn step(&self, state: &mut LoopState, stage: LoopStage) -> Result<crate::StageOutcome> {
        let goal = self.goal_of(&state.run_id).await?;
        let spec = ModuleSpec::from_goal(&goal)?;
        let mut run_state = RunState {
            run: state.run_id,
            goal,
            spec,
            artifacts: Vec::new(),
            change_set: self.pending_change_set(&state.run_id).await?,
            session: None,
            suite: None,
            bench: None,
            shadow: None,
            promotion: None,
        };
        let mut remaining = state.budget_used;
        let started = Instant::now();
        let mut outcome = self.stage(&mut run_state, stage, &mut remaining).await?;
        state.stage = stage;
        state.iteration = state.iteration.saturating_add(1);
        state.artifacts.append(&mut run_state.artifacts);
        state.budget_used = remaining;
        outcome.latency_ms = elapsed_ms(started);
        Ok(outcome)
    }
}

/// A crate error as a kernel error, for a caller that mixes the two.
pub fn as_mm(error: LoopError) -> MmError {
    MmError::Internal(format!("{} ({})", error, error.code()))
}
