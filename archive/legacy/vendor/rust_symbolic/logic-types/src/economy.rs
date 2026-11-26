//! The information–cost economy (completion plan §12; design §16–§18, §45):
//! the common frame in which information acquisition, theorem proving,
//! simulation, search, measurement, LLM calls, solver options, and human
//! review compete under one explicit objective.
//!
//! Three objectives are defined, with their weights carried by the objective
//! itself — never implicit:
//!
//! - `IPC(C) = ExpectedInformationGain(C) / ExpectedComputeCost(C)`
//!   ([`EconomyObjective::InformationPerCompute`], design §17);
//! - `Utility(C) = InformationValue(C) − λ·Cost(C)`
//!   ([`EconomyObjective::NetUtility`], design §16);
//! - `Score(C) = (ExpectedDecisionValue(C) + β·ExpectedInformationGain(C)) /
//!   ResourceCost(C)` ([`EconomyObjective::ValuePerResource`], design §18).
//!
//! Every cost term routes through the shared [`ResourceVector`] (Phase 2),
//! and its scalarization is a declared [`CostProjection`]: a dimension present
//! in a candidate's vector but absent from the projection is a typed error,
//! never a silent collapse. Cost categories stay distinct (completion plan
//! §2.1) — the projection names exactly which dimensions the decision prices
//! and at what weight.
//!
//! Honesty rules enforced here (§2.3, §2.5): a candidate missing the quantity
//! an objective needs is rejected with a typed-gap reason, not fabricated;
//! negative net utility rejects the candidate ("do nothing" beats paying for
//! it); ties break deterministically (objective value desc, then candidate id
//! asc), so identical inputs yield identical records.

use std::collections::BTreeMap;

use rdf_codec::{CodecError, FromRdf, NamedNode, Prim, RdfContext, RdfReader, ToRdf};

use crate::resource::{ResourceDimension, ResourceError, ResourceVector};

/// The kinds of action that compete in one economy (design §16's comparison
/// list plus the solver/LLM/human surfaces of §1.5). The kind is descriptive
/// metadata on a decision; the objective never branches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EconomyActionKind {
    TheoremProving,
    Simulation,
    Search,
    Measurement,
    SolverCall,
    InformationAcquisition,
    LlmInference,
    HumanReview,
    Intervention,
}

impl EconomyActionKind {
    pub const ALL: &'static [Self] = &[
        Self::TheoremProving,
        Self::Simulation,
        Self::Search,
        Self::Measurement,
        Self::SolverCall,
        Self::InformationAcquisition,
        Self::LlmInference,
        Self::HumanReview,
        Self::Intervention,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TheoremProving => "theoremProving",
            Self::Simulation => "simulation",
            Self::Search => "search",
            Self::Measurement => "measurement",
            Self::SolverCall => "solverCall",
            Self::InformationAcquisition => "informationAcquisition",
            Self::LlmInference => "llmInference",
            Self::HumanReview => "humanReview",
            Self::Intervention => "intervention",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(name))
    }
}

/// The decision objective with its weights explicit (completion plan §12.1).
///
/// Validation: weights must be finite and non-negative. A negative λ would
/// reward expensive actions over cheap ones and a negative β would punish
/// information; both are configuration errors, typed as
/// [`EconomyError::InvalidWeight`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EconomyObjective {
    /// `IPC(C) = ExpectedInformationGain(C) / ExpectedComputeCost(C)`
    /// (design §17). Requires `eig`.
    InformationPerCompute,
    /// `Utility(C) = InformationValue(C) − λ·Cost(C)` (design §16). Requires
    /// `iv`. Candidates whose utility is non-positive reject: doing nothing
    /// has zero net value, so paying for negative value is never chosen.
    NetUtility {
        /// λ: the cost weight.
        lambda: f64,
    },
    /// `Score(C) = (ExpectedDecisionValue(C) + β·ExpectedInformationGain(C)) /
    /// Cost(C)` (design §18). Requires `edv` and `eig`.
    ValuePerResource {
        /// β: how strongly expected information counts beside decision value.
        beta: f64,
    },
}

impl EconomyObjective {
    pub fn validate(&self) -> Result<(), EconomyError> {
        // The synthetic dimension names the offending weight in the error.
        let check = |weight: f64| -> Result<(), EconomyError> {
            if !weight.is_finite() || weight < 0.0 {
                Err(EconomyError::InvalidWeight {
                    dimension: ResourceDimension::ComputeUnits,
                    weight,
                })
            } else {
                Ok(())
            }
        };
        match self {
            Self::InformationPerCompute => Ok(()),
            Self::NetUtility { lambda } => check(*lambda),
            Self::ValuePerResource { beta } => check(*beta),
        }
    }

    /// The stable serialized name of this objective form.
    pub fn name(&self) -> &'static str {
        match self {
            Self::InformationPerCompute => "informationPerCompute",
            Self::NetUtility { .. } => "netUtility",
            Self::ValuePerResource { .. } => "valuePerResource",
        }
    }

    /// Human-readable rendering of the objective and its weights — what
    /// provenance records alongside every decision (completion plan §12.4).
    pub fn describe(&self) -> String {
        match self {
            Self::InformationPerCompute => "IPC(C) = EIG(C) / ExpectedComputeCost(C)".to_string(),
            Self::NetUtility { lambda } => format!("Utility(C) = IV(C) − {lambda}·Cost(C)"),
            Self::ValuePerResource { beta } => {
                format!("Score(C) = (EDV(C) + {beta}·EIG(C)) / ResourceCost(C)")
            }
        }
    }
}

