//! The shared capability-path type (master_design.md §15.3).
//!
//! This type lives in `math-types` so both `math-compiler` (which emits a
//! problem `kind`) and `math-solve` (which declares solver capabilities) can
//! use the *same* vocabulary without creating a dependency cycle.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityPath(pub Vec<String>);

impl CapabilityPath {
    pub fn new(parts: &[&str]) -> Self {
        Self(parts.iter().map(|s| s.to_string()).collect())
    }

    pub fn parts(&self) -> &[String] {
        &self.0
    }

    /// Prefix matching: a solver supporting `["solve","algebraic"]` subsumes
    /// `["solve","algebraic","linear-system"]`.
    pub fn starts_with(&self, prefix: &CapabilityPath) -> bool {
        self.0.len() >= prefix.0.len() && self.0[..prefix.0.len()] == prefix.0[..]
    }

    pub fn join(&self, part: &str) -> Self {
        let mut v = self.0.clone();
        v.push(part.to_string());
        Self(v)
    }
}

impl fmt::Display for CapabilityPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join("."))
    }
}
