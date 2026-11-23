//! Integration tests for the data layer and the unified memory pipeline.

use mm_data::{
    initialize_data_layer, AgentstreamEvent, AppSpec, DatasetSpec, EntityId, EpisodicTurn,
    EventProvenance, EventType, InitConfig, ProjectId, ProjectSpec, SearchFilters, UserId,
    UserRecord, UserSpec,
};
use serde_json::json;

fn init_config(temp: &tempfile::TempDir) -> InitConfig {
    InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..InitConfig::default()
    }
}

#[tokio::test]
async fn phase0_shapes_parse_and_validate_an_empty_graph() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(init_config(&temp))
        .await
        .expect("initialize");
    let shapes_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ontology/shapes");
    for name in [
        "mm-shapes.ttl",
        "data.shacl.ttl",
        "plan.shacl.ttl",
        "session.shacl.ttl",
        "log.shacl.ttl",
    ] {
        let report = data
            .engine
            .store()
            .validate_with("world", &shapes_dir.join(name))
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(report.conforms, "{name}: {:?}", report.violations);
    }
}

#[tokio::test]
async fn registries_mint_canonical_iris_and_read_legacy_rows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(init_config(&temp))
        .await
        .expect("initialize");

    let user = data
        .users
        .create_user(UserSpec {
            name: "Legacy Reader".to_owned(),
            ..UserSpec::default()
        })
        .await
        .expect("create user");
    assert_eq!(
        mm_data::rdf::user_iri(&user.user_id),
        format!("https://metamind.dev/data/user/{}", user.user_id)
    );

    // Simulate a row written before Phase 12: legacy subject, same payload.
    let old = UserRecord {
        user_id: "legacy-user".to_owned(),
        name: "Old User".to_owned(),
        email: None,
        role: None,
        home_dir: "users/legacy-user/home".to_owned(),
        created_at: user.created_at,
    };
    let payload = serde_json::to_value(&old).expect("payload");
    mm_data::rdf::upsert(
        &data.engine,
        mm_data::rdf::USER_INDEX_GRAPH,
        &mm_data::rdf::legacy_user_iri(&old.user_id),
        mm_data::users::USER_CLASS,
        Some(&old.name),
        &payload,
    )
    .await
    .expect("legacy row");

    let found = data
        .users
        .get_user(&old.user_id)
        .await
        .expect("get legacy user")
        .expect("legacy user present");
    assert_eq!(found.name, old.name);
    assert_eq!(data.users.list_users().await.expect("list").len(), 2);

    let aliases = data
        .data_service
        .migrate_legacy_identities()
        .await
        .expect("migrate identities");
    assert!(aliases >= 1);
    let expected_subject = format!("https://metamind.dev/data/user/{}", old.user_id);
    let expected_object = mm_data::rdf::legacy_user_iri(&old.user_id);
    let schema = data
        .engine
        .triples(mm_data::data::namespaces::SCHEMA_GRAPH)
        .await
        .expect("schema graph");
    assert!(schema.iter().any(|row| {
        row.get("s").and_then(mm_data::rdf::term_value) == Some(expected_subject.as_str())
            && row.get("p").and_then(mm_data::rdf::term_value)
                == Some("http://www.w3.org/2002/07/owl#sameAs")
            && row.get("o").and_then(mm_data::rdf::term_value) == Some(expected_object.as_str())
    }));
}

