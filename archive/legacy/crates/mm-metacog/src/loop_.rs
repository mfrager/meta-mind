//! The metacognitive loop: minimum sufficient cognition.
//!
//! `scan → tier → compile → investigate → decide → trace`. The loop is where the
//! phase's first invariant is actually enforced: an operation runs only when its
//! value is positive, the tier's forcer permits it, and the budget can pay for it —
//! and the denominator of "enough" is remaining uncertainty rather than a step
//! count.
//!
//! It runs in two explicit phases, and the split is the whole argument:
//!
//! * **Investigation.** Select the highest-value ready operation, apply the
//!   stopping conditions, force, debit, execute, update the provisional model.
//!   This is what a budget and a stopping condition bound, because this is the
//!   part that can be overdone.
//! * **The decision, and the action it authorises.** An episode that stops before
//!   deciding has spent cognition and produced nothing, so the terminal operations
//!   are not subject to the uncertainty or value stops. They are still subject to
//!   the budget: a decision that cannot be afforded is refused, and that refusal is
//!   a stop cause like any other.
//!
//! Three further decisions are worth stating because they are not visible from the
//! shape of the code:
//!
//! * **The provisional model is shared state, not a graph edge.** The decision
//!   depends on the recall that initialised the model; each investigation operation
//!   updates it as a side effect. A hard edge from the decision to every
//!   investigation operation would say the decision *cannot* be taken until all of
//!   them run, which is exactly the over-thinking the stopping conditions exist to
//!   prevent.
//! * **Investigation is ordered by value, the decision by position.** A free
//!   operation has a maximal value per unit of cost, so reflection runs first; the
//!   terminal operations run last, in `steps` order.
//! * **Execution here is simulated.** The phase produces the program and the trace;
//!   the organs that carry an operation out belong to later phases (9 and 10 for
//!   firewall and tool execution, 12 for the closed loop). The trace records what
//!   the controller would have run, in order, with the score and the budget after
//!   each step, which is exactly what a caller needs to reproduce it.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use mm_log::{codes, Level, LogRecord, Logger};

use crate::budget::{BudgetDebit, BudgetForcer, CognitiveBudget, StopCause};
use crate::episode::{CognitiveEpisode, EpisodeId};
use crate::error::{MetacogError, Result};
use crate::graph::NodeId;
use crate::lower;
use crate::op::OpClass;
use crate::program::{
    evidence_weight, uncertainty_target, CognitiveProgram, DefaultCompiler, ProgramCompiler,
};
use crate::scan::{ScanDriver, ScanRequest, ScanResult};
use crate::tier::{uncertainty_factor, Tier};
use crate::trace::{ProgramTrace, TraceOutcome, TraceRow};
use crate::value::select_next;

/// What a completed deliberation produced.
#[derive(Clone, Debug)]
pub struct EpisodeOutcome {
    /// The episode as it ran, with its selected tier and its spent budget. Kept
    /// whole because a caller that only had the id could not persist the run.
    pub episode: CognitiveEpisode,
    /// The episode's identifier.
    pub episode_id: EpisodeId,
    /// The tier it ran at.
    pub tier: Tier,
    /// The scan it started from.
    pub scan: ScanResult,
    /// The program it compiled.
    pub program: CognitiveProgram,
    /// The ordered trace of what it considered and what it ran.
    pub trace: ProgramTrace,
    /// Why it stopped.
    pub stop: StopCause,
    /// What it spent.
    pub budget: CognitiveBudget,
    /// How many operations actually ran.
    pub executed: usize,
    /// The remaining uncertainty when it stopped.
    pub remaining_uncertainty: f64,
}

impl EpisodeOutcome {
    /// True when the loop stopped before spending a budget dimension.
    pub fn stopped_within_budget(&self) -> bool {
        self.stop != StopCause::BudgetExhausted
    }

    /// The number of operations the trace recorded but did not run.
    pub fn skipped(&self) -> usize {
        self.trace.skipped() + self.trace.stopped()
    }
}

/// The executive.
#[async_trait]
pub trait MetacognitiveController: Send + Sync {
    /// Run one deliberation to a stop.
    ///
    /// The context is the episode's own field rather than a second argument: a
    /// caller that passed a different one could make the scan reason about a
    /// situation the episode's own numbers describe differently, and there is no
    /// way to tell which one the program was compiled for.
    async fn run(&self, episode: CognitiveEpisode) -> Result<EpisodeOutcome>;
}

