//! The policy genome: deterministic hill-climbing over behaviour fragments.
//!
//! The plan's risk table lists "genome evolution burns budget / drifts" and asks
//! for two things: a **hard budget** and a rule that a run **retains only fitness
//! gains**. Both are enforced here, and neither depends on a random number
//! generator:
//!
//! * **Nothing probabilistic.** [`mutate`] is a pure function of
//!   `(weights, generation, candidate)`, so the same run over the same evaluation
//!   set produces the same versions with the same fitnesses. A genome whose search
//!   cannot be replayed cannot be held to a gain.
//! * **Fitness is measured, never asserted.** Every candidate is evaluated by
//!   recording real [`Outcome`]s through
//!   [`LibraryManager::record_fitness`](crate::manager::LibraryManager::record_fitness)
//!   and then reading the running mean back out of the store. The report carries
//!   the value the store holds, not a number the search computed for itself.
//! * **Immutable candidates, flagged retention.** Every candidate becomes a new
//!   immutable policy version ([`LibraryManager::new_policy_version`](crate::manager::LibraryManager::new_policy_version));
//!   `retained` says whether it beat its parent. A rejected candidate is kept, and
//!   marked, because "the change that did not help" is evidence too.
//! * **A hard budget.** [`EvoBudget::max_versions`] caps how many versions one run
//!   may create. When it is reached the run stops and reports what it reached,
//!   rather than evolving until someone notices.

use std::path::Path;

use mm_core::ActivationCondition;
use mm_epistemic::{Outcome, OutcomeStatus};
use mm_log::{codes, Level, LogRecord};
use serde::Deserialize;

use crate::entry::iri;
use crate::error::{LibraryError, Result};
use crate::manager::{LibraryManager, TARGET};
use crate::policy::{PolicyBehavior, PolicyDelta, Scope};

/// The three axes a behaviour fragment weighs, in the order the report names them.
pub const AXES: [&str; 3] = ["simplicity", "speed", "safety"];

/// The floor and ceiling a single weight may take.
pub const MIN_WEIGHT: f64 = 0.05;
/// The floor and ceiling a single weight may take.
pub const MAX_WEIGHT: f64 = 0.9;
/// The perturbation applied to one axis, before generation scaling.
pub const BASE_STEP: f64 = 0.05;

/// How hard one evolution run may work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct EvoBudget {
    /// How many generations to run.
    pub generations: u32,
    /// How many candidates to try per generation.
    pub candidates_per_generation: u32,
    /// The hard cap on immutable versions one run may create.
    pub max_versions: u32,
}

impl EvoBudget {
    /// A small, honest default.
    pub fn default_budget() -> Self {
        EvoBudget {
            generations: 3,
            candidates_per_generation: 3,
            max_versions: 12,
        }
    }

    /// Load a budget from a TOML file.
    ///
    /// A file that names zero generations is a refusal rather than a no-op: a run
    /// that does nothing while reporting success is worse than one that fails.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| LibraryError::Config(format!("cannot read {}: {e}", path.display())))?;
        let budget: EvoBudget = toml::from_str(&raw)
            .map_err(|e| LibraryError::Config(format!("{}: {e}", path.display())))?;
        if budget.generations == 0 {
            return Err(LibraryError::validation(
                "generations",
                "a budget of zero generations would report a run that never happened",
            ));
        }
        if budget.max_versions == 0 {
            return Err(LibraryError::validation(
                "max_versions",
                "a budget of zero versions cannot evolve anything",
            ));
        }
        Ok(budget)
    }
}

/// One labelled evaluation case: the weighting that would have been right.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EvalCase {
    /// The ideal weighting.
    pub ideal: [f64; 3],
    /// Which axis the ideal weighting favours.
    pub expected: String,
}

/// The evaluation set a run scores against.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EvaluationSet {
    /// The cases, in file order.
    pub cases: Vec<EvalCase>,
}

