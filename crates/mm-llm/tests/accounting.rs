//! The ledger: one complete row per call, and numbers that reconcile.
//!
//! The property this file exists for is *conservation*: whatever a call did —
//! succeeded, been served from cache, or been rejected — it leaves exactly one
//! `llm_calls` row, exactly one audited commit record, and its tokens and cost
//! sum into the totals. A call that vanishes, or is counted twice, is the failure
//! mode that makes every other number in the system untrustworthy.

mod common;

use std::sync::Arc;

use common::{fixture, Harness};
use mm_llm::accounting::{self, CallAccount, CallStatus, StatsWindow};
use mm_llm::client::{CacheMode, LlmRequest, Message, Purpose, SchemaId};
use mm_llm::mock::{MockClient, ScriptedResponse};
use mm_llm::semantic_cache::SemanticCache;

fn request(text: &str) -> LlmRequest {
    LlmRequest::new(Purpose::Extract, vec![Message::user(text)])
}

#[tokio::test]
async fn every_call_leaves_exactly_one_row_and_one_audit_record() {
    let h = Harness::new(1).await;
    let client = Arc::new(MockClient::constant(ScriptedResponse::text(
        r#"{"name":"Ada","score":9}"#,
    )));
    let mut service = h.service_with(client.clone());
    service
        .register_schema(
            SchemaId::new("extract.v1"),
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "score"],
                "properties": {
                    "name": { "type": "string", "minLength": 1 },
                    "score": { "type": "integer", "minimum": 0, "maximum": 10 }
                }
            }),
        )
        .unwrap();

    // Three successes and one cache hit.
    for who in ["Ada", "Grace", "Alan", "Ada"] {
        service
            .call_structured::<Extract>(request(who).with_schema(SchemaId::new("extract.v1")))
            .await
            .unwrap();
    }
    assert_eq!(client.calls(), 3, "the fourth call was a cache hit");

    let ledger = h.ledger_call_ids().await;
    let committed = h.committed_call_ids().await;
    assert_eq!(ledger.len(), 4);
    assert_eq!(committed.len(), 4, "one audited commit record per row");
    assert_eq!(
        ledger, committed,
        "the ledger and the audit chain name the same calls"
    );

    let stats = accounting::stats(&h.store, &StatsWindow::All)
        .await
        .unwrap();
    assert_eq!(stats.calls, 4);
    assert_eq!(stats.cache_hits, 1);
    assert_eq!(
        stats.calls,
        stats.by_purpose.values().map(|p| p.calls).sum::<u64>()
    );
    assert_eq!(
        stats.tokens_in,
        stats.by_purpose.values().map(|p| p.tokens_in).sum::<u64>()
    );
    assert_eq!(accounting::assert_complete(&h.store).await.unwrap(), 4);
    h.shutdown().await;
}

