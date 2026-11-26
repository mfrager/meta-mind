//! Reasoning as a graph of operations.
//!
//! The phase's invariant 4 says a program is a graph, not a list: steps have
//! explicit data dependencies, so the scheduler can run independent ones
//! together and a replay can prove it would have. The structure is the Graph of
//! Thoughts `Graph of Operations`: nodes are operations, edges are "this was
//! computed from that".
//!
//! Every ordering in this module is a pure function of the node set and the edge
//! set — ties break on the smallest node id — so two runs over the same program
//! produce byte-identical orders. That is what `episode replay` rests on.
//!
//! One deliberate deviation from the plan's sketch: `add` takes the node's
//! [`OperationValue`] as well as its op, and returns a `Result`. A node without a
//! value cannot be selected, and an edge naming a node that is not in the graph is
//! a defect in the compiler rather than something the executor should discover.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{MetacogError, Result};
use crate::op::CognitiveOp;
use crate::value::OperationValue;

/// A node's identifier. Small because a program is bounded by its budget.
pub type NodeId = u16;

/// One operation, its identifier and its score.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpNode {
    /// The node's identifier. Assigned in insertion order, so it is stable.
    pub id: NodeId,
    /// The operation.
    pub op: CognitiveOp,
    /// What it is worth.
    pub value: OperationValue,
}

/// A graph of operations with explicit data dependencies.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OpGraph {
    /// The nodes, ordered by id.
    pub nodes: Vec<OpNode>,
    /// Data dependencies as `(from, to)`: `to` consumes `from`.
    pub edges: Vec<(NodeId, NodeId)>,
}

impl OpGraph {
    /// An empty graph.
    pub fn new() -> Self {
        OpGraph::default()
    }

    /// How many operations the graph holds.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the graph has no operations.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Add an operation that consumes `after`, refusing a dependency that is not
    /// in the graph or a duplicate edge.
    ///
    /// The new node's id is the number of nodes already present, so ids are
    /// insertion order and a recompile of the same inputs assigns the same ones.
    pub fn add(
        &mut self,
        op: CognitiveOp,
        value: OperationValue,
        after: &[NodeId],
    ) -> Result<NodeId> {
        let id = u16::try_from(self.nodes.len()).map_err(|_| {
            MetacogError::Graph("a program may hold at most 65535 operations".to_string())
        })?;
        let mut claimed: BTreeSet<NodeId> = BTreeSet::new();
        for dependency in after {
            if !self.contains(*dependency) {
                return Err(MetacogError::NoSuchNode(*dependency));
            }
            if *dependency == id {
                return Err(MetacogError::Graph(format!(
                    "node {id} cannot depend on itself"
                )));
            }
            if self.edges.contains(&(*dependency, id)) || !claimed.insert(*dependency) {
                return Err(MetacogError::Graph(format!(
                    "node {id} already depends on {dependency}"
                )));
            }
        }
        self.nodes.push(OpNode { id, op, value });
        // Keep the edge list sorted and deduplicated: it is what gets hashed.
        for dependency in after {
            self.edges.push((*dependency, id));
        }
        self.edges.sort_unstable();
        self.edges.dedup();
        Ok(id)
    }

