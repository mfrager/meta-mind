//! Phase 6 integration: snapshots, transition streams, metrics, improvement,
//! reports and dashboards.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use mm_cme::{
    CriterionResult, Dashboard, DriverMessage, GovernanceConfig, ImprovementEngine,
    ImprovementLoop, ImprovementTrigger, Observability, Optimizable, Panel, PanelKind, Report,
    ReportFormat, ReportSection, Session, SessionDriver, SessionEvent, SessionSnapshot, Severity,
    TechnicalLevel, VerificationContext, VerificationKind, Verifier, ViewingContext,
};
use mm_data::UserId;

fn weak_report() -> Report {
    Report::new("r1", "Cache audit")
        .with_section(ReportSection::narrative("s1", "Findings", "x".repeat(700)))
        .with_section(ReportSection {
            id: "s2".into(),
            title: "Next".into(),
            modality: mm_cme::Modality::Narrative,
            detail_level: 5,
            content: String::new(),
            sources: Vec::new(),
            action_items: Vec::new(),
            importance: 5,
        })
}

fn weak_dashboard() -> Dashboard {
    Dashboard::new("d1", "Service health", ViewingContext::Mobile)
        .with_panel(Panel {
            id: "p1".into(),
            title: "Latency".into(),
            kind: PanelKind::Chart,
            cost_per_refresh: 8.0,
            critical: true,
            has_threshold: false,
            detail: 3,
        })
        .with_panel(Panel {
            id: "p2".into(),
            title: "Notes".into(),
            kind: PanelKind::List,
            cost_per_refresh: 6.0,
            critical: false,
            has_threshold: false,
            detail: 3,
        })
        .with_panel(Panel {
            id: "p3".into(),
            title: "Errors".into(),
            kind: PanelKind::Metric,
            cost_per_refresh: 4.0,
            critical: true,
            has_threshold: false,
            detail: 2,
        })
}

fn active_session() -> Session {
    let mut session = Session::new(UserId::new("u1"), None).expect("session");
    session.dispatch(SessionEvent::InitComplete).expect("init");
    session
        .context
        .preferences
        .values
        .insert("detail".to_owned(), 0.5);
    session
}

#[test]
fn snapshots_include_full_session_turn_and_region_state() {
    let mut session = active_session();
    let turn = session.begin_turn("explain the cache").expect("turn");
    turn.plan_ready().expect("plan");
    turn.route_selected(false, "stub").expect("route");
    turn.dispatch(mm_cme::TurnEvent::LlmStreamStart)
        .expect("start");
    turn.spawn_chunks().expect("chunks");
    turn.token("Cached.").expect("token");

    let snapshot = SessionSnapshot::capture(&session);
    assert_eq!(snapshot.session.machine, "session");
    assert_eq!(snapshot.session.regions.len(), 6);
    assert_eq!(snapshot.session.state.to_string(), "active");
    assert_eq!(snapshot.memory.state, "episodic");
    assert_eq!(snapshot.proactive.state, "idle");
    assert_eq!(snapshot.voice.barge_in_count, 0);
    assert_eq!(snapshot.preferences.values["detail"], 0.5);

    let turn = snapshot.turn.expect("turn snapshot");
    assert_eq!(turn.state, "streaming");
    assert!(turn.plan.is_some());
    assert!(!turn.chunks.is_empty());
    assert!(turn.chunks.iter().all(|chunk| chunk.state == "streaming"));
}

#[tokio::test]
async fn the_driver_streams_transitions_and_renders_prometheus() {
    let observability = Arc::new(Observability::new(128));
    let mut receiver = observability.subscribe();
    let session = Session::new(UserId::new("u-events"), None).expect("session");
    let (driver, messages, mut outcomes) = SessionDriver::spawn(session, Duration::from_secs(60));
    let handle = tokio::spawn(driver.with_observability(Arc::clone(&observability)).run());

    messages
        .send(DriverMessage::StartTurn {
            query: "hello".to_owned(),
        })
        .expect("turn");
    assert!(outcomes.recv().await.expect("outcome").is_success());
    messages.send(DriverMessage::Shutdown).expect("shutdown");
    handle.await.expect("join").expect("driver");

    let mut types = BTreeSet::new();
    while let Ok(event) = receiver.try_recv() {
        assert!(!event.from.is_empty());
        assert!(!event.to.is_empty());
        types.insert(event.sm_type);
    }
    assert!(types.contains("session"), "saw {types:?}");
    assert!(types.contains("turn"), "saw {types:?}");
    assert!(types.contains("chunk"), "saw {types:?}");

    let metrics = observability.prometheus();
    assert!(metrics.contains("cme_sm_transitions_total"));
    assert!(metrics.contains("cme_sm_state_duration_ms"));
    assert!(metrics.contains("cme_sm_active_count"));
}

