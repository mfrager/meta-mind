//! Routing: deterministic, cheapest-sufficient, and pinned to a golden table.
//!
//! A router that is only "reasonable" cannot be checked, so the choice for every
//! labelled requirement in the matrix is pinned by a snapshot. The label matrix is
//! the input; the model and the reason are the output.

mod common;

use mm_llm::client::Purpose;
use mm_llm::config::{Complexity, ModelSpec, Precision, Stakes};
use mm_llm::routing::{Router, RoutingReason, RoutingRequest, ThresholdRouter};
use serde_json::json;

fn router() -> ThresholdRouter {
    let cfg = mm_llm::config::LlmConfig::from_toml_str(&common::config_toml(1)).unwrap();
    ThresholdRouter::new(&cfg)
}

/// The labelled requirements the decision table covers.
fn matrix() -> Vec<(&'static str, RoutingRequest)> {
    let mut out = Vec::new();
    for purpose in [
        Purpose::Interpret,
        Purpose::Plan,
        Purpose::Extract,
        Purpose::Critique,
        Purpose::Summarize,
        Purpose::CodeReview,
        Purpose::Classify,
        Purpose::Diagnose,
    ] {
        for complexity in [
            Complexity::Low,
            Complexity::Medium,
            Complexity::High,
            Complexity::VeryHigh,
        ] {
            out.push((
                "plain",
                RoutingRequest::new(purpose).with_complexity(complexity),
            ));
        }
    }
    out.push((
        "high_stakes",
        RoutingRequest::new(Purpose::Plan)
            .with_complexity(Complexity::Medium)
            .with_stakes(Stakes::High),
    ));
    out.push((
        "high_precision",
        RoutingRequest::new(Purpose::Critique).with_precision(Precision::High),
    ));
    out.push((
        "tight_latency",
        RoutingRequest::new(Purpose::Interpret).with_latency_budget(100),
    ));
    out.push((
        "tight_budget",
        RoutingRequest::new(Purpose::Plan)
            .with_complexity(Complexity::VeryHigh)
            .with_cost_budget(1000),
    ));
    out.push((
        "pinned",
        RoutingRequest::new(Purpose::Plan).with_pinned_model("frontier"),
    ));
    out
}

#[test]
fn the_routing_decision_table_is_golden() {
    let router = router();
    let table: Vec<serde_json::Value> = matrix()
        .into_iter()
        .map(|(label, request)| {
            let decision = router.choose(&request).unwrap();
            json!({
                "label": label,
                "purpose": request.purpose.as_str(),
                "complexity": request.complexity.as_str(),
                "stakes": format!("{:?}", request.stakes).to_lowercase(),
                "precision": format!("{:?}", request.required_precision).to_lowercase(),
                "latency_budget_ms": request.latency_budget_ms,
                "cost_budget_micros": request.cost_budget_micros,
                "pinned_model": request.pinned_model,
                "model": decision.model,
                "reason": decision.reason.as_str(),
                "cost_micros": decision.cost_micros,
            })
        })
        .collect();
    insta::assert_json_snapshot!("routing_decision_table", table);
}

#[test]
fn routing_is_repeatable_across_runs() {
    let a = router();
    let b = router();
    for (_, request) in matrix() {
        assert_eq!(
            a.choose(&request).unwrap(),
            b.choose(&request).unwrap(),
            "the same labelled request must route the same way"
        );
    }
}

#[test]
fn the_cheapest_sufficient_model_wins_and_capability_is_the_floor() {
    let router = router();
    // Ordinary work: `mock-small` is the cheapest model that tops out above medium.
    let decision = router
        .choose(&RoutingRequest::new(Purpose::Interpret))
        .unwrap();
    assert_eq!(decision.model, "mock-small");
    assert_eq!(decision.reason, RoutingReason::CheapestSufficient);

    // Very high complexity: only `frontier` claims to be able to take it.
    let decision = router
        .choose(&RoutingRequest::new(Purpose::Plan).with_complexity(Complexity::VeryHigh))
        .unwrap();
    assert_eq!(decision.model, "frontier");

    // High stakes raise the bar past the small model's ceiling.
    let decision = router
        .choose(&RoutingRequest::new(Purpose::Plan).with_stakes(Stakes::High))
        .unwrap();
    assert_ne!(decision.model, "small");
}

#[test]
fn a_budget_that_cannot_be_met_fails_instead_of_overspending() {
    let router = router();
    let decision = router
        .choose(
            &RoutingRequest::new(Purpose::Plan)
                .with_complexity(Complexity::VeryHigh)
                .with_cost_budget(1000),
        )
        .unwrap();
    assert_eq!(decision.reason, RoutingReason::BudgetFallback);
    assert!(decision.cost_micros <= 1000, "{decision:?}");

    let err = router
        .choose(
            &RoutingRequest::new(Purpose::Plan)
                .with_complexity(Complexity::VeryHigh)
                .with_cost_budget(1),
        )
        .unwrap_err();
    assert_eq!(err.kind(), "budget_exhausted");
}

#[test]
fn a_pinned_model_is_honoured_or_refused() {
    let router = router();
    let decision = router
        .choose(&RoutingRequest::new(Purpose::Plan).with_pinned_model("frontier"))
        .unwrap();
    assert_eq!(decision.model, "frontier");
    assert_eq!(decision.reason, RoutingReason::Pinned);
    assert!(decision.cost_micros > 0, "{decision:?}");

    let err = router
        .choose(&RoutingRequest::new(Purpose::Plan).with_pinned_model("not-configured"))
        .unwrap_err();
    assert_eq!(err.kind(), "config");
}

#[test]
fn an_empty_model_list_uses_the_configured_default_or_fails() {
    let with_default = ThresholdRouter::from_specs(vec![], Some("small".into()));
    assert_eq!(
        with_default
            .choose(&RoutingRequest::new(Purpose::Plan))
            .unwrap()
            .reason,
        RoutingReason::Default
    );

    let without = ThresholdRouter::from_specs(vec![], None);
    assert!(without.choose(&RoutingRequest::new(Purpose::Plan)).is_err());
}

#[test]
fn the_cost_estimate_rounds_up_per_started_thousand() {
    let spec = ModelSpec {
        name: "m".into(),
        input_cost_micros_per_1k: 150,
        output_cost_micros_per_1k: 600,
        typical_latency_ms: 1,
        complexity_ceiling: Complexity::Low,
    };
    assert_eq!(spec.cost_micros(0, 0), 0);
    assert_eq!(spec.cost_micros(1000, 1000), 750);
    assert_eq!(spec.cost_micros(1001, 0), 300);
    assert_eq!(spec.worst_case_cost_micros(1000, 1024), 1350);
}
