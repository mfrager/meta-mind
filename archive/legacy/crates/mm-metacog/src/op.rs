//! The operation algebra: what the controller can think, and what each thought
//! costs.
//!
//! [`OpClass`] is the taxonomy of self-* operations (the Self-* survey index in
//! the phase 8 plan): every [`CognitiveOp`] maps to exactly one class, so a
//! compute policy can allow or forbid a *kind* of cognition without enumerating
//! op shapes. The class is the axis the tier ladder gates on; the op is what
//! actually gets compiled into the graph.
//!
//! [`CostClass`] maps to a fixed `f64` through [`COST_TABLE`]. Costs are
//! deterministic on purpose: the phase's invariant 3 says selection is
//! deterministic, and a cost that came from a model would make two runs of the
//! same episode choose differently.
//!
//! Two deliberate additions to the plan's op list: `Clarify`, `Classify`,
//! `Decompose`, `SearchPrecedent`, `Predict`, `RedTeam`, `CheckConstraint`,
//! `CheckContradiction`, `CheckCausality`, and `Synthesize` exist so that *every*
//! `OpClass` is reachable from some op. Without them the taxonomy would contain
//! classes no policy could ever permit, which is a taxonomy that cannot be
//! tested.

use serde::{Deserialize, Serialize};

use crate::episode::{
    ActionId, CandidateId, CaseId, ClaimId, ConstraintId, GoalId, PropositionId, ScenarioId,
    SourceId,
};

/// The kind of cognition an operation performs.
///
/// Ordered as the plan lists it, and `ALL` is that same order, so a policy that
/// serializes its allowed set has a stable rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpClass {
    /// Take in the world directly.
    Observe,
    /// Bring back something already known.
    Recall,
    /// Ask what is missing.
    Clarify,
    /// Put a thing into a category.
    Classify,
    /// Break a goal into parts.
    Decompose,
    /// Weigh two things against each other.
    Compare,
    /// Reason from a partly-matching precedent.
    Analogize,
    /// Look for a precedent.
    SearchPrecedent,
    /// Reason backwards from the goal.
    Invert,
    /// Anticipate a proposition.
    Predict,
    /// Run a scenario forward.
    Simulate,
    /// Establish that a claim is true.
    Verify,
    /// Find what is wrong with an answer.
    Critique,
    /// Improve an answer in light of a critique.
    Refine,
    /// Attack an answer adversarially.
    RedTeam,
    /// Search a space of thoughts.
    Search,
    /// Check a constraint is respected.
    CheckConstraints,
    /// Check an assumption still holds.
    CheckAssumptions,
    /// Check two claims do not conflict.
    CheckContradictions,
    /// Check a claimed cause actually causes.
    CheckCausality,
    /// Make an answer smaller.
    Simplify,
    /// Combine answers into one.
    Synthesize,
    /// Choose.
    Decide,
    /// Do something in the world.
    Act,
    /// Measure what happened.
    Measure,
    /// Keep what was learned.
    Learn,
}

/// Every class, in the plan's order.
pub const OP_CLASSES: [OpClass; 26] = [
    OpClass::Observe,
    OpClass::Recall,
    OpClass::Clarify,
    OpClass::Classify,
    OpClass::Decompose,
    OpClass::Compare,
    OpClass::Analogize,
    OpClass::SearchPrecedent,
    OpClass::Invert,
    OpClass::Predict,
    OpClass::Simulate,
    OpClass::Verify,
    OpClass::Critique,
    OpClass::Refine,
    OpClass::RedTeam,
    OpClass::Search,
    OpClass::CheckConstraints,
    OpClass::CheckAssumptions,
    OpClass::CheckContradictions,
    OpClass::CheckCausality,
    OpClass::Simplify,
    OpClass::Synthesize,
    OpClass::Decide,
    OpClass::Act,
    OpClass::Measure,
    OpClass::Learn,
];

