//! `mm-firewall` — the sanity firewall: the last thing between a decision and an
//! action.
//!
//! Design §46 asks for one scan over everything the system already knows —
//! assumptions, contradictions, comparisons, constraints, missing steps, risk,
//! reversibility, authorization, precedent, the simpler alternative, the stakes —
//! that emits exactly one of seven outcomes with the reasons behind it. This crate
//! is that scan.
//!
//! The rule everything here upholds:
//!
//! > **Hard prohibitions are deterministic and short-circuit to `REJECT`; model
//! > output can only ever make the firewall more cautious, never less.**
//!
//! The data flows through four layers, in this order:
//!
//! 1. [`prohibitions`] — six pure functions of the input, in a published order. A
//!    hit is `REJECT` and no model is consulted at all.
//! 2. [`rails`] — the deterministic signals the input alone justifies. This layer
//!    always runs, which is what makes a rails-only evaluation (no core configured)
//!    meaningful.
//! 3. [`scan`] — the bounded judgments, asked of a [`mm_decision::DecisionCore`],
//!    calibrated before they are trusted and submitted to a conformal threshold
//!    before they can pass. A judgment can raise the outcome and cannot lower it.
//! 4. [`aggregate`] — the precedence. The outcome is the most cautious reason, and
//!    a prohibition forces `REJECT` whatever else said what.
//!
//! Two producers of judgment, one authority: the core *proposes* calibrated
//! answers, and the deterministic aggregation decides. That is why nothing in the
//! aggregation can be talked out of a `REJECT`, and why
//! [`aggregate::aggregate_with_prohibition`] takes the prohibition as a parameter —
//! so the invariant is in the signature rather than in a conversation.
//!
//! [`report`] carries the input and the report, including the input's digest: the
//! same rule Phase 8's semantic cache follows, reused here so a decision can never
//! be replayed against a state it did not see. [`rdf`] is the graph mirror, and
//! [`outcome`] is the vocabulary the CLI, the SQLite columns, the log records and
//! Phase 11's calibration all share.
#![forbid(unsafe_code)]

pub mod aggregate;
pub mod error;
pub mod outcome;
pub mod prohibitions;
pub mod rails;
pub mod rdf;
pub mod report;
pub mod scan;

pub use aggregate::{aggregate, aggregate_with_prohibition};
pub use error::{FirewallError, Result};
pub use outcome::{
    normalized_signals, outcome_for_code, prohibition_reason_code, reason_code_rank, sort_signals,
    upsert_signal, upsert_signal_at, FirewallOutcome, ReasonCode, FIREWALL_OUTCOMES, REASON_CODES,
};
pub use prohibitions::{
    all_prohibitions, first_prohibition, prohibition_check, ProhibitionCheck, PROHIBITIONS,
    PROHIBITION_IDS,
};
pub use rails::{
    run_rails, RailOutcome, MAX_LOSS_REVIEW_THRESHOLD, PRECEDENT_STAKES_THRESHOLD,
    RUIN_REVIEW_THRESHOLD,
};
pub use report::{
    AssumptionRef, AuthorizationRef, ComparisonOutcomeRef, ConstraintRef, ContradictionRef,
    FirewallInput, FirewallReport, HardProhibition, IdentityInvariantHit, MissingStep,
    Reversibility, Spend, Stakes, ToolTarget,
};
pub use scan::{
    alternative_question, authorization_question, comparison_question, residual_risk_question,
    safety_question, scan, BoundedJudgment, FirewallConfig, FirewallCtx, ScanOutcome,
    CLASS_ALTERNATIVE, CLASS_AUTHORIZATION, CLASS_COMPARISON, CLASS_RESIDUAL_RISK, CLASS_SAFETY,
    DECISION_CLASSES, DEFAULT_TARGET_COVERAGE, UNCALIBRATED_HIGH_CONFIDENCE,
};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.firewall";

