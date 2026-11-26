//! `math-types` — the shared mathematical value system, dimensional algebra,
//! index-space types, and capability paths used by every other crate.

pub mod capability;
pub mod dimension;
pub mod index;
pub mod value;

pub use capability::CapabilityPath;
pub use dimension::{BaseDimension, Dimension};
pub use index::{
    IndexColumn, IndexDim, IndexDimId, IndexFrame, IndexSignature, JoinPolicy, MissingPolicy,
};
pub use value::{Series, Value};
