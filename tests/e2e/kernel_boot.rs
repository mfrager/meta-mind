//! End-to-end kernel boot.
//!
//! Proves the whole Phase 1 chain in one place: the CLI opens both stores, applies
//! the migration, loads the ontology, appends and commits exactly one
//! `KernelBoot` event, and the audit chain accounts for every committed event.

use mm_core::store::Tabular;
use mm_e2e::{field, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

#[tokio::test]
async fn doctor_boots_the_kernel_and_leaves_a_consistent_store() {
    let ws = Workspace::new();
    let stdout = run_cli_ok(&ws.data_dir, &["doctor"]);

    // The report names the versions and the ontology, and admits success last.
    assert!(stdout.contains("kernel ready"), "{stdout}");
    assert!(field(&stdout, "sqlite").is_some(), "{stdout}");
    assert!(field(&stdout, "oxigraph").is_some(), "{stdout}");
    assert!(stdout.contains("journal_mode=wal"), "{stdout}");

    let triples: i64 = field(&stdout, "ontology triples")
        .expect("ontology triples reported")
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .expect("triple count is a number");
    assert!(
        triples > 50,
        "the T-Box should be substantial, got {triples}"
    );
    assert!(field(&stdout, "graph invariants").unwrap().contains("ok"));

    // Inspect the store directly: exactly one committed boot event, and one audit
    // record for every settled event.
    let cfg = ws.config();
    let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();

    let boots = sqlite
        .query_json(
            "SELECT count(*) AS n FROM events WHERE kind = 'kernel.boot' AND status = 'committed'",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(
        boots[0]["n"],
        serde_json::json!(1),
        "exactly one boot event"
    );

    let ontology_events = sqlite
        .query_json(
            "SELECT count(*) AS n FROM events WHERE kind = 'ontology.load'",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(
        ontology_events[0]["n"],
        serde_json::json!(1),
        "the ontology load is itself recorded"
    );

    let settled = sqlite
        .query_json(
            "SELECT count(*) AS n FROM events WHERE status IN ('committed','aborted')",
            vec![],
        )
        .await
        .unwrap();
    let audits = sqlite
        .query_json(
            "SELECT count(*) AS n FROM audit_log WHERE event_code IN ('eventlog.commit','eventlog.abort')",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(
        settled[0]["n"], audits[0]["n"],
        "one audit record per settled event"
    );

    let chain = sqlite.audit_chain_report().await.unwrap();
    assert!(chain.is_ok(), "{chain:?}");

    // Every event's payload is well-formed JSON and every id is a ULID.
    let bad = sqlite
        .query_json(
            "SELECT count(*) AS n FROM events WHERE length(id) <> 26 OR json_valid(payload) = 0",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(bad[0]["n"], serde_json::json!(0));
}

#[tokio::test]
async fn a_second_boot_does_not_duplicate_the_ontology() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let second = run_cli_ok(&ws.data_dir, &["doctor"]);

    // The ontology is already loaded, so nothing new is added and no second
    // ontology.load event is recorded.
    assert!(
        second.contains("(0 loaded this run)"),
        "expected no reload, got:\n{second}"
    );

    let cfg = ws.config();
    let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    let ontology_events = sqlite
        .query_json(
            "SELECT count(*) AS n FROM events WHERE kind = 'ontology.load'",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(ontology_events[0]["n"], serde_json::json!(1));

    let boots = sqlite
        .query_json(
            "SELECT count(*) AS n FROM events WHERE kind = 'kernel.boot'",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(
        boots[0]["n"],
        serde_json::json!(2),
        "each boot is its own event"
    );
}
