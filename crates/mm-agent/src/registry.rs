//! Run registry and turn loop for downstream agents.
//!
//! The registry owns live run metadata, enforces one in-flight turn per run,
//! isolates the agentstream per run, and reconstructs backend messages from the
//! raw trace rather than trusting a server-side session.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use mm_data::agentstream::AgentstreamStore;
use mm_data::rdf::new_id;
use mm_prompt::PromptStore;
use tokio::sync::Mutex;

use crate::agentstream::{self, ReplayMessage};
use crate::hermes::HermesBackend;
use crate::pi::PiBackend;
use crate::summarizer::TurnSummarizer;
use crate::{
    lexical_confine, AgentCall, AgentError, AgentKind, AgentMessage, AgentRequest, AgentRun,
    AgentRunStatus, AgentTurnMode, AgentTurnResult, AgentUsage, ExternalWorkPort, Result,
    UserWorkspaceResolver,
};

/// Useful health data for the status panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBackendStatus {
    /// Backend kind.
    pub kind: AgentKind,
    /// Whether the backend can accept work.
    pub available: bool,
    /// Redacted reason when unavailable.
    pub reason: Option<String>,
    /// Models advertised by the backend.
    pub models: Vec<String>,
}

/// The filesystem workspace resolver.
#[derive(Debug, Clone)]
pub struct FilesystemWorkspaces {
    base_dir: PathBuf,
    allowed_roots: Vec<PathBuf>,
}

impl FilesystemWorkspaces {
    /// Build under the data directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let allowed_roots = std::env::var("MM_AGENT_ALLOWED_ROOTS")
            .ok()
            .map(|roots| {
                roots
                    .split(':')
                    .filter(|root| !root.trim().is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            base_dir: base_dir.into(),
            allowed_roots,
        }
    }

    /// Resolve the root actually used by a call.
    pub fn resolve_call(&self, user_id: &str, requested: Option<&Path>) -> Result<PathBuf> {
        let root = self.resolve(user_id)?;
        let Some(requested) = requested else {
            return self.confine(&root, &root);
        };
        if requested.is_absolute() {
            let allowed = self
                .allowed_roots
                .iter()
                .any(|allowed| requested.starts_with(allowed));
            if !allowed {
                return Err(AgentError::WorkspaceEscape {
                    path: requested.to_path_buf(),
                    root,
                });
            }
            return self.confine(requested, requested);
        }
        self.confine(&root, requested)
    }
}

impl UserWorkspaceResolver for FilesystemWorkspaces {
    fn resolve(&self, user_id: &str) -> Result<PathBuf> {
        let safe = user_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>();
        let safe = if safe.is_empty() {
            "operator".to_owned()
        } else {
            safe
        };
        let root = self.base_dir.join("workspace").join(safe);
        std::fs::create_dir_all(&root)?;
        Ok(root)
    }

    fn confine(&self, root: &Path, path: &Path) -> Result<PathBuf> {
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
        let joined = lexical_confine(root, relative)?;
        std::fs::create_dir_all(&joined)?;
        let canonical_root = std::fs::canonicalize(root)
            .map_err(|error| AgentError::Other(format!("workspace root: {error}")))?;
        let canonical = std::fs::canonicalize(&joined)
            .map_err(|error| AgentError::Other(format!("workspace path: {error}")))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(AgentError::WorkspaceEscape {
                path: joined,
                root: root.to_path_buf(),
            });
        }
        Ok(canonical)
    }
}

struct RunState {
    record: AgentRun,
    in_flight: bool,
}

/// The live agent runtime.
pub struct AgentRuntime {
    store: Arc<AgentstreamStore>,
    prompts: Arc<PromptStore>,
    workspaces: FilesystemWorkspaces,
    hermes: Option<Arc<HermesBackend>>,
    pi: Option<Arc<PiBackend>>,
    summarizer: TurnSummarizer,
    runs: Mutex<HashMap<String, RunState>>,
    runs_path: PathBuf,
    max_active_runs: usize,
}

