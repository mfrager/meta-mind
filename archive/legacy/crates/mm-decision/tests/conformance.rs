//! The conformance gate, exercised through the crate's public surface.
//!
//! Phase 9 requires *one* suite that decides whether a decision core may be
//! selected, run against all three implementations, and it requires that the suite
//! be able to *fail*. The suite itself lives in `mm_decision::conformance`, because
//! `mm-cli conformance` is its primary caller and an integration test cannot be
//! linked into a binary; this file is the integration test over that public surface.
//!
//! Two things are checked here that the suite's own unit tests cannot check for it:
//!
//! * the three real cores an operator can name — `rules`, a head-bearing `local`,
//!   and `hosted` with no provider — all conform;
//! * the suite's verdict is *load-bearing*: a core that answers differently on
//!   every call, and a core that answers a question it was not asked, both fail.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use mm_core::Ulid;
use mm_decision::conformance::{
    findings_passed, run_conformance, summary, Finding, CHECKS, CHECK_DETERMINISM, CHECK_TYPING,
};
use mm_decision::core::{CoreId, DecisionCore};
use mm_decision::question::{
    AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState, FeatureVector,
};
use mm_decision::{HostedCore, LocalCore, RulesCore};
use mm_llm::mock::MockClient;
use mm_llm::schema::SchemaRegistry;

fn question(kind: &str) -> DecisionQuestion {
    match kind {
        "yes_no" => DecisionQuestion::YesNo {
            id: Ulid::from_parts(1_700_000_000_000, 1),
            prompt: "is the comparison evidence valid and comparable?".into(),
        },
        _ => unreachable!(),
    }
}

fn state() -> DecisionState {
    DecisionState::new(
        Ulid::from_parts(1_700_000_000_000, 2),
        serde_json::json!({}),
    )
}

fn finding<'a>(findings: &'a [Finding], check: &str) -> &'a Finding {
    findings
        .iter()
        .find(|finding| finding.check == check)
        .unwrap_or_else(|| panic!("no {check} finding in {findings:#?}"))
}

/// The rules core is always available, so its run is the strongest statement the
/// suite can make about a core.
#[test]
fn the_rules_core_conforms() {
    let core = RulesCore::embedded().expect("the embedded table parses and validates");
    let findings = run_conformance(&core);
    assert!(findings_passed(&findings), "{findings:#?}");
    assert_eq!(findings.len(), CHECKS.len());
    for check in CHECKS {
        assert!(finding(&findings, check).passed, "{check} failed");
    }
    assert!(summary(&core).starts_with("rules:"));
}

/// A head-bearing local core answers the shapes a head can answer and refuses the
/// one it cannot, and both are legal.
#[test]
fn a_head_bearing_local_core_conforms() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("head.json");
    let head = serde_json::json!({
        "class": "mm.safety",
        "question_kind": "yes_no",
        "weights": { "state.facts": 0.5, "context.stakes": 2.0 },
        "bias": -0.5,
        "threshold": 0.5,
        "confidence": 0.9
    });
    std::fs::write(&path, serde_json::to_string(&head).unwrap()).unwrap();
    let core = LocalCore::from_path(&path).unwrap();
    assert!(core.head().is_some(), "the head must have loaded");
    let findings = run_conformance(&core);
    assert!(findings_passed(&findings), "{findings:#?}");
}

/// A local core with no head refuses everything, and that is a *passing* run: the
/// adapter's whole job is to refuse rather than to fabricate, and a suite that
/// failed here would make the adapter's correct behaviour a build failure.
#[test]
fn an_unavailable_local_core_conforms() {
    let findings = run_conformance(&LocalCore::unavailable());
    assert!(findings_passed(&findings), "{findings:#?}");
    let availability = finding(&findings, "availability");
    assert!(
        availability.detail.contains("0 answered"),
        "{}",
        availability.detail
    );
}

/// A hosted core with no registered schema refuses before spending a call, and that
/// is the same legal refusal. Without this, the gate's `--core hosted` leg would be
/// unrunnable in an environment with no provider.
#[test]
fn a_hosted_core_without_a_provider_conforms() {
    let core = HostedCore::new(
        Arc::new(MockClient::new()),
        Arc::new(SchemaRegistry::new()),
        256,
    );
    let findings = run_conformance(&core);
    assert!(findings_passed(&findings), "{findings:#?}");
    assert!(finding(&findings, "availability")
        .detail
        .contains("unavailable"));
}

/// A core that returns a different answer each call must not pass: Phase 11
/// calibrates on these answers, and a confidence that means something different
/// every time is not a confidence.
#[test]
fn a_nondeterministic_core_fails() {
    struct Flipping(AtomicU32);

    #[async_trait]
    impl DecisionCore for Flipping {
        fn id(&self) -> CoreId {
            CoreId::Hosted
        }

        async fn answer(
            &self,
            question: &DecisionQuestion,
            _state: &DecisionState,
        ) -> mm_decision::Result<DecisionAnswer> {
            let n = self.0.fetch_add(1, Ordering::SeqCst);
            DecisionAnswer::new(
                question,
                AnswerValue::Bool {
                    value: n.is_multiple_of(2),
                },
                0.5,
                FeatureVector::new(),
                CoreId::Hosted,
                0,
            )
        }
    }

    let core = Flipping(AtomicU32::new(0));
    let findings = run_conformance(&core);
    assert!(!findings_passed(&findings));
    assert!(!finding(&findings, CHECK_DETERMINISM).passed);
}

/// A core that answers about a question it was not asked must not pass. This is the
/// failure `DecisionAnswer::new` cannot catch by itself, because the answer is
/// well-formed — it is simply the answer to something else.
#[test]
fn a_core_that_answers_the_wrong_question_fails() {
    struct Shifting;

    #[async_trait]
    impl DecisionCore for Shifting {
        fn id(&self) -> CoreId {
            CoreId::Hosted
        }

        async fn answer(
            &self,
            _question: &DecisionQuestion,
            _state: &DecisionState,
        ) -> mm_decision::Result<DecisionAnswer> {
            let other = question("yes_no");
            DecisionAnswer::new(
                &other,
                AnswerValue::Bool { value: true },
                0.9,
                FeatureVector::new(),
                CoreId::Hosted,
                0,
            )
        }
    }

    let findings = run_conformance(&Shifting);
    assert!(!findings_passed(&findings));
    assert!(!finding(&findings, CHECK_TYPING).passed);
}

#[test]
fn the_check_list_is_stable_and_unique() {
    let mut sorted = CHECKS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), CHECKS.len(), "a check name is repeated");
    let core = RulesCore::embedded().unwrap();
    assert!(summary(&core).contains("rules"));
    assert!(summary(&core).contains(&format!("{}", CHECKS.len())));
}

#[test]
fn the_state_used_by_the_suite_is_the_one_it_declares() {
    // A guard on the fixture itself: if the state ever grew a non-deterministic
    // field (a clock, a random), the suite's determinism check would be measuring
    // the fixture rather than the core.
    assert_eq!(state().digest(), state().digest());
    assert_eq!(question("yes_no").kind().as_str(), "yes_no");
}
