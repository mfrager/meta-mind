//! Agent-delegation contracts for the conversation manager.
//!
//! Phase 0 defines the seam only: a backend receives one bounded request and
//! returns one turn result. Hermes and Pi adapters, run persistence, and the
//! workspace implementation land in later phases. Keeping this crate free of
//! the console and CME avoids a dependency cycle while making the external-work
//! door explicit.

#![forbid(unsafe_code)]

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A downstream agent implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// Hermes over an OpenAI-compatible endpoint.
    Hermes,
    /// Pi over its programmatic RPC.
    Pi,
}

/// Why a delegated turn was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnMode {
    /// Start a new run.
    #[default]
    New,
    /// Continue an existing run.
    Continue,
    /// Resume after a pause or restart.
    Resume,
}

/// One bounded agent request. There is deliberately no shell, URL, or raw
/// tool-policy field: external work is reached only through the configured
/// backend and its workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRequest {
    /// Stable run identifier. A new run gets a fresh id from the caller.
    pub run_id: String,
    /// The owning conversation session.
    pub session_id: String,
    /// The authenticated user.
    pub user_id: String,
    /// The natural-language task or continuation.
    pub input: String,
    /// Which backend should handle it.
    pub kind: AgentKind,
    /// Whether this starts or continues a run.
    pub mode: AgentTurnMode,
    /// Resolved workspace root. The service sets this, not the model.
    pub workspace: PathBuf,
    /// Optional compact transcript/result context for a continuation.
    pub context: Option<String>,
    /// Wall-clock cap for this turn, in milliseconds.
    pub timeout_ms: u64,
}

/// One downstream run's durable summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRun {
    /// Run id.
    pub run_id: String,
    /// Owning session.
    pub session_id: String,
    /// Backend kind.
    pub kind: AgentKind,
    /// Current status.
    pub status: AgentRunStatus,
    /// Completed turn count.
    pub turns: u32,
    /// Start instant, Unix ms.
    pub started_at: i64,
    /// Last update instant, Unix ms.
    pub updated_at: i64,
    /// Most recent compact result.
    pub last_summary: Option<String>,
    /// Workspace root.
    pub workspace: PathBuf,
}

/// Run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    /// Accepted, not yet started.
    Queued,
    /// A turn is in flight.
    Running,
    /// Waiting for the conversation manager to continue it.
    Waiting,
    /// Completed successfully.
    Done,
    /// Failed; `AgentTurnResult::error` carries the reason.
    Failed,
    /// Cancelled by the operator or owning plan.
    Cancelled,
}

impl AgentRunStatus {
    /// Whether the run has reached a terminal state.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// The compact result of exactly one downstream turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnResult {
    /// Run this result belongs to.
    pub run_id: String,
    /// Turn status.
    pub status: AgentRunStatus,
    /// One or two sentence summary for the outer CM.
    pub summary: String,
    /// Files, diffs, commands, or other artifacts named by the backend.
    pub artifacts: Vec<AgentArtifact>,
    /// Concrete next step, when the run is waiting for continuation.
    pub next: Option<String>,
    /// Error code/reason when the turn failed.
    pub error: Option<String>,
}

impl AgentTurnResult {
    /// Whether the outer CM should call `agent.run` again for this run.
    pub fn needs_continuation(&self) -> bool {
        self.status == AgentRunStatus::Waiting && self.next.is_some()
    }
}

/// A labeled artifact returned by an agent. It is a label, not a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentArtifact {
    /// `file`, `diff`, `command`, `log`, or `url`.
    pub kind: String,
    /// Stable identifier or workspace-relative path.
    pub id: String,
    /// Optional operator-facing label.
    pub label: Option<String>,
}

/// A backend adapter. Implementations are responsible for their own transport
/// and tool policy; they must stay inside the workspace supplied in the request.
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Execute exactly one turn. It must not start a second conversational turn.
    async fn run_turn(&self, request: &AgentRequest) -> Result<AgentTurnResult>;
}

/// Resolves and confines the external-work root.
pub trait UserWorkspaceResolver: Send + Sync {
    /// The default root for a user, usually `data/workspace/<ulid>/`.
    fn resolve(&self, user_id: &str) -> Result<PathBuf>;

    /// Canonicalize `path` and prove it remains under `root`.
    fn confine(&self, root: &Path, path: &Path) -> Result<PathBuf>;
}

