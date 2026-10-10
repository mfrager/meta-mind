//! The entity graph and its Personalized PageRank expansion (HippoRAG).
//!
//! Lexical search cannot find a memory that shares no words with the query. The
//! graph channel closes that gap: seed from the entities the query mentions, walk
//! the entity graph, and reach memories that were never lexically similar.
//!
//! Everything here is deterministic, which for a graph walk is a claim about
//! *order*: nodes are visited in ULID order, the iteration count is fixed rather
//! than convergence-based, and ties in the final scores are broken by ULID. A
//! replay therefore produces the same expansion the first run produced.
//!
//! The walk is bounded: at most [`MAX_HOPS`] hops and [`NODE_BUDGET`] nodes, so a
//! pathological graph cannot turn one recall into an unbounded traversal.

use std::collections::{BTreeMap, BTreeSet};

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::store::{ns, parse_id, timestamp_of, SqliteMemoryStore};

/// The damping factor the plan pins.
pub const DAMPING: f64 = 0.85;
/// The hop cap.
pub const MAX_HOPS: u32 = 2;
/// The node budget for one expansion.
pub const NODE_BUDGET: usize = 4096;
/// Fixed iteration count: convergence-based stopping is not deterministic across
/// machines, and a fixed count is.
pub const ITERATIONS: usize = 24;

/// One bi-temporal edge between two entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityEdge {
    /// The edge's ULID.
    pub id: Ulid,
    /// The entity the edge leaves.
    pub from: Ulid,
    /// The entity it arrives at.
    pub to: Ulid,
    /// How the two are related.
    pub relation: String,
    /// The edge's weight.
    pub weight: f32,
    /// World time: when the relation became true.
    pub valid_from: Timestamp,
    /// World time: when it stopped being true, if it has.
    pub valid_until: Option<Timestamp>,
}

/// A directed weighted entity graph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntityGraph {
    edges: Vec<EntityEdge>,
    adjacency: BTreeMap<Ulid, Vec<(Ulid, f64)>>,
    reverse: BTreeMap<Ulid, Vec<(Ulid, f64)>>,
}

impl EntityGraph {
    /// Build a graph from edges.
    pub fn from_edges(edges: Vec<EntityEdge>) -> Self {
        let mut adjacency: BTreeMap<Ulid, Vec<(Ulid, f64)>> = BTreeMap::new();
        let mut reverse: BTreeMap<Ulid, Vec<(Ulid, f64)>> = BTreeMap::new();
        for edge in &edges {
            adjacency
                .entry(edge.from)
                .or_default()
                .push((edge.to, f64::from(edge.weight)));
            reverse
                .entry(edge.to)
                .or_default()
                .push((edge.from, f64::from(edge.weight)));
        }
        // A neighbour list built from a table scan is already in insertion order;
        // sorting makes the graph a function of its edge set alone.
        for list in adjacency.values_mut().chain(reverse.values_mut()) {
            list.sort_by_key(|(node, _)| *node);
        }
        EntityGraph {
            edges,
            adjacency,
            reverse,
        }
    }

