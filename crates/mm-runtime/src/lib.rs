//! `mm-runtime` — autonomy: the closed developmental loop.
//!
//! Phase 11 built the machinery that turns a *diagnosis* into a judged, reversible
//! change. Phase 12 closes the loop around it: given a goal, the runtime drives
//! experience → event log → meta-analysis → capability gap → change set →
//! self-engineering → test/benchmark → shadow → promotion gate → new version, and the
//! result is a promoted, hot-loadable capability produced without a human code edit.
//!
//! Five organs live here, in the order data flows through them:
//!
//! * [`loop_controller`] — the ten-stage [`ClosedLoop`], an idempotent handler per stage
//!   keyed `(run_id, idx)`, with a hard [`BudgetEnvelope`] and a digest folded from the
//!   run's own events.
//! * [`timescale`] — the five clocks of the parent design (§31), each tick recorded.
//! * [`self_model`] — the numeric Actual/Model/Ideal measurement and its divergences.
//! * [`debt`] and [`gc`] — architectural-debt detection, and a collector that walks an
//!   escalation ladder cheapest-first and refuses anything that touches the immutable
//!   developmental ledger.
//! * [`design_writer`] and [`module_loader`] — design revisions through the same
//!   change-set and promotion pipeline as code, and hot-loading of promoted module
//!   versions without restarting the process.
//!
//! # Five invariants, and where each one is kept
//!
//! 1. **The loop decides, not a model.** Every stage outcome is recorded from code; the
//!    promotion decision is Phase 11's [`mm_selfeng::DeterministicGate`], reused
//!    unchanged.
//! 2. **The developmental ledger is immutable.** `gc` computes `protects_ledger` before
//!    it proposes anything, refuses the action when the flag is set, and
//!    `0012_loop.sql` makes such a row append-only so the refusal survives the process
//!    that recorded it.
//! 3. **Replay determinism.** `LoopState` is a projection of the event log: each stage
//!    appends an event correlated with the run, and `loop replay` re-folds those events
//!    and compares the fold with the digest the run recorded.
//! 4. **Budgets are hard.** A stage whose debit would cross the envelope is denied
//!    whole, never truncated.
//! 5. **Production is written only by promotion.** The loop materialises a promoted
//!    capability under its *artifact root* (kernel state, `data/artifacts` by default)
//!    and never into the repository it is running in, which is the property Phase 11's
//!    `audit production-tree` asserts and this phase must not weaken. The decision to
//!    keep the artifact root out of the source tree is recorded in
//!    [`RuntimePaths::artifacts_dir`].
#![forbid(unsafe_code)]

pub mod debt;
pub mod design_writer;
pub mod error;
pub mod gc;
pub mod loop_controller;
pub mod module_loader;
pub mod rdf;
pub mod scaffold;
pub mod self_model;
pub mod timescale;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mm_core::Ulid;
use serde::{Deserialize, Serialize};

pub use debt::{CompositeDebtScanner, DebtFinding, DebtKind, DebtScanReport, DebtScanner};
pub use design_writer::{
    doc_ref, ChangeSetDesignWriter, DesignDocRef, DesignWriter, Revision, DESIGN_ARTIFACT_SUBDIR,
};
pub use error::{LoopError, Result};
pub use gc::{
    Collector, GcAction, GcActionKind, GcRefusal, GcReport, LadderCollector, LedgerImpact,
    GC_LADDER,
};
pub use loop_controller::{LoopController, LoopServices, ProvenanceChain, ReplayOutcome};
pub use module_loader::{
    LoadReceipt, LoadStatus, ManifestModuleLoader, ModuleLoader, ModuleVersion,
    MODULE_ARTIFACT_SUBDIR,
};
pub use self_model::{Divergence, RunSelfModel, SelfModel, SelfModelReport};
pub use timescale::{
    Scheduler, TableScheduler, Tick, Timescale, TimescaleKind, TimescaleScheduler,
};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.runtime";

