//! `mm-cme` — the three-pass turn pipeline.
//!
//! One pipeline: Interpret → Process → Verify, with explicit retries, staged
//! conversation-state writes, and one append-only agentstream log. There is no
//! session driver, turn/chunk state machine, planner, router, voice tier, memory
//! bridge, plugin host, or process pipeline here.

#![forbid(unsafe_code)]

pub mod budget;
pub mod config;
pub mod error;
pub mod interpret;
pub mod pass_scope;
pub mod pipeline;
pub mod process;
pub mod prompt;
pub mod state_block;
pub mod tools;
pub mod verification;
pub mod verify;

pub use budget::{BudgetClass, BudgetLimits, BudgetSnapshot, PassBudget, TurnEnvelope};
pub use config::{
    BudgetConfig, EnvelopeConfig, InterpretConfig, JudgeConfig, PassConfig, ThreePassConfig,
    ThreePassConfigError, VerificationConfig, VerifyConfig,
};
pub use error::{CmeError, Result};
pub use pass_scope::{PassScope, ALL_TOOLS, CONTROL_TOOLS, CONVERSATION_TOOLS};
pub use pipeline::{TurnPipeline, TurnRequest, TurnResult, MAX_PASS_ROUNDS};
pub use prompt::{CmePromptManager, Persona, PromptVariables, STANDARD_SLOTS};
pub use tools::{all_schemas, Approval, InterpretResult};
pub use verification::{
    BackendRegistry, CheckVerdict, LlmJudgeBackend, ProcessContext, ProcessResult,
    VerificationBackend, VerificationError, VerificationResult, VerifyCheck, VerifyContext,
    MAX_CHECKS,
};
