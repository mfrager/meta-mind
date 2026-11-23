//! `mm-data` — the MetaMind RDF data layer and unified memory.
//!
//! One data root holds everything: the Oxigraph store, agentstream session and
//! event logs, user/project/app homes, datasets, SQLite and the four memory
//! tiers. [`initialize_data_layer`] creates or reopens it; [`MemorySystem`] is
//! the API the conversation engine uses.
//!
//! # Backend note
//!
//! The design names `cubecast` as the RDF engine. Cubecast pins an Oxigraph fork
//! whose `oxrocksdb-sys` declares `links = "rocksdb"`; the kernel's
//! `mm-store-graph` links crates.io Oxigraph, which declares the same native
//! library. Cargo forbids both in one binary, so this crate wraps
//! [`mm_store_graph::GraphStore`] directly. The data model, graph layout and
//! memory APIs are independent of that choice; swapping the engine in Phase 7
//! (when `mm-store-graph` is replaced) does not change them.
//!
//! # Quick start
//!
//! ```no_run
//! use mm_data::{initialize_data_layer, InitConfig};
//!
//! # async fn run() -> Result<(), mm_data::DataError> {
//! let data = initialize_data_layer(InitConfig {
//!     in_memory: true,
//!     ..InitConfig::default()
//! })
//! .await?;
//! let session = data
//!     .memory_system
//!     .start_session(
//!         &mm_data::UserId::new("user-1"),
//!         None,
//!         None,
//!     )
//!     .await?;
//! println!("{}", session.greeting);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod actions;
pub mod agentstream;
pub mod apps;
pub mod config;
pub mod conversation;
pub mod data;
pub mod datasets;
pub mod engine;
pub mod error;
pub mod graphs;
pub mod init;
pub mod log;
pub mod memory;
pub mod migrate;
pub mod ontology;
pub mod plan;
pub mod privacy;
pub mod projects;
pub mod rdf;
pub mod session;
pub mod session_directory;
pub mod sqlite;
pub mod users;

pub use actions::{
    ActionContext, ActionError, ActionOrigin, ActionReceipt, ActionStore, FormField, FormFieldKind,
    FormSchema, FormValue, FormValues, InternalActionRegistry, InternalActionSpec, OutputChannel,
    Recurrence, ScheduleSpec, ScheduledAction, ScheduledActionSpec, ScheduledActionStatus,
};
pub use agentstream::{
    AgentstreamEvent, AgentstreamStore, CleanLog, CleanRole, CleanTurn, EventProvenance, EventType,
    SessionFilter, SessionMetadata, SessionRecord, CLEAN_LOG_MAX_CHARS,
};
pub use apps::{AppRecord, AppRegistry, AppSpec};
pub use config::{
    create_directory_structure, load_or_create_config, resolve_base_dir, CompressionConfig,
    DataConfig, EngineConfig, EpisodicConfig, InitConfig, MemoryConfig, ProjectMemoryConfig,
    SegmentConfig, SyncConfig, UserMemoryConfig, VectorIndexConfig,
};
pub use conversation::{
    reserved_status_type, validate_status_key, ConversationError, ConversationEvent,
    ConversationEventKind, ConversationState, ConversationStore, RefKind, ReservedStatusType,
    StateOrigin, StatusRef, StatusValue,
};
pub use data::{
    DataBroker, DataCreateRequest, DataDeleteMode, DataDeleteRequest, DataEntity, DataFilter,
    DataMutationResult, DataOutbox, DataPatch, DataQuery, DataQueryResult, DataSearchRequest,
    DataService, DataStatus, DataUpdateMode, DataUpdateRequest, DataValue, EntityStatus,
    IndexHealth, IndexVerification, NamespaceBinding, NamespaceRegistry, SchemaCatalog,
    SchemaDetail, SchemaInclude, SchemaSpec, SchemaSummary, SearchHit,
    SearchResult as DataSearchResult, ShaclViolation, ValidationTarget,
};
pub use datasets::{DatasetCatalog, DatasetFile, DatasetRecord, DatasetSpec, DatasetVersion};
pub use engine::RdfEngine;
pub use error::{DataError, Result};
pub use log::{LogEntry, LogError, LogFilter, LogLevel, LogOverview, LogPage, RdfLogService};
pub use memory::contract::{
    ForgetRequest, ForgetSelector, MemoryContractError, MemoryKind, MemoryRecord, MemoryScope,
    MemoryScopeCount, MemoryService, MemoryStatus, RecallQuery, RecallResult, RememberRequest,
};
pub use memory::{
    ArtifactKind, ArtifactRef, ArtifactStatus, Commitment, CommitmentStatus, CompressedSegment,
    DecisionRecord, DecisionStatus, EntityId, EpisodicStore, EpisodicTurn, GoalProgress,
    GoalStatus, MemorySystem, MultiDeviceSync, OpenQuestion, PreferenceDimension, ProjectId,
    ProjectMemory, ProjectMemoryStore, ProjectStatus, QuestionPriority, RdfMemoryService, RuleId,
    SearchFilters, SearchResult, SegmentId, SegmentStore, SessionContext, SessionId, Timestamp,
    Turn, TurnId, UiChannel, UiMessage, UserId, UserMemory, UserMemoryStore,
};
pub use plan::{
    parse_structured_plan, Plan, PlanCancellation, PlanControl, PlanControlReceipt,
    PlanControlRequest, PlanError, PlanFormat, PlanOwner, PlanRegistry, PlanRegistryView,
    PlanStatus, PlanStep, PlanStepStatus, PlanStore, PlanVersion, PlanWrite, RdfPlanStore,
    StructuredPlan, StructuredStep, PLAN_GRAPH, PLAN_REGISTRY_IRI,
};
pub use privacy::contains_secret;
pub use projects::{ProjectRecord, ProjectRegistry, ProjectSpec};
pub use session::{
    ConversationFrame, DeleteMode, FrameKind, FrameStack, FrameStatus, FrameUtterance, Presence,
    RdfSessionStore, ResumeMetadata, SessionBundle, SessionError,
    SessionFilter as ConversationSessionFilter, SessionLifecycle, SessionPage, SessionState,
    SessionStore, SessionSummary, SessionTrace, TraceKind, TracePage, WorkCatalog, WorkItem,
    WorkKind, WorkStatus,
};
pub use session_directory::{SessionDirectoryError, SessionDirectoryService};
pub use sqlite::SqliteStore;
pub use users::{UserRecord, UserRegistry, UserSpec};

pub use init::{initialize_data_layer, DataHealth, InitResult};
pub use migrate::{legacy_paths, migrate_from_mm_serve, MigrationReport};
