//! Lowering a compiled program to an executable computation DAG.
//!
//! This module is the phase's **only seam** to the runtime that will carry the
//! operations out. Everything else in `mm-metacog` speaks in terms of
//! [`CognitiveProgram`] and its graph of operations; here — and nowhere else —
//! that graph becomes a [`ComputationDag`] of nodes that a scheduler can fire in
//! parallel layers, with a [`dag_hash`] stable enough to be written to SQLite and
//! compared on a replay.
//!
//! The decomposition is the LLM-Compiler shape: a plan is a dependency DAG, and
//! every node in the same layer depends only on earlier layers, so the whole layer
//! is safe to run at once. That is the difference between a list of steps and a
//! program.
//!
//! Three rules hold:
//!
//! * **A DAG node id *is* a program node id.** Lowering never invents an identity,
//!   so a trace row's `node_id`, a `program_traces` row, and a DAG node all name
//!   the same operation. A separate numbering would make the trace and the DAG
//!   two records that have to be kept in agreement.
//! * **A tool the catalog does not hold is a refusal, not a runtime surprise.**
//!   A program that needs an unavailable tool cannot be lowered, and the refusal
//!   names the tool.
//! * **The hash covers everything that changes execution.** The program's canonical
//!   content, the DAG's own rendering, and the tool catalog all enter it, so two
//!   runs that would execute differently cannot share a `dag_hash`.

use serde::{Deserialize, Serialize};

use crate::episode::{EpisodeId, ProgramId, ToolId};
use crate::error::{MetacogError, Result};
use crate::op::{CognitiveOp, CostClass, OpClass};
use crate::program::CognitiveProgram;
use crate::tier::Tier;
use crate::value::OperationValue;

/// A DAG node's identifier. It is the program node id widened, so the two number
/// systems are the same one.
pub type DagNodeId = u32;

/// One operation, ready for a scheduler.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DagNode {
    /// The node's identifier, equal to `u32::from(node)`.
    pub id: DagNodeId,
    /// The program node this node came from.
    pub node: crate::graph::NodeId,
    /// The operation's tag, matching the `program_traces.op` column.
    pub op: String,
    /// The operation's class.
    pub op_class: OpClass,
    /// What the operation costs, as a band.
    pub cost_class: CostClass,
    /// The nodes that must run before this one, ascending.
    pub depends_on: Vec<DagNodeId>,
    /// The tools this node needs, ascending. Empty for a node that stays in
    /// process.
    pub required_tools: Vec<ToolId>,
    /// What the node is worth, carried through so a scheduler can argue about
    /// order without re-deriving it.
    pub value: OperationValue,
}

/// A computation DAG: the lowered form of a program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputationDag {
    /// The nodes, ordered by id.
    pub nodes: Vec<DagNode>,
    /// The nodes grouped into layers that may run in parallel.
    pub layers: Vec<Vec<DagNodeId>>,
}

impl ComputationDag {
    /// How many operations the DAG holds.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// How many dependency edges it holds.
    pub fn edge_count(&self) -> usize {
        self.nodes.iter().map(|n| n.depends_on.len()).sum()
    }

    /// The node with this id.
    pub fn node(&self, id: DagNodeId) -> Option<&DagNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The layer that holds this node, if any.
    pub fn layer_of(&self, id: DagNodeId) -> Option<usize> {
        self.layers.iter().position(|layer| layer.contains(&id))
    }

    /// The nodes that can start at once: the first layer.
    pub fn roots(&self) -> Vec<DagNodeId> {
        self.layers.first().cloned().unwrap_or_default()
    }

    /// The nodes nothing depends on.
    pub fn leaves(&self) -> Vec<DagNodeId> {
        let mut out: Vec<DagNodeId> = self
            .nodes
            .iter()
            .map(|n| n.id)
            .filter(|id| !self.nodes.iter().any(|other| other.depends_on.contains(id)))
            .collect();
        out.sort_unstable();
        out
    }

