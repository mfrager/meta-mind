//! The trace store, against a real SQLite database.
//!
//! These tests drive `SqliteTraceStore` through the kernel's `Tabular` boundary
//! and check the two properties the replay gate rests on: persisting the same
//! deliberation twice leaves exactly one program and one trace, and a loaded trace
//! equals the one that was written, byte for byte.

use std::sync::Arc;

use mm_core::{Tabular, Ulid, UlidFactory};
use mm_metacog::trace::{ProgramTrace, SqliteTraceStore, TraceOutcome, TraceRow, TraceStore};
use mm_metacog::{
    CognitiveBudget, CognitiveEpisode, CognitiveProgram, Context, DefaultCompiler, EpisodeStatus,
    ProgramCompiler, ScanIssue, ScanResult, Tier, Uncertainty,
};
use mm_store_sqlite::SqliteStore;

fn ulid(n: u128) -> Ulid {
    Ulid::from_parts(1_700_000_000_000, n)
}

fn scan() -> ScanResult {
    ScanResult {
        issues: vec![ScanIssue {
            kind: "missing_evidence".into(),
            materiality: 0.7,
            target: "rollback".into(),
        }],
        uncertainties: vec![Uncertainty {
            id: "u1".into(),
            description: "does the window hold".into(),
            magnitude: 0.5,
        }],
        stakes: 0.7,
        irreversibility: 0.3,
        verification_value: 0.6,
        ..ScanResult::default()
    }
}

/// An episode, its scan, its compiled program, and a trace over that program that
/// runs the first operation and declines the rest.
///
/// `seed` distinguishes two deliberations so a test can build a second, genuinely
/// different one; every identifier and every candidate comes from it, so the two
/// programs have different content-derived ids.
fn harness(seed: u128) -> (CognitiveEpisode, ScanResult, CognitiveProgram, ProgramTrace) {
    let scan = scan();
    let goal = if seed == 1 { ulid(2) } else { ulid(seed + 100) };
    let candidates: Vec<String> = if seed == 1 {
        vec!["rollback".into(), "roll_forward".into()]
    } else {
        vec!["merge".into(), "revert".into()]
    };
    let mut episode = CognitiveEpisode::new(
        ulid(seed),
        goal,
        Context::new("decide whether to roll back the deploy"),
        CognitiveBudget::from_spec(8, 4, 0.5, 60_000).unwrap(),
    )
    .unwrap();
    episode = episode
        .with_candidates(candidates)
        .with_assumptions(vec![ulid(7)])
        .with_techniques(vec!["https://metamind.dev/library/technique/bisect".into()]);
    episode.tier = Tier::T3;
    episode.select_tier(&scan);

    let program = DefaultCompiler::new()
        .compile(&episode, &scan, &episode.budget)
        .expect("the default compiler compiles a T3 episode");

    let mut trace = ProgramTrace::new(program.id);
    for (index, step) in program.steps.iter().enumerate() {
        let selected = index == 0;
        trace
            .push(TraceRow {
                seq: u32::try_from(index).unwrap(),
                node_id: step.node,
                op: step.op.tag().to_string(),
                op_class: step.op.class(),
                selected,
                value: step.value,
                outcome: if selected {
                    TraceOutcome::Executed
                } else {
                    TraceOutcome::Skipped
                },
                budget_after: episode.budget,
                stopping_reason: if selected {
                    None
                } else {
                    Some("remaining_uncertainty_below".to_string())
                },
            })
            .unwrap();
    }
    trace.seal();
    trace.validate().unwrap();
    (episode, scan, program, trace)
}

async fn store() -> (tempfile::TempDir, SqliteStore, SqliteTraceStore) {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::open(&dir.path().join("metamind.db"))
        .await
        .unwrap();
    sqlite.migrate().await.unwrap();
    let tabular: Arc<dyn Tabular> = Arc::new(sqlite.clone());
    let traces = SqliteTraceStore::new(tabular, Arc::new(UlidFactory::new()));
    (dir, sqlite, traces)
}

