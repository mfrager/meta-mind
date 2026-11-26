//! End-to-end replay determinism.
//!
//! The kernel's central claim is that state is a fold over an append-only log, so
//! replaying the same log must always produce the same hash — with logging on,
//! with logging off, and across separate processes.

use std::sync::Arc;

use mm_core::{EventKind, NewEvent, UlidFactory};
use mm_e2e::{run_cli_ok, value_after, Workspace};
use mm_eventlog::{CountingApplier, EventLog};
use mm_log::{CollectSink, Level, Logger, RedactionPolicy, Sink};
use mm_store_sqlite::{AsOf, SqliteStore};

struct SharedSink(Arc<CollectSink>);

impl Sink for SharedSink {
    fn name(&self) -> &str {
        "collect"
    }
    fn write_line(&self, line: &str) -> Result<(), mm_core::MmError> {
        self.0.write_line(line)
    }
}

/// Load the fixture mutation sequence.
fn mutations() -> Vec<NewEvent> {
    Workspace::fixture("events/mutations.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).expect("fixture is JSON");
            NewEvent {
                kind: EventKind::from_wire(value["kind"].as_str().unwrap()),
                payload: value["payload"].clone(),
                correlation: None,
            }
        })
        .collect()
}

#[tokio::test]
async fn replaying_the_fixture_sequence_twice_yields_the_same_hash() {
    let ws = Workspace::new();
    let cfg = ws.config();
    let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    sqlite.migrate().await.unwrap();

    let loud = Arc::new(Logger::new(
        Level::Trace,
        vec![Box::new(SharedSink(Arc::new(CollectSink::new())))],
        Some(Arc::new(sqlite.clone())),
        RedactionPolicy::kernel_default(),
    ));
    let log = EventLog::new(
        sqlite.clone(),
        Arc::clone(&loud),
        Arc::new(UlidFactory::new()),
    );

    for event in mutations() {
        let id = log.append(event).await.unwrap();
        log.commit(&id).await.unwrap();
    }

    let mut first = CountingApplier::new();
    let a = log.replay(AsOf::now(), &mut first).await.unwrap();
    let mut second = CountingApplier::new();
    let b = log.replay(AsOf::now(), &mut second).await.unwrap();

    assert_eq!(a.log_hash, b.log_hash);
    assert_eq!(a.applier_hash, b.applier_hash);
    assert_eq!(a.applied, 10);
    assert_eq!(a.last_seq, 10);
    assert_eq!(first.sequences, (1..=10).collect::<Vec<i64>>());

    // Logging off must not change the state.
    let quiet = Arc::new(Logger::new(
        Level::Error,
        Vec::new(),
        Some(Arc::new(sqlite.clone())),
        RedactionPolicy::kernel_default(),
    ));
    let quiet_log = EventLog::new(sqlite.clone(), quiet, Arc::new(UlidFactory::new()));
    let mut quiet_applier = CountingApplier::new();
    let c = quiet_log
        .replay(AsOf::now(), &mut quiet_applier)
        .await
        .unwrap();
    assert_eq!(
        a.log_hash, c.log_hash,
        "logging must not change replayed state"
    );
    assert_eq!(a.applier_hash, c.applier_hash);

    log.checkpoint(mm_eventlog::DEFAULT_CHECKPOINT, a.last_seq, &a.log_hash)
        .await
        .unwrap();
    let stored = log
        .checkpoint_of(mm_eventlog::DEFAULT_CHECKPOINT)
        .await
        .unwrap();
    assert_eq!(stored.unwrap().1, a.log_hash);
}

#[tokio::test]
async fn the_cli_reproduces_the_same_state_hash_across_processes() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let first = run_cli_ok(&ws.data_dir, &["replay", "--from", "0"]);
    let second = run_cli_ok(&ws.data_dir, &["replay", "--from", "0"]);

    let h1 = value_after(&first, "state_hash").expect("first replay reports state_hash");
    let h2 = value_after(&second, "state_hash").expect("second replay reports state_hash");
    assert_eq!(h1, h2, "separate replay processes must agree");
    assert_eq!(h1.len(), 64, "the state hash is sha256 hex");

    // A bounded replay reaches a different (but equally deterministic) point.
    let bounded = run_cli_ok(&ws.data_dir, &["replay", "--until", "1"]);
    let hb = value_after(&bounded, "state_hash").unwrap();
    assert_ne!(
        hb, h1,
        "a partial replay must not claim the whole log's hash"
    );
    let bounded_again = run_cli_ok(&ws.data_dir, &["replay", "--until", "1"]);
    assert_eq!(hb, value_after(&bounded_again, "state_hash").unwrap());
}

#[tokio::test]
async fn a_provisional_event_is_aborted_by_the_next_replay() {
    let ws = Workspace::new();
    let cfg = ws.config();
    let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    sqlite.migrate().await.unwrap();
    let logger = Arc::new(Logger::new(
        Level::Error,
        Vec::new(),
        Some(Arc::new(sqlite.clone())),
        RedactionPolicy::kernel_default(),
    ));
    let log = EventLog::new(sqlite, logger, Arc::new(UlidFactory::new()));

    // Crash: append, never commit.
    let orphan = log
        .append(NewEvent::new(EventKind::Custom, &serde_json::json!({"i": 1}), None).unwrap())
        .await
        .unwrap();

    let out = run_cli_ok(&ws.data_dir, &["replay", "--from", "0"]);
    assert!(
        out.contains("aborted provisional 1"),
        "the replay must report the abort:\n{out}"
    );
    let record = log.record(&orphan).await.unwrap().unwrap();
    assert_eq!(record.status, mm_eventlog::EventStatus::Aborted);

    // Idempotent: a second replay aborts nothing new.
    let again = run_cli_ok(&ws.data_dir, &["replay", "--from", "0"]);
    assert!(again.contains("aborted provisional 0"), "{again}");
}
