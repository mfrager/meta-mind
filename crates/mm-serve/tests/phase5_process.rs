//! Phase 5: process tools and the inspector surface.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};
use mm_serve::tool_schemas::ToolCatalog;

async fn booted() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let state = Arc::new(AppState::boot(cfg, lock).await.unwrap());
    (dir, router(state))
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn the_process_inspector_reports_the_flag_off_state() {
    let (_dir, app) = booted().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cme/processes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["enabled"], json!(false));
    assert_eq!(body["processes"], json!([]));
    assert_eq!(body["templates"], json!([]));
}

#[test]
fn the_plan_schema_accepts_process_fields_and_rejects_typos() {
    let catalog = ToolCatalog::embedded().unwrap();
    let parsed = catalog
        .parse(
            "metamind.plan",
            &json!({
                "action": "create",
                "goal": "ship it",
                "structure": "flat",
                "persist": true,
                "never_expires": true,
                "run": true,
            })
            .to_string(),
        )
        .unwrap();
    assert_eq!(parsed["structure"], json!("flat"));
    assert_eq!(parsed["run"], json!(true));
    let list = catalog
        .parse(
            "metamind.plan",
            &json!({
                "action": "list",
                "include_expired": true,
                "limit": 5,
                "last": true,
            })
            .to_string(),
        )
        .unwrap();
    assert_eq!(list["include_expired"], json!(true));
    assert!(catalog
        .parse(
            "metamind.plan",
            &json!({ "action": "list", "sneaky": true }).to_string(),
        )
        .is_err());
}

#[test]
fn the_act_schema_accepts_process_actions_and_fields() {
    let catalog = ToolCatalog::embedded().unwrap();
    for action in [
        "process_start",
        "process_pause",
        "process_resume",
        "process_cancel",
        "process_retry_step",
    ] {
        catalog
            .parse(
                "metamind.act",
                &json!({ "action": action, "process_id": "p", "step_id": "s" }).to_string(),
            )
            .unwrap();
    }
    let parsed = catalog
        .parse(
            "metamind.act",
            &json!({
                "action": "process_start",
                "template": "urn:metamind:process/template/t",
                "data": { "task": "run the checks" },
            })
            .to_string(),
        )
        .unwrap();
    assert_eq!(parsed["template"], json!("urn:metamind:process/template/t"));
    assert_eq!(parsed["data"]["task"], json!("run the checks"));
}