/// The live state of one deliberation.
///
/// A struct rather than a pile of locals because the per-operation step is shared
/// between the two phases, and a function that took every one of these as an
/// argument would be a function nobody could read.
struct RunState<'a> {
    episode_id: EpisodeId,
    program: &'a CognitiveProgram,
    dag_hash: &'a str,
    trace_id: mm_core::Ulid,
    trace: ProgramTrace,
    seq: u32,
    budget: CognitiveBudget,
    executed: BTreeSet<NodeId>,
    remaining: f64,
}

/// The result of attempting one operation.
enum Ran {
    /// It ran.
    Done,
    /// The budget refused it.
    Stopped(StopCause),
}

/// The controller: scan once, select a policy, compile, then spend the least
/// cognition that can change the decision.
pub struct MinimumCognitionController {
    driver: Arc<dyn ScanDriver>,
    compiler: Arc<dyn ProgramCompiler>,
    forcer: BudgetForcer,
    logger: Option<Arc<Logger>>,
}

impl MinimumCognitionController {
    /// A controller that scans with `driver` and compiles with the default
    /// compiler.
    pub fn new(driver: Arc<dyn ScanDriver>) -> Self {
        MinimumCognitionController {
            driver,
            compiler: Arc::new(DefaultCompiler::new()),
            forcer: BudgetForcer::default(),
            logger: None,
        }
    }

    /// Compile with something other than the default compiler.
    pub fn with_compiler(mut self, compiler: Arc<dyn ProgramCompiler>) -> Self {
        self.compiler = compiler;
        self
    }

    /// Hold a different fraction of the budget in reserve.
    pub fn with_forcer(mut self, forcer: BudgetForcer) -> Self {
        self.forcer = forcer;
        self
    }

    /// Record the deliberation to `logger`.
    ///
    /// The audited records (`metacog.op.execute`, `metacog.trace.persist`) need an
    /// audit writer, so a logger configured without one will refuse them; a
    /// controller without a logger records nothing and is what the tests use.
    pub fn with_logger(mut self, logger: Arc<Logger>) -> Self {
        self.logger = Some(logger);
        self
    }

    /// The scan driver.
    pub fn driver(&self) -> &Arc<dyn ScanDriver> {
        &self.driver
    }

    /// Emit a record when a logger is configured. A record below the configured
    /// level is silently dropped by the logger itself.
    async fn emit(&self, record: LogRecord) -> Result<()> {
        let Some(logger) = &self.logger else {
            return Ok(());
        };
        logger.emit(record).await.map_err(MetacogError::from)
    }

    /// Emit an audited record when a logger is configured.
    async fn audit(
        &self,
        code: &str,
        fields: serde_json::Value,
        trace: mm_core::Ulid,
    ) -> Result<()> {
        let Some(logger) = &self.logger else {
            return Ok(());
        };
        logger
            .audit(Level::Info, code, crate::TARGET, Some(trace), fields)
            .await
            .map_err(MetacogError::from)
    }

