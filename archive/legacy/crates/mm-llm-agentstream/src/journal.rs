//! Durable call journaling into agentstream.
//!
//! The journal is deliberately fail-closed before a call: the request event is
//! appended first, and a failure returns [`LlmError::Journal`] without touching a
//! provider. Attempt and terminal events are best-effort after that point, because
//! losing a trace record must not replace the answer the caller already has.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mm_data::agentstream::{
    llm_attempt, llm_error, llm_models, llm_request, llm_response, AgentstreamEvent,
    SessionMetadata,
};
use mm_data::AgentstreamStore;
use mm_llm::redact::Redactor;
use mm_llm::{CallContext, LlmError, LlmRequest, LlmResponse, Result, Usage};
use mm_log::{Level, LogRecord, Logger};
use serde_json::{json, Value};
use ulid::Ulid;

use crate::JournalMode;

/// The narrow journal surface, so tests can count, fail, and inspect appends
/// without booting an RDF engine.
#[async_trait]
pub(crate) trait JournalSink: Send + Sync {
    /// Ensure the session exists. Implementations must be idempotent.
    async fn start_session(
        &self,
        session_id: &str,
        metadata: SessionMetadata,
    ) -> std::result::Result<(), String>;

    /// Append one event.
    async fn append(
        &self,
        event: AgentstreamEvent,
    ) -> std::result::Result<AgentstreamEvent, String>;
}

/// The production sink over [`AgentstreamStore`].
struct StoreJournal {
    store: Arc<AgentstreamStore>,
}

#[async_trait]
impl JournalSink for StoreJournal {
    async fn start_session(
        &self,
        session_id: &str,
        metadata: SessionMetadata,
    ) -> std::result::Result<(), String> {
        self.store
            .start_session(session_id, metadata)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn append(
        &self,
        event: AgentstreamEvent,
    ) -> std::result::Result<AgentstreamEvent, String> {
        self.store
            .log_event(event)
            .await
            .map_err(|error| error.to_string())
    }
}

/// The journal in force for one gateway.
pub(crate) struct CallJournal {
    sink: Arc<dyn JournalSink>,
    mode: JournalMode,
    logger: Arc<Logger>,
    user_id: String,
    text_chars: usize,
    redactor: Redactor,
    started: Mutex<BTreeSet<String>>,
}

impl CallJournal {
    /// A journal over the production RDF store.
    pub(crate) fn new(
        store: Arc<AgentstreamStore>,
        mode: JournalMode,
        logger: Arc<Logger>,
        user_id: String,
    ) -> Self {
        Self::with_sink(Arc::new(StoreJournal { store }), mode, logger, user_id)
    }

    /// A journal over an explicit sink. Tests use this to fail or count appends.
    pub(crate) fn with_sink(
        sink: Arc<dyn JournalSink>,
        mode: JournalMode,
        logger: Arc<Logger>,
        user_id: String,
    ) -> Self {
        Self {
            sink,
            mode,
            logger,
            user_id,
            text_chars: std::env::var("MM_LLM_JOURNAL_TEXT_CHARS")
                .ok()
                .and_then(|raw| raw.parse().ok())
                .filter(|chars| *chars > 0)
                .unwrap_or(16_000),
            redactor: Redactor::new(),
            started: Mutex::new(BTreeSet::new()),
        }
    }

    /// Append the required pre-call event. On failure in required mode this is the
    /// only error the call reports: no provider is touched.
    pub(crate) async fn request(
        &self,
        request: &LlmRequest,
        context: &CallContext,
        provider: &str,
        streamed: bool,
        session_id: &str,
        turn_id: &str,
    ) -> Result<()> {
        self.ensure_session(session_id, &context.source).await?;
        let event = llm_request(
            session_id,
            &mm_core::ulid_string(&request.call_id.unwrap_or_default()),
            request_payload(request, context, provider, streamed),
            request.model.as_deref(),
        )
        .with_turn(turn_id.to_owned());
        self.append_required(event).await
    }

