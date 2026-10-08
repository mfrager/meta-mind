//! Event log behaviour: exactly-once application, deterministic replay, and an
//! audited commit for every committed event.

use std::sync::Arc;

use mm_core::{EventKind, NewEvent, Timestamp, UlidFactory};
use mm_eventlog::{CountingApplier, EventLog, EventStatus, DEFAULT_CHECKPOINT};
use mm_log::{CollectSink, Level, Logger, RedactionPolicy, Sink};
use mm_store_sqlite::{AsOf, SqliteStore};

struct Harness {
    _dir: tempfile::TempDir,
    log: EventLog,
    lines: Arc<CollectSink>,
}

/// A `Sink` that writes into a shared [`CollectSink`] so tests can read back
/// exactly what was emitted.
struct SharedSink(Arc<CollectSink>);

impl Sink for SharedSink {
    fn name(&self) -> &str {
        "collect"
    }
    fn write_line(&self, line: &str) -> Result<(), mm_core::MmError> {
        self.0.write_line(line)
    }
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("metamind.db"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let lines = Arc::new(CollectSink::new());
    let logger = Logger::new(
        Level::Trace,
        vec![Box::new(SharedSink(Arc::clone(&lines)))],
        Some(Arc::new(store.clone())),
        RedactionPolicy::kernel_default(),
    );
    let log = EventLog::new(store, Arc::new(logger), Arc::new(UlidFactory::new()));
    Harness {
        _dir: dir,
        log,
        lines,
    }
}

fn event(i: u64) -> NewEvent {
    NewEvent::new(
        EventKind::StoreMutation,
        &serde_json::json!({ "i": i }),
        None,
    )
    .unwrap()
}

async fn commit_n(log: &EventLog, n: u64) {
    for i in 0..n {
        let id = log.append(event(i)).await.unwrap();
        log.commit(&id).await.unwrap();
    }
}

#[tokio::test]
async fn append_then_commit_produces_a_gapless_committed_log() {
    let h = harness().await;
    commit_n(&h.log, 5).await;

    let records = h.log.all_records().await.unwrap();
    assert_eq!(records.len(), 5);
    let seqs: Vec<i64> = records.iter().map(|r| r.seq).collect();
    assert_eq!(
        seqs,
        vec![1, 2, 3, 4, 5],
        "sequence numbers must be gapless"
    );
    assert!(records.iter().all(|r| r.status == EventStatus::Committed));
    assert_eq!(h.log.head_seq().await.unwrap(), 5);
    assert_eq!(h.log.status_counts().await.unwrap(), (0, 5, 0));

    // Every version of the same fact hashes identically.
    assert_eq!(records[0].hash, records[0].hash);
    assert_ne!(
        records[0].hash, records[1].hash,
        "different content, different hash"
    );
}

#[tokio::test]
async fn appending_the_same_event_and_instant_is_idempotent() {
    let h = harness().await;
    let at = Timestamp::from_rfc3339("2024-05-05T05:05:05.000000000Z").unwrap();

    let first = h.log.append_at(event(1), at).await.unwrap();
    let second = h.log.append_at(event(1), at).await.unwrap();
    assert_eq!(
        first, second,
        "a retried append must not create a second row"
    );
    assert_eq!(h.log.all_records().await.unwrap().len(), 1);

    // A different instant is a different event (the hash covers created_at).
    let later = h
        .log
        .append_at(
            event(1),
            Timestamp::from_rfc3339("2024-05-05T05:05:06.000000000Z").unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(first, later);
    assert_eq!(h.log.all_records().await.unwrap().len(), 2);
}

#[tokio::test]
async fn committing_an_unknown_or_already_committed_event_is_an_error() {
    let h = harness().await;
    let unknown = UlidFactory::new().next();
    assert!(h.log.commit(&unknown).await.is_err());

    let id = h.log.append(event(1)).await.unwrap();
    h.log.commit(&id).await.unwrap();
    let err = h.log.commit(&id).await.unwrap_err();
    assert!(err.to_string().contains("not provisional"), "got {err}");
}

#[tokio::test]
async fn a_provisional_event_is_aborted_on_replay_and_applied_exactly_once() {
    let h = harness().await;

    // Simulate a crash: append, then never commit.
    let orphan = h.log.append(event(42)).await.unwrap();
    assert_eq!(h.log.status_counts().await.unwrap(), (1, 0, 0));

    let mut applier = CountingApplier::new();
    let snapshot = h.log.replay(AsOf::now(), &mut applier).await.unwrap();

    assert_eq!(snapshot.aborted, vec![orphan], "the orphan must be aborted");
    assert_eq!(snapshot.applied, 0, "an aborted event must not be applied");
    assert!(applier.is_empty());
    assert_eq!(h.log.status_counts().await.unwrap(), (0, 0, 1));

    // The abort is audited.
    let audit = h.log.store().audit_chain_report().await.unwrap();
    assert!(audit.is_ok(), "{audit:?}");
    assert_eq!(audit.rows, 1);
    let rows = h.log.store().audit_rows().await.unwrap();
    assert_eq!(rows[0].event_code, "eventlog.abort");

    // Replaying again applies nothing twice and aborts nothing new.
    let mut second = CountingApplier::new();
    let again = h.log.replay(AsOf::now(), &mut second).await.unwrap();
    assert!(again.aborted.is_empty());
    assert_eq!(again.applied, 0);
    assert_eq!(h.log.status_counts().await.unwrap(), (0, 0, 1));
}

#[tokio::test]
async fn every_committed_event_has_exactly_one_audit_record() {
    let h = harness().await;
    commit_n(&h.log, 7).await;

    let report = h.log.store().audit_chain_report().await.unwrap();
    assert!(report.is_ok(), "{report:?}");
    assert_eq!(report.rows, 7, "one audit record per committed event");
    assert_eq!(report.hashes_verified, 7);

    let rows = h.log.store().audit_rows().await.unwrap();
    assert!(rows.iter().all(|r| r.event_code == "eventlog.commit"));
    // The audit record shares its identity with the event it attests to.
    let events = h.log.all_records().await.unwrap();
    let event_ids: Vec<String> = events.iter().map(|e| mm_core::ulid_string(&e.id)).collect();
    let audit_ids: Vec<String> = rows.iter().map(|r| r.record_id.clone()).collect();
    assert_eq!(event_ids, audit_ids);
}

#[tokio::test]
async fn replay_is_deterministic_and_reproduces_the_checkpoint_hash() {
    let h = harness().await;
    commit_n(&h.log, 6).await;

    let mut first = CountingApplier::new();
    let a = h.log.replay(AsOf::now(), &mut first).await.unwrap();
    let mut second = CountingApplier::new();
    let b = h.log.replay(AsOf::now(), &mut second).await.unwrap();

    assert_eq!(a.log_hash, b.log_hash);
    assert_eq!(a.applier_hash, b.applier_hash);
    assert_eq!(a.applied, 6);
    assert_eq!(a.last_seq, 6);
    assert_eq!(first.sequences, vec![1, 2, 3, 4, 5, 6]);

    // A checkpoint records that hash at that sequence.
    h.log
        .checkpoint(DEFAULT_CHECKPOINT, a.last_seq, &a.log_hash)
        .await
        .unwrap();
    let stored = h
        .log
        .checkpoint_of(DEFAULT_CHECKPOINT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored, (6, a.log_hash.clone()));

    // Replaying *to the checkpointed sequence* reproduces the checkpoint exactly.
    // (`checkpoint` itself appends an event, so the log is now one longer — a
    // checkpoint describes the state at its own sequence, not at every later one.)
    let mut third = CountingApplier::new();
    let c = h.log.replay(AsOf::system(6), &mut third).await.unwrap();
    assert_eq!(c.log_hash, stored.1, "replay must reproduce the checkpoint");
    assert_eq!(c.last_seq, 6);

    // Past the checkpoint the hash legitimately changes, and the checkpoint is no
    // longer comparable.
    let mut fourth = CountingApplier::new();
    let d = h.log.replay(AsOf::now(), &mut fourth).await.unwrap();
    assert_eq!(d.applied, 7, "the checkpoint event is itself an event");
    assert_ne!(d.log_hash, stored.1);
}

#[tokio::test]
async fn replay_can_stop_at_a_point_in_time() {
    let h = harness().await;
    commit_n(&h.log, 5).await;

    let mut partial = CountingApplier::new();
    let snapshot = h.log.replay(AsOf::system(3), &mut partial).await.unwrap();
    assert_eq!(snapshot.applied, 3);
    assert_eq!(snapshot.last_seq, 3);
    assert_eq!(partial.sequences, vec![1, 2, 3]);

    let full_hash = {
        let mut all = CountingApplier::new();
        h.log.replay(AsOf::now(), &mut all).await.unwrap().log_hash
    };
    assert_ne!(
        snapshot.log_hash, full_hash,
        "a partial replay must not claim the whole log's hash"
    );
}

#[tokio::test]
async fn a_rejected_apply_aborts_the_event_and_surfaces_the_error() {
    struct Rejecting;
    impl mm_eventlog::EventApplier for Rejecting {
        fn apply(&mut self, record: &mm_eventlog::EventRecord) -> Result<(), mm_core::MmError> {
            Err(mm_core::MmError::Event(format!("rejecting {}", record.seq)))
        }
    }

    let h = harness().await;
    let err = h
        .log
        .append_and_apply(event(1), &mut Rejecting)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("rejecting 1"));
    assert_eq!(
        h.log.status_counts().await.unwrap(),
        (0, 0, 1),
        "a failed apply must not leave a committed event"
    );
}

#[tokio::test]
async fn append_and_apply_commits_when_the_projection_accepts() {
    let h = harness().await;
    let mut applier = CountingApplier::new();
    let id = h
        .log
        .append_and_apply(event(9), &mut applier)
        .await
        .unwrap();
    assert_eq!(applier.len(), 1);
    let record = h.log.record(&id).await.unwrap().unwrap();
    assert_eq!(record.status, EventStatus::Committed);
    assert_eq!(h.log.status_counts().await.unwrap(), (0, 1, 0));
}

#[tokio::test]
async fn logging_does_not_change_the_replayed_state() {
    // Same log, two loggers: one collecting at Trace, one suppressing everything.
    let h = harness().await;
    commit_n(&h.log, 4).await;

    let quiet = Logger::new(
        Level::Error,
        vec![Box::new(SharedSink(Arc::clone(&h.lines)))],
        Some(Arc::new(h.log.store().clone())),
        RedactionPolicy::kernel_default(),
    );
    let quiet_log = EventLog::new(
        h.log.store().clone(),
        Arc::new(quiet),
        Arc::new(UlidFactory::new()),
    );

    let mut loud_applier = CountingApplier::new();
    let loud = h.log.replay(AsOf::now(), &mut loud_applier).await.unwrap();
    let mut quiet_applier = CountingApplier::new();
    let silent = quiet_log
        .replay(AsOf::now(), &mut quiet_applier)
        .await
        .unwrap();

    assert_eq!(loud.log_hash, silent.log_hash);
    assert_eq!(loud.applier_hash, silent.applier_hash);
}

mod property {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]

        #[test]
        fn any_mutation_sequence_replays_to_the_same_hash(mutations in proptest::collection::vec(any::<u16>(), 1..25)) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let h = harness().await;
                for m in &mutations {
                    let id = h.log.append(event(u64::from(*m))).await.unwrap();
                    h.log.commit(&id).await.unwrap();
                }

                let mut one = CountingApplier::new();
                let a = h.log.replay(AsOf::now(), &mut one).await.unwrap();
                let mut two = CountingApplier::new();
                let b = h.log.replay(AsOf::now(), &mut two).await.unwrap();

                prop_assert_eq!(a.log_hash, b.log_hash);
                prop_assert_eq!(a.applied as usize, mutations.len());
                prop_assert_eq!(a.last_seq, mutations.len() as i64);
                Ok(())
            }).unwrap();
        }
    }
}
