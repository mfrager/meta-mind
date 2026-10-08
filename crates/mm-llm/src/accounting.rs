//! The call ledger.
//!
//! Every call — successful, cached, rejected, or failed — leaves exactly one row.
//! That is the whole point: a call that is not in `llm_calls` did not happen as far
//! as the being is concerned, and a call that is in it must be complete enough to
//! audit. `assert_complete` is the check that makes "exactly one complete row" a
//! property the gate can verify rather than a hope.

use std::collections::BTreeMap;

use mm_core::{Param, Params, Tabular};
use mm_store_sqlite::SqliteStore;
use serde::Serialize;
use ulid::Ulid;

use crate::client::{DecoderKind, Purpose};
use crate::error::LlmError;

/// A fresh call identifier.
///
/// The substrate mints raw ULIDs rather than sharing the kernel's watermark
/// factory: a call id must be unique, not globally ordered against the event log,
/// and reaching for the kernel's factory here would make the LLM layer depend on
/// kernel boot order for no benefit.
pub fn new_call_id() -> Ulid {
    Ulid::from_datetime(std::time::SystemTime::now())
}

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    /// The provider (or cache) answered and the output was acceptable.
    Ok,
    /// The output did not satisfy its schema and repair did not fix it.
    SchemaRejected,
    /// The provider could not be reached, was not configured, or errored.
    ProviderError,
    /// The provider did not answer in time.
    Timeout,
    /// The answer came from a recorded session.
    Replay,
}

impl CallStatus {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            CallStatus::Ok => "ok",
            CallStatus::SchemaRejected => "schema_rejected",
            CallStatus::ProviderError => "provider_error",
            CallStatus::Timeout => "timeout",
            CallStatus::Replay => "replay",
        }
    }

    /// Parse the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(CallStatus::Ok),
            "schema_rejected" => Some(CallStatus::SchemaRejected),
            "provider_error" => Some(CallStatus::ProviderError),
            "timeout" => Some(CallStatus::Timeout),
            "replay" => Some(CallStatus::Replay),
            _ => None,
        }
    }
}

/// One row of `llm_calls`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallAccount {
    /// The call's identifier.
    pub call_id: Ulid,
    /// The trace it belongs to.
    pub trace_id: Option<Ulid>,
    /// Why it was made.
    pub purpose: Purpose,
    /// The provider that answered (or was asked).
    pub provider: String,
    /// The model used.
    pub model: String,
    /// sha256 over the canonical request.
    pub prompt_hash: String,
    /// The schema the output had to satisfy.
    pub schema_id: Option<String>,
    /// Whether the output satisfied it.
    pub schema_ok: bool,
    /// Which decoder was used.
    pub decoder: DecoderKind,
    /// Whether the answer was cached.
    pub cached: bool,
    /// `exact`, `semantic`, or `none`.
    pub cache_layer: Option<String>,
    /// The model that was asked for, when routing changed it.
    pub routed_from: Option<String>,
    /// Why the router chose what it chose.
    pub route_reason: Option<String>,
    /// Prompt tokens.
    pub tokens_in: u32,
    /// Completion tokens.
    pub tokens_out: u32,
    /// Cost in micros.
    pub cost_micros: i64,
    /// Latency in milliseconds.
    pub latency_ms: u32,
    /// How the call ended.
    pub status: CallStatus,
    /// The error kind, when it failed.
    pub error_kind: Option<String>,
}

impl CallAccount {
    /// A minimal account for a call that has not completed yet.
    pub fn started(call_id: Ulid, purpose: Purpose, provider: &str, prompt_hash: &str) -> Self {
        CallAccount {
            call_id,
            trace_id: None,
            purpose,
            provider: provider.to_string(),
            model: String::new(),
            prompt_hash: prompt_hash.to_string(),
            schema_id: None,
            schema_ok: true,
            decoder: DecoderKind::None,
            cached: false,
            cache_layer: None,
            routed_from: None,
            route_reason: None,
            tokens_in: 0,
            tokens_out: 0,
            cost_micros: 0,
            latency_ms: 0,
            status: CallStatus::Ok,
            error_kind: None,
        }
    }
}

