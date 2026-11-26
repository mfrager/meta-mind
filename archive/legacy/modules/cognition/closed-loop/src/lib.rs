//! The `closed-loop` module.
//!
//! The developmental loop of Phase 12 runs ten stages over an event-sourced run:
//! experience → event log → meta-analysis → capability gap → change set →
//! self-engineering → test/benchmark → shadow → promotion gate → new version. The
//! *orchestration* of those stages — the budget arithmetic, the sandbox, the promotion
//! call, the hot-load — lives in `mm-runtime::loop_controller`, which is where the
//! evidence and the stores are, and this module deliberately owns none of it.
//!
//! What this module owns is the part that can be checked without a store: the **order**
//! of the ten stages and a deterministic **summary** of a run's stage records. Two
//! surfaces and one field that did not exist before — `first_refusal`, the stage a run
//! stopped at and why. An operator reading `10 stages, 9 ok` cannot tell whether the run
//! failed at the promotion gate or was merely refused a budget debit, and a refusal is
//! not a failure; the summary says which it was, in the order it happened.
//!
//! Three decisions are worth stating because they are not visible in the shape of the
//! code:
//!
//! * **`complete` is stricter than `all stages ran`.** It is true only when all ten
//!   stages are present exactly once *and* every outcome is `ok`. A run whose promotion
//!   gate refused is a run that ran every stage and produced nothing, and a module that
//!   called that complete would be reporting the loop's shape rather than its result.
//! * **A malformed record is refused, never repaired.** An unknown stage name, an index
//!   that repeats, an eleventh index, or an outcome outside `ok|refused|failed` is an
//!   error naming the offending record. Re-ordering or de-duplicating a caller's stages
//!   would silently assert that the loop's record is well formed, which is exactly the
//!   property the summary exists to check.
//! * **The order is a constant, not a computation.** `STAGES` is the list the controller
//!   walks, and `order_matches` compares a recorded run against it. Deriving the order
//!   from the records would make every run correct by construction.
//!
//! The manifest is embedded at compile time and parsed by a test, so a module whose
//! `plugin.toml` is malformed — or whose declared `uri` has drifted from its directory —
//! fails a test rather than a load.
#![forbid(unsafe_code)]

use std::fmt;

use mm_core::MmError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// `cognition.loop_status` — summarize a run's stage records.
pub const COGNITION_LOOP_STATUS: &str = "cognition.loop_status";
/// `cognition.loop_stage_order` — the canonical ten-stage order.
pub const COGNITION_LOOP_STAGE_ORDER: &str = "cognition.loop_stage_order";

/// Every T-Box function this module exposes, in a stable order.
pub const SURFACE_FUNCTIONS: [&str; 2] = [COGNITION_LOOP_STATUS, COGNITION_LOOP_STAGE_ORDER];

/// The capability this module implements.
pub const CAPABILITY: &str = "mm:ClosedLoopAutonomy";

/// This module's path inside `modules/`, which is what its IRI is derived from.
pub const MODULE_PATH: &str = "cognition/closed-loop";

/// The ten stages, in the order the controller runs them.
///
/// This is the loop's contract as data: a stage cannot be inserted, renamed or reordered
/// without changing this constant, and a run that does not match it is reported as not
/// matching rather than accepted because it happened.
pub const STAGES: [&str; 10] = [
    "experience",
    "event_log",
    "meta_analysis",
    "capability_gap",
    "change_set",
    "self_engineering",
    "test_benchmark",
    "shadow",
    "promotion_gate",
    "new_version",
];

/// The outcomes a stage record may carry.
///
/// `refused` and `failed` are distinct on purpose: a stage the budget envelope denied is
/// not a stage that broke, and the phase's rule is that a refusal is a decision rather
/// than an error.
pub const OUTCOMES: [&str; 3] = ["ok", "refused", "failed"];

/// One stage of a run, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageStatus {
    /// Position in the loop, from 0.
    pub idx: u32,
    /// The stage name, one of [`STAGES`].
    pub stage: String,
    /// One of [`OUTCOMES`].
    pub outcome: String,
    /// The artifact the stage produced, or the empty string when it produced none.
    #[serde(default)]
    pub artifact: String,
}