/// The declared scalarization of a [`ResourceVector`] cost into the single
/// number an objective divides or subtracts (completion plan §12.2). Each
/// priced dimension carries an explicit weight; a dimension present in some
/// candidate's vector but absent here is a typed [`EconomyError::MissingWeight`]
/// — costs are never silently collapsed, and distinct categories (compute vs
/// energy vs money) only mix when the caller says so.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CostProjection {
    weights: BTreeMap<ResourceDimension, f64>,
}

impl CostProjection {
    pub fn new() -> Self {
        Self::default()
    }

    /// A projection pricing exactly one dimension at weight 1.
    pub fn single(dimension: ResourceDimension) -> Self {
        let mut projection = Self::new();
        projection.weights.insert(dimension, 1.0);
        projection
    }

    /// Declare (or replace) the weight of one dimension. Non-finite or
    /// negative weights are typed errors.
    pub fn with_weight(
        mut self,
        dimension: ResourceDimension,
        weight: f64,
    ) -> Result<Self, EconomyError> {
        if !weight.is_finite() || weight < 0.0 {
            return Err(EconomyError::InvalidWeight { dimension, weight });
        }
        self.weights.insert(dimension, weight);
        Ok(self)
    }

    /// The declared weight of a dimension, if priced.
    pub fn weight(&self, dimension: ResourceDimension) -> Option<f64> {
        self.weights.get(&dimension).copied()
    }

    /// Every priced dimension, in canonical order.
    pub fn dimensions(&self) -> impl Iterator<Item = ResourceDimension> + '_ {
        self.weights.keys().copied()
    }

    /// Project a candidate's resource vector to the objective's cost scalar.
    pub fn project(&self, vector: &ResourceVector) -> Result<f64, EconomyError> {
        let mut total = 0.0;
        for (dimension, value) in &vector.values {
            let Some(weight) = self.weights.get(dimension) else {
                return Err(EconomyError::MissingWeight {
                    dimension: *dimension,
                });
            };
            total += value.value * weight;
        }
        if !total.is_finite() {
            return Err(EconomyError::Overflow);
        }
        Ok(total)
    }
}

/// One competitor in the economy: any declarable action with its expected
/// information/value quantities and its full resource-vector cost.
///
/// Quantities are optional and honest: `None` means "not declared", and an
/// objective that needs an undeclared quantity produces a typed gap — never
/// a fabricated number (completion plan §2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct EconomyCandidate {
    pub id: String,
    pub kind: EconomyActionKind,
    /// ExpectedInformationGain(C) (bits).
    pub eig: Option<f64>,
    /// InformationValue(C).
    pub iv: Option<f64>,
    /// ExpectedDecisionValue(C).
    pub edv: Option<f64>,
    /// The full multidimensional cost (the Phase 2 shared vector); sparse
    /// entries mean zero demand.
    pub cost: ResourceVector,
}

impl EconomyCandidate {
    pub fn new(id: impl Into<String>, kind: EconomyActionKind, cost: ResourceVector) -> Self {
        Self {
            id: id.into(),
            kind,
            eig: None,
            iv: None,
            edv: None,
            cost,
        }
    }

    pub fn with_eig(mut self, eig: f64) -> Self {
        self.eig = Some(eig);
        self
    }

    pub fn with_iv(mut self, iv: f64) -> Self {
        self.iv = Some(iv);
        self
    }

    pub fn with_edv(mut self, edv: f64) -> Self {
        self.edv = Some(edv);
        self
    }

    fn validate(&self) -> Result<(), EconomyError> {
        let check = |quantity: &'static str, value: Option<f64>| match value {
            Some(v) if v.is_finite() => Ok(()),
            Some(v) => Err(EconomyError::InvalidQuantity {
                candidate: self.id.clone(),
                quantity,
                value: v,
            }),
            None => Ok(()),
        };
        check("expectedInformationGain", self.eig)?;
        check("informationValue", self.iv)?;
        check("expectedDecisionValue", self.edv)?;
        self.cost.validate().map_err(EconomyError::Resource)
    }
}

/// One candidate's outcome in an economy decision: its score, the exact cost
/// scalar it was scored against, and why it holds its rank or was rejected —
/// the per-candidate decomposition provenance records (§12.4).
#[derive(Debug, Clone, PartialEq)]
pub struct EconomyDecision {
    pub candidate: String,
    pub kind: EconomyActionKind,
    /// The objective value (`NaN` never; rejected candidates carry the last
    /// computable value or 0 when none existed).
    pub objective_value: f64,
    /// The projected cost scalar used by the objective.
    pub cost_scalar: f64,
    /// 0-based position in the record's total order (0 = best). Rejections
    /// trail every accepted candidate.
    pub rank: usize,
    pub selected: bool,
    /// True when the candidate may not run (typed gap, non-positive net
    /// value, or non-positive projected cost under a ratio objective).
    pub rejected: bool,
    /// Why this candidate holds its rank / was rejected / is a typed gap.
    pub reason: String,
    // ── decomposition echo (what the score was built from) ──
    pub eig: Option<f64>,
    pub iv: Option<f64>,
    pub edv: Option<f64>,
    pub cost: ResourceVector,
}

/// The full decision record for one economy evaluation (completion plan
/// §12.4): the objective and its weights, the declared cost projection, and
/// every candidate with its decomposition. Nothing here executes anything.
#[derive(Debug, Clone, PartialEq)]
pub struct EconomyDecisionRecord {
    pub objective: EconomyObjective,
    pub projection: CostProjection,
    /// Every evaluated candidate, best-first; rejections trail.
    pub decisions: Vec<EconomyDecision>,
    /// The selected candidate; `None` means every candidate was rejected and
    /// doing nothing wins.
    pub selected: Option<String>,
}

impl EconomyDecisionRecord {
    pub fn is_all_rejected(&self) -> bool {
        self.selected.is_none()
    }
}

