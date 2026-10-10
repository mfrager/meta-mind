//! Shared resource vocabulary for computational planning (completion plan Phase 2).
//!
//! The vector is intentionally independent of any one consumer.  Each
//! dimension has a canonical unit, hard and soft constraints remain separate,
//! and adapters report lossy projections instead of silently dropping data.

use std::collections::BTreeMap;

use rdf_codec::scalar::Prim;
use rdf_codec::{CodecError, FromRdf, RdfContext, RdfReader, ToRdf};

/// Canonical dimensions shared by complexity, solver, event, information, and
/// orchestration consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceDimension {
    WallTime,
    CpuTime,
    /// GPU compute time in seconds (Phase 10 rate basis).
    GpuTime,
    Memory,
    GpuMemory,
    /// Aggregate energy; component dimensions may be used instead.
    Energy,
    EnergyCpu,
    EnergyGpu,
    EnergyMemory,
    EnergyNetwork,
    NetworkBytes,
    Storage,
    MonetaryCost,
    Latency,
    PhysicalRisk,
    ComputeUnits,
    Iterations,
}

impl ResourceDimension {
    pub const ALL: &'static [Self] = &[
        Self::WallTime,
        Self::CpuTime,
        Self::GpuTime,
        Self::Memory,
        Self::GpuMemory,
        Self::Energy,
        Self::EnergyCpu,
        Self::EnergyGpu,
        Self::EnergyMemory,
        Self::EnergyNetwork,
        Self::NetworkBytes,
        Self::Storage,
        Self::MonetaryCost,
        Self::Latency,
        Self::PhysicalRisk,
        Self::ComputeUnits,
        Self::Iterations,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::WallTime => "wallTime",
            Self::CpuTime => "cpuTime",
            Self::GpuTime => "gpuTime",
            Self::Memory => "memory",
            Self::GpuMemory => "gpuMemory",
            Self::Energy => "energy",
            Self::EnergyCpu => "energyCpu",
            Self::EnergyGpu => "energyGpu",
            Self::EnergyMemory => "energyMemory",
            Self::EnergyNetwork => "energyNetwork",
            Self::NetworkBytes => "networkBytes",
            Self::Storage => "storage",
            Self::MonetaryCost => "monetaryCost",
            Self::Latency => "latency",
            Self::PhysicalRisk => "physicalRisk",
            Self::ComputeUnits => "computeUnits",
            Self::Iterations => "iterations",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|dimension| dimension.as_str().eq_ignore_ascii_case(name))
    }

    pub fn canonical_unit(self) -> ResourceUnit {
        match self {
            Self::WallTime | Self::CpuTime | Self::GpuTime | Self::Latency => ResourceUnit::Seconds,
            Self::Memory | Self::GpuMemory | Self::Storage | Self::NetworkBytes => {
                ResourceUnit::Bytes
            }
            Self::Energy
            | Self::EnergyCpu
            | Self::EnergyGpu
            | Self::EnergyMemory
            | Self::EnergyNetwork => ResourceUnit::Joules,
            Self::MonetaryCost => ResourceUnit::Usd,
            Self::PhysicalRisk => ResourceUnit::Probability,
            Self::ComputeUnits | Self::Iterations => ResourceUnit::Count,
        }
    }
}

/// Canonical units accepted by the shared resource vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceUnit {
    Seconds,
    Bytes,
    Joules,
    Usd,
    Probability,
    Count,
}

impl ResourceUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Seconds => "seconds",
            Self::Bytes => "bytes",
            Self::Joules => "joules",
            Self::Usd => "USD",
            Self::Probability => "probability",
            Self::Count => "count",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "seconds" | "second" | "s" => Some(Self::Seconds),
            "bytes" | "byte" | "b" => Some(Self::Bytes),
            "joules" | "joule" | "j" => Some(Self::Joules),
            "usd" | "$" => Some(Self::Usd),
            "probability" | "risk" => Some(Self::Probability),
            "count" | "unit" | "units" => Some(Self::Count),
            _ => None,
        }
    }
}

/// One dimensioned resource quantity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceValue {
    pub value: f64,
    pub unit: ResourceUnit,
}

impl ResourceValue {
    pub fn new(value: f64, unit: ResourceUnit) -> Result<Self, ResourceError> {
        if !value.is_finite() || value < 0.0 {
            return Err(ResourceError::InvalidValue { value });
        }
        Ok(Self { value, unit })
    }