impl std::fmt::Debug for AgentRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentRuntime")
            .field("hermes", &self.hermes.as_ref().map(|hermes| hermes.probe()))
            .field("pi", &self.pi.as_ref().map(|pi| pi.probe()))
            .field("max_active_runs", &self.max_active_runs)
            .finish_non_exhaustive()
    }
}

impl AgentRuntime {
    /// Build a runtime over explicitly supplied parts.
    pub fn new(
        store: Arc<AgentstreamStore>,
        prompts: Arc<PromptStore>,
        base_dir: impl Into<PathBuf>,
        hermes: Option<Arc<HermesBackend>>,
        summarizer: TurnSummarizer,
    ) -> Self {
        let base_dir = base_dir.into();
        let runs_path = base_dir.join("agent-runs.json");
        let recovered = load_runs(&runs_path);
        let _ = write_runs(&runs_path, &recovered);
        Self {
            store,
            prompts,
            workspaces: FilesystemWorkspaces::new(base_dir),
            hermes,
            pi: None,
            summarizer,
            runs: Mutex::new(
                recovered
                    .into_iter()
                    .map(|record| {
                        (
                            record.run_id.clone(),
                            RunState {
                                record,
                                in_flight: false,
                            },
                        )
                    })
                    .collect(),
            ),
            runs_path,
            max_active_runs: 3,
        }
    }

    /// Resolve Hermes from the environment and probe it.
    pub async fn from_env(
        store: Arc<AgentstreamStore>,
        prompts: Arc<PromptStore>,
        base_dir: impl Into<PathBuf>,
        summarizer: TurnSummarizer,
        config_file: Option<&Path>,
    ) -> Self {
        let hermes = HermesBackend::from_config_file(config_file);
        hermes.refresh().await;
        let pi = PiBackend::from_env();
        pi.refresh();
        let runtime = Self::new(store, prompts, base_dir, Some(Arc::new(hermes)), summarizer)
            .with_pi(Arc::new(pi));
        runtime.recover().await;
        runtime
    }

    /// Log the recovery status of runs left non-terminal by a previous process.
    pub async fn recover(&self) {
        let recovered: Vec<AgentRun> = {
            let runs = self.runs.lock().await;
            runs.values()
                .filter(|state| {
                    state.record.status == AgentRunStatus::Failed
                        && state.record.last_error.as_deref() == Some("server restarted")
                })
                .map(|state| state.record.clone())
                .collect()
        };
        for record in recovered {
            let _ = agentstream::log_status(
                &self.store,
                &record.agentstream_id,
                &record.run_id,
                AgentRunStatus::Failed,
                record.turns,
                None,
                Some("server restarted"),
            )
            .await;
            let _ = self.store.snapshot_session(&record.agentstream_id).await;
        }
    }

    /// Attach the Pi backend. Tests and Phase 7 callers use this.
    #[must_use]
    pub fn with_pi(mut self, pi: Arc<PiBackend>) -> Self {
        self.pi = Some(pi);
        self
    }

    /// Override the per-session active-run cap. Tests use this.
    #[must_use]
    pub fn with_max_active_runs(mut self, cap: usize) -> Self {
        self.max_active_runs = cap.max(1);
        self
    }

    /// The workspace resolver.
    pub fn workspaces(&self) -> &FilesystemWorkspaces {
        &self.workspaces
    }

    /// One run record.
    pub async fn status(&self, run_id: &str) -> Result<AgentRun> {
        let runs = self.runs.lock().await;
        runs.get(run_id)
            .map(|state| state.record.clone())
            .ok_or_else(|| AgentError::NotFound {
                run_id: run_id.to_owned(),
                active: active_ids(&runs),
            })
    }

    /// Every known run, newest first.
    pub async fn runs(&self) -> Vec<AgentRun> {
        let runs = self.runs.lock().await;
        let mut records: Vec<_> = runs.values().map(|state| state.record.clone()).collect();
        records.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
        records
    }

    /// Runs owned by a conversation session, newest first.
    pub async fn runs_for_session(&self, session_id: &str) -> Vec<AgentRun> {
        let runs = self.runs.lock().await;
        let mut records: Vec<_> = runs
            .values()
            .filter(|state| state.record.session_id == session_id)
            .map(|state| state.record.clone())
            .collect();
        records.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
        records
    }