    /// Attempt one operation: debit for it, record it, and update the provisional
    /// model.
    ///
    /// The debit happens *before* the operation is recorded, so a spend that
    /// cannot be paid for never appears in the trace as something that ran. The
    /// model update is fixed arithmetic over the operation's own expected error
    /// reduction and its class's evidence weight — no model call, no clock — so a
    /// replay reaches the same number.
    async fn run_node(&self, state: &mut RunState<'_>, node: NodeId) -> Result<Ran> {
        let node_ref = state
            .program
            .graph
            .node(node)
            .ok_or(MetacogError::NoSuchNode(node))?
            .clone();

        if state.budget.debit(BudgetDebit::ops(1)).is_err() {
            self.emit(
                LogRecord::new(Level::Warn, codes::METACOG_BUDGET_EXHAUSTED, crate::TARGET)
                    .with_trace(state.trace_id)
                    .with_field("episode_id", mm_core::ulid_string(&state.episode_id))
                    .with_field("resource", "ops"),
            )
            .await?;
            return Ok(Ran::Stopped(StopCause::BudgetExhausted));
        }
        if node_ref.value.cost > 0.0
            && state
                .budget
                .debit(BudgetDebit::cost(node_ref.value.cost))
                .is_err()
        {
            self.emit(
                LogRecord::new(Level::Warn, codes::METACOG_BUDGET_EXHAUSTED, crate::TARGET)
                    .with_trace(state.trace_id)
                    .with_field("episode_id", mm_core::ulid_string(&state.episode_id))
                    .with_field("resource", "cost"),
            )
            .await?;
            return Ok(Ran::Stopped(StopCause::BudgetExhausted));
        }
        self.emit(
            LogRecord::new(Level::Debug, codes::METACOG_BUDGET_DEBIT, crate::TARGET)
                .with_trace(state.trace_id)
                .with_field("episode_id", mm_core::ulid_string(&state.episode_id))
                .with_field("resource", "cost")
                .with_field("amount", node_ref.value.cost)
                .with_field(
                    "remaining",
                    state.budget.remaining(crate::budget::BudgetResource::Cost),
                ),
        )
        .await?;

        self.audit(
            codes::METACOG_OP_EXECUTE,
            serde_json::json!({
                "program_id": mm_core::ulid_string(&state.program.id),
                "seq": state.seq,
                "node_id": node,
                "op": node_ref.op.tag(),
                "op_class": node_ref.op.class().as_str(),
                "dag_hash": state.dag_hash,
            }),
            state.trace_id,
        )
        .await?;

        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_OP_SELECT, crate::TARGET)
                .with_trace(state.trace_id)
                .with_field("program_id", mm_core::ulid_string(&state.program.id))
                .with_field("seq", state.seq)
                .with_field("node_id", node)
                .with_field("op", node_ref.op.tag())
                .with_field("score", node_ref.value.score())
                .with_field("tie_break", "node_id"),
        )
        .await?;

        state.trace.push(TraceRow {
            seq: state.seq,
            node_id: node,
            op: node_ref.op.tag().to_string(),
            op_class: node_ref.op.class(),
            selected: true,
            value: node_ref.value,
            outcome: TraceOutcome::Executed,
            budget_after: state.budget,
            stopping_reason: None,
        })?;
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_OP_OUTCOME, crate::TARGET)
                .with_trace(state.trace_id)
                .with_field("program_id", mm_core::ulid_string(&state.program.id))
                .with_field("seq", state.seq)
                .with_field("outcome", "executed")
                .with_field("cost", node_ref.value.cost),
        )
        .await?;
        state.seq += 1;
        state.executed.insert(node);
        state.remaining = (state.remaining
            * (1.0
                - 0.5
                    * node_ref.value.expected_error_reduction
                    * evidence_weight(node_ref.op.class())))
        .clamp(0.0, 1.0);
        Ok(Ran::Done)
    }
}

/// True for the operations that end a deliberation.
fn is_terminal(class: OpClass) -> bool {
    matches!(class, OpClass::Decide | OpClass::Act)
}