/// Typed economy failures (§2.3): configuration errors are hard errors;
/// per-candidate gaps become rejections on the record instead.
#[derive(Debug, Clone, PartialEq)]
pub enum EconomyError {
    InvalidWeight {
        dimension: ResourceDimension,
        weight: f64,
    },
    MissingWeight {
        dimension: ResourceDimension,
    },
    MissingQuantity {
        candidate: String,
        quantity: &'static str,
    },
    InvalidQuantity {
        candidate: String,
        quantity: &'static str,
        value: f64,
    },
    NonPositiveCost {
        candidate: String,
        cost: f64,
    },
    Overflow,
    Resource(ResourceError),
}

impl std::fmt::Display for EconomyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWeight { dimension, weight } => {
                write!(
                    f,
                    "invalid economy weight {weight} for {}",
                    dimension.as_str()
                )
            }
            Self::MissingWeight { dimension } => {
                write!(
                    f,
                    "cost projection does not price dimension {} present in some candidate's vector",
                    dimension.as_str()
                )
            }
            Self::MissingQuantity {
                candidate,
                quantity,
            } => {
                write!(f, "candidate `{candidate}` does not declare {quantity}; the objective requires it")
            }
            Self::InvalidQuantity {
                candidate,
                quantity,
                value,
            } => {
                write!(
                    f,
                    "candidate `{candidate}` declares non-finite {quantity}: {value}"
                )
            }
            Self::NonPositiveCost { candidate, cost } => {
                write!(f, "candidate `{candidate}` has non-positive projected cost {cost} under a ratio objective")
            }
            Self::Overflow => write!(f, "economy arithmetic overflow"),
            Self::Resource(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for EconomyError {}

impl From<ResourceError> for EconomyError {
    fn from(error: ResourceError) -> Self {
        Self::Resource(error)
    }
}

/// Score one candidate under the objective (strict form): missing quantities
/// and non-positive ratio costs are errors, so callers that must have an
/// answer get a typed gap instead of a silent rejection.
pub fn score_candidate(
    candidate: &EconomyCandidate,
    objective: &EconomyObjective,
    projection: &CostProjection,
) -> Result<f64, EconomyError> {
    candidate.validate()?;
    objective.validate()?;
    let cost_scalar = projection.project(&candidate.cost)?;
    score_with_cost(candidate, objective, cost_scalar).map(|(value, _)| value)
}

/// Score plus the cost scalar actually used.
fn score_with_cost(
    candidate: &EconomyCandidate,
    objective: &EconomyObjective,
    cost_scalar: f64,
) -> Result<(f64, f64), EconomyError> {
    let require = |quantity: &'static str, value: Option<f64>| {
        value.ok_or(EconomyError::MissingQuantity {
            candidate: candidate.id.clone(),
            quantity,
        })
    };
    match objective {
        EconomyObjective::InformationPerCompute => {
            if cost_scalar <= 0.0 {
                return Err(EconomyError::NonPositiveCost {
                    candidate: candidate.id.clone(),
                    cost: cost_scalar,
                });
            }
            let eig = require("expectedInformationGain", candidate.eig)?;
            Ok((eig / cost_scalar, cost_scalar))
        }
        EconomyObjective::NetUtility { lambda } => {
            let iv = require("informationValue", candidate.iv)?;
            Ok((iv - lambda * cost_scalar, cost_scalar))
        }
        EconomyObjective::ValuePerResource { beta } => {
            if cost_scalar <= 0.0 {
                return Err(EconomyError::NonPositiveCost {
                    candidate: candidate.id.clone(),
                    cost: cost_scalar,
                });
            }
            let edv = require("expectedDecisionValue", candidate.edv)?;
            let eig = require("expectedInformationGain", candidate.eig)?;
            Ok(((edv + beta * eig) / cost_scalar, cost_scalar))
        }
    }
}

/// Rank one pool of heterogeneous competitors under one objective: the
/// acceptance test "an info action and a solver call are ranked together in
/// one decision record" is exactly what this returns.
///
/// Per-candidate problems (undeclared quantities, zero-cost ratios,
/// non-positive net value) become rejections carrying typed reasons; the
/// record still ranks every survivor. Decision-level problems (unpriced
/// dimensions, invalid weights) are hard [`EconomyError`]s.
/// One candidate's score outcome: the objective value and the projected cost
/// scalar it was scored against.
type Scored = Result<(f64, f64), EconomyError>;

