//! The decision engine (`planning/logic2/decision_utility1.md` §2, Backend):
//! expected utility, risk measures, constraint feasibility, preference
//! consistency, and decision selection with regret. Deterministic: all
//! quantities are closed-form over the action's outcome distribution.

use crate::decision::{
    Action, BayesianAction, BayesianProblem, BayesianVerdict, Constraint, DecisionProblem,
    DecisionReport, DecisionVerdict, Mdp, MdpVerdict, MultiObjectiveProblem, ObjectiveAction,
    Outcome, ParetoVerdict, RiskMeasure, Violation,
};

/// The engine. Stateless: all analyses are pure functions of the problem.
#[derive(Debug, Default, Clone, Copy)]
pub struct DecisionEngine;

impl DecisionEngine {
    pub fn new() -> Self {
        Self
    }

    /// The expected utility of an action: Σ p·u − cost.
    pub fn expected_utility(action: &Action) -> f64 {
        let gross: f64 = action
            .outcomes
            .iter()
            .map(|o| o.probability * o.utility)
            .sum();
        gross - action.cost
    }

    /// The risk of an action under the given measure.
    pub fn risk(action: &Action, measure: RiskMeasure) -> f64 {
        match measure {
            RiskMeasure::Variance => {
                let eu = Self::expected_utility(action);
                let gross = eu + action.cost;
                action
                    .outcomes
                    .iter()
                    .map(|o| o.probability * (o.utility - gross).powi(2))
                    .sum()
            }
            RiskMeasure::Var { alpha } | RiskMeasure::Cvar { alpha } => {
                // Sort outcomes worst-to-best by utility.
                let mut sorted: Vec<&Outcome> = action.outcomes.iter().collect();
                sorted.sort_by(|a, b| a.utility.total_cmp(&b.utility));
                let alpha = alpha.clamp(0.0, 1.0);
                match measure {
                    RiskMeasure::Var { .. } => {
                        // The utility bounding the worst `alpha` mass.
                        let mut cum = 0.0;
                        for o in &sorted {
                            cum += o.probability;
                            if cum >= alpha {
                                return o.utility;
                            }
                        }
                        sorted.last().map(|o| o.utility).unwrap_or(0.0)
                    }
                    _ => {
                        // Expected utility within the worst `alpha` mass.
                        let mut cum = 0.0;
                        let mut acc = 0.0;
                        for o in &sorted {
                            let take = (alpha - cum).min(o.probability).max(0.0);
                            acc += take * o.utility;
                            cum += take;
                            if cum >= alpha {
                                break;
                            }
                        }
                        if cum > 0.0 {
                            acc / cum
                        } else {
                            0.0
                        }
                    }
                }
            }
        }
    }

    /// Whether an action meets every constraint that names it.
    pub fn feasible(action: &Action, constraints: &[Constraint]) -> bool {
        constraints
            .iter()
            .filter(|c| c.action == action.name)
            .all(|c| action.cost <= c.max_cost)
    }

    /// The risk-adjusted score used for ranking: `EU − risk`.
    pub fn risk_adjusted(action: &Action, measure: RiskMeasure) -> f64 {
        Self::expected_utility(action) - Self::risk(action, measure)
    }

    /// All actions ranked by expected utility, descending.
    pub fn ranking(problem: &DecisionProblem) -> Vec<(String, f64)> {
        let mut ranking: Vec<(String, f64)> = problem
            .actions
            .iter()
            .map(|a| (a.name.clone(), Self::expected_utility(a)))
            .collect();
        ranking.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranking
    }

    /// Detect preference cycles (a strict preference must be acyclic).
    fn preference_violations(problem: &DecisionProblem) -> Vec<Violation> {
        let mut violations = Vec::new();
        let names: Vec<String> = problem.actions.iter().map(|a| a.name.clone()).collect();
        let mut adj: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for p in &problem.preferences {
            adj.entry(p.preferred.clone())
                .or_default()
                .push(p.over.clone());
        }
        fn has_cycle(
            node: &str,
            adj: &std::collections::HashMap<String, Vec<String>>,
            visiting: &mut Vec<String>,
            visited: &mut Vec<String>,
        ) -> bool {
            if visiting.iter().any(|n| n == node) {
                return true;
            }
            if visited.iter().any(|n| n == node) {
                return false;
            }
            visiting.push(node.to_string());
            if let Some(nexts) = adj.get(node) {
                for n in nexts {
                    if has_cycle(n, adj, visiting, visited) {
                        return true;
                    }
                }
            }
            visiting.pop();
            visited.push(node.to_string());
            false
        }
        for name in &names {
            if has_cycle(name, &adj, &mut Vec::new(), &mut Vec::new()) {
                violations.push(Violation {
                    rule: "acyclic-preferences".into(),
                    detail: format!(
                        "preference relation over `{name}` contains a cycle; expected utility is still reported"
                    ),
                });
                break;
            }
        }
        violations
    }