/// One row of `llm_routing`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoutingRecord {
    /// The call it belongs to, when there was one.
    pub call_id: Option<Ulid>,
    /// The model the request asked for, if any.
    pub requested: Option<String>,
    /// The model the router selected.
    pub selected: String,
    /// Why.
    pub reason: String,
    /// The estimated cost.
    pub cost_micros: i64,
}

/// One row of `llm_repair_attempts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepairRecord {
    /// The call being repaired.
    pub call_id: Ulid,
    /// 1-based attempt number.
    pub attempt_no: u32,
    /// What was wrong.
    pub error_kind: String,
    /// Whether the attempt produced acceptable output.
    pub accepted: bool,
}

/// One row of `llm_grammars`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GrammarRecord {
    /// The schema the grammar encodes.
    pub schema_id: String,
    /// The schema's hash.
    pub schema_sha: String,
    /// The compiled grammar.
    pub grammar: String,
    /// Which decoder compiled it.
    pub decoder: String,
}

fn text(value: &str) -> Param {
    Param::Text(value.to_string())
}

fn sql(e: mm_core::MmError) -> LlmError {
    LlmError::Db(e.to_string())
}

fn optional_ulid(value: Option<Ulid>) -> Param {
    Param::opt_text(value.map(|u| mm_core::ulid_string(&u)))
}

/// Write one `llm_calls` row.
pub async fn commit(store: &SqliteStore, account: &CallAccount) -> Result<(), LlmError> {
    let index: &dyn Tabular = store;
    let created = mm_core::ulid_string(&account.call_id);
    index
        .execute(
            "INSERT INTO llm_calls \
             (id, trace_id, purpose, provider, model, prompt_hash, schema_id, decoder, schema_ok, \
              routed_from, route_reason, cached, cache_layer, tokens_in, tokens_out, cost_micros, \
              latency_ms, status, error_kind, created_ulid, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                text(&created),
                optional_ulid(account.trace_id),
                text(account.purpose.as_str()),
                text(&account.provider),
                text(&account.model),
                text(&account.prompt_hash),
                Param::opt_text(account.schema_id.clone()),
                text(account.decoder.as_str()),
                Param::Int(i64::from(account.schema_ok)),
                Param::opt_text(account.routed_from.clone()),
                Param::opt_text(account.route_reason.clone()),
                Param::Int(i64::from(account.cached)),
                Param::opt_text(account.cache_layer.clone()),
                Param::Int(i64::from(account.tokens_in)),
                Param::Int(i64::from(account.tokens_out)),
                Param::Int(account.cost_micros),
                Param::Int(i64::from(account.latency_ms)),
                text(account.status.as_str()),
                Param::opt_text(account.error_kind.clone()),
                text(&created),
                text(&mm_core::Timestamp::now().to_rfc3339()),
            ],
        )
        .await
        .map_err(sql)?;
    Ok(())
}

/// Record a routing decision.
pub async fn record_routing(store: &SqliteStore, record: &RoutingRecord) -> Result<(), LlmError> {
    let index: &dyn Tabular = store;
    index
        .execute(
            "INSERT INTO llm_routing (id, call_id, requested, selected, reason, cost_micros, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                text(&mm_core::ulid_string(&new_call_id())),
                optional_ulid(record.call_id),
                Param::opt_text(record.requested.clone()),
                text(&record.selected),
                text(&record.reason),
                Param::Int(record.cost_micros),
                text(&mm_core::ulid_string(&new_call_id())),
            ],
        )
        .await
        .map_err(sql)?;
    Ok(())
}