impl OpClass {
    /// Every class, in the plan's order.
    pub const ALL: [OpClass; 26] = OP_CLASSES;

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            OpClass::Observe => "observe",
            OpClass::Recall => "recall",
            OpClass::Clarify => "clarify",
            OpClass::Classify => "classify",
            OpClass::Decompose => "decompose",
            OpClass::Compare => "compare",
            OpClass::Analogize => "analogize",
            OpClass::SearchPrecedent => "search_precedent",
            OpClass::Invert => "invert",
            OpClass::Predict => "predict",
            OpClass::Simulate => "simulate",
            OpClass::Verify => "verify",
            OpClass::Critique => "critique",
            OpClass::Refine => "refine",
            OpClass::RedTeam => "red_team",
            OpClass::Search => "search",
            OpClass::CheckConstraints => "check_constraints",
            OpClass::CheckAssumptions => "check_assumptions",
            OpClass::CheckContradictions => "check_contradictions",
            OpClass::CheckCausality => "check_causality",
            OpClass::Simplify => "simplify",
            OpClass::Synthesize => "synthesize",
            OpClass::Decide => "decide",
            OpClass::Act => "act",
            OpClass::Measure => "measure",
            OpClass::Learn => "learn",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        OP_CLASSES.into_iter().find(|c| c.as_str() == text)
    }

    /// The class's position in [`OP_CLASSES`]. It is the bit a compute policy
    /// uses to allow or forbid the class, so the position is part of the wire
    /// format and may not be reordered without a migration.
    pub fn index(self) -> usize {
        match self {
            OpClass::Observe => 0,
            OpClass::Recall => 1,
            OpClass::Clarify => 2,
            OpClass::Classify => 3,
            OpClass::Decompose => 4,
            OpClass::Compare => 5,
            OpClass::Analogize => 6,
            OpClass::SearchPrecedent => 7,
            OpClass::Invert => 8,
            OpClass::Predict => 9,
            OpClass::Simulate => 10,
            OpClass::Verify => 11,
            OpClass::Critique => 12,
            OpClass::Refine => 13,
            OpClass::RedTeam => 14,
            OpClass::Search => 15,
            OpClass::CheckConstraints => 16,
            OpClass::CheckAssumptions => 17,
            OpClass::CheckContradictions => 18,
            OpClass::CheckCausality => 19,
            OpClass::Simplify => 20,
            OpClass::Synthesize => 21,
            OpClass::Decide => 22,
            OpClass::Act => 23,
            OpClass::Measure => 24,
            OpClass::Learn => 25,
        }
    }
}

impl std::fmt::Display for OpClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What an operation costs, as a band rather than a number.
///
/// The band exists so a compiled program is readable — "this needs a tool" is a
/// fact about the operation, not about today's price list — while
/// [`COST_TABLE`] keeps the number the selector actually uses fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    /// Costs nothing to attempt: pure arithmetic over what is already held.
    Free,
    /// One short, cheap step.
    Cheap,
    /// A model call worth counting.
    Moderate,
    /// A model call or a search that is worth avoiding.
    Expensive,
    /// Reaches outside the process.
    Tool,
}

/// The fixed cost of each band, in arbitrary but consistent units.
///
/// `Free` is genuinely `0.0`; [`crate::value::OperationValue::score`] divides by
/// `max(cost, EPS)`, so a free operation is ranked by its numerator alone and is
/// never a division by zero.
pub const COST_TABLE: [f64; 5] = [0.0, 0.01, 0.05, 0.25, 0.5];

impl CostClass {
    /// Every band, cheapest first.
    pub const ALL: [CostClass; 5] = [
        CostClass::Free,
        CostClass::Cheap,
        CostClass::Moderate,
        CostClass::Expensive,
        CostClass::Tool,
    ];

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            CostClass::Free => "free",
            CostClass::Cheap => "cheap",
            CostClass::Moderate => "moderate",
            CostClass::Expensive => "expensive",
            CostClass::Tool => "tool",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        CostClass::ALL.into_iter().find(|c| c.as_str() == text)
    }

    /// The fixed cost of this band.
    pub fn cost(self) -> f64 {
        COST_TABLE[self.index()]
    }

    /// The band's position in [`COST_TABLE`].
    pub fn index(self) -> usize {
        match self {
            CostClass::Free => 0,
            CostClass::Cheap => 1,
            CostClass::Moderate => 2,
            CostClass::Expensive => 3,
            CostClass::Tool => 4,
        }
    }
}