    /// Cancel a run. The next completed turn observes the cancelled status.
    pub async fn cancel(&self, run_id: &str) -> Result<AgentRun> {
        let (record, session) = {
            let mut runs = self.runs.lock().await;
            let Some(state) = runs.get_mut(run_id) else {
                return Err(AgentError::NotFound {
                    run_id: run_id.to_owned(),
                    active: active_ids(&runs),
                });
            };
            if state.record.status.is_terminal() {
                return Ok(state.record.clone());
            }
            state.record.status = AgentRunStatus::Cancelled;
            state.record.updated_at = mm_data::rdf::now_ms();
            state.in_flight = false;
            (state.record.clone(), state.record.agentstream_id.clone())
        };
        agentstream::log_status(
            &self.store,
            &session,
            run_id,
            AgentRunStatus::Cancelled,
            record.turns,
            None,
            None,
        )
        .await?;
        self.persist().await;
        Ok(record)
    }

    async fn persist(&self) {
        let records: Vec<AgentRun> = {
            let runs = self.runs.lock().await;
            runs.values().map(|state| state.record.clone()).collect()
        };
        let path = self.runs_path.clone();
        let _ = tokio::task::spawn_blocking(move || write_runs(&path, &records)).await;
    }

    /// Backend availability for health and the inspector.
    pub fn backend_status(&self) -> Vec<AgentBackendStatus> {
        let mut statuses = Vec::new();
        if let Some(hermes) = &self.hermes {
            let probe = hermes.probe();
            statuses.push(AgentBackendStatus {
                kind: AgentKind::Hermes,
                available: probe.available,
                reason: probe.reason,
                models: probe.models,
            });
        } else {
            statuses.push(AgentBackendStatus {
                kind: AgentKind::Hermes,
                available: false,
                reason: Some("MM_AGENT_HERMES_BASE_URL is unset".into()),
                models: Vec::new(),
            });
        }
        if let Some(pi) = &self.pi {
            let probe = pi.probe();
            statuses.push(AgentBackendStatus {
                kind: AgentKind::Pi,
                available: probe.available,
                reason: probe.reason,
                models: Vec::new(),
            });
        } else {
            statuses.push(AgentBackendStatus {
                kind: AgentKind::Pi,
                available: false,
                reason: Some("Pi backend is not configured".into()),
                models: Vec::new(),
            });
        }
        statuses
    }

    async fn start_run(&self, call: AgentCall) -> Result<AgentTurnResult> {
        let input = validate_input(&call.input)?;
        let workspace = self
            .workspaces
            .resolve_call(&call.user_id, call.workspace.as_deref())?;
        let run_id = new_id();
        let agentstream_id = agentstream::session_id(&run_id);
        {
            let runs = self.runs.lock().await;
            let active = runs
                .values()
                .filter(|state| {
                    state.record.session_id == call.parent_session
                        && !state.record.status.is_terminal()
                })
                .count();
            if active >= self.max_active_runs {
                return Err(AgentError::Budget(format!(
                    "session already has {active} active runs (cap {})",
                    self.max_active_runs
                )));
            }
        }
        agentstream::start_session(
            &self.store,
            &run_id,
            &call.user_id,
            &call.parent_session,
            &input,
            call.kind,
        )
        .await?;
        let now = mm_data::rdf::now_ms();
        let record = AgentRun {
            run_id: run_id.clone(),
            session_id: call.parent_session.clone(),
            agentstream_id: agentstream_id.clone(),
            kind: call.kind,
            status: AgentRunStatus::Running,
            turns: 0,
            task: input.clone(),
            origin: call.origin,
            started_at: now,
            updated_at: now,
            last_summary: None,
            last_error: None,
            usage: AgentUsage::default(),
            workspace: workspace.clone(),
        };
        {
            let mut runs = self.runs.lock().await;
            runs.insert(
                run_id.clone(),
                RunState {
                    record: record.clone(),
                    in_flight: false,
                },
            );
        }
        self.persist().await;
        self.execute_turn(&record, &input, AgentTurnMode::New).await
    }