/// What a run's stage records say, as a summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopSummary {
    /// How many stage records were summarized.
    pub stages: usize,
    /// How many completed `ok`.
    pub ok: usize,
    /// How many were refused.
    pub refused: usize,
    /// How many failed.
    pub failed: usize,
    /// The first stage (in index order) that was refused or failed, with its outcome.
    pub first_refusal: Option<String>,
    /// True only when all ten stages ran exactly once and every one was `ok`.
    pub complete: bool,
}

/// Everything this module can refuse.
///
/// Two variants, because the two refusals mean different things to a caller: a
/// [`ClosedLoopModuleError::Json`] refusal is the caller's payload not being the shape
/// the function takes, and a [`ClosedLoopModuleError::Refused`] one is the module
/// refusing a payload it understood — a stage the loop does not have, a repeated index,
/// an outcome that is not an outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosedLoopModuleError {
    /// The payload was not the JSON shape the function takes.
    Json {
        /// The T-Box function that refused.
        function: &'static str,
        /// What was wrong with the payload.
        detail: String,
    },
    /// The module refused a payload it understood.
    Refused {
        /// The T-Box function that refused.
        function: &'static str,
        /// A stable code for the refusal.
        code: &'static str,
        /// What was refused and why.
        detail: String,
    },
}

impl ClosedLoopModuleError {
    /// A payload-shape refusal.
    pub fn json(function: &'static str, detail: impl Into<String>) -> Self {
        ClosedLoopModuleError::Json {
            function,
            detail: detail.into(),
        }
    }

    /// A refusal of a payload the module understood.
    pub fn refused(function: &'static str, code: &'static str, detail: impl Into<String>) -> Self {
        ClosedLoopModuleError::Refused {
            function,
            code,
            detail: detail.into(),
        }
    }

    /// The T-Box function that refused.
    pub fn function(&self) -> &'static str {
        match self {
            ClosedLoopModuleError::Json { function, .. }
            | ClosedLoopModuleError::Refused { function, .. } => function,
        }
    }

    /// The stable code, so a caller can branch on the refusal rather than parse it.
    pub fn code(&self) -> &'static str {
        match self {
            ClosedLoopModuleError::Json { .. } => "json",
            ClosedLoopModuleError::Refused { code, .. } => code,
        }
    }
}

impl fmt::Display for ClosedLoopModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClosedLoopModuleError::Json { function, detail } => {
                write!(f, "{function}: malformed payload: {detail}")
            }
            ClosedLoopModuleError::Refused {
                function,
                code,
                detail,
            } => {
                write!(f, "{function}: {code}: {detail}")
            }
        }
    }
}

impl std::error::Error for ClosedLoopModuleError {}

impl From<ClosedLoopModuleError> for MmError {
    fn from(e: ClosedLoopModuleError) -> Self {
        MmError::Internal(format!("{} ({})", e, e.code()))
    }
}

/// Every T-Box function this module exposes.
pub fn functions() -> Vec<&'static str> {
    SURFACE_FUNCTIONS.to_vec()
}

/// This module's stable, path-derived IRI.
pub fn module_iri() -> mm_core::NamedNode {
    mm_core::iri::module(MODULE_PATH)
}

/// The handler behind `cognition.loop_status`, named so the manifest and the binary
/// cannot drift silently.
pub fn status_handler() -> &'static str {
    "handlers::loop_status"
}

/// The handler behind `cognition.loop_stage_order`.
pub fn stage_order_handler() -> &'static str {
    "handlers::loop_stage_order"
}

/// This module's directory, as compiled.
pub fn module_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// This module's manifest path, as compiled.
pub fn manifest_path() -> std::path::PathBuf {
    module_dir().join("plugin.toml")
}

/// The stage name at `idx`, or `None` when the loop has no such position.
pub fn expected_stage(idx: usize) -> Option<&'static str> {
    STAGES.get(idx).copied()
}

/// True when every record names the stage that belongs at its index and all ten
/// positions are covered exactly once.
///
/// A run with a hole in it is not in order: the loop is a sequence, so a missing stage is
/// as much a mismatch as a wrong one.
pub fn order_matches(stages: &[StageStatus]) -> bool {
    if stages.len() != STAGES.len() {
        return false;
    }
    let mut seen = [false; STAGES.len()];
    for stage in stages {
        let idx = stage.idx as usize;
        let Some(expected) = expected_stage(idx) else {
            return false;
        };
        if expected != stage.stage {
            return false;
        }
        if seen[idx] {
            return false;
        }
        seen[idx] = true;
    }
    seen.iter().all(|present| *present)
}