/// Record a repair attempt.
pub async fn record_repair(store: &SqliteStore, record: &RepairRecord) -> Result<(), LlmError> {
    let index: &dyn Tabular = store;
    index
        .execute(
            "INSERT INTO llm_repair_attempts (id, call_id, attempt_no, error_kind, accepted) \
             VALUES (?, ?, ?, ?, ?)",
            vec![
                text(&mm_core::ulid_string(&new_call_id())),
                text(&mm_core::ulid_string(&record.call_id)),
                Param::Int(i64::from(record.attempt_no)),
                text(&record.error_kind),
                Param::Int(i64::from(record.accepted)),
            ],
        )
        .await
        .map_err(sql)?;
    Ok(())
}

/// Persist a compiled grammar, keyed by schema id.
pub async fn record_grammar(store: &SqliteStore, record: &GrammarRecord) -> Result<(), LlmError> {
    let index: &dyn Tabular = store;
    index
        .execute(
            "INSERT OR REPLACE INTO llm_grammars (schema_id, schema_sha, grammar, decoder, created_ulid) \
             VALUES (?, ?, ?, ?, ?)",
            vec![
                text(&record.schema_id),
                text(&record.schema_sha),
                text(&record.grammar),
                text(&record.decoder),
                text(&mm_core::ulid_string(&new_call_id())),
            ],
        )
        .await
        .map_err(sql)?;
    Ok(())
}

/// How much of the ledger to aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StatsWindow {
    /// Everything.
    #[default]
    All,
    /// Everything created at or after this instant.
    Since(String),
    /// One purpose.
    Purpose(Purpose),
}

/// Per-purpose totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PurposeStats {
    /// Calls made.
    pub calls: u64,
    /// Prompt tokens.
    pub tokens_in: u64,
    /// Completion tokens.
    pub tokens_out: u64,
    /// Cost in micros.
    pub cost_micros: i64,
    /// Calls whose output was rejected.
    pub schema_rejects: u64,
    /// Calls that hit a cache.
    pub cache_hits: u64,
}

/// Ledger totals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LlmStats {
    /// Calls recorded.
    pub calls: u64,
    /// Prompt tokens across calls.
    pub tokens_in: u64,
    /// Completion tokens across calls.
    pub tokens_out: u64,
    /// Cost across calls.
    pub cost_micros: i64,
    /// Calls whose output was rejected.
    pub schema_rejects: u64,
    /// Calls that hit a cache.
    pub cache_hits: u64,
    /// Per-purpose breakdown.
    pub by_purpose: BTreeMap<String, PurposeStats>,
}

/// Aggregate the ledger.
pub async fn stats(store: &SqliteStore, window: &StatsWindow) -> Result<LlmStats, LlmError> {
    let index: &dyn Tabular = store;
    let mut where_clause = String::new();
    let params: Vec<Param> = match window {
        StatsWindow::All => Vec::new(),
        StatsWindow::Since(instant) => {
            where_clause.push_str(" WHERE created_at >= ?");
            vec![text(instant)]
        }
        StatsWindow::Purpose(purpose) => {
            where_clause.push_str(" WHERE purpose = ?");
            vec![text(purpose.as_str())]
        }
    };

    let rows = index
        .query_json(
            &format!(
                "SELECT purpose, COUNT(*) AS calls, \
                 COALESCE(SUM(tokens_in), 0) AS tokens_in, \
                 COALESCE(SUM(tokens_out), 0) AS tokens_out, \
                 COALESCE(SUM(cost_micros), 0) AS cost_micros, \
                 COALESCE(SUM(CASE WHEN status = 'schema_rejected' THEN 1 ELSE 0 END), 0) AS schema_rejects, \
                 COALESCE(SUM(cached), 0) AS cache_hits \
                 FROM llm_calls{where_clause} GROUP BY purpose ORDER BY purpose"
            ),
            params,
        )
        .await
        .map_err(sql)?;

    let mut out = LlmStats::default();
    for row in &rows {
        let purpose = row
            .get("purpose")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let per = PurposeStats {
            calls: int_of(row, "calls"),
            tokens_in: int_of(row, "tokens_in"),
            tokens_out: int_of(row, "tokens_out"),
            cost_micros: int_of(row, "cost_micros") as i64,
            schema_rejects: int_of(row, "schema_rejects"),
            cache_hits: int_of(row, "cache_hits"),
        };
        out.calls += per.calls;
        out.tokens_in += per.tokens_in;
        out.tokens_out += per.tokens_out;
        out.cost_micros += per.cost_micros;
        out.schema_rejects += per.schema_rejects;
        out.cache_hits += per.cache_hits;
        out.by_purpose.insert(purpose, per);
    }
    Ok(out)
}

