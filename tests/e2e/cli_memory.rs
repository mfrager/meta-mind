//! Phase 5 end-to-end: the operator's `memory` surface, run as a subprocess.
//!
//! These drive the real `mm-cli` binary against a throwaway data directory, so
//! they check the whole path — the store, the audit chain, and the `/memory`
//! mirror — rather than one crate's view of it. `memory verify` and
//! `graph validate --graph memory` are the Phase 5 gate's own checks.

use mm_e2e::{field, run_cli_ok, Workspace};

/// The repository fixture, as an absolute path for a subprocess argument.
fn fixture(relative: &str) -> String {
    Workspace::repo_root()
        .join(relative)
        .to_str()
        .expect("a UTF-8 path")
        .to_string()
}

/// The first ULID in the command's output.
fn first_ulid(stdout: &str) -> String {
    stdout
        .lines()
        .flat_map(str::split_whitespace)
        .find_map(|token| {
            let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            (token.len() == mm_core::ULID_LEN && mm_core::id::parse_ulid(token).is_ok())
                .then(|| token.to_string())
        })
        .unwrap_or_else(|| panic!("no ULID in:\n{stdout}"))
}

/// Every `<class> <a> vs <b>` line in `memory stats` reports equal counts.
fn assert_reconciled(stats: &str) {
    let mut classes = 0;
    for line in stats.lines() {
        let Some((left, right)) = line.trim().split_once(" vs ") else {
            continue;
        };
        let (Some(a), Some(b)) = (
            left.rsplit(' ').next().and_then(|n| n.parse::<i64>().ok()),
            right.split(' ').next().and_then(|n| n.parse::<i64>().ok()),
        ) else {
            continue;
        };
        assert_eq!(a, b, "SQLite and /memory disagree in {line:?}\n{stats}");
        classes += 1;
    }
    assert!(classes > 0, "no reconciled class in:\n{stats}");
}

/// A memory added through the CLI is retrievable, its row reconciles with
/// `/memory`, and the SHACL shapes accept the mirror.
#[tokio::test]
async fn a_memory_added_through_the_cli_round_trips() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let added = run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "add",
            "--kind",
            "episodic",
            "--content",
            "the deploy window moved to thursday after the outage",
        ],
    );
    let id = first_ulid(&added);

    let got = run_cli_ok(&ws.data_dir, &["memory", "get", &id]);
    assert!(got.contains("episodic"), "{got}");
    assert!(got.contains("thursday"), "{got}");
    assert!(got.contains("provenance"), "{got}");

    let recalled = run_cli_ok(&ws.data_dir, &["memory", "recall", "deploy window"]);
    assert!(recalled.contains("hit(s)"), "{recalled}");
    assert!(recalled.contains(&id), "{recalled}");

    // The row and the projection agree, and nothing is protected-but-archived.
    let stats = run_cli_ok(&ws.data_dir, &["memory", "stats", "--reconcile"]);
    assert_eq!(
        field(&stats, "protected ledger").as_deref(),
        Some("ok"),
        "{stats}"
    );
    assert_eq!(
        field(&stats, "provenance closure").as_deref(),
        Some("ok"),
        "{stats}"
    );
    assert!(!stats.contains("FAIL"), "{stats}");
    assert_reconciled(&stats);

    let verified = run_cli_ok(&ws.data_dir, &["memory", "verify"]);
    assert_eq!(
        field(&verified, "reconcile").as_deref(),
        Some("exact"),
        "{verified}"
    );
    assert!(verified.contains("0 violation(s)"), "{verified}");
    assert_eq!(
        field(&verified, "result").as_deref(),
        Some("ok"),
        "{verified}"
    );

    let shaped = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "memory"]);
    assert!(shaped.contains("conforms: 0 violations"), "{shaped}");
}

