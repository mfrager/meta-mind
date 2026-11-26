//! Phase 1 exit-criteria coverage for the state service and stores.

use std::sync::Arc;

use mm_cme::FrameError;
use mm_data::{
    ConversationError, ConversationSessionFilter, DeleteMode, SessionError, SessionLifecycle,
    StateOrigin, StatusValue, TraceKind, WorkStatus,
};
use mm_prompt::PromptStore;
use mm_serve::session::{DEFAULT_IDLE_EXPIRY_MS, DEFAULT_WORK_STALL_TTL_MS};
use mm_serve::state_service::{StateService, StateServiceError};

fn prompts() -> Arc<PromptStore> {
    Arc::new(
        PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn fixture() -> (StateService, mm_data::InitResult, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let service = StateService::new(
        Arc::clone(&data.engine),
        Arc::clone(&data.agentstream),
        prompts(),
        data.base_dir.clone(),
    )
    .unwrap();
    (service, data, temp)
}

#[tokio::test]
async fn session_state_round_trips_and_replays() {
    let (service, data, _temp) = fixture().await;
    let user = "user-round-trip";
    let bundle = service.open_session(user, None, 1_000).await.unwrap();
    let id = bundle.session.session_id.clone();

    assert_eq!(bundle.conversation.revision, 1);
    assert!(bundle.conversation.topic.is_none());
    assert!(bundle.conversation.status.is_empty());
    assert_eq!(bundle.frames.frames.len(), 1);
    assert_eq!(bundle.frames.frames[0].template_id, "frame.intro");
    assert_eq!(bundle.session.main_trace.as_deref(), Some("main"));
    assert_eq!(bundle.session.traces.len(), 1);
    assert_eq!(bundle.session.traces[0].kind, TraceKind::Main);
    assert!(bundle.session.session_dir.is_none());
    assert!(!data.base_dir.join("sessions").join(&id).exists());

    let topic = service
        .set_topic(
            &id,
            Some("refresh token rollout".into()),
            bundle.conversation.revision,
            StateOrigin::Cm,
            1_001,
        )
        .await
        .unwrap();
    assert_eq!(topic.revision, 2);

    let depth = service
        .set_depth(&id, 2, topic.revision, StateOrigin::User, 1_002)
        .await
        .unwrap();
    assert_eq!(depth.revision, 3);

    let status = service
        .set_status(
            &id,
            "phase",
            StatusValue::Text("working".into()),
            depth.revision,
            StateOrigin::Cm,
            1_003,
        )
        .await
        .unwrap();
    assert_eq!(status.revision, 4);

    let reloaded = service.load(&id).await.unwrap();
    assert_eq!(reloaded.conversation, status);
    assert_eq!(
        reloaded.conversation.topic.as_deref(),
        Some("refresh token rollout")
    );
    assert_eq!(reloaded.conversation.depth, 2);
    assert_eq!(
        reloaded.conversation.status.get("phase"),
        Some(&StatusValue::Text("working".into()))
    );

    let replayed = service.replay(&id).await.unwrap();
    assert_eq!(replayed, status);

    let view = service.status(&id, 1_004).await.unwrap();
    assert_eq!(view.state.topic.as_deref(), Some("refresh token rollout"));
    assert_eq!(view.state.depth, 2);
    assert_eq!(view.frames.frames.len(), 1);
    assert_eq!(view.session.traces, vec!["main".to_string()]);

    let stale = service
        .set_topic(
            &id,
            Some("stale".into()),
            bundle.conversation.revision,
            StateOrigin::Cm,
            1_004,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        stale,
        StateServiceError::Conversation(ConversationError::Conflict { .. })
    ));
}

#[tokio::test]
async fn a_new_session_starts_intro_without_copying_previous_state() {
    let (service, _data, _temp) = fixture().await;
    let previous = service
        .open_session("user-continuity", None, 1_500)
        .await
        .unwrap();
    let previous_id = previous.session.session_id.clone();
    service
        .set_topic(
            &previous_id,
            Some("previous topic".into()),
            previous.conversation.revision,
            StateOrigin::Cm,
            1_501,
        )
        .await
        .unwrap();

    let current = service
        .open_session("user-continuity", Some(&previous_id), 1_502)
        .await
        .unwrap();
    assert_eq!(current.frames.frames[0].template_id, "frame.intro");
    assert!(current.conversation.topic.is_none());
    assert_eq!(current.conversation.revision, 1);
    assert_eq!(
        current.session.resume.resume_of.as_deref(),
        Some(previous_id.as_str())
    );
    assert_eq!(
        service
            .load(&previous_id)
            .await
            .unwrap()
            .session
            .superseded_by,
        Some(current.session.session_id)
    );
}

#[tokio::test]
async fn status_byte_cap_is_enforced() {
    let (service, _data, _temp) = fixture().await;
    let bundle = service
        .open_session("user-large", None, 2_000)
        .await
        .unwrap();
    let id = bundle.session.session_id.clone();
    let mut revision = bundle.conversation.revision;
    let mut error = None;
    for index in 0..10 {
        let key = format!("note_{index}");
        match service
            .set_status(
                &id,
                &key,
                StatusValue::Text("x".repeat(500)),
                revision,
                StateOrigin::Cm,
                2_001 + i64::from(index),
            )
            .await
        {
            Ok(state) => revision = state.revision,
            Err(found) => {
                error = Some(found);
                break;
            }
        }
    }
    assert!(matches!(
        error,
        Some(StateServiceError::Conversation(
            ConversationError::StateTooLarge { .. }
        ))
    ));
}

#[tokio::test]
async fn resume_restores_frames_and_work_and_adds_resume_frame() {
    let (service, _data, _temp) = fixture().await;
    let bundle = service
        .open_session("user-resume", None, 3_000)
        .await
        .unwrap();
    let id = bundle.session.session_id.clone();

    let pushed = service
        .push_frame(&id, "frame.work", bundle.frames.revision, 3_001)
        .await
        .unwrap();
    assert_eq!(pushed.frames.len(), 2);
    assert_eq!(pushed.frames[1].template_id, "frame.work");

    let working = service
        .set_work_status(
            &id,
            "main",
            WorkStatus::Working,
            bundle.session.revision,
            3_002,
        )
        .await
        .unwrap();
    assert_eq!(working.work.main.status, WorkStatus::Working);

    let ended = service.end_session(&id, "user left", 3_003).await.unwrap();
    assert_eq!(ended.lifecycle, SessionLifecycle::Ended);

    let resumed = service.resume_session(&id, 3_004).await.unwrap();
    assert_eq!(resumed.session.lifecycle, SessionLifecycle::Active);
    assert_eq!(resumed.frames.frames.len(), 3);
    assert_eq!(resumed.frames.frames[2].template_id, "frame.resume");
    assert_eq!(resumed.session.work.main.status, WorkStatus::Working);
    assert_eq!(resumed.session.resume.epoch, 1);
    assert_eq!(resumed.session.traces.len(), 1);
    assert_eq!(resumed.session.traces[0].kind, TraceKind::Main);
}

#[tokio::test]
async fn root_frame_cannot_be_popped() {
    let (service, _data, _temp) = fixture().await;
    let bundle = service
        .open_session("user-root", None, 4_000)
        .await
        .unwrap();
    let error = service
        .pop_frame(&bundle.session.session_id, bundle.frames.revision, 4_001)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateServiceError::Frame(FrameError::Session(SessionError::FrameLimit))
    ));
}