    pub const fn zero(unit: ResourceUnit) -> Self {
        Self { value: 0.0, unit }
    }
}

/// A sparse resource vector. Missing dimensions mean zero demand, not an
/// unbounded value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResourceVector {
    pub values: BTreeMap<ResourceDimension, ResourceValue>,
}

impl ResourceVector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, dimension: ResourceDimension, value: f64) -> Result<Self, ResourceError> {
        self.insert(dimension, value)?;
        Ok(self)
    }

    pub fn insert(
        &mut self,
        dimension: ResourceDimension,
        value: f64,
    ) -> Result<(), ResourceError> {
        let value = ResourceValue::new(value, dimension.canonical_unit())?;
        self.values.insert(dimension, value);
        Ok(())
    }

    pub fn insert_value(
        &mut self,
        dimension: ResourceDimension,
        value: ResourceValue,
    ) -> Result<(), ResourceError> {
        let expected = dimension.canonical_unit();
        if value.unit != expected {
            return Err(ResourceError::UnitMismatch {
                dimension,
                expected,
                actual: value.unit,
            });
        }
        if !value.value.is_finite() || value.value < 0.0 {
            return Err(ResourceError::InvalidValue { value: value.value });
        }
        self.values.insert(dimension, value);
        Ok(())
    }

    pub fn get(&self, dimension: ResourceDimension) -> Option<ResourceValue> {
        self.values.get(&dimension).copied()
    }

    pub fn value_or_zero(&self, dimension: ResourceDimension) -> f64 {
        self.get(dimension).map(|value| value.value).unwrap_or(0.0)
    }

    /// Return aggregate energy without double-counting a vector that already
    /// carries an explicit total alongside decomposed components.
    pub fn total_energy(&self) -> f64 {
        self.get(ResourceDimension::Energy)
            .map(|value| value.value)
            .unwrap_or_else(|| {
                [
                    ResourceDimension::EnergyCpu,
                    ResourceDimension::EnergyGpu,
                    ResourceDimension::EnergyMemory,
                    ResourceDimension::EnergyNetwork,
                ]
                .iter()
                .map(|dimension| self.value_or_zero(*dimension))
                .sum()
            })
    }

    pub fn add(&self, other: &Self) -> Result<Self, ResourceError> {
        let has_explicit_total = self.get(ResourceDimension::Energy).is_some()
            || other.get(ResourceDimension::Energy).is_some();
        let mut result = self.clone();
        for (dimension, value) in &other.values {
            let current = result.value_or_zero(*dimension);
            let sum = current + value.value;
            if !sum.is_finite() {
                return Err(ResourceError::Overflow {
                    dimension: *dimension,
                });
            }
            result.insert(*dimension, sum)?;
        }
        if has_explicit_total {
            let total = self.total_energy() + other.total_energy();
            if !total.is_finite() {
                return Err(ResourceError::Overflow {
                    dimension: ResourceDimension::Energy,
                });
            }
            result.insert(ResourceDimension::Energy, total)?;
        }
        Ok(result)
    }

    pub fn with_energy_total(self, value: f64) -> Result<Self, ResourceError> {
        self.set(ResourceDimension::Energy, value)
    }

    pub fn validate(&self) -> Result<(), ResourceError> {
        for (dimension, value) in &self.values {
            if value.unit != dimension.canonical_unit() {
                return Err(ResourceError::UnitMismatch {
                    dimension: *dimension,
                    expected: dimension.canonical_unit(),
                    actual: value.unit,
                });
            }
            if !value.value.is_finite() || value.value < 0.0 {
                return Err(ResourceError::InvalidValue { value: value.value });
            }
        }
        Ok(())
    }
}

/// Hard versus soft constraint semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstraintKind {
    Hard,
    Soft,
}

/// One upper-bound constraint on one resource dimension.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceConstraint {
    pub limit: ResourceValue,
    pub kind: ConstraintKind,
}

impl ResourceConstraint {
    pub fn new(
        dimension: ResourceDimension,
        limit: f64,
        kind: ConstraintKind,
    ) -> Result<Self, ResourceError> {
        Ok(Self {
            limit: ResourceValue::new(limit, dimension.canonical_unit())?,
            kind,
        })
    }
}

/// A sparse collection of hard and soft upper bounds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BudgetSet {
    /// Hard and soft constraints are stored independently so both may exist
    /// for the same dimension.
    pub constraints: BTreeMap<ResourceDimension, Vec<ResourceConstraint>>,
}

