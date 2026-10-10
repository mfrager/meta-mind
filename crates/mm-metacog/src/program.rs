//! Compiling a scan into a typed cognitive program.
//!
//! The compiler is Self-Discover-style composition: it selects from what the
//! episode already brings — frames, doctrines, techniques, candidates — and the
//! classes the tier's policy allows, and *composes* them into a graph for this
//! episode. There is no fixed script: a `T0` episode gets a recall and a decision,
//! and a `T5` one gets inversion, adversarial critique, a bounded search and a
//! causality check, and both of them get exactly the operations their situation
//! justified.
//!
//! Two properties are load-bearing:
//!
//! * **Determinism.** The same `(episode, scan, budget)` produces the same
//!   program bytes *including its id*, because the id is derived from the
//!   program's own canonical content. That is what makes `episode replay`
//!   comparable and the `programs` row idempotent.
//! * **The policy is the ceiling.** Every added operation is checked against the
//!   tier's allowed classes and the operation cap, so a program cannot be
//!   compiled that the situation did not justify.

use mm_core::Ulid;
use serde::{Deserialize, Serialize};

use crate::budget::CognitiveBudget;
use crate::episode::{
    check_unit, AssumptionId, CandidateId, CriterionId, DoctrineId, EpisodeId, FrameId, ProgramId,
    RefClassId, TechniqueId, ToolId,
};
use crate::error::{MetacogError, Result};
use crate::graph::OpGraph;
use crate::op::{CognitiveOp, CostClass, OpClass};
use crate::scan::{ComparisonSpec, FailureMode, ScanResult};
use crate::tier::{ComputePolicy, Tier};
use crate::value::OperationValue;

/// Irreversibility at or above which the controller refuses to compile an `Act`.
pub const IRREVERSIBLE_THRESHOLD: f64 = 0.5;

/// The expected value below which no further operation is worth running.
pub const EXPECTED_VALUE_FLOOR: f64 = 0.01;

/// When the loop must stop even if the budget is not spent.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoppingCondition {
    /// Remaining uncertainty is below this.
    RemainingUncertaintyBelow(f64),
    /// No ready operation's expected value clears this.
    ExpectedValueBelow(f64),
    /// The answer is confident enough.
    RequiredConfidenceMet(f64),
    /// The budget is spent.
    BudgetExhausted,
    /// An irreversible action was reached.
    IrreversibleActionReached,
}

impl StoppingCondition {
    /// The condition's stable name.
    pub fn kind(&self) -> &'static str {
        match self {
            StoppingCondition::RemainingUncertaintyBelow(_) => "remaining_uncertainty_below",
            StoppingCondition::ExpectedValueBelow(_) => "expected_value_below",
            StoppingCondition::RequiredConfidenceMet(_) => "required_confidence_met",
            StoppingCondition::BudgetExhausted => "budget_exhausted",
            StoppingCondition::IrreversibleActionReached => "irreversible_action_reached",
        }
    }

    /// The condition's threshold, when it has one.
    pub fn threshold(&self) -> Option<f64> {
        match self {
            StoppingCondition::RemainingUncertaintyBelow(v)
            | StoppingCondition::ExpectedValueBelow(v)
            | StoppingCondition::RequiredConfidenceMet(v) => Some(*v),
            StoppingCondition::BudgetExhausted | StoppingCondition::IrreversibleActionReached => {
                None
            }
        }
    }

    /// A canonical rendering, used by the program's content hash.
    pub fn canonical(&self) -> String {
        match self.threshold() {
            Some(value) => format!("{}={value:.9}", self.kind()),
            None => self.kind().to_string(),
        }
    }
}

/// One step of a compiled program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgramStep {
    /// The step's position, contiguous from 1.
    pub order: u8,
    /// The node it names.
    pub node: crate::graph::NodeId,
    /// The operation.
    pub op: CognitiveOp,
    /// What it is worth.
    pub value: OperationValue,
}

