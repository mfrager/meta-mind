//! `metamind.memory` service contracts.
//!
//! The existing four-tier [`crate::memory::MemorySystem`] remains the storage
//! implementation. These types are the narrow tool-facing seam: recall,
//! remember, forget, and status. They do not add a second memory store.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum recall results.
pub const MAX_RECALL: usize = 50;
/// Maximum remembered value length.
pub const MAX_MEMORY_VALUE: usize = 8_192;

/// Memory scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Current session.
    Session,
    /// Current project.
    Project,
    /// User-global memory.
    User,
    /// Cross-project/global learned memory.
    Global,
}

/// Memory record kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Stable fact.
    Fact,
    /// User preference.
    Preference,
    /// Decision.
    Decision,
    /// Commitment.
    Commitment,
    /// Artifact.
    Artifact,
    /// Episodic observation.
    Episode,
}

/// One memory record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    /// Stable id.
    pub id: String,
    /// Scope.
    pub scope: MemoryScope,
    /// Kind.
    pub kind: MemoryKind,
    /// Stable lookup key/topic.
    pub key: String,
    /// Value text.
    pub value: String,
    /// Provenance label.
    pub source: String,
    /// Confidence 0.0–1.0.
    pub confidence: f32,
    /// Creation instant.
    pub created_at: i64,
    /// Update instant.
    pub updated_at: i64,
    /// Optional expiry.
    pub expires_at: Option<i64>,
}

impl MemoryRecord {
    /// Validate value bounds and confidence.
    pub fn validate(&self) -> Result<(), MemoryContractError> {
        if self.id.trim().is_empty() || self.key.trim().is_empty() {
            return Err(MemoryContractError::Invalid(
                "memory id and key must not be empty".into(),
            ));
        }
        if self.value.chars().count() > MAX_MEMORY_VALUE {
            return Err(MemoryContractError::Invalid(
                "memory value exceeds its limit".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence) || !self.confidence.is_finite() {
            return Err(MemoryContractError::Invalid(
                "memory confidence must be between 0 and 1".into(),
            ));
        }
        Ok(())
    }
}

/// Recall query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallQuery {
    /// Free-text query.
    pub query: String,
    /// Scopes to search.
    pub scopes: Vec<MemoryScope>,
    /// Kinds to include; empty means all.
    #[serde(default)]
    pub kinds: Vec<MemoryKind>,
    /// Maximum results.
    pub limit: usize,
}

impl RecallQuery {
    /// Validate the query.
    pub fn validate(&self) -> Result<(), MemoryContractError> {
        if self.query.trim().is_empty() {
            return Err(MemoryContractError::Invalid(
                "recall query must not be empty".into(),
            ));
        }
        if self.limit == 0 || self.limit > MAX_RECALL {
            return Err(MemoryContractError::Invalid(format!(
                "recall limit must be 1–{MAX_RECALL}"
            )));
        }
        if self.scopes.is_empty() {
            return Err(MemoryContractError::Invalid(
                "recall must name at least one scope".into(),
            ));
        }
        Ok(())
    }
}

/// Recall result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallResult {
    /// Records.
    pub records: Vec<MemoryRecord>,
    /// Whether the result was truncated by the cap.
    pub truncated: bool,
}

/// Remember request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RememberRequest {
    /// Scope.
    pub scope: MemoryScope,
    /// Kind.
    pub kind: MemoryKind,
    /// Stable key/topic.
    pub key: String,
    /// Value.
    pub value: String,
    /// Provenance label.
    pub source: String,
    /// Optional confidence.
    pub confidence: Option<f32>,
    /// Optional expiry.
    pub expires_at: Option<i64>,
}

impl RememberRequest {
    /// Validate the request.
    pub fn validate(&self) -> Result<(), MemoryContractError> {
        if self.key.trim().is_empty() || self.value.trim().is_empty() {
            return Err(MemoryContractError::Invalid(
                "memory key and value must not be empty".into(),
            ));
        }
        if self.value.chars().count() > MAX_MEMORY_VALUE {
            return Err(MemoryContractError::Invalid(
                "memory value exceeds its limit".into(),
            ));
        }
        if self
            .confidence
            .is_some_and(|confidence| !(0.0..=1.0).contains(&confidence) || !confidence.is_finite())
        {
            return Err(MemoryContractError::Invalid(
                "confidence must be between 0 and 1".into(),
            ));
        }
        Ok(())
    }
}

/// Forget selector. Exactly one selector is required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum ForgetSelector {
    /// One record id.
    Id {
        /// Record id.
        id: String,
    },
    /// Every record with a key in a scope.
    Key {
        /// Scope.
        scope: MemoryScope,
        /// Key.
        key: String,
    },
}

/// Forget request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetRequest {
    /// Selector.
    pub selector: ForgetSelector,
    /// Audit reason.
    pub reason: String,
}

/// Status contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStatus {
    /// Total active records.
    pub records: u64,
    /// Records by scope.
    pub by_scope: Vec<MemoryScopeCount>,
    /// Last write instant.
    pub last_updated_at: Option<i64>,
    /// Storage backend label.
    pub backend: String,
}

/// Count per scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryScopeCount {
    /// Scope.
    pub scope: MemoryScope,
    /// Active record count.
    pub count: u64,
}

/// Tool-facing memory service.
#[async_trait]
pub trait MemoryService: Send + Sync {
    /// Recall relevant records.
    async fn recall(&self, query: RecallQuery) -> Result<RecallResult, MemoryContractError>;

    /// Remember one record, upserting by scope/key/kind.
    async fn remember(&self, request: RememberRequest)
        -> Result<MemoryRecord, MemoryContractError>;

    /// Forget by id or scope/key.
    async fn forget(&self, request: ForgetRequest) -> Result<u64, MemoryContractError>;

    /// Current status.
    async fn status(&self) -> Result<MemoryStatus, MemoryContractError>;
}

/// Memory contract failures.
#[derive(Debug, Error)]
pub enum MemoryContractError {
    /// Invalid arguments.
    #[error("invalid memory request: {0}")]
    Invalid(String),
    /// Record missing.
    #[error("memory record not found: {0}")]
    NotFound(String),
    /// Store unavailable.
    #[error("memory store unavailable: {0}")]
    Unavailable(String),
}

/// Result alias.
pub type Result<T, E = MemoryContractError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_requires_scope_and_bounded_limit() {
        let mut query = RecallQuery {
            query: "auth".into(),
            scopes: vec![MemoryScope::Project],
            kinds: Vec::new(),
            limit: 10,
        };
        assert!(query.validate().is_ok());
        query.scopes.clear();
        assert!(query.validate().is_err());
        query.scopes.push(MemoryScope::User);
        query.limit = 51;
        assert!(query.validate().is_err());
    }

    #[test]
    fn confidence_is_finite_and_bounded() {
        let record = MemoryRecord {
            id: "m".into(),
            scope: MemoryScope::User,
            kind: MemoryKind::Preference,
            key: "tone".into(),
            value: "direct".into(),
            source: "user".into(),
            confidence: f32::NAN,
            created_at: 1,
            updated_at: 1,
            expires_at: None,
        };
        assert!(record.validate().is_err());
    }
}