impl BudgetSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_constraint(
        mut self,
        dimension: ResourceDimension,
        limit: f64,
        kind: ConstraintKind,
    ) -> Result<Self, ResourceError> {
        self.insert(dimension, limit, kind)?;
        Ok(self)
    }

    pub fn insert(
        &mut self,
        dimension: ResourceDimension,
        limit: f64,
        kind: ConstraintKind,
    ) -> Result<(), ResourceError> {
        self.constraints
            .entry(dimension)
            .or_default()
            .push(ResourceConstraint::new(dimension, limit, kind)?);
        Ok(())
    }

    /// Return the tightest hard upper bound for a dimension.
    pub fn hard_limit(&self, dimension: ResourceDimension) -> Option<f64> {
        self.constraints
            .get(&dimension)?
            .iter()
            .filter(|constraint| constraint.kind == ConstraintKind::Hard)
            .map(|constraint| constraint.limit.value)
            .reduce(f64::min)
    }

    /// Return the tightest soft upper bound for a dimension.
    pub fn soft_limit(&self, dimension: ResourceDimension) -> Option<f64> {
        self.constraints
            .get(&dimension)?
            .iter()
            .filter(|constraint| constraint.kind == ConstraintKind::Soft)
            .map(|constraint| constraint.limit.value)
            .reduce(f64::min)
    }

    pub fn check(&self, required: &ResourceVector) -> Result<BudgetCheck, ResourceError> {
        required.validate()?;
        let mut result = BudgetCheck::default();
        for (dimension, constraints) in &self.constraints {
            let required_value = if *dimension == ResourceDimension::Energy {
                required.total_energy()
            } else {
                required.value_or_zero(*dimension)
            };
            for constraint in constraints {
                if required_value > constraint.limit.value {
                    let deficit = required_value - constraint.limit.value;
                    if !deficit.is_finite() {
                        return Err(ResourceError::Overflow {
                            dimension: *dimension,
                        });
                    }
                    let violation = ResourceViolation {
                        dimension: *dimension,
                        required: required_value,
                        available: constraint.limit.value,
                        deficit,
                        kind: constraint.kind,
                    };
                    match constraint.kind {
                        ConstraintKind::Hard => result.hard.push(violation),
                        ConstraintKind::Soft => result.soft.push(violation),
                    }
                }
            }
        }
        Ok(result)
    }
}

/// Result of checking a vector against a budget.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BudgetCheck {
    pub hard: Vec<ResourceViolation>,
    pub soft: Vec<ResourceViolation>,
}

impl BudgetCheck {
    pub fn feasible(&self) -> bool {
        self.hard.is_empty()
    }
}

/// A typed resource violation with exact deficit arithmetic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceViolation {
    pub dimension: ResourceDimension,
    pub required: f64,
    pub available: f64,
    pub deficit: f64,
    pub kind: ConstraintKind,
}

/// Capability flags that can be projected into a resource requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapabilityRequirements {
    pub cpu: bool,
    pub gpu: bool,
    pub network: bool,
    pub memory_bytes: u64,
}

/// A named and dimensional reservation over a time window.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceReservation {
    pub holder: String,
    pub start: f64,
    pub end: f64,
    pub vector: ResourceVector,
    /// Non-canonical event resources are retained here rather than dropped.
    pub named_resources: BTreeMap<String, f64>,
}

impl ResourceReservation {
    pub fn new(holder: impl Into<String>, start: f64, end: f64) -> Result<Self, ResourceError> {
        if !start.is_finite() || !end.is_finite() || end < start {
            return Err(ResourceError::InvalidWindow { start, end });
        }
        Ok(Self {
            holder: holder.into(),
            start,
            end,
            vector: ResourceVector::new(),
            named_resources: BTreeMap::new(),
        })
    }

    pub fn with_dimension(
        mut self,
        dimension: ResourceDimension,
        value: f64,
    ) -> Result<Self, ResourceError> {
        self.vector.insert(dimension, value)?;
        Ok(self)
    }

    pub fn with_named_resource(mut self, name: impl Into<String>, value: f64) -> Self {
        self.named_resources.insert(name.into(), value);
        self
    }

