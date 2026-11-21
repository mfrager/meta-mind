//! Hill-climbing: steepest ascent over any [`Optimizable`] artifact.
//!
//! Every step generates the artifact's neighbors, evaluates them, and keeps the
//! best one only when it improves by at least the configured margin. The result
//! carries the score trajectory so an audit can explain what changed and why.

use serde::{Deserialize, Serialize};

/// Something that can score itself and propose neighboring variants.
pub trait Optimizable: Clone {
    /// A higher score is better.
    fn score(&self) -> f32;

    /// Variants one mutation away.
    fn neighbors(&self) -> Vec<Self>;
}

/// Stopping and acceptance thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HillClimbConfig {
    /// Maximum accepted improvements.
    pub max_iterations: usize,
    /// Minimum score gain to accept a neighbor.
    pub min_improvement: f32,
}

impl Default for HillClimbConfig {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            min_improvement: 0.005,
        }
    }
}

/// One accepted step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HillClimbStep {
    /// One-based iteration.
    pub iteration: usize,
    /// Score before the step.
    pub from_score: f32,
    /// Score after the step.
    pub to_score: f32,
}

/// The outcome of a climb.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HillClimbResult<T> {
    /// The best artifact found.
    pub best: T,
    /// Score of the initial artifact.
    pub initial_score: f32,
    /// Score of `best`.
    pub final_score: f32,
    /// Accepted steps.
    pub steps: Vec<HillClimbStep>,
    /// Neighbors evaluated across all iterations.
    pub evaluated: usize,
    /// Score after each accepted step, starting with the initial score.
    pub trajectory: Vec<f32>,
}

impl<T> HillClimbResult<T> {
    /// Total improvement.
    pub fn improvement(&self) -> f32 {
        self.final_score - self.initial_score
    }

    /// Whether any step improved the artifact.
    pub fn improved(&self) -> bool {
        self.improvement() > 0.0
    }
}

/// Run steepest ascent from `initial`.
pub fn climb<T: Optimizable>(initial: &T, config: HillClimbConfig) -> HillClimbResult<T> {
    let mut best = initial.clone();
    let initial_score = best.score();
    let mut best_score = initial_score;
    let mut steps = Vec::new();
    let mut trajectory = vec![initial_score];
    let mut evaluated = 0;

    for iteration in 1..=config.max_iterations {
        let neighbors = best.neighbors();
        if neighbors.is_empty() {
            break;
        }
        evaluated += neighbors.len();
        let mut candidate = None;
        let mut candidate_score = best_score;
        for neighbor in neighbors {
            let score = neighbor.score();
            if score > candidate_score + config.min_improvement {
                candidate = Some(neighbor);
                candidate_score = score;
            }
        }
        // A plateau is terminal for deterministic steepest ascent: the
        // neighborhood did not improve, so retrying it cannot help.
        let Some(candidate) = candidate else {
            break;
        };
        steps.push(HillClimbStep {
            iteration,
            from_score: best_score,
            to_score: candidate_score,
        });
        best = candidate;
        best_score = candidate_score;
        trajectory.push(best_score);
    }

    HillClimbResult {
        best,
        initial_score,
        final_score: best_score,
        steps,
        evaluated,
        trajectory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Number(i32);

    impl Optimizable for Number {
        fn score(&self) -> f32 {
            -(self.0 as f32)
        }

        fn neighbors(&self) -> Vec<Self> {
            vec![Number(self.0 - 1), Number(self.0 + 1)]
        }
    }

    #[derive(Clone)]
    struct Peak;

    impl Optimizable for Peak {
        fn score(&self) -> f32 {
            1.0
        }

        fn neighbors(&self) -> Vec<Self> {
            Vec::new()
        }
    }

    #[test]
    fn steepest_ascent_walks_toward_the_optimum() {
        let result = climb(
            &Number(5),
            HillClimbConfig {
                max_iterations: 2,
                ..HillClimbConfig::default()
            },
        );
        assert_eq!(result.best.0, 3);
        assert_eq!(result.final_score, -3.0);
        assert_eq!(result.steps.len(), 2);
        assert!(result.improved());
    }

    #[test]
    fn an_artifact_without_better_neighbors_stays_put() {
        let result = climb(&Peak, HillClimbConfig::default());
        assert_eq!(result.best.score(), 1.0);
        assert!(!result.improved());
        assert!(result.steps.is_empty());
    }
}