impl std::fmt::Display for CostClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One cognitive operation, with its operand.
///
/// Serializable with an `op` tag, so the stored form reads
/// `{"op":"recall","query":"…"}` rather than a positional array. The variant set
/// is closed: a new kind of cognition is a new variant *and* a new class mapping,
/// which the coverage test forces a caller to add deliberately.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CognitiveOp {
    /// Bring back something already known.
    Recall {
        /// What to recall.
        query: String,
    },
    /// Take in the world directly.
    Observe {
        /// Where the observation comes from.
        source: SourceId,
    },
    /// Propose a proposition to be tested.
    FormHypothesis {
        /// The proposed proposition.
        proposition: PropositionId,
    },
    /// Weigh candidates against each other.
    Compare {
        /// What is being compared.
        candidates: Vec<CandidateId>,
    },
    /// Reason from a precedent.
    FindSimilar {
        /// The case to reason from.
        case: CaseId,
    },
    /// Check an assumption still holds.
    CheckAssumption {
        /// The assumption.
        assumption: mm_core::Ulid,
    },
    /// Establish that a claim is true.
    Verify {
        /// The claim.
        claim: ClaimId,
    },
    /// Reason backwards from the goal.
    Invert {
        /// The goal to invert.
        goal: GoalId,
    },
    /// Run a scenario forward.
    Simulate {
        /// The scenario.
        scenario: ScenarioId,
    },
    /// Find what is wrong with an answer.
    Critique {
        /// The answer.
        candidate: CandidateId,
    },
    /// Improve an answer in light of a critique.
    Refine {
        /// The answer.
        candidate: CandidateId,
    },
    /// Attack an answer adversarially.
    RedTeam {
        /// The answer.
        candidate: CandidateId,
    },
    /// Search a space of thoughts, bounded by the policy's width.
    Search {
        /// What is being searched.
        over: String,
        /// The branching width the policy allowed.
        budget: u8,
    },
    /// Make an answer smaller.
    Simplify {
        /// The answer.
        solution: CandidateId,
    },
    /// Choose.
    Decide {
        /// The candidates under consideration.
        candidates: Vec<CandidateId>,
    },
    /// Do something in the world. Depends on a `Decide`.
    Act {
        /// The action.
        action: ActionId,
    },
    /// Measure an outcome.
    Evaluate {
        /// What happened. The Phase 6 type, because it is the only one (D2).
        outcome: mm_epistemic::Outcome,
    },
    /// Keep what was learned.
    Learn {
        /// The lesson.
        lesson: ActionId,
    },
    /// Ask what is missing.
    Clarify {
        /// The question.
        question: String,
    },
    /// Put a thing into a category.
    Classify {
        /// What is being classified.
        candidate: CandidateId,
        /// The taxonomy it is classified into.
        taxonomy: String,
    },
    /// Break a goal into parts.
    Decompose {
        /// The goal.
        goal: GoalId,
        /// The parts.
        into: Vec<String>,
    },
    /// Look for a precedent.
    SearchPrecedent {
        /// The problem to find a precedent for.
        problem: String,
    },
    /// Anticipate a proposition.
    Predict {
        /// The proposition.
        proposition: PropositionId,
    },
    /// Check a constraint is respected.
    CheckConstraint {
        /// The constraint.
        constraint: ConstraintId,
    },
    /// Check two claims do not conflict.
    CheckContradiction {
        /// The claim.
        claim: ClaimId,
    },
    /// Check a claimed cause actually causes.
    CheckCausality {
        /// The alleged cause.
        cause: String,
        /// The alleged effect.
        effect: String,
    },
    /// Combine answers into one.
    Synthesize {
        /// The answers.
        candidates: Vec<CandidateId>,
    },
}