/// Rank one pool of heterogeneous competitors under one objective: the
/// acceptance test "an info action and a solver call are ranked together in
/// one decision record" is exactly what this returns.
///
/// Per-candidate problems (undeclared quantities, zero-cost ratios,
/// non-positive net value) become rejections carrying typed reasons; the
/// record still ranks every survivor. Decision-level problems (unpriced
/// dimensions, invalid weights) are hard [`EconomyError`]s.
pub fn decide_economy(
    candidates: &[EconomyCandidate],
    objective: &EconomyObjective,
    projection: &CostProjection,
) -> Result<EconomyDecisionRecord, EconomyError> {
    objective.validate()?;
    let mut evaluations: Vec<(EconomyCandidate, Scored)> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        candidate.validate()?;
        evaluations.push((
            candidate.clone(),
            score_with_cost(candidate, objective, projection.project(&candidate.cost)?),
        ));
    }

    // Accepted candidates ranked by objective value desc, ties by id asc;
    // rejections trail in declaration order (mirrors Phase 7 selection).
    let mut accepted: Vec<usize> = (0..evaluations.len())
        .filter(|i| matches!(evaluations[*i].1, Ok((value, _)) if value > 0.0))
        .collect();
    accepted.sort_by(|a, b| {
        let (a_value, _) = evaluations[*a].1.as_ref().unwrap_or(&(0.0, 0.0));
        let (b_value, _) = evaluations[*b].1.as_ref().unwrap_or(&(0.0, 0.0));
        b_value
            .total_cmp(a_value)
            .then_with(|| evaluations[*a].0.id.cmp(&evaluations[*b].0.id))
    });
    let mut decisions = Vec::with_capacity(evaluations.len());
    let mut selected = None;
    for index in &accepted {
        let (candidate, scored) = &evaluations[*index];
        let (objective_value, cost_scalar) = *scored.as_ref().unwrap_or(&(0.0, 0.0));
        let is_first = selected.is_none();
        if is_first {
            selected = Some(candidate.id.clone());
        }
        decisions.push(EconomyDecision {
            candidate: candidate.id.clone(),
            kind: candidate.kind,
            objective_value,
            cost_scalar,
            rank: decisions.len(),
            selected: is_first,
            rejected: false,
            reason: format!(
                "accepted: {} leads the {} ranking",
                candidate.id,
                objective.name()
            ),
            eig: candidate.eig,
            iv: candidate.iv,
            edv: candidate.edv,
            cost: candidate.cost.clone(),
        });
    }
    for (candidate, outcome) in &evaluations {
        let (objective_value, cost_scalar, reason) = match outcome {
            Ok((value, _)) if *value > 0.0 => continue,
            Ok((value, cost)) => (
                *value,
                *cost,
                "rejected: non-positive objective value; doing nothing has zero net value"
                    .to_string(),
            ),
            Err(EconomyError::MissingQuantity { quantity, .. }) => (
                0.0,
                projection.project(&candidate.cost).unwrap_or_default(),
                format!("rejected: typed gap — {quantity} not declared for this objective"),
            ),
            Err(EconomyError::NonPositiveCost { cost, .. }) => (
                0.0,
                *cost,
                "rejected: non-positive projected cost cannot be divided".to_string(),
            ),
            Err(other) => {
                return Err(other.clone());
            }
        };
        decisions.push(EconomyDecision {
            candidate: candidate.id.clone(),
            kind: candidate.kind,
            objective_value,
            cost_scalar,
            rank: decisions.len(),
            selected: false,
            rejected: true,
            reason,
            eig: candidate.eig,
            iv: candidate.iv,
            edv: candidate.edv,
            cost: candidate.cost.clone(),
        });
    }

    Ok(EconomyDecisionRecord {
        objective: *objective,
        projection: projection.clone(),
        decisions,
        selected,
    })
}

/// The design-§45 Pareto front over (objective value ↑, priced cost ↓):
/// candidates no other candidate dominates. Dominance keeps every priced
/// dimension separate — collapsing them into one scalar is exactly what the
/// design warns against — while the objective supplies the value axis.
/// Returns the surviving ids in input order.
pub fn pareto_front(
    candidates: &[EconomyCandidate],
    objective: &EconomyObjective,
    projection: &CostProjection,
) -> Result<Vec<String>, EconomyError> {
    objective.validate()?;
    let mut scored = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        candidate.validate()?;
        let cost = projection.project(&candidate.cost)?;
        let value = score_with_cost(candidate, objective, cost)
            .map(|(value, _)| value)
            .unwrap_or(f64::NEG_INFINITY);
        scored.push((candidate, value, cost));
    }
    let mut front = Vec::new();
    for (i, (candidate, value, cost)) in scored.iter().enumerate() {
        let dominated = scored
            .iter()
            .enumerate()
            .any(|(j, (_, other_value, other_cost))| {
                j != i
                    && other_value >= value
                    && other_cost <= cost
                    && (other_value > value || other_cost < cost)
            });
        if !dominated {
            front.push(candidate.id.clone());
        }
    }
    Ok(front)
}

// ── RDF codec ───────────────────────────────────────────────────────────────

const ECONOMY_NS: &str = "https://example.org/ns/economy#";

fn economy_prop(class: &str, field: &str) -> String {
    format!("{ECONOMY_NS}{class}/{field}")
}

/// Emit helpers pinned to the canonical shared-economy namespace (the same
/// rule as the shared-resource classes: these types are embedded in many
/// subsystem documents, and the reader is always the canonical namespace).
fn emit_eco_type(ctx: &mut RdfContext, node: &rdf_codec::NamedNode, class: &str) {
    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let class_node = rdf_codec::NamedNode::new(format!("{ECONOMY_NS}{class}")).expect("class IRI");
    let type_pred = rdf_codec::NamedNode::new(RDF_TYPE).expect("constant IRI");
    ctx.graph.insert(&rdf_codec::Triple::new(
        node.clone(),
        type_pred,
        rdf_codec::Term::from(class_node),
    ));
}

fn emit_eco_literal(
    ctx: &mut RdfContext,
    node: &rdf_codec::NamedNode,
    class: &str,
    prop: &str,
    value: rdf_codec::Literal,
) {
    let p = rdf_codec::NamedNode::new(economy_prop(class, prop)).expect("property IRI");
    ctx.graph.insert(&rdf_codec::Triple::new(
        node.clone(),
        p,
        rdf_codec::Term::from(value),
    ));
}

fn emit_eco_object(
    ctx: &mut RdfContext,
    node: &rdf_codec::NamedNode,
    class: &str,
    prop: &str,
    obj: &rdf_codec::NamedNode,
) {
    let p = rdf_codec::NamedNode::new(economy_prop(class, prop)).expect("property IRI");
    ctx.graph.insert(&rdf_codec::Triple::new(
        node.clone(),
        p,
        rdf_codec::Term::from(obj.clone()),
    ));
}

fn open_economy<'a>(graph: &'a rdf_codec::Graph, node: &rdf_codec::NamedNode) -> RdfReader<'a> {
    static NS: std::sync::OnceLock<rdf_codec::Namespace> = std::sync::OnceLock::new();
    RdfReader::new(
        graph,
        node,
        NS.get_or_init(|| rdf_codec::Namespace::new(ECONOMY_NS)),
    )
}