/// A typed graph of cognitive operations, compiled for one episode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CognitiveProgram {
    /// The program's identifier, derived from its own content.
    pub id: ProgramId,
    /// The episode it was compiled for.
    pub episode_id: EpisodeId,
    /// The tier whose policy bounded it.
    pub tier: Tier,
    /// The graph.
    pub graph: OpGraph,
    /// The graph in execution order.
    pub steps: Vec<ProgramStep>,
    /// The frames that shaped it.
    pub frames: Vec<FrameId>,
    /// The doctrines that shaped it.
    pub doctrines: Vec<DoctrineId>,
    /// The techniques that shaped it.
    pub techniques: Vec<TechniqueId>,
    /// The reference classes the answer will be compared against.
    pub reference_classes: Vec<RefClassId>,
    /// The assumptions that must be verified.
    pub assumptions_to_verify: Vec<AssumptionId>,
    /// The comparisons the answer needs.
    pub comparisons: Vec<ComparisonSpec>,
    /// The candidate answers.
    pub candidate_actions: Vec<CandidateId>,
    /// What the answer will be judged on.
    pub evaluation_criteria: Vec<CriterionId>,
    /// What the program needs to run.
    pub required_tools: Vec<ToolId>,
    /// When the loop must stop.
    pub stopping: Vec<StoppingCondition>,
}

impl CognitiveProgram {
    /// The compute policy that bounded this program.
    pub fn policy(&self) -> ComputePolicy {
        self.tier.policy()
    }

    /// The number of operations.
    pub fn node_count(&self) -> usize {
        self.graph.len()
    }

    /// The number of dependency edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edges.len()
    }

    /// The step at this order.
    pub fn step(&self, order: u8) -> Option<&ProgramStep> {
        self.steps.iter().find(|s| s.order == order)
    }

    /// The stopping condition of a kind, if the program carries one.
    pub fn stopping_condition(&self, kind: &str) -> Option<&StoppingCondition> {
        self.stopping.iter().find(|c| c.kind() == kind)
    }

    /// The operations that reach outside the process.
    pub fn tool_bound_ops(&self) -> Vec<&CognitiveOp> {
        self.graph
            .nodes
            .iter()
            .map(|n| &n.op)
            .filter(|op| op.is_tool_bound())
            .collect()
    }

    /// A canonical rendering, which is what the program's id and the DAG hash are
    /// derived from. Deterministic in every field's order.
    pub fn canonical(&self) -> String {
        let stopping: Vec<String> = self.stopping.iter().map(|c| c.canonical()).collect();
        let steps: Vec<String> = self
            .steps
            .iter()
            .map(|s| {
                format!(
                    "{}:{}:{}:{}",
                    s.order,
                    s.node,
                    s.op.tag(),
                    s.value.canonical()
                )
            })
            .collect();
        let comparisons: Vec<String> = self
            .comparisons
            .iter()
            .map(|c| format!("{}|{}|{}", c.subject, c.versus, c.criterion))
            .collect();
        format!(
            "episode={}\ntier={}\ngraph={}\nsteps={}\nframes={}\ndoctrines={}\n\
             techniques={}\nreference_classes={}\nassumptions={}\ncomparisons={}\n\
             candidates={}\ncriteria={}\ntools={}\nstopping={}",
            mm_core::ulid_string(&self.episode_id),
            self.tier.as_str(),
            self.graph.canonical(),
            steps.join(","),
            self.frames.join(","),
            self.doctrines.join(","),
            self.techniques.join(","),
            self.reference_classes.join(","),
            self.assumptions_to_verify
                .iter()
                .map(mm_core::ulid_string)
                .collect::<Vec<_>>()
                .join(","),
            comparisons.join(","),
            self.candidate_actions.join(","),
            self.evaluation_criteria.join(","),
            self.required_tools.join(","),
            stopping.join(","),
        )
    }

    /// The program's content hash: sha256 over its canonical rendering.
    pub fn content_hash(&self) -> String {
        mm_core::content_hash(self.canonical().as_bytes())
    }

    /// Refuse a program that cannot be executed or replayed.
    pub fn validate(&self) -> Result<()> {
        self.graph.validate()?;
        if self.steps.is_empty() {
            return Err(MetacogError::Compile(
                "a program must carry at least one step".to_string(),
            ));
        }
        if self.steps.len() != self.graph.len() {
            return Err(MetacogError::Compile(format!(
                "{} step(s) for {} node(s); every node must appear exactly once",
                self.steps.len(),
                self.graph.len()
            )));
        }
        let policy = self.policy();
        for (index, step) in self.steps.iter().enumerate() {
            let expected = u8::try_from(index + 1).map_err(|_| {
                MetacogError::Compile("a program may hold at most 255 steps".to_string())
            })?;
            if step.order != expected {
                return Err(MetacogError::Compile(format!(
                    "step {} has order {}; orders must be contiguous from 1",
                    index, step.order
                )));
            }
            if !policy.allows_op(&step.op) {
                return Err(MetacogError::Compile(format!(
                    "step {} uses {} which {} forbids",
                    step.order,
                    step.op.class(),
                    self.tier
                )));
            }
            match self.graph.node(step.node) {
                Some(node) if node.op == step.op => {}
                _ => {
                    return Err(MetacogError::Compile(format!(
                        "step {} names node {} which is not in the graph",
                        step.order, step.node
                    )))
                }
            }
        }
        // The steps must be a topological order: a step may only name a node once
        // every node it depends on has run.
        let mut seen: Vec<crate::graph::NodeId> = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            for dependency in self.graph.dependencies(step.node) {
                if !seen.contains(&dependency) {
                    return Err(MetacogError::Compile(format!(
                        "step {} runs node {} before its dependency {dependency}",
                        step.order, step.node
                    )));
                }
            }
            if seen.contains(&step.node) {
                return Err(MetacogError::Compile(format!(
                    "node {} appears twice in the steps",
                    step.node
                )));
            }
            seen.push(step.node);
        }
        if self.stopping.is_empty() {
            return Err(MetacogError::Compile(
                "a program must attach at least one stopping condition".to_string(),
            ));
        }
        if !self.stopping.contains(&StoppingCondition::BudgetExhausted) {
            return Err(MetacogError::Compile(
                "every program must be able to stop on the budget".to_string(),
            ));
        }
        for condition in &self.stopping {
            if let Some(threshold) = condition.threshold() {
                check_unit("stopping.threshold", threshold)?;
            }
        }
        Ok(())
    }
}