    /// Append one provider attempt. Best-effort: the provider call already happened.
    pub(crate) async fn attempt(
        &self,
        scope: &CallScope,
        attempt_no: u32,
        model: Option<&str>,
        status: &str,
        latency_ms: u64,
        usage: Usage,
    ) {
        let event = llm_attempt(
            &scope.session_id,
            &mm_core::ulid_string(&scope.call_id),
            attempt_no,
            json!({
                "provider": scope.provider,
                "model": model,
                "status": status,
                "latencyMs": latency_ms,
                "tokensIn": usage.tokens_in,
                "tokensOut": usage.tokens_out,
            }),
            model,
        )
        .with_turn(scope.turn_id.clone());
        self.append_optional(event).await;
    }

    /// Append the terminal response. Best-effort.
    pub(crate) async fn response(
        &self,
        scope: &CallScope,
        response: &LlmResponse,
        latency_ms: u64,
    ) {
        let (text, truncated) = bounded_text(&response.text, self.text_chars);
        let event = llm_response(
            &scope.session_id,
            &mm_core::ulid_string(&scope.call_id),
            json!({
                "status": if response.cached { "cached" } else if response.schema_valid { "ok" } else { "schema_rejected" },
                "model": response.model,
                "tokensIn": response.usage.tokens_in,
                "tokensOut": response.usage.tokens_out,
                "costMicros": response.usage.cost_micros,
                "latencyMs": latency_ms,
                "schemaValid": response.schema_valid,
                "cached": response.cached,
                "streamed": response.streamed,
                "attemptCount": scope.attempts.load(Ordering::SeqCst),
                "toolCallCount": response.tool_calls.len(),
                "finishReason": response.finish_reason,
                "text": text,
                "truncated": truncated,
                "textHash": mm_core::hash_fields(&[response.text.as_str()]),
            }),
            Some(&response.model),
        )
        .with_turn(scope.turn_id.clone());
        self.append_optional(event).await;
    }

    /// Append the terminal error. Best-effort.
    pub(crate) async fn error(
        &self,
        scope: &CallScope,
        error: &LlmError,
        latency_ms: u64,
        partial_chars: u32,
    ) {
        let message = self.redactor.excerpt(&error.to_string(), 2_000);
        let event = llm_error(
            &scope.session_id,
            &mm_core::ulid_string(&scope.call_id),
            json!({
                "kind": error.kind(),
                "message": message,
                "latencyMs": latency_ms,
                "partialChars": partial_chars,
            }),
            None,
        )
        .with_turn(scope.turn_id.clone());
        self.append_optional(event).await;
    }

    /// Append one provider model probe. Best-effort: discovery is a health
    /// observation, not part of a call's fail-closed path.
    pub(crate) async fn models(
        &self,
        session_id: &str,
        provider: &str,
        available: bool,
        models: &[String],
        reason: Option<&str>,
    ) {
        let _ = self.ensure_session(session_id, "models").await;
        let reason = if available {
            None
        } else {
            Some(reason.unwrap_or("the provider did not answer /models"))
        };
        let event = llm_models(session_id, provider, models, reason);
        self.append_optional(event).await;
    }

    /// A bounded, redacted error string safe for a live event.
    pub(crate) fn redacted_error(&self, error: &LlmError) -> String {
        self.redactor.excerpt(&error.to_string(), 2_000)
    }

    /// Ensure a session exists, remembering successful starts to avoid rewriting
    /// the same graph on every call.
    async fn ensure_session(&self, session_id: &str, source: &str) -> Result<()> {
        {
            let started = self.started.lock().unwrap_or_else(|e| e.into_inner());
            if started.contains(session_id) {
                return Ok(());
            }
        }
        let result = self
            .sink
            .start_session(
                session_id,
                SessionMetadata {
                    user_id: self.user_id.clone(),
                    project_id: None,
                    title: Some(format!("LLM calls: {source}")),
                },
            )
            .await;
        match result {
            Ok(()) => {
                self.started
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(session_id.to_owned());
                Ok(())
            }
            Err(error) => self.handle_failure(error, true).await,
        }
    }

