//! The adversarial corpus: no deliberate bad write is silently accepted (plan §7,
//! §9).
//!
//! `bench/memory/adversarial/` holds one file per failure mode, and each case
//! declares the flag it must raise in `expect_flag`. The test builds the world the
//! case is meant to be judged in — the episode corpus, its consolidation, and the
//! file's other cases — and then screens every case against it. A case is only a
//! "duplicate" or a "contradiction" *of something*, so the something has to exist
//! first; that is why the cases are stored before they are screened.
//!
//! The two blocking flags are asserted through the real door as well:
//! `missing_provenance` never reaches a row because `Memory::validate` refuses it,
//! and `duplicate` is refused by `remember` rather than stored a second time.

mod common;

use common::Harness;
use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_memory::screen::Candidate;
use mm_memory::{
    entity_ulid, Admission, FlagKind, Memory, MemoryEngine, MemoryError, MemoryKind, TimeInterval,
};
use serde_json::Value;

/// One fixture case, parsed.
struct Case {
    /// The case's own name, for a readable assertion.
    name: String,
    /// The flag the fixture declares.
    expect: FlagKind,
    /// The memory to store, or `None` when the case may not become one.
    memory: Option<Memory>,
    /// What the case looks like before it is a memory.
    candidate: Candidate,
}

