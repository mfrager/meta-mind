//! Phase 5 exit criteria: metamind plan, think, memory, log, and act.

use std::sync::Arc;

use mm_cme::{read_only_schemas, ToolHandler};
use mm_data::{
    ActionStore, DataBroker, DataCreateRequest, DataPatch, DataValue, FormValues,
    InternalActionRegistry, RdfEngine, RdfPlanStore, Recurrence, ScheduleSpec, ScheduledActionSpec,
    ScheduledActionStatus,
};
use mm_serve::cm_tools::CmTools;
use mm_serve::state_service::StateService;
use serde_json::{json, Value};

fn prompts() -> Arc<mm_prompt::PromptStore> {
    Arc::new(
        mm_prompt::PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn fixture() -> (
    CmTools,
    Arc<mm_data::DataService>,
    Arc<RdfEngine>,
    Arc<StateService>,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let service = Arc::clone(&data.data_service);
    let engine = Arc::clone(&data.engine);
    let state = Arc::new(
        StateService::new(
            Arc::clone(&engine),
            Arc::clone(&data.agentstream),
            prompts(),
            data.base_dir.clone(),
        )
        .unwrap(),
    );
    let session = state
        .open_session("phase-five-user", None, 1_000)
        .await
        .unwrap()
        .session
        .session_id;
    let tools = CmTools::new(Arc::clone(&state))
        .with_session(session)
        .with_data(Arc::clone(&service))
        .with_metamind(
            Arc::clone(&engine),
            Arc::clone(&service),
            Arc::clone(state.prompts()),
            None,
            None,
        );
    (tools, service, engine, state, temp)
}

async fn call(tools: &CmTools, name: &str, arguments: Value) -> Value {
    let raw = tools
        .call(name, &arguments.to_string())
        .await
        .expect("metamind tool returned an envelope");
    serde_json::from_str(&raw).expect("valid JSON envelope")
}

fn assert_ok(envelope: &Value) {
    assert_eq!(envelope["ok"], json!(true), "{envelope}");
}

fn error_code(envelope: &Value) -> &str {
    envelope["error"]["code"].as_str().unwrap_or("")
}

fn markdown_plan(content: &str) -> Value {
    json!({"action": "create", "goal": "Ship the cache", "format": "markdown", "content": content})
}

#[tokio::test]
async fn plans_version_and_the_registry_never_overwrite() {
    let (tools, _data, engine, _state, _temp) = fixture().await;
    let created = call(
        &tools,
        "metamind.plan",
        markdown_plan("# Plan\n\n1. Warm cache"),
    )
    .await;
    assert_ok(&created);
    let plan_id = created["result"]["planId"].as_str().unwrap().to_string();
    assert_eq!(created["result"]["currentVersion"], json!(1));
    assert_eq!(created["result"]["revision"], json!(1));

    let update = call(
        &tools,
        "metamind.plan",
        json!({
            "action": "update",
            "plan_id": plan_id,
            "expected_revision": 1,
            "content": "# Plan v2\n\n1. Warm and verify"
        }),
    )
    .await;
    assert_ok(&update);
    assert_eq!(update["result"]["currentVersion"], json!(2));

    let get = call(
        &tools,
        "metamind.plan",
        json!({"action": "get", "plan_id": plan_id}),
    )
    .await;
    assert_ok(&get);
    assert_eq!(get["result"]["versions"].as_array().unwrap().len(), 2);
    assert_eq!(
        get["result"]["versions"][0]["body"],
        json!("# Plan\n\n1. Warm cache")
    );

    let stale = call(
        &tools,
        "metamind.plan",
        json!({
            "action": "update",
            "plan_id": plan_id,
            "expected_revision": 1,
            "content": "# stale"
        }),
    )
    .await;
    assert_eq!(error_code(&stale), "PLAN_CONFLICT", "{stale}");

    let registry = RdfPlanStore::new(engine).registry().await.unwrap();
    assert_eq!(registry.plans, vec![plan_id]);
}

#[tokio::test]
async fn markdown_and_structured_plans_both_validate() {
    let (tools, _data, _engine, _state, _temp) = fixture().await;
    let structured = json!({
        "action": "create",
        "goal": "Make the refresh test deterministic",
        "format": "structured",
        "content": {
            "schema": "mm.plan.v1",
            "goal": "Make the refresh test deterministic",
            "assumptions": ["the production clock is authoritative"],
            "steps": [
                {"id": "s1", "title": "reproduce", "dependsOn": [], "owner": "cm"},
                {"id": "s2", "title": "inject clock", "dependsOn": ["s1"], "owner": "agent"}
            ],
            "risks": ["timezone"],
            "openQuestions": [],
            "successCriteria": ["20/20 pass"]
        }
    });
    let created = call(&tools, "metamind.plan", structured).await;
    assert_ok(&created);
    assert_eq!(created["result"]["steps"].as_array().unwrap().len(), 2);

    let generated = call(
        &tools,
        "metamind.plan",
        json!({"action": "create", "goal": "Plan a migration"}),
    )
    .await;
    assert_ok(&generated);
    assert_eq!(generated["result"]["currentVersion"], json!(1));
    assert!(!generated["result"]["body"].as_str().unwrap().is_empty());

    let invalid = call(
        &tools,
        "metamind.plan",
        json!({
            "action": "create",
            "goal": "bad",
            "format": "structured",
            "content": {"schema": "mm.plan.v1", "goal": "bad", "steps": [
                {"id": "a", "title": "a", "dependsOn": ["missing"]}
            ]}
        }),
    )
    .await;
    assert_eq!(error_code(&invalid), "INVALID_ARGUMENTS", "{invalid}");
}

#[tokio::test]
async fn plan_controls_are_state_checked_and_audited() {
    let (tools, _data, _engine, _state, _temp) = fixture().await;
    let created = call(
        &tools,
        "metamind.plan",
        json!({
            "action": "create",
            "goal": "One step",
            "format": "structured",
            "content": {"schema": "mm.plan.v1", "goal": "One step",
                "steps": [{"id": "s1", "title": "do it"}]}
        }),
    )
    .await;
    assert_ok(&created);
    let plan_id = created["result"]["planId"].as_str().unwrap().to_string();

    let started = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_start", "plan_id": plan_id, "expected_revision": 1}),
    )
    .await;
    assert_ok(&started);
    assert_eq!(started["result"]["status"], json!("active"));

    let paused = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_pause", "plan_id": plan_id, "expected_revision": 2}),
    )
    .await;
    assert_ok(&paused);
    assert_eq!(paused["result"]["status"], json!("paused"));

    let resumed = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_resume", "plan_id": plan_id, "expected_revision": 3}),
    )
    .await;
    assert_ok(&resumed);

    let advanced = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_advance_step", "plan_id": plan_id, "expected_revision": 4}),
    )
    .await;
    assert_ok(&advanced);
    assert_eq!(advanced["result"]["status"], json!("done"));

    let completed = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_complete", "plan_id": plan_id, "expected_revision": 5}),
    )
    .await;
    assert_ok(&completed);
    assert_eq!(completed["result"]["status"], json!("completed"));

    let stale = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_pause", "plan_id": plan_id, "expected_revision": 1}),
    )
    .await;
    assert_eq!(error_code(&stale), "PLAN_CONFLICT", "{stale}");

    let second = call(
        &tools,
        "metamind.plan",
        json!({"action": "create", "goal": "Stop me", "content": "# stop"}),
    )
    .await;
    assert_ok(&second);
    let second_id = second["result"]["planId"].as_str().unwrap();
    let started = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_start", "plan_id": second_id, "expected_revision": 1}),
    )
    .await;
    assert_ok(&started);
    let stopped = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_stop", "plan_id": second_id, "expected_revision": 2}),
    )
    .await;
    assert_ok(&stopped);
    assert_eq!(stopped["result"]["status"], json!("stopped"));
    let resumed = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_resume", "plan_id": second_id, "expected_revision": 3}),
    )
    .await;
    assert_eq!(error_code(&resumed), "INVALID_ARGUMENTS", "{resumed}");
    let abandoned = call(
        &tools,
        "metamind.act",
        json!({"action": "plan_abandon", "plan_id": second_id, "expected_revision": 3}),
    )
    .await;
    assert_ok(&abandoned);
    assert_eq!(abandoned["result"]["status"], json!("abandoned"));
}

