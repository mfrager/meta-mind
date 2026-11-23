//! Phase 4 exit criteria: the seven `data.*` tools.

use std::sync::Arc;

use mm_cme::ToolHandler;
use mm_data::data::DataError;
use mm_data::{DataBroker, DataCreateRequest, DataPatch, DataService, DataValue};
use mm_serve::cm_tools::CmTools;
use mm_serve::state_service::StateService;
use serde_json::{json, Map, Value};

fn prompts() -> Arc<mm_prompt::PromptStore> {
    Arc::new(
        mm_prompt::PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn fixture() -> (CmTools, Arc<DataService>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let service = Arc::clone(&data.data_service);
    let state = Arc::new(
        StateService::new(
            Arc::clone(&data.engine),
            Arc::clone(&data.agentstream),
            prompts(),
            data.base_dir.clone(),
        )
        .unwrap(),
    );
    let session = state
        .open_session("phase-four-user", None, 1_000)
        .await
        .unwrap()
        .session
        .session_id;
    let tools = CmTools::new(state)
        .with_session(session)
        .with_data(service.clone());
    (tools, service, temp)
}

async fn call(tools: &CmTools, name: &str, arguments: Value) -> Value {
    let raw = tools
        .call(name, &arguments.to_string())
        .await
        .expect("tool returned a JSON envelope");
    serde_json::from_str(&raw).expect("the envelope is valid JSON")
}

fn create_args(class: &str, label: &str) -> Value {
    let mut properties = Map::new();
    properties.insert("rdfs:label".into(), json!(label));
    json!({ "type": class, "properties": properties })
}

fn assert_ok(envelope: &Value) {
    assert_eq!(envelope["ok"], json!(true), "{envelope}");
}

fn error_code(envelope: &Value) -> &str {
    envelope["error"]["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn create_search_update_and_soft_delete_round_trip() {
    let (tools, _service, _temp) = fixture().await;

    let created = call(
        &tools,
        "data.create",
        create_args("mm:Project", "Auth service"),
    )
    .await;
    assert_ok(&created);
    assert_eq!(created["result"]["index"], json!("healthy"));
    assert_eq!(created["result"]["indexPending"], json!(false));
    assert!(created["dataRevision"].as_u64().unwrap() >= 1);
    let iri = created["result"]["entity"]["iri"]
        .as_str()
        .unwrap()
        .to_string();
    let revision = created["result"]["entity"]["revision"].as_u64().unwrap();

    // The label+type duplicate guard returns the existing entity instead of
    // creating a second one; an explicit id is authoritative and conflicts.
    let duplicate = call(
        &tools,
        "data.create",
        create_args("mm:Project", "Auth service"),
    )
    .await;
    assert_ok(&duplicate);
    assert_eq!(duplicate["result"]["duplicate"], json!(true));
    assert_eq!(duplicate["result"]["entity"]["iri"], json!(iri));
    let explicit = call(
        &tools,
        "data.create",
        json!({
            "type": "mm:Project",
            "id": iri,
            "properties": { "rdfs:label": "Explicit" }
        }),
    )
    .await;
    assert_eq!(error_code(&explicit), "DATA_CONFLICT", "{explicit}");

    let found = call(&tools, "data.search", json!({ "query": "auth" })).await;
    assert_ok(&found);
    assert_eq!(found["result"]["count"], json!(1), "{found}");
    assert_eq!(found["result"]["hits"][0]["iri"], json!(iri));

    let updated = call(
        &tools,
        "data.update",
        json!({
            "subject": iri,
            "properties": { "rdfs:label": "Billing service" },
            "expected_revision": revision,
        }),
    )
    .await;
    assert_ok(&updated);
    let updated_revision = updated["result"]["entity"]["revision"].as_u64().unwrap();
    assert!(updated_revision > revision);

    let no_op = call(
        &tools,
        "data.update",
        json!({ "subject": iri, "properties": { "rdfs:label": "Billing service" } }),
    )
    .await;
    assert_ok(&no_op);
    assert_eq!(no_op["result"]["unchanged"], json!(true));
    assert_eq!(
        no_op["result"]["entity"]["revision"],
        json!(updated_revision)
    );

    let old = call(&tools, "data.search", json!({ "query": "auth" })).await;
    assert_eq!(old["result"]["count"], json!(0), "{old}");
    let new = call(&tools, "data.search", json!({ "query": "billing" })).await;
    assert_eq!(new["result"]["count"], json!(1), "{new}");

    let queried = call(&tools, "data.query", json!({ "subject": iri })).await;
    assert_ok(&queried);
    assert_eq!(
        queried["result"]["entities"][0]["label"],
        json!("Billing service")
    );

    let deleted = call(&tools, "data.delete", json!({ "subject": iri })).await;
    assert_ok(&deleted);
    assert_eq!(deleted["result"]["entity"]["status"], json!("deleted"));

    let excluded = call(&tools, "data.query", json!({ "subject": iri })).await;
    assert_eq!(excluded["result"]["count"], json!(0), "{excluded}");
    let included = call(
        &tools,
        "data.query",
        json!({ "subject": iri, "include_deleted": true }),
    )
    .await;
    assert_eq!(included["result"]["count"], json!(1), "{included}");
    assert_eq!(
        included["result"]["entities"][0]["status"],
        json!("deleted")
    );

    let gone = call(&tools, "data.search", json!({ "query": "billing" })).await;
    assert_eq!(gone["result"]["count"], json!(0), "{gone}");

    let again = call(&tools, "data.delete", json!({ "subject": iri })).await;
    assert_ok(&again);
    assert_eq!(again["result"]["unchanged"], json!(true));
}

#[tokio::test]
async fn hard_delete_is_refused_without_authority() {
    let (tools, _service, _temp) = fixture().await;
    let created = call(&tools, "data.create", create_args("mm:Note", "Temporary")).await;
    assert_ok(&created);
    let iri = created["result"]["entity"]["iri"].as_str().unwrap();

    let refused = call(
        &tools,
        "data.delete",
        json!({ "subject": iri, "mode": "hard" }),
    )
    .await;
    assert_eq!(refused["ok"], json!(false));
    assert_eq!(error_code(&refused), "INVALID_ARGUMENTS", "{refused}");

    let still_there = call(&tools, "data.query", json!({ "subject": iri })).await;
    assert_eq!(still_there["result"]["count"], json!(1));
}

#[tokio::test]
async fn type_errors_and_the_being_singleton_are_typed() {
    let (tools, service, _temp) = fixture().await;

    let first = call(&tools, "data.create", create_args("mm:Being", "Metamind")).await;
    assert_ok(&first);
    let second = call(&tools, "data.create", create_args("mm:Being", "Another")).await;
    assert_eq!(error_code(&second), "SCHEMA_VIOLATION", "{second}");

    let unknown = call(&tools, "data.create", create_args("nope:Thing", "x")).await;
    assert_eq!(error_code(&unknown), "SCHEMA_VIOLATION", "{unknown}");

    let managed = call(&tools, "data.create", create_args("mm:Plan", "Ship it")).await;
    assert_eq!(error_code(&managed), "INVALID_ARGUMENTS", "{managed}");

    let kernel_graph = call(
        &tools,
        "data.create",
        json!({
            "type": "mm:Note",
            "properties": { "rdfs:label": "Smuggled" },
            "graph": "ontology"
        }),
    )
    .await;
    assert_eq!(
        error_code(&kernel_graph),
        "INVALID_ARGUMENTS",
        "{kernel_graph}"
    );

    let person = call(&tools, "data.create", create_args("foaf:Person", "Ada")).await;
    assert_eq!(error_code(&person), "SCHEMA_VIOLATION", "{person}");

    let mut properties = Map::new();
    properties.insert("rdfs:label".into(), json!("Ada"));
    properties.insert("foaf:name".into(), json!("Ada"));
    let named = call(
        &tools,
        "data.create",
        json!({ "type": "foaf:Person", "properties": properties }),
    )
    .await;
    assert_ok(&named);

    // A declared literal datatype that the lexical form cannot satisfy is
    // refused rather than coerced.
    let bad_type = service
        .create(DataCreateRequest {
            r#type: "mm:Note".into(),
            id: None,
            properties: vec![DataPatch {
                predicate: "mm:confidence".into(),
                values: vec![DataValue::Literal {
                    value: "not-a-number".into(),
                    datatype: Some("http://www.w3.org/2001/XMLSchema#double".into()),
                    language: None,
                }],
            }],
            schema: None,
            graph: None,
            idempotency_key: None,
        })
        .await;
    assert!(
        matches!(bad_type, Err(DataError::SchemaViolation(_))),
        "{bad_type:?}"
    );
}

#[tokio::test]
async fn a_stale_revision_is_a_data_conflict() {
    let (tools, _service, _temp) = fixture().await;
    let created = call(&tools, "data.create", create_args("mm:Note", "Versioned")).await;
    let iri = created["result"]["entity"]["iri"]
        .as_str()
        .unwrap()
        .to_string();
    let revision = created["result"]["entity"]["revision"].as_u64().unwrap();

    let first = call(
        &tools,
        "data.update",
        json!({
            "subject": iri,
            "properties": { "rdfs:label": "Second" },
            "expected_revision": revision,
        }),
    )
    .await;
    assert_ok(&first);

    let stale = call(
        &tools,
        "data.update",
        json!({
            "subject": iri,
            "properties": { "rdfs:label": "Stale" },
            "expected_revision": revision,
        }),
    )
    .await;
    assert_eq!(error_code(&stale), "DATA_CONFLICT", "{stale}");
    assert_eq!(stale["error"]["retryable"], json!(true));
}

#[tokio::test]
async fn schema_and_namespace_tools_read_and_manage_the_catalog() {
    let (tools, _service, _temp) = fixture().await;

    let list = call(&tools, "data.schema", json!({ "action": "list" })).await;
    assert_ok(&list);
    let prefixes: Vec<&str> = list["result"]["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|schema| schema["prefix"].as_str())
        .collect();
    assert!(prefixes.contains(&"mm"), "{prefixes:?}");
    assert!(prefixes.contains(&"foaf"), "{prefixes:?}");

    let validation = call(
        &tools,
        "data.schema",
        json!({ "action": "validate", "graph": "world" }),
    )
    .await;
    assert_ok(&validation);
    assert_eq!(validation["result"]["conforms"], json!(true));

    let registered = call(
        &tools,
        "data.schema",
        json!({
            "action": "register",
            "prefix": "ex",
            "namespace": "https://example.com/ns#",
            "source": "inline",
            "format": "turtle",
            "body": "@prefix owl: <http://www.w3.org/2002/07/owl#> . @prefix ex: <https://example.com/ns#> . ex:Widget a owl:Class .",
        }),
    )
    .await;
    assert_ok(&registered);
    assert_eq!(registered["result"]["schema"]["active"], json!(false));

    let activated = call(
        &tools,
        "data.schema",
        json!({ "action": "activate", "schema": "ex" }),
    )
    .await;
    assert_ok(&activated);
    assert_eq!(activated["result"]["schema"]["active"], json!(true));

    let widget = call(
        &tools,
        "data.create",
        json!({
            "type": "ex:Widget",
            "properties": { "rdfs:label": "Gear" },
        }),
    )
    .await;
    assert_ok(&widget);
    assert_eq!(
        widget["result"]["entity"]["types"][0],
        json!("https://example.com/ns#Widget")
    );

    let bound = call(
        &tools,
        "data.namespace",
        json!({
            "action": "bind",
            "prefix": "exp",
            "namespace": "https://example.com/project#",
        }),
    )
    .await;
    assert_ok(&bound);
    assert_eq!(bound["result"]["namespace"]["builtin"], json!(false));

    let collision = call(
        &tools,
        "data.namespace",
        json!({
            "action": "bind",
            "prefix": "exp",
            "namespace": "https://example.com/other#",
        }),
    )
    .await;
    assert_eq!(error_code(&collision), "NAMESPACE_CONFLICT", "{collision}");

    let expanded = call(
        &tools,
        "data.namespace",
        json!({ "action": "expand", "iri": "mm:Note" }),
    )
    .await;
    assert_ok(&expanded);
    assert_eq!(
        expanded["result"]["iri"],
        json!("https://metamind.dev/ontology#Note")
    );
}

#[tokio::test]
async fn a_hundred_result_page_has_more_and_a_valid_envelope() {
    let (tools, service, _temp) = fixture().await;
    // Seed directly, so the turn quota is not what this test measures.
    for index in 0..101 {
        service
            .create(mm_data::DataCreateRequest {
                r#type: "mm:Note".into(),
                id: None,
                properties: vec![mm_data::DataPatch {
                    predicate: "rdfs:label".into(),
                    values: vec![mm_data::DataValue::text(format!("Note {index}"))],
                }],
                schema: None,
                graph: None,
                idempotency_key: None,
            })
            .await
            .unwrap();
    }

    let first = call(&tools, "data.query", json!({ "limit": 100 })).await;
    assert_ok(&first);
    assert_eq!(first["result"]["count"], json!(100), "{first}");
    assert_eq!(first["result"]["entities"].as_array().unwrap().len(), 100);
    assert_eq!(first["result"]["hasMore"], json!(true));
    assert_eq!(first["result"]["nextOffset"], json!(100));

    let second = call(&tools, "data.query", json!({ "limit": 100, "offset": 100 })).await;
    assert_ok(&second);
    assert_eq!(second["result"]["count"], json!(1));
    assert_eq!(second["result"]["hasMore"], json!(false));

    let by_class = call(
        &tools,
        "data.query",
        json!({ "class": "mm:Note", "limit": 100 }),
    )
    .await;
    assert_ok(&by_class);
    assert_eq!(by_class["result"]["count"], json!(100), "{by_class}");
    assert_eq!(by_class["result"]["hasMore"], json!(true));
}

#[tokio::test]
async fn the_thirteenth_write_is_a_budget_outcome() {
    let (tools, _service, _temp) = fixture().await;
    for index in 0..12 {
        let envelope = call(
            &tools,
            "data.create",
            create_args("mm:Note", &format!("Quota {index}")),
        )
        .await;
        assert_ok(&envelope);
    }
    let thirteenth = call(&tools, "data.create", create_args("mm:Note", "Quota 12")).await;
    assert_eq!(thirteenth["ok"], json!(false));
    assert_eq!(error_code(&thirteenth), "BUDGET", "{thirteenth}");

    // Reads do not consume the write quota.
    let search = call(&tools, "data.search", json!({ "query": "quota" })).await;
    assert_ok(&search);
}