/// Summarize a run's stage records.
///
/// The records are validated first and summarized second: a summary computed over
/// records the module would not accept would be a number an operator cannot trust, so a
/// malformed record is a refusal rather than a skipped row.
pub fn summarize(stages: &[StageStatus]) -> Result<LoopSummary, ClosedLoopModuleError> {
    let mut seen = [false; STAGES.len()];
    for stage in stages {
        let idx = stage.idx as usize;
        let Some(expected) = expected_stage(idx) else {
            return Err(ClosedLoopModuleError::refused(
                COGNITION_LOOP_STATUS,
                "validation",
                format!(
                    "index {} is not one of the loop's {} stages",
                    stage.idx,
                    STAGES.len()
                ),
            ));
        };
        if expected != stage.stage {
            return Err(ClosedLoopModuleError::refused(
                COGNITION_LOOP_STATUS,
                "validation",
                format!("index {} is {expected:?}, not {:?}", stage.idx, stage.stage),
            ));
        }
        if seen[idx] {
            return Err(ClosedLoopModuleError::refused(
                COGNITION_LOOP_STATUS,
                "validation",
                format!("index {} appears more than once", stage.idx),
            ));
        }
        seen[idx] = true;
        if !OUTCOMES.contains(&stage.outcome.as_str()) {
            return Err(ClosedLoopModuleError::refused(
                COGNITION_LOOP_STATUS,
                "validation",
                format!(
                    "stage {idx} has outcome {:?}, which is not one of {OUTCOMES:?}",
                    stage.outcome
                ),
            ));
        }
    }

    // Index order, so `first_refusal` is the first refusal *in the run*, not the first
    // one in whatever order the caller handed the records over.
    let mut ordered: Vec<&StageStatus> = stages.iter().collect();
    ordered.sort_by_key(|stage| stage.idx);

    let ok = ordered.iter().filter(|s| s.outcome == "ok").count();
    let refused = ordered.iter().filter(|s| s.outcome == "refused").count();
    let failed = ordered.iter().filter(|s| s.outcome == "failed").count();
    let first_refusal = ordered
        .iter()
        .find(|s| s.outcome != "ok")
        .map(|s| format!("{}: {}", s.stage, s.outcome));
    let complete = ordered.len() == STAGES.len()
        && ordered.iter().enumerate().all(|(idx, stage)| {
            stage.idx as usize == idx && stage.stage == expected_stage(idx).unwrap_or_default()
        })
        && ok == STAGES.len();

    Ok(LoopSummary {
        stages: ordered.len(),
        ok,
        refused,
        failed,
        first_refusal,
        complete,
    })
}

/// Turn a JSON array of stage records into typed ones, or refuse.
pub fn stages_from_json(
    value: &Value,
    function: &'static str,
) -> Result<Vec<StageStatus>, ClosedLoopModuleError> {
    let array = match value.as_array() {
        Some(array) => array,
        None => {
            return Err(ClosedLoopModuleError::json(
                function,
                "expected an array of stage records",
            ))
        }
    };
    let mut stages = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        let stage: StageStatus = serde_json::from_value(item.clone()).map_err(|e| {
            ClosedLoopModuleError::json(
                function,
                format!("record {index} is not a stage record: {e}"),
            )
        })?;
        stages.push(stage);
    }
    Ok(stages)
}

/// The two T-Box handlers. `plugin.toml` names them by their path
/// (`handlers::loop_status`), which is the name the code graph's symbol table knows them
/// by, so a rename here is a rename there or `codex verify` refuses it.
pub mod handlers {
    use serde_json::{json, Value};

    use crate::{
        stages_from_json, summarize, ClosedLoopModuleError, COGNITION_LOOP_STAGE_ORDER,
        COGNITION_LOOP_STATUS, STAGES,
    };

    /// `cognition.loop_status` — summarize a run's stage records.
    ///
    /// `stages` is a JSON array of `{idx, stage, outcome, artifact}`. The returned value
    /// is the [`crate::LoopSummary`].
    pub fn loop_status(stages: &Value) -> Result<Value, ClosedLoopModuleError> {
        let stages = stages_from_json(stages, COGNITION_LOOP_STATUS)?;
        let summary = summarize(&stages)?;
        serde_json::to_value(&summary)
            .map_err(|e| ClosedLoopModuleError::json(COGNITION_LOOP_STATUS, e.to_string()))
    }