#[async_trait]
impl MetacognitiveController for MinimumCognitionController {
    async fn run(&self, mut episode: CognitiveEpisode) -> Result<EpisodeOutcome> {
        episode.validate()?;
        let trace_id = episode.id;

        // ---------------------------------------------------------------- open ----
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_EPISODE_OPEN, crate::TARGET)
                .with_trace(trace_id)
                .with_field("episode_id", mm_core::ulid_string(&episode.id))
                .with_field("goal", mm_core::ulid_string(&episode.goal))
                .with_field("timescale", episode.timescale.as_str())
                .with_field("budget_json", serde_json::to_value(episode.budget)?),
        )
        .await?;

        // ---------------------------------------------------------------- scan ----
        let request = ScanRequest::new(
            episode.id,
            episode.goal_text().to_string(),
            episode.context.clone(),
        );
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_SCAN_BEGIN, crate::TARGET)
                .with_trace(trace_id)
                .with_field("episode_id", mm_core::ulid_string(&episode.id))
                .with_field(
                    "prompt_hash",
                    mm_core::content_hash(request.prompt().as_bytes()),
                ),
        )
        .await?;
        let scan = self.driver.scan(&request).await?;
        scan.validate()?;
        for issue in &scan.issues {
            self.emit(
                LogRecord::new(Level::Info, codes::METACOG_SCAN_ISSUE, crate::TARGET)
                    .with_trace(trace_id)
                    .with_field("issue_kind", issue.kind.clone())
                    .with_field("materiality", issue.materiality)
                    .with_field("target", issue.target.clone()),
            )
            .await?;
        }
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_SCAN_END, crate::TARGET)
                .with_trace(trace_id)
                .with_field("issue_count", scan.issues.len())
                .with_field("stakes", scan.stakes)
                .with_field("irreversibility", scan.irreversibility)
                .with_field("verification_value", scan.verification_value),
        )
        .await?;

        // ---------------------------------------------------------------- tier ----
        let tier = episode.select_tier(&scan);
        let policy = tier.policy();
        policy.validate()?;
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_TIER_SELECT, crate::TARGET)
                .with_trace(trace_id)
                .with_field("episode_id", mm_core::ulid_string(&episode.id))
                .with_field("tier", tier.as_u8())
                .with_field("policy", policy.canonical()),
        )
        .await?;

        // ------------------------------------------------------------- compile ----
        let program = self.compiler.compile(&episode, &scan, &episode.budget)?;
        let catalog = lower::ToolCatalog::from_program(&program);
        let document = lower::to_execution_document(&program, &catalog)?;
        let dag_hash = document.dag_hash.clone();
        let dag_node_count = document.dag.node_count();
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_PROGRAM_COMPILE, crate::TARGET)
                .with_trace(trace_id)
                .with_field("program_id", mm_core::ulid_string(&program.id))
                .with_field("episode_id", mm_core::ulid_string(&episode.id))
                .with_field("node_count", program.node_count())
                .with_field("edge_count", program.edge_count())
                .with_field("dag_hash", dag_hash.clone())
                .with_field("required_tools", program.required_tools.clone()),
        )
        .await?;
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_PROGRAM_LOWER, crate::TARGET)
                .with_trace(trace_id)
                .with_field("program_id", mm_core::ulid_string(&program.id))
                .with_field("dag_hash", dag_hash.clone())
                .with_field("node_count", dag_node_count),
        )
        .await?;

        // ----------------------------------------------------------------- run ----
        let mut state = RunState {
            episode_id: episode.id,
            program: &program,
            dag_hash: &dag_hash,
            trace_id,
            trace: ProgramTrace::new(program.id),
            seq: 0,
            budget: episode.budget,
            executed: BTreeSet::new(),
            remaining: uncertainty_factor(&episode, &scan),
        };
        let target = uncertainty_target(tier);

        // ------------------------------------------------- phase 1: investigate ----
        let mut stop = loop {
            if state.budget.exhausted().is_some() {
                self.emit(
                    LogRecord::new(Level::Warn, codes::METACOG_BUDGET_EXHAUSTED, crate::TARGET)
                        .with_trace(trace_id)
                        .with_field("episode_id", mm_core::ulid_string(&episode.id))
                        .with_field("resource", "any"),
                )
                .await?;
                break StopCause::BudgetExhausted;
            }
            let ready = state.program.graph.ready(&state.executed);
            let investigative: Vec<NodeId> = ready
                .iter()
                .copied()
                .filter(|id| {
                    state
                        .program
                        .graph
                        .node(*id)
                        .is_some_and(|node| !is_terminal(node.op.class()))
                })
                .collect();
            if investigative.is_empty() {
                break StopCause::NoReadyOperation;
            }
            let Some(node) = select_next(&state.program.graph, &investigative) else {
                break StopCause::ExpectedValueBelow;
            };
            let node_ref = state
                .program
                .graph
                .node(node)
                .ok_or(MetacogError::NoSuchNode(node))?
                .clone();
            let score = node_ref.value.score();

            // The stopping conditions bound investigation, and only once something
            // has actually been investigated: the recall that initialises the model
            // is never the last word.
            if !state.trace.rows.is_empty() && state.remaining <= target {
                break StopCause::RemainingUncertaintyBelow;
            }
            if score < crate::program::EXPECTED_VALUE_FLOOR {
                break StopCause::ExpectedValueBelow;
            }

            let decision = self.forcer.permit(score, &state.budget, policy);
            self.emit(
                LogRecord::new(Level::Debug, codes::METACOG_FORCE_DECIDE, crate::TARGET)
                    .with_trace(trace_id)
                    .with_field("episode_id", mm_core::ulid_string(&episode.id))
                    .with_field("decision", decision.as_str())
                    .with_field("score", score)
                    .with_field("headroom", self.forcer.headroom),
            )
            .await?;
            if !decision.allows() {
                break StopCause::ExpectedValueBelow;
            }

            self.emit(
                LogRecord::new(Level::Debug, codes::METACOG_OP_CONSIDER, crate::TARGET)
                    .with_trace(trace_id)
                    .with_field("program_id", mm_core::ulid_string(&program.id))
                    .with_field("node_id", node)
                    .with_field("op", node_ref.op.tag())
                    .with_field("op_class", node_ref.op.class().as_str())
                    .with_field("value", node_ref.value.canonical()),
            )
            .await?;

            match self.run_node(&mut state, node).await? {
                Ran::Done => {}
                Ran::Stopped(cause) => break cause,
            }
        };

        // --------------------------------------- phase 2: decide, then act ----
        let terminals: Vec<NodeId> = program
            .steps
            .iter()
            .map(|step| step.node)
            .filter(|id| {
                program
                    .graph
                    .node(*id)
                    .is_some_and(|node| is_terminal(node.op.class()))
            })
            .collect();
        for node in terminals {
            if state.executed.contains(&node) {
                continue;
            }
            if !state.program.graph.ready(&state.executed).contains(&node) {
                // An `Act` whose `Decide` the budget refused never becomes ready.
                continue;
            }
            match self.run_node(&mut state, node).await? {
                Ran::Done => {}
                Ran::Stopped(cause) => {
                    stop = cause;
                    break;
                }
            }
            let decided = state
                .program
                .graph
                .node(node)
                .is_some_and(|n| n.op.class() == OpClass::Decide);
            if decided
                && program
                    .stopping_condition("irreversible_action_reached")
                    .is_some()
            {
                stop = StopCause::IrreversibleActionReached;
                break;
            }
        }

        // Everything the loop declined to run is recorded too: "the controller
        // chose not to" is as much of the deliberation as what it did.
        let skipped_outcome = match stop {
            StopCause::ExpectedValueBelow | StopCause::RemainingUncertaintyBelow => {
                TraceOutcome::Skipped
            }
            _ => TraceOutcome::Stopped,
        };
        for step in &program.steps {
            if state.executed.contains(&step.node) {
                continue;
            }
            state.trace.push(TraceRow {
                seq: state.seq,
                node_id: step.node,
                op: step.op.tag().to_string(),
                op_class: step.op.class(),
                selected: false,
                value: step.value,
                outcome: skipped_outcome,
                budget_after: state.budget,
                stopping_reason: Some(stop.as_str().to_string()),
            })?;
            state.seq += 1;
        }
        state.trace.seal();
        state.trace.validate()?;

        // ---------------------------------------------------------------- stop ----
        let decision_relevant = state.remaining > target;
        self.emit(
            LogRecord::new(Level::Info, codes::METACOG_STOP, crate::TARGET)
                .with_trace(trace_id)
                .with_field("episode_id", mm_core::ulid_string(&episode.id))
                .with_field("stopping_reason", stop.as_str())
                .with_field("remaining_uncertainty", state.remaining)
                .with_field("decision_relevant", decision_relevant),
        )
        .await?;

        self.audit(
            codes::METACOG_TRACE_PERSIST,
            serde_json::json!({
                "program_id": mm_core::ulid_string(&program.id),
                "row_count": state.trace.rows.len(),
                "first_seq": state.trace.first_seq(),
                "last_seq": state.trace.last_seq(),
                "stop": stop.as_str(),
            }),
            trace_id,
        )
        .await?;

        let episode_id = episode.id;
        let executed = state.executed.len();
        let remaining = state.remaining;
        let budget = state.budget;
        let trace = state.trace;
        Ok(EpisodeOutcome {
            episode,
            episode_id,
            tier,
            scan,
            program,
            trace,
            stop,
            budget,
            executed,
            remaining_uncertainty: remaining,
        })
    }
}