impl EvaluationSet {
    /// Load from a JSONL file, one case per line.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| LibraryError::Config(format!("cannot read {}: {e}", path.display())))?;
        Self::from_jsonl(&raw)
    }

    /// Parse a JSONL document.
    ///
    /// Every line is checked, and a bad line fails the load with its number: an
    /// evolution run that silently scored against fewer cases than the file lists
    /// would make its own gains unfalsifiable.
    pub fn from_jsonl(text: &str) -> Result<Self> {
        let mut cases = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let case: EvalCase = serde_json::from_str(line).map_err(|e| {
                LibraryError::validation("eval_jsonl", format!("line {}: {e}", index + 1))
            })?;
            if !AXES.contains(&case.expected.as_str()) {
                return Err(LibraryError::validation(
                    "eval_jsonl",
                    format!(
                        "line {}: expected {:?} is not one of {AXES:?}",
                        index + 1,
                        case.expected
                    ),
                ));
            }
            cases.push(case);
        }
        if cases.is_empty() {
            return Err(LibraryError::validation(
                "eval_jsonl",
                "an evaluation set with no cases cannot score anything",
            ));
        }
        Ok(EvaluationSet { cases })
    }
}

/// One candidate version, as the run reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The version that was created.
    pub version: u32,
    /// What the mutation changed.
    pub mutation: String,
    /// The mean utility the store holds for it.
    pub fitness: f64,
    /// True when it strictly beat its parent.
    pub retained: bool,
}

/// One generation's result.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationReport {
    /// The generation number.
    pub generation: u32,
    /// Every candidate tried.
    pub candidates: Vec<Candidate>,
    /// The best fitness seen this generation.
    pub best_fitness: f64,
    /// How many candidates were retained.
    pub retained: u32,
}

/// The whole run.
#[derive(Debug, Clone, PartialEq)]
pub struct EvolutionReport {
    /// The policy's name.
    pub name: String,
    /// The generations that ran, in order.
    pub generations: Vec<GenerationReport>,
    /// The version with the best fitness, when any was retained.
    pub best_version: Option<u32>,
    /// The best fitness reached.
    pub best_fitness: f64,
    /// How many immutable versions the run created.
    pub versions_created: u32,
}

/// Perturb one axis, deterministically.
///
/// The axis cycles with `generation + candidate`, the sign alternates, and the
/// step grows slowly with the generation, so a run explores all three axes and
/// then re-examines them. The result is renormalized and rounded to six decimals,
/// because an unrounded weight would make the golden output depend on the
/// floating-point path rather than on the search.
pub fn mutate(weights: [f64; 3], generation: u32, candidate: u32) -> ([f64; 3], String) {
    let axis = ((generation + candidate) % 3) as usize;
    let sign = if (generation + candidate).is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let step = BASE_STEP * (1.0 + f64::from(generation));
    let mut out = weights;
    out[axis] = (out[axis] + sign * step).clamp(MIN_WEIGHT, MAX_WEIGHT);
    let total: f64 = out.iter().sum();
    for value in &mut out {
        *value = round6(*value / total);
    }
    // Renormalizing after rounding can lose or gain a micro-unit; the largest
    // component absorbs it so the vector sums to exactly 1.0 as far as f64 is
    // concerned.
    let residual = round6(1.0 - out.iter().sum::<f64>());
    let largest = out
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(index, _)| index)
        .unwrap_or(0);
    out[largest] = round6(out[largest] + residual);
    let mutation = format!("{}{:+.2}", AXES[axis], sign * step);
    (out, mutation)
}

/// Which axis a weighting favours. Ties go to the earlier axis.
pub fn favored(weights: [f64; 3]) -> &'static str {
    let mut best = 0;
    for index in 1..3 {
        if weights[index] > weights[best] {
            best = index;
        }
    }
    AXES[best]
}

/// Score a weighting against the evaluation set: the fraction of cases whose
/// favoured axis it gets right.
pub fn score(weights: [f64; 3], evals: &EvaluationSet) -> f64 {
    if evals.cases.is_empty() {
        return 0.0;
    }
    let right = evals
        .cases
        .iter()
        .filter(|case| favored(weights) == case.expected)
        .count();
    right as f64 / evals.cases.len() as f64
}

