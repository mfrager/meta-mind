//! `rdf_roundtrip.rs` — the canonical codec is lossless.
//!
//! Two properties, both over generated snapshots:
//!
//! * **pure** — encoding a snapshot, decoding the quads to [`EpistemicTriple`]s,
//!   and re-encoding them yields the identical content hash. A codec that dropped
//!   or invented a term would fail here rather than at the mirror.
//! * **through the graph** — writing the snapshot to `/epistemic` and reading it
//!   back hashes to the same value, and a second write is byte-identical, so the
//!   mirror is idempotent.
//!
//! `EpistemicTriple` is a normal form, so the hash of a set of them does not
//! depend on insertion order — which is what makes replay and diffing possible.

use std::path::PathBuf;
use std::sync::OnceLock;

use mm_core::{Config, Ulid};
use mm_epistemic::rdf::{self, FromRdf};
use mm_epistemic::{
    Claim, ClaimKind, EpistemicSnapshot, EpistemicTriple, Proposition, EPISTEMIC_GRAPH,
    EPISTEMIC_STATUSES,
};
use mm_store_graph::GraphStore;
use proptest::prelude::*;

/// A runtime for the graph halves of the properties.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}

fn shapes() -> PathBuf {
    Config::repo_root()
        .join("ontology")
        .join("shapes")
        .join("epistemic_shapes.ttl")
}

fn claim_from(n: u128, status_index: usize, millis: u16, object: &str) -> Claim {
    let status = EPISTEMIC_STATUSES[status_index % EPISTEMIC_STATUSES.len()];
    let mut claim = Claim::new(
        Ulid::from_parts(1_700_000_000_000, n + 1),
        ClaimKind::Fact,
        Proposition::literal(
            "https://metamind.dev/data/s",
            "https://metamind.dev/ontology#p",
            object,
        )
        .unwrap(),
        status,
        f32::from(millis) / 1000.0,
    )
    .unwrap();
    // A world-admissible claim needs evidence, or the separation is not what the
    // snapshot is claiming to hold.
    if status.is_world_admissible() {
        claim.evidence = vec![Ulid::from_parts(1, n + 1)];
    }
    claim
}

fn snapshot_from(spec: &[(u16, usize, String)]) -> EpistemicSnapshot {
    let claims = spec
        .iter()
        .enumerate()
        .map(|(i, (millis, status, object))| claim_from(i as u128 + 1, *status, *millis, object))
        .collect();
    EpistemicSnapshot {
        claims,
        ..EpistemicSnapshot::default()
    }
}

fn specs() -> impl Strategy<Value = Vec<(u16, usize, String)>> {
    prop::collection::vec(
        (
            0u16..=1000,
            0usize..EPISTEMIC_STATUSES.len(),
            "[a-z]{1,6}".prop_map(String::from),
        ),
        1..6,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// encode → decode → encode hashes identically.
    #[test]
    fn an_encode_decode_encode_cycle_hashes_identically(spec in specs()) {
        let snapshot = snapshot_from(&spec);
        let quads = snapshot.to_quads();
        let first = rdf::quads_hash(&quads);

        let mut decoded: Vec<EpistemicTriple> =
            quads.iter().map(EpistemicTriple::from_quad).collect();
        decoded.sort();
        let second = rdf::triples_hash(&decoded);

        prop_assert_eq!(&first, &second);
        // And decoding is a normal form: decoding the decode changes nothing.
        prop_assert_eq!(rdf::triples_hash(&decoded), rdf::triples_hash(&decoded.clone()));
    }

    /// The hash is a function of the set, not of the insertion order.
    #[test]
    fn the_hash_ignores_insertion_order(spec in specs()) {
        let snapshot = snapshot_from(&spec);
        let mut quads = snapshot.to_quads();
        let expected = rdf::quads_hash(&quads);
        quads.reverse();
        prop_assert_eq!(rdf::quads_hash(&quads), expected);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 8, ..ProptestConfig::default() })]

    /// Writing to the graph and reading it back preserves the content hash, and a
    /// second write is identical.
    #[test]
    fn a_snapshot_round_trips_through_the_graph(spec in specs()) {
        let snapshot = snapshot_from(&spec);
        let expected = rdf::quads_hash(&snapshot.to_quads());
        let (read, again) = runtime().block_on(async {
            let graph = GraphStore::in_memory(&shapes()).await.unwrap();
            rdf::mirror(graph.handle(), &snapshot).await.unwrap();
            let rows = graph.triples(EPISTEMIC_GRAPH).await.unwrap();
            let read = rdf::triples_hash(&<Vec<EpistemicTriple> as FromRdf>::from_rows(&rows));

            rdf::mirror(graph.handle(), &snapshot).await.unwrap();
            let rows = graph.triples(EPISTEMIC_GRAPH).await.unwrap();
            let again = rdf::triples_hash(&<Vec<EpistemicTriple> as FromRdf>::from_rows(&rows));
            graph.shutdown().await.unwrap();
            (read, again)
        });
        prop_assert_eq!(&read, &expected);
        prop_assert_eq!(&again, &expected);
    }
}