fn int_of(row: &serde_json::Value, key: &str) -> u64 {
    row.get(key)
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|n| n.max(0) as u64)))
        .unwrap_or(0)
}

/// Verify every ledger row is complete, returning how many rows there are.
///
/// "Complete" is the gate's criterion: a row with no model or no prompt hash is a
/// call nothing can be said about, so it fails rather than counting as a call.
pub async fn assert_complete(store: &SqliteStore) -> Result<u64, LlmError> {
    let index: &dyn Tabular = store;
    let rows = index
        .query_json(
            "SELECT COUNT(*) AS total, \
             COALESCE(SUM(CASE WHEN COALESCE(model, '') = '' THEN 1 ELSE 0 END), 0) AS no_model, \
             COALESCE(SUM(CASE WHEN COALESCE(prompt_hash, '') = '' THEN 1 ELSE 0 END), 0) AS no_hash, \
             COALESCE(SUM(CASE WHEN COALESCE(purpose, '') = '' THEN 1 ELSE 0 END), 0) AS no_purpose, \
             COALESCE(SUM(CASE WHEN COALESCE(status, '') = '' THEN 1 ELSE 0 END), 0) AS no_status, \
             COALESCE(SUM(CASE WHEN tokens_in IS NULL OR tokens_out IS NULL THEN 1 ELSE 0 END), 0) AS no_tokens, \
             COALESCE(SUM(CASE WHEN cost_micros IS NULL THEN 1 ELSE 0 END), 0) AS no_cost, \
             COALESCE(SUM(CASE WHEN latency_ms IS NULL THEN 1 ELSE 0 END), 0) AS no_latency \
             FROM llm_calls",
            Params::new(),
        )
        .await
        .map_err(sql)?;
    let row = rows.first().cloned().unwrap_or(serde_json::json!({}));
    let total = int_of(&row, "total");
    let mut broken = 0u64;
    for key in [
        "no_model",
        "no_hash",
        "no_purpose",
        "no_status",
        "no_tokens",
        "no_cost",
        "no_latency",
    ] {
        broken += int_of(&row, key);
    }
    if broken > 0 {
        return Err(LlmError::Db(format!(
            "{broken} of {total} llm_calls row(s) are incomplete"
        )));
    }
    Ok(total)
}