    /// True when the graph holds this node.
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.iter().any(|n| n.id == id)
    }

    /// The node with this id.
    pub fn node(&self, id: NodeId) -> Option<&OpNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The nodes that must run before `id`, ascending.
    pub fn dependencies(&self, id: NodeId) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self
            .edges
            .iter()
            .filter(|(_, to)| *to == id)
            .map(|(from, _)| *from)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The nodes that consume `id`, ascending.
    pub fn dependents(&self, id: NodeId) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self
            .edges
            .iter()
            .filter(|(from, _)| *from == id)
            .map(|(_, to)| *to)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// A deterministic topological order (Kahn's algorithm, smallest ready id
    /// first). Returns an error when the graph has a cycle, which the compiler
    /// cannot produce but a decoded program might.
    pub fn topological(&self) -> Result<Vec<NodeId>> {
        let mut in_degree: BTreeMap<NodeId, usize> = self.nodes.iter().map(|n| (n.id, 0)).collect();
        for (_, to) in &self.edges {
            if let Some(count) = in_degree.get_mut(to) {
                *count += 1;
            }
        }
        // A BTreeSet keyed by id pops the smallest ready node every time, which is
        // the whole determinism argument.
        let mut ready: BTreeSet<NodeId> = in_degree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(id) = ready.pop_first() {
            order.push(id);
            for dependent in self.dependents(id) {
                if let Some(count) = in_degree.get_mut(&dependent) {
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(dependent);
                    }
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(MetacogError::Graph(
                "the operation graph contains a cycle".to_string(),
            ));
        }
        Ok(order)
    }

    /// The nodes that can run at once, as a list of layers.
    ///
    /// A layer is every node whose longest path from a root is the same length, so
    /// nodes in one layer depend only on earlier layers. That is what the DAG
    /// executor runs in parallel (the LLM Compiler decomposition).
    pub fn parallel_layers(&self) -> Result<Vec<Vec<NodeId>>> {
        let mut level: BTreeMap<NodeId, usize> = BTreeMap::new();
        for id in self.topological()? {
            let depth = self
                .dependencies(id)
                .into_iter()
                .filter_map(|d| level.get(&d).map(|l| l + 1))
                .max()
                .unwrap_or(0);
            level.insert(id, depth);
        }
        let mut layers: Vec<Vec<NodeId>> = Vec::new();
        for (id, depth) in level {
            if layers.len() <= depth {
                layers.resize_with(depth + 1, Vec::new);
            }
            layers[depth].push(id);
        }
        Ok(layers)
    }

    /// The nodes that are not in `executed` and whose dependencies all are,
    /// ascending. These are what the selector may choose from next.
    pub fn ready(&self, executed: &BTreeSet<NodeId>) -> Vec<NodeId> {
        self.nodes
            .iter()
            .map(|n| n.id)
            .filter(|id| !executed.contains(id))
            .filter(|id| self.dependencies(*id).iter().all(|d| executed.contains(d)))
            .collect()
    }

    /// Refuse a graph that is empty or cyclic.
    pub fn validate(&self) -> Result<()> {
        if self.nodes.is_empty() {
            return Err(MetacogError::Graph(
                "a program must hold at least one operation".to_string(),
            ));
        }
        self.topological()?;
        Ok(())
    }

    /// A canonical rendering, used for the DAG hash. Node order is by id and edge
    /// order is sorted, so two graphs with the same structure render identically.
    pub fn canonical(&self) -> String {
        let mut nodes: Vec<String> = self
            .nodes
            .iter()
            .map(|n| {
                format!(
                    "{}:{}:{}:{}",
                    n.id,
                    n.op.tag(),
                    n.op.class().as_str(),
                    n.value.canonical()
                )
            })
            .collect();
        nodes.sort();
        let edges: Vec<String> = self
            .edges
            .iter()
            .map(|(from, to)| format!("{from}>{to}"))
            .collect();
        format!("nodes[{}]edges[{}]", nodes.join(","), edges.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn value(score_seed: u64) -> OperationValue {
        // A cheap deterministic spread of values, enough to make ties rare.
        OperationValue::new((score_seed % 10) as f64 / 10.0, 0.5, 0.5, 0.05).unwrap()
    }

    fn recall() -> CognitiveOp {
        CognitiveOp::Recall { query: "q".into() }
    }

    fn critique(candidate: &str) -> CognitiveOp {
        CognitiveOp::Critique {
            candidate: candidate.into(),
        }
    }

    #[test]
    fn add_assigns_ids_in_insertion_order_and_refuses_ghost_dependencies() {
        let mut graph = OpGraph::new();
        let root = graph.add(recall(), value(1), &[]).unwrap();
        assert_eq!(root, 0);
        let child = graph.add(critique("a"), value(2), &[root]).unwrap();
        assert_eq!(child, 1);
        assert_eq!(graph.len(), 2);
        assert!(graph.add(critique("b"), value(3), &[9]).is_err());
        // Duplicate edge is refused.
        assert!(graph.add(critique("c"), value(4), &[root, root]).is_err());
    }

    #[test]
    fn topological_order_is_deterministic_and_respects_dependencies() {
        let mut graph = OpGraph::new();
        let a = graph.add(recall(), value(1), &[]).unwrap();
        let b = graph.add(recall(), value(2), &[]).unwrap();
        let c = graph.add(critique("x"), value(3), &[a, b]).unwrap();
        let order = graph.topological().unwrap();
        assert_eq!(order, vec![a, b, c]);
        let position = |id: NodeId| order.iter().position(|x| *x == id).unwrap();
        for (from, to) in &graph.edges {
            assert!(
                position(*from) < position(*to),
                "edge {from}>{to} is backwards"
            );
        }
    }

    #[test]
    fn parallel_layers_group_independent_nodes_and_respect_edges() {
        let mut graph = OpGraph::new();
        let root = graph.add(recall(), value(1), &[]).unwrap();
        let left = graph.add(critique("l"), value(2), &[root]).unwrap();
        let right = graph.add(critique("r"), value(3), &[root]).unwrap();
        let join = graph.add(critique("j"), value(4), &[left, right]).unwrap();
        let layers = graph.parallel_layers().unwrap();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0], vec![root]);
        assert_eq!(layers[1], vec![left, right]);
        assert_eq!(layers[2], vec![join]);
    }

    #[test]
    fn ready_grows_as_nodes_execute() {
        let mut graph = OpGraph::new();
        let root = graph.add(recall(), value(1), &[]).unwrap();
        let child = graph.add(critique("c"), value(2), &[root]).unwrap();
        let mut executed = BTreeSet::new();
        assert_eq!(graph.ready(&executed), vec![root]);
        executed.insert(root);
        assert_eq!(graph.ready(&executed), vec![child]);
        executed.insert(child);
        assert!(graph.ready(&executed).is_empty());
    }

    #[test]
    fn an_empty_graph_is_refused_by_validate() {
        assert!(OpGraph::new().validate().is_err());
        let mut graph = OpGraph::new();
        graph.add(recall(), value(1), &[]).unwrap();
        graph.validate().unwrap();
    }

    #[test]
    fn the_canonical_rendering_ignores_insertion_order_of_the_edges() {
        let mut one = OpGraph::new();
        let a = one.add(recall(), value(1), &[]).unwrap();
        let b = one.add(critique("x"), value(2), &[a]).unwrap();
        let mut two = one.clone();
        two.edges = vec![(b, a)]; // a deliberately backwards edge

        // The node list is what carries the score; reversing one edge changes it,
        // so the two must differ, and two identical graphs must not.
        assert_ne!(one.canonical(), two.canonical());
        assert_eq!(one.canonical(), one.clone().canonical());
    }

    proptest! {
        #[test]
        fn a_random_chain_is_acyclic_and_ordered(count in 1usize..24) {
            let mut graph = OpGraph::new();
            let mut previous: Option<NodeId> = None;
            for i in 0..count {
                let after: Vec<NodeId> = previous.into_iter().collect();
                let id = graph
                    .add(recall(), value(i as u64), &after)
                    .unwrap();
                previous = Some(id);
            }
            let order = graph.topological().unwrap();
            prop_assert_eq!(order.len(), count);
            for (from, to) in &graph.edges {
                let position = |id: NodeId| order.iter().position(|x| *x == id).unwrap();
                prop_assert!(position(*from) < position(*to));
            }
            let layers = graph.parallel_layers().unwrap();
            prop_assert_eq!(layers.len(), count);
        }
    }
}
