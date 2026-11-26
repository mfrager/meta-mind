//! The justification graph: what each derived belief rests on (JTMS/ATMS).
//!
//! The plan's requirement is that a retraction is *dependency-directed*: when a
//! support is withdrawn, everything that rested on it goes with it, at once and
//! without a search. That is a graph property, and it is bought here:
//!
//! * An edge is `consequent <- antecedents`: "this is believed *because of*
//!   these". It is an in-list (ATMS terminology); the out-list is derived, never
//!   stored, so the two can never disagree.
//! * [`JustificationGraph::add`] refuses a cycle. A belief that justifies itself
//!   is not a weak belief, it is an error, and the graph would make every
//!   retraction meaningless.
//! * [`JustificationGraph::transitive_dependents`] is the retraction set, and
//!   [`JustificationGraph::cascade`] returns it in a deterministic order — ULID —
//!   so the same retraction produces the same list on every replay.
//!
//! `criticality` is a caller-supplied `[0,1]` weight. It is recorded and never
//! used to *decide*: the cascade is total, because a support set is withdrawn as
//! a set.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mm_core::Ulid;
use serde::{Deserialize, Serialize};

use crate::error::{EpistemicError, Result};

/// What kind of support an edge records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepKind {
    /// The consequent is an assumption, and the edge records that.
    Assumption,
    /// The consequent was derived from the antecedents.
    Derivation,
    /// The consequent was observed, and the antecedent is the record.
    Observation,
    /// The consequent was predicted from the antecedents.
    Prediction,
    /// The consequent was chosen, and the antecedents are what it weighed.
    Decision,
}

/// Every dependency kind, in a stable order.
pub const DEP_KINDS: [DepKind; 5] = [
    DepKind::Assumption,
    DepKind::Derivation,
    DepKind::Observation,
    DepKind::Prediction,
    DepKind::Decision,
];

impl DepKind {
    /// The stable wire name, matching `dependencies.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            DepKind::Assumption => "assumption",
            DepKind::Derivation => "derivation",
            DepKind::Observation => "observation",
            DepKind::Prediction => "prediction",
            DepKind::Decision => "decision",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        DEP_KINDS.into_iter().find(|kind| kind.as_str() == text)
    }

    /// True when the edge may have no antecedents: a root support.
    ///
    /// Only an assumption is a root. A derivation with no premises would be a
    /// conclusion that nothing supports.
    pub fn may_be_root(self) -> bool {
        matches!(self, DepKind::Assumption)
    }
}

impl std::fmt::Display for DepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One support edge: the consequent is believed because of the antecedents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Justification {
    /// The supported node.
    pub consequent: Ulid,
    /// The nodes it rests on. Empty only for a root support.
    pub antecedents: Vec<Ulid>,
    /// How the support was produced.
    pub kind: DepKind,
    /// How much the consequent leans on this support, in `[0,1]`.
    pub criticality: f32,
}

impl Justification {
    /// Build an edge, validating it without a graph.
    pub fn new(
        consequent: Ulid,
        antecedents: Vec<Ulid>,
        kind: DepKind,
        criticality: f32,
    ) -> Result<Self> {
        if consequent.is_nil() {
            return Err(EpistemicError::validation(
                "consequent",
                "must not be the nil ULID",
            ));
        }
        if antecedents.is_empty() && !kind.may_be_root() {
            return Err(EpistemicError::validation(
                "antecedents",
                format!("a {kind} edge rests on at least one antecedent"),
            ));
        }
        if antecedents.contains(&consequent) {
            return Err(EpistemicError::Cycle(crate::error::id_text(&consequent)));
        }
        let unique: BTreeSet<Ulid> = antecedents.iter().copied().collect();
        if unique.len() != antecedents.len() {
            return Err(EpistemicError::validation(
                "antecedents",
                "an antecedent is named twice in one justification",
            ));
        }
        if !(0.0..=1.0).contains(&criticality) {
            return Err(EpistemicError::validation(
                "criticality",
                format!("must be in [0,1], got {criticality}"),
            ));
        }
        Ok(Justification {
            consequent,
            antecedents,
            kind,
            criticality,
        })
    }
}

