//! Phase 5 integration: memory via `mm-data`, preference hooks, plugins,
//! proactive injection, resume and multi-device sync.

use std::sync::Arc;
use std::time::Duration;

use mm_cme::{
    AffectState, DriverMessage, MemoryBridge, MemoryBridgeConfig, PluginContext, PluginHost,
    PluginSandbox, PreferenceDimension, PreferenceLearner, ProactiveConfig, ProactiveEngine,
    Session, SessionDriver, SessionEvent, TurnExecutor,
};
use mm_data::{
    initialize_data_layer, InitConfig, MemorySystem, ProjectId, SearchFilters, Turn as SyncTurn,
    UiChannel, UiMessage, UserId,
};

async fn memory_system() -> (tempfile::TempDir, Arc<MemorySystem>) {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = initialize_data_layer(InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..InitConfig::default()
    })
    .await
    .expect("initialize data layer");
    (temp, data.memory_system)
}

fn bridge(system: &Arc<MemorySystem>) -> MemoryBridge {
    MemoryBridge::new(
        Arc::clone(system),
        MemoryBridgeConfig {
            compression_trigger_turns: 1,
            auto_aggregate: true,
            auto_sync_user: false,
        },
    )
}

fn active_session(user: &str, project: Option<ProjectId>) -> Session {
    let mut session = Session::new(UserId::new(user), project).expect("session");
    session.dispatch(SessionEvent::InitComplete).expect("init");
    session
}

async fn run_turn(
    executor: &mut TurnExecutor,
    session: &mut Session,
    query: &str,
) -> (u8, mm_cme::TurnOutcome) {
    let context = session.context.clone();
    let turn = session.begin_turn(query).expect("begin turn");
    let detail = turn.plan.as_ref().map_or(3, |plan| plan.detail_level);
    executor
        .execute(turn, &context, None)
        .await
        .expect("execute");
    let outcome = session.finish_turn().expect("finish turn");
    (detail, outcome)
}

#[tokio::test]
async fn completed_turns_compress_into_searchable_segments() {
    let (_temp, system) = memory_system().await;
    let bridge = bridge(&system);
    let mut session = active_session("u1", None);
    let resume = bridge
        .start_session(&mut session, None)
        .await
        .expect("start");
    assert!(!resume.greeting.is_empty());

    let mut executor = TurnExecutor::deterministic();
    let query = "We decided to use Redis for the cache. Next step: add invalidation tests.";
    let (detail, outcome) = run_turn(&mut executor, &mut session, query).await;
    let cycle = bridge
        .after_turn(&mut session, query, detail, &outcome)
        .await
        .expect("memory cycle");

    assert_eq!(cycle.segments.len(), 1);
    assert!(!cycle.segments[0].narrative.is_empty());
    assert_eq!(session.region_state("memory"), Some("segmented"));

    // The segment is queryable through the vector index.
    let hits = bridge
        .search(
            "redis caching",
            SearchFilters {
                user_id: Some(UserId::new("u1")),
                ..SearchFilters::default()
            },
            5,
        )
        .await
        .expect("search");
    assert!(!hits.is_empty(), "semantic search should find the segment");

    // And through SPARQL over its tier-2 graph.
    let graph = mm_data::rdf::segment_graph(cycle.segments[0].segment_id.as_str());
    let query = format!("SELECT ?s WHERE {{ GRAPH <{graph}> {{ ?s ?p ?o }} }}");
    let rows = system
        .engine()
        .sparql(&graph, &query)
        .await
        .expect("sparql");
    assert!(!rows.is_empty(), "segment graph should contain triples");

    // A later turn can compress again from `segmented`.
    let query = "We decided to keep the cache warm. Next step: measure hit rate.";
    let (detail, outcome) = run_turn(&mut executor, &mut session, query).await;
    let cycle = bridge
        .after_turn(&mut session, query, detail, &outcome)
        .await
        .expect("second memory cycle");
    assert_eq!(cycle.segments.len(), 1);
    assert_eq!(session.region_state("memory"), Some("segmented"));
}