/// The ten stages of the closed loop, in order.
///
/// The order is the loop's contract, not a convention: [`LoopStage::idx`] is the `idx`
/// column a stage's idempotency key is built from, so re-ordering the variants without
/// re-ordering the run would silently change which stage a resumed `(run_id, idx)`
/// names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopStage {
    /// Build the episode from the goal and the current capability graph.
    Experience,
    /// Commit the episode and the run's inputs as events.
    EventLog,
    /// Run the Phase 11 trigger/diagnosis pass on the episode.
    MetaAnalysis,
    /// Derive the gap: capability, data, policy, prompt or code.
    CapabilityGap,
    /// Assemble a typed change set from the gap.
    ChangeSet,
    /// Sandbox the candidate and record the Pi session that authored it.
    SelfEngineering,
    /// Run the regression suite and the frozen benchmark.
    TestBenchmark,
    /// Compare the candidate with the baseline it claims not to change.
    Shadow,
    /// Decide, deterministically, with a written reason.
    PromotionGate,
    /// Register the version and hot-load it.
    NewVersion,
}

impl LoopStage {
    /// Every stage, in run order.
    pub const ALL: [LoopStage; 10] = [
        LoopStage::Experience,
        LoopStage::EventLog,
        LoopStage::MetaAnalysis,
        LoopStage::CapabilityGap,
        LoopStage::ChangeSet,
        LoopStage::SelfEngineering,
        LoopStage::TestBenchmark,
        LoopStage::Shadow,
        LoopStage::PromotionGate,
        LoopStage::NewVersion,
    ];

    /// Its position in the run, which is also its idempotency key's `idx`.
    pub fn idx(self) -> usize {
        LoopStage::ALL
            .iter()
            .position(|stage| *stage == self)
            .expect("every stage is in ALL")
    }

    /// The stable wire name, spelled the way `modules/cognition/closed-loop` spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            LoopStage::Experience => "experience",
            LoopStage::EventLog => "event_log",
            LoopStage::MetaAnalysis => "meta_analysis",
            LoopStage::CapabilityGap => "capability_gap",
            LoopStage::ChangeSet => "change_set",
            LoopStage::SelfEngineering => "self_engineering",
            LoopStage::TestBenchmark => "test_benchmark",
            LoopStage::Shadow => "shadow",
            LoopStage::PromotionGate => "promotion_gate",
            LoopStage::NewVersion => "new_version",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<LoopStage> {
        LoopStage::ALL
            .into_iter()
            .find(|stage| stage.as_str() == text.trim())
    }

    /// The stage at a position, or nothing when the position is off the end.
    pub fn at(index: usize) -> Option<LoopStage> {
        LoopStage::ALL.get(index).copied()
    }
}

impl std::fmt::Display for LoopStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The goal a run is given.
///
/// The goal names the *identity* of the capability it wants — the module IRI, the path,
/// the capability and the T-Box function — because the loop's job is to produce a
/// specific capability, and a loop that discovered its own target would be one whose
/// result could not be predetermined. Everything else about the goal is prose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoopGoal {
    /// Its ULID, as a string.
    pub id: String,
    /// What the system is being asked to be able to do.
    pub description: String,
    /// True when no existing capability answers this goal.
    #[serde(default)]
    pub novel: bool,
    /// What counts as success, as statements a reader can check.
    #[serde(default)]
    pub success_criteria: Vec<String>,
    /// The module's directory name.
    #[serde(default)]
    pub module_name: String,
    /// The path the capability lives at, repository-relative.
    #[serde(default)]
    pub target_path: String,
    /// The module IRI.
    #[serde(default)]
    pub target_uri: String,
    /// The capability the module declares, e.g. `mm:GoalAttainment`.
    #[serde(default)]
    pub capability: String,
    /// The T-Box function the module must expose.
    #[serde(default)]
    pub function: String,
    /// The design document the revision is of.
    #[serde(default)]
    pub design_doc: String,
    /// The episode fixture that motivates the goal.
    #[serde(default)]
    pub episode: Option<String>,
    /// The Pi task contract that authors the module.
    #[serde(default)]
    pub pi_task: Option<String>,
    /// The benchmark spec the candidate is judged against.
    #[serde(default)]
    pub benchmark: Option<String>,
    /// The recorded Pi session, when the goal names one directly.
    #[serde(default)]
    pub session_file: Option<String>,
}