impl CognitiveOp {
    /// The class this operation belongs to. Total by construction.
    pub fn class(&self) -> OpClass {
        match self {
            CognitiveOp::Recall { .. } => OpClass::Recall,
            CognitiveOp::Observe { .. } => OpClass::Observe,
            CognitiveOp::FormHypothesis { .. } => OpClass::Predict,
            CognitiveOp::Compare { .. } => OpClass::Compare,
            CognitiveOp::FindSimilar { .. } => OpClass::Analogize,
            CognitiveOp::CheckAssumption { .. } => OpClass::CheckAssumptions,
            CognitiveOp::Verify { .. } => OpClass::Verify,
            CognitiveOp::Invert { .. } => OpClass::Invert,
            CognitiveOp::Simulate { .. } => OpClass::Simulate,
            CognitiveOp::Critique { .. } => OpClass::Critique,
            CognitiveOp::Refine { .. } => OpClass::Refine,
            CognitiveOp::RedTeam { .. } => OpClass::RedTeam,
            CognitiveOp::Search { .. } => OpClass::Search,
            CognitiveOp::Simplify { .. } => OpClass::Simplify,
            CognitiveOp::Decide { .. } => OpClass::Decide,
            CognitiveOp::Act { .. } => OpClass::Act,
            CognitiveOp::Evaluate { .. } => OpClass::Measure,
            CognitiveOp::Learn { .. } => OpClass::Learn,
            CognitiveOp::Clarify { .. } => OpClass::Clarify,
            CognitiveOp::Classify { .. } => OpClass::Classify,
            CognitiveOp::Decompose { .. } => OpClass::Decompose,
            CognitiveOp::SearchPrecedent { .. } => OpClass::SearchPrecedent,
            CognitiveOp::Predict { .. } => OpClass::Predict,
            CognitiveOp::CheckConstraint { .. } => OpClass::CheckConstraints,
            CognitiveOp::CheckContradiction { .. } => OpClass::CheckContradictions,
            CognitiveOp::CheckCausality { .. } => OpClass::CheckCausality,
            CognitiveOp::Synthesize { .. } => OpClass::Synthesize,
        }
    }

    /// What this operation costs.
    pub fn cost_class(&self) -> CostClass {
        match self {
            CognitiveOp::Recall { .. } => CostClass::Cheap,
            CognitiveOp::Observe { .. } => CostClass::Tool,
            CognitiveOp::FormHypothesis { .. } => CostClass::Free,
            CognitiveOp::Compare { .. } => CostClass::Cheap,
            CognitiveOp::FindSimilar { .. } => CostClass::Cheap,
            CognitiveOp::CheckAssumption { .. } => CostClass::Cheap,
            CognitiveOp::Verify { .. } => CostClass::Expensive,
            CognitiveOp::Invert { .. } => CostClass::Free,
            CognitiveOp::Simulate { .. } => CostClass::Expensive,
            CognitiveOp::Critique { .. } => CostClass::Moderate,
            CognitiveOp::Refine { .. } => CostClass::Moderate,
            CognitiveOp::RedTeam { .. } => CostClass::Moderate,
            CognitiveOp::Search { .. } => CostClass::Expensive,
            CognitiveOp::Simplify { .. } => CostClass::Free,
            CognitiveOp::Decide { .. } => CostClass::Free,
            CognitiveOp::Act { .. } => CostClass::Tool,
            CognitiveOp::Evaluate { .. } => CostClass::Moderate,
            CognitiveOp::Learn { .. } => CostClass::Free,
            CognitiveOp::Clarify { .. } => CostClass::Cheap,
            CognitiveOp::Classify { .. } => CostClass::Cheap,
            CognitiveOp::Decompose { .. } => CostClass::Free,
            CognitiveOp::SearchPrecedent { .. } => CostClass::Cheap,
            CognitiveOp::Predict { .. } => CostClass::Moderate,
            CognitiveOp::CheckConstraint { .. } => CostClass::Free,
            CognitiveOp::CheckContradiction { .. } => CostClass::Cheap,
            CognitiveOp::CheckCausality { .. } => CostClass::Cheap,
            CognitiveOp::Synthesize { .. } => CostClass::Moderate,
        }
    }

