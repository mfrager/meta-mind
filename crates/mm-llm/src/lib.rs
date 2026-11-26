//! `mm-llm` — the LLM substrate.
//!
//! The core keeps one provider-neutral request/response shape, the OpenAI
//! transport, a scripted mock, strict schema validation, redaction, and usage
//! accounting. Caches, routing, replay, grammars, and the durable ledger retired
//! with their stores.
#![forbid(unsafe_code)]

pub mod accounting;
pub mod call;
pub mod client;
pub mod config;
pub mod error;
pub mod mock;
pub mod openai;
pub mod redact;
pub mod schema;

pub use accounting::{CallAccount, CallStatus, LlmStats, PurposeStats, StatsWindow};
pub use call::{current_context, scope, CallContext};
pub use client::{
    CacheMode, CollectSink, DecoderKind, DeltaSink, LlmClient, LlmRequest, LlmResponse, Message,
    ProviderToolCall, Purpose, Role, SchemaId, ToolSchema, Usage, DEFAULT_MAX_TOKENS,
};
pub use config::{
    Complexity, LlmConfig, LlmDefaults, ModelSpec, Precision, ProviderEnv, Stakes, StructuredOutput,
};
pub use error::{LlmError, Result};
pub use mock::{MockClient, ScriptedResponse};
pub use openai::{OpenAiClient, ThinkingPolicy, ThinkingStyle, DEFAULT_IDLE_TIMEOUT_MS};
pub use schema::{SchemaError, SchemaErrorKind, SchemaRegistry, StructuredOut};
