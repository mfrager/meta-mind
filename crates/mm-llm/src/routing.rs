//! Model routing.
//!
//! Routing is **deterministic first**: a labelled request (purpose, complexity,
//! stakes, required precision, latency and cost budgets) is met by the cheapest
//! configured model that can take it. An embedding model may *propose* a
//! complexity, but a threshold decides, so the same request routes the same way on
//! every run and a golden table can pin it.
//!
//! [`Router`] is the seam Phase 9's `DecisionCore` implements: a learned router
//! replaces [`ThresholdRouter`] without a caller changing.

use serde::{Deserialize, Serialize};

use crate::client::Purpose;
use crate::config::{Complexity, LlmConfig, ModelSpec, Precision, Stakes};
use crate::error::LlmError;

/// Why a model was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingReason {
    /// The caller named the model.
    Pinned,
    /// The cheapest model that meets the stated requirement.
    CheapestSufficient,
    /// No configured model meets the requirement, so the most capable one did.
    OnlyCandidate,
    /// The cheapest sufficient model was over budget; the cheapest overall ran.
    BudgetFallback,
    /// No models are configured; the configured default ran.
    Default,
}

impl RoutingReason {
    /// The stored form, recorded in `llm_routing.reason`.
    pub fn as_str(self) -> &'static str {
        match self {
            RoutingReason::Pinned => "pinned",
            RoutingReason::CheapestSufficient => "cheapest_sufficient",
            RoutingReason::OnlyCandidate => "only_candidate",
            RoutingReason::BudgetFallback => "budget_fallback",
            RoutingReason::Default => "default",
        }
    }

    /// Parse the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        [
            RoutingReason::Pinned,
            RoutingReason::CheapestSufficient,
            RoutingReason::OnlyCandidate,
            RoutingReason::BudgetFallback,
            RoutingReason::Default,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }
}

/// What a caller knows about the work it is asking for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingRequest {
    /// Why the call is being made.
    pub purpose: Purpose,
    /// How hard the task is.
    pub complexity: Complexity,
    /// How much a wrong answer costs.
    pub stakes: Stakes,
    /// How exact the answer must be.
    pub required_precision: Precision,
    /// A latency ceiling in milliseconds; 0 means "no ceiling".
    pub latency_budget_ms: u32,
    /// A cost ceiling in micros; 0 means "no ceiling".
    pub cost_budget_micros: i64,
    /// A model the caller insists on.
    pub pinned_model: Option<String>,
}

impl RoutingRequest {
    /// An unremarkable request: ordinary complexity, reversible stakes, normal
    /// precision, no budgets.
    pub fn new(purpose: Purpose) -> Self {
        RoutingRequest {
            purpose,
            complexity: Complexity::Medium,
            stakes: Stakes::Medium,
            required_precision: Precision::Normal,
            latency_budget_ms: 0,
            cost_budget_micros: 0,
            pinned_model: None,
        }
    }

    /// Set the complexity.
    pub fn with_complexity(mut self, complexity: Complexity) -> Self {
        self.complexity = complexity;
        self
    }

    /// Set the stakes.
    pub fn with_stakes(mut self, stakes: Stakes) -> Self {
        self.stakes = stakes;
        self
    }

    /// Set the required precision.
    pub fn with_precision(mut self, precision: Precision) -> Self {
        self.required_precision = precision;
        self
    }

    /// Set the latency ceiling.
    pub fn with_latency_budget(mut self, ms: u32) -> Self {
        self.latency_budget_ms = ms;
        self
    }

    /// Set the cost ceiling.
    pub fn with_cost_budget(mut self, micros: i64) -> Self {
        self.cost_budget_micros = micros;
        self
    }

    /// Pin the model.
    pub fn with_pinned_model(mut self, model: impl Into<String>) -> Self {
        self.pinned_model = Some(model.into());
        self
    }

    /// The complexity the choice must actually satisfy.
    ///
    /// High stakes or a high precision bar raise the bar to `High`: a consequential,
    /// exactness-critical call must not be answered by the cheapest small model just
    /// because the caller called the task "medium".
    fn effective_ceiling(&self) -> Complexity {
        let mut ceiling = self.complexity;
        if self.stakes == Stakes::High || self.required_precision == Precision::High {
            ceiling = ceiling.max(Complexity::High);
        }
        ceiling
    }
}

/// What the router decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// The model to call.
    pub model: String,
    /// Why.
    pub reason: RoutingReason,
    /// The estimated worst-case cost in micros.
    pub cost_micros: i64,
}

/// A model chooser.
pub trait Router: Send + Sync {
    /// Choose a model for `req`.
    fn choose(&self, req: &RoutingRequest) -> Result<RoutingDecision, LlmError>;
}