    /// The variant tag, matching the stored `op` column and the `mm:opClass`
    /// rendering.
    pub fn tag(&self) -> &'static str {
        match self {
            CognitiveOp::Recall { .. } => "recall",
            CognitiveOp::Observe { .. } => "observe",
            CognitiveOp::FormHypothesis { .. } => "form_hypothesis",
            CognitiveOp::Compare { .. } => "compare",
            CognitiveOp::FindSimilar { .. } => "find_similar",
            CognitiveOp::CheckAssumption { .. } => "check_assumption",
            CognitiveOp::Verify { .. } => "verify",
            CognitiveOp::Invert { .. } => "invert",
            CognitiveOp::Simulate { .. } => "simulate",
            CognitiveOp::Critique { .. } => "critique",
            CognitiveOp::Refine { .. } => "refine",
            CognitiveOp::RedTeam { .. } => "red_team",
            CognitiveOp::Search { .. } => "search",
            CognitiveOp::Simplify { .. } => "simplify",
            CognitiveOp::Decide { .. } => "decide",
            CognitiveOp::Act { .. } => "act",
            CognitiveOp::Evaluate { .. } => "evaluate",
            CognitiveOp::Learn { .. } => "learn",
            CognitiveOp::Clarify { .. } => "clarify",
            CognitiveOp::Classify { .. } => "classify",
            CognitiveOp::Decompose { .. } => "decompose",
            CognitiveOp::SearchPrecedent { .. } => "search_precedent",
            CognitiveOp::Predict { .. } => "predict",
            CognitiveOp::CheckConstraint { .. } => "check_constraint",
            CognitiveOp::CheckContradiction { .. } => "check_contradiction",
            CognitiveOp::CheckCausality { .. } => "check_causality",
            CognitiveOp::Synthesize { .. } => "synthesize",
        }
    }

    /// The records this operation is about, as IRIs or ids, sorted and
    /// deduplicated. Used for the `mm:targets` edges a program emits.
    pub fn targets(&self) -> Vec<String> {
        let mut out: Vec<String> = match self {
            CognitiveOp::Recall { query } => vec![query.clone()],
            CognitiveOp::Observe { source } => vec![source.clone()],
            CognitiveOp::FormHypothesis { proposition } | CognitiveOp::Predict { proposition } => {
                vec![proposition.clone()]
            }
            CognitiveOp::Compare { candidates }
            | CognitiveOp::Decide { candidates }
            | CognitiveOp::Synthesize { candidates } => candidates.clone(),
            CognitiveOp::FindSimilar { case } => vec![case.clone()],
            CognitiveOp::CheckAssumption { assumption } => {
                vec![mm_core::ulid_string(assumption)]
            }
            CognitiveOp::Verify { claim } | CognitiveOp::CheckContradiction { claim } => {
                vec![mm_core::ulid_string(claim)]
            }
            CognitiveOp::Invert { goal } | CognitiveOp::Decompose { goal, .. } => {
                vec![mm_core::ulid_string(goal)]
            }
            CognitiveOp::Simulate { scenario } => vec![scenario.clone()],
            CognitiveOp::Critique { candidate }
            | CognitiveOp::Refine { candidate }
            | CognitiveOp::RedTeam { candidate }
            | CognitiveOp::Classify { candidate, .. } => vec![candidate.clone()],
            CognitiveOp::Search { over, .. } => vec![over.clone()],
            CognitiveOp::Simplify { solution } => vec![solution.clone()],
            CognitiveOp::Act { action } | CognitiveOp::Learn { lesson: action } => {
                vec![action.clone()]
            }
            CognitiveOp::Evaluate { outcome } => outcome
                .prediction_id
                .map(|id| vec![mm_core::ulid_string(&id)])
                .unwrap_or_default(),
            CognitiveOp::Clarify { question } => vec![question.clone()],
            CognitiveOp::SearchPrecedent { problem } => vec![problem.clone()],
            CognitiveOp::CheckConstraint { constraint } => vec![constraint.clone()],
            CognitiveOp::CheckCausality { cause, effect } => vec![cause.clone(), effect.clone()],
        };
        out.retain(|t| !t.is_empty());
        out.sort();
        out.dedup();
        out
    }

    /// True when the operation reaches outside the process.
    pub fn is_tool_bound(&self) -> bool {
        self.cost_class() == CostClass::Tool
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn ulid(n: u128) -> mm_core::Ulid {
        mm_core::Ulid::from_parts(1_700_000_000_000, n)
    }

    /// One operation of every variant, so the coverage assertions below are
    /// exhaustive by construction.
    fn every_op() -> Vec<CognitiveOp> {
        vec![
            CognitiveOp::Recall { query: "q".into() },
            CognitiveOp::Observe { source: "s".into() },
            CognitiveOp::FormHypothesis {
                proposition: "p".into(),
            },
            CognitiveOp::Compare {
                candidates: vec!["a".into(), "b".into()],
            },
            CognitiveOp::FindSimilar { case: "c".into() },
            CognitiveOp::CheckAssumption {
                assumption: ulid(1),
            },
            CognitiveOp::Verify { claim: ulid(2) },
            CognitiveOp::Invert { goal: ulid(3) },
            CognitiveOp::Simulate {
                scenario: "sc".into(),
            },
            CognitiveOp::Critique {
                candidate: "a".into(),
            },
            CognitiveOp::Refine {
                candidate: "a".into(),
            },
            CognitiveOp::RedTeam {
                candidate: "a".into(),
            },
            CognitiveOp::Search {
                over: "space".into(),
                budget: 3,
            },
            CognitiveOp::Simplify {
                solution: "a".into(),
            },
            CognitiveOp::Decide {
                candidates: vec!["a".into(), "b".into()],
            },
            CognitiveOp::Act {
                action: "do".into(),
            },
            CognitiveOp::Evaluate {
                outcome: mm_epistemic::Outcome::new(mm_epistemic::OutcomeStatus::Success),
            },
            CognitiveOp::Learn { lesson: "l".into() },
            CognitiveOp::Clarify {
                question: "?".into(),
            },
            CognitiveOp::Classify {
                candidate: "a".into(),
                taxonomy: "kind".into(),
            },
            CognitiveOp::Decompose {
                goal: ulid(4),
                into: vec!["x".into()],
            },
            CognitiveOp::SearchPrecedent {
                problem: "p".into(),
            },
            CognitiveOp::Predict {
                proposition: "p".into(),
            },
            CognitiveOp::CheckConstraint {
                constraint: "c".into(),
            },
            CognitiveOp::CheckContradiction { claim: ulid(5) },
            CognitiveOp::CheckCausality {
                cause: "a".into(),
                effect: "b".into(),
            },
            CognitiveOp::Synthesize {
                candidates: vec!["a".into(), "b".into()],
            },
        ]
    }

    #[test]
    fn op_classes_round_trip_and_are_unique() {
        let set: BTreeSet<&str> = OP_CLASSES.iter().map(|c| c.as_str()).collect();
        assert_eq!(set.len(), OP_CLASSES.len(), "duplicate class name");
        for class in OP_CLASSES {
            assert_eq!(OpClass::parse(class.as_str()), Some(class));
        }
        assert_eq!(OpClass::parse("frobnicate"), None);
        assert_eq!(OpClass::ALL.len(), 26);
        // The index is the bit a policy stores, so it must be the array position.
        for (position, class) in OP_CLASSES.iter().enumerate() {
            assert_eq!(class.index(), position, "{class} is out of order");
        }
    }

    #[test]
    fn every_op_maps_to_exactly_one_class_and_every_class_is_reachable() {
        let ops = every_op();
        assert_eq!(ops.len(), 27, "one op per variant");
        let reached: BTreeSet<OpClass> = ops.iter().map(CognitiveOp::class).collect();
        for class in OpClass::ALL {
            assert!(reached.contains(&class), "no op reaches {class}");
        }
    }

    #[test]
    fn the_cost_table_is_total_and_ordered() {
        assert_eq!(COST_TABLE.len(), CostClass::ALL.len());
        let mut previous = f64::NEG_INFINITY;
        for class in CostClass::ALL {
            let cost = class.cost();
            assert!(cost.is_finite(), "{class} is not finite");
            assert!(
                cost >= previous,
                "{class} is cheaper than the band below it"
            );
            assert_eq!(class.cost(), COST_TABLE[class.index()]);
            assert_eq!(CostClass::parse(class.as_str()), Some(class));
            previous = cost;
        }
        assert_eq!(CostClass::Free.cost(), 0.0);
        assert!(CostClass::Tool.cost() > CostClass::Expensive.cost());
    }

    #[test]
    fn ops_round_trip_through_json_with_an_op_tag() {
        for op in every_op() {
            let text = serde_json::to_string(&op).unwrap();
            assert!(
                text.contains(&format!("\"op\":\"{}\"", op.tag())),
                "the tag is missing from {text}"
            );
            let back: CognitiveOp = serde_json::from_str(&text).unwrap();
            assert_eq!(back, op);
        }
    }

    #[test]
    fn tool_bound_ops_are_the_ones_that_reach_outside() {
        assert!(CognitiveOp::Act { action: "x".into() }.is_tool_bound());
        assert!(CognitiveOp::Observe { source: "s".into() }.is_tool_bound());
        assert!(!CognitiveOp::Recall { query: "q".into() }.is_tool_bound());
    }

    #[test]
    fn targets_are_sorted_and_deduplicated() {
        let op = CognitiveOp::Compare {
            candidates: vec!["b".into(), "a".into(), "b".into()],
        };
        assert_eq!(op.targets(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(
            CognitiveOp::Clarify {
                question: String::new()
            }
            .targets(),
            Vec::<String>::new()
        );
    }
}