#[tokio::test]
async fn full_init_sequence_and_memory_pipeline() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(init_config(&temp))
        .await
        .expect("initialize");

    assert!(data.health.sqlite_schema == "1");
    assert!(data.health.quads > 0, "ontology should have loaded");
    assert!(data.base_dir.join("config.toml").exists());
    assert!(data.base_dir.join("agentstream/events").exists());

    // Registries.
    let user = data
        .users
        .create_user(UserSpec {
            name: "Ada".to_owned(),
            email: Some("ada@example.com".to_owned()),
            role: Some("engineer".to_owned()),
            ..UserSpec::default()
        })
        .await
        .expect("create user");
    assert_eq!(data.users.list_users().await.unwrap().len(), 1);
    assert!(data.users.user_home(&user.user_id).exists());

    let project = data
        .projects
        .create_project(ProjectSpec {
            name: "auth-service".to_owned(),
            description: Some("Migrate auth".to_owned()),
            owner: Some(user.user_id.clone()),
        })
        .await
        .expect("create project");
    data.projects
        .add_member(&project.project_id, &user.user_id)
        .await
        .expect("add member");
    assert_eq!(
        data.projects
            .require_project(&project.project_id)
            .await
            .unwrap()
            .members,
        vec![user.user_id.clone()]
    );

    let app = data
        .apps
        .create_app(AppSpec {
            name: "console".to_owned(),
            version: Some("1.0.0".to_owned()),
            owner: Some(user.user_id.clone()),
        })
        .await
        .expect("create app");
    assert_eq!(data.apps.list_apps().await.unwrap().len(), 1);
    assert_eq!(
        data.apps.require_app(&app.app_id).await.unwrap().name,
        "console"
    );

    // A real SPARQL query over the user index.
    let rows = data
        .engine
        .sparql(
            mm_data::rdf::USER_INDEX_GRAPH,
            &format!(
                "SELECT ?user WHERE {{ GRAPH <{}> {{ ?user a <{}> }} }}",
                mm_data::rdf::USER_INDEX_GRAPH,
                mm_data::users::USER_CLASS
            ),
        )
        .await
        .expect("sparql");
    assert_eq!(rows.len(), 1);

    // Start a memory session.
    let user_id = UserId::new(user.user_id.clone());
    let project_id = ProjectId::new(project.project_id.clone());
    let context = data
        .memory_system
        .start_session(&user_id, Some(&project_id), None)
        .await
        .expect("session");
    let session_id = context.session_id.clone().expect("session id");

    let mut turn = EpisodicTurn::new(
        session_id.clone(),
        user_id.clone(),
        1,
        "We decided to use Redis for the cache. We will add invalidation tests.",
        "Implemented Redis in src/cache/redis.rs. Next step: add tests.",
    );
    turn.project_id = Some(project_id.clone());
    data.memory_system.log_turn(turn).await.expect("log turn");

    let events = data
        .agentstream
        .events(session_id.as_str())
        .await
        .expect("events");
    assert!(events.is_empty(), "turns are not events until logged");

    data.memory_system
        .log_agentstream_event(AgentstreamEvent::new(
            session_id.as_str(),
            EventType::UserMessage,
            json!({ "text": "We decided to use Redis." }),
            EventProvenance {
                source: "user".to_owned(),
                model: None,
            },
        ))
        .await
        .expect("event");
    let events = data
        .agentstream
        .events(session_id.as_str())
        .await
        .expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type.as_str(), "user_message");

    // Compress, then search.
    let segments = data
        .memory_system
        .compress_session(&session_id)
        .await
        .expect("compress");
    assert_eq!(segments.len(), 1);
    let segment = &segments[0];
    assert!(!segment.narrative.is_empty());
    assert!(
        !segment.decisions.is_empty(),
        "decision should be extracted"
    );
    assert!(
        !segment.artifacts.is_empty(),
        "file path should be extracted"
    );
    assert_eq!(segment.embedding.len(), 384);
    assert!(!segment.key_topics.is_empty());

    let hits = data
        .memory_system
        .semantic_search(
            "redis caching",
            SearchFilters {
                user_id: Some(user_id.clone()),
                project_id: Some(project_id.clone()),
                ..SearchFilters::default()
            },
            5,
        )
        .await
        .expect("search");
    assert_eq!(hits.len(), 1);

    let decisions = data
        .memory_system
        .find_decisions("redis", &user_id, Some(&project_id))
        .await
        .expect("decisions");
    assert!(!decisions.is_empty());

    // A second compression has nothing left to consume.
    let again = data
        .memory_system
        .compress_session(&session_id)
        .await
        .expect("compress again");
    assert!(again.is_empty());

    // Session snapshot exists on disk.
    assert!(data
        .base_dir
        .join("agentstream/sessions")
        .join(format!("{session_id}.ttl"))
        .exists());
}

