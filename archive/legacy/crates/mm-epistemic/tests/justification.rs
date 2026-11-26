//! `justification.rs` — an accepted justification graph is acyclic, and a cascade
//! names exactly the transitive dependents.

mod common;

use mm_core::Ulid;
use mm_epistemic::{DepKind, Justification, JustificationGraph};
use proptest::prelude::*;

fn id(n: u128) -> Ulid {
    Ulid::from_parts(1_700_000_000_000, n)
}

fn edge(consequent: u128, antecedents: &[u128]) -> Option<Justification> {
    Justification::new(
        id(consequent),
        antecedents.iter().copied().map(id).collect(),
        DepKind::Derivation,
        0.5,
    )
    .ok()
}

proptest! {
    /// Whatever set of edges is accepted, the result is acyclic, and every
    /// dependent returned by a cascade really is reachable from the root.
    #[test]
    fn an_accepted_graph_is_acyclic_and_its_cascades_are_sound(
        edges in prop::collection::vec((0u128..10, prop::collection::vec(0u128..10, 1..3)), 0..40)
    ) {
        let mut graph = JustificationGraph::new();
        for (consequent, antecedents) in edges {
            if let Some(justification) = edge(consequent, &antecedents) {
                // A refusal must leave nothing behind; a success is fine.
                let _ = graph.add(justification);
            }
        }
        prop_assert!(graph.is_acyclic());

        for root in 0u128..10 {
            let cascade = graph.cascade(&id(root));
            // The root is never its own dependent.
            prop_assert!(!cascade.affected.contains(&id(root)));
            // Every affected node has the root somewhere in its in-list closure.
            for node in &cascade.affected {
                prop_assert!(reaches(&graph, *node, root));
            }
        }
    }
}

/// True when `node` transitively rests on `target`.
fn reaches(graph: &JustificationGraph, node: Ulid, target: u128) -> bool {
    let mut stack = vec![node];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current) {
            continue;
        }
        for antecedent in graph.in_list(&current) {
            if antecedent == id(target) {
                return true;
            }
            stack.push(antecedent);
        }
    }
    false
}

#[test]
fn a_cascade_is_the_transitive_dependents_in_ulid_order() {
    let mut graph = JustificationGraph::new();
    graph
        .add(edge(1, &[]).unwrap_or_else(|| {
            Justification::new(id(1), Vec::new(), DepKind::Assumption, 1.0).unwrap()
        }))
        .unwrap();
    graph.add(edge(2, &[1]).unwrap()).unwrap();
    graph.add(edge(3, &[2]).unwrap()).unwrap();
    graph.add(edge(4, &[1]).unwrap()).unwrap();
    assert_eq!(
        graph.transitive_dependents(&id(1)),
        vec![id(2), id(3), id(4)]
    );
    assert_eq!(graph.cascade(&id(2)).affected, vec![id(3)]);
}

#[test]
fn a_cycle_is_refused_and_a_duplicate_pair_is_refused() {
    let mut graph = JustificationGraph::new();
    graph
        .add(edge(1, &[]).unwrap_or_else(|| {
            Justification::new(id(1), Vec::new(), DepKind::Assumption, 1.0).unwrap()
        }))
        .unwrap();
    graph.add(edge(2, &[1]).unwrap()).unwrap();
    // 1 <- 2 closes 1 <- 2 <- 1 (2 already rests on 1).
    let cyclic = Justification::new(id(1), vec![id(2)], DepKind::Derivation, 0.5).unwrap();
    assert_eq!(graph.add(cyclic).unwrap_err().code(), "epistemic.cycle");
    assert_eq!(graph.len(), 2);
    // The exact pair twice is a duplicate.
    assert_eq!(
        graph.add(edge(2, &[1]).unwrap()).unwrap_err().code(),
        "epistemic.validation"
    );
    assert!(graph.is_acyclic());
}
