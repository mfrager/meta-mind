//! Index spaces and index frames (master_design.md §11.2, §14.5).

use std::collections::BTreeSet;

/// A named axis over which a quantity varies (time, asset, scenario, …).
pub type IndexDimId = String;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IndexDim {
    pub id: IndexDimId,
}

impl IndexDim {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

/// The set of index dimensions a node varies over (its "free" indices).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexSignature {
    pub free: BTreeSet<IndexDimId>,
}

impl IndexSignature {
    pub fn scalar() -> Self {
        Self::default()
    }

    pub fn over(dims: impl IntoIterator<Item = IndexDimId>) -> Self {
        Self {
            free: dims.into_iter().collect(),
        }
    }

    pub fn is_scalar(&self) -> bool {
        self.free.is_empty()
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut free = self.free.clone();
        free.extend(other.free.iter().cloned());
        Self { free }
    }

    pub fn difference(&self, other: &Self) -> Self {
        let mut free = self.free.clone();
        for d in &other.free {
            free.remove(d);
        }
        Self { free }
    }

    pub fn contains(&self, dim: &str) -> bool {
        self.free.contains(dim)
    }

    /// Broadcast: the union of free dimensions (used by `Mul`/`Div`).
    pub fn broadcast(&self, other: &Self) -> Self {
        self.union(other)
    }

    /// Contract: remove a summed-over dimension (used by tensor contractions).
    pub fn contract(&self, dim: &IndexDimId) -> Self {
        let mut free = self.free.clone();
        free.remove(dim);
        Self { free }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JoinPolicy {
    #[default]
    Inner,
    Outer,
    Left,
    ErrorOnMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingPolicy {
    DropRow,
    CarryForward,
    Interpolate,
    #[default]
    Error,
}

/// One axis of an index frame: the ordered labels along a dimension.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexColumn {
    pub values: Vec<String>,
}

impl IndexColumn {
    pub fn new(values: Vec<String>) -> Self {
        Self { values }
    }
}

/// A materialized cartesian index frame (one axis per dimension).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexFrame {
    pub dims: Vec<IndexDimId>,
    pub columns: Vec<IndexColumn>,
}

impl IndexFrame {
    pub fn new(dims: Vec<IndexDimId>, columns: Vec<IndexColumn>) -> Self {
        assert_eq!(dims.len(), columns.len(), "dims/columns length mismatch");
        Self { dims, columns }
    }
}
