//! `mm-llm` — the LLM cognitive substrate.
//!
//! The being has exactly one door to a model. Everything a caller needs is here:
//! a provider-neutral request/response shape, a deterministic router, an exact
//! cache with an optional semantic layer, schema-enforced structured output, a
//! per-call ledger, provenance, and a replay path that cannot touch the network.
//!
//! Two rules shape the whole crate:
//!
//! 1. **Constrain at decode when the backend exposes logits; otherwise request
//!    provider-native JSON Schema; otherwise validate and reject — never coerce.**
//!    A plausible-looking value that violates its schema is worse than no value,
//!    because nothing downstream can tell the two apart.
//! 2. **No component may call a provider directly.** A caller depends on
//!    [`LlmClient`] or [`LlmService`], never on an adapter's JSON.
#![forbid(unsafe_code)]

pub mod accounting;
pub mod backend_llguidance;
pub mod backend_xgrammar;
pub mod cached;
pub mod client;
pub mod config;
pub mod error;
pub mod grammar;
pub mod mock;
pub mod openai;
pub mod provenance;
pub mod redact;
pub mod replay;
pub mod routing;
pub mod schema;
pub mod semantic_cache;
pub mod service;

pub use accounting::{CallAccount, CallStatus, LlmStats, PurposeStats, StatsWindow};
pub use cached::{CacheKey, CachedResponse, ExactCache, FileExactCache};
pub use client::{
    CacheMode, DecoderKind, LlmClient, LlmRequest, LlmResponse, Message, Purpose, Role, SchemaId,
    Usage,
};
pub use config::{Complexity, LlmConfig, LlmDefaults, ModelSpec, Precision, ProviderEnv, Stakes};
pub use error::{LlmError, Result};
pub use grammar::{DecoderBackend, GrammarSpec};
pub use mock::{MockClient, ScriptedResponse};
pub use openai::OpenAiClient;
pub use replay::{NetworkGuard, RecordedResponse, ReplayLlmClient, Session};
pub use routing::{Router, RoutingDecision, RoutingReason, RoutingRequest, ThresholdRouter};
pub use schema::{SchemaError, SchemaErrorKind, SchemaRegistry, StructuredOut};
pub use semantic_cache::SemanticCache;
pub use service::LlmService;