#[tokio::test]
async fn aggregation_transfer_and_continuity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(init_config(&temp))
        .await
        .expect("initialize");
    let user = data
        .users
        .create_user(UserSpec {
            name: "Grace".to_owned(),
            ..UserSpec::default()
        })
        .await
        .unwrap();
    let project = data
        .projects
        .create_project(ProjectSpec {
            name: "compiler".to_owned(),
            owner: Some(user.user_id.clone()),
            ..ProjectSpec::default()
        })
        .await
        .unwrap();
    let user_id = UserId::new(user.user_id.clone());
    let project_id = ProjectId::new(project.project_id.clone());

    let context = data
        .memory_system
        .start_session(&user_id, Some(&project_id), None)
        .await
        .unwrap();
    let session_id = context.session_id.clone().unwrap();
    let mut turn = EpisodicTurn::new(
        session_id.clone(),
        user_id.clone(),
        1,
        "We decided to use a rewrite pass. This will fix the regressions.",
        "Goal: finish the rewrite. Blocked: no parser tests yet.",
    );
    turn.project_id = Some(project_id.clone());
    data.memory_system.log_turn(turn).await.unwrap();
    let segments = data
        .memory_system
        .compress_session(&session_id)
        .await
        .unwrap();
    assert_eq!(segments.len(), 1);

    data.memory_system
        .aggregate_project(&project_id)
        .await
        .expect("aggregate");
    let mut project_memory = data
        .memory_system
        .project_context(&project_id)
        .await
        .unwrap()
        .expect("project memory");
    assert_eq!(project_memory.segment_ids.len(), 1);
    assert!(!project_memory.aggregated_decisions.is_empty());
    assert!(project_memory.suggested_continuation.is_some());

    // Teach the project a stack, then transfer it to the user.
    project_memory.tech_stack.languages = vec!["Rust".to_owned()];
    project_memory
        .learned_conventions
        .push(mm_data::memory::project::ConventionRule {
            rule_id: mm_data::RuleId::new("rule-1"),
            context: "git commits".to_owned(),
            rule: "conventional commits".to_owned(),
            tool: Some("git".to_owned()),
            confidence: 0.95,
            learned_from_turns: Vec::new(),
            project_id: project_id.clone(),
        });
    data.memory_system
        .projects
        .save(&project_memory)
        .await
        .unwrap();
    // The user registry knows about the project only after resuming with it;
    // sync_user walks the user's project registry.
    let mut user_memory = data
        .memory_system
        .users
        .load_or_create(&user_id, "Grace")
        .await
        .unwrap();
    user_memory.projects.insert(
        project_id.to_string(),
        mm_data::memory::user::ProjectSummary {
            name: "compiler".to_owned(),
            description: None,
            status: mm_data::memory::ProjectStatus::Active,
            last_activity: mm_data::Timestamp::now(),
            key_topics: vec!["rewrite".to_owned()],
        },
    );
    data.memory_system.users.save(&user_memory).await.unwrap();
    data.memory_system.sync_user(&user_id).await.expect("sync");
    let user_memory = data
        .memory_system
        .user_context(&user_id)
        .await
        .unwrap()
        .expect("user memory");
    assert!(user_memory.expertise_domains.contains_key("Rust"));
    assert!(user_memory
        .preferred_tools
        .iter()
        .any(|tool| tool.tool == "git"));
    assert!(!user_memory.global_narrative.is_empty());

    // Resume the session: the continuation is concrete.
    let resumed = data
        .memory_system
        .start_session(&user_id, Some(&project_id), Some(&session_id))
        .await
        .expect("resume");
    assert!(resumed.project_context.is_some());
    assert!(resumed.user_context.is_some());
    assert!(resumed.previous_session.is_some());
    assert!(resumed.greeting.starts_with("Back at it."));

    // Ending a session writes projections.
    data.memory_system.end_session(&session_id).await.unwrap();
    assert!(data
        .base_dir
        .join("agentstream/projections/conversations.ttl")
        .exists());
}

