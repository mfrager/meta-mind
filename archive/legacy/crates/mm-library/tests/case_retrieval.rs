//! `case_retrieval.rs` — analogies are ordered by structure, not by wording.
//!
//! The plan's testing table asks that gold precedents come back and that the
//! structure mapping — not surface similarity — orders them. This file builds a
//! small corpus where the two criteria deliberately disagree: a case that shares
//! wording but no relation, and a case that shares relations under entirely
//! different names.

use mm_library::case_db::{
    case_utility, retrieve, structural_similarity, Case, CaseKind, CaseQuery,
};

/// A case whose problem is the given Turtle.
fn case(slug: &str, problem: &str, quality: f32, transferability: f32) -> Case {
    let mut case = Case::new(
        slug,
        CaseKind::Edge,
        problem,
        ".",
        ".",
        quality,
        transferability,
        1.0,
    )
    .expect("a well-formed case");
    case.version = 1;
    case
}

const DEPLOY: &str = r#"
@prefix ex: <https://example.dev/> .
@prefix mm: <https://metamind.dev/ontology#> .
ex:service mm:regressedAfter ex:deploy ;
           mm:exhibits ex:latency .
"#;

/// The same relations over different entities: an analogy a lexical ranker misses.
///
/// The relation IRIs are the shared structure — a case can only map onto the
/// library's own vocabulary — while every element name differs, which is what makes
/// the match structural rather than lexical.
const SAME_SHAPE: &str = r#"
@prefix other: <https://other.dev/> .
@prefix mm: <https://metamind.dev/ontology#> .
other:checkout mm:regressedAfter other:release ;
              mm:exhibits other:slowness .
"#;

/// The same *words*, no shared relation: a coincidence, not an analogy.
const SAME_WORDS: &str = r#"
@prefix ex: <https://example.dev/> .
@prefix mm: <https://metamind.dev/ontology#> .
ex:deploy mm:describedBy "service latency regressedAfter exhibits" .
"#;

#[test]
fn relations_rank_analogies_and_surface_words_do_not() {
    let probe = case("query", DEPLOY, 1.0, 1.0);
    let analogous = case("analogous", SAME_SHAPE, 0.9, 0.9);
    let same_words = case("same-words", SAME_WORDS, 1.0, 1.0);

    let hits = retrieve(
        &[same_words.clone(), analogous.clone()],
        &CaseQuery {
            problem_ttl: DEPLOY.to_string(),
            top: 5,
        },
    )
    .expect("the corpus parses");

    assert_eq!(
        hits.len(),
        1,
        "only the structural match is an analogy: {hits:?}"
    );
    assert_eq!(hits[0].iri, analogous.iri.as_str());
    assert!(
        hits[0].correspondences.len() >= 2,
        "both relations correspond: {:?}",
        hits[0].correspondences
    );
    assert_eq!(hits[0].kind, analogous.kind);

    // And the probe itself is not the point: the surface-only case maps nothing.
    assert!(structural_similarity(&probe, &same_words)
        .expect("both parse")
        .is_empty());
}

#[test]
fn the_correspondence_set_does_not_depend_on_statement_order() {
    let shuffled = r#"
@prefix ex: <https://example.dev/> .
@prefix mm: <https://metamind.dev/ontology#> .
ex:service mm:exhibits ex:latency ;
           mm:regressedAfter ex:deploy .
"#;
    let left = case("left", DEPLOY, 1.0, 1.0);
    let right = case("right", shuffled, 1.0, 1.0);

    let mut first: Vec<String> = structural_similarity(&left, &right)
        .expect("both parse")
        .into_iter()
        .map(|c| c.relation)
        .collect();
    let mut second: Vec<String> = structural_similarity(&left, &right)
        .expect("both parse")
        .into_iter()
        .map(|c| c.relation)
        .collect();
    first.sort();
    second.sort();
    assert_eq!(first, second, "the mapping is a set, not a line order");
    assert_eq!(first.len(), 2);
}

#[test]
fn case_utility_is_multiplicative() {
    let strong = case("strong", DEPLOY, 0.8, 0.8);
    let weak = case("weak", DEPLOY, 0.8, 0.4);

    let similarity = 0.5;
    let strong_utility = case_utility(&strong, similarity);
    let weak_utility = case_utility(&weak, similarity);
    assert!(
        (strong_utility - 2.0 * weak_utility).abs() < 1e-6,
        "halving transferability must halve the utility: {strong_utility} vs {weak_utility}"
    );

    // A case that resembles the problem but did not work is worth nothing, and one
    // that worked but cannot transfer is worth nothing either.
    let failed = case("failed", DEPLOY, 0.0, 1.0);
    assert_eq!(case_utility(&failed, 1.0), 0.0);
    let untransferable = case("untransferable", DEPLOY, 1.0, 0.0);
    assert_eq!(case_utility(&untransferable, 1.0), 0.0);
}

#[test]
fn a_case_with_no_shared_relation_is_not_ranked_at_all() {
    let unrelated = case(
        "unrelated",
        r#"
@prefix ex: <https://example.dev/> .
@prefix mm: <https://metamind.dev/ontology#> .
ex:thing mm:unrelated ex:other .
"#,
        1.0,
        1.0,
    );
    let hits = retrieve(
        &[unrelated],
        &CaseQuery {
            problem_ttl: DEPLOY.to_string(),
            top: 5,
        },
    )
    .expect("the corpus parses");
    assert!(
        hits.is_empty(),
        "an analogy of zero structure is not an analogy: {hits:?}"
    );
}