/// Run the evolution loop.
pub async fn evolve(
    manager: &LibraryManager,
    name: &str,
    budget: &EvoBudget,
    evals: &EvaluationSet,
) -> Result<EvolutionReport> {
    let store = manager.store();
    let head = iri::head_of(&iri::policy(name, 0).into_string())
        .ok_or_else(|| LibraryError::validation("name", format!("{name:?} has no head IRI")))?;
    store.ensure_policy(&head, name).await?;

    let history = store.policy_history(&head).await?;
    let mut parent_version = store.head_version(&head).await.unwrap_or(0).max(0);
    let mut parent_weights = history
        .last()
        .and_then(|row| PolicyBehavior::new(&row.behavior).ok())
        .and_then(|behavior| behavior.weights())
        .unwrap_or([0.34, 0.33, 0.33]);
    let mut parent_fitness = match history.last() {
        Some(row) => store
            .fitness_of(&head, row.version)
            .await?
            .map(|(_, _, mean)| mean)
            .unwrap_or(0.0),
        None => 0.0,
    };

    manager
        .logger()
        .emit(
            LogRecord::new(Level::Info, codes::GENOME_EVOLVE_START, TARGET)
                .with_field("name", name.to_string())
                .with_field("generations", budget.generations)
                .with_field(
                    "candidates_per_generation",
                    budget.candidates_per_generation,
                )
                .with_field("max_versions", budget.max_versions)
                .with_field("budget_versions", budget.max_versions),
        )
        .await?;

    let mut report = EvolutionReport {
        name: name.to_string(),
        generations: Vec::new(),
        best_version: history.last().map(|row| row.version as u32),
        best_fitness: parent_fitness,
        versions_created: 0,
    };
    let mut exhausted = false;

    for generation in 0..budget.generations {
        if exhausted {
            break;
        }
        let mut candidates: Vec<Candidate> = Vec::new();
        let base_weights = parent_weights;
        for candidate in 0..budget.candidates_per_generation {
            if report.versions_created >= budget.max_versions {
                exhausted = true;
                break;
            }
            let (weights, mutation) = mutate(base_weights, generation, candidate);
            let delta = PolicyDelta {
                behavior: PolicyBehavior::from_weights(weights),
                scope: Scope::Global,
                activation: ActivationCondition::new("uncertain strategy with high stakes")?,
                confidence: 0.5,
                reason: format!("generation {generation} candidate {candidate}: {mutation}"),
            };
            let policy = manager.new_policy_version(name, delta, &[]).await?;
            report.versions_created += 1;

            // Every case is a real outcome, recorded through the one fitness path.
            for case in &evals.cases {
                let status = if favored(weights) == case.expected {
                    OutcomeStatus::Success
                } else {
                    OutcomeStatus::Failure
                };
                manager
                    // `as_str`, not `to_string`: a `NamedNode`'s `Display` is the
                    // bracketed Turtle form, and the store keys on the bare IRI.
                    .record_fitness(policy.iri.as_str(), policy.version, &Outcome::new(status))
                    .await?;
            }

            let (_, _, mean) = store
                .fitness_of(&head, i64::from(policy.version))
                .await?
                .unwrap_or((0, 0, 0.0));
            let retained = mean > parent_fitness;
            store
                .insert_population_row(
                    i64::from(generation),
                    &head,
                    i64::from(policy.version),
                    (parent_version > 0).then_some(parent_version),
                    None,
                    Some(&mutation),
                    Some(mean),
                    retained,
                )
                .await?;
            candidates.push(Candidate {
                version: policy.version,
                mutation,
                fitness: mean,
                retained,
            });

            if retained {
                parent_fitness = mean;
                parent_weights = weights;
                parent_version = i64::from(policy.version);
                report.best_version = Some(policy.version);
                report.best_fitness = mean;
            }
        }

        if candidates.is_empty() {
            break;
        }
        let best_fitness = candidates
            .iter()
            .map(|c| c.fitness)
            .fold(f64::NEG_INFINITY, f64::max);
        let retained = store.retained_in_generation(i64::from(generation)).await? as u32;
        manager
            .logger()
            .emit(
                LogRecord::new(Level::Info, codes::GENOME_EVOLVE_GENERATION, TARGET)
                    .with_field("name", name.to_string())
                    .with_field("generation", generation)
                    .with_field("retained", retained)
                    .with_field("best_fitness", best_fitness),
            )
            .await?;
        report.generations.push(GenerationReport {
            generation,
            candidates,
            best_fitness,
            retained,
        });
    }

    let best_iri = report
        .best_version
        .map(|version| iri::policy(name, version).into_string())
        .unwrap_or_else(|| head.clone());
    manager
        .logger()
        .emit(
            LogRecord::new(Level::Info, codes::GENOME_EVOLVE_END, TARGET)
                .with_field("name", name.to_string())
                .with_field("best_iri", best_iri)
                .with_field("generations", report.generations.len() as u32)
                .with_field("best_fitness", report.best_fitness),
        )
        .await?;
    Ok(report)
}