    /// Solve a finite-horizon MDP by backward induction (design §23–24):
    /// `V_{t+1}(s) = max_a Σ_s' P(s'|s,a)·(r + γ·V_t(s'))`, the greedy policy
    /// after the full horizon. The value of an unavailable (state, action)
    /// pair is 0.
    pub fn solve_mdp(mdp: &Mdp) -> MdpVerdict {
        let mut value: std::collections::HashMap<String, f64> =
            mdp.states.iter().map(|s| (s.clone(), 0.0)).collect();
        for _ in 0..mdp.horizon {
            let mut next: std::collections::HashMap<String, f64> = value.clone();
            for state in &mdp.states {
                let mut best = f64::NEG_INFINITY;
                for action in &mdp.actions {
                    let q: f64 = mdp
                        .transitions
                        .iter()
                        .filter(|t| &t.state == state && &t.action == action)
                        .map(|t| {
                            t.probability
                                * (t.reward
                                    + mdp.discount
                                        * value.get(&t.next_state).copied().unwrap_or(0.0))
                        })
                        .sum();
                    best = best.max(q);
                }
                if best.is_finite() {
                    next.insert(state.clone(), best);
                }
            }
            value = next;
        }
        let mut values: Vec<(String, f64)> = mdp
            .states
            .iter()
            .map(|s| (s.clone(), value.get(s).copied().unwrap_or(0.0)))
            .collect();
        values.sort_by(|a, b| a.0.cmp(&b.0));
        let mut policy = Vec::new();
        for state in &mdp.states {
            let mut best_action: Option<&String> = None;
            let mut best_q = f64::NEG_INFINITY;
            for action in &mdp.actions {
                let q: f64 = mdp
                    .transitions
                    .iter()
                    .filter(|t| &t.state == state && &t.action == action)
                    .map(|t| {
                        t.probability
                            * (t.reward
                                + mdp.discount * value.get(&t.next_state).copied().unwrap_or(0.0))
                    })
                    .sum();
                if q > best_q {
                    best_q = q;
                    best_action = Some(action);
                }
            }
            if let Some(action) = best_action {
                policy.push((state.clone(), action.clone()));
            }
        }
        MdpVerdict {
            mdp: mdp.name.clone(),
            values,
            policy,
        }
    }

    /// The expected utility of a Bayesian action under a belief over states
    /// (design §11): `Σ_s P(s)·(Σ_o p_o·u_o) − cost`.
    pub fn bayesian_expected_utility(action: &BayesianAction, belief: &[(String, f64)]) -> f64 {
        let mut gross = 0.0;
        for (state, prob) in belief {
            let lottery = action
                .state_lotteries
                .iter()
                .find(|l| &l.state == state)
                .map(|l| {
                    l.outcomes
                        .iter()
                        .map(|o| o.probability * o.utility)
                        .sum::<f64>()
                })
                .unwrap_or(0.0);
            gross += prob * lottery;
        }
        gross - action.cost
    }

    /// The value of information (design §12): the expected improvement in the
    /// best action's utility when the decision is postponed until after an
    /// experiment, minus the best expected utility acting now.
    pub fn value_of_information(problem: &BayesianProblem) -> f64 {
        let Some(experiment) = &problem.experiment else {
            return 0.0;
        };
        let act_now = problem
            .actions
            .iter()
            .map(|a| Self::bayesian_expected_utility(a, &problem.belief))
            .fold(f64::NEG_INFINITY, f64::max);
        let mut expected_after = 0.0;
        for observation in &experiment.observations {
            // Posterior over states after this observation.
            let mut posterior: Vec<(String, f64)> = Vec::new();
            let mut evidence = 0.0;
            for (state, prior) in &problem.belief {
                let likelihood = experiment
                    .likelihoods
                    .iter()
                    .find(|(o, s, _)| o == observation && s == state)
                    .map(|(_, _, p)| *p)
                    .unwrap_or(0.0);
                evidence += prior * likelihood;
                posterior.push((state.clone(), prior * likelihood));
            }
            if evidence <= 0.0 {
                continue;
            }
            for (_, p) in posterior.iter_mut() {
                *p /= evidence;
            }
            let best_after = problem
                .actions
                .iter()
                .map(|a| Self::bayesian_expected_utility(a, &posterior))
                .fold(f64::NEG_INFINITY, f64::max);
            expected_after += evidence * best_after;
        }
        (expected_after - act_now).max(0.0)
    }

