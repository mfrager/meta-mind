//! Focus routes (plan `09` §2.3, `05`).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use mm_selfeng::BudgetLedger;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::docs::HistoryEntry;
use crate::dto::focus::{DocCapability, DocKind, DocSource, DocSourceKind, DocStatus, FocusDoc};
use crate::error::AppError;
use crate::sse::SseFrame;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct DraftBody {
    pub body: String,
}

#[derive(Deserialize)]
pub struct FromQuery {
    pub from: Option<u64>,
    pub format: Option<String>,
}

pub async fn list(State(state): State<Arc<AppState>>) -> Json<Vec<FocusDoc>> {
    Json(state.focus.list())
}

pub async fn get_doc(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
) -> Result<Json<FocusDoc>, AppError> {
    Ok(Json(resolve(&state, &doc_id).await?))
}

pub async fn save_draft(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
    Json(body): Json<DraftBody>,
) -> Result<Json<FocusDoc>, AppError> {
    state
        .focus
        .save_draft(&doc_id, &body.body)
        .map(Json)
        .map_err(AppError::BadRequest)
}

pub async fn history(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
) -> Result<Json<Vec<HistoryEntryJson>>, AppError> {
    let entries = state
        .focus
        .history(&doc_id)
        .ok_or_else(|| AppError::NotFound(format!("no document {doc_id}")))?;
    Ok(Json(
        entries.into_iter().map(HistoryEntryJson::from).collect(),
    ))
}

pub async fn provenance(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let doc = resolve(&state, &doc_id).await?;
    let events = state.focus.history(&doc_id).map(|h| h.len()).unwrap_or(0);
    Ok(Json(json!({
        "events": events,
        "traceId": Value::Null,
        "createdBy": doc.source.kind,
        "updatedAt": doc.subtitle,
    })))
}