    /// Load every edge valid at `at`.
    pub async fn load(store: &SqliteMemoryStore, at: Option<Timestamp>) -> Result<Self> {
        let sql = match at {
            Some(_) => {
                "SELECT id, from_entity, to_entity, relation, weight, valid_from, valid_until \
                 FROM entity_edges WHERE valid_from <= ? AND (valid_until IS NULL OR valid_until > ?) \
                 ORDER BY id"
            }
            None => {
                "SELECT id, from_entity, to_entity, relation, weight, valid_from, valid_until \
                 FROM entity_edges ORDER BY id"
            }
        };
        let args = match at {
            Some(at) => vec![mm_core::Param::Int(ns(at)), mm_core::Param::Int(ns(at))],
            None => Vec::new(),
        };
        let rows = mm_core::Tabular::query_json(store.sqlite(), sql, args).await?;
        let mut edges = Vec::new();
        for row in &rows {
            let (Some(id), Some(from), Some(to), Some(relation)) = (
                row["id"].as_str(),
                row["from_entity"].as_str(),
                row["to_entity"].as_str(),
                row["relation"].as_str(),
            ) else {
                continue;
            };
            edges.push(EntityEdge {
                id: parse_id(id)?,
                from: parse_id(from)?,
                to: parse_id(to)?,
                relation: relation.to_string(),
                weight: row["weight"].as_f64().unwrap_or(1.0) as f32,
                valid_from: row["valid_from"]
                    .as_i64()
                    .map(timestamp_of)
                    .unwrap_or(Timestamp::EPOCH),
                valid_until: row["valid_until"].as_i64().map(timestamp_of),
            });
        }
        Ok(EntityGraph::from_edges(edges))
    }

    /// Every edge, in table order.
    pub fn edges(&self) -> &[EntityEdge] {
        &self.edges
    }

    /// How many edges the graph holds.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// True when the graph has no edges.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// The out-neighbours of a node.
    pub fn neighbours(&self, entity: &Ulid) -> Vec<Ulid> {
        self.adjacency
            .get(entity)
            .map(|list| list.iter().map(|(node, _)| *node).collect())
            .unwrap_or_default()
    }

    /// The union of a node's in- and out-neighbours.
    pub fn connected(&self, entity: &Ulid) -> Vec<Ulid> {
        let mut set: BTreeSet<Ulid> = self.neighbours(entity).into_iter().collect();
        if let Some(list) = self.reverse.get(entity) {
            set.extend(list.iter().map(|(node, _)| *node));
        }
        set.into_iter().collect()
    }

    /// Every node reachable from `seeds` within `hops`, seeded first.
    ///
    /// The traversal is breadth-first over a sorted frontier, so the result is a
    /// function of the edge set rather than of the order rows happened to arrive.
    pub fn expand(&self, seeds: &[Ulid], hops: u32) -> Vec<Ulid> {
        let hops = hops.min(MAX_HOPS);
        let mut seen: BTreeSet<Ulid> = seeds.iter().copied().collect();
        let mut frontier: Vec<Ulid> = seen.iter().copied().collect();
        let mut out: Vec<Ulid> = frontier.clone();
        for _ in 0..hops {
            let mut next: BTreeSet<Ulid> = BTreeSet::new();
            for node in &frontier {
                for neighbour in self.connected(node) {
                    if seen.insert(neighbour) {
                        next.insert(neighbour);
                    }
                }
                if seen.len() >= NODE_BUDGET {
                    break;
                }
            }
            if next.is_empty() || seen.len() >= NODE_BUDGET {
                break;
            }
            frontier = next.iter().copied().collect();
            out.extend(frontier.iter().copied());
        }
        out.truncate(NODE_BUDGET);
        out
    }

    /// Personalized PageRank over the graph, seeded at `seeds`.
    pub fn ppr(&self, seeds: &[Ulid], hops: u32, damping: f64) -> BTreeMap<Ulid, f64> {
        let nodes = self.expand(seeds, hops);
        let adjacency: BTreeMap<Ulid, Vec<(Ulid, f64)>> = nodes
            .iter()
            .map(|node| (*node, self.symmetric_neighbours(node)))
            .collect();
        let reachable: BTreeSet<Ulid> = nodes.into_iter().collect();
        ppr_scores(&adjacency, seeds, &reachable, damping)
    }

    /// In- and out-neighbours merged by node, keeping the larger weight.
    fn symmetric_neighbours(&self, node: &Ulid) -> Vec<(Ulid, f64)> {
        let mut merged: BTreeMap<Ulid, f64> = BTreeMap::new();
        if let Some(list) = self.adjacency.get(node) {
            for (other, weight) in list {
                merged.insert(*other, *weight);
            }
        }
        if let Some(list) = self.reverse.get(node) {
            for (other, weight) in list {
                merged
                    .entry(*other)
                    .and_modify(|w| *w = w.max(*weight))
                    .or_insert(*weight);
            }
        }
        merged.into_iter().collect()
    }

