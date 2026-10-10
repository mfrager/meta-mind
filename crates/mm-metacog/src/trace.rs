//! The program trace: what the controller considered, what it ran, and what it
//! refused to run.
//!
//! The phase's invariant 6 is "every deliberation is replayable", and a trace is
//! what makes that checkable. It records a row for *every* node the program holds
//! — `selected = 1` for the operations that ran and `selected = 0` for the ones
//! the loop declined — because "the controller chose not to" is as much of the
//! deliberation as what it did. A trace that recorded only executions could not
//! distinguish "there was nothing else worth doing" from "the budget ran out with
//! three operations still on the table", and those are different facts about a
//! decision.
//!
//! Two properties the persistence rests on:
//!
//! * **The trace's identity is its content.** `seal` derives the id from the
//!   canonical rendering, the same way a program derives its own, so a replay
//!   re-persists *the same trace* rather than an equivalent one.
//! * **Persisting is idempotent.** Every statement is `INSERT OR IGNORE`, and the
//!   ids are content-derived, so running the same episode twice leaves exactly one
//!   program and one trace. That is what lets `episode replay --all` run against a
//!   store it has already written to.
//!
//! Rows are appended and never rewritten: `push` refuses a sequence number that is
//! not the next one, so a trace cannot develop a gap or a reordering, which is the
//! one defect a replay comparison would not notice.

use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use serde::{Deserialize, Serialize};

use crate::budget::CognitiveBudget;
use crate::episode::{CognitiveEpisode, EpisodeId, EpisodeStatus, ProgramId, TraceId};
use crate::error::{MetacogError, Result};
use crate::graph::NodeId;
use crate::op::OpClass;
use crate::program::CognitiveProgram;
use crate::scan::ScanResult;
use crate::tier::Tier;
use crate::value::OperationValue;

/// How a traced operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceOutcome {
    /// It ran.
    Executed,
    /// It was considered and passed over: the decision was already settled enough.
    Skipped,
    /// It ran and failed.
    Failed,
    /// It was never reached: the budget or an irreversible action stopped the loop.
    Stopped,
}

/// Every outcome, in the order the plan lists them.
pub const TRACE_OUTCOMES: [TraceOutcome; 4] = [
    TraceOutcome::Executed,
    TraceOutcome::Skipped,
    TraceOutcome::Failed,
    TraceOutcome::Stopped,
];

impl TraceOutcome {
    /// Every outcome.
    pub const ALL: [TraceOutcome; 4] = TRACE_OUTCOMES;

    /// The stable wire name, matching the `program_traces.outcome` check
    /// constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            TraceOutcome::Executed => "executed",
            TraceOutcome::Skipped => "skipped",
            TraceOutcome::Failed => "failed",
            TraceOutcome::Stopped => "stopped",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        TRACE_OUTCOMES
            .into_iter()
            .find(|outcome| outcome.as_str() == text)
    }

    /// True when the operation was actually attempted.
    ///
    /// A failure was attempted and therefore ran; this is the predicate the
    /// `selected` column must agree with. [`ProgramTrace::executed`] counts
    /// successes only, which is a different question.
    pub fn ran(self) -> bool {
        matches!(self, TraceOutcome::Executed | TraceOutcome::Failed)
    }
}

impl std::fmt::Display for TraceOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One line of a deliberation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceRow {
    /// The row's position. Contiguous from 0.
    pub seq: u32,
    /// The program node this row is about.
    pub node_id: NodeId,
    /// The operation's tag.
    pub op: String,
    /// The operation's class.
    pub op_class: OpClass,
    /// Whether it ran.
    pub selected: bool,
    /// What it was worth, as it was scored at the moment it was considered.
    pub value: OperationValue,
    /// How it ended.
    pub outcome: TraceOutcome,
    /// The budget *after* the row, so an audit can see where the money went
    /// without replaying the arithmetic.
    pub budget_after: CognitiveBudget,
    /// Why the loop stopped, when this row is one it declined to run.
    pub stopping_reason: Option<String>,
}

