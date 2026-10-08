//! The service facade: the one place a caller touches.
//!
//! Every guarantee the phase promises is enforced here, in one order, for every
//! call — route, consult the cache, decode, validate, repair at most a bounded
//! number of times, and *never* coerce. A call that cannot produce a
//! schema-valid answer ends in [`LlmError::SchemaRejected`] with one
//! `llm_repair_attempts` row per attempt and one `llm_calls` row whose status says
//! so; what it does not do is return a plausible value.
//!
//! Accounting and logging are not a wrapper around the call: the ledger row, the
//! audited `llm.accounting.commit` record, and the `mm:LlmCall` node are written
//! from the same [`CallAccount`], so the three cannot disagree about what a call
//! was.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use mm_log::{codes, Level, LogRecord, Logger};
use mm_store_graph::GraphHandle;
use mm_store_sqlite::SqliteStore;
use serde_json::Value;
use ulid::Ulid;

use crate::accounting::{
    self, CallAccount, CallStatus, GrammarRecord, LlmStats, RepairRecord, RoutingRecord,
    StatsWindow,
};
use crate::cached::{self, CacheKey, CachedResponse, ExactCache, FileExactCache};
use crate::client::{CacheMode, LlmClient, LlmRequest, LlmResponse, Role, SchemaId, Usage};
use crate::config::LlmConfig;
use crate::error::{LlmError, Result};
use crate::grammar::{self, DecoderBackend, GrammarSpec};
use crate::provenance::{self, ProvenanceRecord};
use crate::routing::{Router, RoutingDecision, RoutingRequest, ThresholdRouter};
use crate::schema::{self, SchemaRegistry, StructuredOut};
use crate::semantic_cache::SemanticCache;

/// The target every record from this crate carries.
const TARGET: &str = "mm.llm";

/// The substrate facade.
pub struct LlmService {
    cfg: LlmConfig,
    store: SqliteStore,
    graph: GraphHandle,
    logger: Arc<Logger>,
    client: Arc<dyn LlmClient>,
    schemas: SchemaRegistry,
    backend: Option<Box<dyn DecoderBackend>>,
    router: Box<dyn Router>,
    cache: FileExactCache,
    /// Off unless explicitly enabled; a lock because indexing is a write.
    semantic: Option<Mutex<SemanticCache>>,
    /// Compiled grammars, keyed by fingerprint, so a repeat structured call does not
    /// recompile or re-persist.
    grammars: Mutex<std::collections::BTreeMap<String, GrammarSpec>>,
}

impl std::fmt::Debug for LlmService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmService")
            .field("provider", &self.client.provider())
            .field("decoder", &self.client.decoder())
            .field("models", &self.cfg.models.len())
            .field("cache_dir", &self.cache.dir())
            .finish_non_exhaustive()
    }
}

