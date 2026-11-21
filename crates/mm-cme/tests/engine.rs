//! Cross-machine tests for the Phase 3 conversation engine.

use std::time::Duration;

use mm_cme::{
    DriverMessage, ImprovementEvent, ImprovementRegion, MemoryEvent, MemoryRegion, PluginEvent,
    PluginRegion, PreferenceEvent, PreferenceRegion, ProactiveEvent, ProactiveRegion, Session,
    SessionDriver, SessionEvent, Turn, TurnEvent, TurnPlan, VoiceEvent, VoiceRegion,
};
use mm_data::{SessionId, TurnId, UserId};

#[test]
fn proactive_region_cycle_and_suppression() {
    let mut region = ProactiveRegion::new().unwrap();
    assert_eq!(region.state(), "idle");
    region.dispatch(ProactiveEvent::Tick).unwrap();
    assert_eq!(region.state(), "evaluating");
    region
        .dispatch(ProactiveEvent::CandidateScored(vec!["a".into()]))
        .unwrap();
    assert_eq!(region.state(), "selecting");
    region
        .dispatch(ProactiveEvent::SelectionMade(vec!["a".into()]))
        .unwrap();
    region
        .dispatch(ProactiveEvent::TurnSpawned("t1".into()))
        .unwrap();
    assert_eq!(region.state(), "executing");
    region.dispatch(ProactiveEvent::TurnCompleted).unwrap();
    assert_eq!(region.state(), "injecting");
    region.dispatch(ProactiveEvent::FramesInjected).unwrap();
    assert_eq!(region.state(), "completed");
    region.dispatch(ProactiveEvent::Tick).unwrap();
    assert_eq!(region.state(), "idle");

    // User activity suppresses the cycle.
    region.dispatch(ProactiveEvent::Tick).unwrap();
    region
        .dispatch(ProactiveEvent::UserActivityDetected)
        .unwrap();
    assert_eq!(region.state(), "suppressed");
    region.dispatch(ProactiveEvent::BudgetAvailable).unwrap();
    assert_eq!(region.state(), "idle");
}

#[test]
fn voice_region_tiers_barge_in_and_failover() {
    let mut region = VoiceRegion::new().unwrap();
    region.dispatch(VoiceEvent::StartVoice).unwrap();
    assert_eq!(region.state(), "negotiating");

    region
        .dispatch(VoiceEvent::TierNegotiated("tier1".into()))
        .unwrap();
    assert_eq!(region.state(), "tier1_request_response");
    region
        .dispatch(VoiceEvent::TierFailover("tier1".into()))
        .unwrap();
    assert_eq!(region.state(), "degraded");
    region.dispatch(VoiceEvent::Renegotiate).unwrap();
    region
        .dispatch(VoiceEvent::TierNegotiated("tier2".into()))
        .unwrap();
    assert_eq!(region.state(), "tier2_streaming");

    region.dispatch(VoiceEvent::BargeInDetected).unwrap();
    assert_eq!(region.state(), "barged_in");
    region.dispatch(VoiceEvent::Recovering).unwrap();
    assert_eq!(region.state(), "recovering");
    region.dispatch(VoiceEvent::RecoveryComplete).unwrap();
    assert_eq!(region.state(), "tier2_streaming");

    region
        .dispatch(VoiceEvent::TierNegotiated("tier3".into()))
        .unwrap();
    assert_eq!(region.state(), "tier2_streaming", "no direct tier switch");
    region.dispatch(VoiceEvent::SessionPause).unwrap();
    assert_eq!(region.state(), "idle");
}

#[test]
fn memory_region_follows_the_four_tier_pipeline() {
    let mut region = MemoryRegion::new().unwrap();
    assert_eq!(region.state(), "episodic");
    region.dispatch(MemoryEvent::TurnCompleted).unwrap();
    assert_eq!(region.state(), "episodic");
    region.dispatch(MemoryEvent::CompressionTriggered).unwrap();
    assert_eq!(region.state(), "compressing");
    region
        .dispatch(MemoryEvent::CompressionDone(vec!["seg-1".into()]))
        .unwrap();
    assert_eq!(region.state(), "segmented");
    region
        .dispatch(MemoryEvent::ProjectAggregationTriggered)
        .unwrap();
    region
        .dispatch(MemoryEvent::ProjectAggregationDone)
        .unwrap();
    assert_eq!(region.state(), "project_synced");
    region.dispatch(MemoryEvent::UserSyncTriggered).unwrap();
    region.dispatch(MemoryEvent::UserSyncDone).unwrap();
    assert_eq!(region.state(), "user_synced");
    region.dispatch(MemoryEvent::RetentionTick).unwrap();
    assert_eq!(region.state(), "purging");
    region.dispatch(MemoryEvent::PurgeCompleted).unwrap();
    assert_eq!(region.state(), "episodic");
}