/// Consolidating the committed episode stream reduces the memory count, keeps the
/// provenance closure, and leaves every gate clean.
#[tokio::test]
async fn consolidation_reduces_the_corpus_and_stays_reconciled() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let out = run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "consolidate",
            "--window",
            "30d",
            "--input",
            &fixture("bench/memory/episodes.jsonl"),
        ],
    );
    assert!(out.contains("ingested 12 episode(s)"), "{out}");
    assert_eq!(field(&out, "closure").as_deref(), Some("ok"), "{out}");
    let before: i64 = field(&out, "active before").unwrap().parse().unwrap();
    let after: i64 = field(&out, "active after").unwrap().parse().unwrap();
    assert_eq!(before, 12, "{out}");
    assert!(
        after < before,
        "consolidation must reduce the count:\n{out}"
    );
    assert_eq!(field(&out, "summaries").as_deref(), Some("1"), "{out}");

    // Rerunning the same file adds nothing: ingestion is keyed on the episode ref.
    let rerun = run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "consolidate",
            "--window",
            "30d",
            "--input",
            &fixture("bench/memory/episodes.jsonl"),
        ],
    );
    assert!(
        rerun.contains("ingested 12 episode(s)"),
        "the rerun must ingest the same ids, not new ones:\n{rerun}"
    );

    let stats = run_cli_ok(&ws.data_dir, &["memory", "stats", "--reconcile"]);
    assert_eq!(
        field(&stats, "protected ledger").as_deref(),
        Some("ok"),
        "{stats}"
    );
    assert_eq!(
        field(&stats, "provenance closure").as_deref(),
        Some("ok"),
        "{stats}"
    );

    let shaped = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "memory"]);
    assert!(shaped.contains("conforms: 0 violations"), "{shaped}");

    // The log gate now has the memory correlation check, and it holds.
    let logs = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(logs.contains("memory correlation"), "{logs}");
    assert!(logs.contains("logs verify passed"), "{logs}");
    assert!(!logs.contains("[FAIL]"), "{logs}");
}

/// A dry run decides and logs without writing, and a protected record is kept.
#[tokio::test]
async fn forgetting_is_a_dry_run_and_nothing_is_deleted() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "add",
            "--kind",
            "semantic",
            "--content",
            "an old note that has stopped mattering",
        ],
    );
    run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "add",
            "--kind",
            "developmental",
            "--content",
            "I keep what I learned",
        ],
    );

    let out = run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "forget",
            "--now",
            "2026-10-08T00:00:00Z",
            "--dry-run",
        ],
    );
    assert!(out.contains("memory forget (dry run)"), "{out}");
    let considered: i64 = field(&out, "considered").unwrap().parse().unwrap();
    assert!(considered >= 2, "{out}");
    // A dry run archives nothing.
    assert_eq!(field(&out, "would archive").as_deref(), Some("0"), "{out}");

    let stats = run_cli_ok(&ws.data_dir, &["memory", "stats"]);
    assert!(stats.contains("archived"), "{stats}");
    let archived: i64 = field(&stats, "archived").unwrap().parse().unwrap();
    assert_eq!(
        archived, 0,
        "nothing may be archived by a dry run:\n{stats}"
    );
}

/// Retrieval meets the committed precision@5 / recall@5 floors.
#[tokio::test]
async fn memory_eval_meets_the_committed_thresholds() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let out = run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "eval",
            "--gold",
            &fixture("bench/memory/gold.jsonl"),
            "--k",
            "5",
            "--thresholds",
            &fixture("bench/memory/thresholds.toml"),
            "--episodes",
            &fixture("bench/memory/episodes.jsonl"),
        ],
    );
    assert!(out.contains("meets thresholds"), "{out}");
    assert!(!out.contains("BELOW THRESHOLDS"), "{out}");
    let precision: f64 = field(&out, "precision@5")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let recall: f64 = field(&out, "recall@5")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(precision >= 0.80, "precision {precision} below the floor");
    assert!(recall >= 0.80, "recall {recall} below the floor");
}

/// A seeded mistake is retrievable and names its corrective rule.
#[tokio::test]
async fn a_seeded_mistake_is_retrievable_with_its_rule() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let out = run_cli_ok(
        &ws.data_dir,
        &[
            "memory",
            "mistake",
            "add",
            "--failure-mode",
            "reused a stale lockfile",
            "--corrective-rule",
            "re-resolve the lockfile before building",
            "--risk",
            "0.4",
        ],
    );
    let id = first_ulid(&out);

    let got = run_cli_ok(&ws.data_dir, &["memory", "get", &id]);
    assert!(got.contains("mistake"), "{got}");
    assert!(
        got.contains("re-resolve the lockfile before building"),
        "the mistake must name its corrective rule:\n{got}"
    );

    let recalled = run_cli_ok(&ws.data_dir, &["memory", "recall", "stale lockfile"]);
    assert!(recalled.contains(&id), "{recalled}");

    let verified = run_cli_ok(&ws.data_dir, &["memory", "verify"]);
    assert_eq!(
        field(&verified, "result").as_deref(),
        Some("ok"),
        "{verified}"
    );
}
