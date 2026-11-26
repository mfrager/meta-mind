//! The mathematical value system (master_design.md §7).

use std::fmt;

/// A 1-D series of values carrying its index labels, so alignment and lag/lead
/// are well-defined at runtime (never implicit).
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub values: Vec<f64>,
    pub labels: Vec<String>,
}

impl Series {
    pub fn new(values: Vec<f64>, labels: Vec<String>) -> Self {
        assert_eq!(
            values.len(),
            labels.len(),
            "Series values/labels length mismatch"
        );
        Self { values, labels }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// A mathematical value. RDF describes the *type and semantics*; this enum is
/// the transient machine representation used by the runtime and IR constants.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Scalar(f64),
    Integer(i64),
    Boolean(bool),
    Vector(Vec<f64>),
    Matrix(Vec<Vec<f64>>),
    Series(Series),
    Symbol(String),
}

impl Value {
    /// Coerce to `f64` where that is meaningful.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Scalar(x) => Some(*x),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Scalar(x) => write!(f, "{x}"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Vector(v) => write!(f, "{v:?}"),
            Value::Matrix(m) => write!(f, "{m:?}"),
            Value::Series(s) => write!(f, "series{:?}", s.values),
            Value::Symbol(s) => write!(f, "{s}"),
        }
    }
}