    /// The Pareto frontier over the actions (design §32): `a` dominates `b`
    /// when every objective of `a` is ≥ `b`'s and at least one is strictly
    /// greater. Returns the non-dominated actions and each dominated action's
    /// dominators.
    pub fn pareto(problem: &MultiObjectiveProblem) -> ParetoVerdict {
        let dominates = |a: &ObjectiveAction, b: &ObjectiveAction| -> bool {
            let mut any_strict = false;
            for (x, y) in a.objectives.iter().zip(b.objectives.iter()) {
                if x < y {
                    return false;
                }
                if x > y {
                    any_strict = true;
                }
            }
            any_strict
        };
        let mut frontier = Vec::new();
        let mut dominated_by: Vec<(String, Vec<String>)> = Vec::new();
        for a in &problem.actions {
            let mut dominators: Vec<String> = problem
                .actions
                .iter()
                .filter(|b| b.name != a.name && dominates(b, a))
                .map(|b| b.name.clone())
                .collect();
            if dominators.is_empty() {
                frontier.push(a.name.clone());
            } else {
                dominators.sort();
                dominated_by.push((a.name.clone(), dominators));
            }
        }
        frontier.sort();
        dominated_by.sort_by(|a, b| a.0.cmp(&b.0));
        let lexicographic_best = problem.lexicographic.as_ref().map(|order| {
            problem
                .actions
                .iter()
                .max_by(|a, b| {
                    for &i in order {
                        let av = a.objectives.get(i).copied().unwrap_or(f64::NEG_INFINITY);
                        let bv = b.objectives.get(i).copied().unwrap_or(f64::NEG_INFINITY);
                        if av != bv {
                            return av.total_cmp(&bv);
                        }
                    }
                    std::cmp::Ordering::Equal
                })
                .map(|a| a.name.clone())
                .unwrap_or_default()
        });
        ParetoVerdict {
            problem: problem.name.clone(),
            frontier,
            dominated_by,
            lexicographic_best,
        }
    }

