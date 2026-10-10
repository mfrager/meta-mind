//! The one conformance suite every decision core must pass.
//!
//! It lives in the library rather than in `tests/` because the CLI is the primary
//! caller: `mm-cli conformance` runs exactly this code against the three cores the
//! operator names, and `tests/conformance.rs` is the thin integration test over the
//! same public surface.
//!
//! Phase 9 asks for three interchangeable decision cores — a deterministic rule
//! table, a distilled local head, and a hosted model — behind one `DecisionCore`
//! trait, and for a single suite that decides whether an implementation may be
//! selected. This file is that suite. `mm-cli conformance --core rules --core local
//! --core hosted` runs it through [`run_conformance`], and every property a core is
//! graded on is also reachable as an ordinary `#[test]` below.
//!
//! ## What it checks, and why these six things
//!
//! * **availability** — a core may *answer* or *refuse as unavailable*, and nothing
//!   else. An `Unavailable` refusal is not a defect: the hosted core has no provider
//!   in a test environment and the local core has no head until Phase 11 distils one,
//!   and a suite that failed there would make the gate impossible to pass for
//!   reasons nobody can fix. Any *other* refusal is a hard failure, because a core
//!   whose answer could not be constructed is worse than one that declined.
//! * **typing** — the answer is one the question admitted: the same question ULID,
//!   an option that was offered, a score inside the scale, a boolean for a `yes_no`
//!   and `None` only where the question allowed it.
//! * **confidence_bounds** — `0 <= confidence <= 1` and finite. The firewall's
//!   conformal thresholds mean nothing without this.
//! * **determinism** — the same `(question, state)` asked twice produces a
//!   byte-identical answer. A core whose answer depends on a clock, a counter or a
//!   seed cannot be selected: Phase 11 calibrates on these answers, and
//!   `decision.core.answer` logs a `calibrated_confidence` that has to mean the same
//!   thing twice.
//! * **tie_break** — the same question asked against two states that are *equal but
//!   differently ordered* produces a byte-identical answer. Two callers that hand
//!   over the same facts in a different order describe the same situation, so an
//!   answer that changed with arrival order would make every replay a false diff.
//! * **refusal_shape** — a core never answers a shape it declares it cannot answer.
//!   Only the local core declares one (`choice`), and the check exists because the
//!   alternative — a linear head inventing a ranking over options it cannot compare
//!   — is the one failure a conformance run could not catch by comparison alone.
//!
//! ## Driving the async trait from a synchronously-shaped suite
//!
//! [`DecisionCore::answer`] is `async`, and the suite is a plain function so both the
//! CLI and a `#[test]` can call it. [`block_on`] therefore builds a current-thread
//! runtime when it is not already inside one, and yields to the caller's runtime
//! (`block_in_place`) when it is. That second path needs the multi-threaded
//! scheduler, which is what `#[tokio::main]` in `mm-cli` provides; the tests in this
//! file are plain `#[test]`s on purpose.

use std::collections::BTreeMap;
use std::future::Future;

use crate::core::{CoreId, DecisionCore};
use crate::question::{
    AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState, QuestionKind, ScoreScale,
};
use mm_core::Ulid;
use serde::{Deserialize, Serialize};

/// The core answered or refused only in ways it is allowed to.
pub const CHECK_AVAILABILITY: &str = "availability";
/// The answer is one the question admitted.
pub const CHECK_TYPING: &str = "typing";
/// Every confidence is a real number in `[0,1]`.
pub const CHECK_CONFIDENCE_BOUNDS: &str = "confidence_bounds";
/// The same question and state asked twice give the same bytes.
pub const CHECK_DETERMINISM: &str = "determinism";
/// An equal-but-reordered state gives the same bytes.
pub const CHECK_TIE_BREAK: &str = "tie_break";
/// No core answers a question shape it declares it cannot answer.
pub const CHECK_REFUSAL_SHAPE: &str = "refusal_shape";

/// Every check, in a stable order. The CLI reports them in this order.
pub const CHECKS: [&str; 6] = [
    CHECK_AVAILABILITY,
    CHECK_TYPING,
    CHECK_CONFIDENCE_BOUNDS,
    CHECK_DETERMINISM,
    CHECK_TIE_BREAK,
    CHECK_REFUSAL_SHAPE,
];

