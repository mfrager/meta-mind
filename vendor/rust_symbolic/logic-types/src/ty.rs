//! The semantic type system (`full_system_design.md` §5).
//!
//! `Type` is deliberately *semantic*, not merely syntactic: `Bool` ≠ `Prop`,
//! `Probability` ≠ `TruthDegree`, and a `Relation` application is type-checked
//! against its declared argument domains.

use std::fmt;

use crate::unit::Unit;
use math_types::{Dimension, IndexDimId};

/// The semantic type universe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    // logical
    Bool,
    Prop,
    // numeric
    Int,
    Nat,
    Rational,
    Real,
    Decimal,
    Float,
    BitVec,
    Complex,
    // scalar semantic
    String,
    Symbol,
    Entity(String),
    // containers
    Set(Box<Type>),
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    Tensor {
        indices: Vec<IndexDimId>,
        scalar: Box<Type>,
    },
    // temporal
    Time,
    Instant,
    Duration,
    // semantic-value types (deliberately distinct)
    Probability,
    TruthDegree,
    // contexts
    World,
    Agent,
    Action,
    Event,
    State,
    // functions / relations
    Function {
        args: Vec<Type>,
        result: Box<Type>,
    },
    Relation {
        args: Vec<Type>,
    },
    // uncertainty wrappers
    Interval(Box<Type>),
    RandomVariable(Box<Type>),
    Distribution(Box<Type>),
}

impl Type {
    pub fn entity(name: impl Into<String>) -> Self {
        Type::Entity(name.into())
    }

    pub fn tensor(indices: &[&str], scalar: Type) -> Self {
        Type::Tensor {
            indices: indices.iter().map(|s| s.to_string()).collect(),
            scalar: Box::new(scalar),
        }
    }

    /// A bare subtype check. For now this is exact equality; entity-hierarchy
    /// subtyping arrives with the profile/typecheck phase.
    pub fn accepts(&self, other: &Type) -> bool {
        self == other
    }

    /// Apply a `Relation` (→ `Prop`) or a `Function` (→ its result) to
    /// arguments, type-checking each argument against the declared domain.
    pub fn apply(&self, args: &[Type]) -> Result<Type, TypeError> {
        let params = match self {
            Type::Relation { args } => args,
            Type::Function { args, .. } => args,
            other => return Err(TypeError::NotApplicable(other.clone())),
        };
        if params.len() != args.len() {
            return Err(TypeError::Arity {
                expected: params.len(),
                actual: args.len(),
            });
        }
        for (position, (param, arg)) in params.iter().zip(args).enumerate() {
            if !param.accepts(arg) {
                return Err(TypeError::DomainMismatch {
                    position,
                    expected: param.clone(),
                    actual: arg.clone(),
                });
            }
        }
        match self {
            Type::Relation { .. } => Ok(Type::Prop),
            Type::Function { result, .. } => Ok((**result).clone()),
            _ => unreachable!("params matched only Relation/Function"),
        }
    }

    /// Remove a dimension from a tensor (Einstein-style contraction). Errors
    /// unless the dimension is present — indices are typed, so a tensor over
    /// `{Country, Year}` cannot contract `Employee`.
    pub fn contract(&self, dim: &str) -> Result<Type, TypeError> {
        match self {
            Type::Tensor { indices, scalar } => {
                let pos = indices.iter().position(|d| d == dim).ok_or_else(|| {
                    TypeError::MissingIndexDim {
                        dim: dim.to_string(),
                    }
                })?;
                let mut out = indices.clone();
                out.remove(pos);
                Ok(Type::Tensor {
                    indices: out,
                    scalar: scalar.clone(),
                })
            }
            other => Err(TypeError::NotTensor(other.clone())),
        }
    }

    /// Contract `self` and `other` over a *shared* dimension: both must carry
    /// it, so mismatched index domains are rejected.
    pub fn contract_with(&self, other: &Type, dim: &str) -> Result<(Type, Type), TypeError> {
        let left = self.contract(dim)?;
        let right = other.contract(dim)?;
        Ok((left, right))
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Bool => write!(f, "Bool"),
            Type::Prop => write!(f, "Prop"),
            Type::Int => write!(f, "Int"),
            Type::Nat => write!(f, "Nat"),
            Type::Rational => write!(f, "Rational"),
            Type::Real => write!(f, "Real"),
            Type::Decimal => write!(f, "Decimal"),
            Type::Float => write!(f, "Float"),
            Type::BitVec => write!(f, "BitVec"),
            Type::Complex => write!(f, "Complex"),
            Type::String => write!(f, "String"),
            Type::Symbol => write!(f, "Symbol"),
            Type::Entity(n) => write!(f, "{n}"),
            Type::Set(t) => write!(f, "Set({t})"),
            Type::List(t) => write!(f, "List({t})"),
            Type::Map(k, v) => write!(f, "Map({k}, {v})"),
            Type::Tuple(ts) => write!(f, "({})", join(ts)),
            Type::Tensor { indices, scalar } => write!(f, "{scalar}[{}]", indices.join(", ")),
            Type::Time => write!(f, "Time"),
            Type::Instant => write!(f, "Instant"),
            Type::Duration => write!(f, "Duration"),
            Type::Probability => write!(f, "Probability"),
            Type::TruthDegree => write!(f, "TruthDegree"),
            Type::World => write!(f, "World"),
            Type::Agent => write!(f, "Agent"),
            Type::Action => write!(f, "Action"),
            Type::Event => write!(f, "Event"),
            Type::State => write!(f, "State"),
            Type::Function { args, result } => write!(f, "({}) -> {result}", join(args)),
            Type::Relation { args } => write!(f, "Relation({})", join(args)),
            Type::Interval(t) => write!(f, "Interval({t})"),
            Type::RandomVariable(t) => write!(f, "RV({t})"),
            Type::Distribution(t) => write!(f, "Dist({t})"),
        }
    }
}

fn join(ts: &[Type]) -> String {
    ts.iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A value carrying dimensional/unit metadata, used to reject dimensional
/// nonsense (`USD + percent`) before execution (`full_system_design.md` §5.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Quantity {
    pub ty: Type,
    pub dimension: Dimension,
    pub unit: Option<Unit>,
}

impl Quantity {
    pub fn new(ty: Type, dimension: Dimension, unit: Option<Unit>) -> Self {
        Self {
            ty,
            dimension,
            unit,
        }
    }

    /// Addition is only legal between dimensionally- and unit-consistent
    /// quantities.
    pub fn add(&self, other: &Quantity) -> Result<Quantity, TypeError> {
        if self.dimension != other.dimension {
            return Err(TypeError::DimensionMismatch {
                expected: self.dimension.clone(),
                actual: other.dimension.clone(),
            });
        }
        if self.unit != other.unit {
            return Err(TypeError::UnitMismatch {
                expected: self.unit.clone(),
                actual: other.unit.clone(),
            });
        }
        Ok(self.clone())
    }
}

/// Semantic type errors.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    Arity {
        expected: usize,
        actual: usize,
    },
    DomainMismatch {
        position: usize,
        expected: Type,
        actual: Type,
    },
    NotApplicable(Type),
    NotTensor(Type),
    MissingIndexDim {
        dim: String,
    },
    DimensionMismatch {
        expected: Dimension,
        actual: Dimension,
    },
    UnitMismatch {
        expected: Option<Unit>,
        actual: Option<Unit>,
    },
}