    async fn append_required(&self, event: AgentstreamEvent) -> Result<()> {
        match self.sink.append(event).await {
            Ok(_) => Ok(()),
            Err(error) => self.handle_failure(error, true).await,
        }
    }

    async fn append_optional(&self, event: AgentstreamEvent) {
        if let Err(error) = self.sink.append(event).await {
            self.report(error).await;
        }
    }

    async fn handle_failure(&self, error: String, required: bool) -> Result<()> {
        self.report(error.clone()).await;
        if required && self.mode == JournalMode::Required {
            Err(LlmError::Journal(error))
        } else {
            Ok(())
        }
    }

    async fn report(&self, error: String) {
        let _ = self
            .logger
            .emit(
                LogRecord::new(Level::Warn, "llm.agentstream.journal", "mm.llm.agentstream")
                    .with_field("error", error),
            )
            .await;
    }
}

/// The per-logical-call identity and attempt counter shared with the provider pool.
pub(crate) struct CallScope {
    pub(crate) call_id: Ulid,
    pub(crate) session_id: String,
    pub(crate) turn_id: String,
    pub(crate) provider: String,
    pub(crate) attempts: AtomicU32,
    journal: Arc<CallJournal>,
}

impl CallScope {
    pub(crate) fn new(
        journal: Arc<CallJournal>,
        call_id: Ulid,
        session_id: String,
        turn_id: String,
        provider: String,
    ) -> Self {
        Self {
            call_id,
            session_id,
            turn_id,
            provider,
            attempts: AtomicU32::new(0),
            journal,
        }
    }

    /// The next 1-based provider round.
    pub(crate) fn next_attempt(&self) -> u32 {
        self.attempts.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Append one attempt outcome.
    pub(crate) async fn record_attempt(
        &self,
        attempt_no: u32,
        model: Option<&str>,
        status: &str,
        latency_ms: u64,
        usage: Usage,
    ) {
        self.journal
            .attempt(self, attempt_no, model, status, latency_ms, usage)
            .await;
    }
}

/// Resolve the session id for a call, following the plan's context table.
pub(crate) fn session_id(context: &CallContext) -> String {
    if let Some(session) = &context.session_id {
        return session.clone();
    }
    if context.source == "agent" {
        if let Some(run_id) = &context.run_id {
            return format!("agent/{run_id}");
        }
    }
    format!("llm/{}", sanitize_source(&context.source))
}

/// Resolve the turn id for a call.
pub(crate) fn turn_id(context: &CallContext, call_id: Ulid) -> String {
    if let Some(run_id) = &context.run_id {
        return run_id.clone();
    }
    if let Some(pass) = &context.pass {
        return match context.round {
            Some(round) => format!("{pass}:{round}"),
            None => pass.clone(),
        };
    }
    mm_core::ulid_string(&call_id)
}

fn sanitize_source(source: &str) -> String {
    let sanitized: String = source
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '/' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "other".to_owned()
    } else {
        sanitized
    }
}

fn request_payload(
    req: &LlmRequest,
    context: &CallContext,
    provider: &str,
    streamed: bool,
) -> Value {
    let model = req.model.as_deref().unwrap_or_default();
    json!({
        "traceId": req.trace_id.map(|id| mm_core::ulid_string(&id)),
        "provider": provider,
        "purpose": req.purpose.as_str(),
        "model": req.model,
        "source": context.source,
        "sessionId": context.session_id,
        "runId": context.run_id,
        "pass": context.pass,
        "round": context.round,
        "trigger": context.trigger,
        "parentCallId": context.parent_call_id,
        "step": context.step,
        "tool": context.tool,
        "origin": context.origin,
        "streamed": streamed,
        "cached": false,
        "messageCount": req.messages.len(),
        "toolCount": req.tools.len(),
        "schemaId": req.schema.as_ref().map(ToString::to_string),
        "promptHash": mm_llm::cached::prompt_hash(provider, model, req),
        "timeoutMs": req.timeout_ms,
    })
}

fn bounded_text(text: &str, max_chars: usize) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return (text.to_owned(), false);
    }
    (chars.into_iter().take(max_chars).collect(), true)
}