/// The retraction set for one invalidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cascade {
    /// The node whose support was withdrawn.
    pub root: Ulid,
    /// Every node that depended on it, transitively, in ULID order. The root is
    /// not included: it is what was withdrawn, not what was affected by it.
    pub affected: Vec<Ulid>,
}

impl Cascade {
    /// How many dependents the withdrawal reached.
    pub fn count(&self) -> usize {
        self.affected.len()
    }

    /// The root followed by every affected node: the whole retraction, in order.
    pub fn all(&self) -> Vec<Ulid> {
        let mut out = Vec::with_capacity(self.affected.len() + 1);
        out.push(self.root);
        out.extend(self.affected.iter().copied());
        out
    }

    /// True when pulling the root affected nothing else.
    pub fn is_contained(&self) -> bool {
        self.affected.is_empty()
    }
}

/// The support graph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JustificationGraph {
    edges: Vec<Justification>,
    /// consequent -> the indices of its in-list edges.
    in_lists: BTreeMap<Ulid, Vec<usize>>,
    /// antecedent -> the indices of the edges it supports.
    out_lists: BTreeMap<Ulid, Vec<usize>>,
}

impl JustificationGraph {
    /// An empty graph.
    pub fn new() -> Self {
        JustificationGraph::default()
    }

    /// Build a graph from edges, refusing a set that is not a DAG.
    pub fn from_edges(edges: Vec<Justification>) -> Result<Self> {
        let mut graph = JustificationGraph::new();
        for edge in edges {
            graph.add(edge)?;
        }
        Ok(graph)
    }

    /// Add an edge, refusing a self-reference, a duplicate pair, or a cycle.
    ///
    /// The graph is left untouched on refusal: a caller that catches the error
    /// has not corrupted the support structure.
    pub fn add(&mut self, justification: Justification) -> Result<()> {
        // Re-validate, so a hand-built value cannot slip past the constructor.
        let checked = Justification::new(
            justification.consequent,
            justification.antecedents.clone(),
            justification.kind,
            justification.criticality,
        )?;
        for antecedent in &checked.antecedents {
            // The DDL is UNIQUE(consequent, antecedent), so a repeated pair is a
            // duplicate edge rather than a second reason.
            if self.has_edge(&checked.consequent, antecedent) {
                return Err(EpistemicError::validation(
                    "antecedent",
                    format!(
                        "the edge {} -> {} already exists",
                        crate::error::id_text(&checked.consequent),
                        crate::error::id_text(antecedent)
                    ),
                ));
            }
            // The new edge makes the consequent rest on the antecedent, so it is
            // a cycle exactly when the antecedent already rests on the consequent.
            if self.depends_on(antecedent, &checked.consequent) {
                return Err(EpistemicError::Cycle(format!(
                    "{} -> {} closes a cycle",
                    crate::error::id_text(&checked.consequent),
                    crate::error::id_text(antecedent)
                )));
            }
        }
        let index = self.edges.len();
        self.in_lists
            .entry(checked.consequent)
            .or_default()
            .push(index);
        for antecedent in &checked.antecedents {
            self.out_lists.entry(*antecedent).or_default().push(index);
        }
        self.edges.push(checked);
        Ok(())
    }

    /// Every edge, in insertion order.
    pub fn edges(&self) -> &[Justification] {
        &self.edges
    }

    /// How many edges the graph holds.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// True when nothing is justified.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// True when this exact pair is already an edge.
    pub fn has_edge(&self, consequent: &Ulid, antecedent: &Ulid) -> bool {
        self.out_lists
            .get(antecedent)
            .map(|indices| {
                indices
                    .iter()
                    .any(|index| self.edges[*index].consequent == *consequent)
            })
            .unwrap_or(false)
    }

