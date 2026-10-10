//! `applicability_gold.rs` — the ranker matches the committed table, exactly.
//!
//! The plan's invariant 5 is "deterministic selection", and a claim about
//! determinism is only worth anything if it is checked against a *committed*
//! artefact. `bench/library/gold_applicability.jsonl` was generated from the
//! implementation and is edited only when the arithmetic changes on purpose, so a
//! drift in the blend, the fitness average or the contraindication penalty fails
//! here rather than in a gate run.
//!
//! Everything in this file is pure: a state is read from a fixture, a rank comes
//! out, and nothing consults a store, a clock or a model.

use std::collections::BTreeMap;

use mm_core::Config;
use mm_core::Timestamp;
use mm_library::applicability::{
    activation_score, rank, selectable, state_from_json, ApplicabilityState, CONTRAINDICATION_COST,
    NEUTRAL_FITNESS, W_FREQUENCY, W_RECENCY, W_UTILITY,
};
use mm_library::entry::Declarative;
use mm_library::rdf::declaratives_from_turtle;

fn repo(relative: &str) -> std::path::PathBuf {
    Config::repo_root().join(relative)
}

/// The committed state fixture.
fn state() -> ApplicabilityState {
    let raw = std::fs::read_to_string(repo("bench/library/state_uncertain_strategy.json"))
        .expect("the committed state fixture");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("the state fixture is JSON");
    state_from_json(&value).expect("the state fixture describes a state")
}

/// The seed's selectable entries, decoded from the committed corpus.
fn seed_entries() -> Vec<Declarative> {
    let seed =
        std::fs::read_to_string(repo("ontology/seed/library_seed.ttl")).expect("the seed corpus");
    declaratives_from_turtle(&seed).expect("the seed decodes")
}

/// One row of the gold table.
struct GoldRow {
    iri: String,
    total: f32,
    activation: f32,
    fitness: f32,
    contraindication_penalty: f32,
}

/// The gold rows, in file order.
fn gold() -> Vec<GoldRow> {
    let raw = std::fs::read_to_string(repo("bench/library/gold_applicability.jsonl"))
        .expect("the committed gold table");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).expect("a gold row is JSON");
            let number = |key: &str| {
                value
                    .get(key)
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| panic!("gold row is missing {key}: {line}"))
                    as f32
            };
            GoldRow {
                iri: value
                    .get("iri")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("gold row is missing iri: {line}"))
                    .to_string(),
                total: number("total"),
                activation: number("activation"),
                fitness: number("fitness"),
                contraindication_penalty: number("contraindication_penalty"),
            }
        })
        .collect()
}

/// Close enough for a float that travelled through JSON.
fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn the_rank_matches_the_committed_gold_table() {
    let entries = seed_entries();
    let candidates: Vec<Declarative> = selectable(&entries).into_iter().cloned().collect();
    assert!(
        candidates.len() >= 5,
        "the seed must hold at least the five ranked entries, got {}",
        candidates.len()
    );

    let rows = gold();
    let ranked = rank(&candidates, &state(), rows.len());
    assert_eq!(
        ranked.len(),
        rows.len(),
        "the rank must produce exactly as many entries as the table lists"
    );

    for (index, (row, actual)) in rows.iter().zip(ranked.iter()).enumerate() {
        assert_eq!(
            actual.iri, row.iri,
            "rank position {index}: the table expects {} and the rank chose {}",
            row.iri, actual.iri
        );
        assert!(
            close(actual.score.total, row.total),
            "{}: total {} != gold {}",
            row.iri,
            actual.score.total,
            row.total
        );
        assert!(
            close(actual.score.activation, row.activation),
            "{}: activation {} != gold {}",
            row.iri,
            actual.score.activation,
            row.activation
        );
        assert!(
            close(actual.score.fitness, row.fitness),
            "{}: fitness {} != gold {}",
            row.iri,
            actual.score.fitness,
            row.fitness
        );
        assert!(
            close(
                actual.score.contraindication_penalty,
                row.contraindication_penalty
            ),
            "{}: contraindication penalty {} != gold {}",
            row.iri,
            actual.score.contraindication_penalty,
            row.contraindication_penalty
        );
    }
}