#[test]
fn improvement_raises_report_and_dashboard_scores() {
    let mut session = active_session();
    let mut improvement = ImprovementLoop::new();
    let mut report = weak_report();
    let mut dashboard = weak_dashboard();
    let report_before = report.score();
    let dashboard_before = dashboard.score();

    let report_outcome = improvement
        .run(&mut session, &mut report, ImprovementTrigger::Manual, true)
        .expect("report cycle");
    assert!(report_outcome.verification.passed);
    assert!(report_outcome.accepted);
    assert!(report.score() > report_before);
    assert_eq!(session.region_state("improvement"), Some("idle"));

    let dashboard_outcome = improvement
        .run(
            &mut session,
            &mut dashboard,
            ImprovementTrigger::Manual,
            true,
        )
        .expect("dashboard cycle");
    assert!(dashboard_outcome.verification.passed);
    assert!(dashboard.score() > dashboard_before);
    assert!(!improvement.patterns().is_empty());

    assert!(report
        .render(ReportFormat::Markdown)
        .expect("markdown")
        .contains("# Cache audit"));
    assert!(report
        .render(ReportFormat::Json)
        .expect("json")
        .contains("Findings"));
    assert!(dashboard.summary()["panels"].as_u64().unwrap() >= 2);
}

#[test]
fn improvement_can_run_on_a_schedule() {
    let mut session = active_session();
    let mut engine =
        ImprovementEngine::new(weak_report(), weak_dashboard(), 2, ImprovementLoop::new());
    assert!(engine.on_tick(&mut session).expect("tick 1").is_empty());
    let outcomes = engine.on_tick(&mut session).expect("tick 2");
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|outcome| outcome.skipped.is_none()));
    assert_eq!(engine.loop_().cycles(), 2);
}

#[tokio::test]
async fn the_driver_schedules_improvement_cycles() {
    let observability = Arc::new(Observability::new(256));
    let mut receiver = observability.subscribe();
    let session = Session::new(UserId::new("u-improve"), None).expect("session");
    let (driver, messages, _outcomes) = SessionDriver::spawn(session, Duration::from_millis(10));
    let engine = ImprovementEngine::new(weak_report(), weak_dashboard(), 1, ImprovementLoop::new());
    let handle = tokio::spawn(
        driver
            .with_observability(Arc::clone(&observability))
            .with_improvement(engine)
            .run(),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    messages.send(DriverMessage::Shutdown).expect("shutdown");
    handle.await.expect("join").expect("driver");

    let mut saw_improvement = false;
    while let Ok(event) = receiver.try_recv() {
        if event.sm_type == "improvement" {
            saw_improvement = true;
        }
    }
    assert!(saw_improvement, "the driver should have run a cycle");
}

#[test]
fn verification_failure_rolls_back_the_artifact() {
    #[derive(Debug)]
    struct AlwaysFails;

    impl Verifier for AlwaysFails {
        fn kind(&self) -> VerificationKind {
            VerificationKind::UnitTest
        }

        fn verify(&self, _ctx: &VerificationContext<'_>) -> CriterionResult {
            CriterionResult {
                kind: self.kind(),
                passed: false,
                baseline: 1.0,
                candidate: 0.0,
            }
        }
    }

    let mut session = active_session();
    let mut report = weak_report();
    let baseline = report.clone();
    let mut improvement = ImprovementLoop::new()
        .with_verifiers(vec![Box::new(AlwaysFails)])
        .with_governance(GovernanceConfig {
            auto_rollback: true,
            ..GovernanceConfig::default()
        });
    let outcome = improvement
        .run(
            &mut session,
            &mut report,
            ImprovementTrigger::OnRegression,
            true,
        )
        .expect("cycle");
    assert!(outcome.rolled_back);
    assert!(!outcome.verification.passed);
    assert_eq!(report, baseline);
    assert_eq!(session.region_state("improvement"), Some("idle"));
    assert!(!outcome.issues.is_empty());
}

#[test]
fn reports_carry_audience_and_source_metrics() {
    let report = Report::new("r", "Brief").with_section(ReportSection {
        id: "s".into(),
        title: "Summary".into(),
        modality: mm_cme::Modality::Summarize,
        detail_level: 2,
        content: "Shipped.".into(),
        sources: vec!["pr-1".into()],
        action_items: vec!["Watch the error rate".into()],
        importance: 5,
    });
    assert_eq!(report.audience, TechnicalLevel::Practitioner);
    let metrics = report.metrics();
    assert_eq!(metrics["source_coverage"], 1.0);
    assert_eq!(metrics["completeness"], 1.0);
    assert!(metrics.values().all(|value| (0.0..=1.0).contains(value)));
    assert!(Severity::High > Severity::Low);
}