    /// A canonical rendering, deterministic in node and layer order.
    ///
    /// Nodes are rendered by id and the dependency and tool lists are already
    /// sorted, so the rendering is a function of the DAG alone and not of how it
    /// was assembled.
    pub fn canonical(&self) -> String {
        let nodes: Vec<String> = self
            .nodes
            .iter()
            .map(|node| {
                format!(
                    "{}:{}:{}:{}:deps[{}]:tools[{}]:{}",
                    node.id,
                    node.node,
                    node.op,
                    node.op_class.as_str(),
                    node.depends_on
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                    node.required_tools.join(","),
                    node.value.canonical()
                )
            })
            .collect();
        let layers: Vec<String> = self
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                format!(
                    "{index}[{}]",
                    layer
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect();
        format!("nodes{{{}}}layers{{{}}}", nodes.join(";"), layers.join(";"))
    }

    /// The structural refusals, as a plain message so both [`Self::validate`] and
    /// the lowering path can report them without converting error types twice.
    fn check(&self) -> std::result::Result<(), String> {
        if self.nodes.is_empty() {
            return Err("a computation DAG must hold at least one node".to_string());
        }
        let mut ids: Vec<DagNodeId> = self.nodes.iter().map(|n| n.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != before {
            return Err("a computation DAG may not repeat a node id".to_string());
        }
        for node in &self.nodes {
            for dependency in &node.depends_on {
                if *dependency == node.id {
                    return Err(format!("node {} depends on itself", node.id));
                }
                if !self.nodes.iter().any(|other| other.id == *dependency) {
                    return Err(format!(
                        "node {} depends on {dependency}, which is not in the DAG",
                        node.id
                    ));
                }
            }
        }
        let mut in_layers: Vec<DagNodeId> = self.layers.iter().flatten().copied().collect();
        let count = in_layers.len();
        in_layers.sort_unstable();
        in_layers.dedup();
        if in_layers != ids || count != ids.len() {
            return Err(
                "the layers must cover every node exactly once, and no other node".to_string(),
            );
        }
        Ok(())
    }

    /// Refuse a DAG that cannot be executed.
    pub fn validate(&self) -> Result<()> {
        self.check().map_err(MetacogError::Lower)
    }
}

/// The tools a lowered program needs, as a sorted, deduplicated set.
///
/// A catalog rather than a bare list because it is what the DAG hash is computed
/// against: two programs that need different tools may not share a hash, even when
/// their operations are identical.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCatalog {
    /// The tool names, sorted and deduplicated.
    pub tools: Vec<ToolId>,
}

impl ToolCatalog {
    /// Build a catalog, sorting and deduplicating what it is given.
    pub fn new(tools: Vec<ToolId>) -> Self {
        let mut tools = tools;
        tools.retain(|tool| !tool.trim().is_empty());
        tools.sort();
        tools.dedup();
        ToolCatalog { tools }
    }

    /// The catalog a program needs: exactly the tools its own nodes require.
    pub fn from_program(program: &CognitiveProgram) -> Self {
        ToolCatalog::new(program.required_tools.clone())
    }

    /// True when the catalog holds this tool.
    pub fn contains(&self, tool: &str) -> bool {
        self.tools
            .binary_search_by(|candidate| candidate.as_str().cmp(tool))
            .is_ok()
    }

    /// How many tools the catalog holds.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// True when the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// The catalog's content hash: sha256 over the tool names.
    pub fn hash(&self) -> String {
        mm_core::content_hash(self.tools.join("\n").as_bytes())
    }
}

/// A program lowered for execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionDocument {
    /// The program this document lowers.
    pub program_id: ProgramId,
    /// The episode the program was compiled for.
    pub episode_id: EpisodeId,
    /// The tier whose policy bounded the program.
    pub tier: Tier,
    /// The lowered DAG.
    pub dag: ComputationDag,
    /// The stable hash of the whole document: program, DAG and tool catalog.
    pub dag_hash: String,
    /// The hash of the tool catalog alone.
    pub tool_catalog_hash: String,
}

