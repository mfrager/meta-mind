//! `mm-process` — the durable process pipeline.
//!
//! Phase 5 turns a plan into a reviewable process template and runs it durably.
//! The crate is deliberately split so each piece is testable without a model:
//!
//! - [`template`] validates definitions, injects PROCESS.md and renders bindings.
//! - [`machine`] generates the per-template state machine and snapshots it.
//! - [`queue`] orders work, ages priority, dedupes and leases claims.
//! - [`engine`] supervises instances, steps, waits, sub-processes and recovery.
//! - [`plan_builder`] is the classify/route/decode/verify shell around the model.
//! - [`structure`] compiles a plan into a template deterministically.
//! - [`backend`] is the deliberately empty backend-service seam.
//!
//! The server enables the pipeline only when `MM_PROCESS_V1` is on. The crate
//! itself has no flag: a caller that constructs an engine has opted in.

#![forbid(unsafe_code)]

pub mod backend;
pub mod config;
pub mod engine;
pub mod machine;
pub mod plan_builder;
pub mod queue;
pub mod structure;
pub mod template;

pub use backend::{BackendRegistry, BackendRequest, BackendResponse, ProcessBackend};
pub use config::{EngineConfig, LimitConfig, PlanConfig, ProcessConfig};
pub use engine::{
    Clock, ProcessAnalytics, ProcessEngine, StepOutcome, StepRequest, StepRunner, SystemClock,
    TemplateAnalytics, UnavailableStepRunner,
};
pub use plan_builder::{
    expire_due, materialize, BuiltPlan, Classification, Complexity, PlanBuilder,
};
pub use structure::{
    compile_plan, compile_structure, PlanStructure, StructureKind, StructureStep, STRUCTURE_SCHEMA,
};

/// Re-export the durable model so callers do not import two crates for one type.
pub use mm_data::process::{
    instance_uri, template_uri, ProcessError, ProcessInstance, ProcessStatus, ProcessStore,
    ProcessTemplate, ProcessTemplateVersion, QueueState, StepKind, StepState, StepStatus,
    TemplateSource, TemplateStatus, TemplateStep, WaitCondition, WaitKind,
};
