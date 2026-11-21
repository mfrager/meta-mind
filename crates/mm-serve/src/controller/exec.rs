//! In-process reads. Each arm calls the subsystem the CLI calls.

use std::time::Duration;

use mm_core::ulid_string;
use mm_eventlog::CountingApplier;
use mm_memory::{MemoryEngine, RecallQuery};
use mm_store_sqlite::AsOf;
use serde_json::{json, Value};

use crate::state::AppState;

use super::catalog::{self, GroupId};
use super::guide;
use super::persona;

#[derive(Clone)]
pub struct ToolResult {
    pub ok: bool,
    pub summary: String,
    pub reason: Option<String>,
    pub data: Value,
    pub focus: Option<Value>,
}

impl ToolResult {
    pub(crate) fn done(summary: impl Into<String>, data: Value) -> Self {
        ToolResult {
            ok: true,
            summary: summary.into(),
            reason: None,
            data,
            focus: None,
        }
    }

    pub(crate) fn fail(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        ToolResult {
            ok: false,
            summary: reason.clone(),
            reason: Some(reason),
            data: json!({}),
            focus: None,
        }
    }

    pub fn to_value(&self) -> Value {
        let mut body = json!({
            "ok": self.ok,
            "summary": self.summary,
            "data": self.data,
        });
        if let Some(reason) = &self.reason {
            body["reason"] = json!(reason);
        }
        if let Some(focus) = &self.focus {
            body["focus"] = focus.clone();
        }
        body
    }
}

pub async fn run(app: &AppState, name: &str, input: &Value) -> ToolResult {
    let limit = if matches!(
        name,
        "promote.run" | "sandbox.run" | "loop.run" | "gc.apply" | "gc.propose" | "debt.scan" | "codex.scan" | "codex.verify" | "regression.run"
    ) {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(30)
    };
    match tokio::time::timeout(limit, dispatch(app, name, input)).await {
        Ok(result) => result,
        Err(_) => ToolResult::fail(format!("{name} timed out")),
    }
}

async fn dispatch(app: &AppState, name: &str, input: &Value) -> ToolResult {
    match name {
        "guide.list" => guide_list(),
        "guide.read" => guide_read(input),
        "guide.search" => guide_search(input),
        "catalog.open" => catalog_open(input),
        "state.show" => state_show(app, input).await,
        "focus.open" | "focus.read" => focus_doc(app, name, input).await,
        "session.note" => session_note(input),
        "doctor.run" => doctor(app).await,
        "logs.head" => logs_head(app).await,
        "logs.verify" => logs_verify(app).await,
        "replay.run" => replay(app).await,
        "graph.query" => graph_query(app, input).await,
        "graph.validate" => graph_validate(app, input).await,
        "being.show" => being_show(app).await,
        "being.goal_list" => goal_list(app).await,
        "being.verify" => being_verify(app).await,
        "being.personality_show" => personality_show(app).await,
        "memory.recall" => memory_recall(app, input).await,
        "memory.verify" => memory_verify(app).await,
        "budget.show" => budget_show(app).await,
        "changeset.show" => changeset_show(app).await,
        "being.goal_add" => super::tools::mutate::goal_add(app, input).await,
        "memory.add" => super::tools::mutate::memory_add(app, input).await,
        "memory.forget" => super::tools::mutate::memory_forget(app, input).await,
        "decide.ask" => super::tools::mutate::decide_ask(app, input).await,
        "firewall.eval" => super::tools::mutate::firewall_eval(app, input).await,
        "compare.check" => super::tools::mutate::compare_check(input),
        "risk.analyze" => super::tools::mutate::risk_analyze(input),
        "changeset.new" => super::tools::mutate::changeset_new(app, input).await,
        "sandbox.run" => super::tools::mutate::sandbox_run(app, input).await,
        "promote.run" => super::tools::mutate::promote_run(app, input).await,
        "reject.run" => super::tools::mutate::reject_run(app, input).await,
        "audit.production_tree" => super::tools::mutate::audit_production_tree(app).await,
        "sandbox_tool.list" => super::tools::mutate::sandbox_tool_list(),
        "sandbox_tool.describe" => super::tools::mutate::sandbox_tool_describe(input),
        "sandbox_tool.run" => super::tools::mutate::sandbox_tool_run(app, input).await,
        "action.ledger" => super::tools::mutate::action_ledger(app, input).await,
        "action.rollback" => super::tools::mutate::action_rollback(app, input).await,
        "focus.patch" => super::tools::mutate::focus_patch(app, input).await,
        "epistemic.ingest" => super::tools::rest::epistemic_ingest(app, input).await,
        "epistemic.show" => super::tools::rest::epistemic_show(app).await,
        "library.applicable" => super::tools::rest::library_applicable(app, input).await,
        "policy.history" => super::tools::rest::policy_history(app, input).await,
        "skill.retrieve" => super::tools::rest::skill_retrieve(app, input).await,
        "case.retrieve" => super::tools::rest::case_retrieve(app, input).await,
        "frame.missing" => super::tools::rest::frame_missing(app, input).await,
        "calibration.show" => super::tools::rest::calibration_show(app).await,
        "conformance.run" => super::tools::rest::conformance_run(),
        "calibrate.score" => super::tools::rest::calibrate_score(app).await,
        "loop.run" => super::tools::rest::loop_run(app, input).await,
        "loop.status" => super::tools::rest::loop_status(app, input).await,
        "self_model.report" => super::tools::rest::self_model_report(app, input).await,
        "debt.scan" => super::tools::rest::debt_scan(app).await,
        "gc.propose" => super::tools::rest::gc_propose(app).await,
        "gc.apply" => super::tools::rest::gc_apply(app).await,
        "module.load" => super::tools::rest::module_load(app, input).await,
        "module.rollback" => super::tools::rest::module_rollback(app, input).await,
        "design.show" => super::tools::rest::design_show(app, input).await,
        "pi.run" => super::tools::rest::pi_run(),
        "pi.ingest" => super::tools::rest::pi_ingest(app, input).await,
        "codex.scan" => super::tools::rest::codex_scan(app),
        "codex.verify" => super::tools::rest::codex_verify(app),
        "experience.compile" => super::tools::rest::experience_compile(app, input),
        "llm.stats" => super::tools::rest::llm_stats(app).await,
        "meta.analyze" => super::tools::rest::meta_analyze(app, input).await,
        "regression.run" => super::tools::rest::regression_run(app).await,
        "verify.run" => super::tools::rest::verify_run(input),
        "mcp.list" => super::tools::rest::mcp_list(),
        other => ToolResult::fail(format!("unknown tool {other}")),
    }
}