#[tokio::test]
async fn dataset_versions_and_checksums() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(init_config(&temp))
        .await
        .expect("initialize");
    let dataset = data
        .datasets
        .create_dataset(DatasetSpec {
            title: "fixtures".to_owned(),
            description: Some("test fixtures".to_owned()),
            format: Some("text/turtle".to_owned()),
        })
        .await
        .unwrap();
    let source = temp.path().join("input.ttl");
    std::fs::write(&source, "<urn:a> <urn:b> <urn:c> .\n").unwrap();
    let updated = data
        .datasets
        .add_version(&dataset.dataset_id, "1.0.0", &[source])
        .await
        .expect("add version");
    assert_eq!(updated.versions.len(), 1);
    assert!(updated.versions[0].files[0].checksum.starts_with("sha256:"));
    assert!(data
        .datasets
        .version_dir(&dataset.dataset_id, "1.0.0")
        .join("input.ttl")
        .exists());
    assert_eq!(data.datasets.list_datasets().await.unwrap().len(), 1);
}

#[tokio::test]
async fn multi_device_presence_broadcast_and_handoff() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(init_config(&temp))
        .await
        .expect("initialize");
    let user = data
        .users
        .create_user(UserSpec {
            name: "Lin".to_owned(),
            ..UserSpec::default()
        })
        .await
        .unwrap();
    let context = data
        .memory_system
        .start_session(&UserId::new(user.user_id), None, None)
        .await
        .unwrap();
    let session_id = context.session_id.unwrap();

    let mut text = data
        .memory_system
        .register_channel(&session_id, mm_data::UiChannel::Text)
        .await
        .unwrap();
    let mut voice = data
        .memory_system
        .register_channel(&session_id, mm_data::UiChannel::Voice)
        .await
        .unwrap();
    let presence = data.memory_system.sync.presence(&session_id).await;
    assert_eq!(presence.len(), 2);

    let turn = mm_data::Turn {
        turn_id: "t1".to_owned(),
        user: Some("hello".to_owned()),
        agent: "hi".to_owned(),
        timestamp: 1,
    };
    let sent = data
        .memory_system
        .broadcast_turn(&session_id, turn.clone(), mm_data::UiChannel::Text)
        .await
        .unwrap();
    assert_eq!(sent, 1);
    assert_eq!(voice.recv().await, Some(mm_data::UiMessage::Turn(turn)));
    assert!(text.try_recv().is_err(), "origin is skipped");

    data.memory_system
        .handoff_modality(
            &session_id,
            mm_data::UiChannel::Voice,
            mm_data::UiChannel::Text,
        )
        .await
        .unwrap();
    match text.recv().await {
        Some(mm_data::UiMessage::ModalityChanged { from, to, .. }) => {
            assert_eq!(from, mm_data::UiChannel::Voice);
            assert_eq!(to, mm_data::UiChannel::Text);
        }
        other => panic!("unexpected message: {other:?}"),
    }
}

#[tokio::test]
async fn persistent_root_reopens_with_its_own_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        ..InitConfig::default()
    })
    .await
    .expect("first init");
    let user = data
        .users
        .create_user(UserSpec {
            name: "Persistent".to_owned(),
            ..UserSpec::default()
        })
        .await
        .unwrap();
    assert!(data.config.engine.persistent);
    drop(data);

    let reopened = initialize_data_layer(InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        ..InitConfig::default()
    })
    .await
    .expect("reopen");
    assert_eq!(reopened.users.list_users().await.unwrap().len(), 1);
    assert_eq!(
        reopened
            .users
            .require_user(&user.user_id)
            .await
            .unwrap()
            .name,
        "Persistent"
    );
}

#[tokio::test]
async fn entity_helper_keeps_highest_revision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(init_config(&temp))
        .await
        .expect("initialize");
    let graph = "https://metamind.dev/test/graph";
    let subject = "urn:metamind:test:1";
    mm_data::rdf::upsert(
        &data.engine,
        graph,
        subject,
        "https://metamind.dev/test#Thing",
        Some("first"),
        &json!({ "value": 1 }),
    )
    .await
    .unwrap();
    mm_data::rdf::upsert(
        &data.engine,
        graph,
        subject,
        "https://metamind.dev/test#Thing",
        Some("second"),
        &json!({ "value": 2 }),
    )
    .await
    .unwrap();
    let loaded: Option<serde_json::Value> = mm_data::rdf::load_as(&data.engine, graph, subject)
        .await
        .unwrap();
    assert_eq!(loaded.unwrap()["value"], 2);
    assert_eq!(EntityId::agent().to_string(), "agent");
}