impl LoopGoal {
    /// Read a goal fixture.
    pub fn from_file(path: &Path) -> Result<LoopGoal> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            LoopError::validation("goal_file", format!("cannot read {}: {e}", path.display()))
        })?;
        let goal: LoopGoal = serde_json::from_str(&text).map_err(|e| {
            LoopError::validation(
                "goal_file",
                format!("{} is not a goal: {e}", path.display()),
            )
        })?;
        goal.validate()?;
        Ok(goal)
    }

    /// Refuse a goal that names nothing, or names only some of the capability's identity.
    pub fn validate(&self) -> Result<()> {
        if self.description.trim().is_empty() {
            return Err(LoopError::validation("description", "must not be empty"));
        }
        if self.target_uri.trim().is_empty() {
            return Err(LoopError::validation("target_uri", "must not be empty"));
        }
        if self.target_path.trim().is_empty() {
            return Err(LoopError::validation("target_path", "must not be empty"));
        }
        if self.capability.trim().is_empty() {
            return Err(LoopError::validation("capability", "must not be empty"));
        }
        if self.function.trim().is_empty() {
            return Err(LoopError::validation("function", "must not be empty"));
        }
        if self.module_name.trim().is_empty() {
            return Err(LoopError::validation("module_name", "must not be empty"));
        }
        if self.success_criteria.is_empty() {
            return Err(LoopError::validation(
                "success_criteria",
                "a goal with no success criterion cannot be judged",
            ));
        }
        Ok(())
    }

    /// The goal's ULID, or a refusal when it is not one.
    pub fn goal_ulid(&self) -> Result<Ulid> {
        mm_core::id::parse_ulid(&self.id)
            .map_err(|e| LoopError::validation("id", format!("{} is not a ULID: {e}", self.id)))
    }

    /// The module version this phase produces. One version per phase, stated here rather
    /// than derived, so a promotion's `(uri, version)` pair is predetermined.
    pub fn module_version(&self) -> &'static str {
        "0.1.0"
    }
}

/// The four budgets plus a wall-clock ceiling.
///
/// The numbers are the four Phase 11 budget *kinds*, and this envelope is the run's own
/// hard cap over them: a stage whose debit would cross a bucket is denied whole. The
/// envelope is deliberately not the `budgets` table: that table is what the system may
/// spend over a period, and this is what one run may spend — a run that quietly drained
/// the period's budget would make "the evolution budget is exhausted" a surprise.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetEnvelope {
    /// Meta-analysis spend, in abstract units.
    pub meta_analysis: f64,
    /// Improvement spend (change sets, sandboxes, Pi authoring).
    pub improvement: f64,
    /// Evolution spend (promotions).
    pub evolution: f64,
    /// Metacognitive spend (scheduler ticks and self-measurement).
    pub metacognitive: f64,
    /// The wall-clock ceiling in milliseconds.
    pub wall_ms: u64,
}

impl Default for BudgetEnvelope {
    fn default() -> Self {
        BudgetEnvelope {
            meta_analysis: 200.0,
            improvement: 50.0,
            evolution: 20.0,
            metacognitive: 500.0,
            wall_ms: 600_000,
        }
    }
}

/// The four buckets, named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetBucket {
    /// `meta_analysis`.
    MetaAnalysis,
    /// `improvement`.
    Improvement,
    /// `evolution`.
    Evolution,
    /// `metacognitive`.
    Metacognitive,
}

impl BudgetBucket {
    /// The stable name, which is also the column's name.
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetBucket::MetaAnalysis => "meta_analysis",
            BudgetBucket::Improvement => "improvement",
            BudgetBucket::Evolution => "evolution",
            BudgetBucket::Metacognitive => "metacognitive",
        }
    }
}

impl BudgetEnvelope {
    /// A zero envelope: every debit is refused.
    pub fn zero() -> Self {
        BudgetEnvelope {
            meta_analysis: 0.0,
            improvement: 0.0,
            evolution: 0.0,
            metacognitive: 0.0,
            wall_ms: 0,
        }
    }