/// Everything lowering can refuse.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum LowerError {
    /// A node needs a tool the catalog does not hold.
    #[error("node needs tool {tool:?}, which is not in the catalog")]
    MissingTool {
        /// The tool that is unavailable.
        tool: ToolId,
    },
    /// The program holds no operations, so there is nothing to execute.
    #[error("there is nothing to lower: the program holds no operations")]
    Empty,
    /// The graph is not decomposable into layers — it holds a cycle, or the DAG
    /// built from it is structurally unsound.
    #[error("the program graph cannot be decomposed: {0}")]
    Cycle(String),
    /// An operation is not allowed at the program's tier.
    #[error("{op} is not allowed at {tier}")]
    Disallowed {
        /// The offending operation's tag.
        op: String,
        /// The tier that forbids it.
        tier: Tier,
    },
}

impl From<LowerError> for MetacogError {
    fn from(e: LowerError) -> Self {
        match e {
            LowerError::Disallowed { .. } => MetacogError::Compile(e.to_string()),
            LowerError::MissingTool { .. } | LowerError::Empty | LowerError::Cycle(_) => {
                MetacogError::Lower(e.to_string())
            }
        }
    }
}

/// The tools a single operation reaches for.
///
/// Only two operations reach outside the process, and each names exactly one
/// thing: an `Observe` reads a source, an `Act` performs an action. Everything
/// else is cognition that stays in process, which is why its returned list is
/// empty rather than the operation's own target.
fn node_tools(op: &CognitiveOp) -> Vec<ToolId> {
    match op {
        CognitiveOp::Observe { source } => vec![source.clone()],
        CognitiveOp::Act { action } => vec![action.clone()],
        _ => Vec::new(),
    }
}

/// The canonical rendering the `dag_hash` is taken over.
///
/// The program's canonical content comes first because it is what a replay
/// compares; the DAG rendering and the catalog hash then cover what execution adds
/// — the layering, the tool set, and the resolved dependencies.
fn canonical_document(
    program: &CognitiveProgram,
    dag: &ComputationDag,
    tools: &ToolCatalog,
) -> String {
    format!(
        "program={}\ndag={}\ntools={}",
        program.canonical(),
        dag.canonical(),
        tools.hash()
    )
}

/// Lower a program to an execution document.
///
/// Every refusal is typed: a node needing an unavailable tool, a program with no
/// operations, a graph that cannot be layered, and an operation the tier forbids
/// each come back as their own [`LowerError`].
pub fn to_execution_document(
    program: &CognitiveProgram,
    tools: &ToolCatalog,
) -> std::result::Result<ExecutionDocument, LowerError> {
    if program.graph.is_empty() {
        return Err(LowerError::Empty);
    }
    let policy = program.policy();

    for op_node in &program.graph.nodes {
        if !policy.allows_op(&op_node.op) {
            return Err(LowerError::Disallowed {
                op: op_node.op.tag().to_string(),
                tier: program.tier,
            });
        }
        for tool in node_tools(&op_node.op) {
            if !tools.contains(&tool) {
                return Err(LowerError::MissingTool { tool });
            }
        }
    }

    let dag = build_dag(program)?;
    dag.check().map_err(LowerError::Cycle)?;
    // The program's own validity is re-checked here rather than assumed: lowering
    // is the seam to the executor, and an executor must not be handed a program
    // whose steps and graph disagree.
    program
        .validate()
        .map_err(|e| LowerError::Cycle(format!("the program is not executable: {e}")))?;

    let tool_catalog_hash = tools.hash();
    let dag_hash = mm_core::content_hash(canonical_document(program, &dag, tools).as_bytes());

    Ok(ExecutionDocument {
        program_id: program.id,
        episode_id: program.episode_id,
        tier: program.tier,
        dag,
        dag_hash,
        tool_catalog_hash,
    })
}