/// One check's verdict for one core.
///
/// A `Finding` rather than an assertion because the CLI prints these and exits
/// non-zero, and because a failing core has to say *which* contract it broke.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The core's wire name.
    pub core: String,
    /// Which check this is; one of [`CHECKS`].
    pub check: String,
    /// Whether the core satisfied it.
    pub passed: bool,
    /// Why, in one line. A failure names the case and what was wrong.
    pub detail: String,
}

impl Finding {
    /// A check the core satisfied.
    fn pass(core: &str, check: &str, detail: impl Into<String>) -> Self {
        Finding {
            core: core.to_string(),
            check: check.to_string(),
            passed: true,
            detail: detail.into(),
        }
    }

    /// A check the core did not satisfy.
    fn fail(core: &str, check: &str, detail: impl Into<String>) -> Self {
        Finding {
            core: core.to_string(),
            check: check.to_string(),
            passed: false,
            detail: detail.into(),
        }
    }
}

/// One question asked against one state, named so a failure is actionable.
struct Case {
    /// A short name for the failure message.
    name: &'static str,
    /// The question.
    question: DecisionQuestion,
    /// The state it is asked against.
    state: DecisionState,
}

/// What asking one question produced.
enum Outcome {
    /// The core answered, and the answer passed the construction contract.
    Answered(DecisionAnswer),
    /// The core refused because it cannot answer here. Legal, every time.
    Unavailable(String),
    /// The core refused for any other reason. Not legal: this is the case the suite
    /// exists to catch.
    Refused(String),
}

impl Outcome {
    /// The answer, when there is one.
    fn answer(&self) -> Option<&DecisionAnswer> {
        match self {
            Outcome::Answered(answer) => Some(answer),
            _ => None,
        }
    }

    /// The refusal's reason, for a panic message.
    fn reason(&self) -> String {
        match self {
            Outcome::Answered(answer) => format!("answered {}", answer.answer.canonical()),
            Outcome::Unavailable(reason) | Outcome::Refused(reason) => reason.clone(),
        }
    }
}

/// Drive a future to completion from synchronous code.
///
/// Inside a runtime the only sound way to wait is `block_in_place`, which requires
/// the multi-threaded scheduler — the shape `mm-cli`'s `#[tokio::main]` has. Outside
/// a runtime a current-thread runtime is built, which is the shape a `#[test]` has.
fn block_on<F: Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime can always be built")
            .block_on(future),
    }
}

/// Ask one question and classify what came back.
fn ask(core: &dyn DecisionCore, question: &DecisionQuestion, state: &DecisionState) -> Outcome {
    match block_on(core.answer(question, state)) {
        Ok(answer) => Outcome::Answered(answer),
        Err(error) if crate::is_legal_refusal(&error) => Outcome::Unavailable(error.to_string()),
        Err(error) => Outcome::Refused(format!("{} ({})", error, error.code())),
    }
}

/// Whether a core kind claims it can answer a question shape at all.
///
/// This is the one place a core's *identity* — as opposed to its behaviour — is
/// consulted, and it is why [`CoreId`] is part of the trait: a local core is a
/// single linear head over named features, so it can score a bounded judgment and
/// answer a yes/no, but it has no way to rank a set of options, and a conformance
/// run must not accept one that pretends it has.
fn declared_shape_supports(core: CoreId, kind: QuestionKind) -> bool {
    match core {
        CoreId::Rules | CoreId::Hosted => true,
        CoreId::Local => !matches!(kind, QuestionKind::Choice),
    }
}

fn ulid(n: u128) -> Ulid {
    Ulid::from_parts(1_700_000_000_000, n)
}

/// The state the battery is asked against.
fn state(stakes: f64) -> DecisionState {
    DecisionState::with_refs(
        ulid(900),
        vec![ulid(903), ulid(901), ulid(902)],
        vec![ulid(911), ulid(910)],
        vec!["no-network".to_string(), "no-writes".to_string()],
        serde_json::json!({
            "stakes": stakes,
            "irreversibility": stakes / 2.0,
            "facts_count": 3,
        }),
    )
}

/// A state with the same content as [`state`] but the lists in reverse order.
fn reorder(state: &DecisionState) -> DecisionState {
    let mut facts = state.facts.clone();
    let mut assumptions = state.assumptions.clone();
    let mut constraints = state.constraints.clone();
    facts.reverse();
    assumptions.reverse();
    constraints.reverse();
    DecisionState::with_refs(
        state.episode,
        facts,
        assumptions,
        constraints,
        state.context.clone(),
    )
}