    async fn continue_run(
        &self,
        run_id: &str,
        input: &str,
        parent_session: &str,
        user_id: &str,
    ) -> Result<AgentTurnResult> {
        let input = validate_input(input)?;
        let record = {
            let mut runs = self.runs.lock().await;
            let Some(state) = runs.get_mut(run_id) else {
                return Err(AgentError::NotFound {
                    run_id: run_id.to_owned(),
                    active: active_ids(&runs),
                });
            };
            if state.record.session_id != parent_session {
                return Err(AgentError::Rejected(
                    "agent run belongs to another session".into(),
                ));
            }
            if state.record.status.is_terminal() {
                return Err(AgentError::RunTerminal(Box::new(state.record.clone())));
            }
            if state.record.turns >= 32 {
                return Err(AgentError::Budget("agent run exceeded 32 turns".into()));
            }
            if state.in_flight {
                let mut current = AgentTurnResult::running(run_id);
                current.turn = state.record.turns;
                current.workspace = state.record.workspace.clone();
                current.message = "A turn is already in flight.".into();
                current.summary = "A turn is already in flight.".into();
                return Ok(current);
            }
            state.record.updated_at = mm_data::rdf::now_ms();
            state.record.clone()
        };
        let _ = user_id;
        self.execute_turn(&record, &input, AgentTurnMode::Continue)
            .await
    }

    async fn execute_turn(
        &self,
        record: &AgentRun,
        input: &str,
        mode: AgentTurnMode,
    ) -> Result<AgentTurnResult> {
        {
            let mut runs = self.runs.lock().await;
            let Some(state) = runs.get_mut(&record.run_id) else {
                return Err(AgentError::NotFound {
                    run_id: record.run_id.clone(),
                    active: active_ids(&runs),
                });
            };
            if state.record.status == AgentRunStatus::Cancelled {
                return Ok(AgentTurnResult {
                    status: AgentRunStatus::Cancelled,
                    run_id: record.run_id.clone(),
                    turn: state.record.turns,
                    ..AgentTurnResult::running(&record.run_id)
                });
            }
            state.in_flight = true;
            state.record.status = AgentRunStatus::Running;
            state.record.updated_at = mm_data::rdf::now_ms();
        }

        let result = self.turn_inner(record, input, mode).await;
        let mut error_log = None;
        let mut runs = self.runs.lock().await;
        let outcome = if let Some(state) = runs.get_mut(&record.run_id) {
            state.in_flight = false;
            match result {
                Ok(result) => {
                    let cancelled = state.record.status == AgentRunStatus::Cancelled;
                    state.record.turns = result.turn;
                    state.record.status = if cancelled {
                        AgentRunStatus::Cancelled
                    } else {
                        result.status
                    };
                    state.record.updated_at = mm_data::rdf::now_ms();
                    state.record.last_summary = if result.summary.is_empty() {
                        None
                    } else {
                        Some(result.summary.clone())
                    };
                    state.record.last_error = result.error.clone();
                    state.record.usage = state.record.usage.saturating_add(result.usage);
                    if cancelled {
                        Ok(AgentTurnResult {
                            status: AgentRunStatus::Cancelled,
                            ..result
                        })
                    } else {
                        Ok(result)
                    }
                }
                Err(error) => {
                    state.record.updated_at = mm_data::rdf::now_ms();
                    state.record.last_error = Some(error.to_string());
                    let status = if matches!(error, AgentError::Timeout(_)) {
                        AgentRunStatus::Running
                    } else {
                        state.record.status = AgentRunStatus::Failed;
                        AgentRunStatus::Failed
                    };
                    error_log = Some((
                        state.record.agentstream_id.clone(),
                        state.record.turns,
                        status,
                        error.to_string(),
                    ));
                    Err(error)
                }
            }
        } else {
            result
        };
        drop(runs);
        self.persist().await;
        if let Some((session, turns, status, message)) = error_log {
            let _ = agentstream::log_status(
                &self.store,
                &session,
                &record.run_id,
                status,
                turns,
                None,
                Some(&message),
            )
            .await;
        }
        outcome
    }