/// Turn an episode and its scan into a program.
pub trait ProgramCompiler: Send + Sync {
    /// Compile. Composition selects from the episode's frames and techniques and
    /// the tier's allowed classes, so two materially different episodes compile to
    /// materially different programs.
    fn compile(
        &self,
        episode: &crate::episode::CognitiveEpisode,
        scan: &ScanResult,
        budget: &CognitiveBudget,
    ) -> Result<CognitiveProgram>;
}

/// The compiler the controller uses unless a caller supplies another.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultCompiler;

impl DefaultCompiler {
    /// A compiler.
    pub fn new() -> Self {
        DefaultCompiler
    }
}

impl ProgramCompiler for DefaultCompiler {
    fn compile(
        &self,
        episode: &crate::episode::CognitiveEpisode,
        scan: &ScanResult,
        budget: &CognitiveBudget,
    ) -> Result<CognitiveProgram> {
        episode.validate()?;
        scan.validate()?;
        let policy = episode.tier.policy();
        policy.validate()?;

        // The cap is the tighter of what the tier allows and what the budget can
        // pay for, so a compiled program can never be one the budget refuses.
        let cap = policy.max_ops.min(budget.max_ops).max(1) as usize;

        let mut graph = OpGraph::new();
        let mut assumptions_checked: Vec<AssumptionId> = Vec::new();

        let add = |graph: &mut OpGraph,
                   op: CognitiveOp,
                   episode: &crate::episode::CognitiveEpisode,
                   scan: &ScanResult,
                   policy: &ComputePolicy,
                   after: &[crate::graph::NodeId]|
         -> Result<crate::graph::NodeId> {
            let value = value_for(&op, episode, scan, policy)?;
            graph.add(op, value, after)
        };

        // R1: every episode begins by recalling what it already knows.
        let root = add(
            &mut graph,
            CognitiveOp::Recall {
                query: episode.goal_text().to_string(),
            },
            episode,
            scan,
            &policy,
            &[],
        )?;

        // R2: anything with a category gets classified.
        if policy.allows(OpClass::Classify) && !episode.candidates.is_empty() && graph.len() < cap {
            add(
                &mut graph,
                CognitiveOp::Classify {
                    candidate: episode.candidates[0].clone(),
                    taxonomy: "problem_kind".to_string(),
                },
                episode,
                scan,
                &policy,
                &[root],
            )?;
        }

        // R3: two candidates get compared.
        if policy.allows(OpClass::Compare) && episode.candidates.len() > 1 && graph.len() < cap {
            add(
                &mut graph,
                CognitiveOp::Compare {
                    candidates: episode.candidates.clone(),
                },
                episode,
                scan,
                &policy,
                &[root],
            )?;
        }

        // R4: the first claim is observed directly when observing is allowed.
        if policy.allows(OpClass::Observe) && !episode.claims.is_empty() && graph.len() < cap {
            add(
                &mut graph,
                CognitiveOp::Observe {
                    source: mm_core::ulid_string(&episode.claims[0]),
                },
                episode,
                scan,
                &policy,
                &[root],
            )?;
        }

        // R5: every issue the scan called material becomes a critique.
        if policy.allows(OpClass::Critique) {
            for issue in scan.issues_by_materiality() {
                if graph.len() >= cap {
                    break;
                }
                add(
                    &mut graph,
                    CognitiveOp::Critique {
                        candidate: issue.target.clone(),
                    },
                    episode,
                    scan,
                    &policy,
                    &[root],
                )?;
            }
        }

        // R6: the assumptions worth verifying become checks. The episode's list is
        // taken in identifier order — the controller may not rank assumptions by a
        // priority it cannot see, and the Phase 6 ledger's own ordering is what
        // `epistemic assumption priority` reports.
        let assumption_budget = assumptions_to_verify(episode.tier);
        if policy.allows(OpClass::CheckAssumptions) && assumption_budget > 0 {
            let mut ids = episode.assumptions.clone();
            ids.sort_unstable();
            ids.dedup();
            for id in ids.into_iter().take(assumption_budget) {
                if graph.len() >= cap {
                    break;
                }
                assumptions_checked.push(id);
                add(
                    &mut graph,
                    CognitiveOp::CheckAssumption { assumption: id },
                    episode,
                    scan,
                    &policy,
                    &[root],
                )?;
            }
        }

        // R7: an active technique means there is a precedent worth finding.
        if policy.allows(OpClass::SearchPrecedent)
            && !episode.active_techniques.is_empty()
            && graph.len() < cap
        {
            add(
                &mut graph,
                CognitiveOp::SearchPrecedent {
                    problem: episode.goal_text().to_string(),
                },
                episode,
                scan,
                &policy,
                &[root],
            )?;
        }

        // R8: contradiction and causality checks answer the scan's own warnings.
        if policy.allows(OpClass::CheckContradictions) {
            for claim in episode.claims.iter().take(2) {
                if graph.len() >= cap {
                    break;
                }
                add(
                    &mut graph,
                    CognitiveOp::CheckContradiction { claim: *claim },
                    episode,
                    scan,
                    &policy,
                    &[root],
                )?;
            }
        }
        if policy.allows(OpClass::CheckCausality) {
            for FailureMode { mode, .. } in scan.failure_modes.iter().take(2) {
                if graph.len() >= cap {
                    break;
                }
                add(
                    &mut graph,
                    CognitiveOp::CheckCausality {
                        cause: mode.clone(),
                        effect: "the answer".to_string(),
                    },
                    episode,
                    scan,
                    &policy,
                    &[root],
                )?;
            }
        }

        // R9: a simpler alternative is a hypothesis worth testing.
        if policy.allows(OpClass::Predict) {
            for alternative in scan.simpler_alternatives.iter().take(2) {
                if graph.len() >= cap {
                    break;
                }
                add(
                    &mut graph,
                    CognitiveOp::FormHypothesis {
                        proposition: alternative.clone(),
                    },
                    episode,
                    scan,
                    &policy,
                    &[root],
                )?;
            }
        }

        // R10: an inversion of the goal, at the tiers that can hold one.
        if policy.allows(OpClass::Invert) && graph.len() < cap {
            add(
                &mut graph,
                CognitiveOp::Invert { goal: episode.goal },
                episode,
                scan,
                &policy,
                &[root],
            )?;
        }

        // R11: each critique is refined, which is Self-Refine's generate → critique
        // → refine with the refinement attached to the criticism that prompted it.
        if policy.allows(OpClass::Refine) {
            let critiques: Vec<(crate::graph::NodeId, CandidateId)> = graph
                .nodes
                .iter()
                .filter_map(|node| match &node.op {
                    CognitiveOp::Critique { candidate } => Some((node.id, candidate.clone())),
                    _ => None,
                })
                .collect();
            for (node, candidate) in critiques {
                if graph.len() >= cap {
                    break;
                }
                add(
                    &mut graph,
                    CognitiveOp::Refine { candidate },
                    episode,
                    scan,
                    &policy,
                    &[node],
                )?;
            }
        }

        // R12: a bounded search, whose width came from the tier and nothing else.
        if policy.allows(OpClass::Search) && policy.search_width > 0 && graph.len() < cap {
            add(
                &mut graph,
                CognitiveOp::Search {
                    over: episode.goal_text().to_string(),
                    budget: policy.search_width,
                },
                episode,
                scan,
                &policy,
                &[root],
            )?;
        }

        // R13: several candidates are combined before a decision.
        let synthesise = policy.allows(OpClass::Synthesize) && episode.candidates.len() > 1;
        let synthesised = if synthesise && graph.len() < cap {
            Some(add(
                &mut graph,
                CognitiveOp::Synthesize {
                    candidates: episode.candidates.clone(),
                },
                episode,
                scan,
                &policy,
                &[root],
            )?)
        } else {
            None
        };

        // R14: decide, after everything that produced an answer.
        //
        // A decision with no candidate is a decision to do nothing, and that still
        // has to name what was decided. The goal statement stands in, so a `Decide`
        // node always targets something — an operation with no target is an
        // operation no shape can check.
        let effective_candidates = if episode.candidates.is_empty() {
            vec![episode.goal_text().to_string()]
        } else {
            episode.candidates.clone()
        };
        //
        // The decision depends on the recall that initialised the provisional model,
        // and on nothing else: every investigation operation updates that model as a
        // side effect, so a hard edge to each of them would say the decision *cannot*
        // be taken until all of them run, which is exactly the over-thinking the
        // stopping conditions exist to prevent. `steps` still places the decision
        // last, and the loop honours that ordering.
        let mut dependencies: Vec<crate::graph::NodeId> = vec![root];
        if let Some(s) = synthesised {
            dependencies.push(s);
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        let decide = add(
            &mut graph,
            CognitiveOp::Decide {
                candidates: effective_candidates.clone(),
            },
            episode,
            scan,
            &policy,
            &dependencies,
        )?;

        // R15: an irreversible action is reached and *not taken*. Act is compiled
        // only when the decision is reversible enough to make one at all.
        let irreversible = scan.irreversibility >= IRREVERSIBLE_THRESHOLD;
        if policy.allows(OpClass::Act) && !irreversible && !episode.candidates.is_empty() {
            add(
                &mut graph,
                CognitiveOp::Act {
                    action: episode.candidates[0].clone(),
                },
                episode,
                scan,
                &policy,
                &[decide],
            )?;
        }

        // Steps: the graph's own deterministic topological order, ordered from 1.
        let order = graph.topological()?;
        let mut steps = Vec::with_capacity(order.len());
        for (index, node) in order.iter().enumerate() {
            let node_ref = graph.node(*node).ok_or(MetacogError::NoSuchNode(*node))?;
            steps.push(ProgramStep {
                order: u8::try_from(index + 1).map_err(|_| {
                    MetacogError::Compile("a program may hold at most 255 steps".to_string())
                })?,
                node: *node,
                op: node_ref.op.clone(),
                value: node_ref.value,
            });
        }

        let mut required_tools: Vec<ToolId> = graph
            .nodes
            .iter()
            .filter_map(|node| match &node.op {
                CognitiveOp::Observe { source } => Some(source.clone()),
                CognitiveOp::Act { action } => Some(action.clone()),
                _ => None,
            })
            .collect();
        required_tools.sort();
        required_tools.dedup();

        let mut evaluation_criteria: Vec<CriterionId> = scan
            .comparison_requirements
            .iter()
            .map(|c| c.criterion.clone())
            .collect();
        evaluation_criteria.sort();
        evaluation_criteria.dedup();

        let mut frames = episode.active_frames.clone();
        frames.sort();
        frames.dedup();
        let mut doctrines = episode.active_doctrines.clone();
        doctrines.sort();
        doctrines.dedup();
        let mut techniques = episode.active_techniques.clone();
        techniques.sort();
        techniques.dedup();

        let mut stopping = vec![
            StoppingCondition::RemainingUncertaintyBelow(uncertainty_target(episode.tier)),
            StoppingCondition::ExpectedValueBelow(EXPECTED_VALUE_FLOOR),
            StoppingCondition::BudgetExhausted,
        ];
        if episode.tier >= Tier::T3 {
            stopping.push(StoppingCondition::RequiredConfidenceMet(confidence_target(
                episode.tier,
            )));
        }
        if irreversible {
            stopping.push(StoppingCondition::IrreversibleActionReached);
        }

        let mut program = CognitiveProgram {
            id: Ulid::nil(),
            episode_id: episode.id,
            tier: episode.tier,
            graph,
            steps,
            // A frame is what a problem is compared against, so the reference
            // classes are the active frames; the two fields are kept separate
            // because a caller may later supply reference classes directly.
            frames: frames.clone(),
            doctrines,
            techniques,
            reference_classes: frames,
            assumptions_to_verify: assumptions_checked,
            comparisons: scan.comparison_requirements.clone(),
            candidate_actions: effective_candidates,
            evaluation_criteria,
            required_tools,
            stopping,
        };
        program.id = program_id_from(&program.canonical());
        program.validate()?;
        Ok(program)
    }
}

/// The content-derived program id: the first 128 bits of the canonical content's
/// sha256, rendered as a ULID.
///
/// Deterministic rather than time-based so a replay recompiles *the same program*,
/// not merely an equivalent one. The episode id is part of the hashed content, so
/// two episodes with identical bodies still get distinct ids.
pub fn program_id_from(canonical: &str) -> ProgramId {
    let hex = mm_core::content_hash(canonical.as_bytes());
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let pair = hex.get(index * 2..index * 2 + 2).unwrap_or("00");
        *byte = u8::from_str_radix(pair, 16).unwrap_or(0);
    }
    Ulid::from_bytes(bytes)
}