    /// Analyze a decision problem: pick the feasible action maximizing
    /// expected utility (ties broken by lower risk), and report regret.
    pub fn analyze(problem: &DecisionProblem) -> DecisionReport {
        let mut violations = Self::preference_violations(problem);
        let ranking = Self::ranking(problem);

        // Feasible actions only.
        let mut candidates: Vec<&Action> = problem
            .actions
            .iter()
            .filter(|a| Self::feasible(a, &problem.constraints))
            .collect();
        if candidates.is_empty() {
            violations.push(Violation {
                rule: "no-feasible-action".into(),
                detail: "every action violates at least one constraint".into(),
            });
            candidates = problem.actions.iter().collect();
        }

        // Rank feasible candidates by EU, then lower risk.
        candidates.sort_by(|a, b| {
            Self::expected_utility(b)
                .total_cmp(&Self::expected_utility(a))
                .then_with(|| Self::risk(a, problem.risk).total_cmp(&Self::risk(b, problem.risk)))
        });
        let chosen = candidates[0];

        let eu = Self::expected_utility(chosen);
        let risk = Self::risk(chosen, problem.risk);
        let best_alternative = ranking
            .iter()
            .find(|(name, _)| *name != chosen.name)
            .map(|(_, eu)| *eu)
            .unwrap_or(eu);
        let regret = (best_alternative - eu).max(0.0);

        let mdp_verdicts: Vec<MdpVerdict> = problem.mdps.iter().map(Self::solve_mdp).collect();
        let bayesian_verdicts: Vec<BayesianVerdict> = problem
            .bayesian
            .iter()
            .map(|b| {
                let mut ranking: Vec<(String, f64)> = b
                    .actions
                    .iter()
                    .map(|a| {
                        (
                            a.name.clone(),
                            Self::bayesian_expected_utility(a, &b.belief),
                        )
                    })
                    .collect();
                ranking.sort_by(|a, b| b.1.total_cmp(&a.1));
                let chosen = ranking
                    .first()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default();
                let expected_utility = ranking.first().map(|(_, eu)| *eu).unwrap_or(0.0);
                BayesianVerdict {
                    problem: b.name.clone(),
                    chosen,
                    expected_utility,
                    value_of_information: Self::value_of_information(b),
                    ranking,
                }
            })
            .collect();
        let pareto_verdicts: Vec<ParetoVerdict> =
            problem.multi_objective.iter().map(Self::pareto).collect();

        DecisionReport {
            problem: problem.name.clone(),
            verdict: DecisionVerdict {
                action: chosen.name.clone(),
                expected_utility: eu,
                risk,
                regret,
                ranking,
            },
            mdp_verdicts,
            bayesian_verdicts,
            pareto_verdicts,
            violations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{
        BayesianAction, BayesianProblem, Experiment, Mdp, MdpTransition, MultiObjectiveProblem,
        ObjectiveAction, Preference, StateLottery,
    };

    fn coin_flip(win: f64, lose: f64) -> Action {
        Action {
            name: "bet".into(),
            outcomes: vec![
                Outcome {
                    name: "win".into(),
                    probability: 0.5,
                    utility: win,
                },
                Outcome {
                    name: "lose".into(),
                    probability: 0.5,
                    utility: lose,
                },
            ],
            cost: 0.0,
        }
    }

    #[test]
    fn expected_utility_is_probability_weighted() {
        let a = coin_flip(10.0, -2.0);
        let eu = DecisionEngine::expected_utility(&a);
        assert!((eu - 4.0).abs() < 1e-9);
    }

    #[test]
    fn variance_risk_of_fair_coin() {
        let a = coin_flip(10.0, -2.0);
        let var = DecisionEngine::risk(&a, RiskMeasure::Variance);
        // E[(u - 4)^2] = 0.5*36 + 0.5*36 = 36
        assert!((var - 36.0).abs() < 1e-9);
    }

    #[test]
    fn cvar_is_tail_expectation() {
        // Outcomes: -10 (p=0.25), 0 (p=0.25), 10 (p=0.5).
        let a = Action {
            name: "a".into(),
            outcomes: vec![
                Outcome {
                    name: "x".into(),
                    probability: 0.25,
                    utility: -10.0,
                },
                Outcome {
                    name: "y".into(),
                    probability: 0.25,
                    utility: 0.0,
                },
                Outcome {
                    name: "z".into(),
                    probability: 0.5,
                    utility: 10.0,
                },
            ],
            cost: 0.0,
        };
        let cvar = DecisionEngine::risk(&a, RiskMeasure::Cvar { alpha: 0.5 });
        // Worst 50% mass: -10 (0.25) and 0 (0.25) → mean -5.
        assert!((cvar - (-5.0)).abs() < 1e-9);
        let var = DecisionEngine::risk(&a, RiskMeasure::Var { alpha: 0.5 });
        // The utility bounding the worst 50%: after -10 (cum 0.25) + 0 (cum 0.5) → 0.
        assert!((var - 0.0).abs() < 1e-9);
    }

    #[test]
    fn constraint_removes_expensive_action() {
        let problem = DecisionProblem {
            name: "p".into(),
            actions: vec![
                Action {
                    name: "cheap".into(),
                    outcomes: vec![],
                    cost: 1.0,
                },
                Action {
                    name: "pricey".into(),
                    outcomes: vec![],
                    cost: 100.0,
                },
            ],
            mdps: vec![],
            bayesian: vec![],
            multi_objective: vec![],
            preferences: vec![],
            constraints: vec![Constraint {
                name: "budget".into(),
                action: "pricey".into(),
                max_cost: 50.0,
            }],
            risk: RiskMeasure::Variance,
        };
        let report = DecisionEngine::analyze(&problem);
        assert_eq!(report.verdict.action, "cheap");
        assert!(report.violations.is_empty());
    }

    #[test]
    fn preference_cycle_is_reported() {
        let problem = DecisionProblem {
            name: "p".into(),
            actions: vec![
                Action {
                    name: "a".into(),
                    outcomes: vec![],
                    cost: 0.0,
                },
                Action {
                    name: "b".into(),
                    outcomes: vec![],
                    cost: 0.0,
                },
            ],
            preferences: vec![
                Preference {
                    preferred: "a".into(),
                    over: "b".into(),
                },
                Preference {
                    preferred: "b".into(),
                    over: "a".into(),
                },
            ],
            constraints: vec![],
            risk: RiskMeasure::Variance,
            mdps: vec![],
            bayesian: vec![],
            multi_objective: vec![],
        };
        let report = DecisionEngine::analyze(&problem);
        assert!(report
            .violations
            .iter()
            .any(|v| v.rule == "acyclic-preferences"));
    }

    #[test]
    fn regret_is_zero_for_best_action() {
        let problem = DecisionProblem {
            name: "p".into(),
            actions: vec![coin_flip(10.0, -2.0), coin_flip(5.0, 0.0)],
            preferences: vec![],
            constraints: vec![],
            risk: RiskMeasure::Variance,
            mdps: vec![],
            bayesian: vec![],
            multi_objective: vec![],
        };
        let report = DecisionEngine::analyze(&problem);
        assert_eq!(report.verdict.action, "bet");
        assert!((report.verdict.regret - 0.0).abs() < 1e-9);
    }

    #[test]
    fn mdp_backward_induction_solves_two_step_chain() {
        // s0 -a-> s1 (reward 2, p=1); only action; V(s0) = 2 + γ·V(s1) = 2.
        let mdp = Mdp {
            name: "m".into(),
            states: vec!["s0".into(), "s1".into()],
            actions: vec!["a".into()],
            transitions: vec![MdpTransition {
                state: "s0".into(),
                action: "a".into(),
                next_state: "s1".into(),
                probability: 1.0,
                reward: 2.0,
            }],
            discount: 0.9,
            horizon: 2,
        };
        let verdict = DecisionEngine::solve_mdp(&mdp);
        let v0 = verdict
            .values
            .iter()
            .find(|(s, _)| s == "s0")
            .map(|(_, v)| *v)
            .unwrap();
        assert!((v0 - 2.0).abs() < 1e-9);
        assert_eq!(verdict.policy[0], ("s0".to_string(), "a".to_string()));
    }

    #[test]
    fn bayesian_eu_weights_lotteries_by_state_belief() {
        // State g: utility 10 (p=1). State b: utility 0. Prior 0.5/0.5 → EU 5.
        let action = BayesianAction {
            name: "bet".into(),
            cost: 0.0,
            state_lotteries: vec![StateLottery {
                state: "g".into(),
                outcomes: vec![Outcome {
                    name: "win".into(),
                    probability: 1.0,
                    utility: 10.0,
                }],
            }],
        };
        let belief = vec![("g".to_string(), 0.5), ("b".to_string(), 0.5)];
        let eu = DecisionEngine::bayesian_expected_utility(&action, &belief);
        assert!((eu - 5.0).abs() < 1e-9);
    }

    #[test]
    fn value_of_information_is_nonnegative() {
        // A perfect experiment on the state: observing g resolves the state.
        let problem = BayesianProblem {
            name: "b".into(),
            states: vec!["g".into(), "b".into()],
            belief: vec![("g".into(), 0.5), ("b".into(), 0.5)],
            actions: vec![BayesianAction {
                name: "bet".into(),
                cost: 0.0,
                state_lotteries: vec![StateLottery {
                    state: "g".into(),
                    outcomes: vec![Outcome {
                        name: "win".into(),
                        probability: 1.0,
                        utility: 10.0,
                    }],
                }],
            }],
            experiment: Some(Experiment {
                name: "e".into(),
                observations: vec!["g".into(), "b".into()],
                likelihoods: vec![("g".into(), "g".into(), 1.0), ("b".into(), "b".into(), 1.0)],
            }),
        };
        let voi = DecisionEngine::value_of_information(&problem);
        assert!(voi >= 0.0);
    }

    #[test]
    fn pareto_frontier_keeps_non_dominated_actions() {
        // x=(1,2), y=(2,1), z=(0,0). x and y dominate z; neither dominates
        // the other.
        let problem = MultiObjectiveProblem {
            name: "mo".into(),
            objective_names: vec!["a".into(), "b".into()],
            actions: vec![
                ObjectiveAction {
                    name: "x".into(),
                    objectives: vec![1.0, 2.0],
                },
                ObjectiveAction {
                    name: "y".into(),
                    objectives: vec![2.0, 1.0],
                },
                ObjectiveAction {
                    name: "z".into(),
                    objectives: vec![0.0, 0.0],
                },
            ],
            lexicographic: None,
        };
        let verdict = DecisionEngine::pareto(&problem);
        assert_eq!(verdict.frontier, vec!["x".to_string(), "y".to_string()]);
        assert_eq!(verdict.dominated_by.len(), 1);
        assert_eq!(verdict.dominated_by[0].0, "z");
    }
}
