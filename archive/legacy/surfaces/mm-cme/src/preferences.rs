//! Phase 5 preference learning: a posterior per dimension, convergence and
//! drift, and the hooks that feed the planner.
//!
//! The region state machine owns the lifecycle; this module owns the numbers.
//! Observations are weighted, so a strong signal can move a posterior further
//! than a weak one, and convergence is reported once the variance is small
//! enough or enough evidence has accumulated. A stable posterior that is
//! contradicted by a signal more than three standard deviations away enters
//! `DriftDetected` and relearns from the new signal.

use std::collections::HashMap;

use mm_data::{MemorySystem, Timestamp, UserId};

pub use mm_data::PreferenceDimension;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::regions::preferences::PreferenceState;
use crate::session::{PreferenceProfile, Session};

/// Thresholds for learning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreferenceConfig {
    /// Observations required to call a posterior stable.
    pub convergence_observations: u32,
    /// Variance at or below which a posterior is stable.
    pub convergence_variance: f32,
    /// How many standard deviations count as drift.
    pub drift_sigmas: f32,
    /// Variance a drifted posterior restarts from.
    pub prior_variance: f32,
    /// Variance floor.
    pub min_variance: f32,
    /// Evidence required before values reach the planner.
    pub planner_min_observations: u32,
}

impl Default for PreferenceConfig {
    fn default() -> Self {
        Self {
            convergence_observations: 10,
            convergence_variance: 0.05,
            drift_sigmas: 3.0,
            prior_variance: 1.0,
            min_variance: 0.0001,
            planner_min_observations: 3,
        }
    }
}

/// One dimension's learned posterior.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PreferencePosterior {
    /// Mean, −1.0 to 1.0.
    pub mean: f32,
    /// Posterior variance.
    pub variance: f32,
    /// Weighted observation count.
    pub observations: u32,
    /// Lifecycle state.
    pub state: PreferenceState,
}

impl PreferencePosterior {
    /// A neutral prior.
    pub fn cold_start(config: &PreferenceConfig) -> Self {
        Self {
            mean: 0.0,
            variance: config.prior_variance,
            observations: 0,
            state: PreferenceState::ColdStart,
        }
    }

    /// Whether this posterior is safe to apply to the planner.
    pub fn is_applicable(&self, config: &PreferenceConfig) -> bool {
        self.observations >= config.planner_min_observations
            && matches!(
                self.state,
                PreferenceState::Converging | PreferenceState::Stable | PreferenceState::Relearning
            )
    }
}

/// The result of one observation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PreferenceUpdate {
    /// Dimension observed.
    pub dimension: PreferenceDimension,
    /// Posterior after the update.
    pub posterior: PreferencePosterior,
    /// True when the signal contradicted a stable posterior.
    pub drifted: bool,
}

/// Per-dimension posterior learner.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferenceLearner {
    posteriors: HashMap<PreferenceDimension, PreferencePosterior>,
    config: PreferenceConfig,
}

impl Default for PreferenceLearner {
    fn default() -> Self {
        Self::new()
    }
}

impl PreferenceLearner {
    /// A learner with the default thresholds.
    pub fn new() -> Self {
        Self::with_config(PreferenceConfig::default())
    }

    /// A learner with explicit thresholds.
    pub fn with_config(config: PreferenceConfig) -> Self {
        Self {
            posteriors: HashMap::new(),
            config,
        }
    }

    /// The configuration.
    pub fn config(&self) -> PreferenceConfig {
        self.config
    }

    /// One dimension's posterior.
    pub fn posterior(&self, dimension: PreferenceDimension) -> Option<PreferencePosterior> {
        self.posteriors.get(&dimension).copied()
    }

    /// Every observed posterior.
    pub fn posteriors(&self) -> &HashMap<PreferenceDimension, PreferencePosterior> {
        &self.posteriors
    }

