//! Server boot and the shared application state.
//!
//! One process owns `data/core/server.lock`, one JSONL store, one pipeline, and
//! one gateway-wrapped client. No call can bypass the log.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_agentstream::{EventStore, LlmGateway};
use mm_cme::{Persona, ThreePassConfig, TurnPipeline};
use mm_core::Config;
use mm_llm::{LlmClient, LlmConfig, MockClient, OpenAiClient, ScriptedResponse};
use mm_prompt::PromptStore;

use crate::error::AppError;
use crate::metrics::Metrics;

/// Default prompt tree.
pub const DEFAULT_PROMPT_DIR: &str = "config/prompts";
/// Default persona file.
pub const DEFAULT_PERSONA_PATH: &str = "config/persona.toml";
/// Default LLM configuration.
pub const DEFAULT_LLM_CONFIG: &str = "config/llm.toml";

/// The single-writer lock.
pub struct ServerLock {
    path: PathBuf,
    _file: File,
}

impl std::fmt::Debug for ServerLock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerLock")
            .field("path", &self.path)
            .finish()
    }
}

impl ServerLock {
    /// Acquire `server.lock` in `data_dir`.
    pub fn acquire(data_dir: &Path) -> Result<Self, AppError> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join("server.lock");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    AppError::Internal(format!(
                        "another server owns {}; remove it only if no server is running",
                        path.display()
                    ))
                } else {
                    AppError::Internal(format!("cannot acquire {}: {error}", path.display()))
                }
            })?;
        writeln!(file, "{}", std::process::id())?;
        file.flush()?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for ServerLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Everything a route needs.
pub struct AppState {
    /// Process configuration.
    pub cfg: Config,
    /// The only durable event log.
    pub store: Arc<EventStore>,
    /// The one turn pipeline.
    pub pipeline: Arc<TurnPipeline>,
    /// Minimal counters.
    pub metrics: Metrics,
    _lock: ServerLock,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppState")
            .field("data_dir", &self.cfg.data_dir)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Boot over a real provider.
    pub fn boot(cfg: Config) -> Result<Self, AppError> {
        let llm_config = LlmConfig::load_or_default(Path::new(DEFAULT_LLM_CONFIG))?;
        let model = cfg
            .model
            .clone()
            .or_else(|| llm_config.defaults.default_model.clone())
            .ok_or_else(|| {
                AppError::BadRequest(
                    "no turn model: set `model` in config/server.toml or `default_model` in config/llm.toml"
                        .into(),
                )
            })?;
        let client: Arc<dyn LlmClient> = Arc::new(OpenAiClient::new(&llm_config)?);
        Self::build(cfg, client, model)
    }

    /// Boot over a scripted mock. Used by the route tests and by local smoke runs.
    pub fn for_test(cfg: Config, script: Vec<ScriptedResponse>) -> Result<Self, AppError> {
        let client: Arc<dyn LlmClient> = Arc::new(MockClient::script(script));
        Self::build(cfg, client, "mock-small".to_owned())
    }

    fn build(cfg: Config, client: Arc<dyn LlmClient>, model: String) -> Result<Self, AppError> {
        let lock = ServerLock::acquire(&cfg.data_dir)?;
        let store = Arc::new(EventStore::new(&cfg.data_dir)?);
        let prompt_dir = std::env::var_os("MM_PROMPT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PROMPT_DIR));
        let prompts = Arc::new(PromptStore::load(resolve(prompt_dir))?);
        let config_path = std::env::var_os("MM_THREE_PASS_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(mm_cme::config::DEFAULT_CONFIG_PATH));
        let mut three_pass = ThreePassConfig::load_or_default(resolve(config_path))
            .map_err(|error| AppError::Internal(error.to_string()))?;
        three_pass
            .apply_env()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if let Some(judge_model) = &cfg.judge_model {
            let mut judge = three_pass.verification.llm_judge();
            judge.model = Some(judge_model.clone());
            three_pass
                .verification
                .backends
                .insert("llm_judge".into(), judge);
        }
        let persona_path = std::env::var_os("MM_PERSONA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PERSONA_PATH));
        let persona = Persona::load_or_default(resolve(persona_path))?;
        let gateway: Arc<dyn LlmClient> = Arc::new(LlmGateway::new(client, store.clone()));
        let pipeline = Arc::new(TurnPipeline::new(
            store.clone(),
            gateway,
            prompts,
            three_pass,
            model,
            persona,
        )?);
        Ok(Self {
            cfg,
            store,
            pipeline,
            metrics: Metrics::default(),
            _lock: lock,
        })
    }
}

/// Resolve a repository-relative path even when the process cwd is a crate.
fn resolve(path: PathBuf) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path;
    }
    let rooted = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(&path);
    if rooted.exists() {
        rooted
    } else {
        path
    }
}