#[tokio::test]
async fn think_reads_data_and_never_returns_the_inner_prompt() {
    let (tools, service, _engine, _state, _temp) = fixture().await;
    let created = service
        .create(DataCreateRequest {
            r#type: "mm:Project".into(),
            id: None,
            properties: vec![DataPatch {
                predicate: "rdfs:label".into(),
                values: vec![DataValue::text("Redis cache cluster")],
            }],
            schema: None,
            graph: None,
            idempotency_key: None,
        })
        .await
        .unwrap();
    let iri = created.entity.iri;

    let thought = call(
        &tools,
        "metamind.think",
        json!({"question": "What cache cluster do we run?", "focus": [iri]}),
    )
    .await;
    assert_ok(&thought);
    assert_eq!(
        thought["result"]["findings"][0]["evidence"][0]["id"],
        json!(iri)
    );
    assert!(thought["result"].get("prompt").is_none());
    assert!(thought["result"].get("system").is_none());

    let names: Vec<String> = read_only_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    assert!(!names.iter().any(|name| name == "data.create"));
    assert!(names.contains(&"data.query".to_string()));
}

#[tokio::test]
async fn memory_remember_recall_forget_round_trips_and_refuses_secrets() {
    let (tools, _data, _engine, _state, _temp) = fixture().await;
    let remembered = call(
        &tools,
        "metamind.memory",
        json!({
            "action": "remember",
            "content": "The operator prefers dark mode",
            "kinds": ["user"],
            "importance": 0.8
        }),
    )
    .await;
    assert_ok(&remembered);
    let memory_id = remembered["result"]["memory"]["id"].as_str().unwrap();

    let recalled = call(
        &tools,
        "metamind.memory",
        json!({"action": "recall", "query": "dark mode", "kinds": ["user"]}),
    )
    .await;
    assert_ok(&recalled);
    assert_eq!(recalled["result"]["count"], json!(1), "{recalled}");

    let forgotten = call(
        &tools,
        "metamind.memory",
        json!({"action": "forget", "memory_id": memory_id}),
    )
    .await;
    assert_ok(&forgotten);
    assert_eq!(forgotten["result"]["forgotten"], json!(1));

    let gone = call(
        &tools,
        "metamind.memory",
        json!({"action": "recall", "query": "dark mode", "kinds": ["user"]}),
    )
    .await;
    assert_eq!(gone["result"]["count"], json!(0), "{gone}");

    let secret = call(
        &tools,
        "metamind.memory",
        json!({"action": "remember", "content": "password=hunter2", "kinds": ["user"]}),
    )
    .await;
    assert_eq!(error_code(&secret), "INVALID_ARGUMENTS", "{secret}");
}