fn eco_text_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<String, CodecError> {
    match Prim::from_literal(&reader.literal(&economy_prop(class, field))?)? {
        Prim::Text(value) => Ok(value),
        other => Err(CodecError::Decode(format!(
            "{class}/{field} must be text, got {other:?}"
        ))),
    }
}

fn eco_float_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<f64, CodecError> {
    match Prim::from_literal(&reader.literal(&economy_prop(class, field))?)? {
        Prim::Float(value) => Ok(value),
        other => Err(CodecError::Decode(format!(
            "{class}/{field} must be float, got {other:?}"
        ))),
    }
}

fn eco_optional_float_field(
    reader: &RdfReader<'_>,
    class: &str,
    field: &str,
) -> Result<Option<f64>, CodecError> {
    match reader.optional_literal(&economy_prop(class, field))? {
        None => Ok(None),
        Some(literal) => match Prim::from_literal(&literal)? {
            Prim::Float(value) => Ok(Some(value)),
            other => Err(CodecError::Decode(format!(
                "{class}/{field} must be float, got {other:?}"
            ))),
        },
    }
}

impl ToRdf for EconomyObjective {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<NamedNode, CodecError> {
        let node = ctx.instance();
        emit_eco_type(ctx, &node, "EconomyObjective");
        emit_eco_literal(
            ctx,
            &node,
            "EconomyObjective",
            "variant",
            Prim::Text(self.name().to_string()).to_literal()?,
        );
        match self {
            Self::InformationPerCompute => {}
            Self::NetUtility { lambda } => emit_eco_literal(
                ctx,
                &node,
                "EconomyObjective",
                "lambda",
                Prim::Float(*lambda).to_literal()?,
            ),
            Self::ValuePerResource { beta } => emit_eco_literal(
                ctx,
                &node,
                "EconomyObjective",
                "beta",
                Prim::Float(*beta).to_literal()?,
            ),
        }
        Ok(node)
    }
}

impl FromRdf for EconomyObjective {
    fn from_rdf(node: &NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_economy(graph, node);
        reader.expect_type("EconomyObjective")?;
        let variant = eco_text_field(&reader, "EconomyObjective", "variant")?;
        let objective = match variant.as_str() {
            "informationPerCompute" => Self::InformationPerCompute,
            "netUtility" => Self::NetUtility {
                lambda: eco_float_field(&reader, "EconomyObjective", "lambda")?,
            },
            "valuePerResource" => Self::ValuePerResource {
                beta: eco_float_field(&reader, "EconomyObjective", "beta")?,
            },
            other => {
                return Err(CodecError::Decode(format!(
                    "unknown economy objective variant `{other}`"
                )))
            }
        };
        objective
            .validate()
            .map_err(|error| CodecError::Decode(error.to_string()))?;
        Ok(objective)
    }
}

impl ToRdf for CostProjection {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<NamedNode, CodecError> {
        let node = ctx.instance();
        emit_eco_type(ctx, &node, "CostProjection");
        let mut entries = Vec::new();
        for (dimension, weight) in &self.weights {
            let entry = ctx.instance();
            emit_eco_type(ctx, &entry, "ProjectionWeight");
            emit_eco_literal(
                ctx,
                &entry,
                "ProjectionWeight",
                "dimension",
                Prim::Text(dimension.as_str().to_string()).to_literal()?,
            );
            emit_eco_literal(
                ctx,
                &entry,
                "ProjectionWeight",
                "weight",
                Prim::Float(*weight).to_literal()?,
            );
            entries.push(rdf_codec::Term::from(entry));
        }
        let list = ctx.list(&entries)?;
        emit_eco_object(ctx, &node, "CostProjection", "weights", &list);
        Ok(node)
    }
}

impl FromRdf for CostProjection {
    fn from_rdf(node: &NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_economy(graph, node);
        reader.expect_type("CostProjection")?;
        let mut projection = Self::new();
        for term in reader.list(&economy_prop("CostProjection", "weights"))? {
            let rdf_codec::Term::NamedNode(entry) = term else {
                return Err(CodecError::Decode(
                    "projection weights must be nodes".into(),
                ));
            };
            let entry_reader = open_economy(graph, &entry);
            entry_reader.expect_type("ProjectionWeight")?;
            let dimension_name = eco_text_field(&entry_reader, "ProjectionWeight", "dimension")?;
            let dimension = ResourceDimension::from_name(&dimension_name).ok_or_else(|| {
                CodecError::Decode(format!("unknown resource dimension `{dimension_name}`"))
            })?;
            let weight = eco_float_field(&entry_reader, "ProjectionWeight", "weight")?;
            projection = projection
                .with_weight(dimension, weight)
                .map_err(|error| CodecError::Decode(error.to_string()))?;
        }
        Ok(projection)
    }
}

impl ToRdf for EconomyCandidate {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<NamedNode, CodecError> {
        self.validate()
            .map_err(|error| CodecError::Encode(error.to_string()))?;
        let node = ctx.instance();
        emit_eco_type(ctx, &node, "EconomyCandidate");
        emit_eco_literal(
            ctx,
            &node,
            "EconomyCandidate",
            "id",
            Prim::Text(self.id.clone()).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyCandidate",
            "kind",
            Prim::Text(self.kind.as_str().to_string()).to_literal()?,
        );
        if let Some(eig) = self.eig {
            emit_eco_literal(
                ctx,
                &node,
                "EconomyCandidate",
                "eig",
                Prim::Float(eig).to_literal()?,
            );
        }
        if let Some(iv) = self.iv {
            emit_eco_literal(
                ctx,
                &node,
                "EconomyCandidate",
                "iv",
                Prim::Float(iv).to_literal()?,
            );
        }
        if let Some(edv) = self.edv {
            emit_eco_literal(
                ctx,
                &node,
                "EconomyCandidate",
                "edv",
                Prim::Float(edv).to_literal()?,
            );
        }
        let cost_node = self.cost.to_rdf(ctx)?;
        emit_eco_object(ctx, &node, "EconomyCandidate", "cost", &cost_node);
        Ok(node)
    }
}