/// The number of ledger rows (used by tests and the CLI).
pub async fn call_count(store: &SqliteStore) -> Result<u64, LlmError> {
    let index: &dyn Tabular = store;
    let rows = index
        .query_json("SELECT COUNT(*) AS n FROM llm_calls", Params::new())
        .await
        .map_err(sql)?;
    Ok(rows.first().map(|r| int_of(r, "n")).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("m.db")).await.unwrap();
        store.migrate().await.unwrap();
        (dir, store)
    }

    fn account(id: Ulid, purpose: Purpose, status: CallStatus) -> CallAccount {
        let mut a = CallAccount::started(id, purpose, "mock", &"a".repeat(64));
        a.model = "small".into();
        a.tokens_in = 10;
        a.tokens_out = 5;
        a.cost_micros = 12;
        a.latency_ms = 7;
        a.status = status;
        a
    }

    #[test]
    fn statuses_round_trip() {
        for status in [
            CallStatus::Ok,
            CallStatus::SchemaRejected,
            CallStatus::ProviderError,
            CallStatus::Timeout,
            CallStatus::Replay,
        ] {
            assert_eq!(CallStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(CallStatus::parse("nope"), None);
    }

    #[tokio::test]
    async fn one_complete_row_per_call_and_stats_reconcile() {
        let (_dir, store) = store().await;
        commit(
            &store,
            &account(new_call_id(), Purpose::Plan, CallStatus::Ok),
        )
        .await
        .unwrap();
        commit(
            &store,
            &account(new_call_id(), Purpose::Plan, CallStatus::SchemaRejected),
        )
        .await
        .unwrap();
        let mut cached = account(new_call_id(), Purpose::Summarize, CallStatus::Ok);
        cached.cached = true;
        cached.cache_layer = Some("exact".into());
        commit(&store, &cached).await.unwrap();

        assert_eq!(call_count(&store).await.unwrap(), 3);
        assert_eq!(assert_complete(&store).await.unwrap(), 3);

        let all = stats(&store, &StatsWindow::All).await.unwrap();
        assert_eq!(all.calls, 3);
        assert_eq!(all.tokens_in, 30);
        assert_eq!(all.tokens_out, 15);
        assert_eq!(all.cost_micros, 36);
        assert_eq!(all.schema_rejects, 1);
        assert_eq!(all.cache_hits, 1);
        assert_eq!(all.by_purpose.len(), 2);
        assert_eq!(all.by_purpose["plan"].calls, 2);
        // The per-purpose rows must sum to the totals.
        let summed: u64 = all.by_purpose.values().map(|p| p.calls).sum();
        assert_eq!(summed, all.calls);

        let only_plan = stats(&store, &StatsWindow::Purpose(Purpose::Plan))
            .await
            .unwrap();
        assert_eq!(only_plan.calls, 2);
        assert_eq!(only_plan.by_purpose.len(), 1);
        store.close().await;
    }

    #[tokio::test]
    async fn an_incomplete_row_fails_the_completeness_check() {
        let (_dir, store) = store().await;
        let mut broken = account(new_call_id(), Purpose::Plan, CallStatus::Ok);
        broken.model = String::new();
        commit(&store, &broken).await.unwrap();
        let err = assert_complete(&store).await.unwrap_err();
        assert!(err.to_string().contains("incomplete"), "{err}");
        store.close().await;
    }

    #[tokio::test]
    async fn routing_repair_and_grammar_records_persist() {
        let (_dir, store) = store().await;
        let call_id = new_call_id();
        record_routing(
            &store,
            &RoutingRecord {
                call_id: Some(call_id),
                requested: None,
                selected: "small".into(),
                reason: "cheapest_sufficient".into(),
                cost_micros: 500,
            },
        )
        .await
        .unwrap();
        // A repair row references the call it repaired, and foreign keys are
        // enforced, so the call must exist first.
        commit(&store, &account(call_id, Purpose::Plan, CallStatus::Ok))
            .await
            .unwrap();
        record_repair(
            &store,
            &RepairRecord {
                call_id,
                attempt_no: 1,
                error_kind: "extra_field".into(),
                accepted: false,
            },
        )
        .await
        .unwrap();
        record_grammar(
            &store,
            &GrammarRecord {
                schema_id: "rating.v1".into(),
                schema_sha: "b".repeat(64),
                grammar: "root ::= object".into(),
                decoder: "llguidance".into(),
            },
        )
        .await
        .unwrap();
        let index: &dyn Tabular = &store;
        for table in ["llm_routing", "llm_repair_attempts", "llm_grammars"] {
            let rows = index
                .query_json(&format!("SELECT COUNT(*) AS n FROM {table}"), Params::new())
                .await
                .unwrap();
            assert_eq!(int_of(&rows[0], "n"), 1, "{table}");
        }
        store.close().await;
    }
}
