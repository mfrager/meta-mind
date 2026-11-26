//! The identity and the four core invariants (plan §4.1, §8).
//!
//! Two claims are load-bearing here. An identity is created once and only once,
//! so state has an owner across restarts. And each of the four invariants is
//! reachable through the public API — measured by running the adversarial corpus
//! through the facade, not by asserting that it exists.

mod common;

use common::{corpus_path, jsonl, Harness};
use mm_being::{BeingError, BeingOp, BlockKind, InvariantCode, CORE_INVARIANTS};
use mm_log::codes;

/// Opening a facade creates exactly one identity, binds all four invariants, and
/// writes the `being.identity.init` audit record.
#[tokio::test]
async fn opening_a_being_creates_one_identity_with_four_invariants() {
    let h = Harness::new().await;
    let facade = h.facade().await;

    assert_eq!(facade.identity().invariant_codes().len(), 4);
    assert_eq!(facade.identity().current_version, "v1");
    assert_eq!(h.scalar("SELECT count(*) FROM identity").await, 1);
    assert_eq!(h.scalar("SELECT count(*) FROM invariants").await, 4);

    let codes: Vec<String> = h
        .query("SELECT code FROM invariants ORDER BY code")
        .await
        .into_iter()
        .filter_map(|row| row["code"].as_str().map(str::to_string))
        .collect();
    let mut expected: Vec<String> = CORE_INVARIANTS
        .iter()
        .map(|c| c.as_str().to_string())
        .collect();
    expected.sort();
    assert_eq!(codes, expected);

    assert!(
        h.audit_codes()
            .await
            .contains(&codes::BEING_IDENTITY_INIT.to_string()),
        "identity creation must be audited"
    );
    h.shutdown().await;
}

/// A second facade over the same store reuses the identity rather than creating a
/// second one: continuity across restarts is the whole point of the substrate.
#[tokio::test]
async fn reopening_the_being_reuses_the_identity() {
    let h = Harness::new().await;
    let first = h.facade().await;
    let id = mm_core::ulid_string(&first.identity().id);
    let init_records = h.records_of(codes::BEING_IDENTITY_INIT).len();
    drop(first);

    let second = h.facade().await;
    assert_eq!(mm_core::ulid_string(&second.identity().id), id);
    assert_eq!(h.scalar("SELECT count(*) FROM identity").await, 1);
    assert_eq!(
        h.records_of(codes::BEING_IDENTITY_INIT).len(),
        init_records,
        "reopening an existing being emits no second init record"
    );
    h.shutdown().await;
}

/// The guard refuses a second identity as a rewrite of the first.
#[tokio::test]
async fn a_second_identity_is_refused_as_a_history_rewrite() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;
    let err = facade
        .apply(BeingOp::InitIdentity {
            self_description: "a different self".into(),
        })
        .await
        .unwrap_err();
    match err {
        BeingError::Invariant(v) => assert_eq!(v.code, InvariantCode::NoHistoryRewrite),
        other => panic!("expected an invariant violation, got {other:?}"),
    }
    assert_eq!(h.scalar("SELECT count(*) FROM identity").await, 1);
    h.shutdown().await;
}

/// Every line of the adversarial corpus is denied, for exactly the invariant it
/// names, without writing anything.
#[tokio::test]
async fn the_adversarial_corpus_is_denied_for_the_declared_invariant() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    let entries = jsonl(&corpus_path());
    assert!(
        !entries.is_empty(),
        "the corpus at {} must not be empty",
        corpus_path().display()
    );

    for entry in &entries {
        let name = entry["name"].as_str().unwrap();
        let op_name = entry["op"].as_str().unwrap();
        let invariant = entry["invariant"].as_str().unwrap();
        let params = entry
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let op = BeingOp::from_corpus(op_name, &params)
            .unwrap_or_else(|e| panic!("{name}: cannot build `{op_name}`: {e}"));
        let err = match facade.apply(op).await {
            Ok(_) => panic!("{name}: forbidden op was accepted"),
            Err(e) => e,
        };
        match err {
            BeingError::Invariant(v) => assert_eq!(
                v.code.as_str(),
                invariant,
                "{name}: refused as `{}`, corpus declares `{invariant}`",
                v.code.as_str()
            ),
            other => panic!("{name}: refused with the wrong error: {other:?}"),
        }
    }

    let violations = h.records_of(codes::BEING_INVARIANT_VIOLATION);
    assert_eq!(
        violations.len(),
        entries.len(),
        "every refusal must emit one violation record"
    );

    // The report the gate reads agrees with this test.
    let report = facade.verify(&corpus_path()).await.unwrap();
    assert!(report.ok(), "verify failures: {:?}", report.failures);
    assert_eq!(report.corpus_denied, report.corpus_total);
    assert!(report.corpus_total >= 20);

    h.shutdown().await;
}

/// A corpus that names an op this build does not know is an error, never a skip:
/// otherwise the corpus could quietly shrink.
#[tokio::test]
async fn an_unknown_corpus_op_is_refused() {
    let err = BeingOp::from_corpus("frobnicate_world", &serde_json::json!({})).unwrap_err();
    assert_eq!(err.kind(), "config");
    assert!(err.to_string().contains("frobnicate_world"));
}

/// A core block over its limit is refused before it is stored.
#[tokio::test]
async fn an_over_limit_core_block_is_refused_and_not_stored() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    let err = facade
        .apply(BeingOp::SetBlock {
            kind: BlockKind::Self_,
            label: "self".into(),
            content: "x".repeat(5_000),
            limit_chars: 200,
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "block_too_large");
    assert_eq!(h.scalar("SELECT count(*) FROM core_blocks").await, 0);

    // Within the limit it is stored and audited.
    facade
        .apply(BeingOp::SetBlock {
            kind: BlockKind::Self_,
            label: "self".into(),
            content: "I am Metamind".into(),
            limit_chars: 200,
        })
        .await
        .unwrap();
    assert_eq!(h.scalar("SELECT count(*) FROM core_blocks").await, 1);
    assert_eq!(h.records_of(codes::BEING_BLOCK_SET).len(), 1);
    h.shutdown().await;
}

/// The identity is mirrored into `/being`, so the graph and the tables agree.
#[tokio::test]
async fn the_identity_is_mirrored_into_the_being_graph() {
    let h = Harness::new().await;
    let facade = h.facade().await;
    let triples = h.graph.triples("being").await.unwrap();
    let id = mm_core::ulid_string(&facade.identity().id);
    let subjects: Vec<&str> = triples.iter().filter_map(|row| row["s"].as_str()).collect();
    assert!(
        subjects.iter().any(|s| s.ends_with(&id)),
        "the identity node must appear in /being"
    );
    h.shutdown().await;
}
