//! The behaviour this module promises, exercised through the compare module it
//! delegates to.
//!
//! The contract module deliberately holds no arithmetic of its own, so a test that
//! only read `surfaces()` would prove nothing about the promise. These tests take
//! the promise at face value: a matched pair is `Valid`, a pair with a different
//! objective is `Rejected`, a pair with disjoint timeframes is `NonComparable`, and
//! the normalized values are produced only for the pair that may be compared.

use std::collections::BTreeMap;

use comparison_integrity::{compare_contract, surfaces, COMPARE_CONTRACT, INVARIANT};
use mm_core::Ulid;
use mm_decision::compare::{
    check_contract, ComparisonContract, ComparisonVerdict, Condition, Dimension, Timeframe, Unit,
};

fn ulid(n: u128) -> Ulid {
    Ulid::from_parts(1_700_000_000_000, n)
}

fn contract(objective: &str, start: &str, end: &str) -> ComparisonContract {
    ComparisonContract {
        objective: objective.into(),
        object_class: "storage backend".into(),
        objects: vec!["candidate".into()],
        dimensions: vec![Dimension {
            name: "p99 latency".into(),
            unit: Some("ms".into()),
            higher_is_better: false,
        }],
        units: vec![Unit {
            name: "ms".into(),
            dimension: "duration".into(),
            to_base: 0.001,
        }],
        timeframe: Timeframe {
            start: start.into(),
            end: end.into(),
        },
        conditions: Vec::new(),
        constraints: Vec::new(),
        definitions: BTreeMap::new(),
        evidence: vec![ulid(9)],
        values: BTreeMap::from([("p99 latency".to_string(), 9.0)]),
    }
}

#[test]
fn the_contract_names_one_surface_and_its_invariant() {
    let surfaces = surfaces();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces[0].function, COMPARE_CONTRACT);
    assert_eq!(compare_contract().invariant, Some(INVARIANT));
    assert_eq!(surfaces[0].writes, &["comparisons"]);
}

#[test]
fn a_matched_pair_is_valid_and_normalizes() {
    let a = contract("choose the smaller p99 latency", "2026-01-01", "2026-02-01");
    let b = contract("choose the smaller p99 latency", "2026-01-01", "2026-02-01");
    let check = check_contract(&a, &b);
    assert_eq!(check.verdict, ComparisonVerdict::Valid);
    assert!(check.violations.is_empty());
    assert!(check.normalized.is_some());
}

#[test]
fn a_different_objective_is_rejected_rather_than_coerced() {
    let a = contract("choose the smaller p99 latency", "2026-01-01", "2026-02-01");
    let b = contract("choose the cheaper p99 latency", "2026-01-01", "2026-02-01");
    let check = check_contract(&a, &b);
    assert_eq!(check.verdict, ComparisonVerdict::Rejected);
    assert!(check.normalized.is_none(), "a rejection has no numbers");
    assert!(check
        .violations
        .iter()
        .any(|violation| violation.code == "objective_mismatch"));
}

#[test]
fn disjoint_timeframes_are_non_comparable() {
    let a = contract("choose the smaller p99 latency", "2026-01-01", "2026-02-01");
    let b = contract("choose the smaller p99 latency", "2026-06-01", "2026-07-01");
    let check = check_contract(&a, &b);
    assert_eq!(check.verdict, ComparisonVerdict::NonComparable);
    assert!(check
        .violations
        .iter()
        .any(|violation| violation.code == "timeframe_disjoint"));
}

#[test]
fn a_differing_operating_condition_is_non_comparable() {
    let mut a = contract("choose the smaller p99 latency", "2026-01-01", "2026-02-01");
    let mut b = a.clone();
    a.conditions = vec![Condition {
        name: "load".into(),
        value: "steady".into(),
    }];
    b.conditions = vec![Condition {
        name: "load".into(),
        value: "burst".into(),
    }];
    let check = check_contract(&a, &b);
    assert_eq!(check.verdict, ComparisonVerdict::NonComparable);
}