impl TraceRow {
    /// A canonical rendering, used by the trace's content hash.
    ///
    /// Every number is rendered at a fixed width, so a score that differs in its
    /// ninth decimal is a different trace rather than the same one.
    pub fn canonical(&self) -> String {
        format!(
            "seq={},node={},op={},class={},selected={},outcome={},value=[{}],\
             budget=[{}],stop={}",
            self.seq,
            self.node_id,
            self.op,
            self.op_class.as_str(),
            u8::from(self.selected),
            self.outcome.as_str(),
            self.value.canonical(),
            budget_canonical(&self.budget_after),
            self.stopping_reason.as_deref().unwrap_or(""),
        )
    }
}

/// A fixed rendering of a budget, so a trace's hash cannot depend on float
/// formatting choices.
fn budget_canonical(budget: &CognitiveBudget) -> String {
    format!(
        "max_ops={},max_llm_calls={},max_cost={:.9},max_wall_ms={},\
         spent_ops={},spent_llm_calls={},spent_cost={:.9},spent_wall_ms={}",
        budget.max_ops,
        budget.max_llm_calls,
        budget.max_cost,
        budget.max_wall_ms,
        budget.spent_ops,
        budget.spent_llm_calls,
        budget.spent_cost,
        budget.spent_wall_ms,
    )
}

/// The ordered trace of one program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgramTrace {
    /// The trace's identifier, derived from its own content by [`Self::seal`].
    pub id: TraceId,
    /// The program it traces.
    pub program_id: ProgramId,
    /// The rows, in sequence order.
    pub rows: Vec<TraceRow>,
}

impl ProgramTrace {
    /// An empty, unsealed trace.
    pub fn new(program_id: ProgramId) -> Self {
        ProgramTrace {
            id: Ulid::nil(),
            program_id,
            rows: Vec::new(),
        }
    }

    /// Append a row, refusing anything that is not the next sequence number.
    ///
    /// A gap or a reordering is exactly the corruption a replay comparison would
    /// miss — the rows would still be present, just not the deliberation.
    pub fn push(&mut self, row: TraceRow) -> Result<()> {
        let expected = u32::try_from(self.rows.len()).map_err(|_| {
            MetacogError::Internal("a trace may hold at most 4294967295 rows".to_string())
        })?;
        if row.seq != expected {
            return Err(MetacogError::Internal(format!(
                "trace row {} was appended where {expected} was expected; \
                 rows are appended in order and never rewritten",
                row.seq
            )));
        }
        self.rows.push(row);
        Ok(())
    }

    /// Derive the trace's identifier from its content.
    ///
    /// Must be called after the last [`Self::push`]: a trace that is sealed early
    /// would carry an id describing rows it no longer matches.
    pub fn seal(&mut self) {
        self.id = crate::program::program_id_from(&self.canonical());
    }

    /// The row at this sequence number.
    pub fn row(&self, seq: u32) -> Option<&TraceRow> {
        self.rows.iter().find(|row| row.seq == seq)
    }