/// The fixed battery of bounded questions.
///
/// One battery for every core, on purpose: two cores graded on different questions
/// would not be comparable, and comparability is the whole point of having one
/// suite. The battery deliberately includes
///
/// * a question no rule matches, so the fallback path is exercised;
/// * a question whose prompt matches several rules, so the priority ordering is;
/// * a `choice` that forbids "none" and has no admissible rule, which is the one
///   question a rule table must refuse rather than answer;
/// * a `score` on a non-unit scale, because a rule's answer is only admissible if it
///   lands inside the question's own interval.
fn battery() -> Vec<Case> {
    vec![
        Case {
            name: "yes_no/safety",
            question: DecisionQuestion::YesNo {
                id: ulid(1),
                prompt: "Is this action safe to proceed with as described?".into(),
            },
            state: state(0.4),
        },
        Case {
            name: "yes_no/authorization",
            question: DecisionQuestion::YesNo {
                id: ulid(2),
                prompt: "Is the authorization for this action sufficient?".into(),
            },
            state: state(0.4),
        },
        Case {
            name: "yes_no/irreversible_without_approval",
            question: DecisionQuestion::YesNo {
                id: ulid(3),
                prompt: "Is this irreversible action missing an approval record?".into(),
            },
            state: state(0.8),
        },
        Case {
            name: "score/residual_risk",
            question: DecisionQuestion::Score {
                id: ulid(4),
                prompt: "How much residual risk remains after the stated mitigations?".into(),
                scale: ScoreScale::unit(),
            },
            state: state(0.5),
        },
        Case {
            name: "score/non_unit_scale",
            question: DecisionQuestion::Score {
                id: ulid(5),
                prompt: "How large is the blast radius on a ten-point scale?".into(),
                scale: ScoreScale::new(0.0, 10.0),
            },
            state: state(0.7),
        },
        Case {
            name: "choice/multi_rule",
            question: DecisionQuestion::Choice {
                id: ulid(6),
                prompt: "A regression appeared and roll back is available, but the change is \
                         deferrable; which mitigation should be applied?"
                    .into(),
                options: vec!["roll back".into(), "defer".into()],
                allow_none: true,
            },
            state: state(0.6),
        },
        Case {
            name: "choice/ambiguous_intent",
            question: DecisionQuestion::Choice {
                id: ulid(7),
                prompt: "The intent is ambiguous; which option should be taken?".into(),
                options: vec!["ask the user".into(), "defer".into()],
                allow_none: false,
            },
            state: state(0.5),
        },
        Case {
            name: "choice/no_admissible_rule",
            question: DecisionQuestion::Choice {
                id: ulid(8),
                prompt: "Which outcome should be recorded in the ledger for this run?".into(),
                options: vec!["proceed".into(), "abort".into()],
                allow_none: false,
            },
            state: state(0.3),
        },
    ]
}

/// What one case produced, every time it was asked.
struct Asked<'a> {
    /// The case, for the failure message.
    name: &'static str,
    /// The question that was asked.
    question: &'a DecisionQuestion,
    /// The first ask.
    first: Outcome,
    /// The second ask, against the same state.
    second: Outcome,
    /// The ask against the equal-but-reordered state.
    reordered: Outcome,
}

/// Run the suite against one core and return the findings, never panicking.
///
/// Six findings, one per check in [`CHECKS`] order. Nothing here panics on a bad
/// core: a misbehaving core produces failed findings, because the CLI has to be able
/// to grade three cores in one run and report all of them.
pub fn run_conformance(core: &dyn DecisionCore) -> Vec<Finding> {
    let name = core.id().as_str().to_string();
    let cases = battery();

    let asked: Vec<Asked<'_>> = cases
        .iter()
        .map(|case| Asked {
            name: case.name,
            question: &case.question,
            first: ask(core, &case.question, &case.state),
            second: ask(core, &case.question, &case.state),
            reordered: ask(core, &case.question, &reorder(&case.state)),
        })
        .collect();

    vec![
        availability(&name, &asked),
        typing(&name, &asked),
        confidence_bounds(&name, &asked),
        determinism(&name, &asked),
        tie_break(&name, &asked),
        refusal_shape(&name, core.id(), &asked),
    ]
}