    pub fn validate(&self) -> Result<(), ResourceError> {
        if !self.start.is_finite() || !self.end.is_finite() || self.end < self.start {
            return Err(ResourceError::InvalidWindow {
                start: self.start,
                end: self.end,
            });
        }
        self.vector.validate()?;
        if self
            .named_resources
            .values()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(ResourceError::InvalidValue {
                value: self
                    .named_resources
                    .values()
                    .copied()
                    .find(|value| !value.is_finite() || *value < 0.0)
                    .unwrap_or(f64::NAN),
            });
        }
        Ok(())
    }
}

/// A projection records what was mapped and what was intentionally lossy.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversionReport<T> {
    pub value: T,
    pub assumptions: Vec<String>,
    pub losses: Vec<String>,
}

impl<T> ConversionReport<T> {
    pub fn exact(value: T) -> Self {
        Self {
            value,
            assumptions: Vec::new(),
            losses: Vec::new(),
        }
    }

    pub fn is_lossless(&self) -> bool {
        self.losses.is_empty()
    }
}

/// Adapter implemented by consumer crates for legacy resource descriptors.
pub trait ResourceVectorAdapter {
    fn resource_vector(&self) -> ConversionReport<ResourceVector>;
}

/// Adapter for legacy capability flags and the shared capability requirement
/// vocabulary. Capability requirements are distinct from resource quantities.
pub trait CapabilityRequirementsAdapter {
    fn capability_requirements(&self) -> CapabilityRequirements;
    fn from_capability_requirements(requirements: CapabilityRequirements) -> Self
    where
        Self: Sized;
}

/// Adapter implemented by consumer crates for legacy budget descriptors.
pub trait BudgetSetAdapter {
    fn budget_set(&self) -> Result<ConversionReport<BudgetSet>, ResourceError>;
}

/// Adapter implemented by schedulers whose resources occupy time windows.
pub trait ResourceReservationAdapter {
    fn resource_reservation(
        &self,
        holder: impl Into<String>,
        start: f64,
        end: f64,
    ) -> Result<ConversionReport<ResourceReservation>, ResourceError>;
}

/// Errors raised before a resource comparison can fabricate a verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceError {
    InvalidValue {
        value: f64,
    },
    InvalidWindow {
        start: f64,
        end: f64,
    },
    UnitMismatch {
        dimension: ResourceDimension,
        expected: ResourceUnit,
        actual: ResourceUnit,
    },
    Overflow {
        dimension: ResourceDimension,
    },
    MissingCalibration {
        dimension: ResourceDimension,
    },
    UnknownNamedResource,
}

impl ResourceError {
    pub fn dimension(&self) -> Option<ResourceDimension> {
        match self {
            Self::UnitMismatch { dimension, .. }
            | Self::Overflow { dimension }
            | Self::MissingCalibration { dimension } => Some(*dimension),
            Self::InvalidValue { .. } | Self::InvalidWindow { .. } | Self::UnknownNamedResource => {
                None
            }
        }
    }
}

impl std::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue { value } => write!(f, "invalid resource value {value}"),
            Self::InvalidWindow { start, end } => {
                write!(f, "invalid resource window [{start}, {end}]")
            }
            Self::UnitMismatch {
                dimension,
                expected,
                actual,
            } => write!(
                f,
                "resource {} requires {} units, got {}",
                dimension.as_str(),
                expected.as_str(),
                actual.as_str()
            ),
            Self::Overflow { dimension } => {
                write!(f, "resource {} arithmetic overflow", dimension.as_str())
            }
            Self::MissingCalibration { dimension } => write!(
                f,
                "resource {} requires an explicit calibration",
                dimension.as_str()
            ),
            Self::UnknownNamedResource => write!(f, "resource name is not declared in the pool"),
        }
    }
}

impl std::error::Error for ResourceError {}

// ── RDF codec ───────────────────────────────────────────────────────────────

const RESOURCE_NS: &str = "https://example.org/ns/resource#";

fn resource_prop(class: &str, field: &str) -> String {
    format!("{RESOURCE_NS}{class}/{field}")
}

fn resource_class(class: &str) -> String {
    format!("{RESOURCE_NS}{class}")
}

/// Emit helpers pinned to the canonical shared-resource namespace. These
/// classes are embedded in many subsystem documents; emitting them under the
/// enclosing document's schema namespace would break the one-vocabulary rule
/// (the reader is always the canonical namespace).
fn emit_res_type(ctx: &mut RdfContext, node: &rdf_codec::NamedNode, class: &str) {
    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let class_node = rdf_codec::NamedNode::new(resource_class(class)).expect("class IRI");
    let type_pred = rdf_codec::NamedNode::new(RDF_TYPE).expect("constant IRI");
    ctx.graph.insert(&rdf_codec::Triple::new(
        node.clone(),
        type_pred,
        rdf_codec::Term::from(class_node),
    ));
}

