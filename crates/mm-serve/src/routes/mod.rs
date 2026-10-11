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
use axum::routing::get;
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
    "http://127.0.0.1:5173",
    "http://localhost:5173",
    "http://127.0.0.1:4173",
    "http://localhost:4173",
];

/// Build the console router. `state` is shared by every handler.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .fallback(not_found)
        .with_state(Arc::clone(&state))
        .layer(from_fn_with_state(state, access_log))
        .layer(from_fn(security_headers))
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
        HeaderValue::from_static("GET, POST, OPTIONS"),
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
        assert_eq!(graph["state"], "unavailable");
        assert!(graph["reason"].as_str().unwrap().contains("RDF"));
        let llm = services.iter().find(|s| s["name"] == "llm").unwrap();
        assert_eq!(llm["state"], "unavailable");
        assert!(!llm["reason"].as_str().unwrap().is_empty());
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
    async fn cors_allows_the_dev_server_only() {
        let (_dir, app) = booted().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header(header::ORIGIN, "http://127.0.0.1:5173")
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
            Some("http://127.0.0.1:5173")
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
}
