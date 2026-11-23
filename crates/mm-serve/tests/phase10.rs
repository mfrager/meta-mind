//! Phase 10: status service and inspector routes.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use mm_serve::feature_flags::FeatureFlags;
use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};
use mm_serve::status_service::StatusSection;

async fn booted() -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let state = Arc::new(AppState::boot(cfg, lock).await.unwrap());
    let app = router(Arc::clone(&state));
    (dir, state, app)
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn create_session(app: &axum::Router) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/console/sessions")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_json(response).await
}

#[tokio::test]
async fn the_flag_off_legacy_path_still_boots() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let flags = FeatureFlags {
        cm_v3: false,
        cme_enabled: false,
        ..FeatureFlags::default()
    };
    let state = Arc::new(AppState::boot_with_flags(cfg, lock, flags).await.unwrap());
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn create_status_and_session_pages_are_available() {
    let (_dir, state, app) = booted().await;
    let created = create_session(&app).await;
    let session_id = created["session"]["sessionId"]
        .as_str()
        .unwrap()
        .to_string();

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/console/sessions/{session_id}/status"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status = body_json(status).await;
    assert_eq!(status["sessionId"], session_id);
    assert!(status["revision"].as_u64().unwrap() >= 1);
    assert!(status["stateRevision"].as_u64().unwrap() >= 1);
    assert!(status["dataRevision"].is_u64());
    assert_eq!(status["session"]["lifecycle"], "active");
    assert!(status["unavailable"]["prompt"].is_string());
    assert!(status["unavailable"]["budget"].is_string());
    assert!(status["unavailable"]["data"].is_null());

    for index in 0..3 {
        let _ = create_session(&app).await;
        tokio::time::sleep(Duration::from_millis(2 * index)).await;
    }
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/console/sessions?limit=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let page = body_json(page).await;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["hasMore"], true);
    let cursor = page["nextCursor"].as_str().unwrap().to_string();
    let next = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/console/sessions?limit=2&cursor={cursor}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let next = body_json(next).await;
    assert!(!next["items"].as_array().unwrap().is_empty());
    assert_eq!(
        state
            .state_service
            .list_sessions("", None, 100, Default::default())
            .await
            .unwrap()
            .items
            .len(),
        4
    );
}

#[tokio::test]
async fn a_presence_mutation_publishes_a_status_revision_within_250ms() {
    let (_dir, state, app) = booted().await;
    let created = create_session(&app).await;
    let session_id = created["session"]["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let mut events = state.status_service.subscribe();
    let before = state
        .status_service
        .snapshot(&session_id)
        .await
        .unwrap()
        .revision;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/console/sessions/{session_id}/presence"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "presence failed: {}",
        String::from_utf8_lossy(&bytes)
    );

    let event = tokio::time::timeout(Duration::from_millis(250), events.recv())
        .await
        .expect("status frame within 250 ms")
        .unwrap();
    assert_eq!(event.session_id, session_id);
    assert!(event.view.revision > before);
    assert_eq!(event.reason, "update");
}

#[tokio::test]
async fn a_burst_of_dirty_marks_is_coalesced() {
    let (_dir, state, app) = booted().await;
    let created = create_session(&app).await;
    let session_id = created["session"]["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let mut events = state.status_service.subscribe();
    while events.try_recv().is_ok() {}

    for _ in 0..100 {
        state
            .status_service
            .mark_dirty(&session_id, StatusSection::State);
    }
    let mut revisions = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1_200);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(120), events.recv()).await
        {
            revisions.push(event.view.revision);
        }
    }
    assert!(
        revisions.len() <= 10,
        "100 marks must coalesce, got {} frames",
        revisions.len()
    );
    assert!(!revisions.is_empty());
    let latest = state
        .status_service
        .snapshot(&session_id)
        .await
        .unwrap()
        .revision;
    assert!(latest >= *revisions.last().unwrap());
}