/// The stable hash of a program lowered against a tool catalog.
///
/// Equal to `to_execution_document(...)?.dag_hash` for every program that lowers,
/// because both build the DAG the same way. A graph that cannot be layered cannot
/// be lowered at all, so there is no execution to name: that case hashes the
/// program and the catalog alone, which keeps this function total without
/// inventing a DAG nobody can execute.
pub fn dag_hash(program: &CognitiveProgram, tools: &ToolCatalog) -> String {
    match build_dag(program) {
        Ok(dag) => mm_core::content_hash(canonical_document(program, &dag, tools).as_bytes()),
        Err(_) => mm_core::content_hash(
            format!("program={}\ntools={}", program.canonical(), tools.hash()).as_bytes(),
        ),
    }
}

/// Build the DAG half of a document, without the tool or program-validity checks,
/// so both [`to_execution_document`] and [`dag_hash`] lower a program identically.
fn build_dag(program: &CognitiveProgram) -> std::result::Result<ComputationDag, LowerError> {
    let nodes = program
        .graph
        .nodes
        .iter()
        .map(|op_node| {
            let mut depends_on: Vec<DagNodeId> = program
                .graph
                .dependencies(op_node.id)
                .into_iter()
                .map(u32::from)
                .collect();
            depends_on.sort_unstable();
            depends_on.dedup();
            DagNode {
                id: u32::from(op_node.id),
                node: op_node.id,
                op: op_node.op.tag().to_string(),
                op_class: op_node.op.class(),
                cost_class: op_node.op.cost_class(),
                depends_on,
                required_tools: node_tools(&op_node.op),
                value: op_node.value,
            }
        })
        .collect();
    let layers: Vec<Vec<DagNodeId>> = program
        .graph
        .parallel_layers()
        .map_err(|e| LowerError::Cycle(e.to_string()))?
        .into_iter()
        .map(|layer| layer.into_iter().map(u32::from).collect())
        .collect();
    Ok(ComputationDag { nodes, layers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::CognitiveBudget;
    use crate::episode::{CognitiveEpisode, Context};
    use crate::graph::OpGraph;
    use crate::program::{DefaultCompiler, ProgramCompiler};
    use crate::scan::{ScanIssue, ScanResult, Uncertainty};
    use mm_core::Ulid;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    /// An episode with something to observe, two candidates to compare, and a
    /// material issue — enough to compile a program that needs a tool.
    fn fixture() -> (CognitiveEpisode, ScanResult) {
        let mut episode = CognitiveEpisode::new(
            ulid(1),
            ulid(2),
            Context::new("decide whether to roll back the deploy").with_novelty(0.4),
            CognitiveBudget::from_spec(12, 6, 1.0, 60_000).unwrap(),
        )
        .unwrap()
        .with_claims(vec![ulid(3)])
        .with_candidates(vec!["rollback".to_string(), "roll_forward".to_string()]);
        let scan = ScanResult {
            issues: vec![ScanIssue {
                kind: "missing_evidence".to_string(),
                materiality: 0.7,
                target: "rollback".to_string(),
            }],
            uncertainties: vec![Uncertainty {
                id: "u1".to_string(),
                description: "does the window hold".to_string(),
                magnitude: 0.6,
            }],
            stakes: 0.8,
            irreversibility: 0.4,
            verification_value: 0.7,
            ..ScanResult::default()
        };
        episode.select_tier(&scan);
        (episode, scan)
    }

    fn compiled_program() -> CognitiveProgram {
        let (episode, scan) = fixture();
        DefaultCompiler::new()
            .compile(&episode, &scan, &episode.budget)
            .unwrap()
    }

    /// A program with no operations, for the empty-lowering refusal.
    fn empty_program() -> CognitiveProgram {
        let mut program = compiled_program();
        program.graph = OpGraph::new();
        program.steps.clear();
        program
    }

    /// A program that reaches for nothing: no claims to observe and no action to
    /// take, so an empty catalog can lower it.
    fn tool_free_program() -> CognitiveProgram {
        let mut episode = CognitiveEpisode::new(
            ulid(9),
            ulid(10),
            Context::new("choose between two known options"),
            CognitiveBudget::from_spec(8, 4, 1.0, 60_000).unwrap(),
        )
        .unwrap();
        episode.claims.clear();
        let scan = ScanResult {
            stakes: 0.1,
            ..ScanResult::default()
        };
        episode.select_tier(&scan);
        DefaultCompiler::new()
            .compile(&episode, &scan, &episode.budget)
            .unwrap()
    }

    #[test]
    fn lowering_twice_is_identical() {
        let program = compiled_program();
        let tools = ToolCatalog::from_program(&program);
        let first = to_execution_document(&program, &tools).unwrap();
        let second = to_execution_document(&program, &tools).unwrap();
        assert_eq!(first.dag_hash, second.dag_hash);
        assert_eq!(first.dag.canonical(), second.dag.canonical());
        assert_eq!(first, second);
        assert_eq!(dag_hash(&program, &tools), first.dag_hash);
        assert_eq!(first.tool_catalog_hash, tools.hash());
    }

    #[test]
    fn the_dag_hash_is_sha256_hex() {
        let program = compiled_program();
        let tools = ToolCatalog::from_program(&program);
        let hash = dag_hash(&program, &tools);
        assert_eq!(hash.len(), 64, "sha256 renders as 64 hex characters");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{hash}");
        let document = to_execution_document(&program, &tools).unwrap();
        assert_eq!(document.dag_hash, hash);
    }

    #[test]
    fn a_missing_tool_is_refused() {
        let program = compiled_program();
        assert!(
            !program.required_tools.is_empty(),
            "the fixture must compile an operation that reaches outside"
        );
        let empty = ToolCatalog::new(Vec::new());
        let error = to_execution_document(&program, &empty).unwrap_err();
        match error {
            LowerError::MissingTool { tool } => {
                assert!(program.required_tools.contains(&tool), "{tool}");
            }
            other => panic!("expected a missing tool, got {other:?}"),
        }
        assert_eq!(
            MetacogError::from(LowerError::MissingTool {
                tool: "x".to_string()
            })
            .code(),
            "lower"
        );
    }

    #[test]
    fn an_empty_program_is_refused() {
        let program = empty_program();
        let tools = ToolCatalog::new(Vec::new());
        assert_eq!(
            to_execution_document(&program, &tools).unwrap_err(),
            LowerError::Empty
        );
        let mut dag = ComputationDag {
            nodes: Vec::new(),
            layers: Vec::new(),
        };
        assert!(dag.validate().is_err());
        dag.nodes.push(DagNode {
            id: 0,
            node: 0,
            op: "recall".to_string(),
            op_class: OpClass::Recall,
            cost_class: CostClass::Cheap,
            depends_on: Vec::new(),
            required_tools: Vec::new(),
            value: OperationValue::new(0.5, 0.5, 0.5, 0.05).unwrap(),
        });
        assert!(dag.validate().is_err(), "the layers still cover nothing");
        dag.layers = vec![vec![0]];
        dag.validate().unwrap();
    }

    #[test]
    fn dependencies_and_layers_agree_with_the_program_graph() {
        let program = compiled_program();
        let tools = ToolCatalog::from_program(&program);
        let document = to_execution_document(&program, &tools).unwrap();
        let dag = &document.dag;

        assert_eq!(dag.node_count(), program.node_count());
        assert_eq!(dag.edge_count(), program.edge_count());

        for program_node in &program.graph.nodes {
            let dag_node = dag.node(u32::from(program_node.id)).unwrap();
            assert_eq!(dag_node.node, program_node.id);
            assert_eq!(dag_node.op, program_node.op.tag());
            assert_eq!(dag_node.op_class, program_node.op.class());
            let expected: Vec<DagNodeId> = program
                .graph
                .dependencies(program_node.id)
                .into_iter()
                .map(u32::from)
                .collect();
            assert_eq!(dag_node.depends_on, expected);
            assert!(dag_node.depends_on.iter().all(|dep| *dep < dag_node.id));
        }

        // Every node appears in exactly one layer, and the layers reproduce the
        // program's own layering.
        let mut flattened: Vec<DagNodeId> = dag.layers.iter().flatten().copied().collect();
        let expected_layers = program.graph.parallel_layers().unwrap();
        assert_eq!(dag.layers.len(), expected_layers.len());
        for (layer, expected) in dag.layers.iter().zip(expected_layers.iter()) {
            let expected: Vec<DagNodeId> = expected.iter().copied().map(u32::from).collect();
            assert_eq!(layer, &expected);
        }
        flattened.sort_unstable();
        let mut ids: Vec<DagNodeId> = dag.nodes.iter().map(|n| n.id).collect();
        ids.sort_unstable();
        assert_eq!(flattened, ids);
        assert_eq!(dag.layer_of(0), Some(0));
        assert_eq!(dag.layer_of(999), None);
        assert!(dag.node(999).is_none());
        assert!(!dag.roots().is_empty());
        assert!(!dag.leaves().is_empty());
    }

    #[test]
    fn different_tool_sets_hash_differently() {
        let program = compiled_program();
        let small = ToolCatalog::from_program(&program);
        let larger = ToolCatalog::new(
            program
                .required_tools
                .iter()
                .cloned()
                .chain(["extra:tool".to_string()])
                .collect(),
        );
        assert_ne!(small.hash(), larger.hash());
        assert!(larger.contains("extra:tool"));
        assert!(!small.contains("extra:tool"));
        assert_ne!(
            dag_hash(&program, &small),
            dag_hash(&program, &larger),
            "the catalog is part of what a hash covers"
        );

        // A program that needs nothing lowers against an empty catalog.
        let tool_free = tool_free_program();
        assert!(tool_free.required_tools.is_empty());
        let document = to_execution_document(&tool_free, &ToolCatalog::new(Vec::new())).unwrap();
        assert!(document.dag.node_count() > 0);
        assert_eq!(
            document.dag_hash,
            dag_hash(&tool_free, &ToolCatalog::new(Vec::new()))
        );
    }

    #[test]
    fn a_catalog_sorts_deduplicates_and_refuses_blank_names() {
        let catalog = ToolCatalog::new(vec![
            "b".to_string(),
            "a".to_string(),
            "b".to_string(),
            "  ".to_string(),
        ]);
        assert_eq!(catalog.tools, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(catalog.len(), 2);
        assert!(!catalog.is_empty());
        assert!(catalog.contains("a"));
        assert!(!catalog.contains("c"));
        assert_eq!(ToolCatalog::default().len(), 0);
        assert!(ToolCatalog::new(Vec::new()).is_empty());
    }

    #[test]
    fn a_tier_that_forbids_an_operation_refuses_to_lower_it() {
        let mut program = compiled_program();
        assert!(program.node_count() > 1);
        // Lower the tier to `T0`, which allows only observe/recall/decide. The
        // program now holds operations `T0` forbids, and lowering must say so
        // rather than hand the executor something its policy never licensed.
        program.tier = Tier::T0;
        let tools = ToolCatalog::from_program(&program);
        match to_execution_document(&program, &tools) {
            Err(LowerError::Disallowed { op, tier }) => {
                assert_eq!(tier, Tier::T0);
                assert!(!op.is_empty());
            }
            other => panic!("expected a disallowed operation, got {other:?}"),
        }
    }
}