    /// Read an envelope from a TOML document.
    pub fn parse(text: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Raw {
            meta_analysis: f64,
            improvement: f64,
            evolution: f64,
            metacognitive: f64,
            wall_ms: u64,
        }
        let raw: Raw = toml::from_str(text)
            .map_err(|e| LoopError::validation("budget", format!("not an envelope: {e}")))?;
        let envelope = BudgetEnvelope {
            meta_analysis: raw.meta_analysis,
            improvement: raw.improvement,
            evolution: raw.evolution,
            metacognitive: raw.metacognitive,
            wall_ms: raw.wall_ms,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    /// Read an envelope from a file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            LoopError::validation(
                "budget_file",
                format!("cannot read {}: {e}", path.display()),
            )
        })?;
        BudgetEnvelope::parse(&text)
    }

    /// Refuse a negative cap. A budget that can be negative is not a cap.
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("meta_analysis", self.meta_analysis),
            ("improvement", self.improvement),
            ("evolution", self.evolution),
            ("metacognitive", self.metacognitive),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(LoopError::validation(
                    "budget",
                    format!("{name} must be finite and not negative, got {value}"),
                ));
            }
        }
        Ok(())
    }

    /// What is left in one bucket.
    ///
    /// A `BudgetEnvelope` always means *remaining* amounts: the value a run is given is
    /// its cap, and every debit subtracts from it in place, so a report that carries an
    /// envelope carries what is left rather than a second number to reconcile.
    pub fn remaining(&self, bucket: BudgetBucket) -> f64 {
        match bucket {
            BudgetBucket::MetaAnalysis => self.meta_analysis,
            BudgetBucket::Improvement => self.improvement,
            BudgetBucket::Evolution => self.evolution,
            BudgetBucket::Metacognitive => self.metacognitive,
        }
    }

    /// True when `amount` more of `bucket` would fit.
    ///
    /// The comparison is inclusive at the limit: a bucket whose remainder is exactly the
    /// debit still fits, and the next one is denied.
    pub fn may_debit(&self, bucket: BudgetBucket, amount: f64) -> bool {
        amount >= 0.0 && amount <= self.remaining(bucket) + f64::EPSILON
    }

    /// Spend `amount` of `bucket`, or refuse whole.
    ///
    /// Refusal leaves the envelope byte-identical: there is no clamping and no partial
    /// debit, because a budget that "spent what it could" reports a number nobody
    /// authorized.
    pub fn debit(&mut self, bucket: BudgetBucket, amount: f64) -> Result<()> {
        if !amount.is_finite() || amount < 0.0 {
            return Err(LoopError::validation(
                "budget",
                format!("a debit must be finite and not negative, got {amount}"),
            ));
        }
        if !self.may_debit(bucket, amount) {
            return Err(LoopError::BudgetDenied {
                bucket: bucket.as_str().to_string(),
                needed: amount,
                remaining: self.remaining(bucket),
            });
        }
        match bucket {
            BudgetBucket::MetaAnalysis => self.meta_analysis -= amount,
            BudgetBucket::Improvement => self.improvement -= amount,
            BudgetBucket::Evolution => self.evolution -= amount,
            BudgetBucket::Metacognitive => self.metacognitive -= amount,
        }
        Ok(())
    }

    /// The envelope's own rendering of how much of the *original* cap each bucket has
    /// left, which is what an operator reads.
    pub fn remaining_fraction(&self, bucket: BudgetBucket, origin: &BudgetEnvelope) -> f64 {
        let cap = origin.remaining(bucket);
        if cap <= f64::EPSILON {
            1.0
        } else {
            (self.remaining(bucket) / cap).clamp(0.0, 1.0)
        }
    }

    /// Whether the envelope still holds anything at all.
    pub fn is_exhausted(&self) -> bool {
        self.meta_analysis <= f64::EPSILON
            && self.improvement <= f64::EPSILON
            && self.evolution <= f64::EPSILON
            && self.metacognitive <= f64::EPSILON
    }

    /// The canonical rendering the events and the report carry.
    pub fn canonical(&self) -> String {
        format!(
            "meta_analysis={:.6},improvement={:.6},evolution={:.6},metacognitive={:.6},wall_ms={}",
            self.meta_analysis, self.improvement, self.evolution, self.metacognitive, self.wall_ms
        )
    }
}

/// What a stage produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageOutcome {
    /// The stage's position in the run.
    pub idx: usize,
    /// The stage.
    pub stage: LoopStage,
    /// `ok`, `refused` or `failed`.
    pub outcome: String,
    /// What it produced, or why it refused.
    pub detail: String,
    /// The artifact it registered, when it registered one.
    pub artifact: String,
    /// How long it took. Recorded, never a budget input: the envelope's `wall_ms` is
    /// checked against the wall clock, and a stage's own latency is what an operator
    /// reads when a run is slow.
    pub latency_ms: u64,
}

impl StageOutcome {
    /// True when the stage completed.
    pub fn is_ok(&self) -> bool {
        self.outcome == "ok"
    }
}