fn emit_res_literal(
    ctx: &mut RdfContext,
    node: &rdf_codec::NamedNode,
    class: &str,
    prop: &str,
    value: rdf_codec::Literal,
) {
    let p = rdf_codec::NamedNode::new(resource_prop(class, prop)).expect("property IRI");
    ctx.graph.insert(&rdf_codec::Triple::new(
        node.clone(),
        p,
        rdf_codec::Term::from(value),
    ));
}

fn emit_res_object(
    ctx: &mut RdfContext,
    node: &rdf_codec::NamedNode,
    class: &str,
    prop: &str,
    obj: &rdf_codec::NamedNode,
) {
    let p = rdf_codec::NamedNode::new(resource_prop(class, prop)).expect("property IRI");
    ctx.graph.insert(&rdf_codec::Triple::new(
        node.clone(),
        p,
        rdf_codec::Term::from(obj.clone()),
    ));
}

fn open_resource<'a>(graph: &'a rdf_codec::Graph, node: &rdf_codec::NamedNode) -> RdfReader<'a> {
    static NS: std::sync::OnceLock<rdf_codec::Namespace> = std::sync::OnceLock::new();
    RdfReader::new(
        graph,
        node,
        NS.get_or_init(|| rdf_codec::Namespace::new(RESOURCE_NS)),
    )
}

fn text_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<String, CodecError> {
    match Prim::from_literal(&reader.literal(&resource_prop(class, field))?)? {
        Prim::Text(value) => Ok(value),
        other => Err(CodecError::Decode(format!(
            "{class}/{field} must be text, got {other:?}"
        ))),
    }
}

fn float_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<f64, CodecError> {
    match Prim::from_literal(&reader.literal(&resource_prop(class, field))?)? {
        Prim::Float(value) => Ok(value),
        other => Err(CodecError::Decode(format!(
            "{class}/{field} must be float, got {other:?}"
        ))),
    }
}

impl ToRdf for ResourceVector {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        self.validate()
            .map_err(|error| CodecError::Encode(error.to_string()))?;
        let node = ctx.instance();
        emit_res_type(ctx, &node, "ResourceVector");
        let mut entries = Vec::new();
        for (dimension, value) in &self.values {
            let entry = ctx.instance();
            emit_res_type(ctx, &entry, "ResourceEntry");
            emit_res_literal(
                ctx,
                &entry,
                "ResourceEntry",
                "dimension",
                Prim::Text(dimension.as_str().to_string()).to_literal()?,
            );
            emit_res_literal(
                ctx,
                &entry,
                "ResourceEntry",
                "value",
                Prim::Float(value.value).to_literal()?,
            );
            emit_res_literal(
                ctx,
                &entry,
                "ResourceEntry",
                "unit",
                Prim::Text(value.unit.as_str().to_string()).to_literal()?,
            );
            entries.push(rdf_codec::Term::from(entry));
        }
        let list = ctx.list(&entries)?;
        emit_res_object(ctx, &node, "ResourceVector", "entries", &list);
        Ok(node)
    }
}

impl FromRdf for ResourceVector {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_resource(graph, node);
        reader.expect_type("ResourceVector")?;
        let mut vector = Self::new();
        for term in reader.list(&resource_prop("ResourceVector", "entries"))? {
            let rdf_codec::Term::NamedNode(entry) = term else {
                return Err(CodecError::Decode(
                    "ResourceVector entries must be nodes".into(),
                ));
            };
            let entry_reader = open_resource(graph, &entry);
            entry_reader.expect_type("ResourceEntry")?;
            let dimension_name = text_field(&entry_reader, "ResourceEntry", "dimension")?;
            let dimension = ResourceDimension::from_name(&dimension_name).ok_or_else(|| {
                CodecError::Decode(format!("unknown resource dimension `{dimension_name}`"))
            })?;
            let value = float_field(&entry_reader, "ResourceEntry", "value")?;
            let unit_name = text_field(&entry_reader, "ResourceEntry", "unit")?;
            let unit = ResourceUnit::from_name(&unit_name).ok_or_else(|| {
                CodecError::Decode(format!("unknown resource unit `{unit_name}`"))
            })?;
            vector
                .insert_value(dimension, ResourceValue { value, unit })
                .map_err(|error| CodecError::Decode(error.to_string()))?;
        }
        Ok(vector)
    }
}