impl FromRdf for EconomyCandidate {
    fn from_rdf(node: &NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_economy(graph, node);
        reader.expect_type("EconomyCandidate")?;
        let id = eco_text_field(&reader, "EconomyCandidate", "id")?;
        let kind_name = eco_text_field(&reader, "EconomyCandidate", "kind")?;
        let kind = EconomyActionKind::from_name(&kind_name).ok_or_else(|| {
            CodecError::Decode(format!("unknown economy action kind `{kind_name}`"))
        })?;
        let cost_node = reader.object(&economy_prop("EconomyCandidate", "cost"))?;
        let cost = ResourceVector::from_rdf(&cost_node, graph)?;
        let mut candidate = Self::new(id, kind, cost);
        candidate.eig = eco_optional_float_field(&reader, "EconomyCandidate", "eig")?;
        candidate.iv = eco_optional_float_field(&reader, "EconomyCandidate", "iv")?;
        candidate.edv = eco_optional_float_field(&reader, "EconomyCandidate", "edv")?;
        candidate
            .validate()
            .map_err(|error| CodecError::Decode(error.to_string()))?;
        Ok(candidate)
    }
}

impl ToRdf for EconomyDecision {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<NamedNode, CodecError> {
        let node = ctx.instance();
        emit_eco_type(ctx, &node, "EconomyDecision");
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "candidate",
            Prim::Text(self.candidate.clone()).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "kind",
            Prim::Text(self.kind.as_str().to_string()).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "objectiveValue",
            Prim::Float(self.objective_value).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "costScalar",
            Prim::Float(self.cost_scalar).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "rank",
            Prim::UInt(self.rank as u64).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "selected",
            Prim::Bool(self.selected).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "rejected",
            Prim::Bool(self.rejected).to_literal()?,
        );
        emit_eco_literal(
            ctx,
            &node,
            "EconomyDecision",
            "reason",
            Prim::Text(self.reason.clone()).to_literal()?,
        );
        if let Some(eig) = self.eig {
            emit_eco_literal(
                ctx,
                &node,
                "EconomyDecision",
                "eig",
                Prim::Float(eig).to_literal()?,
            );
        }
        if let Some(iv) = self.iv {
            emit_eco_literal(
                ctx,
                &node,
                "EconomyDecision",
                "iv",
                Prim::Float(iv).to_literal()?,
            );
        }
        if let Some(edv) = self.edv {
            emit_eco_literal(
                ctx,
                &node,
                "EconomyDecision",
                "edv",
                Prim::Float(edv).to_literal()?,
            );
        }
        let cost_node = self.cost.to_rdf(ctx)?;
        emit_eco_object(ctx, &node, "EconomyDecision", "cost", &cost_node);
        Ok(node)
    }
}

impl FromRdf for EconomyDecision {
    fn from_rdf(node: &NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_economy(graph, node);
        reader.expect_type("EconomyDecision")?;
        let candidate = eco_text_field(&reader, "EconomyDecision", "candidate")?;
        let kind_name = eco_text_field(&reader, "EconomyDecision", "kind")?;
        let kind = EconomyActionKind::from_name(&kind_name).ok_or_else(|| {
            CodecError::Decode(format!("unknown economy action kind `{kind_name}`"))
        })?;
        let cost_node = reader.object(&economy_prop("EconomyDecision", "cost"))?;
        Ok(Self {
            candidate,
            kind,
            objective_value: eco_float_field(&reader, "EconomyDecision", "objectiveValue")?,
            cost_scalar: eco_float_field(&reader, "EconomyDecision", "costScalar")?,
            rank: match Prim::from_literal(
                &reader.literal(&economy_prop("EconomyDecision", "rank"))?,
            )? {
                Prim::UInt(rank) => usize::try_from(rank)
                    .map_err(|_| CodecError::Decode("decision rank must be non-negative".into()))?,
                Prim::Int(rank) => usize::try_from(rank.max(0))
                    .map_err(|_| CodecError::Decode("decision rank must be non-negative".into()))?,
                other => {
                    return Err(CodecError::Decode(format!(
                        "EconomyDecision/rank must be an integer, got {other:?}"
                    )))
                }
            },
            selected: match Prim::from_literal(
                &reader.literal(&economy_prop("EconomyDecision", "selected"))?,
            )? {
                Prim::Bool(selected) => selected,
                other => {
                    return Err(CodecError::Decode(format!(
                        "EconomyDecision/selected must be boolean, got {other:?}"
                    )))
                }
            },
            rejected: match Prim::from_literal(
                &reader.literal(&economy_prop("EconomyDecision", "rejected"))?,
            )? {
                Prim::Bool(rejected) => rejected,
                other => {
                    return Err(CodecError::Decode(format!(
                        "EconomyDecision/rejected must be boolean, got {other:?}"
                    )))
                }
            },
            reason: eco_text_field(&reader, "EconomyDecision", "reason")?,
            eig: eco_optional_float_field(&reader, "EconomyDecision", "eig")?,
            iv: eco_optional_float_field(&reader, "EconomyDecision", "iv")?,
            edv: eco_optional_float_field(&reader, "EconomyDecision", "edv")?,
            cost: ResourceVector::from_rdf(&cost_node, graph)?,
        })
    }
}