pub async fn export_doc(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
    Query(query): Query<FromQuery>,
) -> Result<Response, AppError> {
    let doc = resolve(&state, &doc_id).await?;
    let format = query.format.as_deref().unwrap_or("json");
    let (content_type, body) = match format {
        "json" => (
            "application/json",
            serde_json::to_string(&doc).unwrap_or_default(),
        ),
        "md" => (
            "text/markdown",
            doc.snapshot
                .get("markdown")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "csv" => ("text/csv", snapshot_csv(&doc.snapshot)),
        other => {
            return Err(AppError::BadRequest(format!(
                "{other} is not exported by the server; png and svg are built in the pane"
            )))
        }
    };
    Ok(([(header::CONTENT_TYPE, content_type)], body).into_response())
}

pub async fn stream(
    State(state): State<Arc<AppState>>,
    Path(doc_id): Path<String>,
    Query(query): Query<FromQuery>,
) -> Result<Response, AppError> {
    let from = query.from.unwrap_or(0);
    let mut body = String::new();
    for doc in state.focus.live_since(from) {
        let frame = SseFrame::json(
            doc.version,
            "focus",
            &json!({ "type": "focus", "doc": doc }),
        )
        .map_err(|err| AppError::Internal(err.to_string()))?;
        body.push_str(&frame.encode());
    }
    if doc_id == crate::docs::RUN_ID {
        let tick = super::runs::stage_tick();
        let frame = SseFrame::json(1, "stage_tick", &tick)
            .map_err(|err| AppError::Internal(err.to_string()))?;
        body.push_str(&frame.encode());
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .map_err(|err| AppError::Internal(err.to_string()))?)
}

pub(crate) async fn resolve(state: &AppState, id: &str) -> Result<FocusDoc, AppError> {
    if id == "graph" || id == "llm" {
        return service_doc(state, id).await;
    }
    if id == "budget" {
        return budget_doc(state).await;
    }
    if id == "goals" {
        return goals_doc(state).await;
    }
    if id == "changesets" {
        return changesets_doc(state).await;
    }
    state
        .focus
        .get(id)
        .ok_or_else(|| AppError::NotFound(format!("no document {id}")))
}

pub(crate) fn install_memory(state: &AppState, rows: Value) {
    state
        .focus
        .upsert(live_table("memory", "Recall", rows, None));
}

async fn service_doc(state: &AppState, name: &str) -> Result<FocusDoc, AppError> {
    let health = state.health().await;
    let service = health
        .kernel
        .services
        .into_iter()
        .find(|service| service.name == name)
        .ok_or_else(|| AppError::NotFound(format!("no service {name}")))?;
    let (status, reason) = match service.state {
        crate::dto::ServiceState::Unavailable => (
            DocStatus::Unavailable,
            service
                .reason
                .unwrap_or_else(|| format!("{name} is unavailable")),
        ),
        crate::dto::ServiceState::Degraded => (
            DocStatus::Degraded,
            service
                .reason
                .unwrap_or_else(|| format!("{name} is degraded")),
        ),
        crate::dto::ServiceState::Ok => (DocStatus::Saved, service.reason.unwrap_or_default()),
    };
    Ok(FocusDoc {
        v: 1,
        id: name.into(),
        kind: if name == "graph" {
            DocKind::Tree
        } else {
            DocKind::Prose
        },
        title: name.into(),
        subtitle: None,
        status,
        reason: if reason.is_empty() {
            None
        } else {
            Some(reason)
        },
        epistemic: None,
        version: 1,
        source: DocSource {
            kind: DocSourceKind::Nav,
            ref_id: None,
        },
        capabilities: vec![DocCapability::Refresh],
        snapshot: json!({}),
    })
}

async fn budget_doc(state: &AppState) -> Result<FocusDoc, AppError> {
    let ledger = BudgetLedger::new(state.sqlite.clone(), Arc::clone(&state.logger));
    match ledger.show().await {
        Ok(rows) if rows.is_empty() => Ok(budget_status(
            DocStatus::Unavailable,
            "no budget rows are recorded",
            json!({ "rows": [] }),
        )),
        Ok(rows) => {
            let snapshot = json!({
                "rows": rows.iter().map(|row| json!({
                    "kind": row.kind.as_str(),
                    "period": row.period,
                    "limit": row.limit,
                    "spent": row.spent,
                })).collect::<Vec<_>>()
            });
            Ok(budget_status(DocStatus::Saved, "", snapshot))
        }
        Err(err) => Ok(budget_status(
            DocStatus::Unavailable,
            &err.to_string(),
            json!({ "rows": [] }),
        )),
    }
}

fn budget_status(status: DocStatus, reason: &str, snapshot: Value) -> FocusDoc {
    FocusDoc {
        v: 1,
        id: "budget".into(),
        kind: DocKind::Budget,
        title: "Budgets".into(),
        subtitle: None,
        status,
        reason: if reason.is_empty() {
            None
        } else {
            Some(reason.into())
        },
        epistemic: None,
        version: 1,
        source: DocSource {
            kind: DocSourceKind::Nav,
            ref_id: None,
        },
        capabilities: vec![DocCapability::Refresh],
        snapshot,
    }
}

async fn goals_doc(state: &AppState) -> Result<FocusDoc, AppError> {
    let Some(being) = &state.being else {
        return Ok(live_table(
            "goals",
            "Goals",
            json!([]),
            Some(
                state
                    .being_error
                    .clone()
                    .unwrap_or_else(|| "the being store did not open".into()),
            ),
        ));
    };
    let guard = being.lock().await;
    let rows: Vec<Value> = guard
        .goals()
        .values()
        .map(|goal| {
            json!({
                "id": mm_core::ulid_string(&goal.id),
                "name": goal.description,
                "state": goal.status.as_str(),
            })
        })
        .collect();
    let empty = rows.is_empty();
    let mut doc = live_table("goals", "Goals", Value::Array(rows), None);
    if empty {
        doc.status = DocStatus::Saved;
        doc.reason = Some("No goals are recorded.".into());
    }
    Ok(doc)
}

async fn changesets_doc(state: &AppState) -> Result<FocusDoc, AppError> {
    let store = mm_selfeng::ChangeSetStore::new(
        state.sqlite.clone(),
        Arc::clone(&state.logger),
        Arc::clone(&state.ids),
    );
    match store.list().await {
        Ok(rows) if rows.is_empty() => Ok(live_table(
            "changesets",
            "Change sets",
            json!([]),
            Some("No change sets are recorded.".into()),
        )),
        Ok(rows) => {
            let table: Vec<Value> = rows
                .iter()
                .map(|(change_set, status)| {
                    let id = mm_core::ulid_string(&change_set.id);
                    json!({ "id": id, "name": id, "state": status.as_str() })
                })
                .collect();
            Ok(live_table(
                "changesets",
                "Change sets",
                Value::Array(table),
                None,
            ))
        }
        Err(err) => Ok(live_table(
            "changesets",
            "Change sets",
            json!([]),
            Some(err.to_string()),
        )),
    }
}

fn live_table(id: &str, title: &str, rows: Value, reason: Option<String>) -> FocusDoc {
    let unavailable = reason.is_some();
    FocusDoc {
        v: 1,
        id: id.into(),
        kind: DocKind::Table,
        title: title.into(),
        subtitle: None,
        status: if unavailable {
            DocStatus::Unavailable
        } else {
            DocStatus::Saved
        },
        reason,
        epistemic: None,
        version: 1,
        source: DocSource {
            kind: DocSourceKind::Chat,
            ref_id: None,
        },
        capabilities: vec![DocCapability::Refresh],
        snapshot: json!({ "rows": rows }),
    }
}

fn snapshot_csv(snapshot: &Value) -> String {
    let Some(series) = snapshot.get("series").and_then(|v| v.as_array()) else {
        return "name,x,y\n".into();
    };
    let mut out = String::from("name,x,y\n");
    for item in series {
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let Some(points) = item.get("points").and_then(|v| v.as_array()) else {
            continue;
        };
        for point in points {
            let x = point.get("x").and_then(|v| v.as_str()).unwrap_or("");
            let y = point.get("y").map(|v| v.to_string()).unwrap_or_default();
            out.push_str(&format!("{name},{x},{y}\n"));
        }
    }
    out
}

#[derive(serde::Serialize)]
pub struct HistoryEntryJson {
    pub version: u64,
    #[serde(rename = "savedAt")]
    pub saved_at: String,
    pub actor: String,
    pub status: DocStatus,
}

impl From<HistoryEntry> for HistoryEntryJson {
    fn from(entry: HistoryEntry) -> Self {
        HistoryEntryJson {
            version: entry.version,
            saved_at: entry.saved_at,
            actor: entry.actor,
            status: entry.status,
        }
    }
}