    /// `cognition.loop_stage_order` — the canonical ten-stage order.
    pub fn loop_stage_order() -> Result<Value, ClosedLoopModuleError> {
        Ok(json!({
            "stages": STAGES,
            "count": STAGES.len(),
            "function": COGNITION_LOOP_STAGE_ORDER,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stage(idx: u32, outcome: &str) -> StageStatus {
        StageStatus {
            idx,
            stage: expected_stage(idx as usize)
                .expect("a stage at that index")
                .to_string(),
            outcome: outcome.to_string(),
            artifact: format!("artifact-{idx}"),
        }
    }

    fn complete_run() -> Vec<StageStatus> {
        (0..STAGES.len() as u32)
            .map(|idx| stage(idx, "ok"))
            .collect()
    }

    #[test]
    fn a_complete_run_summarizes_as_complete() {
        let summary = summarize(&complete_run()).expect("a summary");
        assert_eq!(summary.stages, 10);
        assert_eq!(summary.ok, 10);
        assert_eq!(summary.refused, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.first_refusal, None);
        assert!(summary.complete);
    }

    #[test]
    fn a_refused_stage_is_not_complete_and_is_named() {
        let mut run = complete_run();
        run[8] = stage(8, "refused");
        let summary = summarize(&run).expect("a summary");
        assert_eq!(summary.ok, 9);
        assert_eq!(summary.refused, 1);
        assert!(!summary.complete);
        assert_eq!(
            summary.first_refusal.as_deref(),
            Some("promotion_gate: refused")
        );
    }

    #[test]
    fn the_first_refusal_is_the_first_in_run_order_not_record_order() {
        let mut run = complete_run();
        run[7] = stage(7, "failed");
        run[3] = stage(3, "refused");
        run.reverse();
        let summary = summarize(&run).expect("a summary");
        assert_eq!(
            summary.first_refusal.as_deref(),
            Some("capability_gap: refused")
        );
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn a_repeated_index_is_refused() {
        let mut run = complete_run();
        run[1].idx = 0;
        run[1].stage = expected_stage(0).unwrap().to_string();
        let error = summarize(&run).expect_err("a refusal");
        assert_eq!(error.code(), "validation");
        assert_eq!(error.function(), COGNITION_LOOP_STATUS);
        assert!(error.to_string().contains("more than once"), "{error}");
    }

    #[test]
    fn an_unknown_stage_name_is_refused() {
        let mut run = complete_run();
        run[4].stage = "shadow_gate".to_string();
        let error = summarize(&run).expect_err("a refusal");
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("shadow_gate"), "{error}");
    }

    #[test]
    fn an_index_beyond_the_tenth_stage_is_refused() {
        let mut run = complete_run();
        run.push(StageStatus {
            idx: 10,
            stage: "post_new_version".to_string(),
            outcome: "ok".to_string(),
            artifact: String::new(),
        });
        let error = summarize(&run).expect_err("a refusal");
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("10"), "{error}");
    }

    #[test]
    fn an_unknown_outcome_is_refused() {
        let mut run = complete_run();
        run[2].outcome = "maybe".to_string();
        let error = summarize(&run).expect_err("a refusal");
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("maybe"), "{error}");
    }

    #[test]
    fn a_short_run_is_summarized_but_is_not_complete() {
        let run = complete_run();
        let summary = summarize(&run[..9]).expect("a summary");
        assert_eq!(summary.stages, 9);
        assert!(!summary.complete);
    }

    #[test]
    fn the_order_matches_only_the_canonical_order() {
        let run = complete_run();
        assert!(order_matches(&run));
        let mut wrong = run.clone();
        wrong.swap(0, 1);
        // The records still name the stage that belongs at each index, because `swap`
        // moved the (idx, stage) pairs together, so the *set* is right and the recorded
        // indices are what the summary sorts by; a genuinely wrong order is a record
        // whose name does not match its index.
        assert!(order_matches(&wrong));
        wrong[0].stage = "completed".to_string();
        assert!(!order_matches(&wrong));
        assert!(!order_matches(&run[..9]));
        // A run with a hole is not in order, even though every record is well formed.
        let mut hole = run.clone();
        hole[3].idx = 4;
        hole[3].stage = expected_stage(4).unwrap().to_string();
        assert!(!order_matches(&hole));
    }

    #[test]
    fn the_handlers_answer_the_same_summary_as_the_typed_api() {
        let run = complete_run();
        let as_json = serde_json::to_value(&run).expect("the records serialize");
        let out = handlers::loop_status(&as_json).expect("a summary");
        let summary = summarize(&run).expect("a summary");
        assert_eq!(out["stages"].as_u64(), Some(summary.stages as u64));
        assert_eq!(out["ok"].as_u64(), Some(summary.ok as u64));
        assert_eq!(out["complete"], json!(true));
        assert_eq!(out["first_refusal"], json!(null));

        let order = handlers::loop_stage_order().expect("an order");
        assert_eq!(order["count"].as_u64(), Some(10));
        assert_eq!(order["stages"][0], json!("experience"));
        assert_eq!(order["stages"][9], json!("new_version"));
    }

    #[test]
    fn the_handlers_refuse_a_malformed_payload() {
        for payload in [json!({}), json!(null), json!(7), json!("stages")] {
            let error = handlers::loop_status(&payload).expect_err("a refusal");
            assert_eq!(error.code(), "json");
            assert_eq!(error.function(), COGNITION_LOOP_STATUS);
        }
        let bad = json!([{ "idx": 0, "stage": "experience", "outcome": "perhaps" }]);
        let error = handlers::loop_status(&bad).expect_err("a refusal");
        assert_eq!(error.code(), "validation");
        let mm: MmError = error.into();
        assert!(mm.to_string().contains("validation"), "{mm}");
    }

    #[test]
    fn the_embedded_manifest_declares_this_module_and_its_functions() {
        let manifest: toml::Value = toml::from_str(MANIFEST_TOML).expect("plugin.toml parses");
        assert_eq!(manifest["plugin"]["name"].as_str(), Some("closed-loop"));
        assert_eq!(manifest["plugin"]["version"].as_str(), Some("0.1.0"));
        assert_eq!(
            manifest["plugin"]["uri"].as_str(),
            Some(module_iri().as_str()),
            "plugin.toml's uri has drifted from {MODULE_PATH}"
        );
        assert_eq!(manifest["metadata"]["category"].as_str(), Some("cognition"));
        assert_eq!(
            manifest["metadata"]["owned_by_phase"].as_integer(),
            Some(12)
        );
        assert_eq!(
            manifest["metadata"]["capability"].as_str(),
            Some(CAPABILITY)
        );

        let functions = manifest["tbox"]["functions"]
            .as_table()
            .expect("a tbox function table");
        assert_eq!(functions.len(), SURFACE_FUNCTIONS.len());
        for function in SURFACE_FUNCTIONS {
            assert!(functions.contains_key(function), "{function} is undeclared");
        }
        assert_eq!(
            manifest["tbox"]["functions"][COGNITION_LOOP_STATUS]["source"].as_str(),
            Some(status_handler())
        );
        assert_eq!(
            manifest["tbox"]["functions"][COGNITION_LOOP_STAGE_ORDER]["source"].as_str(),
            Some(stage_order_handler())
        );
        assert_eq!(
            manifest["monad"]["operations"]["name"].as_str(),
            Some("cognition")
        );
        assert_eq!(
            manifest["monad"]["operations"]["arity"].as_integer(),
            Some(1)
        );
    }

    #[test]
    fn the_module_names_itself_without_inventing_a_second_spelling() {
        assert_eq!(functions(), SURFACE_FUNCTIONS.to_vec());
        assert_eq!(CAPABILITY, "mm:ClosedLoopAutonomy");
        assert_eq!(
            module_iri().as_str(),
            "https://metamind.dev/code/module/cognition/closed-loop"
        );
        assert_eq!(manifest_path().file_name().unwrap(), "plugin.toml");
        assert_eq!(
            module_dir().file_name().unwrap(),
            std::ffi::OsStr::new("closed-loop")
        );
        assert_eq!(expected_stage(0), Some("experience"));
        assert_eq!(expected_stage(9), Some("new_version"));
        assert_eq!(expected_stage(10), None);
        assert_eq!(STAGES.len(), 10);
    }
}
