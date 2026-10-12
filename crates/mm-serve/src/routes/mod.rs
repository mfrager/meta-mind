//! Router assembly for `mm-serve` (plan `01` §2, `09` §1).
//!
//! M0 serves `GET /api/health`. Later milestones add chat, focus, and domain
//! routes to this router. Every response carries the localhost CORS allowlist,
//! a locked-down CSP, and an access line in the kernel log.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use mm_log::{Level, LogRecord};

/// Access-log event code. Not a kernel code: request lines are operator
/// telemetry, and `logs verify` only requires a non-empty `event_code`.
const SERVE_REQUEST: &str = "serve.request";

use crate::dto::HealthEnvelope;
use crate::error::AppError;
use crate::state::AppState;

/// Origins the dev SPA may call when it is not going through the Vite proxy.
const LOCAL_ORIGINS: &[&str] = &[
    "http://127.0.0.1:8900",
    "http://localhost:8900",
    "http://127.0.0.1:4173",
    "http://localhost:4173",
];

mod changesets;
mod chat;
mod exec;
pub(crate) mod focus;
mod runs;
mod trees;

/// Build the console router. `state` is shared by every handler.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(console_state))
        .route("/api/focus", get(focus::list))
        .route("/api/focus/{doc_id}/draft", put(focus::save_draft))
        .route("/api/focus/{doc_id}/history", get(focus::history))
        .route("/api/focus/{doc_id}/export", get(focus::export_doc))
        .route("/api/focus/{doc_id}/provenance", get(focus::provenance))
        .route("/api/focus/{doc_id}/stream", get(focus::stream))
        .route("/api/focus/{doc_id}", get(focus::get_doc))
        .route("/api/changesets", post(changesets::create))
        .route("/api/trees/{name}/ops", post(trees::apply_ops))
        .route("/api/trees/{name}", get(trees::get_tree))
        .route("/api/maps/{name}", get(trees::get_map))
        .route(
            "/api/chat/sessions",
            get(chat::list_sessions).post(chat::create_session),
        )
        .route(
            "/api/chat/sessions/{session_id}/input-answer",
            post(chat::answer),
        )
        .route("/api/chat/sessions/{session_id}/title", post(chat::rename))
        .route("/api/chat/sessions/{session_id}/branch", post(chat::branch))
        .route("/api/chat/{request_id}/stream", get(chat::stream))
        .route("/api/chat/{request_id}/stop", post(chat::stop))
        .route("/api/chat/{request_id}/regenerate", post(chat::regenerate))
        .route("/api/chat", post(chat::open))
        .route("/api/commands", get(exec::commands))
        .route("/api/exec", post(exec::exec))
        .route("/api/runs", get(runs::list))
        .route("/api/runs/{id}", get(runs::get))
        .route("/api/modules/{id}/load", post(runs::load_module))
        .fallback(not_found)
        .with_state(Arc::clone(&state))
        .layer(from_fn_with_state(state, access_log))
        .layer(from_fn(security_headers))
}

async fn console_state(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(crate::controller::tools::state::snapshot(&state).await)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthEnvelope> {
    Json(state.health().await)
}

async fn not_found() -> AppError {
    AppError::NotFound("no such route".into())
}

/// Access line. Failures to log must not fail the request the operator is waiting on.
async fn access_log(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let started = std::time::Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let record = LogRecord::new(Level::Info, SERVE_REQUEST, "mm.serve")
        .with_field("http.method", method)
        .with_field("http.path", path)
        .with_field("http.status", status)
        .with_field("elapsed_ms", elapsed_ms);
    if let Err(err) = state.logger.emit(record).await {
        eprintln!("mm-serve: access log failed: {err}");
    }
    response
}

/// Localhost CORS, plus the headers every API response carries.
async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).cloned();
    if req.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_cors(response.headers_mut(), origin.as_ref());
        apply_security(response.headers_mut());
        return response;
    }
    let mut response = next.run(req).await;
    apply_cors(response.headers_mut(), origin.as_ref());
    apply_security(response.headers_mut());
    response
}

