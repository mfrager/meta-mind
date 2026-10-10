//! The `epistemic-ops` nexus module: the epistemic operations as a T-Box surface.
//!
//! A module is the being's *interface* to a subsystem, not the subsystem. The
//! discipline lives in `mm-epistemic`; this crate names the three operations the
//! plan exposes (`epistemic.claim`, `epistemic.assume`, `epistemic.predict`) and
//! describes each one's arity and effect so the code graph, the module manual, and
//! the operator's mental model agree.
//!
//! Every surface here is a *description*, not a second implementation: the fields
//! are exactly the ones `mm-epistemic` validates, so a handler wired to this
//! surface cannot accept something the engine would refuse.

#![forbid(unsafe_code)]

use serde::Serialize;

/// The stable IRI of this module.
pub const MODULE_URI: &str = "https://metamind.dev/code/module/epistemic/epistemic-ops";

/// The T-Box function names this module exposes.
pub const CLAIM: &str = "epistemic.claim";
/// Hold a proposition without support.
pub const ASSUME: &str = "epistemic.assume";
/// Expect something about the future.
pub const PREDICT: &str = "epistemic.predict";

/// One callable operation, as the plugin manifest and the manual describe it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Surface {
    /// The T-Box function name.
    pub function: &'static str,
    /// How many arguments it takes.
    pub arity: usize,
    /// What the arguments are, in order.
    pub arguments: &'static [&'static str],
    /// What it does, in one sentence.
    pub effect: &'static str,
}

/// The operation that records a claim of the given kind.
pub fn claim() -> Surface {
    Surface {
        function: CLAIM,
        arity: 5,
        arguments: &["subject", "predicate", "object", "kind", "confidence"],
        effect: "ingest a claim at the status its kind starts at, refusing any silent promotion",
    }
}

/// The operation that records an assumption.
pub fn assume() -> Surface {
    Surface {
        function: ASSUME,
        arity: 5,
        arguments: &[
            "subject",
            "predicate",
            "object",
            "consequence_if_false",
            "verification_cost",
        ],
        effect: "hold a proposition without support, with the risk of being wrong and the cost of checking",
    }
}

/// The operation that records a prediction.
pub fn predict() -> Surface {
    Surface {
        function: PREDICT,
        arity: 4,
        arguments: &["subject", "predicate", "object", "probability"],
        effect:
            "expect a proposition with a caller-supplied probability, resolvable against an outcome",
    }
}

/// Every surface, in a stable order.
pub fn surfaces() -> [Surface; 3] {
    [claim(), assume(), predict()]
}

/// The module's T-Box functions, for `plugin.toml` and the manual.
pub fn functions() -> [&'static str; 3] {
    [CLAIM, ASSUME, PREDICT]
}

/// The `mm:` class a surface's handle is typed as.
pub fn handle_class(surface: &Surface) -> &'static str {
    match surface.function {
        CLAIM => "mm:Claim",
        ASSUME => "mm:Assumption",
        PREDICT => "mm:Prediction",
        _ => "mm:Claim",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_surfaces_are_the_ones_the_plan_names() {
        assert_eq!(functions().len(), 3);
        let names: Vec<&str> = surfaces().iter().map(|s| s.function).collect();
        assert_eq!(names, vec![CLAIM, ASSUME, PREDICT]);
    }

    #[test]
    fn every_surface_has_an_arity_and_an_effect() {
        for surface in surfaces() {
            assert!(surface.arity > 0, "{}", surface.function);
            assert_eq!(
                surface.arguments.len(),
                surface.arity,
                "{} must name one argument per arity",
                surface.function
            );
            assert!(!surface.effect.is_empty());
            assert!(handle_class(&surface).starts_with("mm:"));
        }
    }
}