impl ToRdf for BudgetSet {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let node = ctx.instance();
        emit_res_type(ctx, &node, "BudgetSet");
        let mut entries = Vec::new();
        for (dimension, constraints) in &self.constraints {
            for constraint in constraints {
                let entry = ctx.instance();
                emit_res_type(ctx, &entry, "ResourceConstraint");
                emit_res_literal(
                    ctx,
                    &entry,
                    "ResourceConstraint",
                    "dimension",
                    Prim::Text(dimension.as_str().to_string()).to_literal()?,
                );
                emit_res_literal(
                    ctx,
                    &entry,
                    "ResourceConstraint",
                    "limit",
                    Prim::Float(constraint.limit.value).to_literal()?,
                );
                emit_res_literal(
                    ctx,
                    &entry,
                    "ResourceConstraint",
                    "unit",
                    Prim::Text(constraint.limit.unit.as_str().to_string()).to_literal()?,
                );
                emit_res_literal(
                    ctx,
                    &entry,
                    "ResourceConstraint",
                    "kind",
                    Prim::Text(
                        match constraint.kind {
                            ConstraintKind::Hard => "hard",
                            ConstraintKind::Soft => "soft",
                        }
                        .to_string(),
                    )
                    .to_literal()?,
                );
                entries.push(rdf_codec::Term::from(entry));
            }
        }
        let list = ctx.list(&entries)?;
        emit_res_object(ctx, &node, "BudgetSet", "constraints", &list);
        Ok(node)
    }
}

impl ToRdf for ResourceReservation {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        self.validate()
            .map_err(|error| CodecError::Encode(error.to_string()))?;
        let node = ctx.instance();
        emit_res_type(ctx, &node, "ResourceReservation");
        emit_res_literal(
            ctx,
            &node,
            "ResourceReservation",
            "holder",
            Prim::Text(self.holder.clone()).to_literal()?,
        );
        emit_res_literal(
            ctx,
            &node,
            "ResourceReservation",
            "start",
            Prim::Float(self.start).to_literal()?,
        );
        emit_res_literal(
            ctx,
            &node,
            "ResourceReservation",
            "end",
            Prim::Float(self.end).to_literal()?,
        );
        let vector = self.vector.to_rdf(ctx)?;
        emit_res_object(ctx, &node, "ResourceReservation", "vector", &vector);
        let mut entries = Vec::new();
        for (name, value) in &self.named_resources {
            let entry = ctx.instance();
            emit_res_type(ctx, &entry, "NamedResourceEntry");
            emit_res_literal(
                ctx,
                &entry,
                "NamedResourceEntry",
                "name",
                Prim::Text(name.clone()).to_literal()?,
            );
            emit_res_literal(
                ctx,
                &entry,
                "NamedResourceEntry",
                "value",
                Prim::Float(*value).to_literal()?,
            );
            entries.push(rdf_codec::Term::from(entry));
        }
        let list = ctx.list(&entries)?;
        emit_res_object(ctx, &node, "ResourceReservation", "namedResources", &list);
        Ok(node)
    }
}

impl FromRdf for ResourceReservation {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_resource(graph, node);
        reader.expect_type("ResourceReservation")?;
        let holder = text_field(&reader, "ResourceReservation", "holder")?;
        let start = float_field(&reader, "ResourceReservation", "start")?;
        let end = float_field(&reader, "ResourceReservation", "end")?;
        let vector_node = reader.object(&resource_prop("ResourceReservation", "vector"))?;
        let vector = ResourceVector::from_rdf(&vector_node, graph)?;
        let mut named_resources = BTreeMap::new();
        for term in reader.list(&resource_prop("ResourceReservation", "namedResources"))? {
            let rdf_codec::Term::NamedNode(entry) = term else {
                return Err(CodecError::Decode(
                    "ResourceReservation named resources must be nodes".into(),
                ));
            };
            let entry_reader = open_resource(graph, &entry);
            entry_reader.expect_type("NamedResourceEntry")?;
            let name = text_field(&entry_reader, "NamedResourceEntry", "name")?;
            let value = float_field(&entry_reader, "NamedResourceEntry", "value")?;
            named_resources.insert(name, value);
        }
        ResourceReservation::new(holder, start, end)
            .map(|mut reservation| {
                reservation.vector = vector;
                reservation.named_resources = named_resources;
                reservation
            })
            .map_err(|error| CodecError::Decode(error.to_string()))
    }
}

