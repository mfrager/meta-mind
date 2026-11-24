//! Server state: the kernel boot, the console write lock, and the health
//! report (plan `01` §5).
//!
//! The boot is deliberately the same ordered open `mm-cli`'s kernel performs
//! (`crates/mm-cli/src/kernel.rs`): configuration, then the tabular store and
//! its migrations, then the logger, then the event log, then the RDF store.
//! A graph that fails to open is reported on the health line; it does not
//! abort the rest of the boot. The language-model line reads `OPENAI_BASE_URL`
//! and `OPENAI_API_KEY` (already loaded from `<repo>/.env` by `main`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use mm_agent::{AgentRuntime, ExternalWorkPort, TurnSummarizer};
use mm_being::BeingFacade;
use mm_cme::{
    BudgetClass, BudgetLedger, CleanLogProvider, ConversationRouter, FrameEngine,
    PromptContextProvider, PromptVariables, RoutingTable, ThreePassConfig, ThreePassRunner,
    ToolHandler, TurnExecutor, UserProjection,
};
use mm_core::{Config, MmError, UlidFactory};
use mm_data::{
    initialize_data_layer, ActionStore, AgentstreamStore, CleanLog, InitConfig, InitResult,
    InternalActionRegistry, MemorySystem, RdfEngine, RdfLogService, RdfPlanStore,
    CLEAN_LOG_MAX_CHARS,
};
use mm_eventlog::EventLog;
use mm_llm::{CallContext, LlmClient, LlmConfig, ProviderEnv, ThresholdRouter};
use mm_log::Logger;
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

use crate::automation::{
    ActionExecutor, AutomationConfig, AutomationDeps, AutomationManager, PromptActionExecutor,
    PromptFrameExecutor,
};
use crate::chat::ChatCatalog;
use crate::cm_tools::CmTools;
use crate::cme::CmeRuntime;
use crate::controller::{ControllerModel, SELF_DESCRIPTION};
use crate::docs::FocusCatalog;
use crate::dto::{
    EventHead, HealthEnvelope, KernelStatus, ServiceState, ServiceStatus, WIRE_VERSION,
};
use crate::error::AppError;
use crate::feature_flags::FeatureFlags;
use crate::interjection::InterjectionRouter;
use crate::llm_feed::observe;
use crate::llm_feed::LlmActivityFeed;
use crate::metrics::Metrics;
use crate::plan_monitor::PlanMonitor;
use crate::state_service::StateService;
use crate::status_service::StatusService;
use crate::trees::TreeCatalog;

/// Everything the request handlers share.
pub struct AppState {
    /// When the process started (epoch ms), for uptime in the status bar.
    pub started_at: i64,
    /// The resolved configuration (same resolution rules as the CLI).
    pub cfg: Config,
    /// The tabular store.
    pub sqlite: SqliteStore,
    /// The append-only event log.
    pub events: EventLog,
    /// The RDF store, when it opened. `None` means `graph_error` says why.
    pub graph: Option<Arc<GraphStore>>,
    /// Why the RDF store is closed, when `graph` is `None`.
    pub graph_error: Option<String>,
    /// The being, when it opened. Mutations take the mutex.
    pub being: Option<Arc<tokio::sync::Mutex<BeingFacade>>>,
    /// Why the being store is closed, when `being` is `None`.
    pub being_error: Option<String>,
    /// How the controller reaches a model. Tests replace the default.
    pub controller_model: std::sync::Mutex<ControllerModel>,
    /// When set, the firewall denies the next write with this reason. Tests use it.
    pub firewall_denial: std::sync::Mutex<Option<String>>,
    /// The audit-chained logger.
    pub logger: Arc<Logger>,
    /// The monotonic identifier factory, shared with the event log.
    pub ids: Arc<UlidFactory>,
    /// Held for its `Drop`: releasing `data/console.lock` on shutdown.
    pub _lock: ConsoleLock,
    /// Focus documents for the left pane (plan `05`).
    pub focus: FocusCatalog,
    /// Domain trees for the tree pane and mind map (plan `06`).
    pub trees: TreeCatalog,
    /// Chat sessions and resumable runs (plan `08`).
    pub chat: ChatCatalog,
    /// The unified data layer, including RDF, agentstream and memory.
    pub data: InitResult,
    /// Shared unified memory API.
    pub memory_system: Arc<MemorySystem>,
    /// The data layer's RDF engine.
    pub rdf_engine: Arc<RdfEngine>,
    /// The data layer's agentstream store.
    pub agentstream: Arc<AgentstreamStore>,
    /// Phase 6 downstream agent runs (Hermes).
    pub agent: Arc<AgentRuntime>,
    /// Phase 7 feature flags.
    pub features: FeatureFlags,
    /// The reviewed three-pass configuration (Phase 2).
    pub three_pass: ThreePassConfig,
    /// Conversation-engine sessions and their drivers.
    pub cme: Arc<CmeRuntime>,
    /// Phase 1/2 conversation state and session operations.
    pub state_service: Arc<StateService>,
    /// Phase 9 per-session automation supervisors.
    pub automation: Option<Arc<AutomationManager>>,
    /// Phase 9 global plan monitor.
    pub plan_monitor: Option<Arc<PlanMonitor>>,
    /// RDF plan store used by the monitor and the inspector.
    pub plans: Arc<RdfPlanStore>,
    /// Scheduled-action store used by automation and the inspector.
    pub actions: Arc<ActionStore>,
    /// Phase 10 dirty-section status service and SSE broadcast.
    pub status_service: Arc<StatusService>,
    /// Process metrics.
    pub metrics: Arc<Metrics>,
    /// The live LLM activity feed, when `MM_LLM_ACTIVITY` is on.
    pub llm_activity: Option<Arc<LlmActivityFeed>>,
    /// Phase 5 durable process supervisor, built lazily after boot so it can
    /// resolve the controller's model through the assembled state.
    pub process: std::sync::OnceLock<Arc<mm_process::ProcessEngine>>,
}

