//! `mm-cme` — the MetaMind conversation engine core.
//!
//! The session is a top-level `mm-sm` instance with six orthogonal regions
//! (proactive, voice, memory, preferences, plugins, improvement). Each turn is a
//! child state machine; each turn owns one chunk state machine per planned
//! emission. [`SessionDriver`] is the runtime that turns the machines.
//!
//! The planner is deterministic and never calls a model, the router selects a
//! model from the turn's modality and negotiated voice tier, and the executor
//! drives planning, routing, speculative warm-up, chunk streaming, confirmation-
//! gated tools and finalization through the machines.
//!
//! Phase 5 connects the orthogonal regions to real work: [`MemoryBridge`] logs
//! turns and runs the `mm-data` compression pipeline, [`PreferenceLearner`]
//! learns the five dimensions and feeds the planner, and [`PluginHost`] loads
//! proactive plugins whose selected proposals are injected into the session
//! stream.
//!
//! Phase 6 adds introspection and self-improvement: [`Observability`] captures
//! complete session snapshots, streams `SMTransition` events and exposes
//! Prometheus metrics; [`ImprovementLoop`] reflects, plans, hill-climbs,
//! verifies and learns over [`Report`] and [`Dashboard`] artifacts.
//!
//! ```no_run
//! use std::time::Duration;
//! use mm_cme::{DriverMessage, Session, SessionDriver};
//!
//! # async fn run() -> Result<(), mm_cme::CmeError> {
//! let session = Session::new("user-1", None)?;
//! let (driver, messages, mut outcomes) =
//!     SessionDriver::spawn(session, Duration::from_secs(15));
//! let handle = tokio::spawn(driver.run());
//! messages
//!     .send(DriverMessage::StartTurn { query: "hello".into() })
//!     .unwrap();
//! let outcome = outcomes.recv().await.unwrap();
//! assert!(outcome.is_success());
//! messages.send(DriverMessage::Shutdown).unwrap();
//! let session = handle.await.unwrap()?;
//! assert_eq!(session.state(), "terminated");
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod chunk;
pub mod dashboard;
pub mod driver;
pub mod error;
pub mod executor;
pub mod frames;
pub mod hill_climb;
pub mod improvement;
pub mod memory;
pub mod observability;
pub mod planner;
pub mod plugins;
pub mod preferences;
pub mod prompt;
pub mod regions;
pub mod report;
pub mod routing;
pub mod session;
pub mod turn;
pub mod voice;

pub use chunk::{Chunk, ChunkDirective, ChunkEvent, ChunkId, ChunkKind, ChunkState};
pub use dashboard::{Dashboard, Panel, PanelKind, ViewingContext};
pub use driver::{DriverMessage, SessionDriver};
pub use error::{CmeError, Result};
pub use executor::{
    ConfirmationHandler, DenyUnconfirmed, ToolHandler, TurnExecutor, TurnLoopPolicy,
    UnavailableTools,
};
pub use frames::{FrameEngine, FrameError, FrameTimerView};
pub use hill_climb::{climb, HillClimbConfig, HillClimbResult, HillClimbStep, Optimizable};
pub use improvement::{
    default_critics, default_verifiers, CriterionResult, Critic, GovernanceConfig,
    ImprovementEngine, ImprovementLoop, ImprovementOutcome, ImprovementScheduler,
    ImprovementTrigger, Issue, IssueCategory, LearnedPattern, Reviewable, Severity,
    VerificationContext, VerificationKind, VerificationReport, Verifier,
};
pub use memory::{MemoryBridge, MemoryBridgeConfig, MemoryCycle, SessionResume};
pub use observability::{
    ChunkSnapshot, Observability, PreferencesView, RegionView, SessionSnapshot, SmObserver,
    SmTransitionEvent, TurnSnapshot, VoiceView,
};
pub use planner::{ResponseModality, ToolPolicy, TurnBudget, TurnContext, TurnPlanner};
pub use plugins::{
    BuiltIn, Plugin, PluginCapability, PluginContext, PluginHost, PluginManifest, PluginResult,
    PluginSandbox, ProactiveProposal,
};
pub use preferences::{
    PlannerHooks, PreferenceConfig, PreferenceDimension, PreferenceLearner, PreferencePosterior,
    PreferenceUpdate,
};
pub use prompt::{CmePromptManager, PromptVariables, PromptingError, STANDARD_SLOTS};
pub use regions::{
    improvement::{ImprovementEvent, ImprovementRegion, ImprovementState},
    memory::{MemoryEvent, MemoryRegion, MemoryState},
    plugins::{PluginEvent, PluginRegion, PluginState},
    preferences::{PreferenceEvent, PreferenceRegion, PreferenceState},
    proactive::{
        ProactiveConfig, ProactiveEngine, ProactiveEvent, ProactiveItem, ProactiveRegion,
        ProactiveState,
    },
    voice::{VoiceEvent, VoiceRegion, VoiceState},
    RegionSm, RegionSnapshot,
};
pub use report::{Modality, Report, ReportFormat, ReportSection, TechnicalLevel};
pub use routing::{ConversationRouter, ConversationRoutingRequest, RoutingTable};
pub use session::{
    AffectState, MemoryContext, PreferenceProfile, Session, SessionBudgets, SessionContext,
    SessionEvent, SessionState, REGION_NAMES,
};
pub use turn::{
    ChunkEmission, ToolCallEmission, Turn, TurnEvent, TurnOutcome, TurnPlan, TurnPlanMetadata,
    TurnState,
};
pub use voice::{
    AsrClient, AsrEvent, AsrFinal, AsrPartial, AudioChunk, AudioInput, AudioOutput,
    BargeInDetector, BargeInInfo, EchoResponder, MockAsr, MockAudioInput, MockAudioOutput,
    MockRealtimeClient, MockTts, ProsodyController, ProsodyParams, RealTimeEvent, RealtimeClient,
    TtsClient, Vad, VoiceCapabilities, VoiceNegotiation, VoicePipeline, VoiceResponder, VoiceTier,
    VoiceTurn, VoiceTurnConfig,
};