/// True when every finding passed.
pub fn findings_passed(findings: &[Finding]) -> bool {
    findings.iter().all(|finding| finding.passed)
}

/// One human line for one core: its name and how many checks it passed.
pub fn summary(core: &dyn DecisionCore) -> String {
    let findings = run_conformance(core);
    let passed = findings.iter().filter(|finding| finding.passed).count();
    format!(
        "{}: {passed}/{} checks passed",
        core.id().as_str(),
        findings.len()
    )
}

// --------------------------------------------------------------- the checks ----

/// A core may answer or refuse as unavailable; nothing else is legal.
///
/// An entirely unavailable core is a *passing* run: "the hosted core has no provider
/// here" and "the local core has no distilled head yet" are both normal states this
/// phase defined, and the firewall is built to degrade from them. What is not legal
/// is a refusal of any other kind — that is a core whose answer could not be
/// constructed, and such an answer must never reach the aggregation.
fn availability(core: &str, asked: &[Asked<'_>]) -> Finding {
    let mut failures: Vec<String> = Vec::new();
    let mut refusals: Vec<String> = Vec::new();
    let mut counts: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for entry in asked {
        let slot = counts
            .entry(entry.question.kind().as_str())
            .or_insert((0, 0, 0));
        match &entry.first {
            Outcome::Answered(_) => slot.0 += 1,
            Outcome::Unavailable(_) => {
                slot.1 += 1;
                // The reason is carried into the report rather than discarded. It is
                // not a failure, but it is the one thing an operator reading "the
                // hosted core did not answer" actually needs: *why* not.
                refusals.push(format!("{}: {}", entry.name, entry.first.reason()));
            }
            Outcome::Refused(reason) => {
                slot.2 += 1;
                failures.push(format!("{}: refused with {reason}", entry.name));
            }
        }
    }
    let observed: Vec<String> = counts
        .iter()
        .map(|(kind, (answered, unavailable, refused))| {
            format!("{kind}: {answered} answered, {unavailable} unavailable, {refused} refused")
        })
        .collect();
    let totals = format!(
        "{} cases: {} answered, {} unavailable, {} refused",
        asked.len(),
        counts.values().map(|slot| slot.0).sum::<usize>(),
        counts.values().map(|slot| slot.1).sum::<usize>(),
        counts.values().map(|slot| slot.2).sum::<usize>()
    );
    let mut detail = format!("{totals} [{}]", observed.join("; "));
    if !refusals.is_empty() {
        detail.push_str(&format!(" unavailable because: {}", refusals.join("; ")));
    }
    if failures.is_empty() {
        Finding::pass(core, CHECK_AVAILABILITY, detail)
    } else {
        Finding::fail(
            core,
            CHECK_AVAILABILITY,
            format!("{detail} {}", failures.join("; ")),
        )
    }
}

/// Every answer is one the question admitted.
///
/// The question id is compared against the question the suite asked, so this is a
/// check on the answer rather than a restatement of `DecisionAnswer::new`.
fn typing(core: &str, asked: &[Asked<'_>]) -> Finding {
    let mut failures: Vec<String> = Vec::new();
    for entry in asked {
        let Some(answer) = entry.first.answer() else {
            continue;
        };
        let kind = entry.question.kind();
        if answer.question_kind != kind {
            failures.push(format!(
                "{}: question_kind {} does not match the question's {kind}",
                entry.name, answer.question_kind
            ));
        }
        if answer.question != entry.question.id() {
            failures.push(format!(
                "{}: answered question {} but was asked {}",
                entry.name,
                mm_core::ulid_string(&answer.question),
                mm_core::ulid_string(&entry.question.id())
            ));
        }
        match (entry.question, &answer.answer) {
            (DecisionQuestion::Choice { options, .. }, AnswerValue::Option { value }) => {
                if !options.iter().any(|option| option == value) {
                    failures.push(format!("{}: option {value:?} was not offered", entry.name));
                }
            }
            (DecisionQuestion::Choice { allow_none, .. }, AnswerValue::None) => {
                if !allow_none {
                    failures.push(format!(
                        "{}: answered none where none was forbidden",
                        entry.name
                    ));
                }
            }
            (DecisionQuestion::Choice { .. }, other) => {
                failures.push(format!(
                    "{}: a choice question cannot be answered with {}",
                    entry.name,
                    other.canonical()
                ));
            }
            (DecisionQuestion::Score { scale, .. }, AnswerValue::Score { value }) => {
                if !value.is_finite() || *value < scale.min || *value > scale.max {
                    failures.push(format!(
                        "{}: score {value} is outside {}..{}",
                        entry.name, scale.min, scale.max
                    ));
                }
            }
            (DecisionQuestion::Score { .. }, other) => {
                failures.push(format!(
                    "{}: a score question cannot be answered with {}",
                    entry.name,
                    other.canonical()
                ));
            }
            (DecisionQuestion::YesNo { .. }, AnswerValue::Bool { .. }) => {}
            (DecisionQuestion::YesNo { .. }, other) => {
                failures.push(format!(
                    "{}: a yes_no question cannot be answered with {}",
                    entry.name,
                    other.canonical()
                ));
            }
        }
    }
    if failures.is_empty() {
        Finding::pass(
            core,
            CHECK_TYPING,
            format!(
                "every answer was admissible for its question ({} cases)",
                asked.len()
            ),
        )
    } else {
        Finding::fail(core, CHECK_TYPING, failures.join("; "))
    }
}

/// Every confidence is a finite real number in `[0,1]`.
fn confidence_bounds(core: &str, asked: &[Asked<'_>]) -> Finding {
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for entry in asked {
        if let Some(answer) = entry.first.answer() {
            checked += 1;
            if !answer.confidence.is_finite() || !(0.0..=1.0).contains(&answer.confidence) {
                failures.push(format!(
                    "{}: confidence {} is outside [0,1]",
                    entry.name, answer.confidence
                ));
            }
        }
    }
    if failures.is_empty() {
        Finding::pass(
            core,
            CHECK_CONFIDENCE_BOUNDS,
            format!("{checked} confidences are in [0,1]"),
        )
    } else {
        Finding::fail(core, CHECK_CONFIDENCE_BOUNDS, failures.join("; "))
    }
}

/// The same question and state asked twice gives the same bytes.
fn determinism(core: &str, asked: &[Asked<'_>]) -> Finding {
    let mut failures: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for entry in asked {
        match (entry.first.answer(), entry.second.answer()) {
            (Some(first), Some(second)) => {
                compared += 1;
                if first.canonical() != second.canonical() {
                    failures.push(format!(
                        "{}: the second ask differed\n  first:  {}\n  second: {}",
                        entry.name,
                        first.canonical(),
                        second.canonical()
                    ));
                }
            }
            (None, None) => {}
            _ => failures.push(format!(
                "{}: answered one time and not the other",
                entry.name
            )),
        }
    }
    if failures.is_empty() {
        Finding::pass(
            core,
            CHECK_DETERMINISM,
            format!("{compared} answers are byte-identical across two asks"),
        )
    } else {
        Finding::fail(core, CHECK_DETERMINISM, failures.join("; "))
    }
}

/// An equal-but-reordered state gives the same bytes.
///
/// The two states differ only in the order their lists arrived in, so a core whose
/// answer changed has answered a question about the *caller* rather than about the
/// situation. This is the same property the rule table's `(priority, id)` tie-break
/// provides inside the table: any remaining ambiguity has to be resolved by
/// something stable. Every priority in the embedded table is distinct, so it has no
/// equal-priority pair to break — rather than skip the check silently the suite
/// records that, and the id-ascending tie-break itself is asserted directly by
/// `same_priority_rules_break_the_tie_by_id` below.
fn tie_break(core: &str, asked: &[Asked<'_>]) -> Finding {
    let mut failures: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for entry in asked {
        match (entry.first.answer(), entry.reordered.answer()) {
            (Some(first), Some(reordered)) => {
                compared += 1;
                if first.canonical() != reordered.canonical() {
                    failures.push(format!(
                        "{}: reordering the state changed the answer\n  in order:  {}\n  reordered: {}",
                        entry.name,
                        first.canonical(),
                        reordered.canonical()
                    ));
                }
            }
            (None, None) => {}
            _ => failures.push(format!(
                "{}: answered for one state order and not the other",
                entry.name
            )),
        }
    }
    if failures.is_empty() {
        Finding::pass(
            core,
            CHECK_TIE_BREAK,
            format!(
                "{compared} answers are independent of the state's list order; the embedded \
                 table has no equal-priority pair, so its id-ascending tie-break is asserted \
                 by `same_priority_rules_break_the_tie_by_id`"
            ),
        )
    } else {
        Finding::fail(core, CHECK_TIE_BREAK, failures.join("; "))
    }
}

/// No core answers a shape it declares it cannot answer.
fn refusal_shape(core: &str, core_id: CoreId, asked: &[Asked<'_>]) -> Finding {
    let mut failures: Vec<String> = Vec::new();
    for entry in asked {
        let kind = entry.question.kind();
        if entry.first.answer().is_some() && !declared_shape_supports(core_id, kind) {
            failures.push(format!(
                "{}: answered a {kind} question, which a {core_id} core declares it cannot answer",
                entry.name
            ));
        }
    }
    let supported: Vec<&str> = [
        QuestionKind::Choice,
        QuestionKind::Score,
        QuestionKind::YesNo,
    ]
    .into_iter()
    .filter(|kind| declared_shape_supports(core_id, *kind))
    .map(|kind| kind.as_str())
    .collect();
    if failures.is_empty() {
        Finding::pass(
            core,
            CHECK_REFUSAL_SHAPE,
            format!(
                "declares {}; refused but never answered the rest",
                supported.join(", ")
            ),
        )
    } else {
        Finding::fail(core, CHECK_REFUSAL_SHAPE, failures.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question::FeatureVector;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use crate::error::DecisionError;
    use crate::local::{DecisionHead, LocalCore};
    use crate::rules::{RuleTable, RulesCore};
    use mm_llm::mock::MockClient;
    use mm_llm::schema::SchemaRegistry;

    fn findings_for(core: &dyn DecisionCore) -> Vec<Finding> {
        run_conformance(core)
    }

    fn named<'a>(findings: &'a [Finding], check: &str) -> &'a Finding {
        findings
            .iter()
            .find(|finding| finding.check == check)
            .unwrap_or_else(|| panic!("no `{check}` finding in {findings:?}"))
    }

    #[test]
    fn rules_core_conforms() {
        let core = RulesCore::embedded().unwrap();
        let findings = findings_for(&core);
        assert!(findings_passed(&findings), "{findings:#?}");
        assert_eq!(findings.len(), CHECKS.len());
        // The rules core is the one core that is always available, so it answers the
        // whole battery except the `choice` that forbids "none" and matches no rule.
        assert!(
            named(&findings, CHECK_AVAILABILITY)
                .detail
                .contains("8 cases: 7 answered, 1 unavailable, 0 refused"),
            "{:#?}",
            named(&findings, CHECK_AVAILABILITY)
        );
    }

    #[test]
    fn local_core_without_a_head_conforms() {
        let core = LocalCore::unavailable();
        let findings = findings_for(&core);
        // An unavailable core is not a failing core: it refuses every question with
        // `Unavailable`, which is exactly what "no head is loaded" means, and the
        // firewall is built to escalate from there rather than receive a guess.
        assert!(findings_passed(&findings), "{findings:#?}");
        assert_eq!(findings.len(), CHECKS.len());
        assert!(named(&findings, CHECK_AVAILABILITY)
            .detail
            .contains("8 cases: 0 answered, 8 unavailable, 0 refused"));
    }

    #[test]
    fn local_core_with_a_head_conforms() {
        let dir = tempfile::tempdir().unwrap();
        let path: PathBuf = dir.path().join("head.json");
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
        assert!(core.head().is_some());
        let findings = findings_for(&core);
        assert!(findings_passed(&findings), "{findings:#?}");
        // This head is a `yes_no` head, so it answers the three yes/no cases and
        // refuses the two score cases (a head declares the shape it was distilled
        // for) and the three choice cases (a linear head ranks a decision, not a set
        // of options). Both refusals are `Unavailable`, which is why the run still
        // conforms.
        assert!(
            named(&findings, CHECK_AVAILABILITY)
                .detail
                .contains("8 cases: 3 answered, 5 unavailable, 0 refused"),
            "{:?}",
            named(&findings, CHECK_AVAILABILITY)
        );
    }

    #[test]
    fn hosted_core_without_a_registry_conforms() {
        let core = crate::hosted::HostedCore::new(
            Arc::new(MockClient::new()),
            Arc::new(SchemaRegistry::new()),
            256,
        );
        let findings = findings_for(&core);
        // The registry has no reply schema, so the core refuses *before* spending a
        // call. That is a legal refusal, and a conformance run that failed here would
        // make the gate impossible to pass without a provider.
        assert!(findings_passed(&findings), "{findings:#?}");
        assert!(named(&findings, CHECK_AVAILABILITY)
            .detail
            .contains("8 cases: 0 answered, 8 unavailable, 0 refused"));
    }

    /// A core whose answer changes between two asks of the same question.
    struct NondeterministicCore {
        calls: AtomicU32,
    }

    #[async_trait::async_trait]
    impl DecisionCore for NondeterministicCore {
        fn id(&self) -> CoreId {
            CoreId::Rules
        }

        async fn answer(
            &self,
            question: &DecisionQuestion,
            _state: &DecisionState,
        ) -> crate::Result<DecisionAnswer> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let odd = n % 2 == 1;
            let answer = match question {
                DecisionQuestion::YesNo { .. } => AnswerValue::Bool { value: odd },
                DecisionQuestion::Score { scale, .. } => AnswerValue::Score {
                    value: if odd { scale.min } else { scale.max },
                },
                DecisionQuestion::Choice { options, .. } => AnswerValue::Option {
                    value: options[0].clone(),
                },
            };
            DecisionAnswer::new(
                question,
                answer,
                0.5 + if odd { 0.4 } else { 0.0 },
                FeatureVector::new().with("calls", f64::from(n)),
                CoreId::Rules,
                0,
            )
        }
    }

    #[test]
    fn a_nondeterministic_core_fails() {
        let core = NondeterministicCore {
            calls: AtomicU32::new(0),
        };
        let findings = findings_for(&core);
        assert!(!findings_passed(&findings), "{findings:#?}");
        let determinism = named(&findings, CHECK_DETERMINISM);
        assert!(!determinism.passed, "{determinism:#?}");
        assert!(
            determinism.detail.contains("the second ask differed"),
            "{determinism:#?}"
        );
        // The failure is specific: everything a nondeterministic answer does not
        // touch still passes, so an operator can act on the finding rather than
        // re-reading six of them.
        assert!(named(&findings, CHECK_TYPING).passed);
        assert!(named(&findings, CHECK_CONFIDENCE_BOUNDS).passed);
    }

    /// A core that answers the question it was *not* asked.
    struct WrongQuestionCore;

    fn with_different_id(question: &DecisionQuestion) -> DecisionQuestion {
        match question {
            DecisionQuestion::Choice {
                prompt,
                options,
                allow_none,
                ..
            } => DecisionQuestion::Choice {
                id: ulid(7001),
                prompt: prompt.clone(),
                options: options.clone(),
                allow_none: *allow_none,
            },
            DecisionQuestion::Score { prompt, scale, .. } => DecisionQuestion::Score {
                id: ulid(7002),
                prompt: prompt.clone(),
                scale: scale.clone(),
            },
            DecisionQuestion::YesNo { prompt, .. } => DecisionQuestion::YesNo {
                id: ulid(7003),
                prompt: prompt.clone(),
            },
        }
    }

    #[async_trait::async_trait]
    impl DecisionCore for WrongQuestionCore {
        fn id(&self) -> CoreId {
            CoreId::Rules
        }

        async fn answer(
            &self,
            question: &DecisionQuestion,
            _state: &DecisionState,
        ) -> crate::Result<DecisionAnswer> {
            let decoy = with_different_id(question);
            let answer = match &decoy {
                DecisionQuestion::Choice { options, .. } => AnswerValue::Option {
                    value: options[0].clone(),
                },
                DecisionQuestion::Score { scale, .. } => AnswerValue::Score { value: scale.min },
                DecisionQuestion::YesNo { .. } => AnswerValue::Bool { value: false },
            };
            DecisionAnswer::new(&decoy, answer, 0.5, FeatureVector::new(), CoreId::Rules, 0)
        }
    }

    #[test]
    fn a_core_that_breaks_typing_fails() {
        let findings = findings_for(&WrongQuestionCore);
        assert!(!findings_passed(&findings), "{findings:#?}");
        let typing = named(&findings, CHECK_TYPING);
        assert!(!typing.passed, "{typing:#?}");
        assert!(typing.detail.contains("was asked"), "{typing:#?}");
    }

    /// A core that refuses for a reason that is not "unavailable".
    struct BadRefusalCore;

    #[async_trait::async_trait]
    impl DecisionCore for BadRefusalCore {
        fn id(&self) -> CoreId {
            CoreId::Hosted
        }

        async fn answer(
            &self,
            _question: &DecisionQuestion,
            _state: &DecisionState,
        ) -> crate::Result<DecisionAnswer> {
            Err(DecisionError::Internal("the core is a stub".into()))
        }
    }

    #[test]
    fn a_refusal_that_is_not_unavailable_fails() {
        let findings = findings_for(&BadRefusalCore);
        assert!(!findings_passed(&findings), "{findings:#?}");
        let availability = named(&findings, CHECK_AVAILABILITY);
        assert!(!availability.passed, "{availability:#?}");
        assert!(
            availability.detail.contains("refused with"),
            "{availability:#?}"
        );
    }

    /// Two rules at the same priority, in two file orders.
    ///
    /// The first table deliberately lists the *later* id first, so a core that
    /// answered by iteration order instead of by id would answer `beta option`.
    const TIE_TOML_BETA_FIRST: &str = r#"
version = "tie"
default_confidence = 0.1

[[rules]]
id = "route.beta"
priority = 50
kind = "choice"
keywords = ["tie"]
answer = { kind = "option", value = "beta option" }
confidence = 0.5

[[rules]]
id = "route.alpha"
priority = 50
kind = "choice"
keywords = ["tie"]
answer = { kind = "option", value = "alpha option" }
confidence = 0.5
"#;

    /// The same two rules with the blocks the other way round.
    const TIE_TOML_ALPHA_FIRST: &str = r#"
version = "tie"
default_confidence = 0.1

[[rules]]
id = "route.alpha"
priority = 50
kind = "choice"
keywords = ["tie"]
answer = { kind = "option", value = "alpha option" }
confidence = 0.5

[[rules]]
id = "route.beta"
priority = 50
kind = "choice"
keywords = ["tie"]
answer = { kind = "option", value = "beta option" }
confidence = 0.5
"#;

    #[test]
    fn same_priority_rules_break_the_tie_by_id() {
        let question = DecisionQuestion::Choice {
            id: ulid(42),
            prompt: "tie".into(),
            options: vec!["alpha option".into(), "beta option".into()],
            allow_none: true,
        };
        let status = state(0.5);

        for toml in [TIE_TOML_BETA_FIRST, TIE_TOML_ALPHA_FIRST] {
            let core = RulesCore::new(RuleTable::from_toml(toml).unwrap()).unwrap();
            let outcome = ask(&core, &question, &status);
            let answer = outcome
                .answer()
                .unwrap_or_else(|| panic!("the tie-break core refused: {}", outcome.reason()));
            assert_eq!(
                answer.answer.canonical(),
                "option:alpha option",
                "the id-ascending tie-break must win whatever the file order"
            );
        }
    }

    #[test]
    fn the_check_list_is_stable() {
        let unique: std::collections::BTreeSet<&str> = CHECKS.into_iter().collect();
        assert_eq!(unique.len(), CHECKS.len(), "a check name is repeated");
        let core = RulesCore::embedded().unwrap();
        assert!(summary(&core).starts_with("rules: "), "{}", summary(&core));
        let findings = findings_for(&core);
        let names: Vec<&str> = findings.iter().map(|f| f.check.as_str()).collect();
        assert_eq!(names, CHECKS.to_vec());
        assert!(findings.iter().all(|f| f.core == "rules"));
    }

    /// The head deserialises from the shape the JSON artifact uses, so a suite that
    /// built a head in memory could not drift from one loaded from disk.
    #[test]
    fn a_head_round_trips_through_json() {
        let head = DecisionHead {
            class: "mm.safety".into(),
            question_kind: QuestionKind::YesNo,
            weights: BTreeMap::from([("state.facts".to_string(), 0.5)]),
            bias: -0.5,
            threshold: 0.5,
            confidence: 0.9,
        };
        let text = serde_json::to_string(&head).unwrap();
        let back: DecisionHead = serde_json::from_str(&text).unwrap();
        assert_eq!(back, head);
        assert!(text.contains("\"yes_no\""));
    }
}