    /// Fold one weighted signal into a dimension.
    pub fn observe(
        &mut self,
        dimension: PreferenceDimension,
        value: f32,
        weight: f32,
    ) -> PreferenceUpdate {
        let value = value.clamp(-1.0, 1.0);
        let weight = weight.max(0.0);
        let prior = *self
            .posteriors
            .entry(dimension)
            .or_insert_with(|| PreferencePosterior::cold_start(&self.config));

        if weight == 0.0 {
            return PreferenceUpdate {
                dimension,
                posterior: prior,
                drifted: false,
            };
        }

        let sigma = prior.variance.max(self.config.min_variance).sqrt();
        let drifted = prior.state == PreferenceState::Stable
            && (value - prior.mean).abs() > self.config.drift_sigmas * sigma;
        if drifted {
            let posterior = PreferencePosterior {
                mean: value,
                variance: self.config.prior_variance,
                observations: 1,
                state: PreferenceState::DriftDetected,
            };
            self.posteriors.insert(dimension, posterior);
            return PreferenceUpdate {
                dimension,
                posterior,
                drifted: true,
            };
        }

        let previous_observations = prior.observations as f32;
        let total = previous_observations + weight;
        let delta = value - prior.mean;
        let mean = prior.mean + weight * delta / total;
        let variance = ((prior.variance * previous_observations + weight * delta * delta) / total)
            .max(self.config.min_variance);
        let observations = prior
            .observations
            .saturating_add(weight.ceil().max(1.0) as u32);
        let converged = observations >= self.config.convergence_observations
            || (observations >= 3 && variance <= self.config.convergence_variance);
        let state = match prior.state {
            PreferenceState::DriftDetected | PreferenceState::Relearning => {
                if converged {
                    PreferenceState::Stable
                } else {
                    PreferenceState::Relearning
                }
            }
            _ if converged => PreferenceState::Stable,
            _ if observations >= 3 => PreferenceState::Converging,
            _ => PreferenceState::Observing,
        };
        let posterior = PreferencePosterior {
            mean,
            variance,
            observations,
            state,
        };
        self.posteriors.insert(dimension, posterior);
        PreferenceUpdate {
            dimension,
            posterior,
            drifted: false,
        }
    }

    /// Observe a whole signal set.
    pub fn observe_all(
        &mut self,
        signals: &[(PreferenceDimension, f32, f32)],
    ) -> Vec<PreferenceUpdate> {
        signals
            .iter()
            .map(|&(dimension, value, weight)| self.observe(dimension, value, weight))
            .collect()
    }

    /// A profile containing every applicable posterior, with planner hooks
    /// already applied.
    pub fn profile(&self) -> PreferenceProfile {
        let mut profile = PreferenceProfile::default();
        self.apply_to(&mut profile);
        profile
    }

    /// Write applicable values into `profile`, then apply the planner hooks.
    pub fn apply_to(&self, profile: &mut PreferenceProfile) {
        for (dimension, posterior) in &self.posteriors {
            if posterior.is_applicable(&self.config) {
                profile
                    .values
                    .insert(dimension.as_str().to_owned(), posterior.mean);
            }
        }
        PlannerHooks::apply(profile);
    }

    /// Write applicable values into a live session.
    pub fn apply_to_session(&self, session: &mut Session) {
        self.apply_to(&mut session.context.preferences);
    }

    /// Persist the learned posteriors into the user's Tier 4 memory.
    pub async fn persist(&self, system: &MemorySystem, user_id: &UserId) -> Result<()> {
        let mut user = system
            .users
            .load_or_create(user_id, user_id.as_str())
            .await?;
        for (dimension, posterior) in &self.posteriors {
            user.preferences.learned.insert(
                dimension.as_str().to_owned(),
                mm_data::memory::user::LearnedPreference {
                    mean: posterior.mean,
                    variance: posterior.variance,
                    n_observations: posterior.observations,
                    last_updated: Timestamp::now(),
                    state: data_state(posterior.state),
                },
            );
        }
        user.preferences.merge();
        system.users.save(&user).await?;
        Ok(())
    }

    /// Forget every posterior.
    pub fn reset(&mut self) {
        self.posteriors.clear();
    }
}

/// Derives the keys the planner already reads from the five dimensions.
///
/// Explicit values always win: the hooks only fill a key that is not present.
pub struct PlannerHooks;

impl PlannerHooks {
    /// Apply the derived keys to a profile.
    pub fn apply(profile: &mut PreferenceProfile) {
        if let Some(detail) = mean(
            profile,
            &[
                PreferenceDimension::TechnicalDepth,
                PreferenceDimension::Verbosity,
            ],
        ) {
            profile.values.entry("detail".to_owned()).or_insert(detail);
        }
        if let Some(formality) = profile
            .values
            .get(PreferenceDimension::Formality.as_str())
            .copied()
        {
            profile
                .values
                .entry("format".to_owned())
                .or_insert(formality);
        }
    }
}