impl LlmService {
    /// Build the service over a store, a provenance graph handle, a logger, and one
    /// provider adapter.
    ///
    /// The decoder backend is chosen from the *client*, because that is what knows
    /// whether its endpoint exposes logits; the configured preference is the
    /// fallback when the client cannot be constrained at all.
    pub fn new(
        cfg: LlmConfig,
        store: SqliteStore,
        graph: GraphHandle,
        logger: Arc<Logger>,
        client: Arc<dyn LlmClient>,
    ) -> Result<Self> {
        let backend = grammar::backend_for(client.decoder())
            .or_else(|| grammar::backend_for(cfg.defaults.decoder));
        let router = Box::new(ThresholdRouter::new(&cfg));
        let cache = FileExactCache::new(cfg.cache_dir.clone());
        Ok(LlmService {
            cfg,
            store,
            graph,
            logger,
            client,
            schemas: SchemaRegistry::new(),
            backend,
            router,
            cache,
            semantic: None,
            grammars: Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    /// Register a type's strict schema, so structured calls can name it.
    pub fn register<T: StructuredOut>(&mut self) -> Result<()> {
        schema::register_structured::<T>(&mut self.schemas)
    }

    /// Register an ad-hoc schema by id.
    pub fn register_schema(&mut self, id: SchemaId, schema: Value) -> Result<()> {
        self.schemas.register(id, schema)
    }

    /// Use an explicit router.
    pub fn with_router(mut self, router: Box<dyn Router>) -> Self {
        self.router = router;
        self
    }

    /// Enable the semantic cache. It still refuses any purpose the configuration
    /// does not list.
    pub fn with_semantic_cache(mut self, cache: Option<SemanticCache>) -> Self {
        self.semantic = cache.map(Mutex::new);
        self
    }

    /// Root the cache somewhere else (a test, or a resolved repository path).
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache = FileExactCache::new(dir.clone());
        self.cfg.cache_dir = dir;
        self
    }

    /// The configuration in force.
    pub fn config(&self) -> &LlmConfig {
        &self.cfg
    }

    /// The ledger store.
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// The exact cache.
    pub fn cache(&self) -> &FileExactCache {
        &self.cache
    }

    /// The registered schemas.
    pub fn schemas(&self) -> &SchemaRegistry {
        &self.schemas
    }

    /// The provider adapter's name.
    pub fn provider(&self) -> &str {
        self.client.provider()
    }

    /// The provider adapter.
    pub fn client(&self) -> &Arc<dyn LlmClient> {
        &self.client
    }

    /// Make an unstructured call.
    pub async fn call(&self, req: LlmRequest) -> Result<LlmResponse> {
        let routing = RoutingRequest::new(req.purpose);
        self.call_routed(req, routing).await
    }

    /// Make a call with an explicit routing requirement.
    pub async fn call_routed(
        &self,
        req: LlmRequest,
        routing: RoutingRequest,
    ) -> Result<LlmResponse> {
        let provider = self.client.provider().to_string();
        let decoder = self.client.decoder();
        let call_id = accounting::new_call_id();
        let started = Instant::now();

        // ---- route ---------------------------------------------------------
        let mut routing = routing;
        routing.purpose = req.purpose;
        if routing.pinned_model.is_none() {
            routing.pinned_model = req.model.clone();
        }
        let requested_model = req.model.clone();
        let decision = self.router.choose(&routing)?;

        let mut resolved = req;
        resolved.model = Some(decision.model.clone());

        // A schema that is named but not registered can never be enforced, so the
        // call is refused before a provider is asked rather than after.
        if let Some(schema_id) = resolved.schema.clone() {
            if self.schemas.get(&schema_id).is_none() {
                return Err(LlmError::Config(format!(
                    "schema `{schema_id}` is not registered with this service"
                )));
            }
        }

        let prompt_hash = cached::prompt_hash(&provider, &decision.model, &resolved);
        let key = CacheKey::of(&provider, &decision.model, &resolved);

        self.emit(
            LogRecord::new(Level::Info, codes::LLM_ROUTE_SELECT, TARGET)
                .with_field("requested_model", requested_model.clone())
                .with_field("selected_model", decision.model.clone())
                .with_field("reason", decision.reason.as_str())
                .with_field("cost_budget_micros", routing.cost_budget_micros)
                .with_field("latency_budget_ms", routing.latency_budget_ms),
        )
        .await?;
        accounting::record_routing(
            &self.store,
            &RoutingRecord {
                call_id: Some(call_id),
                requested: requested_model.clone(),
                selected: decision.model.clone(),
                reason: decision.reason.as_str().to_string(),
                cost_micros: decision.cost_micros,
            },
        )
        .await?;

        self.emit(
            LogRecord::new(Level::Info, codes::LLM_REQUEST, TARGET)
                .with_field("call_id", mm_core::ulid_string(&call_id))
                .with_field("purpose", resolved.purpose.as_str())
                .with_field("provider", provider.clone())
                .with_field("model", decision.model.clone())
                .with_field("prompt_hash", prompt_hash.clone())
                .with_field("schema_id", resolved.schema.as_ref().map(|s| s.to_string()))
                .with_field("decoder", decoder.as_str())
                .with_field("cache_mode", cache_mode_name(resolved.cache))
                .with_field(
                    "trace_id",
                    resolved.trace_id.map(|t| mm_core::ulid_string(&t)),
                ),
        )
        .await?;

        // ---- exact cache ---------------------------------------------------
        if matches!(resolved.cache, CacheMode::Use | CacheMode::Replay) {
            if let Some(hit) = self.cache.get(&key) {
                // Mirror the row even on a hit: a body whose write landed without its
                // index row heals here instead of surviving as a stray forever.
                cached::put_index(
                    &self.store,
                    &key,
                    &self.cache.path_for(&key),
                    &hit.response_sha,
                )
                .await?;
                self.emit(
                    LogRecord::new(Level::Debug, codes::LLM_CACHE_HIT, TARGET)
                        .with_field("call_id", mm_core::ulid_string(&call_id))
                        .with_field("prompt_hash", prompt_hash.clone())
                        .with_field("model", decision.model.clone())
                        .with_field("schema_id", resolved.schema.as_ref().map(|s| s.to_string()))
                        .with_field("layer", "exact"),
                )
                .await?;
                return self
                    .finish_cached(
                        call_id,
                        &resolved,
                        &requested_model,
                        &provider,
                        &prompt_hash,
                        &decision,
                        "exact",
                        &hit,
                    )
                    .await;
            }
        }
        if matches!(
            resolved.cache,
            CacheMode::Use | CacheMode::Replay | CacheMode::Refresh
        ) {
            self.emit(
                LogRecord::new(Level::Debug, codes::LLM_CACHE_MISS, TARGET)
                    .with_field("call_id", mm_core::ulid_string(&call_id))
                    .with_field("prompt_hash", prompt_hash.clone())
                    .with_field("model", decision.model.clone())
                    .with_field("schema_id", resolved.schema.as_ref().map(|s| s.to_string()))
                    .with_field("layer", "exact"),
            )
            .await?;
        }

        // ---- semantic cache (opt-in) ---------------------------------------
        if let Some(hit) = self.semantic_lookup(&resolved) {
            return self
                .finish_cached(
                    call_id,
                    &resolved,
                    &requested_model,
                    &provider,
                    &prompt_hash,
                    &decision,
                    "semantic",
                    &hit,
                )
                .await;
        }

        // ---- provider ------------------------------------------------------
        let response = match self.client.complete(&resolved).await {
            Ok(response) => response,
            Err(e) => {
                let mut account =
                    CallAccount::started(call_id, resolved.purpose, &provider, &prompt_hash);
                account.trace_id = resolved.trace_id;
                account.model = decision.model.clone();
                account.schema_id = resolved.schema.as_ref().map(|s| s.to_string());
                account.decoder = decoder;
                account.status = e.status();
                account.error_kind = Some(e.kind().to_string());
                account.route_reason = Some(decision.reason.as_str().to_string());
                account.routed_from = requested_model.filter(|m| m != &decision.model);
                account.latency_ms = elapsed_ms(started);
                self.emit(
                    LogRecord::new(Level::Error, codes::LLM_ERROR, TARGET)
                        .with_field("call_id", mm_core::ulid_string(&call_id))
                        .with_field("error_kind", e.kind())
                        .with_field("cause", e.to_string()),
                )
                .await?;
                return self.commit_and_fail(account, &[], e).await;
            }
        };

        // ---- validate, repair, or reject -----------------------------------
        // Repair rows reference the call row, and foreign keys are enforced, so they
        // are buffered here and written by `finish` *after* the call row lands.
        let mut repairs: Vec<RepairRecord> = Vec::new();
        let mut response = response;
        let mut account = CallAccount::started(call_id, resolved.purpose, &provider, &prompt_hash);
        account.trace_id = resolved.trace_id;
        account.model = response.model.clone();
        account.schema_id = resolved.schema.as_ref().map(|s| s.to_string());
        account.decoder = response.decoder;
        account.tokens_in = response.usage.tokens_in;
        account.tokens_out = response.usage.tokens_out;
        account.cost_micros = response.usage.cost_micros;
        account.latency_ms = elapsed_ms(started).max(response.usage.latency_ms);
        account.route_reason = Some(decision.reason.as_str().to_string());
        account.routed_from = requested_model.filter(|m| m != &response.model);

        if let Some(schema_id) = resolved.schema.clone() {
            match self
                .ensure_valid(
                    call_id,
                    &resolved,
                    &schema_id,
                    response,
                    &mut account,
                    started,
                    &mut repairs,
                )
                .await
            {
                Ok(valid) => response = valid,
                Err(e) => return self.commit_and_fail(account, &repairs, e).await,
            }
        }

        response.schema_valid = true;
        account.schema_ok = true;
        account.cached = false;
        account.status = CallStatus::Ok;

        if matches!(resolved.cache, CacheMode::Use | CacheMode::Refresh) {
            self.store_cached(&key, &resolved, &response).await?;
        }

        // The counts are reported under `usage` rather than as `tokens_in`/
        // `tokens_out`. The kernel's redaction policy scrubs any field whose *name*
        // contains a credential word — including `token` — and it is deliberately
        // blunt because it fails closed. Naming a count something that does not look
        // like a secret is cheaper, and far safer, than loosening that policy for a
        // log field.
        self.emit(
            LogRecord::new(Level::Info, codes::LLM_RESPONSE, TARGET)
                .with_field("call_id", mm_core::ulid_string(&call_id))
                .with_field(
                    "usage",
                    serde_json::json!({
                        "in": account.tokens_in,
                        "out": account.tokens_out,
                        "cost_micros": account.cost_micros,
                        "latency_ms": account.latency_ms,
                    }),
                )
                .with_field("cached", account.cached)
                .with_field("status", account.status.as_str()),
        )
        .await?;
        self.finish(account, &repairs).await?;
        Ok(response)
    }

    /// Make a structured call and decode the result into `T`.
    ///
    /// The schema is compiled to a grammar first (and persisted), so the decode is
    /// constrained whenever the client can constrain; when it cannot, the strict
    /// validator and the bounded repair loop are the guarantee instead.
    pub async fn call_structured<T: StructuredOut>(
        &self,
        req: LlmRequest,
    ) -> Result<(T, LlmResponse)> {
        let id = T::schema_id();
        let schema_value = self.schemas.get(&id).cloned().ok_or_else(|| {
            LlmError::Config(format!(
                "schema `{id}` is not registered with this service; call `register::<T>()`"
            ))
        })?;
        self.compile_grammar(&id, &schema_value).await?;

        let mut req = req;
        req.schema = Some(id.clone());
        let response = self.call(req).await?;
        let value =
            self.schemas
                .validate(&id, &response.text)
                .map_err(|e| LlmError::SchemaRejected {
                    schema_id: id.to_string(),
                    detail: e.to_string(),
                })?;
        let typed: T = schema::decode(value).map_err(|e| LlmError::SchemaRejected {
            schema_id: id.to_string(),
            detail: e.to_string(),
        })?;
        Ok((typed, response))
    }

    /// Aggregate the ledger.
    pub async fn stats(&self, window: StatsWindow) -> Result<LlmStats> {
        accounting::stats(&self.store, &window).await
    }

    /// Verify every ledger row is complete.
    pub async fn assert_complete(&self) -> Result<u64> {
        accounting::assert_complete(&self.store).await
    }

    // ---- internals ---------------------------------------------------------

    /// Validate the answer; on failure run the bounded repair loop, then reject.
    ///
    /// `account` is mutated so the caller can account the outcome: a rejected answer
    /// and a provider failure mid-repair both leave the ledger complete.
    async fn ensure_valid(
        &self,
        call_id: Ulid,
        resolved: &LlmRequest,
        schema_id: &SchemaId,
        response: LlmResponse,
        account: &mut CallAccount,
        started: Instant,
        repairs: &mut Vec<RepairRecord>,
    ) -> Result<LlmResponse> {
        let mut last = match self.schemas.validate(schema_id, &response.text) {
            Ok(_) => return Ok(response),
            Err(e) => e,
        };

        self.emit(
            LogRecord::new(Level::Warn, codes::LLM_SCHEMA_REJECT, TARGET)
                .with_field("call_id", mm_core::ulid_string(&call_id))
                .with_field("schema_id", schema_id.to_string())
                .with_field("error_kind", last.kind.as_str())
                // The length, never the content: a rejected payload is caller data and
                // may carry anything, including a secret.
                .with_field("raw_len", response.text.chars().count()),
        )
        .await?;

        // A constrained decode that violated its own grammar is an inconsistency in
        // the decoder, not something a second identical decode can fix, so it is
        // rejected rather than repaired.
        let attempts = if response.decoder.is_constrained() {
            0
        } else {
            self.cfg.defaults.max_repair_attempts
        };

        for attempt in 1..=attempts {
            self.emit(
                LogRecord::new(Level::Info, codes::LLM_REPAIR_ATTEMPT, TARGET)
                    .with_field("call_id", mm_core::ulid_string(&call_id))
                    .with_field("attempt_no", attempt)
                    .with_field("error_kind", last.kind.as_str()),
            )
            .await?;

            match self.client.complete(resolved).await {
                Ok(candidate) => match self.schemas.validate(schema_id, &candidate.text) {
                    Ok(_) => {
                        repairs.push(RepairRecord {
                            call_id,
                            attempt_no: attempt,
                            error_kind: last.kind.as_str().to_string(),
                            accepted: true,
                        });
                        // The repair's usage is added to the first attempt's, so the
                        // ledger row is the cost of *the call*, not of its last try.
                        account.model = candidate.model.clone();
                        account.decoder = candidate.decoder;
                        account.tokens_in += candidate.usage.tokens_in;
                        account.tokens_out += candidate.usage.tokens_out;
                        account.cost_micros += candidate.usage.cost_micros;
                        account.latency_ms = elapsed_ms(started).max(account.latency_ms);
                        return Ok(candidate);
                    }
                    Err(e) => {
                        repairs.push(RepairRecord {
                            call_id,
                            attempt_no: attempt,
                            error_kind: e.kind.as_str().to_string(),
                            accepted: false,
                        });
                        last = e;
                    }
                },
                Err(e) => {
                    repairs.push(RepairRecord {
                        call_id,
                        attempt_no: attempt,
                        error_kind: e.kind().to_string(),
                        accepted: false,
                    });
                    account.status = e.status();
                    account.error_kind = Some(e.kind().to_string());
                    return Err(e);
                }
            }
        }

        account.schema_ok = false;
        account.status = CallStatus::SchemaRejected;
        account.error_kind = Some("schema_rejected".into());
        Err(LlmError::SchemaRejected {
            schema_id: schema_id.to_string(),
            detail: last.to_string(),
        })
    }

    /// Compile and persist the grammar for a schema, once per fingerprint.
    async fn compile_grammar(
        &self,
        id: &SchemaId,
        schema_value: &Value,
    ) -> Result<Option<GrammarSpec>> {
        let Some(backend) = &self.backend else {
            return Ok(None);
        };
        let spec = backend.compile(id, schema_value)?;
        // The guard is never held across an await.
        let fresh = {
            let mut compiled = self.grammars.lock().unwrap_or_else(|e| e.into_inner());
            if compiled.contains_key(&spec.fingerprint) {
                false
            } else {
                compiled.insert(spec.fingerprint.clone(), spec.clone());
                true
            }
        };
        if fresh {
            accounting::record_grammar(
                &self.store,
                &GrammarRecord {
                    schema_id: id.to_string(),
                    schema_sha: spec.schema_sha.clone(),
                    grammar: spec.grammar.clone(),
                    decoder: spec.decoder.as_str().to_string(),
                },
            )
            .await?;
            self.emit(
                LogRecord::new(Level::Debug, codes::LLM_GRAMMAR_COMPILE, TARGET)
                    .with_field("schema_id", id.to_string())
                    .with_field("schema_sha", spec.schema_sha.clone())
                    .with_field("decoder", spec.decoder.as_str())
                    .with_field("ok", true),
            )
            .await?;
        }
        Ok(Some(spec))
    }

    /// A semantic hit, when the purpose allows one and the body is intact.
    fn semantic_lookup(&self, resolved: &LlmRequest) -> Option<CachedResponse> {
        let semantic = self.semantic.as_ref()?;
        let path = {
            let guard = semantic.lock().unwrap_or_else(|e| e.into_inner());
            if !guard.allowed_for(resolved.purpose) {
                return None;
            }
            let prompt = last_user_text(resolved);
            let (entry, _similarity) = guard.lookup(resolved.purpose, &prompt)?;
            entry.response_path.clone()
        };
        let raw = std::fs::read_to_string(path).ok()?;
        let stored: CachedResponse = serde_json::from_str(&raw).ok()?;
        stored.is_intact().then_some(stored)
    }

    /// Write a response into the exact cache, the index mirror, and the semantic
    /// index when the purpose allows it.
    async fn store_cached(
        &self,
        key: &CacheKey,
        resolved: &LlmRequest,
        response: &LlmResponse,
    ) -> Result<()> {
        let body = CachedResponse::new(
            response.text.clone(),
            response.model.clone(),
            response.decoder,
            response.usage.tokens_in,
            response.usage.tokens_out,
            response.usage.latency_ms,
        );
        let path = self.cache.path_for(key);
        self.cache.put(key.clone(), body.clone())?;
        cached::put_index(&self.store, key, &path, &body.response_sha).await?;

        if let Some(semantic) = &self.semantic {
            let mut guard = semantic.lock().unwrap_or_else(|e| e.into_inner());
            if guard.allowed_for(resolved.purpose) {
                let prompt = last_user_text(resolved);
                guard.insert(
                    resolved.purpose,
                    &prompt,
                    &key.prompt_hash,
                    &path.display().to_string(),
                );
            }
        }
        Ok(())
    }

    /// Finish a call served from a cache layer.
    #[allow(clippy::too_many_arguments)]
    async fn finish_cached(
        &self,
        call_id: Ulid,
        resolved: &LlmRequest,
        requested_model: &Option<String>,
        provider: &str,
        prompt_hash: &str,
        decision: &RoutingDecision,
        layer: &str,
        hit: &CachedResponse,
    ) -> Result<LlmResponse> {
        let mut account = CallAccount::started(call_id, resolved.purpose, provider, prompt_hash);
        account.trace_id = resolved.trace_id;
        account.model = hit.model.clone();
        account.schema_id = resolved.schema.as_ref().map(|s| s.to_string());
        account.decoder = hit.decoder;
        // A hit bills nothing: no provider was asked, so a cost here would be a
        // number the ledger cannot justify.
        account.tokens_in = hit.tokens_in;
        account.tokens_out = hit.tokens_out;
        account.cost_micros = 0;
        account.latency_ms = 0;
        account.cached = true;
        account.cache_layer = Some(layer.to_string());
        account.route_reason = Some(decision.reason.as_str().to_string());
        account.routed_from = requested_model.clone().filter(|m| m != &hit.model);
        account.status = CallStatus::Ok;
        self.finish(account, &[]).await?;
        Ok(LlmResponse {
            call_id,
            text: hit.text.clone(),
            model: hit.model.clone(),
            usage: Usage {
                tokens_in: hit.tokens_in,
                tokens_out: hit.tokens_out,
                cost_micros: 0,
                latency_ms: hit.latency_ms,
            },
            cached: true,
            schema_valid: true,
            decoder: hit.decoder,
        })
    }

    /// Write the ledger row, every repair row, the audited commit record, and the
    /// provenance node.
    ///
    /// The call row is written first because `llm_repair_attempts` references it;
    /// buffering the repair rows until here is what lets the foreign key be enforced
    /// rather than merely declared.
    async fn finish(&self, account: CallAccount, repairs: &[RepairRecord]) -> Result<()> {
        let at = mm_core::Timestamp::now().to_rfc3339();
        accounting::commit(&self.store, &account).await?;
        for repair in repairs {
            accounting::record_repair(&self.store, repair).await?;
        }
        self.emit_commit(&account).await?;
        provenance::record(&self.graph, &ProvenanceRecord::from_account(&account, at)).await?;
        Ok(())
    }

    /// Account a failed call, then return its error.
    async fn commit_and_fail(
        &self,
        account: CallAccount,
        repairs: &[RepairRecord],
        error: LlmError,
    ) -> Result<LlmResponse> {
        self.finish(account, repairs).await?;
        Err(error)
    }

    /// The audited record that makes a ledger row correlatable.
    ///
    /// Info rather than Debug: a Debug record is filtered out at the kernel's
    /// default level, and a *filtered* audit record is a completeness check that can
    /// never pass.
    async fn emit_commit(&self, account: &CallAccount) -> Result<()> {
        self.logger
            .audit(
                Level::Info,
                codes::LLM_ACCOUNTING_COMMIT,
                TARGET,
                account.trace_id,
                serde_json::json!({
                    "call_id": mm_core::ulid_string(&account.call_id),
                    "trace_id": account.trace_id.map(|t| mm_core::ulid_string(&t)),
                    "row_written": true,
                    "status": account.status.as_str(),
                }),
            )
            .await
            .map_err(LlmError::from)
    }

    async fn emit(&self, record: LogRecord) -> Result<()> {
        self.logger.emit(record).await.map_err(LlmError::from)
    }
}

fn elapsed_ms(started: Instant) -> u32 {
    started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}

fn cache_mode_name(mode: CacheMode) -> &'static str {
    match mode {
        CacheMode::Use => "use",
        CacheMode::Bypass => "bypass",
        CacheMode::Refresh => "refresh",
        CacheMode::Replay => "replay",
    }
}

fn last_user_text(req: &LlmRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

/// Resolve a possibly-relative cache directory against a root.
pub fn resolve_cache_dir(root: &Path, configured: &Path) -> PathBuf {
    if configured.is_absolute() || root.as_os_str().is_empty() {
        configured.to_path_buf()
    } else {
        root.join(configured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Purpose};
    use crate::mock::{MockClient, ScriptedResponse};
    use mm_log::RedactionPolicy;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Rating {
        name: String,
        score: i64,
    }

    impl StructuredOut for Rating {
        fn schema_id() -> SchemaId {
            SchemaId::new("rating.v1")
        }
        fn schema() -> Value {
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "score"],
                "properties": {
                    "name": { "type": "string", "minLength": 1 },
                    "score": { "type": "integer", "minimum": 0, "maximum": 10 }
                }
            })
        }
    }