/// How much an operation of this class can move the provisional model.
///
/// This is *not* the value an operation is ranked by; it is how far the loop's
/// model update moves when the operation runs. The two are separate on purpose:
/// a free conceptual operation is worth running first — its value per unit cost is
/// maximal — but it settles much less than an observation or a verification, so it
/// cannot substitute for one. Without this weight a cheap inversion and a
/// verified measurement would reduce uncertainty by the same amount, and the
/// controller would happily decide on reflection alone.
pub fn evidence_weight(class: OpClass) -> f64 {
    match class {
        // Operations that look at something outside the model.
        OpClass::Observe
        | OpClass::Verify
        | OpClass::Simulate
        | OpClass::Search
        | OpClass::CheckAssumptions
        | OpClass::CheckContradictions
        | OpClass::CheckCausality
        | OpClass::Critique
        | OpClass::RedTeam
        | OpClass::Compare
        | OpClass::Analogize
        | OpClass::Measure => 1.0,
        // Operations that reorganise the model without adding evidence.
        OpClass::Recall
        | OpClass::Classify
        | OpClass::Clarify
        | OpClass::SearchPrecedent
        | OpClass::Synthesize
        | OpClass::Predict
        | OpClass::Refine
        | OpClass::Decompose => 0.5,
        // Pure reflection: worth doing first, worth very little on its own.
        OpClass::Invert
        | OpClass::Simplify
        | OpClass::CheckConstraints
        | OpClass::Learn
        | OpClass::Decide
        | OpClass::Act => 0.25,
    }
}

