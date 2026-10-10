//! Decision & Utility Theory types (`planning/logic2/decision_utility1.md` §2):
//! actions with probabilistic outcomes and utilities, risk measures,
//! constraints, pairwise preferences, and the decision verdict.

/// The risk measure applied when ranking actions. All are computed over the
/// utility *distribution* of an action's outcomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiskMeasure {
    /// The variance of the outcome utilities.
    Variance,
    /// The value at risk at level `alpha` (0 < alpha <= 1): the utility value
    /// bounding the worst `alpha` probability mass.
    Var { alpha: f64 },
    /// The conditional value at risk at level `alpha`: the *expected* utility
    /// within the worst `alpha` probability mass.
    Cvar { alpha: f64 },
}

impl RiskMeasure {
    pub fn name(&self) -> &'static str {
        match self {
            RiskMeasure::Variance => "Variance",
            RiskMeasure::Var { .. } => "Var",
            RiskMeasure::Cvar { .. } => "Cvar",
        }
    }
}

/// One probabilistic consequence of an action: the utility realized with the
/// given probability.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub name: String,
    pub probability: f64,
    pub utility: f64,
}

/// An action a decision maker may choose: a lottery over outcomes plus a
/// deterministic cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub name: String,
    pub outcomes: Vec<Outcome>,
    pub cost: f64,
}

impl Action {
    pub fn new(name: impl Into<String>, cost: f64) -> Self {
        Self {
            name: name.into(),
            outcomes: Vec::new(),
            cost,
        }
    }
}

/// A pairwise preference: `preferred` is (weakly) preferred over `over`.
#[derive(Debug, Clone, PartialEq)]
pub struct Preference {
    pub preferred: String,
    pub over: String,
}

/// A hard feasibility constraint on an action.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    pub name: String,
    pub action: String,
    /// The maximum acceptable cost.
    pub max_cost: f64,
}

/// The engine's input document: the decision problem.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionProblem {
    pub name: String,
    pub actions: Vec<Action>,
    pub preferences: Vec<Preference>,
    pub constraints: Vec<Constraint>,
    pub risk: RiskMeasure,
    // ── the Phase 2.x upgrade surface (all optional; empty defaults keep
    //    existing documents unchanged) ──
    pub mdps: Vec<Mdp>,
    pub bayesian: Vec<BayesianProblem>,
    pub multi_objective: Vec<MultiObjectiveProblem>,
}

/// The chosen action plus its decision-theoretic analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionVerdict {
    pub action: String,
    pub expected_utility: f64,
    pub risk: f64,
    /// `best_alternative_eu - chosen_eu` (0 when the chosen action is best).
    pub regret: f64,
    /// Every action ranked by expected utility, descending.
    pub ranking: Vec<(String, f64)>,
}

/// A structural problem with the decision problem input.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub rule: String,
    pub detail: String,
}

// ── Dynamic decisions: finite-horizon MDPs (design §23–24) ──────────────────

/// One transition of an MDP: from `state` under `action`, the process moves to
/// `next_state` with `probability`, collecting `reward`.
#[derive(Debug, Clone, PartialEq)]
pub struct MdpTransition {
    pub state: String,
    pub action: String,
    pub next_state: String,
    pub probability: f64,
    pub reward: f64,
}

/// A finite-horizon Markov decision process (design §23: `State_t → Action_t →
/// State_{t+1}`; the objective is `max E[Σ γ^t U(state, action)]`). Solved by
/// backward induction over the transition table.
#[derive(Debug, Clone, PartialEq)]
pub struct Mdp {
    pub name: String,
    pub states: Vec<String>,
    pub actions: Vec<String>,
    pub transitions: Vec<MdpTransition>,
    /// The discount factor `γ` applied per step.
    pub discount: f64,
    /// The finite horizon: the number of decision steps considered.
    pub horizon: u32,
}