#[test]
fn two_ranks_of_the_same_state_are_identical() {
    let entries = seed_entries();
    let candidates: Vec<Declarative> = selectable(&entries).into_iter().cloned().collect();
    let state = state();
    let first = rank(&candidates, &state, 5);
    let second = rank(&candidates, &state, 5);
    assert_eq!(first, second, "the ranker is a pure function of its inputs");

    // The state's own hash is likewise a function of the ranked inputs, not of the
    // instant it is evaluated at.
    let later = ApplicabilityState {
        now: Timestamp::from_epoch_seconds(4_000_000_000),
        ..state.clone()
    };
    assert_eq!(
        state.hash(),
        later.hash(),
        "the state hash must not fold in the clock"
    );
}

#[test]
fn ranking_is_input_order_independent_because_ties_break_on_the_iri() {
    let entries = seed_entries();
    let candidates: Vec<Declarative> = selectable(&entries).into_iter().cloned().collect();
    let mut reversed = candidates.clone();
    reversed.reverse();
    let state = state();
    assert_eq!(
        rank(&candidates, &state, 5),
        rank(&reversed, &state, 5),
        "the same entries in the opposite input order must rank the same way"
    );
}

#[test]
fn the_activation_blend_is_clamped_and_weighted_as_documented() {
    // The weights are the formula's public constants, so they are asserted as
    // constants rather than recomputed from a magic number.
    assert!(
        close(W_RECENCY + W_FREQUENCY + W_UTILITY, 1.0),
        "the blend's weights must sum to 1"
    );
    assert!(close(activation_score(1.0, 1.0, 1.0), 1.0));
    assert!(close(activation_score(0.0, 0.0, 0.0), 0.0));
    assert!(
        close(activation_score(4.0, 4.0, 4.0), 1.0),
        "an out-of-range input cannot push the blend above 1"
    );
    assert!(
        close(activation_score(-4.0, -4.0, -4.0), 0.0),
        "nor below 0"
    );
    assert!(close(activation_score(1.0, 0.0, 0.0), W_RECENCY));
}

#[test]
fn an_unknown_domain_is_neutral_and_a_stated_contraindication_costs() {
    let state = state();
    let entries = seed_entries();
    let technique = entries
        .iter()
        .find(|entry| entry.kind == mm_library::EntryKind::Technique)
        .expect("the seed has a technique");

    // Everything is unknown to a state with no domains, so the fitness component is
    // the neutral prior rather than zero.
    let empty = ApplicabilityState::new("nothing in particular", Timestamp::EPOCH);
    assert!(close(
        mm_library::applicability::fitness(technique, &empty),
        NEUTRAL_FITNESS
    ));

    // A matched contraindication costs the documented fraction of its list.
    let mut flagged = technique.clone();
    flagged.contraindications = vec![mm_library::when("high stakes").expect("a literal condition")];
    assert!(
        close(
            mm_library::applicability::contraindication_penalty(&flagged, &state),
            CONTRAINDICATION_COST,
        ),
        "one matched contraindication of one costs the full constant"
    );

    // And the ranking's stated-but-unmet condition costs half of it, which is why
    // the seed's unmatched entries sit below the matched ones.
    let top = rank(
        &selectable(&entries)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        &state,
        3,
    );
    assert!(
        top[0].score.total >= top[2].score.total,
        "the rank is ordered best first"
    );
}

#[test]
fn the_gold_table_is_not_vacuous() {
    // A table of one row, or of identical scores, would pass a comparison test while
    // proving nothing about ordering.
    let rows = gold();
    assert!(rows.len() >= 5, "the gate ranks five entries");
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        *seen.entry(format!("{:.6}", row.total)).or_insert(0) += 1;
    }
    assert!(
        seen.len() >= 3,
        "the gold scores must differ, otherwise ordering is untested: {seen:?}"
    );
}