/// How many assumptions a tier is willing to verify.
fn assumptions_to_verify(tier: Tier) -> usize {
    match tier {
        Tier::T0 | Tier::T1 => 0,
        Tier::T2 | Tier::T3 => 1,
        Tier::T4 => 2,
        Tier::T5 => 3,
    }
}

/// The remaining-uncertainty target a tier is trying to reach.
pub fn uncertainty_target(tier: Tier) -> f64 {
    match tier {
        Tier::T0 => 0.60,
        Tier::T1 => 0.50,
        Tier::T2 => 0.40,
        Tier::T3 => 0.30,
        Tier::T4 => 0.20,
        Tier::T5 => 0.10,
    }
}

/// The confidence a tier demands before it will stop on confidence.
pub fn confidence_target(tier: Tier) -> f64 {
    match tier {
        Tier::T0 | Tier::T1 | Tier::T2 => 0.5,
        Tier::T3 => 0.6,
        Tier::T4 => 0.8,
        Tier::T5 => 0.9,
    }
}

/// What an operation is worth, as fixed arithmetic over the scan's own beliefs.
///
/// Every table below is constant, and every caller-supplied number is clamped into
/// `[0,1]`, so two runs of the same episode produce byte-identical scores. The
/// tables are *not* tuned by a model: they are the phase's stated preference —
/// verification and adversarial critique are worth more than recall, and a cheap
/// operation is worth more than an expensive one that does the same thing.
pub fn value_for(
    op: &CognitiveOp,
    episode: &crate::episode::CognitiveEpisode,
    scan: &ScanResult,
    policy: &ComputePolicy,
) -> Result<OperationValue> {
    let class = op.class();
    let eer = base_error_reduction(class);
    let importance = clamp_unit(
        scan.stakes
            + if matches!(class, OpClass::Decide | OpClass::Act | OpClass::Synthesize) {
                0.1
            } else {
                0.0
            },
    );
    let p_change = clamp_unit(
        base_change_probability(class) * scan.verification_value + 0.5 * scan.mean_uncertainty(),
    );
    let cost = policy.price_of(class_to_cost(op));
    // The episode's own capping is part of the score, so a context that says
    // "nothing is novel and nothing is at stake" lowers every operation's value.
    let eer = clamp_unit(eer * (0.5 + 0.5 * episode.context.novelty.max(scan.stakes)));
    OperationValue::new(eer, importance, p_change, cost)
}