impl ToRdf for CapabilityRequirements {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let node = ctx.instance();
        emit_res_type(ctx, &node, "CapabilityRequirements");
        emit_res_literal(
            ctx,
            &node,
            "CapabilityRequirements",
            "cpu",
            Prim::Bool(self.cpu).to_literal()?,
        );
        emit_res_literal(
            ctx,
            &node,
            "CapabilityRequirements",
            "gpu",
            Prim::Bool(self.gpu).to_literal()?,
        );
        emit_res_literal(
            ctx,
            &node,
            "CapabilityRequirements",
            "network",
            Prim::Bool(self.network).to_literal()?,
        );
        emit_res_literal(
            ctx,
            &node,
            "CapabilityRequirements",
            "memoryBytes",
            Prim::UInt(self.memory_bytes).to_literal()?,
        );
        Ok(node)
    }
}

impl FromRdf for CapabilityRequirements {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_resource(graph, node);
        reader.expect_type("CapabilityRequirements")?;
        let bool_value = |field: &str| -> Result<bool, CodecError> {
            match Prim::from_literal(
                &reader.literal(&resource_prop("CapabilityRequirements", field))?,
            )? {
                Prim::Bool(value) => Ok(value),
                other => Err(CodecError::Decode(format!(
                    "CapabilityRequirements/{field} must be bool, got {other:?}"
                ))),
            }
        };
        let memory_bytes = match Prim::from_literal(
            &reader.literal(&resource_prop("CapabilityRequirements", "memoryBytes"))?,
        )? {
            Prim::UInt(value) => value,
            Prim::Int(value) => {
                u64::try_from(value).map_err(|error| CodecError::Decode(error.to_string()))?
            }
            other => {
                return Err(CodecError::Decode(format!(
                    "CapabilityRequirements/memoryBytes must be uint, got {other:?}"
                )))
            }
        };
        Ok(Self {
            cpu: bool_value("cpu")?,
            gpu: bool_value("gpu")?,
            network: bool_value("network")?,
            memory_bytes,
        })
    }
}