#[tokio::test]
async fn project_memory_and_resume_carry_context_forward() {
    let (_temp, system) = memory_system().await;
    let bridge = bridge(&system);
    let project = ProjectId::new("project-1");
    let mut session = active_session("u1", Some(project.clone()));
    bridge
        .start_session(&mut session, None)
        .await
        .expect("start");

    let mut executor = TurnExecutor::deterministic();
    let query = "We decided the cache uses Redis. The open question is invalidation.";
    let (detail, outcome) = run_turn(&mut executor, &mut session, query).await;
    let cycle = bridge
        .after_turn(&mut session, query, detail, &outcome)
        .await
        .expect("cycle");
    assert!(cycle.aggregated);
    assert_eq!(session.region_state("memory"), Some("project_synced"));
    assert!(system
        .project_context(&project)
        .await
        .expect("project")
        .is_some());

    let previous = session.id.clone();
    bridge.end_session(&session).await.expect("end");

    let mut next = active_session("u1", Some(project));
    let resume = bridge
        .start_session(&mut next, Some(&previous))
        .await
        .expect("resume");
    assert!(resume
        .project_summary
        .as_deref()
        .is_some_and(|summary| !summary.is_empty()));
    assert!(!resume.greeting.is_empty());
    assert!(!resume.previous_topics.is_empty());
    assert_eq!(
        next.context.memory.previous_session_id.as_ref(),
        Some(&previous)
    );
}

#[tokio::test]
async fn learned_preferences_reach_the_planner_after_convergence() {
    let (_temp, system) = memory_system().await;
    let user = UserId::new("u-pref");
    let mut learner = PreferenceLearner::new();
    for _ in 0..10 {
        learner.observe(PreferenceDimension::TechnicalDepth, 0.5, 1.0);
    }
    assert_eq!(
        learner
            .posterior(PreferenceDimension::TechnicalDepth)
            .expect("posterior")
            .observations,
        10
    );
    learner.persist(&system, &user).await.expect("persist");

    let bridge = bridge(&system);
    let mut session = active_session("u-pref", None);
    let resume = bridge
        .start_session(&mut session, None)
        .await
        .expect("resume");
    assert!(
        (resume.preferences["technical_depth"] - 0.5).abs() < 0.01,
        "resume carries the learned value"
    );
    let detail = session.context.preferences.value("detail");
    assert!(
        (detail - 0.5).abs() < 0.01,
        "the planner hook derives detail"
    );

    learner.apply_to_session(&mut session);
    let plan = mm_cme::TurnPlanner
        .build_plan(
            &mm_cme::TurnContext::user_input(
                mm_data::TurnId::new("t-pref"),
                session.id.clone(),
                "explain the cache",
            ),
            &session.context,
        )
        .expect("plan");
    assert_eq!(plan.detail_level, 4);
}

#[tokio::test]
async fn plugins_load_evaluate_and_execute_within_the_sandbox() {
    let mut host = PluginHost::with_builtins(PluginSandbox {
        max_proposals_per_cycle: 2,
        ..PluginSandbox::default()
    })
    .expect("host");
    assert_eq!(host.ids().len(), 6);
    let profile = mm_cme::PreferenceProfile::default();
    let hits = Vec::<String>::new();
    let ctx = PluginContext {
        query: "search the plan for the answer?",
        preferences: &profile,
        affect: AffectState::default(),
        memory_hits: &hits,
    };
    let proposals = host.evaluate(&ctx);
    assert_eq!(proposals.len(), 2, "the cycle cap is enforced");
    let result = host.execute(&proposals[0]).expect("execute");
    assert!(!result.detail.is_empty());
    assert!(host.execute(&proposals[1]).is_ok());
}

#[tokio::test]
async fn proactive_items_are_injected_into_the_session_stream() {
    let mut session = active_session("u-proactive", None);
    let mut engine = ProactiveEngine::with_builtins(ProactiveConfig::default()).expect("engine");
    let profile = session.context.preferences.clone();
    let hits = Vec::<String>::new();
    let ctx = PluginContext {
        query: "search for the plan?",
        preferences: &profile,
        affect: AffectState::default(),
        memory_hits: &hits,
    };
    let items = engine.run(&mut session, &ctx).expect("proactive cycle");
    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item.plugin == "search_lookup"));
    assert_eq!(session.region_state("proactive"), Some("completed"));
    assert_eq!(session.proactive_items.len(), items.len());
    assert_eq!(engine.cycles(), 1);
    assert_eq!(session.drain_proactive().len(), items.len());
    assert!(session.proactive_items.is_empty());

    // The repeat window suppresses the same titles on the next cycle.
    let again = engine.run(&mut session, &ctx).expect("second cycle");
    assert!(again.is_empty());
    assert_eq!(session.region_state("proactive"), Some("completed"));
}