    async fn turn_inner(
        &self,
        record: &AgentRun,
        input: &str,
        mode: AgentTurnMode,
    ) -> Result<AgentTurnResult> {
        let session = &record.agentstream_id;
        let mut replayed = agentstream::replay(&self.store, session).await?;
        let context_tokens = self
            .hermes
            .as_ref()
            .map(|hermes| hermes.max_context_tokens())
            .unwrap_or(96_000);
        agentstream::bound_messages(&mut replayed, context_tokens);
        agentstream::log_user(&self.store, session, record.turns + 1, input).await?;
        let mut messages: Vec<AgentMessage> = replayed
            .iter()
            .map(|message: &ReplayMessage| AgentMessage {
                role: message.role.to_owned(),
                content: message.content.clone(),
            })
            .collect();
        messages.push(AgentMessage {
            role: "user".into(),
            content: input.to_owned(),
        });
        let system = self.agent_prompt(record, input)?;
        let request = AgentRequest {
            run_id: record.run_id.clone(),
            session_id: record.session_id.clone(),
            user_id: String::new(),
            input: input.to_owned(),
            kind: record.kind,
            mode,
            workspace: record.workspace.clone(),
            context: Some(system),
            messages,
            timeout_ms: match record.kind {
                AgentKind::Hermes => self
                    .hermes
                    .as_ref()
                    .map(|hermes| hermes.timeout_ms())
                    .unwrap_or(120_000),
                AgentKind::Pi => self
                    .pi
                    .as_ref()
                    .map(|pi| pi.timeout_ms())
                    .unwrap_or(120_000),
            },
        };
        let backend: &dyn crate::AgentBackend = match record.kind {
            AgentKind::Hermes => self
                .hermes
                .as_deref()
                .map(|backend| backend as &dyn crate::AgentBackend)
                .ok_or_else(|| {
                    AgentError::Unavailable("MM_AGENT_HERMES_BASE_URL is unset".into())
                })?,
            AgentKind::Pi => self
                .pi
                .as_deref()
                .map(|backend| backend as &dyn crate::AgentBackend)
                .ok_or_else(|| AgentError::Unavailable("Pi backend is not configured".into()))?,
        };
        let mut result = backend.run_turn(&request).await?;
        let raw_turn = result.message.clone();
        result.run_id = record.run_id.clone();
        result.turn = record.turns + 1;
        result.workspace = record.workspace.clone();
        result.model = result.model.or_else(|| {
            Some(match record.kind {
                AgentKind::Hermes => self
                    .hermes
                    .as_ref()
                    .map(|_| "hermes")
                    .unwrap_or("hermes")
                    .to_owned(),
                AgentKind::Pi => self
                    .pi
                    .as_ref()
                    .map(|pi| pi.model().to_owned())
                    .unwrap_or_else(|| "pi".into()),
            })
        });
        let (message, summary) = self
            .summarizer
            .maybe_summarize(
                &raw_turn,
                record.last_summary.as_deref(),
                &result.artifacts,
                &result.tool_calls,
                result.usage.completion_tokens,
            )
            .await;
        result.message = message.clone();
        if let Some(summary) = &summary {
            result.summary = summary.headline.clone();
        } else {
            result.summary = first_summary_line(&raw_turn);
        }
        result.summary_detail = summary.clone();
        let event_id =
            agentstream::log_turn(&self.store, session, result.turn, &result, &raw_turn).await?;
        result.raw_ref = Some(format!("agentstream/{session}/{event_id}"));
        if let Some(summary) = &summary {
            agentstream::log_summary(&self.store, session, result.turn, summary).await?;
        }
        for call in &result.tool_calls {
            let event = mm_data::agentstream::AgentstreamEvent::new(
                session,
                mm_data::agentstream::EventType::ToolCall,
                serde_json::json!({
                    "schema": agentstream::AGENT_TURN_SCHEMA,
                    "turn": result.turn,
                    "name": call.name,
                    "arguments": call.arguments,
                }),
                mm_data::agentstream::EventProvenance {
                    source: "agent".into(),
                    model: result.model.clone(),
                },
            )
            .with_turn(format!("t{}", result.turn));
            self.store.log_event(event).await?;
        }
        agentstream::log_status(
            &self.store,
            session,
            &record.run_id,
            result.status,
            result.turn,
            result.next.as_deref(),
            result.error.as_deref(),
        )
        .await?;
        self.store.snapshot_session(session).await?;
        let runs = self.runs.lock().await;
        let cancelled = runs
            .get(&record.run_id)
            .is_some_and(|state| state.record.status == AgentRunStatus::Cancelled);
        drop(runs);
        if cancelled {
            result.status = AgentRunStatus::Cancelled;
            result.next = None;
        }
        Ok(result)
    }