#[tokio::test]
async fn inspector_read_routes_and_metrics_are_wired() {
    let (_dir, _state, app) = booted().await;
    let created = create_session(&app).await;
    let session_id = created["session"]["sessionId"]
        .as_str()
        .unwrap()
        .to_string();

    for path in [
        format!("/api/console/sessions/{session_id}"),
        format!("/api/console/sessions/{session_id}/state"),
        format!("/api/console/sessions/{session_id}/frames"),
        format!("/api/console/sessions/{session_id}/traces"),
        format!("/api/console/sessions/{session_id}/logs"),
        format!("/api/console/sessions/{session_id}/operations"),
        format!("/api/console/sessions/{session_id}/interjections"),
        "/api/console/interjections".into(),
        "/api/console/data/health".into(),
        "/api/console/data/entities".into(),
        "/api/console/data/schemas".into(),
        "/api/console/data/namespaces".into(),
        "/api/console/plans".into(),
        "/api/console/automation".into(),
        "/api/console/prompts".into(),
        "/api/console/files/tree?path=/sessions".into(),
        "/api/agents".into(),
        "/api/agentstream/sessions".into(),
        "/api/status".into(),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{path} returned {}",
            response.status()
        );
    }

    let metrics = app
        .oneshot(
            Request::builder()
                .uri("/api/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let text = String::from_utf8(
        metrics
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    for name in [
        "metamind_http_requests_total",
        "metamind_status_frames_total",
        "metamind_prompt_unavailable_total",
        "metamind_budget_denied_total",
        "metamind_automation_emissions_total",
        "metamind_plan_external_starts_total",
        "metamind_plan_failures_total",
    ] {
        assert!(text.contains(name), "metrics missing {name}");
    }
}

#[tokio::test]
async fn actions_list_and_cancel_accept_iri_ids() {
    let (_dir, _state, app) = booted().await;
    let _ = create_session(&app).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/console/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let action_id = body["scheduled"]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["actionId"].as_str())
        .expect("the built-in action is seeded")
        .to_string();
    assert!(action_id.contains("/"));

    let cancelled = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/console/actions/{action_id}/cancel"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    assert_eq!(body_json(cancelled).await["status"], "cancelled");
}

#[tokio::test]
async fn status_stream_starts_with_a_full_view() {
    let (_dir, _state, app) = booted().await;
    let created = create_session(&app).await;
    let session_id = created["session"]["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/console/sessions/{session_id}/status/stream?from=0"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_millis(250), body.frame())
        .await
        .expect("first status frame")
        .expect("body frame")
        .expect("frame data");
    let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
    assert!(text.contains("event: status"), "{text}");
    assert!(text.contains("\"revision\""), "{text}");
    assert!(text.contains("\"sessionId\":"), "{text}");
}

#[tokio::test]
async fn status_stream_does_not_forward_another_session() {
    let (_dir, _state, app) = booted().await;
    let target = create_session(&app).await;
    let target_id = target["session"]["sessionId"].as_str().unwrap().to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/console/sessions/{target_id}/status/stream?from=0"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_millis(250), body.frame())
        .await
        .expect("first status frame")
        .expect("body frame")
        .expect("frame data");
    let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
    assert!(
        text.contains(&format!("\"sessionId\":\"{target_id}\"")),
        "{text}"
    );

    // Another session's publish must not leak into the target stream.
    let _other = create_session(&app).await;
    let leaked = tokio::time::timeout(Duration::from_millis(300), body.frame()).await;
    if let Ok(Some(Ok(frame))) = leaked {
        let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap_or_default();
        // A follow-up frame for the target itself is legitimate (automation
        // updates the session right after creation); another session's id is
        // the leak this test guards against.
        assert!(
            text.is_empty() || text.contains(&format!("\"sessionId\":\"{target_id}\"")),
            "another session's status frame was forwarded: {text}"
        );
    }
}

#[tokio::test]
async fn session_delete_requires_an_ended_session() {
    let (_dir, _state, app) = booted().await;
    let created = create_session(&app).await;
    let session_id = created["session"]["sessionId"]
        .as_str()
        .unwrap()
        .to_string();

    let active = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/console/sessions/{session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(active.status(), StatusCode::CONFLICT);

    let ended = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/console/sessions/{session_id}/end"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ended.status(), StatusCode::OK);

    let deleted = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/console/sessions/{session_id}?mode=soft"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(body_json(deleted).await["mode"], "soft");
}
