//! What the sanity firewall refuses, and why.
//!
//! The refusals are typed because each one is something different an operator
//! acts on. A *validation* refusal names the field of the input that made no
//! sense; a *prohibition* refusal names a hard prohibition that fired, and it is
//! the one refusal that is also a legitimate — indeed the intended — result of an
//! evaluation; a *scan* refusal is the bounded-judgment layer failing to produce a
//! judgment at all; *store* and *codec* are the persistence boundary; and
//! *internal* is a bug here.
//!
//! A refusal is deliberately **not** how a hard prohibition is reported. A
//! prohibition firing is the firewall working: it is an `Ok(FirewallReport)` whose
//! `hard_prohibition` is `Some`. `FirewallError::Prohibition` exists only for the
//! case where a *caller* asserts a prohibition it expected to be evaluated and the
//! input cannot support that assertion — never as the ordinary path, because a
//! report that is an `Err` cannot be persisted, graded, or compared with the gold
//! corpus.

/// Everything this crate can refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FirewallError {
    /// A field of the input, or of the configuration, failed its own validation.
    #[error("{field}: {detail}")]
    Validation {
        /// The field that failed.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// A caller asserted a prohibition that the evaluated input cannot support.
    #[error("prohibition: {0}")]
    Prohibition(String),
    /// The bounded-judgment layer could not run.
    #[error("scan: {0}")]
    Scan(String),
    /// The store refused a write or a read.
    #[error("store: {0}")]
    Store(String),
    /// A codec refused a document.
    #[error("codec: {0}")]
    Codec(String),
    /// A bug in this crate.
    #[error("internal: {0}")]
    Internal(String),
}

impl FirewallError {
    /// A validation refusal, with the field named.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        FirewallError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// The stable code an operator filters on.
    pub fn code(&self) -> &'static str {
        match self {
            FirewallError::Validation { .. } => "validation",
            FirewallError::Prohibition(_) => "prohibition",
            FirewallError::Scan(_) => "scan",
            FirewallError::Store(_) => "store",
            FirewallError::Codec(_) => "codec",
            FirewallError::Internal(_) => "internal",
        }
    }
}

impl From<mm_core::MmError> for FirewallError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(message) => FirewallError::Store(message),
            mm_core::MmError::Codec(message) => FirewallError::Codec(message),
            other => FirewallError::Internal(other.to_string()),
        }
    }
}

/// A refusal from the decision layer is this crate's refusal, keeping its code.
///
/// The code is appended rather than lost because it is what tells an operator
/// whether a decision refusal came from validation, from a schema violation, or
/// from a core being unavailable — and those three call for different actions.
impl From<mm_decision::DecisionError> for FirewallError {
    fn from(e: mm_decision::DecisionError) -> Self {
        match e {
            mm_decision::DecisionError::Store(message) => FirewallError::Store(message),
            mm_decision::DecisionError::Codec(message) => FirewallError::Codec(message),
            other => FirewallError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The kernel's error type, so a command can `?` through this crate.
impl From<FirewallError> for mm_core::MmError {
    fn from(e: FirewallError) -> Self {
        match e {
            FirewallError::Store(message) => mm_core::MmError::Store(message),
            FirewallError::Codec(message) => mm_core::MmError::Codec(message),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, FirewallError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            FirewallError::validation("stakes", "out of range"),
            FirewallError::Prohibition("identity_invariant".into()),
            FirewallError::Scan("no core".into()),
            FirewallError::Store("closed".into()),
            FirewallError::Codec("bad json".into()),
            FirewallError::Internal("bug".into()),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
    }

    #[test]
    fn a_decision_refusal_keeps_its_code() {
        let e: FirewallError = mm_decision::DecisionError::Schema("bad shape".into()).into();
        assert_eq!(e.code(), "internal");
        assert!(e.to_string().contains("schema"), "{e}");
    }
}
