//! The `metacog` module.
//!
//! Phase 8's metacognitive controller, stated as a contract. The behaviour lives
//! in `mm-metacog`, where the one broad scan, the tier policy, the deterministic
//! operation value, the compiler and the budget are pure functions with their own
//! tests; duplicating any of that here would create a second place for it to
//! drift, and the whole point of the controller is that there is exactly one place
//! where cognition is allocated.
//!
//! What this module owns is the *door*: two T-Box functions a caller can invoke
//! without linking the controller.
//!
//! * `cognition.plan` — compile an episode and its scan into a typed cognitive
//!   program. It refuses a malformed episode or scan rather than coercing one, and
//!   it never mints compute of its own: the tier's policy and the budget come from
//!   the episode, because a boundary that could raise either would be a second,
//!   unaudited source of compute.
//! * `cognition.op_value` — the operation-value expression, exposed so a caller
//!   can reproduce the controller's ranking without reimplementing the arithmetic.
//!
//! Both return `serde_json::Value` beside a typed refusal, and neither panics on
//! malformed input: a module boundary that unwrapped would turn a caller's bad
//! payload into a dead process.
//!
//! The manifest is embedded at compile time, so a module whose `plugin.toml` is
//! malformed — or whose declared `uri` has drifted from its directory — fails a
//! test rather than a load.
#![forbid(unsafe_code)]

use std::fmt;

/// `cognition.plan` — compile an episode and its scan into a cognitive program.
pub const COGNITION_PLAN: &str = "cognition.plan";
/// `cognition.op_value` — score one operation by the fixed expression.
pub const COGNITION_OP_VALUE: &str = "cognition.op_value";

/// Every T-Box function this module exposes, in a stable order.
pub const SURFACE_FUNCTIONS: [&str; 2] = [COGNITION_PLAN, COGNITION_OP_VALUE];

/// The capability this module implements.
pub const CAPABILITY: &str = "mm:CognitiveControl";

/// This module's path inside `modules/`, which is what its IRI is derived from.
pub const MODULE_PATH: &str = "cognition/metacog";

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// Every T-Box function this module exposes.
pub fn functions() -> Vec<&'static str> {
    SURFACE_FUNCTIONS.to_vec()
}

/// This module's stable, path-derived IRI.
pub fn module_iri() -> mm_core::NamedNode {
    mm_core::iri::module(MODULE_PATH)
}

/// The handler behind `cognition.plan`, named so the manifest and the binary
/// cannot drift silently.
pub fn plan_handler() -> &'static str {
    "handlers::plan"
}

/// The handler behind `cognition.op_value`.
pub fn op_value_handler() -> &'static str {
    "handlers::op_value"
}

/// Everything this module can refuse.
///
/// Two variants, because the two refusals mean different things to a caller: a
/// [`MetacogModuleError::Json`] refusal is the caller's payload not being the shape
/// the function takes, and a [`MetacogModuleError::Refused`] one is the controller
/// refusing a payload it understood. The controller's own code
/// (`validation`, `scan`, `graph`, …) is carried through rather than flattened, so
/// a caller can act on it the same way it would on a direct `mm-metacog` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetacogModuleError {
    /// The payload was not the JSON shape the function takes.
    Json {
        /// The T-Box function that refused.
        function: &'static str,
        /// What was wrong with the payload.
        detail: String,
    },
    /// `mm-metacog` refused a payload it understood.
    Refused {
        /// The T-Box function that refused.
        function: &'static str,
        /// The refusing error's own stable code.
        code: &'static str,
        /// The refusal, rendered.
        detail: String,
    },
}

impl MetacogModuleError {
    /// A payload-shape refusal.
    pub fn json(function: &'static str, detail: impl Into<String>) -> Self {
        MetacogModuleError::Json {
            function,
            detail: detail.into(),
        }
    }

    /// A controller refusal, carrying the controller's own code.
    pub fn refused(function: &'static str, code: &'static str, detail: impl Into<String>) -> Self {
        MetacogModuleError::Refused {
            function,
            code,
            detail: detail.into(),
        }
    }

    /// The T-Box function that refused.
    pub fn function(&self) -> &'static str {
        match self {
            MetacogModuleError::Json { function, .. }
            | MetacogModuleError::Refused { function, .. } => function,
        }
    }

    /// The stable code, so a caller can branch on the refusal rather than parse it.
    pub fn code(&self) -> &'static str {
        match self {
            MetacogModuleError::Json { .. } => "json",
            MetacogModuleError::Refused { code, .. } => code,
        }
    }
}

impl fmt::Display for MetacogModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetacogModuleError::Json { function, detail } => {
                write!(f, "{function}: malformed payload: {detail}")
            }
            MetacogModuleError::Refused {
                function,
                code,
                detail,
            } => write!(f, "{function}: {code}: {detail}"),
        }
    }
}

impl std::error::Error for MetacogModuleError {}