#[tokio::test]
async fn persist_then_load_round_trips_every_object() {
    let (_dir, sqlite, traces) = store().await;
    let (episode, scan, program, trace) = harness(1);

    traces
        .persist(&episode, &scan, &program, &trace)
        .await
        .unwrap();

    let loaded_program = traces
        .load_program(program.id)
        .await
        .unwrap()
        .expect("the program was written");
    assert_eq!(loaded_program, program);
    assert_eq!(loaded_program.content_hash(), program.content_hash());

    let loaded_trace = traces.load_trace(program.id).await.unwrap();
    assert_eq!(loaded_trace, trace);
    assert_eq!(loaded_trace.id, trace.id);
    assert_eq!(loaded_trace.executed(), 1);
    assert_eq!(loaded_trace.rows.len(), program.steps.len());

    let record = traces
        .load_episode(episode.id)
        .await
        .unwrap()
        .expect("the episode was written");
    assert_eq!(record.id, episode.id);
    assert_eq!(record.tier, program.tier);
    assert_eq!(record.status, EpisodeStatus::Programmed);
    assert_eq!(record.trace_id, episode.id);
    assert_eq!(record.goal, mm_core::ulid_string(&episode.goal));
    assert!((record.stakes - 0.7).abs() < 1e-12);
    assert!((record.uncertainty - 0.5).abs() < 1e-12);
    assert!((record.reversibility - 0.3).abs() < 1e-12);
    assert_eq!(record.budget, episode.budget);

    assert_eq!(
        traces.program_ids_for(episode.id).await.unwrap(),
        vec![program.id]
    );
    assert_eq!(sqlite.row_count("episodes").await.unwrap(), 1);
}