/// Evaluate one input end to end and return the report.
///
/// This is the whole firewall in one call, and the CLI's `firewall eval` is a thin
/// wrapper over it: rails, then — only when no prohibition fired — the bounded
/// judgments, then the aggregation. The prohibition short-circuit is here rather
/// than in the caller because a caller that ran the judgments anyway would spend a
/// model call on a run that is already refused, and, worse, would put a model
/// answer into a report about a `REJECT`.
pub async fn evaluate(input: &FirewallInput, ctx: &mut FirewallCtx) -> Result<FirewallReport> {
    input.validate()?;

    let rails = run_rails(input);
    let mut signals = rails.signals.clone();
    let mut magnitudes = rails.signals_map.clone();
    let mut decisions: Vec<mm_core::Ulid> = Vec::new();

    if rails.prohibition.is_none() {
        let scan = scan::scan(input, ctx).await?;
        signals.extend(scan.signals);
        for (code, value) in scan.signals_map {
            magnitudes.insert(code, value);
        }
        decisions = scan.decisions;
    }

    let id = ctx.next_id();
    let (outcome, reason_codes, hard_prohibition) =
        aggregate_with_prohibition(rails.prohibition, &signals);
    Ok(FirewallReport::new(
        id,
        input,
        outcome,
        reason_codes,
        decisions,
        hard_prohibition,
        magnitudes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_decision::risk::{analyze_risk_with, LossOutcome, RiskMeasure, RiskOptions};

    fn ulid(n: u128) -> mm_core::Ulid {
        mm_core::Ulid::from_parts(1_700_000_000_000, n)
    }

    /// A risk budget nothing in these fixtures can deplete, so "clean" means clean
    /// in every rail and not only in the ones a test happens to be looking at.
    fn input() -> FirewallInput {
        FirewallInput::new(
            ulid(1),
            analyze_risk_with(
                &[LossOutcome {
                    name: "small".into(),
                    probability: 1.0,
                    loss: 1.0,
                }],
                RiskMeasure::Variance,
                &RiskOptions {
                    capital: 1.0e9,
                    reversibility: 0.5,
                    optionality: 0.5,
                },
            ),
        )
    }

    #[tokio::test]
    async fn a_clean_input_with_no_core_proceeds_with_a_nominal_reason() {
        let mut ctx = FirewallCtx::new(FirewallConfig::default());
        let report = evaluate(&input(), &mut ctx).await.unwrap();
        assert_eq!(report.outcome, FirewallOutcome::Proceed);
        assert_eq!(
            report.reason_code_strings(),
            vec!["signal.nominal".to_string()]
        );
        assert!(report.decisions.is_empty());
        assert!(!report.has_prohibition());
    }

    #[tokio::test]
    async fn a_prohibition_rejects_without_asking_any_question() {
        let mut input = input();
        input.reversibility = Reversibility::Irreversible;
        input.authorization = AuthorizationRef {
            granted: false,
            scope: "none".into(),
            approval_id: None,
        };
        let mut ctx = FirewallCtx::new(FirewallConfig::default());
        let report = evaluate(&input, &mut ctx).await.unwrap();
        assert_eq!(report.outcome, FirewallOutcome::Reject);
        assert_eq!(
            report.hard_prohibition_id(),
            Some("irreversible_without_approval")
        );
        assert!(report.decisions.is_empty(), "a refusal consults nothing");
        assert!(ctx.consulted.is_empty());
        assert!(report
            .reason_code_strings()
            .contains(&"prohibition.irreversible_without_approval".to_string()));
    }

    #[tokio::test]
    async fn a_prohibition_beats_a_permissive_core() {
        use async_trait::async_trait;
        use mm_decision::core::CoreId;
        use mm_decision::question::{AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState};

        struct AlwaysFine;

        #[async_trait]
        impl mm_decision::DecisionCore for AlwaysFine {
            fn id(&self) -> CoreId {
                CoreId::Hosted
            }

            async fn answer(
                &self,
                question: &DecisionQuestion,
                _state: &DecisionState,
            ) -> mm_decision::Result<DecisionAnswer> {
                let value = match question {
                    DecisionQuestion::YesNo { .. } => AnswerValue::Bool { value: true },
                    DecisionQuestion::Score { .. } => AnswerValue::Score { value: 0.0 },
                    DecisionQuestion::Choice { .. } => AnswerValue::None,
                };
                DecisionAnswer::new(
                    question,
                    value,
                    1.0,
                    mm_decision::FeatureVector::new(),
                    CoreId::Hosted,
                    0,
                )
            }
        }

        let mut input = input();
        input.action_schema_valid = false;
        let mut ctx = FirewallCtx::new(FirewallConfig::with_core(std::sync::Arc::new(AlwaysFine)));
        let report = evaluate(&input, &mut ctx).await.unwrap();
        assert_eq!(report.outcome, FirewallOutcome::Reject);
        assert_eq!(report.hard_prohibition_id(), Some("schema_invalid_action"));
        assert!(report.decisions.is_empty());
    }

    #[tokio::test]
    async fn an_invalid_input_is_refused_before_anything_runs() {
        let mut input = input();
        input.stakes.level = 2.0;
        let mut ctx = FirewallCtx::new(FirewallConfig::default());
        assert_eq!(
            evaluate(&input, &mut ctx).await.unwrap_err().code(),
            "validation"
        );
    }

    #[tokio::test]
    async fn rails_only_signals_still_reach_an_outcome() {
        let mut input = input();
        input.authorization = AuthorizationRef {
            granted: false,
            scope: "read-only".into(),
            approval_id: None,
        };
        let mut ctx = FirewallCtx::new(FirewallConfig::default());
        let report = evaluate(&input, &mut ctx).await.unwrap();
        assert_eq!(report.outcome, FirewallOutcome::AskUser);
        assert!(!report.has_prohibition());
    }

    #[tokio::test]
    async fn the_report_digest_is_the_input_digest() {
        let input = input();
        let mut ctx = FirewallCtx::new(FirewallConfig::default());
        let report = evaluate(&input, &mut ctx).await.unwrap();
        assert_eq!(report.input_digest, input.digest());
        assert_eq!(report.episode, input.episode);
    }
}