#[tokio::test]
async fn listing_is_cursor_paginated() {
    let (service, _data, _temp) = fixture().await;
    let user = "user-list";
    let mut ids = Vec::new();
    for index in 0..3 {
        let bundle = service
            .open_session(user, None, 5_000 + index)
            .await
            .unwrap();
        ids.push(bundle.session.session_id);
    }

    let first = service
        .list_sessions(user, None, 2, ConversationSessionFilter::default())
        .await
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert!(first.has_more);
    assert!(first.next_cursor.is_some());
    assert!(first.items.iter().all(|row| row.trace_count == 1));
    assert!(first
        .items
        .iter()
        .all(|row| row.trace_kinds == vec![TraceKind::Main]));

    let second = service
        .list_sessions(
            user,
            first.next_cursor.as_deref(),
            2,
            ConversationSessionFilter::default(),
        )
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(!second.has_more);
    assert_ne!(first.items[0].session_id, second.items[0].session_id);
    assert_eq!(
        first.items.len() + second.items.len(),
        ids.len(),
        "the full set is never returned in one page"
    );
}

#[tokio::test]
async fn directory_is_lazy_and_delete_preserves_then_purges_traces() {
    let (service, data, _temp) = fixture().await;
    let user = "user-delete";
    let bundle = service.open_session(user, None, 6_000).await.unwrap();
    let id = bundle.session.session_id.clone();
    let other = service
        .open_session("user-delete-other", None, 6_000)
        .await
        .unwrap()
        .session
        .session_id;
    let directory = data.base_dir.join("sessions").join(&id);
    assert!(!directory.exists());

    let created = service.ensure_session_directory(&id, 6_001).await.unwrap();
    assert!(created.exists());
    assert!(!created.join("files").exists());
    let mirrored = service.load(&id).await.unwrap();
    assert_eq!(
        mirrored.session.session_dir.as_deref(),
        Some(format!("data/sessions/{id}").as_str())
    );

    let file = service
        .write_session_file(&id, "notes/first.txt", b"hello", 6_002)
        .await
        .unwrap();
    assert!(file.is_file());
    assert!(created.join("files").is_dir());

    service.delete(&id, DeleteMode::Soft, user).await.unwrap();
    assert!(service
        .load(&id)
        .await
        .unwrap()
        .session
        .deleted_at
        .is_some());
    let visible = service
        .list_sessions(user, None, 10, ConversationSessionFilter::default())
        .await
        .unwrap();
    assert!(visible.items.is_empty());
    let traces = service.list_traces(&id, None, 10).await.unwrap();
    assert_eq!(traces.items.len(), 1);

    service.delete(&id, DeleteMode::Purge, user).await.unwrap();
    assert!(matches!(
        service.load(&id).await.unwrap_err(),
        StateServiceError::Missing(_)
    ));
    assert!(service
        .list_traces(&id, None, 10)
        .await
        .unwrap()
        .items
        .is_empty());
    assert!(data.agentstream.events(&id).await.unwrap().is_empty());
    let purged_filter = ConversationSessionFilter {
        include_deleted: true,
        ..ConversationSessionFilter::default()
    };
    assert!(service
        .list_sessions(user, None, 10, purged_filter)
        .await
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        service
            .list_sessions(
                "user-delete-other",
                None,
                10,
                ConversationSessionFilter::default()
            )
            .await
            .unwrap()
            .items[0]
            .session_id,
        other
    );
    assert!(!directory.exists());
}

#[tokio::test]
async fn idle_and_work_stall_expiry() {
    let (service, _data, _temp) = fixture().await;
    let idle_user = "user-idle";
    let idle = service.open_session(idle_user, None, 7_000).await.unwrap();
    let idle_id = idle.session.session_id.clone();
    assert_eq!(
        service.sweep(7_000 + DEFAULT_IDLE_EXPIRY_MS).await.unwrap(),
        1
    );
    assert_eq!(
        service.load(&idle_id).await.unwrap().session.lifecycle,
        SessionLifecycle::Ended
    );

    let work_user = "user-stall";
    let work = service.open_session(work_user, None, 8_000).await.unwrap();
    let work_id = work.session.session_id.clone();
    service
        .set_work_status(
            &work_id,
            "main",
            WorkStatus::Working,
            work.session.revision,
            8_001,
        )
        .await
        .unwrap();
    service.registry().touch_work(&work_id, 8_002).await;
    assert_eq!(
        service
            .sweep(8_002 + DEFAULT_WORK_STALL_TTL_MS + 1)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .load(&work_id)
            .await
            .unwrap()
            .session
            .work
            .main
            .status,
        WorkStatus::Waiting
    );
}
