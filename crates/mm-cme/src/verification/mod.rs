//! Verification backends and check contracts.
//!
//! A check is model-authored configuration, validated at registration and then
//! executed by a backend. The first backend is an LLM judge with a strict JSON
//! result; the trait exists so deterministic or symbolic backends can be added
//! without touching the turn runner.

mod backend;
mod journal;
mod llm_judge;
mod registry;
mod types;

pub use backend::VerificationBackend;
pub use journal::{
    is_mutating_call, CommittedMutation, MutationJournal, ProcessMutation, StagedMutation,
    WriteMode, MAX_STAGED_MUTATIONS,
};
pub use llm_judge::{LlmJudgeBackend, DEFAULT_CHECK_TIMEOUT_MS, JUDGE_MAX_OUTPUT_TOKENS};
pub use registry::{
    BackendRegistry, MAX_BACKEND_PARAMS_BYTES, MAX_CHECKS, MAX_DESCRIPTION_CHARS,
    MAX_DETAILS_CHARS, MAX_LABEL_CHARS,
};
pub use types::{
    CheckVerdict, ProcessContext, ProcessResult, VerificationError, VerificationResult,
    VerifyCheck, VerifyContext,
};