    /// How many operations succeeded.
    pub fn executed(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.outcome == TraceOutcome::Executed)
            .count()
    }

    /// How many operations were considered and passed over.
    pub fn skipped(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.outcome == TraceOutcome::Skipped)
            .count()
    }

    /// How many operations were never reached.
    pub fn stopped(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.outcome == TraceOutcome::Stopped)
            .count()
    }

    /// The first sequence number, when the trace has rows.
    pub fn first_seq(&self) -> Option<u32> {
        self.rows.first().map(|row| row.seq)
    }

    /// The last sequence number, when the trace has rows.
    pub fn last_seq(&self) -> Option<u32> {
        self.rows.last().map(|row| row.seq)
    }

    /// A canonical rendering of the whole trace.
    pub fn canonical(&self) -> String {
        let rows: Vec<String> = self.rows.iter().map(TraceRow::canonical).collect();
        format!(
            "program={}\nrows={}",
            mm_core::ulid_string(&self.program_id),
            rows.join("\n")
        )
    }

    /// The trace's content hash: sha256 over its canonical rendering.
    pub fn content_hash(&self) -> String {
        mm_core::content_hash(self.canonical().as_bytes())
    }

    /// Refuse a trace that cannot be replayed or compared.
    pub fn validate(&self) -> Result<()> {
        if self.program_id.is_nil() {
            return Err(MetacogError::validation(
                "trace.program_id",
                "must not be the nil ULID",
            ));
        }
        if self.rows.is_empty() {
            return Err(MetacogError::validation(
                "trace.rows",
                "a trace must hold at least one row; a deliberation with nothing \
                 in it did not happen",
            ));
        }
        if self.id.is_nil() {
            return Err(MetacogError::validation(
                "trace.id",
                "the trace has not been sealed; its id is what makes a replay \
                 comparison meaningful",
            ));
        }
        for (index, row) in self.rows.iter().enumerate() {
            let expected = u32::try_from(index).map_err(|_| {
                MetacogError::Internal("a trace may hold at most 4294967295 rows".to_string())
            })?;
            if row.seq != expected {
                return Err(MetacogError::validation(
                    "trace.rows",
                    format!(
                        "row {index} has seq {}; sequence numbers are contiguous from 0",
                        row.seq
                    ),
                ));
            }
            if row.selected != row.outcome.ran() {
                return Err(MetacogError::validation(
                    "trace.rows",
                    format!(
                        "row {index} says selected={} but outcome={}; the two must agree",
                        row.selected, row.outcome
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// What a persisted episode looks like without its full body.
///
/// The `episodes` table is an index, not an authority: it records the situation's
/// numbers (the tier, the three factors, the budget) so an audit can ask "which
/// episodes were hard" without decoding a program. Reconstructing the *episode*
/// is the caller's job, which is why this type is not a `CognitiveEpisode`.
#[derive(Clone, Debug, PartialEq)]
pub struct EpisodeRecord {
    /// The episode's ULID.
    pub id: EpisodeId,
    /// The goal's ULID.
    pub goal: String,
    /// The tier it ran at.
    pub tier: Tier,
    /// What was at stake, in `[0,1]`.
    pub stakes: f64,
    /// How uncertain the situation was, in `[0,1]`.
    pub uncertainty: f64,
    /// How hard the decision is to undo, in `[0,1]`.
    pub reversibility: f64,
    /// Where the episode is in its life.
    pub status: EpisodeStatus,
    /// The budget it was granted.
    pub budget: CognitiveBudget,
    /// The log trace id, which is the episode's own ULID.
    pub trace_id: TraceId,
}

/// Reading and writing deliberations.
#[async_trait]
pub trait TraceStore: Send + Sync {
    /// Persist an episode, its program and its trace. Idempotent: the same
    /// deliberation written twice leaves exactly one row per object.
    async fn persist(
        &self,
        episode: &CognitiveEpisode,
        scan: &ScanResult,
        program: &CognitiveProgram,
        trace: &ProgramTrace,
    ) -> Result<()>;

    /// The episode's index row, when it exists.
    async fn load_episode(&self, id: EpisodeId) -> Result<Option<EpisodeRecord>>;

    /// A program by its own id.
    async fn load_program(&self, id: ProgramId) -> Result<Option<CognitiveProgram>>;

    /// A program's trace, in sequence order.
    async fn load_trace(&self, program_id: ProgramId) -> Result<ProgramTrace>;

    /// Every program compiled for an episode, oldest version first.
    async fn program_ids_for(&self, episode: EpisodeId) -> Result<Vec<ProgramId>>;
}

/// The `INSERT OR IGNORE` statements.
///
/// `OR IGNORE` rather than `OR REPLACE` on purpose: a conflicting row means the
/// same content-derived id was written twice, and the second write carries
/// nothing the first does not. `REPLACE` would delete and re-insert, which would
/// silently rewrite a program whose body a later version legitimately differs on.
const INSERT_EPISODE: &str = "INSERT OR IGNORE INTO episodes \
 (id, goal, tier, stakes, uncertainty, reversibility, status, budget_json, trace_id, \
  created_ulid, system_from, system_to, valid_from, valid_to) \
 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, NULL)";

const INSERT_PROGRAM: &str = "INSERT OR IGNORE INTO programs \
 (id, episode_id, version, tier, program_json, dag_hash, operation_value_json, created_ulid) \
 VALUES (?, ?, 1, ?, ?, ?, ?, ?)";

const INSERT_TRACE_ROW: &str = "INSERT OR IGNORE INTO program_traces \
 (id, program_id, seq, node_id, op, op_class, selected, value_json, outcome, \
  budget_after_json, stopping_reason, at_ulid) \
 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

const SELECT_EPISODE: &str =
    "SELECT id, goal, tier, stakes, uncertainty, reversibility, status, budget_json, trace_id \
     FROM episodes WHERE id = ?";

const SELECT_PROGRAM: &str = "SELECT program_json FROM programs WHERE id = ?";

const SELECT_PROGRAM_IDS: &str =
    "SELECT id FROM programs WHERE episode_id = ? ORDER BY version, created_ulid";

const SELECT_TRACE_ROWS: &str = "SELECT \
 id, program_id, seq, node_id, op, op_class, selected, value_json, outcome, \
 budget_after_json, stopping_reason, at_ulid \
 FROM program_traces WHERE program_id = ? ORDER BY seq";

const COUNT_PROGRAMS: &str = "SELECT count(*) AS n FROM programs WHERE id = ?";

/// The tabular trace store.
///
/// It programs against [`mm_core::Tabular`] rather than a SQLite handle, so the
/// crate does not depend on the store implementation and a test can hand it any
/// store that speaks the kernel's boundary.
#[derive(Clone)]
pub struct SqliteTraceStore {
    store: Arc<dyn Tabular>,
    ids: Arc<UlidFactory>,
}

impl SqliteTraceStore {
    /// Build a store over an already-open tabular store.
    pub fn new(store: Arc<dyn Tabular>, ids: Arc<UlidFactory>) -> Self {
        SqliteTraceStore { store, ids }
    }

    /// The identifier factory, so a caller can see the watermark in use.
    pub fn ids(&self) -> &Arc<UlidFactory> {
        &self.ids
    }

    async fn rows(&self, sql: &str, args: Params) -> Result<Vec<serde_json::Value>> {
        self.store
            .query_json(sql, args)
            .await
            .map_err(|e| MetacogError::Store(format!("{e} (statement: {sql})")))
    }

    async fn execute(&self, sql: &str, args: Params) -> Result<u64> {
        self.store
            .execute(sql, args)
            .await
            .map_err(|e| MetacogError::Store(format!("{e} (statement: {sql})")))
    }
}

#[async_trait]
impl TraceStore for SqliteTraceStore {
    async fn persist(
        &self,
        episode: &CognitiveEpisode,
        scan: &ScanResult,
        program: &CognitiveProgram,
        trace: &ProgramTrace,
    ) -> Result<()> {
        episode.validate()?;
        scan.validate()?;
        program.validate()?;
        trace.validate()?;
        if trace.program_id != program.id {
            return Err(MetacogError::validation(
                "trace.program_id",
                format!(
                    "the trace is for {} but the program is {}",
                    mm_core::ulid_string(&trace.program_id),
                    mm_core::ulid_string(&program.id)
                ),
            ));
        }

        let now = Timestamp::now().to_rfc3339();
        self.execute(
            INSERT_EPISODE,
            vec![
                Param::Text(mm_core::ulid_string(&episode.id)),
                Param::Text(mm_core::ulid_string(&episode.goal)),
                Param::Int(i64::from(episode.tier.as_u8())),
                Param::Real(scan.stakes),
                Param::Real(scan.mean_uncertainty()),
                Param::Real(scan.irreversibility),
                Param::Text(EpisodeStatus::Programmed.as_str().to_string()),
                Param::Text(serde_json::to_string(&episode.budget)?),
                Param::Text(mm_core::ulid_string(&episode.id)),
                Param::Text(self.ids.next_string()),
                Param::Text(now.clone()),
                Param::Text(now),
            ],
        )
        .await?;

        let operation_values: Vec<serde_json::Value> = program
            .steps
            .iter()
            .map(|step| {
                serde_json::json!({
                    "order": step.order,
                    "node_id": step.node,
                    "expected_error_reduction": step.value.expected_error_reduction,
                    "decision_importance": step.value.decision_importance,
                    "probability_change": step.value.probability_change,
                    "cost": step.value.cost,
                    "score": step.value.score(),
                })
            })
            .collect();
        let catalog = crate::lower::ToolCatalog::from_program(program);
        let dag_hash = crate::lower::dag_hash(program, &catalog);
        self.execute(
            INSERT_PROGRAM,
            vec![
                Param::Text(mm_core::ulid_string(&program.id)),
                Param::Text(mm_core::ulid_string(&episode.id)),
                Param::Int(i64::from(program.tier.as_u8())),
                Param::Text(serde_json::to_string(program)?),
                Param::Text(dag_hash),
                Param::Text(serde_json::to_string(&operation_values)?),
                Param::Text(self.ids.next_string()),
            ],
        )
        .await?;

        for row in &trace.rows {
            let value_json = serde_json::json!({
                "expected_error_reduction": row.value.expected_error_reduction,
                "decision_importance": row.value.decision_importance,
                "probability_change": row.value.probability_change,
                "cost": row.value.cost,
                "score": row.value.score(),
            });
            self.execute(
                INSERT_TRACE_ROW,
                vec![
                    Param::Text(self.ids.next_string()),
                    Param::Text(mm_core::ulid_string(&trace.program_id)),
                    Param::Int(i64::from(row.seq)),
                    Param::Int(i64::from(row.node_id)),
                    Param::Text(row.op.clone()),
                    Param::Text(row.op_class.as_str().to_string()),
                    Param::Int(i64::from(row.selected)),
                    Param::Text(serde_json::to_string(&value_json)?),
                    Param::Text(row.outcome.as_str().to_string()),
                    Param::Text(serde_json::to_string(&row.budget_after)?),
                    Param::from(row.stopping_reason.clone()),
                    Param::Text(self.ids.next_string()),
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn load_episode(&self, id: EpisodeId) -> Result<Option<EpisodeRecord>> {
        let rows = self
            .rows(SELECT_EPISODE, vec![Param::Text(mm_core::ulid_string(&id))])
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let tier_raw = int_column(row, "tier")?;
        let tier = u8::try_from(tier_raw)
            .ok()
            .and_then(Tier::from_u8)
            .ok_or_else(|| {
                MetacogError::Store(format!(
                    "episode {} has tier {tier_raw}, which is not a tier",
                    mm_core::ulid_string(&id)
                ))
            })?;
        let status_text = text_column(row, "status")?;
        let status = EpisodeStatus::parse(&status_text).ok_or_else(|| {
            MetacogError::Store(format!(
                "episode {} has status {status_text:?}",
                mm_core::ulid_string(&id)
            ))
        })?;
        let budget_text = text_column(row, "budget_json")?;
        let budget: CognitiveBudget = serde_json::from_str(&budget_text).map_err(|e| {
            MetacogError::Store(format!(
                "episode {} has an unreadable budget_json: {e}",
                mm_core::ulid_string(&id)
            ))
        })?;
        let trace_id = mm_core::id::parse_ulid(&text_column(row, "trace_id")?)?;
        Ok(Some(EpisodeRecord {
            id,
            goal: text_column(row, "goal")?,
            tier,
            stakes: real_column(row, "stakes")?,
            uncertainty: real_column(row, "uncertainty")?,
            reversibility: real_column(row, "reversibility")?,
            status,
            budget,
            trace_id,
        }))
    }

    async fn load_program(&self, id: ProgramId) -> Result<Option<CognitiveProgram>> {
        let rows = self
            .rows(SELECT_PROGRAM, vec![Param::Text(mm_core::ulid_string(&id))])
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let text = text_column(row, "program_json")?;
        let program: CognitiveProgram = serde_json::from_str(&text).map_err(|e| {
            MetacogError::Store(format!(
                "program {} did not decode: {e}",
                mm_core::ulid_string(&id)
            ))
        })?;
        Ok(Some(program))
    }

    async fn load_trace(&self, program_id: ProgramId) -> Result<ProgramTrace> {
        let id_text = mm_core::ulid_string(&program_id);
        let exists = self
            .rows(COUNT_PROGRAMS, vec![Param::Text(id_text.clone())])
            .await?;
        let count = exists
            .first()
            .map(|row| int_column(row, "n"))
            .transpose()?
            .unwrap_or(0);
        if count == 0 {
            return Err(MetacogError::Store(format!(
                "no program {id_text}; there is no trace to load"
            )));
        }
        let rows = self
            .rows(SELECT_TRACE_ROWS, vec![Param::Text(id_text.clone())])
            .await?;
        if rows.is_empty() {
            return Err(MetacogError::Store(format!(
                "program {id_text} has no trace rows"
            )));
        }
        let mut trace = ProgramTrace::new(program_id);
        for row in &rows {
            let value_text = text_column(row, "value_json")?;
            let value: OperationValue = serde_json::from_str(&value_text).map_err(|e| {
                MetacogError::Store(format!("trace value_json did not decode: {e}"))
            })?;
            let budget_text = text_column(row, "budget_after_json")?;
            let budget_after: CognitiveBudget =
                serde_json::from_str(&budget_text).map_err(|e| {
                    MetacogError::Store(format!("trace budget_after_json did not decode: {e}"))
                })?;
            let op_class_text = text_column(row, "op_class")?;
            let op_class = OpClass::parse(&op_class_text).ok_or_else(|| {
                MetacogError::Store(format!("trace has op_class {op_class_text:?}"))
            })?;
            let outcome_text = text_column(row, "outcome")?;
            let outcome = TraceOutcome::parse(&outcome_text).ok_or_else(|| {
                MetacogError::Store(format!("trace has outcome {outcome_text:?}"))
            })?;
            let seq = u32::try_from(int_column(row, "seq")?)
                .map_err(|_| MetacogError::Store("trace has a negative seq".to_string()))?;
            let node_id = u16::try_from(int_column(row, "node_id")?)
                .map_err(|_| MetacogError::Store("trace has a node_id outside u16".to_string()))?;
            trace.push(TraceRow {
                seq,
                node_id,
                op: text_column(row, "op")?,
                op_class,
                selected: int_column(row, "selected")? != 0,
                value,
                outcome,
                budget_after,
                stopping_reason: optional_text_column(row, "stopping_reason")?,
            })?;
        }
        trace.seal();
        trace.validate()?;
        Ok(trace)
    }

    async fn program_ids_for(&self, episode: EpisodeId) -> Result<Vec<ProgramId>> {
        let rows = self
            .rows(
                SELECT_PROGRAM_IDS,
                vec![Param::Text(mm_core::ulid_string(&episode))],
            )
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push(mm_core::id::parse_ulid(&text_column(row, "id")?)?);
        }
        Ok(out)
    }
}

/// A text column, or a refusal naming it.
fn text_column(row: &serde_json::Value, column: &str) -> Result<String> {
    row.get(column)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| MetacogError::Store(format!("the row has no text {column}")))
}

/// An optional text column: a NULL is `None`, a missing column is a refusal.
fn optional_text_column(row: &serde_json::Value, column: &str) -> Result<Option<String>> {
    match row.get(column) {
        None => Err(MetacogError::Store(format!("the row has no {column}"))),
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(MetacogError::Store(format!(
            "the row's {column} is neither text nor null"
        ))),
    }
}

/// An integer column, or a refusal naming it.
fn int_column(row: &serde_json::Value, column: &str) -> Result<i64> {
    row.get(column)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| MetacogError::Store(format!("the row has no integer {column}")))
}

/// A real column, accepting an integer value as a real one.
fn real_column(row: &serde_json::Value, column: &str) -> Result<f64> {
    row.get(column)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| MetacogError::Store(format!("the row has no real {column}")))
}