fn guide_list() -> ToolResult {
    let chapters: Vec<Value> = guide::chapters()
        .iter()
        .map(|chapter| json!({ "id": chapter.id, "title": chapter.title }))
        .collect();
    ToolResult::done(
        format!("{} guide chapters", chapters.len()),
        json!({ "chapters": chapters }),
    )
}

fn guide_read(input: &Value) -> ToolResult {
    let id = input.get("id").and_then(Value::as_str).unwrap_or("");
    match guide::get(id) {
        Some(chapter) => ToolResult::done(
            chapter.title.to_string(),
            json!({ "id": chapter.id, "title": chapter.title, "markdown": chapter.body }),
        ),
        None => ToolResult::fail(format!("no guide chapter {id}")),
    }
}

fn guide_search(input: &Value) -> ToolResult {
    let query = input.get("query").and_then(Value::as_str).unwrap_or("");
    let hits: Vec<Value> = guide::search(query, 3)
        .into_iter()
        .map(|chunk| {
            json!({
                "id": chunk.id,
                "title": chunk.title,
                "excerpt": chunk.excerpt,
            })
        })
        .collect();
    ToolResult::done(
        format!("{} guide hits", hits.len()),
        json!({ "hits": hits }),
    )
}

fn catalog_open(input: &Value) -> ToolResult {
    let group = input.get("group").and_then(Value::as_str).unwrap_or("");
    let Some(id) = GroupId::parse(group) else {
        return ToolResult::fail(format!("unknown group {group}"));
    };
    let names: Vec<&str> = catalog::table()
        .iter()
        .filter(|spec| spec.group == id)
        .map(|spec| spec.name)
        .collect();
    ToolResult::done(
        format!("opened {group}"),
        json!({ "group": id.as_str(), "tools": names }),
    )
}

fn session_note(input: &Value) -> ToolResult {
    let text = input
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    ToolResult::done("noted for this chat", json!({ "text": text }))
}

