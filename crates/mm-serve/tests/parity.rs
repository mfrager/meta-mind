//! Forty-two CLI subcommands are bespoke or passthrough (plan `09` §5, `10` §8).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};
use tower::ServiceExt;

async fn booted() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let state = Arc::new(AppState::boot(cfg, lock).await.unwrap());
    (dir, router(state))
}

#[tokio::test]
async fn forty_two_commands_are_bespoke_or_passthrough() {
    let (_dir, app) = booted().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/commands")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let commands = body["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 42);
    let names: Vec<&str> = commands
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names.len(),
        names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
    for row in commands {
        let tier = row["tier"].as_str().unwrap();
        assert!(tier == "bespoke" || tier == "passthrough", "{row}");
        assert!(!row["op"].as_str().unwrap().is_empty());
    }
}

#[tokio::test]
async fn exec_allowlist_rejects_unknown_names_and_shell_meta() {
    let (_dir, app) = booted().await;
    let unknown = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/exec")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"argv":["not-a-command"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
    let unknown_body = String::from_utf8(
        unknown
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(unknown_body.contains("no operation is registered"));

    let meta = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/exec")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"argv":["doctor;rm"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(meta.status(), StatusCode::BAD_REQUEST);

    let doctor = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/exec")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"argv":["doctor"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(doctor.status(), StatusCode::OK);
    let doctor_body = String::from_utf8(
        doctor
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(doctor_body.contains("bespoke"));
    assert!(doctor_body.contains("kernel"));
}

#[tokio::test]
async fn flows_read_the_kernel_without_inventing_a_run() {
    let (_dir, app) = booted().await;
    let runs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let runs_body = String::from_utf8(
        runs.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(runs_body.contains("no run is recorded"));

    let stage = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"why is stage 6 slow?","commandMode":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let stage_body = String::from_utf8(
        stage
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(stage_body.contains("No stage timings are recorded for this run."));
    assert!(stage_body.contains("01jdocrun00000000000000000"));
    assert!(stage_body.contains("meta analyze"));

    let goal = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"/being goal add a quieter loop","commandMode":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let goal_body = String::from_utf8(
        goal.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(goal_body.contains("does not open that store"));
    assert!(goal_body.contains("\"kind\":\"form\""));

    let promote = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"query":"promote","commandMode":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let promote_body = String::from_utf8(
        promote
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(promote_body.contains("use the change-set flow"));
    assert!(promote_body.contains("Approve"));

    let changeset = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"changeset show","commandMode":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let changeset_body = String::from_utf8(
        changeset
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(changeset_body.contains("No change sets are recorded."));

    let audit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"audit production-tree","commandMode":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let audit_body = String::from_utf8(
        audit
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(audit_body.contains("no production audit was recorded"));

    let tick = app
        .oneshot(
            Request::builder()
                .uri("/api/focus/01jdocrun00000000000000000/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let tick_body = String::from_utf8(
        tick.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(tick_body.contains("stage_tick"));
    assert!(tick_body.contains("No stage timings are recorded for this run."));
}