#[tokio::test]
async fn logs_round_trip_across_agentstream_and_rdf() {
    let (tools, _data, _engine, _state, _temp) = fixture().await;
    let appended = call(
        &tools,
        "metamind.log",
        json!({"action": "append", "level": "info", "message": "Cache warmed"}),
    )
    .await;
    assert_ok(&appended);

    let overview = call(&tools, "metamind.log", json!({"action": "overview"})).await;
    assert_ok(&overview);
    assert!(overview["result"]["entries"].as_u64().unwrap() >= 1);
    assert!(overview["result"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["source"] == "agentstream"));

    let latest = call(
        &tools,
        "metamind.log",
        json!({"action": "latest", "source": "rdf", "query": "Cache"}),
    )
    .await;
    assert_ok(&latest);
    assert_eq!(latest["result"]["count"], json!(1));

    for message in ["segment two", "segment three"] {
        let appended = call(
            &tools,
            "metamind.log",
            json!({"action": "append", "message": message}),
        )
        .await;
        assert_ok(&appended);
    }
    let first = call(
        &tools,
        "metamind.log",
        json!({"action": "segment", "source": "rdf", "limit": 2}),
    )
    .await;
    assert_ok(&first);
    assert_eq!(first["result"]["hasMore"], json!(true));
    let cursor = first["result"]["nextCursor"].as_str().unwrap();
    let second = call(
        &tools,
        "metamind.log",
        json!({"action": "segment", "source": "rdf", "limit": 2, "cursor": cursor}),
    )
    .await;
    assert_ok(&second);
    assert_eq!(second["result"]["hasMore"], json!(false));
    let first_ids: Vec<&str> = first["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["id"].as_str())
        .collect();
    assert!(second["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| !first_ids.contains(&entry["id"].as_str().unwrap())));

    let secret = call(
        &tools,
        "metamind.log",
        json!({"action": "append", "message": "api_key=sk-live"}),
    )
    .await;
    assert_eq!(error_code(&secret), "INVALID_ARGUMENTS", "{secret}");
}

#[tokio::test]
async fn act_runs_known_kinds_schedules_cancels_and_introspects() {
    let (tools, _data, _engine, _state, _temp) = fixture().await;
    let immediate = call(
        &tools,
        "metamind.act",
        json!({"action": "internal", "kind": "joke", "input": "a short aside"}),
    )
    .await;
    assert_ok(&immediate);
    assert_eq!(immediate["result"]["kind"], json!("joke"));

    let unknown = call(
        &tools,
        "metamind.act",
        json!({"action": "internal", "kind": "not_a_kind", "input": "x"}),
    )
    .await;
    assert_eq!(error_code(&unknown), "ACTION_UNKNOWN", "{unknown}");

    let scheduled = call(
        &tools,
        "metamind.act",
        json!({
            "action": "internal",
            "kind": "joke",
            "input": "later",
            "mode": "scheduled",
            "schedule": {"delay_seconds": 60}
        }),
    )
    .await;
    assert_ok(&scheduled);
    let action_id = scheduled["result"]["actionId"].as_str().unwrap();
    assert_eq!(scheduled["result"]["status"], json!("scheduled"));

    let cancelled = call(
        &tools,
        "metamind.act",
        json!({"action": "cancel", "action_id": action_id}),
    )
    .await;
    assert_ok(&cancelled);
    assert_eq!(cancelled["result"]["cancelled"], json!(true));

    let cron = call(
        &tools,
        "metamind.act",
        json!({
            "action": "internal",
            "kind": "joke",
            "input": "every five minutes",
            "mode": "scheduled",
            "schedule": {"cron": "*/5 * * * *", "recurrence": "cron", "max_runs": 2}
        }),
    )
    .await;
    assert_ok(&cron);
    assert_eq!(cron["result"]["recurrence"], json!("cron"));
    assert!(cron["result"]["nextRunAt"].is_string());
    let cron_id = cron["result"]["actionId"].as_str().unwrap();
    let cancelled = call(
        &tools,
        "metamind.act",
        json!({"action": "cancel", "action_id": cron_id}),
    )
    .await;
    assert_ok(&cancelled);

    let report = call(
        &tools,
        "metamind.act",
        json!({"action": "introspection", "input": "what are we missing?"}),
    )
    .await;
    assert_ok(&report);
    assert!(report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["area"] == "plans"));
}

#[tokio::test]
async fn scheduled_actions_are_idempotent_due_and_capped() {
    let (_tools, _data, engine, _state, _temp) = fixture().await;
    let store = ActionStore::new(engine, Arc::new(InternalActionRegistry::builtin()));
    let now = mm_data::rdf::now_ms();
    let spec = ScheduledActionSpec {
        session_id: "session-a".into(),
        kind: "joke".into(),
        input: Some("later".into()),
        parameters: FormValues::new(),
        schedule: ScheduleSpec::Delay { delay_ms: 1_000 },
        idempotency_key: "same-key-123".into(),
        status: ScheduledActionStatus::Scheduled,
        created_at: now,
        last_run_at: None,
        runs: 0,
    };
    let first = store
        .schedule(spec.clone(), Recurrence::Once, None, now)
        .await
        .unwrap();
    let second = store
        .schedule(spec, Recurrence::Once, None, now)
        .await
        .unwrap();
    assert_eq!(first.id, second.id);

    let due = store
        .claim_due(first.next_run_at.unwrap() + 1)
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    let finished = store
        .finish_run(&first.id, now + 2_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.spec.status, ScheduledActionStatus::Done);
    assert_eq!(finished.spec.runs, 1);

    let cron = ScheduledActionSpec {
        idempotency_key: "cron-key-123".into(),
        schedule: ScheduleSpec::Cron {
            expression: "*/5 * * * *".into(),
            timezone: Some("UTC".into()),
        },
        ..first.spec.clone()
    };
    let recurring = store
        .schedule(cron, Recurrence::Cron, Some(2), now)
        .await
        .unwrap();
    let due_at = recurring.next_run_at.unwrap();
    store.claim_due(due_at + 1).await.unwrap();
    let once = store
        .finish_run(&recurring.id, due_at + 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(once.spec.status, ScheduledActionStatus::Scheduled);
    assert_eq!(once.spec.runs, 1);
    let due_at = once.next_run_at.unwrap();
    store.claim_due(due_at + 1).await.unwrap();
    let capped = store
        .finish_run(&recurring.id, due_at + 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capped.spec.status, ScheduledActionStatus::Done);
    assert_eq!(capped.spec.runs, 2);
}

#[tokio::test]
async fn subagent_traces_are_registered_on_the_owning_session() {
    let (tools, _data, _engine, state, _temp) = fixture().await;
    let created = call(&tools, "metamind.plan", markdown_plan("# Trace")).await;
    assert_ok(&created);
    let page = state
        .list_traces(tools.session_id().unwrap(), None, 100)
        .await
        .unwrap();
    assert!(page
        .items
        .iter()
        .any(|trace| matches!(trace.kind, mm_data::TraceKind::Plan)));
}