/// The cost band an operation is priced at, through its class.
fn class_to_cost(op: &CognitiveOp) -> CostClass {
    op.cost_class()
}

/// How much error an operation of this class is expected to remove.
fn base_error_reduction(class: OpClass) -> f64 {
    match class {
        OpClass::Observe => 0.7,
        OpClass::Recall => 0.3,
        OpClass::Clarify => 0.6,
        OpClass::Classify => 0.4,
        OpClass::Decompose => 0.5,
        OpClass::Compare => 0.6,
        OpClass::Analogize => 0.5,
        OpClass::SearchPrecedent => 0.6,
        OpClass::Invert => 0.5,
        OpClass::Predict => 0.4,
        OpClass::Simulate => 0.8,
        OpClass::Verify => 0.9,
        OpClass::Critique => 0.7,
        OpClass::Refine => 0.5,
        OpClass::RedTeam => 0.7,
        OpClass::Search => 0.8,
        OpClass::CheckConstraints => 0.4,
        OpClass::CheckAssumptions => 0.7,
        OpClass::CheckContradictions => 0.8,
        OpClass::CheckCausality => 0.6,
        OpClass::Simplify => 0.3,
        OpClass::Synthesize => 0.4,
        OpClass::Decide => 0.9,
        OpClass::Act => 0.9,
        OpClass::Measure => 0.5,
        OpClass::Learn => 0.2,
    }
}