impl From<MetacogModuleError> for mm_core::MmError {
    fn from(e: MetacogModuleError) -> Self {
        mm_core::MmError::Internal(format!("{} ({})", e, e.code()))
    }
}

/// The two T-Box handlers. `plugin.toml` names them by their path
/// (`handlers::plan`), which is the name the code graph's symbol table knows them
/// by, so a rename here is a rename there or `codex verify` refuses it.
pub mod handlers {
    use serde_json::{json, Value};

    use crate::{MetacogModuleError, COGNITION_OP_VALUE, COGNITION_PLAN};

    // The trait is what provides `compile`; it is not re-exported by name because
    // a caller of this module must not be able to substitute a compiler.
    use mm_metacog::ProgramCompiler as _;

    /// `cognition.plan` — compile an episode and its scan into a cognitive program.
    ///
    /// `episode` is an `mm_metacog::CognitiveEpisode` as JSON and `scan` an
    /// `mm_metacog::ScanResult` as JSON. The returned value is the compiled
    /// `mm_metacog::CognitiveProgram`, including its content-derived `id`.
    ///
    /// A malformed payload and a program the controller refuses are both errors,
    /// never a partial program: a caller that received a program with no
    /// operations could not tell a refusal from a successful compile of nothing.
    pub fn plan(episode: &Value, scan: &Value) -> Result<Value, MetacogModuleError> {
        let episode: mm_metacog::CognitiveEpisode = serde_json::from_value(episode.clone())
            .map_err(|e| MetacogModuleError::json(COGNITION_PLAN, e.to_string()))?;
        let scan: mm_metacog::ScanResult = serde_json::from_value(scan.clone())
            .map_err(|e| MetacogModuleError::json(COGNITION_PLAN, e.to_string()))?;

        // The budget and the tier come from the episode, never from this function:
        // a door that could raise either would be a second, unaudited source of
        // compute.
        let budget = episode.budget;
        let program = mm_metacog::DefaultCompiler::new()
            .compile(&episode, &scan, &budget)
            .map_err(|e| MetacogModuleError::refused(COGNITION_PLAN, e.code(), e.to_string()))?;
        serde_json::to_value(&program)
            .map_err(|e| MetacogModuleError::json(COGNITION_PLAN, e.to_string()))
    }

    /// `cognition.op_value` — score one operation by the fixed expression.
    ///
    /// The expression is `eer * importance * p_change / max(cost, EPS)`, the same
    /// one `mm-metacog` selects by. An input outside `[0,1]`, or a negative or
    /// non-finite cost, is refused rather than clamped: a clamped score would let a
    /// caller's bug read as a legitimate ranking.
    pub fn op_value(
        expected_error_reduction: f64,
        decision_importance: f64,
        probability_change: f64,
        cost: f64,
    ) -> Result<Value, MetacogModuleError> {
        let value = mm_metacog::OperationValue::new(
            expected_error_reduction,
            decision_importance,
            probability_change,
            cost,
        )
        .map_err(|e| MetacogModuleError::refused(COGNITION_OP_VALUE, e.code(), e.to_string()))?;

        Ok(json!({
            "expected_error_reduction": value.expected_error_reduction,
            "decision_importance": value.decision_importance,
            "probability_change": value.probability_change,
            "cost": value.cost,
            "score": value.score(),
            "canonical": value.canonical(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_declares_this_module_and_its_functions() {
        assert!(MANIFEST_TOML.contains("name = \"metacog\""));
        assert!(MANIFEST_TOML.contains("owned_by_phase = 8"));
        assert!(MANIFEST_TOML.contains(CAPABILITY));
        assert!(MANIFEST_TOML.contains("category = \"cognition\""));
        for function in SURFACE_FUNCTIONS {
            assert!(
                MANIFEST_TOML.contains(function),
                "{function} is not declared in plugin.toml"
            );
        }
        assert!(MANIFEST_TOML.contains(plan_handler()));
        assert!(MANIFEST_TOML.contains(op_value_handler()));
    }

    #[test]
    fn the_declared_uri_is_the_path_derived_form_for_this_directory() {
        assert!(
            MANIFEST_TOML.contains(module_iri().as_str()),
            "plugin.toml's uri has drifted from {}, which is {}",
            MODULE_PATH,
            module_iri()
        );
    }

    #[test]
    fn a_refusal_names_the_function_and_carries_its_code() {
        let json_error = MetacogModuleError::json(COGNITION_PLAN, "expected a map");
        assert_eq!(json_error.function(), COGNITION_PLAN);
        assert_eq!(json_error.code(), "json");
        assert!(json_error.to_string().contains(COGNITION_PLAN));

        let refused =
            MetacogModuleError::refused(COGNITION_OP_VALUE, "validation", "cost must be >= 0");
        assert_eq!(refused.code(), "validation");
        assert!(refused.to_string().contains("validation"));

        // The module refusal converts into the kernel error rather than panicking.
        let mm: mm_core::MmError = refused.into();
        assert!(mm.to_string().contains("validation"), "{mm}");
    }
}