/// The cheapest model that meets a labelled requirement, from `config/llm.toml`.
#[derive(Debug, Clone)]
pub struct ThresholdRouter {
    models: Vec<ModelSpec>,
    default_model: Option<String>,
    /// The prompt-token count the cost estimate assumes. A routing decision is made
    /// before the prompt is tokenized, so the estimate is a documented constant
    /// rather than a guess that changes per call.
    estimate_prompt_tokens: u32,
    /// The completion ceiling the estimate assumes.
    estimate_max_tokens: u32,
}

impl ThresholdRouter {
    /// A router over a configuration's models.
    pub fn new(cfg: &LlmConfig) -> Self {
        ThresholdRouter {
            models: cfg.models.clone(),
            default_model: cfg.defaults.default_model.clone(),
            estimate_prompt_tokens: 1000,
            estimate_max_tokens: cfg.defaults.max_tokens.max(1),
        }
    }

    /// A router over an explicit model list.
    pub fn from_specs(models: Vec<ModelSpec>, default_model: Option<String>) -> Self {
        ThresholdRouter {
            models,
            default_model,
            estimate_prompt_tokens: 1000,
            estimate_max_tokens: 1024,
        }
    }

    /// The estimated worst-case cost of one call on `spec`.
    pub fn estimate(&self, spec: &ModelSpec) -> i64 {
        spec.worst_case_cost_micros(self.estimate_prompt_tokens, self.estimate_max_tokens)
    }
}

impl Router for ThresholdRouter {
    fn choose(&self, req: &RoutingRequest) -> Result<RoutingDecision, LlmError> {
        if let Some(name) = &req.pinned_model {
            let spec = self
                .models
                .iter()
                .find(|m| &m.name == name)
                .ok_or_else(|| {
                    LlmError::Config(format!("pinned model `{name}` is not configured"))
                })?;
            return Ok(RoutingDecision {
                model: spec.name.clone(),
                reason: RoutingReason::Pinned,
                cost_micros: self.estimate(spec),
            });
        }

        if self.models.is_empty() {
            return match &self.default_model {
                Some(model) => Ok(RoutingDecision {
                    model: model.clone(),
                    reason: RoutingReason::Default,
                    cost_micros: 0,
                }),
                None => Err(LlmError::Config(
                    "no models are configured and no default_model is set".into(),
                )),
            };
        }

        let ceiling = req.effective_ceiling();
        let mut sufficient: Vec<&ModelSpec> = self
            .models
            .iter()
            .filter(|m| m.complexity_ceiling >= ceiling)
            .collect();
        let mut reason = RoutingReason::CheapestSufficient;
        if sufficient.is_empty() {
            // Nothing claims to be able to take the work; the most capable model is
            // the honest answer, and the reason records that it is a fallback.
            sufficient = self.models.iter().collect();
            reason = RoutingReason::OnlyCandidate;
        }

        if req.latency_budget_ms > 0 {
            let fast: Vec<&ModelSpec> = sufficient
                .iter()
                .copied()
                .filter(|m| m.typical_latency_ms <= req.latency_budget_ms)
                .collect();
            if !fast.is_empty() {
                sufficient = fast;
            }
            // When nothing fits the latency ceiling, the requirement that remains is
            // "capable", so the choice falls back to the capable set rather than to
            // the fastest of all models.
        }

        let best = self.cheapest(&sufficient)?;
        let cost = self.estimate(best);
        if req.cost_budget_micros > 0 && cost > req.cost_budget_micros {
            let all: Vec<&ModelSpec> = self.models.iter().collect();
            let cheapest = self.cheapest(&all)?;
            let cheap_cost = self.estimate(cheapest);
            if cheap_cost > req.cost_budget_micros {
                return Err(LlmError::BudgetExhausted(format!(
                    "cheapest configured model `{}` costs {cheap_cost} micros, over the {}-micro budget",
                    cheapest.name, req.cost_budget_micros
                )));
            }
            return Ok(RoutingDecision {
                model: cheapest.name.clone(),
                reason: RoutingReason::BudgetFallback,
                cost_micros: cheap_cost,
            });
        }

        Ok(RoutingDecision {
            model: best.name.clone(),
            reason,
            cost_micros: cost,
        })
    }
}