/// How likely an operation of this class is to change the answer, before the
/// scan's own `verification_value` modulates it.
fn base_change_probability(class: OpClass) -> f64 {
    match class {
        OpClass::Observe => 0.9,
        OpClass::Recall => 0.3,
        OpClass::Clarify => 0.8,
        OpClass::Classify => 0.4,
        OpClass::Decompose => 0.5,
        OpClass::Compare => 0.7,
        OpClass::Analogize => 0.6,
        OpClass::SearchPrecedent => 0.7,
        OpClass::Invert => 0.6,
        OpClass::Predict => 0.5,
        OpClass::Simulate => 0.9,
        OpClass::Verify => 1.0,
        OpClass::Critique => 0.8,
        OpClass::Refine => 0.6,
        OpClass::RedTeam => 0.8,
        OpClass::Search => 0.9,
        OpClass::CheckConstraints => 0.5,
        OpClass::CheckAssumptions => 0.8,
        OpClass::CheckContradictions => 0.9,
        OpClass::CheckCausality => 0.7,
        OpClass::Simplify => 0.3,
        OpClass::Synthesize => 0.5,
        OpClass::Decide => 1.0,
        OpClass::Act => 1.0,
        OpClass::Measure => 0.6,
        OpClass::Learn => 0.2,
    }
}