/// The solution of an MDP by backward induction: the optimal value per state
/// and the greedy policy (best action per state).
#[derive(Debug, Clone, PartialEq)]
pub struct MdpVerdict {
    pub mdp: String,
    /// (state, optimal value) after the full horizon.
    pub values: Vec<(String, f64)>,
    /// (state, optimal action) — the policy.
    pub policy: Vec<(String, String)>,
}

// ── Bayesian decision theory (design §11–12) ────────────────────────────────

/// A lottery an action pays *in a given state*: the outcomes whose
/// probabilities and utilities apply when the state obtains.
#[derive(Debug, Clone, PartialEq)]
pub struct StateLottery {
    pub state: String,
    pub outcomes: Vec<Outcome>,
}

/// An action whose utility distribution depends on the unknown state.
#[derive(Debug, Clone, PartialEq)]
pub struct BayesianAction {
    pub name: String,
    pub cost: f64,
    /// The lottery in each state (empty outcomes → 0 utility in that state).
    pub state_lotteries: Vec<StateLottery>,
}

/// An experiment the decision maker may run before acting (design §12): each
/// observation has a likelihood in every state.
#[derive(Debug, Clone, PartialEq)]
pub struct Experiment {
    pub name: String,
    pub observations: Vec<String>,
    /// (observation, state, P(observation | state)).
    pub likelihoods: Vec<(String, String, f64)>,
}

/// A Bayesian decision problem: choose an action to maximize expected utility
/// under a belief distribution over states (design §11), optionally after
/// acquiring information (design §12).
#[derive(Debug, Clone, PartialEq)]
pub struct BayesianProblem {
    pub name: String,
    pub states: Vec<String>,
    /// (state, prior P(state)); must sum to 1.
    pub belief: Vec<(String, f64)>,
    pub actions: Vec<BayesianAction>,
    /// The experiment available before deciding; `None` = act now.
    pub experiment: Option<Experiment>,
}

/// The Bayesian decision outcome: the chosen action under the current belief,
/// plus the value of the available information (0 when acting now).
#[derive(Debug, Clone, PartialEq)]
pub struct BayesianVerdict {
    pub problem: String,
    pub chosen: String,
    pub expected_utility: f64,
    /// Every action ranked by expected utility, descending.
    pub ranking: Vec<(String, f64)>,
    /// `EU(best action after information) − EU(best action now)`.
    pub value_of_information: f64,
}

// ── Multi-objective decisions (design §32) ──────────────────────────────────

/// An action with a vector of objective values.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveAction {
    pub name: String,
    /// One value per objective (aligned with `objective_names`).
    pub objectives: Vec<f64>,
}

/// A multi-objective decision problem: the Pareto frontier over the actions
/// (design §32), optionally refined by a lexicographic objective order.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiObjectiveProblem {
    pub name: String,
    pub objective_names: Vec<String>,
    pub actions: Vec<ObjectiveAction>,
    /// Objective indices in lexicographic priority order (most important
    /// first); `None` = no single best is declared.
    pub lexicographic: Option<Vec<usize>>,
}

/// The Pareto analysis: the non-dominated actions, the dominators of each
/// dominated action, and the lexicographic best when an order is given.
#[derive(Debug, Clone, PartialEq)]
pub struct ParetoVerdict {
    pub problem: String,
    /// The non-dominated actions (sorted by name).
    pub frontier: Vec<String>,
    /// (dominated action, frontier actions that dominate it), sorted.
    pub dominated_by: Vec<(String, Vec<String>)>,
    /// The lexicographically best action, when an order is given.
    pub lexicographic_best: Option<String>,
}

/// The engine's output document.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionReport {
    pub problem: String,
    pub verdict: DecisionVerdict,
    pub mdp_verdicts: Vec<MdpVerdict>,
    pub bayesian_verdicts: Vec<BayesianVerdict>,
    pub pareto_verdicts: Vec<ParetoVerdict>,
    pub violations: Vec<Violation>,
}