#[tokio::test]
async fn a_rejected_call_is_accounted_and_a_provider_failure_too() {
    let h = Harness::new(1).await;

    // A schema violation.
    let bad = Arc::new(MockClient::constant(ScriptedResponse::text(
        r#"{"name":"Ada"}"#,
    )));
    let mut service = h.service_with(bad);
    service
        .register_schema(
            SchemaId::new("extract.v1"),
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "score"],
                "properties": {
                    "name": { "type": "string", "minLength": 1 },
                    "score": { "type": "integer", "minimum": 0, "maximum": 10 }
                }
            }),
        )
        .unwrap();
    let err = service
        .call(request("Ada").with_schema(SchemaId::new("extract.v1")))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "schema_rejected");

    // A provider that cannot be reached.
    let broken = Arc::new(MockClient::constant(ScriptedResponse::failing(
        "connection reset",
    )));
    let service = h.service_with(broken);
    let err = service.call(request("Grace")).await.unwrap_err();
    assert_eq!(err.kind(), "transport");

    let stats = accounting::stats(&h.store, &StatsWindow::All)
        .await
        .unwrap();
    assert_eq!(stats.calls, 2, "a failed call is still a call");
    assert_eq!(stats.schema_rejects, 1);
    assert!(stats.cost_micros >= 0);
    assert_eq!(accounting::assert_complete(&h.store).await.unwrap(), 2);

    // Every failure carries its kind, so the ledger says *why*.
    let index: &dyn mm_core::Tabular = &h.store;
    let rows = index
        .query_json(
            "SELECT status, error_kind FROM llm_calls ORDER BY status",
            mm_core::Params::new(),
        )
        .await
        .unwrap();
    let pairs: Vec<(String, String)> = rows
        .iter()
        .map(|row| {
            (
                row["status"].as_str().unwrap_or_default().to_string(),
                row["error_kind"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert!(
        pairs.contains(&("provider_error".into(), "transport".into())),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&("schema_rejected".into(), "schema_rejected".into())),
        "{pairs:?}"
    );
    h.shutdown().await;
}

#[tokio::test]
async fn a_row_that_is_not_complete_is_reported_not_ignored() {
    let h = Harness::new(0).await;
    let mut account = CallAccount::started(
        accounting::new_call_id(),
        Purpose::Plan,
        "mock",
        &"a".repeat(64),
    );
    account.model = String::new();
    accounting::commit(&h.store, &account).await.unwrap();
    let err = accounting::assert_complete(&h.store).await.unwrap_err();
    assert!(err.to_string().contains("incomplete"), "{err}");
    h.shutdown().await;
}

#[tokio::test]
async fn a_cache_hit_costs_nothing_and_is_recorded_as_such() {
    let h = Harness::new(0).await;
    let client = Arc::new(MockClient::constant(
        ScriptedResponse::text("an answer").with_tokens(10, 4),
    ));
    let service = h.service_with(client);
    let first = service.call(request("hello")).await.unwrap();
    let second = service.call(request("hello")).await.unwrap();
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(second.usage.cost_micros, 0);

    let stats = accounting::stats(&h.store, &StatsWindow::All)
        .await
        .unwrap();
    assert_eq!(stats.calls, 2);
    assert_eq!(stats.cache_hits, 1);
    let index: &dyn mm_core::Tabular = &h.store;
    let rows = index
        .query_json(
            "SELECT cached, cache_layer, cost_micros FROM llm_calls ORDER BY cached",
            mm_core::Params::new(),
        )
        .await
        .unwrap();
    assert_eq!(rows[1]["cached"], 1);
    assert_eq!(rows[1]["cache_layer"], "exact");
    assert_eq!(rows[1]["cost_micros"], 0);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_calls_do_not_lose_or_duplicate_a_row() {
    let h = Harness::new(0).await;
    let client = Arc::new(MockClient::constant(
        ScriptedResponse::text("an answer").with_tokens(10, 4),
    ));
    let service = Arc::new(h.service_with(client.clone()));

    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..12 {
        let service = Arc::clone(&service);
        tasks.spawn(async move {
            service
                .call(request(&format!("subject {i}")).with_cache(CacheMode::Bypass))
                .await
        });
    }
    let mut done = 0;
    while let Some(joined) = tasks.join_next().await {
        let outcome = joined.unwrap();
        assert!(outcome.is_ok(), "{:?}", outcome.err());
        done += 1;
    }
    assert_eq!(done, 12);
    assert_eq!(h.store.row_count("llm_calls").await.unwrap(), 12);
    assert_eq!(h.committed_call_ids().await.len(), 12);
    assert_eq!(accounting::assert_complete(&h.store).await.unwrap(), 12);
    assert_eq!(client.calls(), 12);
    h.shutdown().await;
}

#[tokio::test]
async fn the_stats_window_filters_without_changing_the_totals() {
    let h = Harness::new(0).await;
    let client = Arc::new(MockClient::constant(ScriptedResponse::text("an answer")));
    let service = h.service_with(client);
    service
        .call(LlmRequest::new(Purpose::Plan, vec![Message::user("a")]))
        .await
        .unwrap();
    service
        .call(LlmRequest::new(
            Purpose::Summarize,
            vec![Message::user("b")],
        ))
        .await
        .unwrap();

    let all = accounting::stats(&h.store, &StatsWindow::All)
        .await
        .unwrap();
    let plans = accounting::stats(&h.store, &StatsWindow::Purpose(Purpose::Plan))
        .await
        .unwrap();
    assert_eq!(all.calls, 2);
    assert_eq!(plans.calls, 1);
    assert_eq!(plans.by_purpose.len(), 1);
    assert_eq!(plans.tokens_in, all.by_purpose["plan"].tokens_in);
    h.shutdown().await;
}

#[tokio::test]
async fn a_semantic_hit_is_labelled_and_still_leaves_one_row() {
    let h = Harness::new(0).await;
    let client = Arc::new(MockClient::constant(ScriptedResponse::text("a summary")));
    let service = h
        .service_with(client.clone())
        .with_semantic_cache(Some(SemanticCache::new(0.8, vec![Purpose::Summarize])));
    let first = service
        .call(LlmRequest::new(
            Purpose::Summarize,
            vec![Message::user("summarize the gate")],
        ))
        .await
        .unwrap();
    assert!(!first.cached);
    let second = service
        .call(LlmRequest::new(
            Purpose::Summarize,
            vec![Message::user("summarize the gate please")],
        ))
        .await
        .unwrap();
    assert!(second.cached);
    assert_eq!(client.calls(), 1);

    let index: &dyn mm_core::Tabular = &h.store;
    let rows = index
        .query_json(
            "SELECT cache_layer FROM llm_calls WHERE cached = 1",
            mm_core::Params::new(),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["cache_layer"], "semantic");
    h.shutdown().await;
}

#[tokio::test]
async fn the_ledger_and_the_provenance_graph_agree() {
    let h = Harness::new(0).await;
    let client = Arc::new(MockClient::constant(ScriptedResponse::text("an answer")));
    let service = h.service_with(client);
    service.call(request("Ada Lovelace")).await.unwrap();
    service.call(request("Grace Hopper")).await.unwrap();

    assert_eq!(h.store.row_count("llm_calls").await.unwrap(), 2);
    let provenance = h.graph.dump_turtle("provenance").await.unwrap();
    assert_eq!(
        provenance.matches("#LlmCall").count(),
        2,
        "one mm:LlmCall node per ledger row"
    );
    // The graph carries the hash, never the prompt.
    assert!(!provenance.contains("Ada Lovelace"), "{provenance}");
    assert!(provenance.contains("#promptHash"), "{provenance}");
    h.shutdown().await;
}

/// The fixture path is used by the other files; this keeps the module honest about
/// what it needs without an unused-import warning.
#[test]
fn the_fixture_root_resolves() {
    assert!(fixture("bench/llm/malformed").is_dir());
}

#[derive(Debug, serde::Deserialize)]
struct Extract {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    score: i64,
}

impl mm_llm::schema::StructuredOut for Extract {
    fn schema_id() -> SchemaId {
        SchemaId::new("extract.v1")
    }
    fn schema() -> serde_json::Value {
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

/// `CallStatus` is part of the stored vocabulary, so a rename is caught here.
#[test]
fn call_statuses_round_trip() {
    for status in [
        CallStatus::Ok,
        CallStatus::SchemaRejected,
        CallStatus::ProviderError,
        CallStatus::Timeout,
        CallStatus::Replay,
    ] {
        assert_eq!(CallStatus::parse(status.as_str()), Some(status));
    }
}