    /// The in-list of a node: the antecedents its support names.
    pub fn in_list(&self, node: &Ulid) -> Vec<Ulid> {
        let mut out: Vec<Ulid> = self
            .in_lists
            .get(node)
            .into_iter()
            .flatten()
            .flat_map(|index| self.edges[*index].antecedents.iter().copied())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The out-list of a node: the nodes whose support names it.
    pub fn out_list(&self, node: &Ulid) -> Vec<Ulid> {
        let mut out: Vec<Ulid> = self
            .out_lists
            .get(node)
            .into_iter()
            .flatten()
            .map(|index| self.edges[*index].consequent)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Why a node is believed: the edges that support it.
    pub fn why(&self, node: &Ulid) -> Vec<Justification> {
        self.in_lists
            .get(node)
            .into_iter()
            .flatten()
            .map(|index| self.edges[*index].clone())
            .collect()
    }

    /// True when `from` depends on `to`, transitively.
    fn depends_on(&self, from: &Ulid, to: &Ulid) -> bool {
        if from == to {
            return true;
        }
        let mut seen: BTreeSet<Ulid> = BTreeSet::new();
        let mut queue: VecDeque<Ulid> = VecDeque::new();
        queue.push_back(*from);
        while let Some(node) = queue.pop_front() {
            if !seen.insert(node) {
                continue;
            }
            for next in self.in_list(&node) {
                if next == *to {
                    return true;
                }
                queue.push_back(next);
            }
        }
        false
    }

    /// Every node that depends on `root`, transitively, in ULID order.
    ///
    /// This *is* the dependency-directed retraction set: withdrawing `root`
    /// withdraws exactly these.
    pub fn transitive_dependents(&self, root: &Ulid) -> Vec<Ulid> {
        let mut affected: BTreeSet<Ulid> = BTreeSet::new();
        let mut queue: VecDeque<Ulid> = VecDeque::new();
        queue.push_back(*root);
        while let Some(node) = queue.pop_front() {
            for dependent in self.out_list(&node) {
                if affected.insert(dependent) {
                    queue.push_back(dependent);
                }
            }
        }
        affected.remove(root);
        affected.into_iter().collect()
    }

    /// The full retraction a withdrawal of `root` implies.
    pub fn cascade(&self, root: &Ulid) -> Cascade {
        Cascade {
            root: *root,
            affected: self.transitive_dependents(root),
        }
    }

    /// True when the graph is acyclic.
    ///
    /// `add` refuses cycles, so this is a property test rather than an operational
    /// check: it proves the invariant holds for an arbitrary accepted edge set.
    pub fn is_acyclic(&self) -> bool {
        // Kahn's algorithm over "depends on" edges: a node with no in-list is a
        // root, and every node must be removed.
        let mut remaining: BTreeMap<Ulid, usize> = BTreeMap::new();
        for edge in &self.edges {
            remaining.entry(edge.consequent).or_insert(0);
            for antecedent in &edge.antecedents {
                remaining.entry(*antecedent).or_insert(0);
            }
        }
        for edge in &self.edges {
            *remaining.entry(edge.consequent).or_default() += edge.antecedents.len();
        }
        let mut roots: VecDeque<Ulid> = remaining
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(node, _)| *node)
            .collect();
        let mut removed = 0usize;
        let total = remaining.len();
        while let Some(node) = roots.pop_front() {
            removed += 1;
            for dependent in self.out_list(&node) {
                if let Some(degree) = remaining.get_mut(&dependent) {
                    // One edge contributes at most one count per antecedent; a
                    // dependent with several supports loses all of them.
                    let shared = self
                        .edges
                        .iter()
                        .filter(|edge| {
                            edge.consequent == dependent && edge.antecedents.contains(&node)
                        })
                        .count();
                    *degree = degree.saturating_sub(shared.max(1));
                    if *degree == 0 {
                        roots.push_back(dependent);
                    }
                }
            }
        }
        removed == total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    fn root(n: u128) -> Justification {
        Justification::new(id(n), Vec::new(), DepKind::Assumption, 1.0).unwrap()
    }

    fn derived(n: u128, from: &[u128]) -> Justification {
        Justification::new(
            id(n),
            from.iter().copied().map(id).collect(),
            DepKind::Derivation,
            0.5,
        )
        .unwrap()
    }

    #[test]
    fn only_an_assumption_may_be_a_root_and_a_self_reference_is_refused() {
        assert!(Justification::new(id(1), Vec::new(), DepKind::Assumption, 1.0).is_ok());
        assert!(Justification::new(id(1), Vec::new(), DepKind::Derivation, 1.0).is_err());
        assert!(Justification::new(id(1), vec![id(1)], DepKind::Derivation, 1.0).is_err());
        assert!(Justification::new(id(1), vec![id(2), id(2)], DepKind::Derivation, 1.0).is_err());
        assert!(Justification::new(id(1), vec![id(2)], DepKind::Derivation, 1.5).is_err());
    }

    #[test]
    fn a_cycle_is_refused_and_the_graph_is_left_alone() {
        let mut graph = JustificationGraph::new();
        graph.add(root(1)).unwrap();
        graph.add(derived(2, &[1])).unwrap();
        graph.add(derived(3, &[2])).unwrap();
        assert_eq!(graph.len(), 3);
        // 1 rests on 3 closes 1 <- 2 <- 3 <- 1.
        let error = graph
            .add(Justification::new(id(1), vec![id(3)], DepKind::Derivation, 0.5).unwrap())
            .unwrap_err();
        assert_eq!(error.code(), "epistemic.cycle");
        assert_eq!(graph.len(), 3, "a refused edge must not be recorded");
        assert!(!graph.has_edge(&id(1), &id(3)));
        assert!(graph.is_acyclic());
    }

    #[test]
    fn a_duplicate_pair_is_refused() {
        let mut graph = JustificationGraph::new();
        graph.add(derived(2, &[1])).unwrap();
        let error = graph.add(derived(2, &[1])).unwrap_err();
        assert_eq!(error.code(), "epistemic.validation");
        assert_eq!(graph.len(), 1);
    }

    #[test]
    fn the_cascade_reaches_every_transitive_dependent_in_ulid_order() {
        let mut graph = JustificationGraph::new();
        graph.add(root(1)).unwrap();
        graph.add(derived(2, &[1])).unwrap();
        graph.add(derived(3, &[2])).unwrap();
        // A second branch that also rests on 1, and an unrelated node.
        graph.add(derived(5, &[1])).unwrap();
        graph.add(root(9)).unwrap();
        graph.add(derived(10, &[9])).unwrap();

        assert_eq!(
            graph.transitive_dependents(&id(1)),
            vec![id(2), id(3), id(5)]
        );
        let cascade = graph.cascade(&id(1));
        assert_eq!(cascade.count(), 3);
        assert_eq!(cascade.all(), vec![id(1), id(2), id(3), id(5)]);
        assert!(!cascade.is_contained());
        // A leaf withdrawal reaches nothing.
        assert!(graph.cascade(&id(3)).is_contained());
        // And the unrelated branch is untouched.
        assert_eq!(graph.transitive_dependents(&id(9)), vec![id(10)]);
    }

    #[test]
    fn in_and_out_lists_are_derived_and_agree() {
        let mut graph = JustificationGraph::new();
        graph.add(derived(2, &[1, 4])).unwrap();
        assert_eq!(graph.in_list(&id(2)), vec![id(1), id(4)]);
        assert_eq!(graph.out_list(&id(1)), vec![id(2)]);
        assert_eq!(graph.out_list(&id(4)), vec![id(2)]);
        assert!(graph.in_list(&id(1)).is_empty());
        assert!(graph.out_list(&id(2)).is_empty());
        let why = graph.why(&id(2));
        assert_eq!(why.len(), 1);
        assert_eq!(why[0].antecedents, vec![id(1), id(4)]);
        assert_eq!(why[0].kind, DepKind::Derivation);
    }

    #[test]
    fn a_diamond_is_acyclic_and_the_cascade_visits_each_node_once() {
        let mut graph = JustificationGraph::new();
        graph.add(root(1)).unwrap();
        graph.add(derived(2, &[1])).unwrap();
        graph.add(derived(3, &[1])).unwrap();
        graph.add(derived(4, &[2, 3])).unwrap();
        assert!(graph.is_acyclic());
        assert_eq!(
            graph.transitive_dependents(&id(1)),
            vec![id(2), id(3), id(4)]
        );
    }
}