    /// Add an edge, returning its ULID.
    pub async fn add_edge(
        store: &SqliteMemoryStore,
        from: Ulid,
        to: Ulid,
        relation: &str,
        weight: f32,
        valid_from: Timestamp,
    ) -> Result<Ulid> {
        let id = store.next_id();
        mm_core::Tabular::execute(
            store.sqlite(),
            "INSERT INTO entity_edges (id, from_entity, to_entity, relation, weight, valid_from, \
             valid_until) VALUES (?, ?, ?, ?, ?, ?, NULL)",
            vec![
                mm_core::Param::Text(mm_core::ulid_string(&id)),
                mm_core::Param::Text(mm_core::ulid_string(&from)),
                mm_core::Param::Text(mm_core::ulid_string(&to)),
                mm_core::Param::Text(relation.to_string()),
                mm_core::Param::Real(f64::from(weight)),
                mm_core::Param::Int(ns(valid_from)),
            ],
        )
        .await?;
        Ok(id)
    }

    /// Close an edge at `until` rather than deleting it.
    pub async fn close_edge(store: &SqliteMemoryStore, id: &Ulid, until: Timestamp) -> Result<u64> {
        let affected = mm_core::Tabular::execute(
            store.sqlite(),
            "UPDATE entity_edges SET valid_until = ? WHERE id = ? AND valid_until IS NULL",
            vec![
                mm_core::Param::Int(ns(until)),
                mm_core::Param::Text(mm_core::ulid_string(id)),
            ],
        )
        .await?;
        Ok(affected)
    }
}