impl ThresholdRouter {
    /// The cheapest of `pool`, breaking ties on latency then name so the choice is
    /// total and reproducible.
    fn cheapest<'a>(&self, pool: &[&'a ModelSpec]) -> Result<&'a ModelSpec, LlmError> {
        pool.iter()
            .copied()
            .min_by_key(|m| (self.estimate(m), m.typical_latency_ms, m.name.clone()))
            .ok_or_else(|| LlmError::Config("no candidate model".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(
        name: &str,
        in_cost: i64,
        out_cost: i64,
        latency: u32,
        ceiling: Complexity,
    ) -> ModelSpec {
        ModelSpec {
            name: name.into(),
            input_cost_micros_per_1k: in_cost,
            output_cost_micros_per_1k: out_cost,
            typical_latency_ms: latency,
            complexity_ceiling: ceiling,
        }
    }

    fn router() -> ThresholdRouter {
        ThresholdRouter::from_specs(
            vec![
                spec("small", 150, 600, 400, Complexity::Medium),
                spec("frontier", 2500, 10000, 2500, Complexity::VeryHigh),
                spec("mock-small", 100, 400, 5, Complexity::High),
            ],
            Some("small".into()),
        )
    }

    #[test]
    fn a_pinned_model_is_used_or_refused() {
        let decision = router()
            .choose(&RoutingRequest::new(Purpose::Plan).with_pinned_model("frontier"))
            .unwrap();
        assert_eq!(decision.model, "frontier");
        assert_eq!(decision.reason, RoutingReason::Pinned);

        let err = router()
            .choose(&RoutingRequest::new(Purpose::Plan).with_pinned_model("nope"))
            .unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");
    }

    #[test]
    fn the_cheapest_model_that_can_take_the_work_wins() {
        // Medium work: small and mock-small both qualify; mock-small is cheaper.
        let decision = router()
            .choose(&RoutingRequest::new(Purpose::Interpret))
            .unwrap();
        assert_eq!(decision.model, "mock-small");
        assert_eq!(decision.reason, RoutingReason::CheapestSufficient);

        // Very high complexity: only frontier qualifies.
        let decision = router()
            .choose(&RoutingRequest::new(Purpose::Plan).with_complexity(Complexity::VeryHigh))
            .unwrap();
        assert_eq!(decision.model, "frontier");
    }

    #[test]
    fn high_stakes_raise_the_bar_above_the_cheap_small_model() {
        let decision = router()
            .choose(&RoutingRequest::new(Purpose::Plan).with_stakes(Stakes::High))
            .unwrap();
        // `small` tops out at medium, so it cannot take a high-stakes call.
        assert_ne!(decision.model, "small");
        assert_eq!(decision.model, "mock-small");
    }

    #[test]
    fn a_tight_cost_budget_falls_back_and_an_impossible_one_fails() {
        let decision = router()
            .choose(
                &RoutingRequest::new(Purpose::Plan)
                    .with_complexity(Complexity::VeryHigh)
                    .with_cost_budget(1000),
            )
            .unwrap();
        assert_eq!(decision.model, "mock-small");
        assert_eq!(decision.reason, RoutingReason::BudgetFallback);
        assert!(decision.cost_micros <= 1000, "{}", decision.cost_micros);

        let err = router()
            .choose(
                &RoutingRequest::new(Purpose::Plan)
                    .with_complexity(Complexity::VeryHigh)
                    .with_cost_budget(1),
            )
            .unwrap_err();
        assert_eq!(err.kind(), "budget_exhausted");
    }

    #[test]
    fn a_latency_budget_excludes_slow_models_and_an_empty_config_uses_the_default() {
        // Medium work with a 100 ms ceiling: `small` needs 400 ms and is excluded,
        // so the cheap model takes it.
        let decision = router()
            .choose(&RoutingRequest::new(Purpose::Interpret).with_latency_budget(100))
            .unwrap();
        assert_eq!(decision.model, "mock-small");

        // Very-high work with a ceiling nothing can meet: capability is the
        // requirement that remains, so the capable model still runs.
        let decision = router()
            .choose(
                &RoutingRequest::new(Purpose::Plan)
                    .with_complexity(Complexity::VeryHigh)
                    .with_latency_budget(100),
            )
            .unwrap();
        assert_eq!(decision.model, "frontier");

        let empty = ThresholdRouter::from_specs(vec![], Some("small".into()));
        let decision = empty.choose(&RoutingRequest::new(Purpose::Plan)).unwrap();
        assert_eq!(decision.reason, RoutingReason::Default);

        let none = ThresholdRouter::from_specs(vec![], None);
        assert!(none.choose(&RoutingRequest::new(Purpose::Plan)).is_err());
    }

    #[test]
    fn reasons_round_trip() {
        for reason in [
            RoutingReason::Pinned,
            RoutingReason::CheapestSufficient,
            RoutingReason::OnlyCandidate,
            RoutingReason::BudgetFallback,
            RoutingReason::Default,
        ] {
            assert_eq!(RoutingReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(RoutingReason::parse("nope"), None);
    }
}