/// Clamp a caller-supplied factor into `[0,1]`, mapping a non-finite value to `0`.
fn clamp_unit(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::Context;
    use crate::scan::{ComparisonSpec, FailureMode, ScanIssue, Uncertainty};
    use crate::tier::Tier;
    use proptest::prelude::*;

    fn ulid(n: u128) -> mm_core::Ulid {
        mm_core::Ulid::from_parts(1_700_000_000_000, n)
    }

    /// An episode with something to recall, check, compare and decide, so every
    /// kind of operation the compiler may add has a reason to exist.
    fn episode() -> crate::episode::CognitiveEpisode {
        let mut episode = crate::episode::CognitiveEpisode::new(
            ulid(1),
            ulid(2),
            Context::new("decide how to recover a sharded store that lost a replica")
                .with_novelty(0.5),
            CognitiveBudget::from_spec(16, 8, 2.0, 60_000).unwrap(),
        )
        .unwrap()
        .with_claims(vec![ulid(3), ulid(4)])
        .with_assumptions(vec![ulid(5), ulid(6)])
        .with_candidates(vec!["restore from the backup".into(), "fail over".into()])
        .with_frames(vec![
            "https://metamind.dev/library/frame/incident_response".into()
        ])
        .with_techniques(vec![
            "https://metamind.dev/library/technique/narrow-before-acting".into(),
        ]);
        // A pinned tier is a floor: the selector may still raise it.
        episode.tier = Tier::T4;
        episode
    }

    /// One broad scan whose four numbers the caller chooses.
    fn scan(
        stakes: f64,
        irreversibility: f64,
        verification_value: f64,
        uncertainty: f64,
    ) -> ScanResult {
        ScanResult {
            issues: vec![ScanIssue {
                kind: "missing_evidence".into(),
                materiality: 0.8,
                target: "fail over".into(),
            }],
            uncertainties: vec![Uncertainty {
                id: "u1".into(),
                description: "does the window hold".into(),
                magnitude: uncertainty,
            }],
            comparison_requirements: vec![ComparisonSpec {
                subject: "restore from the backup".into(),
                versus: "fail over".into(),
                criterion: "data loss".into(),
            }],
            failure_modes: vec![FailureMode {
                mode: "partial restore".into(),
                likelihood: 0.2,
                impact: 0.8,
            }],
            simpler_alternatives: vec!["ask the on-call".into()],
            stakes,
            irreversibility,
            verification_value,
        }
    }

    /// Compile, requiring success: a valid episode and scan always compile, and a
    /// refusal here would be the bug.
    fn compile(episode: &crate::episode::CognitiveEpisode, scan: &ScanResult) -> CognitiveProgram {
        DefaultCompiler::new()
            .compile(episode, scan, &episode.budget)
            .expect("a valid episode and scan must compile")
    }

    #[test]
    fn the_same_inputs_compile_to_the_same_program() {
        let episode = episode();
        let scan = scan(0.8, 0.4, 0.7, 0.6);
        let first = compile(&episode, &scan);
        let second = compile(&episode, &scan);

        assert_eq!(first.id, second.id, "the id is derived from the content");
        assert_eq!(first.content_hash(), second.content_hash());
        assert_eq!(first.canonical(), second.canonical());
        assert_eq!(first.episode_id, episode.id);
        first.validate().expect("a compiled program validates");
        assert!(first.node_count() >= 1, "a program holds at least one op");

        // Contiguous from 1, which is what makes the step list replayable.
        let orders: Vec<u8> = first.steps.iter().map(|step| step.order).collect();
        let expected: Vec<u8> = (1..=first.steps.len() as u8).collect();
        assert_eq!(orders, expected);
    }

    #[test]
    fn a_program_never_outgrows_what_the_tier_and_budget_allow() {
        let episode = episode();
        let scan = scan(0.95, 0.6, 0.9, 0.85);
        let program = compile(&episode, &scan);
        let cap = episode
            .tier
            .policy()
            .max_ops
            .min(episode.budget.max_ops)
            .max(1) as usize;
        assert!(
            program.node_count() <= cap,
            "a compiled program is the tighter of the tier's allowance and the budget: \
             {} nodes > {cap}",
            program.node_count()
        );

        // The same episode with room for one operation compiles a *shorter*
        // program rather than refusing: the cap bounds the program, it does not
        // veto it. The root recall and the final decide are unconditional — a
        // program with neither is not a program — so those two survive any cap and
        // every optional operation is what the budget buys.
        let mut cramped = episode;
        cramped.budget = CognitiveBudget::from_spec(1, 1, 2.0, 60_000).unwrap();
        let small = compile(&cramped, &scan);
        assert!(
            small.node_count() < program.node_count(),
            "a one-operation budget must buy a smaller program: {} vs {}",
            small.node_count(),
            program.node_count()
        );
        assert!(
            small.node_count() >= 2,
            "the recall-and-decide spine survives"
        );
        small.validate().expect("a cramped program still validates");
    }

    proptest! {
        /// Whatever the scan says, it says it twice: the program, its id and every
        /// score are a function of the inputs alone. That is what makes
        /// `episode replay --compare` a gate rather than a habit.
        #[test]
        fn compile_is_deterministic_whatever_the_scan_says(
            stakes in 1u32..1000,
            irreversibility in 1u32..1000,
            verification_value in 1u32..1000,
            uncertainty in 1u32..1000,
            novelty in 1u32..1000,
        ) {
            let unit = |n: u32| f64::from(n) / 1000.0;
            let mut episode = episode();
            episode.context.novelty = unit(novelty);
            let scan = scan(
                unit(stakes),
                unit(irreversibility),
                unit(verification_value),
                unit(uncertainty),
            );

            let first = compile(&episode, &scan);
            let second = compile(&episode, &scan);
            prop_assert_eq!(first.id, second.id);
            prop_assert_eq!(&first.canonical(), &second.canonical());
            prop_assert_eq!(first.content_hash(), second.content_hash());
            prop_assert!(first.validate().is_ok());
            prop_assert!(first.node_count() >= 1);
        }
    }
}