#[tokio::test]
async fn persisting_twice_leaves_exactly_one_program_and_one_trace() {
    let (_dir, sqlite, traces) = store().await;
    let (episode, scan, program, trace) = harness(1);

    traces
        .persist(&episode, &scan, &program, &trace)
        .await
        .unwrap();
    traces
        .persist(&episode, &scan, &program, &trace)
        .await
        .unwrap();

    assert_eq!(sqlite.row_count("episodes").await.unwrap(), 1);
    assert_eq!(sqlite.row_count("programs").await.unwrap(), 1);
    assert_eq!(
        sqlite.row_count("program_traces").await.unwrap(),
        i64::try_from(trace.rows.len()).unwrap()
    );
    // A duplicated sequence number would make `push` refuse, so a successful load
    // is itself the proof that the trace was not written twice.
    assert_eq!(traces.load_trace(program.id).await.unwrap(), trace);
    assert_eq!(traces.program_ids_for(episode.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_trace_for_another_program_is_refused() {
    let (_dir, _sqlite, traces) = store().await;
    let (episode, scan, program, _trace) = harness(1);
    let (other_episode, other_scan, other_program, other_trace) = harness(20);
    assert_ne!(program.id, other_program.id);

    // A trace that belongs to a different program id is refused outright.
    let err = traces
        .persist(&episode, &scan, &program, &other_trace)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("the trace is for"), "{err}");

    // The honest pair still writes.
    traces
        .persist(&other_episode, &other_scan, &other_program, &other_trace)
        .await
        .unwrap();
    assert_eq!(
        traces.load_trace(other_program.id).await.unwrap(),
        other_trace
    );
}

#[tokio::test]
async fn an_unknown_program_has_no_trace_to_load() {
    let (_dir, _sqlite, traces) = store().await;
    let err = traces.load_trace(ulid(999)).await.unwrap_err();
    assert!(err.to_string().contains("no program"), "{err}");
    assert!(traces.load_program(ulid(999)).await.unwrap().is_none());
    assert!(traces.load_episode(ulid(999)).await.unwrap().is_none());
    assert!(traces.program_ids_for(ulid(999)).await.unwrap().is_empty());
}

#[test]
fn a_row_appended_out_of_order_is_refused() {
    let mut trace = ProgramTrace::new(ulid(3));
    let row = TraceRow {
        seq: 5,
        node_id: 0,
        op: "recall".into(),
        op_class: mm_metacog::OpClass::Recall,
        selected: true,
        value: mm_metacog::OperationValue::new(0.5, 0.5, 0.5, 0.05).unwrap(),
        outcome: TraceOutcome::Executed,
        budget_after: CognitiveBudget::from_spec(4, 2, 1.0, 100).unwrap(),
        stopping_reason: None,
    };
    let err = trace.push(row).unwrap_err();
    assert!(err.to_string().contains("row 5"), "{err}");
    assert!(trace.rows.is_empty());
}

#[test]
fn validate_refuses_an_unsealed_gapped_or_inconsistent_trace() {
    let budget = CognitiveBudget::from_spec(4, 2, 1.0, 100).unwrap();
    let value = mm_metacog::OperationValue::new(0.5, 0.5, 0.5, 0.05).unwrap();
    let row = |seq: u32, selected: bool, outcome: TraceOutcome| TraceRow {
        seq,
        node_id: 0,
        op: "recall".into(),
        op_class: mm_metacog::OpClass::Recall,
        selected,
        value,
        outcome,
        budget_after: budget,
        stopping_reason: None,
    };

    // Never sealed.
    let unsealed = ProgramTrace {
        id: Ulid::nil(),
        program_id: ulid(4),
        rows: vec![row(0, true, TraceOutcome::Executed)],
    };
    assert!(unsealed.validate().is_err());

    // Sealed, but the sequence numbers are not contiguous from zero.
    let gapped = ProgramTrace {
        id: ulid(5),
        program_id: ulid(4),
        rows: vec![
            row(0, true, TraceOutcome::Executed),
            row(3, false, TraceOutcome::Skipped),
        ],
    };
    let err = gapped.validate().unwrap_err();
    assert!(err.to_string().contains("contiguous"), "{err}");

    // `selected` and `outcome` disagree.
    let inconsistent = ProgramTrace {
        id: ulid(6),
        program_id: ulid(4),
        rows: vec![row(0, true, TraceOutcome::Skipped)],
    };
    assert!(inconsistent.validate().is_err());

    // No rows at all.
    let empty = ProgramTrace {
        id: ulid(7),
        program_id: ulid(4),
        rows: Vec::new(),
    };
    assert!(empty.validate().is_err());
}

#[test]
fn sealing_is_content_addressed_and_stable() {
    let budget = CognitiveBudget::from_spec(4, 2, 1.0, 100).unwrap();
    let value = mm_metacog::OperationValue::new(0.5, 0.5, 0.5, 0.05).unwrap();
    let row = |seq: u32| TraceRow {
        seq,
        node_id: u16::try_from(seq).unwrap(),
        op: "recall".into(),
        op_class: mm_metacog::OpClass::Recall,
        selected: true,
        value,
        outcome: TraceOutcome::Executed,
        budget_after: budget,
        stopping_reason: None,
    };

    let mut one = ProgramTrace::new(ulid(8));
    one.push(row(0)).unwrap();
    one.push(row(1)).unwrap();
    one.seal();

    let mut two = ProgramTrace::new(ulid(8));
    two.push(row(0)).unwrap();
    two.push(row(1)).unwrap();
    two.seal();
    assert_eq!(one.id, two.id, "identical traces must seal identically");
    assert!(!one.id.is_nil());

    let mut different = ProgramTrace::new(ulid(8));
    different.push(row(0)).unwrap();
    different.seal();
    assert_ne!(one.id, different.id);
    assert_ne!(one.content_hash(), different.content_hash());

    // The program id is part of the content, so two programs' traces differ.
    let mut elsewhere = ProgramTrace::new(ulid(9));
    elsewhere.push(row(0)).unwrap();
    elsewhere.push(row(1)).unwrap();
    elsewhere.seal();
    assert_ne!(one.id, elsewhere.id);
}

#[test]
fn outcomes_round_trip_through_their_stored_form() {
    for outcome in TraceOutcome::ALL {
        assert_eq!(TraceOutcome::parse(outcome.as_str()), Some(outcome));
    }
    assert_eq!(TraceOutcome::parse("nope"), None);
    assert!(TraceOutcome::Executed.ran());
    assert!(!TraceOutcome::Stopped.ran());
    assert_eq!(TraceOutcome::Skipped.to_string(), "skipped");
}