/// The policy head IRI of a name, for callers that need to ask about it.
pub fn head_of(name: &str) -> String {
    iri::head_of(&iri::policy(name, 0).into_string()).unwrap_or_default()
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evals() -> EvaluationSet {
        // Deliberately unbalanced: one case of each axis would score every
        // weighting exactly 1/3 and prove nothing about the arithmetic.
        EvaluationSet::from_jsonl(
            r#"{"ideal": [0.5, 0.3, 0.2], "expected": "simplicity"}
{"ideal": [0.4, 0.35, 0.25], "expected": "simplicity"}
{"ideal": [0.2, 0.6, 0.2], "expected": "speed"}"#,
        )
        .unwrap()
    }

    #[test]
    fn mutation_is_pure_and_the_weights_still_sum_to_one() {
        let start = [0.34, 0.33, 0.33];
        let (first, mutation) = mutate(start, 1, 2);
        let (again, same_mutation) = mutate(start, 1, 2);
        assert_eq!(first, again, "the same inputs must give the same weights");
        assert_eq!(mutation, same_mutation);
        assert!(
            (first.iter().sum::<f64>() - 1.0).abs() < 1e-9,
            "weights must sum to 1.0, got {first:?}"
        );
        for value in first {
            assert!(
                (MIN_WEIGHT..=MAX_WEIGHT).contains(&value),
                "a weight left its range: {value}"
            );
        }
    }

    #[test]
    fn mutation_names_the_axis_it_changed() {
        let (_, mutation) = mutate([0.34, 0.33, 0.33], 0, 0);
        assert!(mutation.starts_with("simplicity"), "{mutation}");
        let (_, mutation) = mutate([0.34, 0.33, 0.33], 0, 1);
        assert!(mutation.starts_with("speed"), "{mutation}");
        let (_, mutation) = mutate([0.34, 0.33, 0.33], 0, 2);
        assert!(mutation.starts_with("safety"), "{mutation}");
    }

    #[test]
    fn a_weighting_is_scored_against_the_cases_it_gets_right() {
        let evals = evals();
        assert_eq!(score([0.5, 0.3, 0.2], &evals), 2.0 / 3.0);
        assert_eq!(score([0.2, 0.6, 0.2], &evals), 1.0 / 3.0);
        assert_eq!(score([0.2, 0.2, 0.6], &evals), 0.0);
        assert_eq!(favored([0.5, 0.3, 0.2]), "simplicity");
        assert_eq!(
            favored([0.2, 0.2, 0.2]),
            "simplicity",
            "ties go to the first axis"
        );
    }

    #[test]
    fn an_equal_fitness_is_not_a_gain() {
        // The rule the run enforces, stated once on its own: `retained` is a
        // strict improvement, so a candidate that merely matches its parent is
        // kept as a version and flagged as not retained.
        let parent = 0.5_f64;
        let candidate = 0.5_f64;
        assert!(candidate.partial_cmp(&parent) != Some(std::cmp::Ordering::Greater));
        assert!(0.500_001_f64 > parent);
    }

    #[test]
    fn a_malformed_line_is_refused_with_its_line_number() {
        let error = EvaluationSet::from_jsonl(
            r#"{"ideal": [0.5, 0.3, 0.2], "expected": "simplicity"}
not json"#,
        )
        .unwrap_err();
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("line 2"), "{error}");
    }

    #[test]
    fn an_unknown_axis_is_refused() {
        let error = EvaluationSet::from_jsonl(r#"{"ideal": [0.5, 0.3, 0.2], "expected": "vibes"}"#)
            .unwrap_err();
        assert!(error.to_string().contains("vibes"), "{error}");
    }

    #[test]
    fn an_empty_evaluation_set_is_refused() {
        assert!(EvaluationSet::from_jsonl("\n\n").is_err());
    }

    #[test]
    fn the_default_budget_is_bounded() {
        let budget = EvoBudget::default_budget();
        assert!(budget.max_versions >= budget.generations * budget.candidates_per_generation);
    }
}