fn apply_cors(headers: &mut HeaderMap, origin: Option<&HeaderValue>) {
    let Some(origin) = origin else {
        return;
    };
    let Ok(value) = origin.to_str() else {
        return;
    };
    if !LOCAL_ORIGINS.contains(&value) {
        return;
    }
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type, last-event-id, x-request-id"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("origin"));
}

fn apply_security(headers: &mut HeaderMap) {
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    async fn booted() -> (tempfile::TempDir, Router) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = mm_core::Config::for_data_dir(dir.path());
        let lock = crate::state::ConsoleLock::acquire(dir.path()).unwrap();
        let state = Arc::new(crate::state::AppState::boot(cfg, lock).await.unwrap());
        (dir, router(state))
    }

    #[tokio::test]
    async fn health_is_ok_and_names_unavailable_services() {
        let (_dir, app) = booted().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .is_some());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["wireVersion"], 1);
        assert!(!body["dataDir"].as_str().unwrap().is_empty());
        assert!(body["eventHead"]["seq"].is_number());
        let services = body["kernel"]["services"].as_array().unwrap();
        let graph = services.iter().find(|s| s["name"] == "graph").unwrap();
        assert_eq!(graph["state"], "ok");
        let llm = services.iter().find(|s| s["name"] == "llm").unwrap();
        if mm_llm::ProviderEnv::from_env().is_some() {
            assert_eq!(llm["state"], "ok");
            assert!(llm["reason"].as_str().unwrap().contains("model="));
            assert!(llm["reason"].as_str().unwrap().contains("key=<redacted>"));
        } else {
            assert_eq!(llm["state"], "unavailable");
            assert_eq!(llm["reason"], "no language-model provider is configured");
        }
    }

    #[tokio::test]
    async fn unknown_routes_carry_a_reason() {
        let (_dir, app) = booted().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "not_found");
        assert!(body["correlationId"].as_str().unwrap().len() == 26);
    }

    #[tokio::test]
    async fn focus_doc_and_changeset_round_trip() {
        let (_dir, app) = booted().await;
        let graph = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/focus/graph")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(graph.status(), StatusCode::OK);
        let graph_body: serde_json::Value =
            serde_json::from_slice(&graph.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(graph_body["status"], "saved");

        let created = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/changesets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"docId":"01jdocprose000000000000000","reason":"Record the console note.","summary":"Keep the note as a draft change set.","path":"docs/console-note.md","content":"edited"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let created_body: serde_json::Value =
            serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(created_body["status"], "draft");
        assert_eq!(created_body["id"].as_str().unwrap().len(), 26);
        assert!(created_body["doc"]["subtitle"]
            .as_str()
            .unwrap()
            .contains(created_body["id"].as_str().unwrap()));
    }

    #[tokio::test]
    async fn cors_allows_the_dev_server_only() {
        let (_dir, app) = booted().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header(header::ORIGIN, "http://127.0.0.1:8900")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("http://127.0.0.1:8900")
        );

        let (_dir, app) = booted().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header(header::ORIGIN, "https://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test]
    async fn tree_ops_round_trip_and_empty_kernel_trees_say_why() {
        let (_dir, app) = booted().await;
        let loaded = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/trees/ontology")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(loaded.status(), StatusCode::OK);
        let tree: serde_json::Value =
            serde_json::from_slice(&loaded.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(tree["fixture"], true);
        assert!(tree["reason"].as_str().unwrap().contains("fixture"));

        let renamed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/trees/ontology/ops")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"ops":[{"op":"rename","id":"ont-thing","label":"Entity"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(renamed.status(), StatusCode::OK);
        let renamed_body: serde_json::Value =
            serde_json::from_slice(&renamed.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(renamed_body["eventId"].as_str().unwrap().len(), 26);
        let label = renamed_body["tree"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "ont-thing")
            .unwrap()["label"]
            .as_str()
            .unwrap();
        assert_eq!(label, "Entity");

        let undone = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/trees/ontology/ops")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"ops":[{"op":"rename","id":"ont-thing","label":"Thing"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undone.status(), StatusCode::OK);

        let moved = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/trees/ontology/ops")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"ops":[{"op":"move","id":"ont-name","targetId":"ont-thing","position":"inside"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(moved.status(), StatusCode::OK);
        let moved_body: serde_json::Value =
            serde_json::from_slice(&moved.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let parent = moved_body["tree"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "ont-name")
            .unwrap()["parentId"]
            .as_str()
            .unwrap();
        assert_eq!(parent, "ont-thing");

        let removed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/trees/ontology/ops")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"ops":[{"op":"remove","id":"ont-name"}]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(removed.status(), StatusCode::OK);

        let restored = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/trees/ontology/ops")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"ops":[{"op":"add","id":"ont-name","parentId":"ont-agent","kind":"property","label":"name"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(restored.status(), StatusCode::OK);

        let budgets = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/trees/budgets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let budgets_body: serde_json::Value =
            serde_json::from_slice(&budgets.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(budgets_body["fixture"], false);
        assert!(budgets_body["reason"].is_null());
        assert!(budgets_body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["label"] == "meta_analysis"));

        let map = app
            .oneshot(
                Request::builder()
                    .uri("/api/maps/ontology")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(map.status(), StatusCode::OK);
        let map_body: serde_json::Value =
            serde_json::from_slice(&map.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(map_body["rootId"], "ont");
        assert!(map_body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == "ont-thing"));
    }

    #[tokio::test]
    async fn chat_decide_answer_budget_and_resume() {
        assert!(crate::chat::wired_op_count() >= 20);
        let (_dir, app) = booted().await;
        let opened = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"decide whether to continue","commandMode":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(opened.status(), StatusCode::OK);
        let raw = String::from_utf8(
            opened
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(raw.contains("no decision core is bound on this route"));
        let request_id = raw
            .split("data: ")
            .filter_map(|chunk| {
                serde_json::from_str::<serde_json::Value>(chunk.lines().next()?).ok()
            })
            .find(|frame| frame["type"] == "run")
            .unwrap()["requestId"]
            .as_str()
            .unwrap()
            .to_string();
        let session_id = raw
            .split("data: ")
            .filter_map(|chunk| {
                serde_json::from_str::<serde_json::Value>(chunk.lines().next()?).ok()
            })
            .find(|frame| frame["type"] == "run")
            .unwrap()["sessionId"]
            .as_str()
            .unwrap()
            .to_string();
        let choice_id = raw
            .split("data: ")
            .filter_map(|chunk| {
                serde_json::from_str::<serde_json::Value>(chunk.lines().next()?).ok()
            })
            .find(|frame| frame["type"] == "input_request")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        let answered = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/chat/sessions/{session_id}/input-answer"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"id":"{choice_id}","answer":"accept"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(answered.status(), StatusCode::OK);
        let answered_body: serde_json::Value =
            serde_json::from_slice(&answered.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let echo = answered_body["frames"]
            .as_array()
            .unwrap()
            .iter()
            .find(|frame| frame["frame"]["type"] == "final_response")
            .unwrap()["frame"]["text"]
            .as_str()
            .unwrap();
        assert_eq!(echo, "✓ Accept");

        let resumed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/chat/{request_id}/stream?from=3"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let resumed_raw = String::from_utf8(
            resumed
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(!resumed_raw.contains("id: 1\n"));
        assert!(resumed_raw.contains("id: 3\n"));
        assert_eq!(resumed_raw.matches("\"type\":\"run\"").count(), 0);

        let stopped = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/chat/{request_id}/stop"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let stopped_body: serde_json::Value =
            serde_json::from_slice(&stopped.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(stopped_body["reason"], "this run already finished");

        let budget = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"query":"budget show","commandMode":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let budget_raw = String::from_utf8(
            budget
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(budget_raw.contains("meta_analysis"));
        assert!(budget_raw.contains("Open the budgets tree"));
    }
}