fn mean(profile: &PreferenceProfile, dimensions: &[PreferenceDimension]) -> Option<f32> {
    let values: Vec<f32> = dimensions
        .iter()
        .filter_map(|dimension| profile.values.get(dimension.as_str()).copied())
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f32>() / values.len() as f32)
    }
}

fn data_state(state: PreferenceState) -> mm_data::memory::PreferenceState {
    match state {
        PreferenceState::ColdStart => mm_data::memory::PreferenceState::ColdStart,
        PreferenceState::Observing => mm_data::memory::PreferenceState::Observing,
        PreferenceState::Converging | PreferenceState::Relearning => {
            mm_data::memory::PreferenceState::Converging
        }
        PreferenceState::Stable => mm_data::memory::PreferenceState::Stable,
        PreferenceState::DriftDetected => mm_data::memory::PreferenceState::DriftDetected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{TurnContext, TurnPlanner};
    use crate::session::SessionContext;
    use mm_data::{SessionId, TurnId};

    fn planner_level(learner: &PreferenceLearner) -> u8 {
        let profile = learner.profile();
        let context = SessionContext {
            preferences: profile,
            ..SessionContext::default()
        };
        let ctx = TurnContext::user_input(TurnId::new("t"), SessionId::new("s"), "explain it");
        TurnPlanner.build_plan(&ctx, &context).unwrap().detail_level
    }

    #[test]
    fn ten_observations_converge_and_reach_the_planner() {
        let mut learner = PreferenceLearner::new();
        let mut update = None;
        for _ in 0..10 {
            update = Some(learner.observe(PreferenceDimension::TechnicalDepth, 0.5, 1.0));
        }
        let update = update.unwrap();
        assert_eq!(update.posterior.state, PreferenceState::Stable);
        assert_eq!(update.posterior.observations, 10);
        assert!((update.posterior.mean - 0.5).abs() < 0.01);
        assert_eq!(planner_level(&learner), 4);
    }

    #[test]
    fn early_observations_converge_quickly_with_low_variance() {
        let config = PreferenceConfig {
            convergence_variance: 0.5,
            ..PreferenceConfig::default()
        };
        let mut learner = PreferenceLearner::with_config(config);
        for _ in 0..3 {
            learner.observe(PreferenceDimension::Verbosity, 0.4, 1.0);
        }
        assert_eq!(
            learner
                .posterior(PreferenceDimension::Verbosity)
                .unwrap()
                .state,
            PreferenceState::Stable
        );
    }

    #[test]
    fn a_stable_preference_detects_drift_and_relearns() {
        let mut learner = PreferenceLearner::new();
        for _ in 0..10 {
            learner.observe(PreferenceDimension::Pace, 0.8, 1.0);
        }
        let drift = learner.observe(PreferenceDimension::Pace, -0.9, 1.0);
        assert!(drift.drifted);
        assert_eq!(drift.posterior.state, PreferenceState::DriftDetected);
        assert!((drift.posterior.mean + 0.9).abs() < 0.01);

        let mut last = None;
        for _ in 0..10 {
            last = Some(learner.observe(PreferenceDimension::Pace, -0.6, 1.0));
        }
        let last = last.unwrap();
        assert_eq!(last.posterior.state, PreferenceState::Stable);
        assert!((last.posterior.mean + 0.6).abs() < 0.05);
    }

    #[test]
    fn explicit_values_win_over_derived_hooks() {
        let mut learner = PreferenceLearner::new();
        for _ in 0..10 {
            learner.observe(PreferenceDimension::Verbosity, 0.8, 1.0);
        }
        let mut profile = PreferenceProfile::default();
        profile.values.insert("detail".to_owned(), -0.75);
        learner.apply_to(&mut profile);
        assert_eq!(profile.value("detail"), -0.75);
        assert!((profile.value("verbosity") - 0.8).abs() < 0.01);
    }

    #[test]
    fn weighted_signals_move_faster_than_weak_ones() {
        let mut weak = PreferenceLearner::new();
        let mut strong = PreferenceLearner::new();
        weak.observe(PreferenceDimension::Initiative, 0.0, 1.0);
        strong.observe(PreferenceDimension::Initiative, 0.0, 1.0);
        weak.observe(PreferenceDimension::Initiative, 1.0, 0.5);
        strong.observe(PreferenceDimension::Initiative, 1.0, 2.0);
        assert!(
            strong
                .posterior(PreferenceDimension::Initiative)
                .unwrap()
                .mean
                > weak
                    .posterior(PreferenceDimension::Initiative)
                    .unwrap()
                    .mean
        );
    }
}