/// The generic agent service. Later phases add persistence and run registry
/// state around this same contract.
pub struct AgentService<B, R> {
    backend: Arc<B>,
    workspaces: Arc<R>,
}

impl<B, R> AgentService<B, R>
where
    B: AgentBackend,
    R: UserWorkspaceResolver,
{
    /// Construct the service over a backend and workspace resolver.
    pub fn new(backend: Arc<B>, workspaces: Arc<R>) -> Self {
        Self {
            backend,
            workspaces,
        }
    }

    /// Resolve the workspace, confine it, and run one turn.
    pub async fn run(&self, request: AgentRequest) -> Result<AgentTurnResult> {
        let root = self.workspaces.resolve(&request.user_id)?;
        let workspace = self.workspaces.confine(&root, &request.workspace)?;
        let request = AgentRequest {
            workspace,
            ..request
        };
        self.backend.run_turn(&request).await
    }
}

/// Recursively reject `..` and return a lexical path below a trusted root.
///
/// This is intentionally a lexical check in Phase 0. The final implementation
/// canonicalizes both paths and also rejects symlink escapes.
pub fn lexical_confine(root: &Path, path: &Path) -> Result<PathBuf> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .map_err(|_| AgentError::WorkspaceEscape {
                path: path.to_path_buf(),
                root: root.to_path_buf(),
            })?
    } else {
        path
    };
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(AgentError::WorkspaceEscape {
            path: path.to_path_buf(),
            root: root.to_path_buf(),
        });
    }
    Ok(root.join(relative))
}

/// Agent-contract failures.
#[derive(Debug, Error)]
pub enum AgentError {
    /// The workspace is outside the user's allowed root.
    #[error("workspace escape: {} is outside {}", path.display(), root.display())]
    WorkspaceEscape {
        /// Requested path.
        path: PathBuf,
        /// Trusted root.
        root: PathBuf,
    },
    /// The backend is unavailable.
    #[error("agent backend unavailable: {0}")]
    Unavailable(String),
    /// The backend rejected the request.
    #[error("agent request rejected: {0}")]
    Rejected(String),
    /// The turn exceeded its wall-clock budget.
    #[error("agent turn timed out after {0} ms")]
    Timeout(u64),
    /// Any other backend failure.
    #[error("agent: {0}")]
    Other(String),
}

/// Result alias for the agent contracts.
pub type Result<T, E = AgentError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct FakeBackend;

    #[async_trait]
    impl AgentBackend for FakeBackend {
        async fn run_turn(&self, request: &AgentRequest) -> Result<AgentTurnResult> {
            Ok(AgentTurnResult {
                run_id: request.run_id.clone(),
                status: AgentRunStatus::Done,
                summary: "done".into(),
                artifacts: Vec::new(),
                next: None,
                error: None,
            })
        }
    }

    struct Workspaces;

    impl UserWorkspaceResolver for Workspaces {
        fn resolve(&self, user_id: &str) -> Result<PathBuf> {
            Ok(PathBuf::from("/data/workspace").join(user_id))
        }

        fn confine(&self, root: &Path, path: &Path) -> Result<PathBuf> {
            lexical_confine(root, path)
        }
    }

    #[test]
    fn lexical_confinement_rejects_parent_traversal() {
        let root = Path::new("/data/workspace/u1");
        assert_eq!(
            lexical_confine(root, Path::new("src/lib.rs")).unwrap(),
            root.join("src/lib.rs")
        );
        assert!(lexical_confine(root, Path::new("../secret")).is_err());
        assert!(lexical_confine(root, Path::new("/tmp/secret")).is_err());
    }

    #[tokio::test]
    async fn service_resolves_the_workspace_before_calling_the_backend() {
        let service = AgentService::new(Arc::new(FakeBackend), Arc::new(Workspaces));
        let result = service
            .run(AgentRequest {
                run_id: "01R".into(),
                session_id: "01S".into(),
                user_id: "u1".into(),
                input: "inspect".into(),
                kind: AgentKind::Hermes,
                mode: AgentTurnMode::New,
                workspace: PathBuf::from("src"),
                context: None,
                timeout_ms: 1_000,
            })
            .await
            .unwrap();
        assert_eq!(result.status, AgentRunStatus::Done);
    }
}
