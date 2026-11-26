//! Production writes outside a change set are refused (plan `09` §2.4, `10` §8).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};
use tower::ServiceExt;

#[tokio::test]
async fn direct_module_load_is_409_and_a_changeset_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let state = Arc::new(AppState::boot(cfg, lock).await.unwrap());
    let app = router(state);

    let refused = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modules/01jmodule0000000000000000/load")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let body: serde_json::Value =
        serde_json::from_slice(&refused.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["code"], "conflict");
    assert_eq!(body["reason"], "use the change-set flow");

    let created = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/changesets")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"docId":"01jdocprose000000000000000","reason":"keep the note a draft","summary":"note","path":"docs/console-note.md","content":"A note."}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(created.status(), StatusCode::CONFLICT);
    assert!(created.status().is_success());
}