impl AppState {
    /// Wrap a client in the activity observer when the feed is enabled.
    ///
    /// Every server construction path that can call a model goes through here
    /// or [`crate::llm_feed::openai_client`], so no call bypasses the feed.
    pub fn observed(&self, client: Arc<dyn LlmClient>, context: CallContext) -> Arc<dyn LlmClient> {
        observe(client, self.llm_activity.as_ref(), context)
    }

    /// Resolve configuration exactly like `mm-cli` (`--data-dir` wins over
    /// `--config`, which wins over the repository default).
    pub fn resolve_config(
        config: Option<PathBuf>,
        data_dir: Option<PathBuf>,
    ) -> Result<Config, MmError> {
        if let Some(dir) = data_dir {
            return Ok(Config::for_data_dir(dir));
        }
        match config {
            Some(path) => Config::load(&path),
            None => Config::load_default(),
        }
    }

    /// Boot the kernel: the ordered open above, plus a doctor-style preflight
    /// that proves the stores are writable before any route is served
    /// (plan `01` §5.1).
    pub async fn boot(cfg: Config, lock: ConsoleLock) -> Result<Self, MmError> {
        Self::boot_with_flags(cfg, lock, FeatureFlags::from_env()).await
    }

    /// Boot with explicit feature flags. Tests and embedders use this to opt
    /// into the conversation engine without changing the process environment.
    pub async fn boot_with_flags(
        cfg: Config,
        lock: ConsoleLock,
        features: FeatureFlags,
    ) -> Result<Self, MmError> {
        std::fs::create_dir_all(&cfg.store.data_dir)?;

        let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await?;
        sqlite.migrate().await?;

        let logger = Arc::new(Logger::from_config(
            &cfg.log,
            Some(Arc::new(sqlite.clone())),
        )?);

        let ids = Arc::new(UlidFactory::open(&cfg.ulid_watermark_path())?);
        let events = EventLog::new(sqlite.clone(), Arc::clone(&logger), Arc::clone(&ids));

        let (graph, graph_error) =
            match GraphStore::open(&cfg.store.graph_dir, &cfg.shapes_file()).await {
                Ok(store) => (Some(Arc::new(store)), None),
                Err(err) => (None, Some(err.to_string())),
            };

        let (being, being_error) = match &graph {
            Some(store) => match BeingFacade::open_with_description(
                &sqlite,
                store.handle(),
                Arc::clone(&logger),
                Arc::clone(&ids),
                SELF_DESCRIPTION,
            )
            .await
            {
                Ok(facade) => (Some(Arc::new(tokio::sync::Mutex::new(facade))), None),
                Err(err) => (None, Some(err.to_string())),
            },
            None => (
                None,
                Some(
                    graph_error
                        .clone()
                        .unwrap_or_else(|| "the RDF store did not open".to_string()),
                ),
            ),
        };

        let focus = FocusCatalog::seed();
        let trees = TreeCatalog::seed();
        let chat =
            crate::chat::ChatCatalog::open(Some(cfg.store.data_dir.join("chat-sessions.json")));

        // Phase 7: one unified data layer owns RDF, agentstream and memory. It
        // lives beside the kernel's legacy stores until the migration removes
        // them; the two directories do not share a RocksDB instance.
        let data = initialize_data_layer(InitConfig {
            base_dir: Some(cfg.store.data_dir.clone()),
            ..InitConfig::default()
        })
        .await
        .map_err(|error| MmError::Internal(format!("mm-data boot failed: {error}")))?;
        let memory_system = Arc::clone(&data.memory_system);
        let rdf_engine = Arc::clone(&data.engine);
        let agentstream = Arc::clone(&data.agentstream);
        let cme = Arc::new(CmeRuntime::new(Arc::clone(&memory_system), features));
        let prompts = Arc::new(
            mm_prompt::PromptStore::load(Config::repo_root().join("config/prompts"))
                .map_err(|error| MmError::Internal(format!("prompt tree failed: {error}")))?,
        );
        let state_service = Arc::new(
            StateService::new(
                Arc::clone(&rdf_engine),
                Arc::clone(&agentstream),
                Arc::clone(&prompts),
                data.base_dir.clone(),
            )
            .map_err(|error| {
                MmError::Internal(format!("conversation state service failed: {error}"))
            })?,
        );
        let llm_config = LlmConfig::load_or_default(&Config::repo_root().join("config/llm.toml"))
            .unwrap_or_default();
        // `config/llm.toml` stays authoritative for routing; the registry is a
        // boot-time mirror so the configured catalog is inspectable in SQLite.
        sync_llm_models(&sqlite, &llm_config).await?;
        let llm_activity = features
            .llm_activity
            .then(LlmActivityFeed::from_env)
            .map(Arc::new);
        let summarizer_client = crate::llm_feed::openai_client(
            &llm_config,
            llm_activity.as_ref(),
            CallContext::new("summarize"),
        );
        let action_client = crate::llm_feed::openai_client(
            &llm_config,
            llm_activity.as_ref(),
            CallContext::new("automation"),
        );
        let summarizer = TurnSummarizer::new(
            summarizer_client,
            Arc::clone(&prompts),
            llm_config.defaults.default_model.clone(),
        );
        let agents_path = Config::repo_root().join("config/agents.toml");
        let agent = Arc::new(
            AgentRuntime::from_env(
                Arc::clone(&agentstream),
                Arc::clone(&prompts),
                data.base_dir.clone(),
                summarizer,
                Some(&agents_path),
            )
            .await,
        );
        let action_registry = Arc::new(InternalActionRegistry::builtin());
        let actions = Arc::new(ActionStore::new(
            Arc::clone(&rdf_engine),
            Arc::clone(&action_registry),
        ));
        let logs = Arc::new(RdfLogService::new(Arc::clone(&rdf_engine)));
        let plans: Arc<RdfPlanStore> = Arc::new(RdfPlanStore::new(Arc::clone(&rdf_engine)));
        let automation = features.automation_v1.then(|| {
            let config =
                AutomationConfig::load(&Config::repo_root().join("config/automation.toml"));
            let action_executor = Arc::new(PromptActionExecutor::new(
                Arc::clone(&state_service),
                Arc::clone(&logs),
                Arc::clone(&prompts),
                Arc::clone(&action_registry),
                action_client.clone(),
                llm_config.defaults.default_model.clone(),
            )) as Arc<dyn ActionExecutor>;
            let frame_executor = Arc::new(PromptFrameExecutor::new(
                FrameEngine::new(Arc::clone(&prompts)),
                action_client.clone(),
                llm_config.defaults.default_model.clone(),
            )) as Arc<dyn crate::automation::FrameExecutor>;
            let interjections = Arc::new(
                InterjectionRouter::defaults()
                    .map_err(|error| MmError::Internal(format!("interjection policy: {error}")))?,
            );
            Ok::<Arc<AutomationManager>, MmError>(Arc::new(AutomationManager::new(
                AutomationDeps {
                    state: Arc::clone(&state_service),
                    actions: Arc::clone(&actions),
                    logs: Arc::clone(&logs),
                    frames: FrameEngine::new(Arc::clone(&prompts)),
                    action_executor,
                    frame_executor,
                    interjections,
                    disabled_kinds: AutomationConfig::disabled_kinds(),
                    config,
                },
            )))
        });
        let automation = match automation {
            Some(Ok(automation)) => Some(automation),
            Some(Err(error)) => return Err(error),
            None => None,
        };
        let plan_monitor = if features.automation_v1 {
            Some(Arc::new(PlanMonitor::new(
                Arc::clone(&plans),
                Arc::clone(&state_service),
                Arc::clone(&agent) as Arc<dyn ExternalWorkPort>,
            )))
        } else {
            None
        };
        let mut three_pass = ThreePassConfig::load_or_default(
            Config::repo_root().join(mm_cme::config::DEFAULT_CONFIG_PATH),
        )
        .map_err(|error| MmError::Internal(format!("three-pass config: {error}")))?;
        three_pass
            .apply_env()
            .map_err(|error| MmError::Internal(format!("three-pass config: {error}")))?;
        if features.three_pass_v2 {
            let mut fragments = vec!["interpret_repair", "verify_repair", "default_action_check"];
            let mut strategies = vec![
                &three_pass.interpret.strategy,
                &three_pass.process.strategy,
                &three_pass.verify.strategy,
            ];
            if three_pass.think.three_pass {
                fragments.push("think_check");
                strategies.push(&three_pass.think.strategy);
            }
            for fragment in fragments {
                if prompts.fragment(fragment).is_none() {
                    return Err(MmError::Internal(format!(
                        "three-pass prompt fragment `{fragment}` is missing"
                    )));
                }
            }
            for strategy in strategies {
                if prompts.strategy(strategy).is_none() {
                    return Err(MmError::Internal(format!(
                        "three-pass prompt strategy `{strategy}` is missing"
                    )));
                }
            }
        }
        if features.process_v1 {
            if prompts.fragment("step_check").is_none() {
                return Err(MmError::Internal(
                    "process prompt fragment `step_check` is missing".into(),
                ));
            }
            if prompts.strategy(&three_pass.step.strategy).is_none() {
                return Err(MmError::Internal(format!(
                    "process prompt strategy `{}` is missing",
                    three_pass.step.strategy
                )));
            }
            mm_process::ProcessConfig::load(Config::repo_root().join("config/process.toml"))
                .map_err(|error| MmError::Internal(format!("process config: {error}")))?;
        }
        let metrics = Arc::new(Metrics::default());
        let status_service = Arc::new(StatusService::new(
            Arc::clone(&state_service),
            Arc::clone(&data.data_service),
            Arc::clone(&agent),
            Arc::clone(&plans),
            Arc::clone(&actions),
            Arc::clone(&logs),
            Arc::clone(&prompts),
            automation.clone(),
            Arc::clone(&metrics),
        ));

        logger
            .emit(
                mm_log::LogRecord::new(mm_log::Level::Info, mm_log::codes::LOG_INIT, "mm.serve")
                    .with_field("sinks", logger.sink_names())
                    .with_field("level", logger.min_level().as_str())
                    .with_field("log_dir", cfg.log.dir.display().to_string()),
            )
            .await?;

        let state = Self {
            started_at: now_ms(),
            cfg,
            sqlite,
            events,
            graph,
            graph_error,
            being,
            being_error,
            controller_model: std::sync::Mutex::new(ControllerModel::FromEnv),
            firewall_denial: std::sync::Mutex::new(None),
            logger,
            ids,
            _lock: lock,
            focus,
            trees,
            chat,
            data,
            memory_system,
            rdf_engine,
            agentstream,
            agent,
            features,
            three_pass,
            cme,
            state_service,
            automation,
            plan_monitor,
            plans,
            actions,
            status_service,
            metrics,
            llm_activity,
            process: std::sync::OnceLock::new(),
        };
        if let Some(automation) = &state.automation {
            let mut events = automation.subscribe();
            let status = Arc::downgrade(&state.status_service);
            tokio::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let Some(status) = status.upgrade() else {
                                break;
                            };
                            if matches!(event.kind.as_str(), "action" | "frame") {
                                status.metrics().automation_emission();
                            }
                            status.mark_dirty_many(
                                &event.session_id,
                                [
                                    crate::status_service::StatusSection::Automation,
                                    crate::status_service::StatusSection::Actions,
                                    crate::status_service::StatusSection::Frames,
                                ],
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
        if let Some(monitor) = &state.plan_monitor {
            let monitor = Arc::downgrade(monitor);
            tokio::spawn(async move {
                // Do not hold the monitor through an idle wait; a dropped
                // console must release the graph store immediately.
                tokio::time::sleep(Duration::from_secs(15)).await;
                loop {
                    let Some(monitor) = monitor.upgrade() else {
                        break;
                    };
                    let decisions = monitor.tick(mm_data::rdf::now_ms()).await;
                    drop(monitor);
                    for decision in decisions {
                        eprintln!(
                            "mm-serve: plan monitor {} step={:?}: {}",
                            decision.action, decision.step_id, decision.detail
                        );
                    }
                    tokio::time::sleep(Duration::from_secs(15)).await;
                }
            });
        }
        Ok(state)
    }

    /// The lazy Phase 5 process supervisor, when `MM_PROCESS_V1` is enabled.
    ///
    /// Construction happens after boot because the step runner resolves the
    /// controller's model through the assembled state (including a scripted test
    /// client). The supervisor loop and restart recovery start exactly once.
    pub fn process_engine(self: &Arc<Self>) -> Result<Arc<mm_process::ProcessEngine>, MmError> {
        if let Some(engine) = self.process.get() {
            return Ok(Arc::clone(engine));
        }
        if !self.features.process_v1 {
            return Err(MmError::Internal(
                "MM_PROCESS_V1 is disabled; no process engine was built".into(),
            ));
        }
        let config_path = Config::repo_root().join("config/process.toml");
        let config = mm_process::ProcessConfig::load(&config_path)
            .map_err(|error| MmError::Internal(format!("process config: {error}")))?;
        let llm_path = Config::repo_root().join("config/llm.toml");
        let llm = LlmConfig::load_or_default(&llm_path).unwrap_or_default();
        let client = crate::controller::resolve_client(self, &llm)
            .ok_or_else(|| MmError::Internal("process pipeline needs a configured model".into()))?;
        let table =
            mm_cme::RoutingTable::load(&Config::repo_root().join("config/routing_table.toml"))
                .unwrap_or_default();
        let model = llm
            .defaults
            .default_model
            .clone()
            .or_else(|| table.default_model.clone());
        let client: Arc<dyn LlmClient> =
            match crate::controller::turn::service(self, Arc::clone(&client)) {
                Some(service) => Arc::new(service),
                None => client,
            };
        let runner = Arc::new(crate::process_runner::ProcessStepRunner::new(
            Arc::clone(&self.state_service),
            Arc::clone(&self.rdf_engine),
            Arc::clone(&self.data.data_service),
            Arc::clone(&self.agent),
            Arc::clone(self.state_service.prompts()),
            client,
            model,
            self.three_pass.clone(),
        ));
        let store = Arc::new(mm_data::RdfProcessStore::new(Arc::clone(&self.rdf_engine)));
        let known_tools = crate::tool_schemas::ToolCatalog::embedded()
            .map(|catalog| {
                catalog
                    .schemas()
                    .iter()
                    .map(|schema| schema.name.clone())
                    .collect::<Vec<_>>()
            })
            .map_err(|error| MmError::Internal(format!("tool catalog: {error}")))?;
        let engine = Arc::new(
            mm_process::ProcessEngine::new(
                store,
                runner,
                config.clone(),
                Arc::new(mm_process::SystemClock),
                mm_process::BackendRegistry::empty(),
                Config::repo_root(),
                known_tools,
            )
            .map_err(|error| MmError::Internal(format!("process engine: {error}")))?,
        );
        if self.process.set(Arc::clone(&engine)).is_err() {
            return Ok(Arc::clone(
                self.process.get().expect("process engine was set"),
            ));
        }
        let plans = Arc::clone(&self.plans);
        let supervisor = Arc::clone(&engine);
        let tick = Duration::from_millis(config.engine.tick_ms.max(50));
        tokio::spawn(async move {
            let mut ticks = 0u64;
            loop {
                if let Err(error) = supervisor.drain_queue("mm-serve").await {
                    eprintln!("mm-serve: process queue drain failed: {error}");
                }
                ticks = ticks.wrapping_add(1);
                if ticks.is_multiple_of(300) {
                    if let Err(error) = supervisor.expire_plans(plans.as_ref()).await {
                        eprintln!("mm-serve: process plan expiry failed: {error}");
                    }
                }
                tokio::time::sleep(tick).await;
            }
        });
        let recovery = Arc::clone(&engine);
        tokio::spawn(async move {
            match recovery.recover().await {
                Ok(recovered) if !recovered.is_empty() => {
                    eprintln!("mm-serve: recovered {} processes", recovered.len())
                }
                Ok(_) => {}
                Err(error) => eprintln!("mm-serve: process recovery failed: {error}"),
            }
        });
        Ok(engine)
    }

    /// Build a conversation-engine executor from the current controller model.
    ///
    /// The executor shares the console's full tool catalog through a weak
    /// reference, so a long-lived CME driver does not keep the application state
    /// alive. The deterministic executor remains the fallback when no model is
    /// configured, so the CME routes work in tests and offline sessions.
    pub fn cme_executor(self: &Arc<Self>) -> TurnExecutor {
        let path = Config::repo_root().join("config/llm.toml");
        let config = LlmConfig::load_or_default(&path).unwrap_or_default();
        let Some(client) = crate::controller::resolve_client(self, &config) else {
            return TurnExecutor::deterministic();
        };
        if config.models.is_empty() {
            return TurnExecutor::deterministic();
        }
        let router = ThresholdRouter::from_specs(
            config.models.clone(),
            config.defaults.default_model.clone(),
        );
        let table = RoutingTable::load(&Config::repo_root().join("config/routing_table.toml"))
            .unwrap_or_default();
        // Route every CM turn through the accounting facade so its calls are in
        // `llm_calls`; the raw adapter records nothing.
        let client: Arc<dyn LlmClient> =
            match crate::controller::turn::service(self, Arc::clone(&client)) {
                Some(service) => Arc::new(service),
                None => client,
            };
        let process = if self.features.process_v1 {
            match self.process_engine() {
                Ok(engine) => Some(engine),
                Err(error) => {
                    eprintln!("mm-serve: process pipeline is unavailable: {error}");
                    None
                }
            }
        } else {
            None
        };
        let tool_client = Arc::clone(&client);
        let runner_client = Arc::clone(&client);
        let model = config
            .defaults
            .default_model
            .clone()
            .or_else(|| table.default_model.clone());
        let prompt_provider: Arc<dyn PromptContextProvider> = Arc::new(SessionPromptProvider {
            state: Arc::downgrade(self),
        });
        let executor = TurnExecutor::with_llm(
            client,
            ConversationRouter::with_table(Arc::new(router), table),
        )
        .with_prompts(Arc::clone(self.state_service.prompts()))
        .with_prompt_provider(Arc::clone(&prompt_provider));
        // `MM_CM_V3` selects the conversation-manager contract surface. It is on
        // by default; turning it off leaves the legacy console controller reachable
        // and gives the CME executor a plain model round with no CM tools.
        let tools: Arc<dyn ToolHandler> = if self.features.cm_v3 {
            let mut cm_tools = CmTools::new(Arc::clone(&self.state_service))
                .with_data(Arc::clone(&self.data.data_service))
                .with_metamind(
                    Arc::clone(&self.rdf_engine),
                    Arc::clone(&self.data.data_service),
                    Arc::clone(self.state_service.prompts()),
                    Some(Arc::clone(&tool_client)),
                    model.clone(),
                )
                .with_agent(Arc::clone(&self.agent))
                .with_process(process.clone());
            // A recursive thought gets its own handler fixed to the Think scope,
            // so its read-only contract is enforced by the dispatcher, not only
            // by its prompt. Its runner has no context provider: the thought has
            // no conversation bundle to reload.
            if self.features.three_pass_v2 && self.three_pass.think.three_pass {
                let think_tools: Arc<dyn ToolHandler> = Arc::new(
                    CmTools::new(Arc::clone(&self.state_service))
                        .with_data(Arc::clone(&self.data.data_service))
                        .with_metamind(
                            Arc::clone(&self.rdf_engine),
                            Arc::clone(&self.data.data_service),
                            Arc::clone(self.state_service.prompts()),
                            Some(Arc::clone(&tool_client)),
                            model.clone(),
                        )
                        .with_agent(Arc::clone(&self.agent))
                        .with_pass_scope(mm_cme::PassScope::Think)
                        .with_process(process.clone()),
                );
                let think_runner = ThreePassRunner::new(
                    Arc::clone(&runner_client),
                    Arc::clone(self.state_service.prompts()),
                    think_tools,
                    self.three_pass.clone(),
                )
                .expect("three-pass runner was validated at boot");
                cm_tools = cm_tools.with_think_runner(Arc::new(think_runner));
            }
            Arc::new(cm_tools)
        } else {
            Arc::new(mm_cme::UnavailableTools)
        };
        let executor = executor.with_tools(Arc::clone(&tools));
        let executor = if self.features.three_pass_v2 {
            let runner = ThreePassRunner::new(
                runner_client,
                Arc::clone(self.state_service.prompts()),
                tools,
                self.three_pass.clone(),
            )
            .expect("three-pass runner was validated at boot")
            .with_context_provider(prompt_provider);
            executor
                .with_three_pass(runner)
                .with_clean_log_provider(Arc::new(SessionCleanLogProvider {
                    state: Arc::downgrade(self),
                }))
        } else {
            executor
        };
        executor
            .with_budgeting(self.three_pass.budget.enabled)
            .with_loop_policy(mm_cme::TurnLoopPolicy::ContinueUntilDone)
    }
}

/// Reloads the current session bundle for prompt rebuilds before each model
/// call, including after a tool-side write. The weak reference avoids an
/// executor -> state cycle.
struct SessionPromptProvider {
    state: Weak<AppState>,
}

#[async_trait]
impl PromptContextProvider for SessionPromptProvider {
    async fn prompt_variables(
        &self,
        session_id: &str,
    ) -> std::result::Result<PromptVariables, String> {
        let state = self
            .state
            .upgrade()
            .ok_or_else(|| "application state is closed".to_owned())?;
        let bundle = state
            .state_service
            .load(session_id)
            .await
            .map_err(|error| format!("conversation bundle: {error}"))?;
        PromptVariables::from_bundle(
            &bundle,
            UserProjection {
                iri: "mmd:current-user".into(),
                entity: bundle.session.user_id.clone(),
                name: bundle.session.user_id.clone(),
                workspace: format!("workspace/{session_id}"),
            },
            BudgetLedger::new(BudgetClass::Standard).snapshot(),
        )
        .map_err(|error| format!("prompt variables: {error}"))
    }
}

/// Rebuilds the clean log from the agentstream before Interpret. Only user
/// messages and `final:true` agent messages contribute.
struct SessionCleanLogProvider {
    state: Weak<AppState>,
}

#[async_trait]
impl CleanLogProvider for SessionCleanLogProvider {
    async fn clean_log(&self, session_id: &str) -> std::result::Result<CleanLog, String> {
        let state = self
            .state
            .upgrade()
            .ok_or_else(|| "application state is closed".to_owned())?;
        let events = state
            .agentstream
            .events(session_id)
            .await
            .map_err(|error| format!("agentstream: {error}"))?;
        Ok(CleanLog::from_events(&events, CLEAN_LOG_MAX_CHARS))
    }
}

impl AppState {
    /// The `GET /api/health` envelope (plan `09` §1).
    ///
    /// `ok` reports whether the server is serving; kernel capability problems
    /// are reported inside `kernel`, never by making `ok` lie (plan `00` §4.3).
    pub async fn health(&self) -> HealthEnvelope {
        let (event_seq, event_err) = match self.events.head_seq().await {
            Ok(seq) => (seq, None),
            Err(e) => (0, Some(e.to_string())),
        };
        let (audit_seq, audit_hash, audit_err) = match self.sqlite.audit_head().await {
            Ok(Some((seq, hash))) => (Some(seq), Some(hash), None),
            Ok(None) => (None, None, None),
            Err(e) => (None, None, Some(e.to_string())),
        };

        let mut services = vec![ServiceStatus {
            name: "sqlite".to_string(),
            state: ServiceState::Ok,
            reason: None,
        }];
        services.push(match event_err {
            None => ServiceStatus {
                name: "eventlog".to_string(),
                state: ServiceState::Ok,
                reason: None,
            },
            Some(reason) => ServiceStatus {
                name: "eventlog".to_string(),
                state: ServiceState::Degraded,
                reason: Some(reason),
            },
        });
        services.push(match &self.graph {
            Some(_) => ServiceStatus {
                name: "graph".to_string(),
                state: ServiceState::Ok,
                reason: None,
            },
            None => ServiceStatus {
                name: "graph".to_string(),
                state: ServiceState::Unavailable,
                reason: Some(
                    self.graph_error
                        .clone()
                        .unwrap_or_else(|| "the RDF store did not open".to_string()),
                ),
            },
        });
        services.push(ServiceStatus {
            name: "cme".to_string(),
            state: ServiceState::Ok,
            reason: Some(if self.features.cme_enabled {
                "conversation engine enabled".to_string()
            } else {
                "disabled by MM_CME_ENABLED".to_string()
            }),
        });
        services.push(llm_status());
        let agent_status = self.agent.backend_status();
        let hermes = agent_status
            .iter()
            .find(|status| status.kind == mm_agent::AgentKind::Hermes);
        services.push(match hermes {
            Some(status) if status.available => ServiceStatus {
                name: "agent".to_string(),
                state: ServiceState::Ok,
                reason: Some(format!(
                    "Hermes models: {}",
                    if status.models.is_empty() {
                        "discovered".to_string()
                    } else {
                        status.models.join(", ")
                    }
                )),
            },
            Some(status) => ServiceStatus {
                name: "agent".to_string(),
                state: ServiceState::Unavailable,
                reason: status.reason.clone(),
            },
            None => ServiceStatus {
                name: "agent".to_string(),
                state: ServiceState::Unavailable,
                reason: Some("the agent runtime is not configured".into()),
            },
        });
        services.push(match &self.being {
            Some(_) => ServiceStatus {
                name: "being".to_string(),
                state: ServiceState::Ok,
                reason: None,
            },
            None => ServiceStatus {
                name: "being".to_string(),
                state: ServiceState::Unavailable,
                reason: Some(
                    self.being_error
                        .clone()
                        .unwrap_or_else(|| "the being store did not open".to_string()),
                ),
            },
        });

        if let Some(reason) = audit_err {
            services.push(ServiceStatus {
                name: "audit".to_string(),
                state: ServiceState::Degraded,
                reason: Some(reason),
            });
        }

        let all_ok = services.iter().all(|service| {
            service.state == ServiceState::Ok || matches!(service.name.as_str(), "llm" | "agent")
        });

        HealthEnvelope {
            ok: true,
            wire_version: WIRE_VERSION,
            kernel: KernelStatus {
                state: if all_ok {
                    ServiceState::Ok
                } else {
                    ServiceState::Degraded
                },
                reason: if all_ok {
                    None
                } else {
                    Some("see kernel.services for the failing component".to_string())
                },
                services,
            },
            data_dir: self.cfg.store.data_dir.display().to_string(),
            event_head: EventHead {
                seq: event_seq,
                audit_seq,
                audit_hash,
            },
        }
    }
}

/// `data/console.lock` — the single-writer guard (plan `01` §5.1).
///
/// While any `mm-serve` holds this file the data directory is owned by the
/// console; a second console (or a store-locking CLI command) gets a `locked`
/// error naming the holder. A stale lock (dead pid) is reclaimed on boot.
#[derive(Debug)]
pub struct ConsoleLock {
    path: PathBuf,
}

impl ConsoleLock {
    /// Acquire the lock for `data_dir`, or explain who holds it.
    pub fn acquire(data_dir: &Path) -> Result<Self, AppError> {
        let path = data_dir.join("console.lock");
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write as _;
                    let _ = writeln!(f, "{} {}", std::process::id(), now_ms());
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let holder = std::fs::read_to_string(&path).unwrap_or_default();
                    if lock_holder_alive(&holder) {
                        return Err(AppError::Locked(format!(
                            "{} is held by {}; stop that console (or the CLI command) first",
                            path.display(),
                            holder.trim()
                        )));
                    }
                    // Stale lock from a dead process: reclaim and retry once.
                    let _ = std::fs::remove_file(&path);
                }
                Err(e) => {
                    return Err(AppError::Internal(format!(
                        "cannot create {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        Err(AppError::Locked(format!(
            "could not acquire {}",
            path.display()
        )))
    }
}

impl Drop for ConsoleLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether the pid recorded in a lock file is still running (Linux `/proc`).
fn lock_holder_alive(holder: &str) -> bool {
    let pid = holder
        .split_whitespace()
        .next()
        .and_then(|p| p.parse::<u32>().ok());
    match pid {
        Some(pid) if pid == std::process::id() => true,
        Some(pid) => Path::new(&format!("/proc/{pid}")).exists(),
        None => false,
    }
}

/// The language-model line. Credentials come from the process environment
/// (`OPENAI_BASE_URL` and `OPENAI_API_KEY`). Model names come from
/// `config/llm.toml`. This does not call the endpoint.
fn llm_status() -> ServiceStatus {
    let Some(env) = ProviderEnv::from_env() else {
        return ServiceStatus {
            name: "llm".to_string(),
            state: ServiceState::Unavailable,
            reason: Some("no language-model provider is configured".to_string()),
        };
    };
    let path = Config::repo_root().join("config/llm.toml");
    let cfg = LlmConfig::load_or_default(&path).unwrap_or_default();
    let model = match cfg.defaults.default_model {
        Some(name) => name,
        None if cfg.models.is_empty() => "unset".to_string(),
        None => cfg
            .models
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>()
            .join(","),
    };
    ServiceStatus {
        name: "llm".to_string(),
        state: ServiceState::Ok,
        reason: Some(format!("model={model}; {}", env.redacted())),
    }
}

/// Replace the `llm_models` registry with the models loaded from
/// `config/llm.toml`. The table is a mirror, not a second source of truth, so a
/// model removed from the file disappears from the table on the next boot.
async fn sync_llm_models(store: &SqliteStore, config: &LlmConfig) -> Result<usize, MmError> {
    let synced_at = mm_core::Timestamp::now().to_rfc3339();
    let mut tx = store
        .pool()
        .begin()
        .await
        .map_err(|error| MmError::Store(format!("llm model registry: {error}")))?;
    sqlx::query("DELETE FROM llm_models")
        .execute(&mut *tx)
        .await
        .map_err(|error| MmError::Store(format!("llm model registry: {error}")))?;
    for spec in &config.models {
        sqlx::query(
            "INSERT INTO llm_models (\
               name, input_cost_micros_per_1k, output_cost_micros_per_1k, \
               typical_latency_ms, complexity_ceiling, mock, synced_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&spec.name)
        .bind(spec.input_cost_micros_per_1k)
        .bind(spec.output_cost_micros_per_1k)
        .bind(i64::from(spec.typical_latency_ms))
        .bind(spec.complexity_ceiling.as_str())
        .bind(i64::from(spec.mock))
        .bind(&synced_at)
        .execute(&mut *tx)
        .await
        .map_err(|error| MmError::Store(format!("llm model registry: {error}")))?;
    }
    tx.commit()
        .await
        .map_err(|error| MmError::Store(format!("llm model registry: {error}")))?;
    Ok(config.models.len())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_llm::{Complexity, ModelSpec};

    #[test]
    fn a_live_holder_blocks_and_a_dead_one_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("console.lock");

        // A lock naming a live process (this one) is honored.
        std::fs::write(&lock_path, format!("{} 0", std::process::id())).unwrap();
        let err = ConsoleLock::acquire(dir.path()).unwrap_err();
        assert!(matches!(err, AppError::Locked(_)), "{err:?}");

        // A lock naming a dead pid is reclaimed.
        std::fs::write(&lock_path, "999999999 0").unwrap();
        let _lock = ConsoleLock::acquire(dir.path()).unwrap();
        assert!(lock_path.exists(), "lock file exists while held");
    }

    #[test]
    fn dropping_the_lock_releases_it() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("console.lock");
        {
            let _lock = ConsoleLock::acquire(dir.path()).unwrap();
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists(), "Drop must remove console.lock");
    }

    #[tokio::test]
    async fn model_registry_mirrors_config_and_drops_removed_models() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("metamind.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let spec = |name: &str, mock: bool| ModelSpec {
            name: name.to_string(),
            input_cost_micros_per_1k: 10,
            output_cost_micros_per_1k: 20,
            typical_latency_ms: 300,
            complexity_ceiling: Complexity::Medium,
            mock,
        };
        let mut config = LlmConfig {
            models: vec![spec("alpha/model", false), spec("mock-small", true)],
            ..LlmConfig::default()
        };
        assert_eq!(sync_llm_models(&store, &config).await.unwrap(), 2);
        let rows: Vec<(String, i64, i64, i64, String, i64)> = sqlx::query_as(
            "SELECT name, input_cost_micros_per_1k, output_cost_micros_per_1k, \
             typical_latency_ms, complexity_ceiling, mock FROM llm_models ORDER BY name",
        )
        .fetch_all(store.pool())
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "alpha/model".to_string(),
                    10,
                    20,
                    300,
                    "medium".to_string(),
                    0
                ),
                (
                    "mock-small".to_string(),
                    10,
                    20,
                    300,
                    "medium".to_string(),
                    1
                ),
            ]
        );

        config.models.truncate(1);
        assert_eq!(sync_llm_models(&store, &config).await.unwrap(), 1);
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM llm_models ORDER BY name")
            .fetch_all(store.pool())
            .await
            .unwrap();
        assert_eq!(names, vec!["alpha/model"]);
        store.close().await;
    }

    #[tokio::test]
    async fn a_second_graph_open_fails_while_this_process_holds_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = mm_core::Config::for_data_dir(dir.path());
        let lock = ConsoleLock::acquire(dir.path()).unwrap();
        let state = AppState::boot(cfg, lock).await.unwrap();
        assert!(state.graph.is_some(), "the console holds the graph");
        let configured = LlmConfig::load_or_default(&Config::repo_root().join("config/llm.toml"))
            .unwrap_or_default()
            .models
            .len() as i64;
        assert_eq!(
            state.sqlite.row_count("llm_models").await.unwrap(),
            configured,
            "boot must mirror config/llm.toml into llm_models"
        );
        let opened = GraphStore::open(&state.cfg.store.graph_dir, &state.cfg.shapes_file()).await;
        assert!(
            opened.is_err(),
            "a second open must fail while the first store is held"
        );
    }

    #[test]
    fn only_the_activity_feed_constructs_a_provider_client() {
        // Build the needle so this test's own source does not match it.
        let needle = format!("{}::new", "OpenAiClient");

        fn visit(dir: &std::path::Path, needle: &str, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, needle, hits);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if source.contains(needle) {
                    hits.push(path.display().to_string());
                }
            }
        }

        let mut hits = Vec::new();
        visit(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &needle,
            &mut hits,
        );
        assert_eq!(hits.len(), 1, "direct provider construction: {hits:?}");
        assert!(
            hits[0].ends_with("llm_feed.rs"),
            "the feed must be the only provider construction path: {hits:?}"
        );
    }
}