impl ToRdf for EconomyDecisionRecord {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<NamedNode, CodecError> {
        let node = ctx.instance();
        emit_eco_type(ctx, &node, "EconomyDecisionRecord");
        let objective_node = self.objective.to_rdf(ctx)?;
        emit_eco_object(
            ctx,
            &node,
            "EconomyDecisionRecord",
            "objective",
            &objective_node,
        );
        let projection_node = self.projection.to_rdf(ctx)?;
        emit_eco_object(
            ctx,
            &node,
            "EconomyDecisionRecord",
            "projection",
            &projection_node,
        );
        let mut decision_nodes = Vec::new();
        for decision in &self.decisions {
            decision_nodes.push(rdf_codec::Term::from(decision.to_rdf(ctx)?));
        }
        let list = ctx.list(&decision_nodes)?;
        emit_eco_object(ctx, &node, "EconomyDecisionRecord", "decisions", &list);
        if let Some(selected) = &self.selected {
            emit_eco_literal(
                ctx,
                &node,
                "EconomyDecisionRecord",
                "selected",
                Prim::Text(selected.clone()).to_literal()?,
            );
        }
        Ok(node)
    }
}

impl FromRdf for EconomyDecisionRecord {
    fn from_rdf(node: &NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_economy(graph, node);
        reader.expect_type("EconomyDecisionRecord")?;
        let objective_node = reader.object(&economy_prop("EconomyDecisionRecord", "objective"))?;
        let objective = EconomyObjective::from_rdf(&objective_node, graph)?;
        let projection_node =
            reader.object(&economy_prop("EconomyDecisionRecord", "projection"))?;
        let projection = CostProjection::from_rdf(&projection_node, graph)?;
        let mut decisions = Vec::new();
        for term in reader.list(&economy_prop("EconomyDecisionRecord", "decisions"))? {
            let rdf_codec::Term::NamedNode(entry) = term else {
                return Err(CodecError::Decode("record decisions must be nodes".into()));
            };
            decisions.push(EconomyDecision::from_rdf(&entry, graph)?);
        }
        let selected =
            match reader.optional_literal(&economy_prop("EconomyDecisionRecord", "selected"))? {
                None => None,
                Some(literal) => match Prim::from_literal(&literal)? {
                    Prim::Text(selected) => Some(selected),
                    other => {
                        return Err(CodecError::Decode(format!(
                            "EconomyDecisionRecord/selected must be text, got {other:?}"
                        )))
                    }
                },
            };
        Ok(Self {
            objective,
            projection,
            decisions,
            selected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compute_cost(units: f64) -> ResourceVector {
        ResourceVector::new()
            .set(ResourceDimension::ComputeUnits, units)
            .unwrap()
    }

    /// Design §17's own example: solver A (gain 5.0, cost 100) vs solver B
    /// (gain 3.0, cost 2).
    fn design_fixture() -> Vec<EconomyCandidate> {
        vec![
            EconomyCandidate::new(
                "solver-a",
                EconomyActionKind::SolverCall,
                compute_cost(100.0),
            )
            .with_eig(5.0),
            EconomyCandidate::new("solver-b", EconomyActionKind::SolverCall, compute_cost(2.0))
                .with_eig(3.0),
        ]
    }

    #[test]
    fn ipc_prefers_cheap_low_info_over_expensive_high_info() {
        let record = decide_economy(
            &design_fixture(),
            &EconomyObjective::InformationPerCompute,
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert_eq!(record.selected.as_deref(), Some("solver-b"));
        let a = record
            .decisions
            .iter()
            .find(|d| d.candidate == "solver-a")
            .unwrap();
        assert!((a.objective_value - 0.05).abs() < 1e-12);
        let b = record
            .decisions
            .iter()
            .find(|d| d.candidate == "solver-b")
            .unwrap();
        assert!((b.objective_value - 1.5).abs() < 1e-12);
        assert_eq!(b.rank, 0);
        assert!(b.selected);
    }

    #[test]
    fn reversed_weighting_reverses_the_choice_deterministically() {
        // Utility competitors declare their information value.
        let valuable = EconomyCandidate::new(
            "valuable",
            EconomyActionKind::TheoremProving,
            compute_cost(100.0),
        )
        .with_iv(5.0);
        let modest =
            EconomyCandidate::new("modest", EconomyActionKind::Simulation, compute_cost(2.0))
                .with_iv(3.0);

        // Light λ: the high-value expensive route wins on net utility.
        let light = decide_economy(
            &[valuable.clone(), modest.clone()],
            &EconomyObjective::NetUtility { lambda: 0.01 },
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert_eq!(light.selected.as_deref(), Some("valuable"));
        assert!((light.decisions[0].objective_value - (5.0 - 1.0)).abs() < 1e-12);

        // Heavy λ: the cost term dominates and the choice flips.
        let heavy_objective = EconomyObjective::NetUtility { lambda: 0.04 };
        let heavy = decide_economy(
            &[valuable, modest],
            &heavy_objective,
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert_eq!(heavy.selected.as_deref(), Some("modest"));

        // Determinism: identical inputs yield identical records.
        let again = decide_economy(
            &[
                EconomyCandidate::new(
                    "valuable",
                    EconomyActionKind::TheoremProving,
                    compute_cost(100.0),
                )
                .with_iv(5.0),
                EconomyCandidate::new("modest", EconomyActionKind::Simulation, compute_cost(2.0))
                    .with_iv(3.0),
            ],
            &heavy_objective,
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert_eq!(heavy, again);
    }

    #[test]
    fn negative_net_utility_rejects_with_reason() {
        let candidates = vec![EconomyCandidate::new(
            "not-worth-it",
            EconomyActionKind::Measurement,
            compute_cost(50.0),
        )
        .with_iv(1.0)];
        let record = decide_economy(
            &candidates,
            &EconomyObjective::NetUtility { lambda: 0.5 },
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert!(record.is_all_rejected());
        assert_eq!(record.selected, None);
        let decision = &record.decisions[0];
        assert!(decision.rejected);
        assert!(
            decision.reason.contains("non-positive"),
            "{}",
            decision.reason
        );
        assert!((decision.objective_value - (1.0 - 25.0)).abs() < 1e-12);
    }

    #[test]
    fn value_per_resource_matches_hand_computation() {
        // (EDV + β·EIG) / cost = (10 + 2·4) / 9 = 2.
        let candidate = EconomyCandidate::new("c", EconomyActionKind::Search, compute_cost(9.0))
            .with_edv(10.0)
            .with_eig(4.0);
        let score = score_candidate(
            &candidate,
            &EconomyObjective::ValuePerResource { beta: 2.0 },
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert!((score - 2.0).abs() < 1e-12);
    }

    #[test]
    fn missing_quantities_are_typed_gaps_not_fabrications() {
        let candidate =
            EconomyCandidate::new("blind", EconomyActionKind::Search, compute_cost(1.0));
        match score_candidate(
            &candidate,
            &EconomyObjective::InformationPerCompute,
            &CostProjection::single(ResourceDimension::ComputeUnits),
        ) {
            Err(EconomyError::MissingQuantity { quantity, .. }) => {
                assert_eq!(quantity, "expectedInformationGain");
            }
            other => panic!("expected a typed gap, got {other:?}"),
        }
        // On the record the same gap is a rejection carrying its reason.
        let record = decide_economy(
            &[candidate],
            &EconomyObjective::ValuePerResource { beta: 1.0 },
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert!(record.is_all_rejected());
        assert!(record.decisions[0].reason.contains("typed gap"));
    }

    #[test]
    fn unpriced_dimensions_are_typed_errors() {
        // The candidate costs wall-time; the projection prices only compute.
        let candidate = EconomyCandidate::new(
            "timed",
            EconomyActionKind::Simulation,
            ResourceVector::new()
                .set(ResourceDimension::WallTime, 3.0)
                .unwrap(),
        )
        .with_eig(1.0);
        match decide_economy(
            &[candidate],
            &EconomyObjective::InformationPerCompute,
            &CostProjection::single(ResourceDimension::ComputeUnits),
        ) {
            Err(EconomyError::MissingWeight { dimension }) => {
                assert_eq!(dimension, ResourceDimension::WallTime)
            }
            other => panic!("expected MissingWeight, got {other:?}"),
        }
    }

    #[test]
    fn invalid_weights_and_quantities_are_typed_errors() {
        assert!(matches!(
            EconomyObjective::NetUtility { lambda: -1.0 }.validate(),
            Err(EconomyError::InvalidWeight { .. })
        ));
        let projection = CostProjection::single(ResourceDimension::ComputeUnits)
            .with_weight(ResourceDimension::Memory, f64::NAN);
        assert!(projection.is_err());
        let candidate = EconomyCandidate::new("x", EconomyActionKind::Search, compute_cost(1.0))
            .with_eig(f64::NAN);
        assert!(matches!(
            score_candidate(
                &candidate,
                &EconomyObjective::InformationPerCompute,
                &CostProjection::single(ResourceDimension::ComputeUnits)
            ),
            Err(EconomyError::InvalidQuantity { .. })
        ));
    }

    #[test]
    fn zero_cost_ratio_candidates_reject_instead_of_dividing() {
        let free = EconomyCandidate::new("free", EconomyActionKind::Search, ResourceVector::new())
            .with_eig(5.0);
        let record = decide_economy(
            &[free],
            &EconomyObjective::InformationPerCompute,
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert!(record.is_all_rejected());
        assert!(record.decisions[0]
            .reason
            .contains("non-positive projected cost"));
    }

    #[test]
    fn pareto_front_keeps_the_nondominated_set() {
        // b dominates nothing but is dominated by nobody on cost; c has the
        // best value; a is dominated by c (same value class, higher cost).
        let cheap = EconomyCandidate::new("cheap", EconomyActionKind::Search, compute_cost(1.0))
            .with_eig(1.0);
        let mid = EconomyCandidate::new("mid", EconomyActionKind::Search, compute_cost(2.0))
            .with_eig(1.0);
        let best = EconomyCandidate::new("best", EconomyActionKind::Search, compute_cost(3.0))
            .with_eig(10.0);
        let front = pareto_front(
            &[cheap, mid, best],
            &EconomyObjective::InformationPerCompute,
            &CostProjection::single(ResourceDimension::ComputeUnits),
        )
        .unwrap();
        assert_eq!(front, vec!["cheap", "best"]);
    }

    #[test]
    fn records_round_trip_through_rdf() {
        let mut candidates = design_fixture();
        // One info action with a second cost dimension joins the frame — and
        // forces the projection to declare both categories explicitly.
        candidates.push(
            EconomyCandidate::new(
                "observe",
                EconomyActionKind::Measurement,
                compute_cost(1.0)
                    .set(ResourceDimension::WallTime, 0.5)
                    .unwrap(),
            )
            .with_eig(0.8),
        );
        let projection = CostProjection::single(ResourceDimension::ComputeUnits)
            .with_weight(ResourceDimension::WallTime, 1.0)
            .unwrap();
        let record = decide_economy(
            &candidates,
            &EconomyObjective::InformationPerCompute,
            &projection,
        )
        .unwrap();

        let mut ctx = RdfContext::new(
            rdf_codec::Namespace::new(ECONOMY_NS.to_string()),
            rdf_codec::Namespace::new("https://example.org/data/economy/".to_string()),
        )
        .with_id_policy(rdf_codec::IdPolicy::FromContentHash);
        let node = record.to_rdf(&mut ctx).unwrap();
        let decoded = EconomyDecisionRecord::from_rdf(&node, &ctx.graph).unwrap();
        assert_eq!(decoded, record);
    }
}