/// One power iteration of Personalized PageRank over `adjacency`.
///
/// `adjacency` is the *scoped* neighbour map — already restricted to the nodes the
/// expansion is allowed to visit — and `reachable` is the node set the mass may
/// occupy. Mass that lands outside is returned to the seeds, which is what keeps
/// the walk bounded without silently dropping probability.
pub fn ppr_scores(
    adjacency: &BTreeMap<Ulid, Vec<(Ulid, f64)>>,
    seeds: &[Ulid],
    reachable: &BTreeSet<Ulid>,
    damping: f64,
) -> BTreeMap<Ulid, f64> {
    let mut scores: BTreeMap<Ulid, f64> = reachable.iter().map(|node| (*node, 0.0)).collect();
    if reachable.is_empty() {
        return scores;
    }
    let live_seeds: Vec<Ulid> = seeds
        .iter()
        .copied()
        .filter(|seed| reachable.contains(seed))
        .collect();
    if live_seeds.is_empty() {
        return scores;
    }
    let seed_share = 1.0 / live_seeds.len() as f64;
    let mut personalization: BTreeMap<Ulid, f64> = BTreeMap::new();
    for seed in &live_seeds {
        personalization.insert(*seed, seed_share);
    }
    for (node, score) in scores.iter_mut() {
        *score = personalization.get(node).copied().unwrap_or(0.0);
    }

    let out_weights: BTreeMap<Ulid, f64> = adjacency
        .iter()
        .map(|(node, list)| {
            let total: f64 = list
                .iter()
                .filter(|(other, _)| reachable.contains(other))
                .map(|(_, weight)| weight.max(0.0))
                .sum();
            (*node, total)
        })
        .collect();

    for _ in 0..ITERATIONS {
        let mut next: BTreeMap<Ulid, f64> = reachable.iter().map(|node| (*node, 0.0)).collect();
        let mut dangling = 0.0f64;
        for (node, rank) in &scores {
            if *rank == 0.0 {
                continue;
            }
            let total = out_weights.get(node).copied().unwrap_or(0.0);
            if total <= 0.0 {
                dangling += rank;
                continue;
            }
            for (other, weight) in adjacency.get(node).into_iter().flatten() {
                if !reachable.contains(other) {
                    continue;
                }
                let share = rank * (weight.max(0.0) / total);
                if let Some(slot) = next.get_mut(other) {
                    *slot += damping * share;
                }
            }
        }
        // A dangling node has no out-edges; its mass goes back to the seeds rather
        // than vanishing, so the vector still sums to one.
        if dangling > 0.0 {
            for (seed, share) in &personalization {
                if let Some(slot) = next.get_mut(seed) {
                    *slot += damping * dangling * share;
                }
            }
        }
        for (node, share) in &personalization {
            if let Some(slot) = next.get_mut(node) {
                *slot += (1.0 - damping) * share;
            }
        }
        scores = next;
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn graph() -> EntityGraph {
        let ts = Timestamp::EPOCH;
        let edge = |n: u128, from: u128, to: u128, w: f32| EntityEdge {
            id: id(n),
            from: id(from),
            to: id(to),
            relation: "related".into(),
            weight: w,
            valid_from: ts,
            valid_until: None,
        };
        EntityGraph::from_edges(vec![
            edge(10, 1, 2, 1.0),
            edge(11, 2, 3, 1.0),
            edge(12, 3, 4, 1.0),
            edge(13, 4, 5, 1.0),
            edge(14, 1, 9, 0.5),
        ])
    }

    #[test]
    fn expansion_stops_at_the_hop_cap() {
        let graph = graph();
        let one = graph.expand(&[id(1)], 1);
        assert!(one.contains(&id(2)));
        assert!(
            !one.contains(&id(3)),
            "one hop must not reach two hops away"
        );
        let two = graph.expand(&[id(1)], 2);
        assert!(two.contains(&id(3)));
        assert!(!two.contains(&id(4)), "two hops must not reach three away");
    }

    #[test]
    fn ppr_is_deterministic_for_a_fixed_graph_and_seed() {
        let graph = graph();
        let a = graph.ppr(&[id(1)], MAX_HOPS, DAMPING);
        let b = graph.ppr(&[id(1)], MAX_HOPS, DAMPING);
        assert_eq!(a, b);
        assert!(!a.is_empty());
        // The seed itself carries the most mass, and a two-hop node carries less
        // than a one-hop node.
        assert!(a[&id(1)] > a[&id(2)]);
        assert!(a[&id(2)] > a[&id(3)]);
    }

    #[test]
    fn ppr_mass_is_conserved() {
        let graph = graph();
        let scores = graph.ppr(&[id(1)], MAX_HOPS, DAMPING);
        let total: f64 = scores.values().sum();
        assert!((total - 1.0).abs() < 1e-6, "mass was {total}");
    }

    #[test]
    fn a_seed_with_no_edges_keeps_its_own_mass() {
        // A seed the graph has never seen is not a bug: it is an entity the store
        // knows nothing about yet. It scores itself and nothing else, so the walk
        // adds no fabricated neighbours.
        let graph = graph();
        let scores = graph.ppr(&[id(999)], MAX_HOPS, DAMPING);
        assert_eq!(scores.len(), 1);
        assert!((scores[&id(999)] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn connected_merges_both_directions() {
        let graph = graph();
        assert_eq!(graph.connected(&id(3)), vec![id(2), id(4)]);
        assert_eq!(graph.connected(&id(1)), vec![id(2), id(9)]);
    }

    #[test]
    fn edges_are_sorted_by_node_not_by_arrival() {
        let forward = graph();
        let mut reversed = forward.edges().to_vec();
        reversed.reverse();
        let rebuilt = EntityGraph::from_edges(reversed);
        assert_eq!(
            forward.ppr(&[id(1)], MAX_HOPS, DAMPING),
            rebuilt.ppr(&[id(1)], MAX_HOPS, DAMPING)
        );
    }
}
