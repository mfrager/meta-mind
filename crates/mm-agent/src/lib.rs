//! Agent-delegation contracts and the Hermes runtime for the conversation manager.
//!
//! Phase 6 adds the concrete agent service around the Phase 0 seam: a
//! turn-by-turn registry, a separate agentstream per run, per-turn
//! summarization, and an OpenAI-compatible Hermes backend with SSE and
//! non-streaming fallback. Keeping the transport here rather than in the
//! console keeps external work behind one door.

#![forbid(unsafe_code)]

pub mod agentstream;
pub mod hermes;
pub mod pi;
pub mod registry;
pub mod summarizer;

pub use hermes::{HermesBackend, HermesConfig, HermesProbe};
pub use pi::{PiBackend, PiProbe, PiSettings};
pub use registry::{AgentBackendStatus, AgentRuntime, FilesystemWorkspaces};
pub use summarizer::TurnSummarizer;

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

/// Who asked for the external work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentOrigin {
    /// The conversation manager.
    #[default]
    Cm,
    /// The plan monitor.
    PlanMonitor,
    /// A registered internal action.
    Action,
    /// Session automation.
    Automation,
}

impl AgentOrigin {
    /// A stable operator-facing label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cm => "conversation manager",
            Self::PlanMonitor => "plan monitor",
            Self::Action => "internal action",
            Self::Automation => "automation",
        }
    }
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
    /// Replayed conversation messages, oldest first.
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    /// Wall-clock cap for this turn, in milliseconds.
    pub timeout_ms: u64,
}

/// One replayed downstream message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    /// `user` or `assistant`.
    pub role: String,
    /// Message text.
    pub content: String,
}

/// One downstream run's durable summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRun {
    /// Run id.
    pub run_id: String,
    /// Owning conversation session.
    pub session_id: String,
    /// Isolated agentstream session (`agent/<run_id>`).
    pub agentstream_id: String,
    /// Backend kind.
    pub kind: AgentKind,
    /// Current status.
    pub status: AgentRunStatus,
    /// Completed turn count.
    pub turns: u32,
    /// The original natural-language task.
    pub task: String,
    /// Origin of the request.
    pub origin: AgentOrigin,
    /// Start instant, Unix ms.
    pub started_at: i64,
    /// Last update instant, Unix ms.
    pub updated_at: i64,
    /// Most recent compact result.
    pub last_summary: Option<String>,
    /// Most recent error.
    pub last_error: Option<String>,
    /// Aggregate token usage.
    pub usage: AgentUsage,
    /// Resolved workspace root.
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
    /// The downstream agent asked the operator a question.
    WaitingInput,
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

    /// Stable wire label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::WaitingInput => "waiting_input",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Token usage for one turn or a whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentUsage {
    /// Prompt/input tokens.
    pub prompt_tokens: u64,
    /// Completion/output tokens.
    pub completion_tokens: u64,
}

impl AgentUsage {
    /// Add another turn's usage.
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            prompt_tokens: self.prompt_tokens.saturating_add(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_add(other.completion_tokens),
        }
    }
}

/// A tool call the downstream agent reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolSummary {
    /// Tool name.
    pub name: String,
    /// Optional argument excerpt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Compact per-turn summary shown to the outer conversation manager.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnSummary {
    /// One-line headline, at most 200 characters.
    pub headline: String,
    /// Concrete actions taken.
    #[serde(default)]
    pub did: Vec<String>,
    /// Facts the manager needs to decide the next step.
    #[serde(default)]
    pub learned: Vec<String>,
    /// Artifacts named by the turn.
    #[serde(default)]
    pub artifacts: Vec<AgentArtifact>,
    /// Questions or failures that block progress.
    #[serde(default)]
    pub blockers: Vec<String>,
    /// Recommended next action.
    #[serde(default)]
    pub next_recommended: Option<String>,
    /// Confidence in the summary, 0..1.
    #[serde(default)]
    pub confidence: f32,
    /// True when the deterministic extractor replaced a failed model summary.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub summary_failed: bool,
}