    fn config(max_repair_attempts: u32) -> LlmConfig {
        LlmConfig::from_toml_str(&format!(
            r#"
[defaults]
decoder = "llguidance"
max_repair_attempts = {max_repair_attempts}
max_tokens = 128

[[model]]
name = "mock-small"
input_cost_micros_per_1k = 100
output_cost_micros_per_1k = 400
typical_latency_ms = 5
complexity_ceiling = "high"

[[model]]
name = "frontier"
input_cost_micros_per_1k = 2500
output_cost_micros_per_1k = 10000
typical_latency_ms = 2500
complexity_ceiling = "very_high"
"#
        ))
        .unwrap()
    }

    struct Harness {
        _dir: tempfile::TempDir,
        store: SqliteStore,
        graph: mm_store_graph::GraphStore,
        service: LlmService,
        log_path: PathBuf,
    }

    async fn harness(client: Arc<dyn LlmClient>, max_repair_attempts: u32) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("m.db")).await.unwrap();
        store.migrate().await.unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("mm-shapes.ttl");
        let graph = mm_store_graph::GraphStore::in_memory(&shapes)
            .await
            .unwrap();
        // The sink is a file so a test can read exactly what `logs verify` reads.
        let log_path = dir.path().join("mm.jsonl");
        let sink = mm_log::JsonlSink::open(&log_path).unwrap();
        let logger = Arc::new(Logger::new(
            Level::Debug,
            vec![Box::new(sink)],
            Some(Arc::new(store.clone())),
            RedactionPolicy::kernel_default(),
        ));
        let mut cfg = config(max_repair_attempts);
        cfg.defaults.max_tokens = 128;
        let mut service =
            LlmService::new(cfg, store.clone(), graph.handle().clone(), logger, client)
                .unwrap()
                .with_cache_dir(dir.path().join("cache"));
        service.register::<Rating>().unwrap();
        Harness {
            _dir: dir,
            store,
            graph,
            service,
            log_path,
        }
    }

    /// The emitted record for `event_code`, parsed.
    fn emitted(h: &Harness, event_code: &str) -> Vec<serde_json::Value> {
        let raw = std::fs::read_to_string(&h.log_path).unwrap_or_default();
        raw.lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|record| record["event_code"] == event_code)
            .collect()
    }

    fn extract_request(text: &str) -> LlmRequest {
        LlmRequest::new(Purpose::Extract, vec![Message::user(text)])
    }

    async fn row_count(store: &SqliteStore, table: &str) -> i64 {
        store.row_count(table).await.unwrap()
    }

    #[tokio::test]
    async fn a_structured_call_lands_one_complete_row_and_a_provenance_node() {
        let client = Arc::new(MockClient::constant(ScriptedResponse::text(
            r#"{"name":"Ada","score":9}"#,
        )));
        let h = harness(client.clone(), 1).await;

        let (rating, response) = h
            .service
            .call_structured::<Rating>(extract_request("Ada Lovelace, 1843"))
            .await
            .unwrap();
        assert_eq!(
            rating,
            Rating {
                name: "Ada".into(),
                score: 9
            }
        );
        assert!(response.schema_valid);
        assert!(!response.cached);

        assert_eq!(row_count(&h.store, "llm_calls").await, 1);
        assert_eq!(row_count(&h.store, "llm_routing").await, 1);
        assert_eq!(row_count(&h.store, "llm_grammars").await, 1);
        assert_eq!(h.service.assert_complete().await.unwrap(), 1);
        let stats = h.service.stats(StatsWindow::All).await.unwrap();
        assert_eq!(stats.calls, 1);
        assert_eq!(stats.schema_rejects, 0);
        assert!(stats.tokens_in > 0 && stats.tokens_out > 0);
        assert!(stats.cost_micros >= 0);
        assert_eq!(stats.by_purpose["extract"].calls, 1);
        let provenance = h.graph.dump_turtle("provenance").await.unwrap();
        assert!(provenance.contains("#LlmCall"), "{provenance}");
        assert!(
            provenance.contains("#promptHash"),
            "the graph carries the hash, not the prompt: {provenance}"
        );
        assert!(!provenance.contains("Ada Lovelace"), "{provenance}");
        h.graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_payload_that_never_satisfies_its_schema_is_rejected_not_coerced() {
        let client = Arc::new(MockClient::constant(ScriptedResponse::text(
            r#"{"name": "Ada", "score": 99, "extra": true}"#,
        )));
        let h = harness(client.clone(), 1).await;

        let err = h
            .service
            .call_structured::<Rating>(extract_request("Ada"))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "schema_rejected");
        assert!(err.to_string().contains("rating.v1"), "{err}");

        // The repair pass asked again and got the same unsatisfying answer.
        assert_eq!(client.calls(), 2);
        assert_eq!(row_count(&h.store, "llm_repair_attempts").await, 1);
        assert_eq!(row_count(&h.store, "llm_calls").await, 1);
        let stats = h.service.stats(StatsWindow::All).await.unwrap();
        assert_eq!(stats.schema_rejects, 1);
        assert_eq!(stats.calls, 1);
        // The ledger row is still complete: a rejected call is a call that happened.
        assert_eq!(h.service.assert_complete().await.unwrap(), 1);
        h.graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_repair_that_succeeds_is_recorded_as_accepted() {
        let client = Arc::new(MockClient::script(vec![
            ScriptedResponse::text(r#"{"name":"Ada","score":99}"#),
            ScriptedResponse::text(r#"{"name":"Ada","score":9}"#),
        ]));
        let h = harness(client.clone(), 1).await;
        let (rating, _) = h
            .service
            .call_structured::<Rating>(extract_request("Ada"))
            .await
            .unwrap();
        assert_eq!(rating.score, 9);
        assert_eq!(client.calls(), 2);
        let index: &dyn mm_core::Tabular = &h.store;
        let rows = index
            .query_json(
                "SELECT accepted FROM llm_repair_attempts",
                mm_core::Params::new(),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["accepted"], 1);
        let stats = h.service.stats(StatsWindow::All).await.unwrap();
        assert_eq!(stats.schema_rejects, 0);
        h.graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_second_identical_call_is_served_from_the_exact_cache() {
        let client = Arc::new(MockClient::constant(ScriptedResponse::text(
            r#"{"name":"Ada","score":9}"#,
        )));
        let h = harness(client.clone(), 1).await;
        let first = h
            .service
            .call_structured::<Rating>(extract_request("Ada"))
            .await
            .unwrap()
            .1;
        let second = h
            .service
            .call_structured::<Rating>(extract_request("Ada"))
            .await
            .unwrap()
            .1;
        assert!(!first.cached);
        assert!(second.cached);
        assert_eq!(second.text, first.text);
        assert_eq!(client.calls(), 1, "the provider was asked exactly once");
        assert_eq!(row_count(&h.store, "llm_calls").await, 2);
        assert_eq!(row_count(&h.store, "llm_cache_index").await, 1);
        let stats = h.service.stats(StatsWindow::All).await.unwrap();
        assert_eq!(stats.calls, 2);
        assert_eq!(stats.cache_hits, 1);
        // The cache row points at a body that re-hashes identically.
        let report = cached::verify_index(&h.store, h.service.cache().dir())
            .await
            .unwrap();
        assert!(report.is_clean(), "{report:?}");
        h.graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_provider_failure_is_accounted_with_its_kind() {
        let client = Arc::new(MockClient::constant(ScriptedResponse::failing(
            "connection reset",
        )));
        let h = harness(client.clone(), 1).await;
        let err = h.service.call(extract_request("Ada")).await.unwrap_err();
        assert_eq!(err.kind(), "transport");
        assert_eq!(row_count(&h.store, "llm_calls").await, 1);
        assert_eq!(h.service.assert_complete().await.unwrap(), 1);
        let index: &dyn mm_core::Tabular = &h.store;
        let rows = index
            .query_json(
                "SELECT status, error_kind FROM llm_calls",
                mm_core::Params::new(),
            )
            .await
            .unwrap();
        assert_eq!(rows[0]["status"], "provider_error");
        assert_eq!(rows[0]["error_kind"], "transport");
        h.graph.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_calls_each_leave_exactly_one_row() {
        let client = Arc::new(MockClient::constant(ScriptedResponse::text(
            r#"{"name":"Ada","score":9}"#,
        )));
        let h = harness(client.clone(), 0).await;
        let service = Arc::new(h.service);
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..8 {
            let service = Arc::clone(&service);
            tasks.spawn(async move {
                let req = extract_request(&format!("subject {i}")).with_cache(CacheMode::Bypass);
                service.call_structured::<Rating>(req).await
            });
        }
        let mut ok = 0;
        while let Some(joined) = tasks.join_next().await {
            let outcome = joined.unwrap();
            assert!(outcome.is_ok(), "{:?}", outcome.err());
            ok += 1;
        }
        assert_eq!(ok, 8);
        assert_eq!(row_count(&h.store, "llm_calls").await, 8);
        assert_eq!(service.assert_complete().await.unwrap(), 8);
        let stats = service.stats(StatsWindow::All).await.unwrap();
        assert_eq!(stats.calls, 8);
        assert_eq!(
            stats.calls,
            stats.by_purpose.values().map(|p| p.calls).sum::<u64>(),
            "per-purpose rows must reconcile with the total"
        );
        h.graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_semantic_cache_only_serves_a_declared_purpose() {
        let client = Arc::new(MockClient::script(vec![ScriptedResponse::text(
            "summary one",
        )]));
        let h = harness(client.clone(), 0).await;
        // 0.85 because a bag-of-tokens embedding scores a one-word extension at
        // sqrt(3/4) ≈ 0.866; the threshold is still the thing making the decision.
        let semantic = SemanticCache::new(0.85, vec![Purpose::Summarize]);
        let service = h.service.with_semantic_cache(Some(semantic));

        let request = LlmRequest::new(
            Purpose::Summarize,
            vec![Message::user("summarize the gate")],
        );
        let first = service.call(request).await.unwrap();
        assert!(!first.cached);
        // A *similar* prompt is an exact-cache miss but a semantic hit, so the
        // provider is not asked again.
        let near = LlmRequest::new(
            Purpose::Summarize,
            vec![Message::user("summarize the gate please")],
        );
        let second = service.call(near).await.unwrap();
        assert!(second.cached);
        assert_eq!(second.text, "summary one");
        assert_eq!(client.calls(), 1);
        assert_eq!(service.stats(StatsWindow::All).await.unwrap().cache_hits, 1);

        // A purpose the configuration does not list never reaches the index.
        let consequential =
            LlmRequest::new(Purpose::Plan, vec![Message::user("summarize the gate")]);
        let third = service.call(consequential).await.unwrap_err();
        assert_eq!(third.kind(), "transport", "the script is exhausted");
        h.graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_response_record_carries_its_counts_and_leaks_nothing() {
        let client = Arc::new(MockClient::constant(
            ScriptedResponse::text(r#"{"name":"Ada","score":9}"#).with_tokens(16, 7),
        ));
        let h = harness(client, 1).await;
        h.service
            .call_structured::<Rating>(extract_request("Ada Lovelace"))
            .await
            .unwrap();

        let responses = emitted(&h, codes::LLM_RESPONSE);
        assert_eq!(responses.len(), 1, "exactly one response record per call");
        let record = &responses[0];
        // The numbers are the point of the record, so they must survive redaction.
        assert_eq!(record["fields"]["usage"]["in"], 16, "{record}");
        assert_eq!(record["fields"]["usage"]["out"], 7, "{record}");
        assert_eq!(record["fields"]["status"], "ok");

        // Nothing in any sink was redacted, and no secret name appears.
        let raw = std::fs::read_to_string(&h.log_path).unwrap();
        assert!(!raw.contains("[redacted]"), "{raw}");
        assert_eq!(h.service.store().audit_count().await.unwrap(), 1);
        h.graph.shutdown().await.unwrap();
    }

    #[test]
    fn a_relative_cache_dir_resolves_against_the_root() {
        assert_eq!(
            resolve_cache_dir(Path::new("/repo"), Path::new("data/llm_cache")),
            PathBuf::from("/repo/data/llm_cache")
        );
        assert_eq!(
            resolve_cache_dir(Path::new("/repo"), Path::new("/abs")),
            PathBuf::from("/abs")
        );
    }
}