async fn state_show(app: &AppState, input: &Value) -> ToolResult {
    let wanted: Vec<String> = input
        .get("sections")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .filter(|items: &Vec<String>| !items.is_empty())
        .unwrap_or_else(|| {
            ["health", "being", "budgets", "goals"]
                .into_iter()
                .map(str::to_string)
                .collect()
        });
    let full = super::tools::state::snapshot(app).await;
    let mut data = serde_json::Map::new();
    for section in ["kernel", "being", "budgets", "goals", "last_action"] {
        let keep = match section {
            "kernel" => wanted.iter().any(|name| name == "health"),
            "last_action" => true,
            other => wanted.iter().any(|name| name == other),
        };
        if keep {
            if let Some(value) = full.get(section) {
                data.insert(section.to_string(), value.clone());
            }
        }
    }
    let mut result = ToolResult::done("state", Value::Object(data));
    result.focus = Some(json!({ "id": "state", "kind": "state", "title": "State" }));
    result
}

async fn focus_doc(app: &AppState, name: &str, input: &Value) -> ToolResult {
    let id = input.get("id").and_then(Value::as_str).unwrap_or("");
    match crate::routes::focus::resolve(app, id).await {
        Ok(doc) => {
            let value = serde_json::to_value(&doc).unwrap_or(Value::Null);
            let mut result = ToolResult::done(doc.title.clone(), value);
            if name == "focus.open" {
                result.focus = Some(json!({
                    "id": doc.id,
                    "kind": doc.kind,
                    "title": doc.title,
                }));
            }
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn doctor(app: &AppState) -> ToolResult {
    let health = app.health().await;
    let quads = match &app.graph {
        Some(graph) => match graph.quad_count().await {
            Ok(count) => Some(count),
            Err(err) => return ToolResult::fail(err.to_string()),
        },
        None => None,
    };
    let graph_reason = if app.graph.is_some() {
        "the RDF store is open"
    } else {
        "the RDF store did not open"
    };
    let mut result = ToolResult::done(
        format!("kernel {}", service_name(health.kernel.state)),
        json!({
            "kernel": service_name(health.kernel.state),
            "services": health.kernel.services,
            "quad_count": quads,
            "graph_reason": graph_reason,
            "graph_holder": app.graph.as_ref().map(|_| "this process"),
        }),
    );
    result.focus = Some(json!({ "id": "state", "kind": "state", "title": "State" }));
    result
}

async fn logs_head(app: &AppState) -> ToolResult {
    match app.events.head_seq().await {
        Ok(0) => {
            let mut result = ToolResult::done("No events are recorded.", json!({ "seq": 0 }));
            result.reason = Some("No events are recorded.".into());
            result
        }
        Ok(seq) => ToolResult::done(format!("event head {seq}"), json!({ "seq": seq })),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn logs_verify(app: &AppState) -> ToolResult {
    match app.sqlite.audit_chain_report().await {
        Ok(report) => {
            let summary = report.summary();
            let ok = report.is_ok();
            let mut result = ToolResult::done(
                summary.clone(),
                serde_json::to_value(&report).unwrap_or(json!({})),
            );
            result.ok = ok;
            if !ok {
                result.reason = Some(summary);
            }
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn replay(app: &AppState) -> ToolResult {
    let mut applier = CountingApplier::new();
    match app.events.replay(AsOf::now(), &mut applier).await {
        Ok(snapshot) => ToolResult::done(
            format!("replayed {} events", snapshot.applied),
            json!({
                "state_hash": snapshot.log_hash,
                "applied": snapshot.applied,
                "applier_hash": snapshot.applier_hash,
            }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn graph_query(app: &AppState, input: &Value) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(
            app.graph_error
                .clone()
                .unwrap_or_else(|| "the RDF store did not open".into()),
        );
    };
    let query = input.get("query").and_then(Value::as_str).unwrap_or("");
    let name = input
        .get("graph")
        .and_then(Value::as_str)
        .unwrap_or("being");
    match graph.sparql(name, query).await {
        Ok(rows) => ToolResult::done(format!("{} rows", rows.len()), json!({ "rows": rows })),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn graph_validate(app: &AppState, input: &Value) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(
            app.graph_error
                .clone()
                .unwrap_or_else(|| "the RDF store did not open".into()),
        );
    };
    let name = input
        .get("graph")
        .and_then(Value::as_str)
        .unwrap_or("being");
    match graph.validate_named_graph(name).await {
        Ok(report) => {
            let summary = if report.conforms {
                format!("{name} conforms")
            } else {
                format!("{name} has {} violation(s)", report.violations.len())
            };
            ToolResult::done(summary, serde_json::to_value(&report).unwrap_or(json!({})))
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn being_show(app: &AppState) -> ToolResult {
    let Some(being) = &app.being else {
        return ToolResult::fail(being_error(app));
    };
    let guard = being.lock().await;
    let identity = guard.identity();
    let data = json!({
        "self_description": identity.self_description,
        "version": identity.current_version,
        "invariants": identity.invariants,
        "affect": serde_json::to_value(guard.affect()).unwrap_or(Value::Null),
        "goal_count": guard.goals().len(),
        "commitment_count": guard.commitments().len(),
        "accounts": serde_json::to_value(guard.resources()).unwrap_or(Value::Null),
    });
    ToolResult::done(identity.self_description.clone(), data)
}

pub(crate) async fn goal_list(app: &AppState) -> ToolResult {
    let Some(being) = &app.being else {
        return ToolResult::fail(being_error(app));
    };
    let guard = being.lock().await;
    let goals: Vec<Value> = guard
        .goals()
        .values()
        .map(|goal| {
            json!({
                "id": ulid_string(&goal.id),
                "description": goal.description,
                "status": goal.status.as_str(),
                "priority": goal.priority,
            })
        })
        .collect();
    let mut result = ToolResult::done(format!("{} goals", goals.len()), json!({ "goals": goals }));
    result.focus = Some(json!({ "id": "goals", "kind": "tree", "title": "Goals" }));
    result
}

async fn being_verify(app: &AppState) -> ToolResult {
    let Some(being) = &app.being else {
        return ToolResult::fail(being_error(app));
    };
    let corpus = app
        .cfg
        .root
        .join("bench")
        .join("being")
        .join("invariants.jsonl");
    let guard = being.lock().await;
    match guard.verify(&corpus).await {
        Ok(report) => {
            let summary = if report.failures.is_empty() {
                format!("{} invariants enforced", report.invariants_enforced)
            } else {
                report.failures.join("; ")
            };
            let data = json!({
                "ok": report.ok(),
                "invariants_enforced": report.invariants_enforced,
                "corpus_denied": report.corpus_denied,
                "corpus_total": report.corpus_total,
                "failures": report.failures,
            });
            let mut result = ToolResult::done(summary.clone(), data);
            if !report.ok() {
                result.ok = false;
                result.reason = Some(summary);
            }
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn personality_show(app: &AppState) -> ToolResult {
    let Some(being) = &app.being else {
        return ToolResult::fail(being_error(app));
    };
    let guard = being.lock().await;
    let projected = persona::project(guard.personality(), false);
    let data = json!({
        "personality": serde_json::to_value(guard.personality()).unwrap_or(Value::Null),
        "projected_chat": {
            "warmth": projected.warmth,
            "directness": projected.directness,
            "curiosity": projected.curiosity,
            "playfulness": projected.playfulness,
            "formality": projected.formality,
        }
    });
    ToolResult::done("projected chat voice", data)
}

async fn memory_recall(app: &AppState, input: &Value) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(
            app.graph_error
                .clone()
                .unwrap_or_else(|| "the RDF store did not open".into()),
        );
    };
    let query = input.get("query").and_then(Value::as_str).unwrap_or("");
    let engine = match MemoryEngine::open(
        &app.sqlite,
        graph.handle(),
        std::sync::Arc::clone(&app.logger),
        std::sync::Arc::clone(&app.ids),
    )
    .await
    {
        Ok(engine) => engine,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let recall = RecallQuery::new(query, 5, mm_core::Timestamp::now());
    match engine.recall(recall).await {
        Ok(hits) if hits.is_empty() => {
            crate::routes::focus::install_memory(app, json!([]));
            let mut result = ToolResult::done("no memories matched", json!({ "hits": [] }));
            result.focus = Some(json!({ "id": "memory", "kind": "table", "title": "Recall" }));
            result
        }
        Ok(hits) => {
            let rows: Vec<Value> = hits
                .iter()
                .map(|hit| {
                    json!({
                        "id": ulid_string(&hit.memory.id),
                        "content": hit.memory.content,
                        "kind": hit.memory.kind.as_str(),
                        "score": hit.score,
                        "parts": hit.parts.iter().map(|part| json!({
                            "channel": part.channel.as_str(),
                            "raw": part.raw,
                            "weight": part.weight,
                            "contribution": part.contribution,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let table_rows: Vec<Value> = rows
                .iter()
                .map(|row| {
                    json!({
                        "id": row.get("id").cloned().unwrap_or(Value::Null),
                        "name": row.get("content").cloned().unwrap_or(Value::Null),
                        "state": row.get("kind").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect();
            crate::routes::focus::install_memory(app, Value::Array(table_rows));
            let mut result =
                ToolResult::done(format!("{} hits", rows.len()), json!({ "hits": rows }));
            result.focus = Some(json!({ "id": "memory", "kind": "table", "title": "Recall" }));
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn memory_verify(app: &AppState) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(
            app.graph_error
                .clone()
                .unwrap_or_else(|| "the RDF store did not open".into()),
        );
    };
    let engine = match MemoryEngine::open(
        &app.sqlite,
        graph.handle(),
        std::sync::Arc::clone(&app.logger),
        std::sync::Arc::clone(&app.ids),
    )
    .await
    {
        Ok(engine) => engine,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    match engine.verify().await {
        Ok(report) => {
            let summary = if report.ok() {
                format!("{} memory rows reconcile", report.sqlite_rows)
            } else {
                report.failures.join("; ")
            };
            let mut result = ToolResult::done(
                summary.clone(),
                json!({
                    "ok": report.ok(),
                    "sqlite_rows": report.sqlite_rows,
                    "rdf_triples": report.rdf_triples,
                    "failures": report.failures,
                }),
            );
            if !report.ok() {
                result.ok = false;
                result.reason = Some(if summary.is_empty() {
                    "memory verify failed".into()
                } else {
                    summary
                });
            }
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub(crate) async fn budget_show(app: &AppState) -> ToolResult {
    let ledger =
        mm_selfeng::BudgetLedger::new(app.sqlite.clone(), std::sync::Arc::clone(&app.logger));
    match ledger.show().await {
        Ok(rows) if rows.is_empty() => ToolResult {
            ok: true,
            summary: "no budget rows are recorded".into(),
            reason: Some("no budget rows are recorded".into()),
            data: json!({
                "status": "unavailable",
                "reason": "no budget rows are recorded",
                "rows": []
            }),
            focus: Some(json!({ "id": "budget", "kind": "tree", "title": "Budgets" })),
        },
        Ok(rows) => {
            let data = json!({
                "rows": rows.iter().map(|row| json!({
                    "kind": row.kind.as_str(),
                    "period": row.period,
                    "limit": row.limit,
                    "spent": row.spent,
                })).collect::<Vec<_>>()
            });
            let mut result = ToolResult::done(format!("{} budget rows", rows.len()), data);
            result.focus = Some(json!({ "id": "budget", "kind": "tree", "title": "Budgets" }));
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

async fn changeset_show(app: &AppState) -> ToolResult {
    let store = mm_selfeng::ChangeSetStore::new(
        app.sqlite.clone(),
        std::sync::Arc::clone(&app.logger),
        std::sync::Arc::clone(&app.ids),
    );
    match store.list().await {
        Ok(rows) if rows.is_empty() => ToolResult {
            ok: false,
            summary: "No change sets are recorded.".into(),
            reason: Some("No change sets are recorded.".into()),
            data: json!({
                "status": "unavailable",
                "reason": "No change sets are recorded."
            }),
            focus: Some(json!({ "id": "changesets", "kind": "tree", "title": "Change sets" })),
        },
        Ok(rows) => {
            let data = json!({
                "rows": rows.iter().map(|(change_set, status)| json!({
                    "id": ulid_string(&change_set.id),
                    "status": status.as_str(),
                })).collect::<Vec<_>>()
            });
            let mut result = ToolResult::done(format!("{} change sets", rows.len()), data);
            result.focus =
                Some(json!({ "id": "changesets", "kind": "tree", "title": "Change sets" }));
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

fn being_error(app: &AppState) -> String {
    app.being_error
        .clone()
        .unwrap_or_else(|| "the being store did not open".into())
}

fn service_name(state: crate::dto::ServiceState) -> &'static str {
    match state {
        crate::dto::ServiceState::Ok => "ok",
        crate::dto::ServiceState::Degraded => "degraded",
        crate::dto::ServiceState::Unavailable => "unavailable",
    }
}