/// The compact result of exactly one downstream turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTurnResult {
    /// Run this result belongs to.
    pub run_id: String,
    /// One-based turn number.
    pub turn: u32,
    /// Turn status.
    pub status: AgentRunStatus,
    /// Resolved workspace root.
    pub workspace: PathBuf,
    /// The agent's message for this turn, possibly summarized.
    pub message: String,
    /// One or two sentence compact result for the outer CM.
    pub summary: String,
    /// Structured summary, when the turn was long enough to summarize.
    pub summary_detail: Option<AgentTurnSummary>,
    /// Tool calls the downstream agent reported.
    pub tool_calls: Vec<AgentToolSummary>,
    /// Files, diffs, commands, or other artifacts named by the backend.
    pub artifacts: Vec<AgentArtifact>,
    /// Concrete next step, when the run is waiting for continuation.
    pub next: Option<String>,
    /// Error code/reason when the turn failed.
    pub error: Option<String>,
    /// Token usage for this turn.
    pub usage: AgentUsage,
    /// Whether the backend used SSE.
    pub streamed: bool,
    /// Model that answered.
    pub model: Option<String>,
    /// Agentstream pointer to the retained raw turn.
    pub raw_ref: Option<String>,
}

impl AgentTurnResult {
    /// A minimal result for a turn that has not started.
    pub fn running(run_id: &str) -> Self {
        Self {
            run_id: run_id.into(),
            turn: 0,
            status: AgentRunStatus::Running,
            workspace: PathBuf::new(),
            message: String::new(),
            summary: String::new(),
            summary_detail: None,
            tool_calls: Vec::new(),
            artifacts: Vec::new(),
            next: Some("continue".into()),
            error: None,
            usage: AgentUsage::default(),
            streamed: false,
            model: None,
            raw_ref: None,
        }
    }

    /// Whether the outer CM should call `agent.run` again for this run.
    pub fn needs_continuation(&self) -> bool {
        matches!(
            self.status,
            AgentRunStatus::Waiting | AgentRunStatus::WaitingInput
        ) && self.next.is_some()
    }
}

/// One call to the external-work door.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCall {
    /// Natural-language task or continuation.
    pub input: String,
    /// Continue this run; `None` starts a new run.
    pub run_id: Option<String>,
    /// Backend to use.
    pub kind: AgentKind,
    /// Optional workspace beneath the user's root.
    pub workspace: Option<PathBuf>,
    /// Owning conversation session.
    pub parent_session: String,
    /// Authenticated user.
    pub user_id: String,
    /// Who requested the work.
    pub origin: AgentOrigin,
}

/// The only external-work port available to plans, actions, and automation.
#[async_trait]
pub trait ExternalWorkPort: Send + Sync {
    /// Start or continue one bounded turn.
    async fn run(&self, call: AgentCall) -> Result<AgentTurnResult>;

    /// Continue an existing run.
    async fn continue_run(
        &self,
        run_id: &str,
        input: &str,
        parent_session: &str,
        user_id: &str,
    ) -> Result<AgentTurnResult>;

    /// Current durable run state, when the run is known.
    async fn status(&self, run_id: &str) -> Result<Option<AgentRun>>;
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
    /// A run id is unknown.
    #[error("unknown agent run {run_id}")]
    NotFound {
        /// Requested run.
        run_id: String,
        /// Currently active runs, for the error's `next` hint.
        active: Vec<String>,
    },
    /// A terminal run cannot be continued.
    #[error("agent run is already terminal: {0:?}")]
    RunTerminal(Box<AgentRun>),
    /// A per-session or per-run budget was exhausted.
    #[error("agent budget exhausted: {0}")]
    Budget(String),
    /// A required prompt strategy or template is unavailable.
    #[error("agent prompt unavailable: {0}")]
    Prompt(String),
    /// Any other backend failure.
    #[error("agent: {0}")]
    Other(String),
}

/// Result alias for the agent contracts.
pub type Result<T, E = AgentError> = std::result::Result<T, E>;

impl From<std::io::Error> for AgentError {
    fn from(error: std::io::Error) -> Self {
        AgentError::Other(error.to_string())
    }
}

impl From<mm_data::DataError> for AgentError {
    fn from(error: mm_data::DataError) -> Self {
        AgentError::Other(error.to_string())
    }
}

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
                turn: 1,
                status: AgentRunStatus::Done,
                workspace: request.workspace.clone(),
                message: "done".into(),
                summary: "done".into(),
                summary_detail: None,
                tool_calls: Vec::new(),
                artifacts: Vec::new(),
                next: None,
                error: None,
                usage: AgentUsage::default(),
                streamed: false,
                model: None,
                raw_ref: None,
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
                messages: Vec::new(),
                timeout_ms: 1_000,
            })
            .await
            .unwrap();
        assert_eq!(result.status, AgentRunStatus::Done);
    }
}
