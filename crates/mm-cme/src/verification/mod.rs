//! Verification check contracts and the LLM judge.
//!
//! A check is model-authored configuration, validated at registration and then
//! executed by a backend. The LLM judge is the only backend in the retained core.

mod backend;
mod llm_judge;
mod registry;
mod types;

pub use backend::VerificationBackend;
pub use llm_judge::{LlmJudgeBackend, DEFAULT_CHECK_TIMEOUT_MS, JUDGE_MAX_OUTPUT_TOKENS};
pub use registry::{
    BackendRegistry, MAX_BACKEND_PARAMS_BYTES, MAX_CHECKS, MAX_DESCRIPTION_CHARS,
    MAX_DETAILS_CHARS, MAX_LABEL_CHARS,
};
pub use types::{
    CheckVerdict, ProcessContext, ProcessResult, VerificationError, VerificationResult,
    VerifyCheck, VerifyContext,
};
