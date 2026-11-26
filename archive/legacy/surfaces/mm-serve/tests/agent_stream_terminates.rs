//! A finished agent run's event stream must close.
//!
//! `GET /api/agents/{run_id}/stream` used to poll a terminal run every 500 ms
//! forever: a run has a finish line, so its stream does too.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mm_serve::feature_flags::FeatureFlags;
use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn a_completed_run_closes_its_stream() {
    let dir = tempfile::tempdir().unwrap();
    // Runs are recovered from disk at boot, so a terminal record is enough to
    // exercise the stream without starting an external agent.
    let runs = json!([{
        "run_id": "run-1",
        "session_id": "session-1",
        "agentstream_id": "agent/run-1",
        "kind": "hermes",
        "status": "done",
        "turns": 1,
        "task": "finish the task",
        "origin": "cm",
        "started_at": 1,
        "updated_at": 2,
        "usage": { "prompt_tokens": 3, "completion_tokens": 4 },
        "workspace": dir.path().join("workspace"),
    }]);
    std::fs::write(
        dir.path().join("agent-runs.json"),
        serde_json::to_vec(&runs).unwrap(),
    )
    .unwrap();

    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let state = Arc::new(
        AppState::boot_with_flags(cfg, lock, FeatureFlags::default())
            .await
            .unwrap(),
    );

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri("/api/agents/run-1/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = response.into_body();
    let mut frames = 0;
    loop {
        let next = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("a terminal run's stream must end instead of polling forever");
        match next {
            Some(Ok(frame)) => {
                if frame.data_ref().is_some() {
                    frames += 1;
                }
            }
            Some(Err(error)) => panic!("body error: {error}"),
            None => break,
        }
    }
    assert_eq!(frames, 0, "the seeded run has no events");
}