/// Parse one fixture line into a case.
fn case(value: &Value, ids: &UlidFactory) -> Case {
    let name = value["case"].as_str().expect("case name").to_string();
    let expect = FlagKind::parse(value["expect_flag"].as_str().expect("expect_flag"))
        .expect("a known flag name");
    let kind = MemoryKind::parse(value["kind"].as_str().expect("kind")).expect("a known kind");
    let content = value["content"].as_str().expect("content").to_string();
    let confidence = value["confidence"].as_f64().expect("confidence") as f32;
    let importance = value["importance"].as_f64().expect("importance") as f32;
    let entities: Vec<Ulid> = value["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .map(|entity| entity_ulid(entity.as_str().expect("an entity name")))
        .collect();
    let valid_from =
        Timestamp::from_rfc3339(value["valid_from"].as_str().expect("valid_from")).expect("a date");
    // `nil` is the fixture's way of saying "no source", and such a case may not be
    // stored — which is the point of the file it lives in.
    let provenance = match value["provenance"].as_str() {
        Some("nil") => None,
        _ => Some(ids.next()),
    };
    let id = ids.next();
    let memory = provenance.map(|provenance| {
        Memory::new(
            id,
            kind,
            content.clone(),
            TimeInterval::open(valid_from),
            confidence,
            importance,
            provenance,
            valid_from,
        )
        .expect("a storable case")
        .with_entities(entities.clone())
    });
    let candidate = Candidate {
        id: Some(id),
        kind,
        content,
        provenance,
        entities,
        valid_from,
        valid_until: None,
    };
    Case {
        name,
        expect,
        memory,
        candidate,
    }
}

/// Build the world a fixture file is judged in and return it with its cases.
async fn world(file: &str) -> (Harness, MemoryEngine, Vec<Case>) {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;
    // The corpus, consolidated: episodes alone would not give a stale claim
    // anything newer to be stale *of*, because the generalization is what carries
    // the current state of a subject.
    engine
        .ingest_episodes(&common::episodes_path(), &harness.logger)
        .await
        .expect("the corpus ingests");
    engine
        .consolidate(TimeInterval::open(Timestamp::EPOCH))
        .await
        .expect("the corpus consolidates");

    let path = common::adversarial_dir().join(file);
    let cases: Vec<Case> = common::jsonl(&path)
        .iter()
        .map(|value| case(value, engine.ids()))
        .collect();
    // Store everything that can legally be stored, so each case is screened
    // against the others rather than against an empty store.
    for case in &cases {
        if let Some(memory) = &case.memory {
            engine.store().insert(memory).await.expect("a case stores");
        }
    }
    engine
        .reindex(&harness.dir.path().join("memory.pack"))
        .await
        .expect("the index rebuilds");
    (harness, engine, cases)
}

/// The instant the fixtures are judged at — the day the gate runs.
fn now() -> Timestamp {
    Timestamp::from_rfc3339("2026-10-08T00:00:00Z").expect("a date")
}

/// Every case in every adversarial file raises the flag it declares.
#[tokio::test]
async fn every_adversarial_case_is_flagged_for_what_it_is() {
    for file in [
        "duplicates.jsonl",
        "near_duplicates.jsonl",
        "stale.jsonl",
        "contradictory.jsonl",
        "orphan_entity.jsonl",
        "missing_provenance.jsonl",
    ] {
        let (harness, engine, cases) = world(file).await;
        assert!(!cases.is_empty(), "{file} has no cases");
        for case in &cases {
            let report = engine.screen(&case.candidate, now()).await.unwrap();
            assert!(
                report.flagged(case.expect),
                "{file}/{}: expected {:?}, got {:?}",
                case.name,
                case.expect,
                report.names()
            );
        }
        harness.shutdown().await;
    }
}

/// A duplicate is refused: the same fact stored twice would double its weight in
/// every rank.
#[tokio::test]
async fn a_duplicate_is_refused_by_the_real_door() {
    let (harness, mut engine, cases) = world("duplicates.jsonl").await;
    let duplicates: Vec<&Case> = cases
        .iter()
        .filter(|case| case.expect == FlagKind::Duplicate)
        .collect();
    assert_eq!(duplicates.len(), 3, "the fixture declares three duplicates");
    for case in duplicates {
        let memory = case.memory.clone().expect("a duplicate is storable");
        match engine.remember(memory).await.unwrap() {
            Admission::Duplicate { existing } => {
                assert_ne!(existing, case.candidate.id.unwrap(), "{} itself", case.name)
            }
            other => panic!("{}: expected a duplicate refusal, got {other:?}", case.name),
        }
    }
    harness.shutdown().await;
}

/// A missing source is a typed refusal, and it is refused *before* a row exists —
/// the fixture's stated enforcement point.
#[tokio::test]
async fn a_memory_without_a_source_is_refused_typed() {
    let at = Timestamp::from_rfc3339("2026-10-08T00:00:00Z").unwrap();
    let id = Ulid::from_parts(1_700_000_000_000, 1);

    // A candidate cannot even be constructed: the constructor is the first guard.
    let error = Memory::new(
        id,
        MemoryKind::Semantic,
        "an unsourced assertion",
        TimeInterval::open(at),
        0.9,
        0.8,
        Ulid::nil(),
        at,
    )
    .unwrap_err();
    match error {
        MemoryError::Validation { field, .. } => assert_eq!(field, "provenance"),
        other => panic!("expected a provenance validation error, got {other:?}"),
    }

    // And the store-side guard bites too, which is what "before the first INSERT"
    // means: a memory whose provenance is cleared after construction is refused
    // rather than written.
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;
    let mut memory = Memory::draft(
        engine.ids().next(),
        MemoryKind::Semantic,
        "a claim that lost its source",
        at,
    )
    .unwrap();
    memory.provenance = Ulid::nil();
    let error = engine.remember(memory).await.unwrap_err();
    match error {
        MemoryError::Validation { field, .. } => assert_eq!(field, "provenance"),
        other => panic!("expected a provenance validation error, got {other:?}"),
    }
    assert_eq!(
        engine.store().count_of(None).await.unwrap(),
        0,
        "nothing may be stored without a source"
    );
    harness.shutdown().await;
}

/// The non-blocking flags are stored and made visible, not swallowed: a
/// contradiction becomes an `mm:contradicts` link and an orphan is logged.
#[tokio::test]
async fn the_non_blocking_flags_are_stored_and_made_visible() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;
    let at = now();
    let (id1, p1, id2, p2, id3, p3) = (
        engine.ids().next(),
        engine.ids().next(),
        engine.ids().next(),
        engine.ids().next(),
        engine.ids().next(),
        engine.ids().next(),
    );
    let entities = vec![entity_ulid("rust"), entity_ulid("toolchain-policy")];
    let claim = |id, provenance, content: &str, confidence, importance| {
        Memory::new(
            id,
            MemoryKind::Semantic,
            content,
            TimeInterval::open(at),
            confidence,
            importance,
            provenance,
            at,
        )
        .unwrap()
        .with_entities(entities.clone())
    };

    // Two opposing claims about the same entities. The first is admitted, and the
    // second is admitted *and linked* rather than quietly resolved.
    let first = claim(
        id1,
        p1,
        "the toolchain policy pins rust to exactly 1.96.1 for reproducibility",
        0.9,
        0.8,
    );
    let second = claim(
        id2,
        p2,
        "the toolchain policy allows any stable rust release for reproducibility",
        0.6,
        0.5,
    );
    assert!(matches!(
        engine.remember(first).await.unwrap(),
        Admission::Accepted
    ));
    assert!(matches!(
        engine.remember(second).await.unwrap(),
        Admission::Accepted
    ));
    let links = engine.store().list_links().await.unwrap();
    assert!(
        links
            .iter()
            .any(|(from, _, relation, _)| *from == id2
                && *relation == mm_memory::LinkKind::Contradicts),
        "a contradiction must be recorded as a link, got {links:?}"
    );

    // An orphan entity is a real trace: stored, never dropped, and logged as what
    // it is so a corpus quietly losing its edges is visible.
    let orphan = Memory::new(
        id3,
        MemoryKind::Semantic,
        "the quantum tessellator calibration depends on the lunar phase",
        TimeInterval::open(at),
        0.4,
        0.3,
        p3,
        at,
    )
    .unwrap()
    .with_entities(vec![entity_ulid("quantum-tessellator")]);
    assert!(matches!(
        engine.remember(orphan).await.unwrap(),
        Admission::Accepted
    ));
    assert_eq!(engine.store().count_of(None).await.unwrap(), 3);
    let orphan_id = mm_core::ulid_string(&id3);
    let flags = harness
        .records_of("memory.add")
        .into_iter()
        .filter(|record| {
            record["fields"]["memory_id"].as_str() == Some(orphan_id.as_str())
                && record["fields"]["flags"].as_str() == Some("orphan_entity")
        })
        .count();
    assert_eq!(flags, 1, "the orphan write must be logged as such");
    harness.shutdown().await;
}
