//! Phase 11 end-to-end: a recorded Pi session ingested twice, byte for byte.
//!
//! The plan's Pi-ingestion property is "a recorded-session replay is deterministic, and
//! ingestion round-trips to identical canonical RDF". Three claims make that checkable,
//! and this test asserts all three through the CLI and the crate together:
//!
//! * **Ingestion is idempotent.** The session's ULID is the one in its header, and
//!   `pi_events` is keyed by `(session, seq)` with `ON CONFLICT DO NOTHING`, so a second
//!   ingest of the same file adds no rows. The test ingests twice and compares counts.
//! * **The sequence is gapless.** `min(seq) = 1`, `max(seq) = count`, and the count of
//!   distinct sequences equals the count of rows: a dropped record would show up as a
//!   hole here rather than as a session that merely looked shorter.
//! * **The mirror is canonical.** The hash the CLI prints is the hash of
//!   `PiSessionGraph::canonical_turtle` computed independently in the test, so what is
//!   compared is not two runs of the same code path agreeing with themselves but the
//!   bytes the graph received.
//!
//! It also runs the `/code` shape gate over the mirror, because a session that ingested
//! but did not satisfy `mmc:PiSessionShape` would be a `/code` graph an operator cannot
//! validate.

use mm_core::{Param, Params, Tabular};
use mm_e2e::{field, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// The recorded session `bench/pi/module_scaffold_01.json` names.
const FIXTURE: &str = "crates/mm-pi/fixtures/recorded_session_module_scaffold_01.jsonl";

/// The workspace's store, migrated.
async fn store(ws: &Workspace) -> SqliteStore {
    let cfg = ws.config();
    let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    store.migrate().await.unwrap();
    store
}

/// One number from a `SELECT`.
async fn scalar(store: &SqliteStore, sql: &str, params: Params) -> i64 {
    let rows = store.query_json(sql, params).await.unwrap();
    rows[0]["n"].as_i64().unwrap_or(-1)
}

/// Ingestion is idempotent, gapless, and mirrors to exactly the bytes the crate renders.
#[tokio::test]
async fn a_recorded_session_replays_to_the_same_rows_and_the_same_rdf() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let first = run_cli_ok(&ws.data_dir, &["pi", "ingest", FIXTURE]);
    let second = run_cli_ok(&ws.data_dir, &["pi", "ingest", FIXTURE]);

    // The two runs report the same session, the same record count and the same hash: the
    // second ingest is a no-op that still sees everything the first one wrote.
    for key in ["session", "events", "edits", "tool_calls", "turtle_hash"] {
        assert_eq!(
            field(&first, key),
            field(&second, key),
            "the second ingest changed {key}:\n{first}\n{second}"
        );
    }
    let session = field(&first, "session").expect("a session ULID");
    let events: i64 = field(&first, "events").expect("events").parse().unwrap();
    let reported_hash = field(&first, "turtle_hash").expect("a turtle hash");
    assert!(events > 0, "{first}");

    // The hash is the crate's own rendering, computed here rather than taken from the
    // CLI: what is compared is the bytes the graph received, not a self-report.
    let fixture = Workspace::repo_root().join(FIXTURE);
    let parsed = mm_pi::PiSessionGraph::parse(&fixture).expect("the fixture parses");
    // `mm_core::ulid_string` is the kernel's one canonical rendering; `Ulid::to_string`
    // is the crate's own and spells Crockford's digits differently.
    assert_eq!(mm_core::ulid_string(&parsed.session_id), session);
    assert_eq!(u32::try_from(events).unwrap(), parsed.event_count);
    assert_eq!(
        mm_core::content_hash(parsed.canonical_turtle().as_bytes()),
        reported_hash,
        "the CLI's hash is the canonical rendering's hash"
    );
    // Two parses of one file render the same bytes: the "round-trips to identical
    // canonical RDF" half of the plan's claim.
    let again = mm_pi::PiSessionGraph::parse(&fixture).expect("the fixture parses twice");
    assert_eq!(parsed.canonical_turtle(), again.canonical_turtle());

    let sql = store(&ws).await;
    assert_eq!(
        scalar(
            &sql,
            "SELECT count(*) AS n FROM pi_sessions WHERE id = ?",
            vec![Param::Text(session.clone())]
        )
        .await,
        1,
        "the session row is keyed by the header's ULID"
    );
    let rows = scalar(
        &sql,
        "SELECT count(*) AS n FROM pi_events WHERE session_ulid = ?",
        vec![Param::Text(session.clone())],
    )
    .await;
    assert_eq!(
        rows, events,
        "the second ingest added no event rows (idempotent)"
    );
    // Gapless: one row per sequence number, starting at one and ending at the count.
    let gaps = sql
        .query_json(
            "SELECT count(*) AS n FROM pi_events WHERE session_ulid = ? \
             AND (seq < 1 OR seq > ?)",
            vec![Param::Text(session.clone()), Param::Int(events)],
        )
        .await
        .unwrap();
    assert_eq!(gaps[0]["n"].as_i64(), Some(0), "no sequence outside 1..=n");
    let distinct = scalar(
        &sql,
        "SELECT count(DISTINCT seq) AS n FROM pi_events WHERE session_ulid = ?",
        vec![Param::Text(session.clone())],
    )
    .await;
    assert_eq!(distinct, events, "no sequence number repeats");

    // The edits the session made are rows and nodes: the module it scaffolded is what the
    // task contract asked for.
    for edited in [
        "modules/cognition/calibration/plugin.toml",
        "modules/cognition/calibration/src/lib.rs",
        "modules/cognition/calibration/manual/module.md",
    ] {
        assert!(
            parsed
                .edited_paths()
                .iter()
                .any(|path| path.to_string_lossy().contains(edited)),
            "the session did not edit {edited}"
        );
    }

    // The `/code` shape gate accepts the mirror.
    let validated = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "code"]);
    assert!(
        validated.to_lowercase().contains("ok") || validated.contains("0 violation"),
        "the mirrored session must satisfy mmc-shapes:\n{validated}"
    );
}

/// A truncated session is a hard error, not a shorter session.
///
/// The plan's risk row is "session format drift (Pi upgrade)"; the mitigation is that a
/// record this build cannot read is a refusal. A truncated file is the same failure
/// without the version bump, and it is the one that silently loses the last thing the
/// agent did.
#[tokio::test]
async fn a_truncated_session_is_refused_rather_than_partly_ingested() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let truncated = "crates/mm-pi/fixtures/recorded_session_truncated.jsonl";
    let output = mm_e2e::run_cli(&ws.data_dir, &["pi", "ingest", truncated]);
    assert!(
        !output.status.success(),
        "a truncated session must be refused:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let sql = store(&ws).await;
    assert_eq!(
        scalar(&sql, "SELECT count(*) AS n FROM pi_sessions", Params::new()).await,
        0,
        "nothing was ingested"
    );
    assert_eq!(
        scalar(&sql, "SELECT count(*) AS n FROM pi_events", Params::new()).await,
        0,
        "and no events were written"
    );
}
