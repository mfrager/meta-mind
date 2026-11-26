//! A streamed three-pass turn must close its response body.
//!
//! The session driver outlives the request and keeps the turn's pass observer,
//! so the body used to stay open after `completed`: the SSE stream ends only
//! when its sender drops, and nothing dropped it.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use mm_llm::{MockClient, ProviderToolCall, ScriptedResponse};
use mm_serve::cme_stream::ChatStream;
use mm_serve::controller::ControllerModel;
use mm_serve::feature_flags::FeatureFlags;
use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn closing_the_stream_ends_the_receiver_even_when_the_stream_is_retained() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let state = Arc::new(
        AppState::boot_with_flags(cfg, lock, FeatureFlags::default())
            .await
            .unwrap(),
    );

    let (stream, mut receiver) = ChatStream::new(Arc::clone(&state), "request-1".into());
    // The session driver holds its own clone; closing must not wait for it.
    let retained = Arc::clone(&stream);
    stream.close();
    stream.close();

    assert!(
        receiver.recv().await.is_none(),
        "a closed stream must end its receiver"
    );
    // Two owners are still alive, so the sender was not dropped by refcount:
    // close() ended it explicitly.
    assert_eq!(Arc::strong_count(&retained), 2);
}

#[tokio::test]
async fn a_streamed_turn_closes_its_body_after_the_terminal_frame() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let mut state = AppState::boot_with_flags(
        cfg,
        lock,
        FeatureFlags {
            cme_enabled: true,
            three_pass_v2: true,
            ..FeatureFlags::default()
        },
    )
    .await
    .unwrap();
    state.three_pass.turn.stream = true;
    let state = Arc::new(state);

    let client = Arc::new(MockClient::script(vec![ScriptedResponse::text("")
        .with_tool_calls(vec![ProviderToolCall {
            id: "call-approve".into(),
            name: "metamind.approve".into(),
            arguments: json!({ "success": true, "reply": "Hello!" }).to_string(),
        }])]));
    *state.controller_model.lock().unwrap() = ControllerModel::Scripted(client);

    let session_id = state.ids.next_string();
    state.chat.create_session(session_id.clone()).unwrap();

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/cme/chat")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "sessionId": session_id, "query": "hello" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = response.into_body();
    let mut raw = String::new();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(10), body.frame())
            .await
            .expect("the response body must end instead of staying open");
        match next {
            Some(Ok(frame)) => {
                if let Some(data) = frame.data_ref() {
                    raw.push_str(&String::from_utf8_lossy(data));
                }
            }
            Some(Err(error)) => panic!("body error: {error}"),
            None => break,
        }
    }

    assert!(raw.contains("\"type\":\"completed\""), "{raw}");
    assert!(raw.contains("\"type\":\"final_response\""), "{raw}");
}