impl FromRdf for BudgetSet {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open_resource(graph, node);
        reader.expect_type("BudgetSet")?;
        let mut budget = Self::new();
        for term in reader.list(&resource_prop("BudgetSet", "constraints"))? {
            let rdf_codec::Term::NamedNode(entry) = term else {
                return Err(CodecError::Decode(
                    "BudgetSet constraints must be nodes".into(),
                ));
            };
            let entry_reader = open_resource(graph, &entry);
            entry_reader.expect_type("ResourceConstraint")?;
            let dimension_name = text_field(&entry_reader, "ResourceConstraint", "dimension")?;
            let dimension = ResourceDimension::from_name(&dimension_name).ok_or_else(|| {
                CodecError::Decode(format!("unknown resource dimension `{dimension_name}`"))
            })?;
            let limit = float_field(&entry_reader, "ResourceConstraint", "limit")?;
            let unit_name = text_field(&entry_reader, "ResourceConstraint", "unit")?;
            let unit = ResourceUnit::from_name(&unit_name).ok_or_else(|| {
                CodecError::Decode(format!("unknown resource unit `{unit_name}`"))
            })?;
            if unit != dimension.canonical_unit() {
                return Err(CodecError::Decode(format!(
                    "unit `{unit_name}` does not match {dimension_name}"
                )));
            }
            let kind = match text_field(&entry_reader, "ResourceConstraint", "kind")?.as_str() {
                "hard" => ConstraintKind::Hard,
                "soft" => ConstraintKind::Soft,
                other => {
                    return Err(CodecError::Decode(format!(
                        "unknown constraint kind `{other}`"
                    )))
                }
            };
            budget
                .insert(dimension, limit, kind)
                .map_err(|error| CodecError::Decode(error.to_string()))?;
        }
        Ok(budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_addition_and_energy_decomposition() {
        let a = ResourceVector::new()
            .set(ResourceDimension::WallTime, 2.0)
            .unwrap()
            .set(ResourceDimension::EnergyCpu, 3.0)
            .unwrap();
        let b = ResourceVector::new()
            .set(ResourceDimension::WallTime, 4.0)
            .unwrap()
            .set(ResourceDimension::EnergyGpu, 5.0)
            .unwrap();
        let sum = a.add(&b).unwrap();
        assert_eq!(sum.value_or_zero(ResourceDimension::WallTime), 6.0);
        assert_eq!(sum.total_energy(), 8.0);
    }

    #[test]
    fn aggregate_energy_is_checked_without_double_counting() {
        let required = ResourceVector::new()
            .set(ResourceDimension::EnergyCpu, 3.0)
            .unwrap()
            .set(ResourceDimension::EnergyGpu, 2.0)
            .unwrap();
        assert_eq!(required.total_energy(), 5.0);

        let budget = BudgetSet::new()
            .with_constraint(ResourceDimension::Energy, 4.0, ConstraintKind::Hard)
            .unwrap();
        let check = budget.check(&required).unwrap();
        assert_eq!(check.hard.len(), 1);
        assert_eq!(check.hard[0].required, 5.0);
        assert_eq!(check.hard[0].deficit, 1.0);

        let explicit = ResourceVector::new()
            .set(ResourceDimension::Energy, 5.0)
            .unwrap()
            .set(ResourceDimension::EnergyCpu, 3.0)
            .unwrap();
        assert_eq!(explicit.total_energy(), 5.0);
    }

    #[test]
    fn vector_addition_reports_numeric_overflow() {
        let left = ResourceVector::new()
            .set(ResourceDimension::WallTime, f64::MAX)
            .unwrap();
        let right = ResourceVector::new()
            .set(ResourceDimension::WallTime, f64::MAX)
            .unwrap();
        assert!(matches!(
            left.add(&right),
            Err(ResourceError::Overflow {
                dimension: ResourceDimension::WallTime
            })
        ));
    }

    #[test]
    fn hard_and_soft_constraints_remain_distinct() {
        let budget = BudgetSet::new()
            .with_constraint(ResourceDimension::WallTime, 5.0, ConstraintKind::Hard)
            .unwrap()
            .with_constraint(ResourceDimension::WallTime, 6.0, ConstraintKind::Soft)
            .unwrap()
            .with_constraint(ResourceDimension::Memory, 3.0, ConstraintKind::Soft)
            .unwrap();
        let required = ResourceVector::new()
            .set(ResourceDimension::WallTime, 6.0)
            .unwrap()
            .set(ResourceDimension::Memory, 4.0)
            .unwrap();
        let result = budget.check(&required).unwrap();
        assert!(!result.feasible());
        assert_eq!(result.hard.len(), 1);
        assert_eq!(result.soft.len(), 1);
        assert_eq!(result.hard[0].deficit, 1.0);
    }

    #[test]
    fn vector_and_budget_roundtrip() {
        let vector = ResourceVector::new()
            .set(ResourceDimension::WallTime, 2.5)
            .unwrap()
            .set(ResourceDimension::MonetaryCost, 1.25)
            .unwrap();
        let budget = BudgetSet::new()
            .with_constraint(ResourceDimension::WallTime, 3.0, ConstraintKind::Hard)
            .unwrap();
        let mut ctx = RdfContext::new(
            rdf_codec::Namespace::new(RESOURCE_NS),
            rdf_codec::Namespace::new("https://example.org/data/resource/"),
        )
        .with_id_policy(rdf_codec::IdPolicy::FromContentHash);
        let node = vector.to_rdf(&mut ctx).unwrap();
        let decoded = ResourceVector::from_rdf(&node, &ctx.graph).unwrap();
        assert_eq!(vector, decoded);
        let budget_node = budget.to_rdf(&mut ctx).unwrap();
        let budget_decoded = BudgetSet::from_rdf(&budget_node, &ctx.graph).unwrap();
        assert_eq!(budget, budget_decoded);

        let reservation = ResourceReservation::new("task", 1.0, 3.0)
            .unwrap()
            .with_dimension(ResourceDimension::ComputeUnits, 2.0)
            .unwrap()
            .with_named_resource("machine", 1.0);
        let reservation_node = reservation.to_rdf(&mut ctx).unwrap();
        assert_eq!(
            reservation,
            ResourceReservation::from_rdf(&reservation_node, &ctx.graph).unwrap()
        );

        let capabilities = CapabilityRequirements {
            cpu: true,
            gpu: false,
            network: true,
            memory_bytes: 4096,
        };
        let capabilities_node = capabilities.to_rdf(&mut ctx).unwrap();
        assert_eq!(
            capabilities,
            CapabilityRequirements::from_rdf(&capabilities_node, &ctx.graph).unwrap()
        );
    }
}