#[test]
fn preference_region_converges_and_relearns_on_drift() {
    let mut region = PreferenceRegion::new().unwrap();
    assert_eq!(region.state(), "cold_start");
    region
        .dispatch(PreferenceEvent::SignalObserved {
            dimension: "verbosity".into(),
            value: -0.4,
            weight: 1.0,
        })
        .unwrap();
    assert_eq!(region.state(), "observing");
    region
        .dispatch(PreferenceEvent::ConvergenceReached)
        .unwrap();
    assert_eq!(region.state(), "converging");
    region
        .dispatch(PreferenceEvent::ConvergenceReached)
        .unwrap();
    assert_eq!(region.state(), "stable");
    region
        .dispatch(PreferenceEvent::DriftDetected("verbosity".into()))
        .unwrap();
    assert_eq!(region.state(), "drift_detected");
    region
        .dispatch(PreferenceEvent::SignalObserved {
            dimension: "verbosity".into(),
            value: 0.5,
            weight: 1.0,
        })
        .unwrap();
    assert_eq!(region.state(), "relearning");
    region
        .dispatch(PreferenceEvent::ConvergenceReached)
        .unwrap();
    assert_eq!(region.state(), "converging");
    region.dispatch(PreferenceEvent::ResetRequested).unwrap();
    assert_eq!(region.state(), "cold_start");
}

#[test]
fn plugin_region_loads_runs_suspends_and_unloads() {
    let mut region = PluginRegion::new().unwrap();
    assert_eq!(region.state(), "loading");
    region
        .dispatch(PluginEvent::LoadRequest("p1".into()))
        .unwrap();
    assert_eq!(region.state(), "validating");
    region
        .dispatch(PluginEvent::ValidationOk("manifest".into()))
        .unwrap();
    assert_eq!(region.state(), "registered");
    region
        .dispatch(PluginEvent::TriggerFired("tick".into()))
        .unwrap();
    assert_eq!(region.state(), "running");
    region.dispatch(PluginEvent::ExecutionComplete).unwrap();
    region.dispatch(PluginEvent::BudgetPressure).unwrap();
    assert_eq!(region.state(), "suspended");
    region.dispatch(PluginEvent::BudgetAvailable).unwrap();
    assert_eq!(region.state(), "registered");
    region
        .dispatch(PluginEvent::ReloadRequest("p1".into()))
        .unwrap();
    assert_eq!(region.state(), "updating");
    region.dispatch(PluginEvent::UpdateComplete).unwrap();
    region
        .dispatch(PluginEvent::UnloadRequest("p1".into()))
        .unwrap();
    assert_eq!(region.state(), "unloading");
    region
        .dispatch(PluginEvent::LoadRequest("p1".into()))
        .unwrap();
    assert_eq!(region.state(), "validating");
}

#[test]
fn improvement_region_completes_a_cycle() {
    let mut region = ImprovementRegion::new().unwrap();
    region.dispatch(ImprovementEvent::Start).unwrap();
    assert_eq!(region.state(), "reflecting");
    region
        .dispatch(ImprovementEvent::ReflectionDone(vec!["slow".into()]))
        .unwrap();
    assert_eq!(region.state(), "planning");
    region
        .dispatch(ImprovementEvent::PlanReady("cache".into()))
        .unwrap();
    assert_eq!(region.state(), "acting");
    region.dispatch(ImprovementEvent::ActionDone).unwrap();
    assert_eq!(region.state(), "verifying");
    region.dispatch(ImprovementEvent::VerificationDone).unwrap();
    assert_eq!(region.state(), "learning");
    region.dispatch(ImprovementEvent::Learned).unwrap();
    assert_eq!(region.state(), "completed");
    region.dispatch(ImprovementEvent::CycleComplete).unwrap();
    assert_eq!(region.state(), "idle");
}

#[test]
fn session_forwards_events_to_regions() {
    let mut session = Session::new(UserId::new("u1"), None).unwrap();
    session.dispatch(SessionEvent::InitComplete).unwrap();
    session.dispatch(SessionEvent::Tick).unwrap();
    assert_eq!(session.region_state("proactive"), Some("evaluating"));
    session.dispatch_regions("start_voice", None).unwrap();
    assert_eq!(session.region_state("voice"), Some("negotiating"));
}

#[test]
fn turn_and_chunk_complete_with_text() {
    let plan = TurnPlan::simple(SessionId::new("s"), TurnId::new("t"), "hello");
    let mut turn = Turn::new(plan).unwrap();
    turn.plan_ready().unwrap();
    turn.route_selected(false, "stub").unwrap();
    turn.dispatch(TurnEvent::LlmStreamStart).unwrap();
    turn.spawn_chunks().unwrap();
    turn.token("Hi").unwrap();
    turn.token("!").unwrap();
    let outcome = turn.finalize().unwrap();
    assert!(outcome.is_success());
    assert_eq!(outcome.text, "Hi!");
    assert_eq!(outcome.chunks_completed, 1);
}

#[tokio::test]
async fn driver_runs_a_full_session_turn_and_shutdown() {
    let session = Session::new(UserId::new("user-1"), None).unwrap();
    let (driver, messages, mut outcomes) =
        SessionDriver::spawn(session, Duration::from_secs(3_600));
    let handle = tokio::spawn(driver.run());

    messages
        .send(DriverMessage::StartTurn {
            query: "hello".to_owned(),
        })
        .unwrap();
    let outcome = outcomes.recv().await.expect("outcome");
    assert!(outcome.is_success());
    assert_eq!(outcome.state, "completed");
    assert_eq!(outcome.chunks_completed, 2);
    assert_eq!(outcome.text, "Echo: hello");
    assert_eq!(outcome.model.as_deref(), Some("stub-model"));

    messages.send(DriverMessage::Shutdown).unwrap();
    let session = handle.await.expect("driver task").expect("driver");
    assert_eq!(session.state(), "terminated");
    assert_eq!(session.turns_completed, 1);
    assert_eq!(session.region_state("voice"), Some("idle"));
}