#[tokio::test]
async fn the_driver_schedules_proactive_cycles() {
    let session = Session::new(UserId::new("u-tick"), None).expect("session");
    let (driver, messages, mut outcomes) = SessionDriver::spawn(session, Duration::from_millis(10));
    let engine = ProactiveEngine::with_builtins(ProactiveConfig::default()).expect("engine");
    let handle = tokio::spawn(driver.with_proactive(engine).run());

    messages
        .send(DriverMessage::StartTurn {
            query: "search for the plan?".to_owned(),
        })
        .expect("send turn");
    assert!(outcomes.recv().await.expect("outcome").is_success());
    tokio::time::sleep(Duration::from_millis(100)).await;
    messages.send(DriverMessage::Shutdown).expect("shutdown");

    let session = handle.await.expect("join").expect("driver");
    assert!(
        !session.proactive_items.is_empty(),
        "a proactive tick should have injected an item"
    );
}

#[tokio::test]
async fn presence_broadcast_and_handoff_reach_other_devices() {
    let (_temp, system) = memory_system().await;
    let bridge = bridge(&system);
    let mut session = active_session("u-sync", None);
    bridge
        .start_session(&mut session, None)
        .await
        .expect("start");

    let mut text = bridge
        .register_channel(&session.id, UiChannel::Text)
        .await
        .expect("text channel");
    let mut voice = bridge
        .register_channel(&session.id, UiChannel::Voice)
        .await
        .expect("voice channel");
    assert_eq!(
        bridge.presence(&session.id).await.expect("presence").len(),
        2
    );

    let turn = SyncTurn {
        turn_id: "turn-1".to_owned(),
        user: Some("hello".to_owned()),
        agent: "hi".to_owned(),
        timestamp: 1,
    };
    let sent = bridge
        .broadcast_turn(&session.id, turn.clone(), UiChannel::Text)
        .await
        .expect("broadcast");
    assert_eq!(sent, 1, "the origin channel does not receive its own turn");
    assert!(text.try_recv().is_err());
    match voice.recv().await.expect("voice turn") {
        UiMessage::Turn(received) => assert_eq!(received, turn),
        other => panic!("expected a turn, got {other:?}"),
    }

    let sent = bridge
        .handoff_modality(&session.id, UiChannel::Text, UiChannel::Voice)
        .await
        .expect("handoff");
    assert_eq!(sent, 1);
    match voice.recv().await.expect("handoff") {
        UiMessage::ModalityChanged { from, to, .. } => {
            assert_eq!(from, UiChannel::Text);
            assert_eq!(to, UiChannel::Voice);
        }
        other => panic!("expected a handoff, got {other:?}"),
    }
}

#[tokio::test]
async fn the_driver_logs_and_compresses_completed_turns() {
    let (_temp, system) = memory_system().await;
    let bridge = Arc::new(bridge(&system));
    let session = Session::new(UserId::new("u-driver"), None).expect("session");
    let (driver, messages, mut outcomes) = SessionDriver::spawn(session, Duration::from_secs(60));
    let handle = tokio::spawn(driver.with_memory(Arc::clone(&bridge)).run());

    messages
        .send(DriverMessage::StartTurn {
            query: "We decided to use Redis for the cache.".to_owned(),
        })
        .expect("send turn");
    let outcome = outcomes.recv().await.expect("outcome");
    assert!(outcome.is_success(), "{outcome:?}");
    messages.send(DriverMessage::Shutdown).expect("shutdown");

    let session = handle.await.expect("join").expect("driver");
    assert_eq!(session.state(), "terminated");

    let hits = bridge
        .search(
            "redis cache",
            SearchFilters {
                user_id: Some(UserId::new("u-driver")),
                ..SearchFilters::default()
            },
            5,
        )
        .await
        .expect("search");
    assert!(
        !hits.is_empty(),
        "the driver should have compressed the turn"
    );
}
