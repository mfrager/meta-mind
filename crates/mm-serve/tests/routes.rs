//! Route tests over a real `AppState` with a scripted mock client.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mm_core::Config;
use mm_llm::{ProviderToolCall, ScriptedResponse};
use mm_serve::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

fn tool(id: &str, name: &str, arguments: Value) -> ProviderToolCall {
    ProviderToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.to_string(),
    }
}

fn script() -> Vec<ScriptedResponse> {
    vec![
        ScriptedResponse::text("").with_tool_calls(vec![tool(
            "call-verify",
            "metamind.verify",
            json!({
                "checks": [],
                "process": { "task": "answer the question" }
            }),
        )]),
        ScriptedResponse::text("The deploy finished at 14:02."),
        ScriptedResponse::text(
            json!({ "pass": true, "reasoning": "answers the request", "confidence": 0.9 })
                .to_string(),
        ),
        ScriptedResponse::text("").with_tool_calls(vec![tool(
            "call-approve",
            "metamind.approve",
            json!({ "success": true }),
        )]),
    ]
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn app(dir: &std::path::Path) -> axum::Router {
    let cfg = Config::for_data_dir(dir);
    let state = Arc::new(AppState::for_test(cfg, script()).unwrap());
    router(state)
}

async fn start_and_wait(app: &axum::Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/turns")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "text": "answer the question", "userId": "operator" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let session_id = body_json(response).await["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    for _ in 0..200 {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/agentstream/sessions/{session_id}/events?from=1"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        let page = body_json(response).await;
        if page["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "turn.completed")
        {
            return session_id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("turn did not complete");
}

async fn read_sse_until(response: axum::response::Response, needle: &str) -> String {
    let mut body = response.into_body();
    let mut text = String::new();
    while !text.contains(needle) {
        let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("SSE read timed out")
            .expect("SSE body ended")
            .unwrap();
        if let Some(data) = frame.data_ref() {
            text.push_str(std::str::from_utf8(data).unwrap());
        }
    }
    text
}

#[tokio::test]
async fn a_turn_is_accepted_and_its_jsonl_is_served() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/turns")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "text": "answer the question", "userId": "operator" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    let session_id = accepted["sessionId"].as_str().unwrap().to_owned();
    assert!(!accepted["turnId"].as_str().unwrap().is_empty());

    let mut page = Value::Null;
    for _ in 0..200 {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/agentstream/sessions/{session_id}/events?from=1"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        page = body_json(response).await;
        let done = page["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "turn.completed");
        if done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let events = page["events"].as_array().unwrap();
    assert!(
        events.iter().any(|event| event["type"] == "turn.completed"),
        "{page}"
    );
    assert!(events.iter().any(|event| event["type"] == "agent.message"));
    let request = events
        .iter()
        .find(|event| event["type"] == "llm.request" && event["data"]["pass"] == "process")
        .expect("process request");
    assert!(request["data"]["messages"].as_array().unwrap().len() > 1);
    assert_eq!(request["data"]["tools"].as_array().unwrap().len(), 4);
    assert!(events
        .windows(2)
        .all(|pair| pair[0]["seq"].as_u64().unwrap() < pair[1]["seq"].as_u64().unwrap()));
}

#[tokio::test]
async fn sse_replays_the_same_bytes_as_the_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let session_id = start_and_wait(&app).await;
    let path = dir
        .path()
        .join("agentstream")
        .join(format!("{session_id}.jsonl"));
    let file = std::fs::read_to_string(&path).unwrap();
    let lines: HashMap<u64, String> = file
        .lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            (value["seq"].as_u64().unwrap(), line.to_owned())
        })
        .collect();

    let response = app
        .oneshot(
            Request::get(format!(
                "/api/agentstream/sessions/{session_id}/stream?from=1"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = read_sse_until(response, "\"turn.completed\"").await;

    let mut seen = 0;
    for block in text.split("\n\n") {
        let mut id = None;
        let mut event = None;
        let mut data = None;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("id: ") {
                id = Some(value);
            } else if let Some(value) = line.strip_prefix("event: ") {
                event = Some(value);
            } else if let Some(value) = line.strip_prefix("data: ") {
                data = Some(value);
            }
        }
        if event == Some("resync") {
            continue;
        }
        let (Some(id), Some(data)) = (id, data) else {
            continue;
        };
        let id: u64 = id.parse().unwrap();
        assert_eq!(
            data, lines[&id],
            "wire bytes differ from the file at seq {id}"
        );
        seen += 1;
    }
    assert_eq!(seen, lines.len());
}

#[tokio::test]
async fn events_and_stream_return_the_same_records_from_a_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let session_id = start_and_wait(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/agentstream/sessions/{session_id}/events?from=4&limit=500"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page = body_json(response).await;
    let expected = page["events"].as_array().unwrap().clone();
    assert!(!expected.is_empty());
    assert_eq!(page["from"], 4);
    assert_eq!(
        page["next"],
        expected.last().unwrap()["seq"].as_u64().unwrap() + 1
    );

    let response = app
        .oneshot(
            Request::get(format!(
                "/api/agentstream/sessions/{session_id}/stream?from=4"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let text = read_sse_until(response, "\"turn.completed\"").await;
    let mut actual = Vec::new();
    for block in text.split("\n\n") {
        let event = block.lines().find_map(|line| line.strip_prefix("event: "));
        if event == Some("resync") {
            continue;
        }
        if let Some(data) = block.lines().find_map(|line| line.strip_prefix("data: ")) {
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                actual.push(value);
            }
        }
    }
    assert_eq!(
        actual, expected,
        "the snapshot and the tail must serve the same records"
    );
}

#[tokio::test]
async fn a_cursor_below_the_retained_start_gets_a_resync_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let session_id = start_and_wait(&app).await;
    let original = dir
        .path()
        .join("agentstream")
        .join(format!("{session_id}.jsonl"));
    let file = std::fs::read_to_string(&original).unwrap();
    let retained = file
        .lines()
        .filter(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            value["seq"].as_u64().unwrap() >= 3
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Simulate a rotated-away prefix: only the later segment survives.
    std::fs::write(
        dir.path()
            .join("agentstream")
            .join(format!("{session_id}.1.jsonl")),
        format!("{retained}\n"),
    )
    .unwrap();
    std::fs::remove_file(&original).unwrap();

    let response = app
        .oneshot(
            Request::get(format!(
                "/api/agentstream/sessions/{session_id}/stream?from=1"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let text = read_sse_until(response, "event: resync").await;
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .expect("resync data");
    let page: Value = serde_json::from_str(data).unwrap();
    assert_eq!(page["sessionId"], session_id);
    assert_eq!(page["from"], 3);
    assert_eq!(page["events"][0]["seq"], 3);
}

#[tokio::test]
async fn health_reports_the_data_root() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let response = app
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let health = body_json(response).await;
    assert_eq!(health["status"], "ok");
    assert_eq!(health["dataDir"], dir.path().display().to_string());
}

#[tokio::test]
async fn retired_routes_are_not_served() {
    for path in [
        "/api/llm-activity",
        "/api/chat",
        "/api/cme/chat",
        "/api/state",
        "/api/focus",
        "/api/trees/x",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let response = app(dir.path())
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn an_empty_turn_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let response = app(dir.path())
        .oneshot(
            Request::post("/api/turns")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "text": "  " }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
