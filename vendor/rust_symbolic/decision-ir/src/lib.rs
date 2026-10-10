//! `decision-ir` — Decision & Utility Theory
//! (`planning/logic2/SUBSYSTEM_INTEGRATION_DESIGN.md` §2.2): decision problems
//! as first-class mathematical objects — actions as lotteries over outcomes,
//! expected utility, risk measures (variance, VaR, CVaR), constraints,
//! pairwise preferences, and decision selection with regret.

pub mod decision;
pub mod engine;
pub mod executable;
pub mod lowering;
pub mod rdf;

pub use decision::{
    Action, BayesianAction, BayesianProblem, BayesianVerdict, Constraint, DecisionProblem,
    DecisionReport, DecisionVerdict, Experiment, Mdp, MdpTransition, MdpVerdict,
    MultiObjectiveProblem, ObjectiveAction, Outcome, ParetoVerdict, Preference, RiskMeasure,
    StateLottery, Violation,
};
pub use engine::DecisionEngine;
pub use lowering::to_logic_model;
pub use rdf::{data, schema, DECISION_DATA, DECISION_NS};
