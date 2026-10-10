//! What `mm-library` refuses, and why.
//!
//! Every refusal in this crate is *typed*, because each one is a different thing
//! an operator can do something about: a shape violation names the shape, a
//! duplicate names the entry it collides with, an orphan names the reference that
//! does not resolve, and an unverified skill names its state. A single
//! `Error(String)` would make `library validate`, `library orphans` and `skill
//! verify` print the same thing.

use std::fmt;

/// One SHACL violation, as the gate reports it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShapeViolation {
    /// The shape that was violated.
    pub shape: String,
    /// What the shape required.
    pub message: String,
    /// The property path, when the validator names one.
    pub path: String,
}

impl fmt::Display for ShapeViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} ({})", self.shape, self.message, self.path)
    }
}

/// Everything this crate can refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LibraryError {
    /// A field failed its own validation.
    #[error("{field}: {detail}")]
    Validation {
        /// The field that failed.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// The entry did not satisfy its SHACL shapes.
    #[error("{entry} violates {} shape(s): {}", violations.len(), describe(violations))]
    Shape {
        /// The entry's IRI.
        entry: String,
        /// The violations, in the validator's order.
        violations: Vec<ShapeViolation>,
    },
    /// An entry with the same content already exists.
    #[error("{iri} duplicates {existing} (content hash {content_hash})")]
    Duplicate {
        /// The entry being written.
        iri: String,
        /// The existing entry with the same body.
        existing: String,
        /// The shared content hash.
        content_hash: String,
    },
    /// A referenced IRI does not resolve.
    #[error("{entry} references {missing:?}, which is not in the library")]
    Orphan {
        /// The referring entry.
        entry: String,
        /// The references that do not resolve.
        missing: Vec<String>,
    },
    /// A skill was used before its test passed.
    #[error("skill {iri} is {state}, not verified")]
    SkillNotVerified {
        /// The skill's IRI.
        iri: String,
        /// Its current verification state.
        state: &'static str,
    },
    /// A version was requested that does not exist.
    #[error("no such entry: {0}")]
    NotFound(String),
    /// The store refused a write.
    #[error("store: {0}")]
    Store(String),
    /// The graph store refused a write.
    #[error("graph: {0}")]
    Graph(String),
    /// A codec refused a document.
    #[error("codec: {0}")]
    Codec(String),
    /// The configuration is wrong.
    #[error("config: {0}")]
    Config(String),
    /// An internal invariant did not hold.
    #[error("internal: {0}")]
    Internal(String),
}

/// A short rendering of a violation list for the error message.
fn describe(violations: &[ShapeViolation]) -> String {
    violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

impl From<crate::LibraryError> for mm_core::MmError {
    fn from(e: LibraryError) -> Self {
        match e {
            LibraryError::Store(m) => mm_core::MmError::Store(m),
            LibraryError::Graph(m) => mm_core::MmError::Graph(m),
            LibraryError::Codec(m) => mm_core::MmError::Codec(m),
            LibraryError::Config(m) => mm_core::MmError::Config(m),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

impl From<mm_core::MmError> for LibraryError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => LibraryError::Store(m),
            mm_core::MmError::Graph(m) => LibraryError::Graph(m),
            mm_core::MmError::Codec(m) => LibraryError::Codec(m),
            mm_core::MmError::Config(m) => LibraryError::Config(m),
            other => LibraryError::Internal(other.to_string()),
        }
    }
}

impl LibraryError {
    /// A validation refusal.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        LibraryError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// A not-found refusal.
    pub fn not_found(iri: impl Into<String>) -> Self {
        LibraryError::NotFound(iri.into())
    }

    /// The stable code the logger records for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            LibraryError::Validation { .. } => "validation",
            LibraryError::Shape { .. } => "shape",
            LibraryError::Duplicate { .. } => "duplicate",
            LibraryError::Orphan { .. } => "orphan",
            LibraryError::SkillNotVerified { .. } => "skill_not_verified",
            LibraryError::NotFound(_) => "not_found",
            LibraryError::Store(_) => "store",
            LibraryError::Graph(_) => "graph",
            LibraryError::Codec(_) => "codec",
            LibraryError::Config(_) => "config",
            LibraryError::Internal(_) => "internal",
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, LibraryError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            LibraryError::validation("title", "empty"),
            LibraryError::Shape {
                entry: "a".into(),
                violations: vec![ShapeViolation {
                    shape: "s".into(),
                    message: "m".into(),
                    path: "p".into(),
                }],
            },
            LibraryError::Duplicate {
                iri: "a".into(),
                existing: "b".into(),
                content_hash: "c".into(),
            },
            LibraryError::Orphan {
                entry: "a".into(),
                missing: vec!["b".into()],
            },
            LibraryError::SkillNotVerified {
                iri: "a".into(),
                state: "draft",
            },
            LibraryError::not_found("a"),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
    }

    #[test]
    fn a_shape_refusal_names_the_shapes_it_violated() {
        let error = LibraryError::Shape {
            entry: "https://metamind.dev/library/technique/x".into(),
            violations: vec![
                ShapeViolation {
                    shape: "TechniqueShape".into(),
                    message: "must have at least 1 applicableWhen".into(),
                    path: "mm:applicableWhen".into(),
                },
                ShapeViolation {
                    shape: "TechniqueShape".into(),
                    message: "must have at least 1 evidence".into(),
                    path: "mm:evidence".into(),
                },
            ],
        };
        let rendered = error.to_string();
        assert!(rendered.contains("TechniqueShape"), "{rendered}");
        assert!(rendered.contains("mm:applicableWhen"), "{rendered}");
        assert_eq!(error.code(), "shape");
        assert!(
            rendered.contains("2 shape(s)"),
            "the count is part of the refusal: {rendered}"
        );
    }

    #[test]
    fn an_unverified_skill_says_which_state_it_is_in() {
        let error = LibraryError::SkillNotVerified {
            iri: "https://metamind.dev/library/skill/x@1".into(),
            state: "failing",
        };
        assert!(error.to_string().contains("failing"));
    }
}