    fn agent_prompt(&self, record: &AgentRun, input: &str) -> Result<String> {
        let spec = self
            .prompts
            .strategy("agent.run.v1")
            .ok_or_else(|| AgentError::Prompt("missing prompt strategy agent.run.v1".into()))?;
        let mut variables: BTreeMap<String, String> = spec
            .slots
            .iter()
            .map(|slot| (slot.clone(), String::new()))
            .collect();
        variables.insert("being_name".into(), "Metamind".into());
        variables.insert("user_name".into(), "operator".into());
        variables.insert(
            "workspace_label".into(),
            record.workspace.display().to_string(),
        );
        variables.insert(
            "agent_kind".into(),
            agentstream::kind_name(record.kind).into(),
        );
        variables.insert(
            "agent_workspace".into(),
            record.workspace.display().to_string(),
        );
        variables.insert("agent_input".into(), input.to_owned());
        variables.insert("tone".into(), "direct and specific".into());
        self.prompts
            .render("agent.run.v1", &variables)
            .map(|instance| {
                format!(
                    "{}\nOriginal task: {}\nRun origin: {}",
                    instance.text,
                    record.task,
                    record.origin.label()
                )
            })
            .map_err(|error| AgentError::Prompt(error.to_string()))
    }
}

#[async_trait::async_trait]
impl ExternalWorkPort for AgentRuntime {
    async fn run(&self, call: AgentCall) -> Result<AgentTurnResult> {
        match call.run_id.clone() {
            Some(run_id) => {
                self.continue_run(&run_id, &call.input, &call.parent_session, &call.user_id)
                    .await
            }
            None => self.start_run(call).await,
        }
    }

    async fn continue_run(
        &self,
        run_id: &str,
        input: &str,
        parent_session: &str,
        user_id: &str,
    ) -> Result<AgentTurnResult> {
        AgentRuntime::continue_run(self, run_id, input, parent_session, user_id).await
    }

    async fn status(&self, run_id: &str) -> Result<Option<AgentRun>> {
        Ok(self
            .runs
            .lock()
            .await
            .get(run_id)
            .map(|state| state.record.clone()))
    }
}

fn validate_input(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AgentError::Rejected("agent input must not be empty".into()));
    }
    if trimmed.chars().count() > 8_000 {
        return Err(AgentError::Rejected(
            "agent input exceeds 8000 characters".into(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn load_runs(path: &Path) -> Vec<AgentRun> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(mut records) = serde_json::from_str::<Vec<AgentRun>>(&text) else {
        return Vec::new();
    };
    let now = mm_data::rdf::now_ms();
    for record in &mut records {
        if !record.status.is_terminal() {
            record.status = AgentRunStatus::Failed;
            record.updated_at = now;
            record.last_error = Some("server restarted".into());
        }
    }
    records
}

fn write_runs(path: &Path, runs: &[AgentRun]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(runs)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, body)?;
    std::fs::rename(temporary, path)
}

fn active_ids(runs: &HashMap<String, RunState>) -> Vec<String> {
    let mut ids: Vec<_> = runs
        .values()
        .filter(|state| !state.record.status.is_terminal())
        .map(|state| state.record.run_id.clone())
        .collect();
    ids.sort();
    ids
}

fn first_summary_line(raw: &str) -> String {
    raw.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Agent turn completed")
        .trim()
        .chars()
        .take(300)
        .collect()
}

impl AgentRuntime {
    /// Access to the underlying store, for status projections.
    pub fn store(&self) -> &Arc<AgentstreamStore> {
        &self.store
    }
}
