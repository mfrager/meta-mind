//! Adversarial fixtures: a violating payload must never become a typed value.
//!
//! The fixtures are answered by a client that returns the payload verbatim, so the
//! substrate is the only thing standing between a malformed payload and a caller.
//! Each case asserts more than "it failed": the failure has to be the *right* kind,
//! each rejection has to leave exactly one repair row, and the record that says so
//! must not contain the payload.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{fixture, Harness};
use mm_llm::client::{CacheMode, LlmRequest, Message, Purpose, SchemaId};
use mm_llm::mock::{MockClient, ScriptedResponse};
use mm_llm::LlmError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct MalformedFixture {
    name: String,
    schema: serde_json::Value,
    payload: String,
    expect: String,
}

fn load_fixtures(dir: &str) -> Vec<MalformedFixture> {
    let root = fixture(dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            let raw = std::fs::read_to_string(path).unwrap();
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

/// The plan names these five; the set may grow but never shrink.
const REQUIRED: [&str; 5] = [
    "truncated",
    "wrong_types",
    "not_json",
    "schema_extra_field",
    "empty",
];

#[tokio::test]
async fn every_malformed_fixture_is_rejected_for_its_declared_reason() {
    let fixtures = load_fixtures("bench/llm/malformed");
    assert!(
        fixtures.len() >= REQUIRED.len(),
        "the fixture set must not shrink: {fixtures:?}"
    );
    for required in REQUIRED {
        assert!(
            fixtures.iter().any(|f| f.name == required),
            "the `{required}` fixture is missing"
        );
    }

    let h = Harness::new(1).await;
    let mut rejections = 0usize;

    for f in &fixtures {
        let client = Arc::new(MockClient::constant(ScriptedResponse::text(
            f.payload.clone(),
        )));
        let mut service = h.service_with(client.clone());
        service
            .register_schema(SchemaId::new(f.name.clone()), f.schema.clone())
            .unwrap();

        let request = LlmRequest::new(Purpose::Extract, vec![Message::user("adversarial fixture")])
            .with_schema(SchemaId::new(f.name.clone()))
            .with_cache(CacheMode::Bypass);

        match service.call(request).await {
            // The invariant: never a typed value, and never a coercion. The payload
            // is rejected for the reason the fixture declares.
            Err(LlmError::SchemaRejected { schema_id, detail }) => {
                assert_eq!(schema_id, f.name, "the rejection names its schema");
                assert!(
                    detail.contains(&f.expect),
                    "{}: expected a `{}` rejection, got {detail}",
                    f.name,
                    f.expect
                );
                rejections += 1;
                // One bounded repair pass, then the call is rejected.
                assert_eq!(client.calls(), 2, "{}: one repair attempt", f.name);
            }
            Err(other) => panic!("{}: unexpected error {other}", f.name),
            Ok(response) => panic!(
                "{}: a violating payload was accepted as {:?}",
                f.name, response.text
            ),
        }
    }

    // One repair row per rejection, and the ledger stays complete: a rejected call
    // is a call that happened.
    let repairs = h.store.row_count("llm_repair_attempts").await.unwrap();
    assert_eq!(repairs as usize, rejections);
    assert_eq!(
        h.store.row_count("llm_calls").await.unwrap() as usize,
        rejections
    );
    assert_eq!(service_complete(&h).await as usize, rejections);

    // Exactly one reject record per rejection, carrying the kind and a length —
    // never the payload.
    let rejects = h.records_of(mm_log::codes::LLM_SCHEMA_REJECT);
    assert_eq!(rejects.len(), rejections);
    for record in &rejects {
        assert!(record["fields"]["error_kind"].is_string(), "{record}");
        assert!(record["fields"]["raw_len"].is_u64(), "{record}");
    }
    for f in &fixtures {
        if !f.payload.is_empty() {
            assert!(
                !h.raw_log().contains(&f.payload),
                "the payload of `{}` reached a sink",
                f.name
            );
        }
    }
    h.shutdown().await;
}

/// A11: the fixtures are a real test only if the same schemas accept a good value.
#[tokio::test]
async fn the_same_schemas_accept_a_valid_payload() {
    let fixtures = load_fixtures("bench/llm/malformed");
    let extract = fixtures
        .iter()
        .find(|f| f.name == "schema_extra_field")
        .expect("the extract fixture is present");

    let h = Harness::new(1).await;
    let client = Arc::new(MockClient::constant(ScriptedResponse::text(
        r#"{"name":"Ada Lovelace","score":9}"#,
    )));
    let mut service = h.service_with(client);
    service
        .register_schema(SchemaId::new("extract.v1"), extract.schema.clone())
        .unwrap();

    let response = service
        .call(
            LlmRequest::new(Purpose::Extract, vec![Message::user("Ada Lovelace")])
                .with_schema(SchemaId::new("extract.v1")),
        )
        .await
        .expect("a valid payload must be accepted");
    assert!(response.schema_valid);
    assert_eq!(h.store.row_count("llm_repair_attempts").await.unwrap(), 0);
    h.shutdown().await;
}

async fn service_complete(h: &Harness) -> u64 {
    mm_llm::accounting::assert_complete(&h.store).await.unwrap()
}
