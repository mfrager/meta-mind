//! `mm-metacog` — the metacognitive controller: the system's executive.
//!
//! Design §11 / Phase 8 asks for an executive that (a) inspects a provisional
//! model of a problem, (b) decides *what kind* of cognition is needed and *how
//! much* is sufficient, and (c) compiles a temporary **cognitive program** — a
//! typed graph of cognitive operations — then lowers it to an executable
//! computation DAG. It is not a reasoning engine and not a checklist: it
//! allocates cognition, bounds it with a budget, records a replayable trace, and
//! stops the moment remaining uncertainty is no longer decision-relevant.
//!
//! The modules, in the order data flows through them:
//!
//! * [`scan`] — one broad structured pass over the provisional model.
//! * [`tier`] — stakes × uncertainty × irreversibility × novelty → a compute
//!   *policy*, not a fixed script.
//! * [`op`], [`graph`], [`value`] — the operation algebra, the graph of
//!   operations, and the deterministic value an operation is ranked by.
//! * [`program`] — the compiler, composing frames and techniques per episode.
//! * [`budget`] — the enforced budget and the s1-style forcer.
//! * [`loop_`] — minimum-sufficient cognition: select, force, execute, update,
//!   and stop.
//! * [`lower`] — the Graph of Operations lowered to a [`lower::ComputationDag`]
//!   with a stable `dag_hash`.
//! * [`trace`] — the ordered trace, persisted and replayed.
//! * [`rdf`] — the T-Box emission into `/epistemic`.
//!
//! Six invariants hold across the crate, and each is a test somewhere:
//!
//! 1. **Spend the least cognition that can change the decision.** No operation
//!    runs without a positive marginal [`value::OperationValue`].
//! 2. **One scan, many issues.** The scan is one structured call, never a set of
//!    per-check prompts.
//! 3. **Deterministic selection.** Costs and scores are fixed arithmetic; ties
//!    break by `(score, node id)`.
//! 4. **A program is a graph.** Steps carry explicit dependencies, so independent
//!    nodes may run in parallel.
//! 5. **Budget is enforced, never clamped.** An over-debit is refused whole and
//!    becomes a stop cause.
//! 6. **Every deliberation is replayable.** The same inputs and trace produce a
//!    byte-identical program and scores.
#![forbid(unsafe_code)]

pub mod budget;
pub mod episode;
pub mod error;
pub mod graph;
pub mod loop_;
pub mod lower;
pub mod op;
pub mod program;
pub mod rdf;
pub mod scan;
pub mod tier;
pub mod trace;
pub mod value;

pub use budget::{
    BudgetDebit, BudgetForcer, BudgetResource, CognitiveBudget, Decision, StopCause,
    BUDGET_RESOURCES,
};
pub use episode::{
    ActionId, AssumptionId, CandidateId, CaseId, ClaimId, CognitiveEpisode, ConstraintId, Context,
    CriterionId, DoctrineId, EpisodeId, EpisodeStatus, FrameId, GoalId, OutcomeId, PropositionId,
    RefClassId, ScenarioId, SourceId, TechniqueId, Timescale, ToolId,
};
pub use error::{BudgetExceeded, MetacogError, Result};
pub use graph::{NodeId, OpGraph, OpNode};
pub use loop_::{EpisodeOutcome, MetacognitiveController, MinimumCognitionController};
pub use lower::{dag_hash, to_execution_document, ComputationDag, DagNode, ExecutionDocument};
pub use op::{CognitiveOp, CostClass, OpClass, COST_TABLE, OP_CLASSES};
pub use program::{
    CognitiveProgram, DefaultCompiler, ProgramCompiler, ProgramStep, StoppingCondition,
};
pub use scan::{
    ComparisonSpec, FailureMode, LlmScanDriver, MockScanDriver, ScanDriver, ScanIssue, ScanRequest,
    ScanResult, Uncertainty,
};
pub use tier::{ComputePolicy, Tier, TIERS};
pub use trace::{ProgramTrace, TraceOutcome, TraceRow, TraceStore};
pub use value::{select_next, OperationValue};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.metacog";