/// One artifact a stage registered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// The kind: `episode`, `analysis`, `gap`, `changeset`, `sandbox`, `benchmark`,
    /// `shadow`, `promotion`, `module`, `revision`.
    pub kind: String,
    /// Its IRI, when it has one.
    pub uri: String,
    /// Its path, when it has one.
    pub path: String,
    /// One line about it.
    pub detail: String,
}

/// The run's state: a projection of the event log, never an authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoopState {
    /// The run's ULID.
    #[serde(with = "mm_core::serde_ulid")]
    pub run_id: Ulid,
    /// The stage currently running.
    pub stage: LoopStage,
    /// How many stages have completed.
    pub iteration: u32,
    /// What has been produced so far.
    pub artifacts: Vec<ArtifactRef>,
    /// What is left of the envelope.
    pub budget_used: BudgetEnvelope,
}

/// The final report of a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoopReport {
    /// The run's ULID.
    #[serde(with = "mm_core::serde_ulid")]
    pub run_id: Ulid,
    /// Every stage's outcome, in order.
    pub stages: Vec<StageOutcome>,
    /// The promotion the run produced, when it produced one.
    pub promoted: Option<PromotionId>,
    /// The envelope as it stood when the run ended (the *remaining* caps).
    pub budget_used: BudgetEnvelope,
    /// The fold of the run's events, which `loop replay` must reproduce.
    pub digest: String,
}

impl LoopReport {
    /// True when every stage completed.
    pub fn completed(&self) -> bool {
        self.stages.len() == LoopStage::ALL.len() && self.stages.iter().all(StageOutcome::is_ok)
    }
}

/// A promotion row's identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PromotionId(#[serde(with = "mm_core::serde_ulid")] pub Ulid);

impl std::fmt::Display for PromotionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&mm_core::ulid_string(&self.0))
    }
}

/// Where a run may write.
///
/// The three roots are separate on purpose. `root` is the repository the run reads
/// fixtures and the frozen benchmark from. `sandbox_dir` is where a candidate is
/// materialised *before* it is judged, and `artifacts_dir` is where a **promoted**
/// capability is materialised *after* it is judged — both under kernel state, never in
/// the source tree. The reason is the phase's invariant 5: a run must not be able to
/// mutate the repository it is running in, because Phase 11's `audit production-tree`
/// asserts exactly that, and a loop that wrote a module into `modules/` would make the
/// audit's exclusion list the thing that hides it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePaths {
    /// The repository root: fixtures, the regression suite, the frozen benchmark.
    pub root: PathBuf,
    /// Where a candidate is materialised before judgment.
    pub sandbox_dir: PathBuf,
    /// Where a promoted capability and a revised design document are materialised.
    pub artifacts_dir: PathBuf,
}

impl RuntimePaths {
    /// The default paths for a repository root and a data directory.
    pub fn under(root: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let data_dir = data_dir.into();
        RuntimePaths {
            root,
            sandbox_dir: data_dir.join("sandbox"),
            artifacts_dir: data_dir.join("artifacts"),
        }
    }

    /// Resolve a repository-relative path against the root.
    pub fn repo_path(&self, relative: impl AsRef<Path>) -> PathBuf {
        let path = relative.as_ref();
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    /// Where one run's candidate is materialised.
    pub fn run_sandbox(&self, run_id: &Ulid) -> PathBuf {
        self.sandbox_dir
            .join("loop")
            .join(mm_core::ulid_string(run_id))
    }
}

/// The ten-stage loop.
///
/// Two entry points, and the difference between them is the phase's resume story:
/// [`ClosedLoop::run`] drives all ten stages from the beginning, and [`ClosedLoop::step`]
/// runs exactly one stage against a state — which is what a crashed run resumes with,
/// starting at the last `(run_id, idx)` whose row is committed.
#[async_trait]
pub trait ClosedLoop {
    /// Run the whole loop for a goal.
    async fn run(&self, goal: LoopGoal, budget: BudgetEnvelope) -> Result<LoopReport>;

    /// Run one stage.
    async fn step(&self, state: &mut LoopState, stage: LoopStage) -> Result<StageOutcome>;
}

/// A wall-clock measurement, so a stage's latency is a number the run recorded.
pub(crate) fn elapsed_ms(started: std::time::Instant) -> u64 {
    let millis = started.elapsed().as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}
