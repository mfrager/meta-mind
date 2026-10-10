//! Phase 11 end-to-end: the self-engineering cycle, driven through the real `mm-cli`
//! binary against a throwaway data directory.
//!
//! This is the plan's §8 pass gate as a test, written the way the earlier phases' gates
//! are: a corpus in `bench/` rather than fixtures duplicated here, so the test fails when
//! the artifact and the fixture drift apart.
//!
//! Five properties make this an end-to-end test rather than a second unit test:
//!
//! * **Calibration improves on a recorded baseline.** The corpus is deliberately
//!   overconfident, `--assert-brier-le`/`--assert-ece-le`/`--assert-improves-baseline`
//!   are all asserted by the command, and the run is recorded in `calibration_runs`. The
//!   numbers come from `bench/calibration/reference.json`, which
//!   `compute_reference.py` produced independently.
//! * **An analysis cannot classify nothing.** `meta analyze` refuses to write a row
//!   without at least one error class, and the seeded failure names `retrieval`.
//! * **The regression case is real.** `seeded_bug_01` fails on the buggy subject and
//!   passes after its fix; the command asserts both halves or exits non-zero.
//! * **A gap becomes a typed, reversible change set**, and the sandbox materialises it
//!   under `data/sandbox/` and nowhere else — which the write audit checks afterwards.
//! * **The gate decides, and the decision is written down.** A promotion carries a
//!   reason, a `self_version`, a `promotions` row and a journal entry, and the change
//!   set ends `promoted`.
//!
//! # One thing this test deliberately does not do
//!
//! It does not run `sandbox run --build` or `--test`: those shell out to `cargo` inside a
//! fresh git worktree, which is minutes of compilation for a property (the worktree
//! builds) that `mm-selfeng`'s own tests cover with a real build. What the gate needs from
//! the sandbox is that the candidate was written *there* and nowhere else, and `--apply`
//! plus `--assert-isolated` is exactly that claim.

use mm_core::{Param, Params, Tabular};
use mm_e2e::{field, run_cli, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// The corpus the calibration gate scores.
const CORPUS: &str = "bench/calibration/predictions.jsonl";

/// A repository-relative member's absolute path, so a command never depends on the
/// process's working directory. The CLI resolves relative paths against the repository
/// root, so this is the same request written unambiguously.
/// The workspace's store, migrated.
async fn store(ws: &Workspace) -> SqliteStore {
    let cfg = ws.config();
    let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    store.migrate().await.unwrap();
    store
}

/// A `SELECT count(*)` from the store.
async fn count(store: &SqliteStore, sql: &str, params: Params) -> i64 {
    let rows = store.query_json(sql, params).await.unwrap();
    rows[0]["n"].as_i64().unwrap_or(-1)
}

/// The whole gate, in the plan's order.
#[tokio::test]
async fn the_phase_eleven_gate_passes_end_to_end() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // 1. Calibration: the corpus is overconfident, and one pass must make it honest and
    //    improve on the recorded baseline. `--record-ledger` additionally stakes every
    //    labeled point in the ledger and resolves it, so the same corpus is a prediction
    //    history rather than only a scoring input.
    let calibrated = run_cli_ok(
        &ws.data_dir,
        &[
            "calibrate",
            "--bench",
            CORPUS,
            "--assert-brier-le",
            "0.20",
            "--assert-ece-le",
            "0.10",
            "--assert-improves-baseline",
            "--record-ledger",
        ],
    );
    let brier: f64 = field(&calibrated, "brier")
        .expect("calibrate prints a brier")
        .parse()
        .expect("a number");
    let baseline: f64 = field(&calibrated, "baseline_brier")
        .expect("calibrate prints the baseline")
        .parse()
        .expect("a number");
    assert!(
        brier <= 0.20 && brier < baseline,
        "the calibrated brier must beat the baseline:\n{calibrated}"
    );
    assert_eq!(
        field(&calibrated, "baseline_source").as_deref(),
        Some("reference.json"),
        "the baseline comes from the independent reference:\n{calibrated}"
    );
    assert_eq!(field(&calibrated, "n").as_deref(), Some("100"));

    // 2. Meta-analysis: the seeded failure is diagnosed, classified, and turned into a
    //    lesson. The diagnoser is the heuristic one, so this needs no provider.
    let analyzed = run_cli_ok(
        &ws.data_dir,
        &[
            "meta",
            "analyze",
            "--episode",
            "bench/episodes/seeded_failure_01.json",
            "--assert-lessons-ge",
            "1",
        ],
    );
    assert_eq!(field(&analyzed, "lessons").as_deref(), Some("1"));
    assert_eq!(
        field(&analyzed, "trigger").as_deref(),
        Some("repeated_failure")
    );
    let classes = field(&analyzed, "error_classes").expect("classes are printed");
    assert!(
        classes.split(',').any(|class| class == "retrieval"),
        "the seeded failure is a retrieval error:\n{analyzed}"
    );

    // 3. The mistake compiler's suite: the seeded case holds both halves of its contract.
    let regression = run_cli_ok(
        &ws.data_dir,
        &[
            "regression",
            "run",
            "--suite",
            "bench/regression",
            "--assert-fail-before-pass-after",
            "seeded_bug_01",
        ],
    );
    assert_eq!(field(&regression, "failed").as_deref(), Some("0"));
    assert!(
        regression.contains("fails_before true passes_after true"),
        "{regression}"
    );

    // 4. The gap becomes a typed change set. The command prints its ULID so a shell — or
    //    this test — can carry it forward.
    let created = run_cli_ok(
        &ws.data_dir,
        &["changeset", "new", "--from-gap", "bench/gaps/gap_01.json"],
    );
    let changeset = field(&created, "changeset").expect("the change set's ULID");
    assert_eq!(changeset.len(), mm_core::ULID_LEN, "{created}");
    assert!(created.contains("rollback"), "a change set has a way back");

    // 5. Pi is the only code editor: the recorded session is ingested, its records are
    //    rows, and its edits reach `/code`.
    let pi = run_cli_ok(
        &ws.data_dir,
        &[
            "pi",
            "run",
            "--task",
            "bench/pi/module_scaffold_01.json",
            "--offline",
            "--assert-session-ingested",
        ],
    );
    let events: i64 = field(&pi, "events").expect("events").parse().unwrap();
    let triples: i64 = field(&pi, "triples").expect("triples").parse().unwrap();
    assert!(events > 0 && triples > 0, "{pi}");

    // 6. The candidate is materialised inside the sandbox, and the audit says nothing
    //    outside it moved.
    let sandboxed = run_cli_ok(
        &ws.data_dir,
        &[
            "sandbox",
            "run",
            changeset.as_str(),
            "--apply",
            "--assert-isolated",
        ],
    );
    assert_eq!(
        field(&sandboxed, "outside_count").as_deref(),
        Some("0"),
        "{sandboxed}"
    );
    assert_eq!(field(&sandboxed, "audit_complete").as_deref(), Some("true"));
    let sandbox_dir = field(&sandboxed, "sandbox").expect("the sandbox directory");
    assert!(
        std::path::Path::new(&sandbox_dir).starts_with(Workspace::repo_root().join("data/sandbox")),
        "a candidate lives under data/sandbox: {sandbox_dir}"
    );
    assert!(
        std::path::Path::new(&sandbox_dir)
            .join("modules/cognition/calibration/SESSION.md")
            .exists(),
        "the change set was written into the sandbox"
    );

    // 7. The gate decides, with a written reason and a new self version.
    let promoted = run_cli_ok(
        &ws.data_dir,
        &["promote", changeset.as_str(), "--assert-reason-present"],
    );
    assert_eq!(field(&promoted, "decision").as_deref(), Some("promote"));
    assert_eq!(field(&promoted, "status").as_deref(), Some("promoted"));
    let self_version = field(&promoted, "self_version").expect("a self version");
    assert!(self_version.starts_with("self-"), "{promoted}");
    let reason = field(&promoted, "reason").expect("a written reason");
    assert!(!reason.trim().is_empty(), "{promoted}");

    // 8. No budget was exceeded, and the production tree was not written outside the
    //    promotion.
    let budgets = run_cli_ok(&ws.data_dir, &["budget", "show", "--assert-no-overspend"]);
    assert_eq!(field(&budgets, "overspent").as_deref(), Some("0"));
    let audited = run_cli_ok(
        &ws.data_dir,
        &[
            "audit",
            "production-tree",
            "--assert-unmodified-outside-promotion",
        ],
    );
    assert_eq!(field(&audited, "outside_count").as_deref(), Some("0"));
    assert_eq!(field(&audited, "promotions").as_deref(), Some("1"));

    // 9. The rows are the ones the commands claimed. A command that printed a ULID and
    //    wrote nothing would pass every assertion above and fail here.
    let sql = store(&ws).await;
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM calibration_runs",
            Params::new()
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM meta_analyses",
            Params::new()
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM prediction_ledger",
            Params::new()
        )
        .await,
        100,
        "every labeled point was staked"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM prediction_outcomes",
            Params::new()
        )
        .await,
        100,
        "and resolved exactly once"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM change_sets", Params::new()).await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM change_sets WHERE status = 'promoted'",
            Params::new()
        )
        .await,
        1,
        "the change set ended promoted"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM promotions", Params::new()).await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM evolution_journal WHERE self_version = ?",
            vec![Param::Text(self_version.clone())]
        )
        .await,
        1,
        "the journal holds the version the decision produced"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM pi_sessions", Params::new()).await,
        1
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM pi_events", Params::new()).await,
        events,
        "the session's records are rows"
    );
    // The ledger is append-only in SQL, not only in the API. A direct edit must abort.
    let refused = sql
        .execute(
            "UPDATE prediction_ledger SET probability = 0.5",
            Params::new(),
        )
        .await;
    assert!(refused.is_err(), "the ledger refuses an UPDATE");

    // 10. The log and audit streams verify over everything the cycle wrote.
    let verified = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(
        verified.to_lowercase().contains("ok") || verified.contains("records"),
        "{verified}"
    );
}

/// The gate's other direction: a rejection is a decision too, it carries its written
/// reason, and a rejection without one is refused before anything is recorded.
///
/// The reason this matters as a test rather than as a convention: the phase's contract is
/// that every change is judged and the judgment is readable afterwards, so a rejection
/// path that quietly recorded nothing — or recorded an empty reason — would make "why did
/// this not land" unanswerable while every command still exited zero.
#[tokio::test]
async fn a_rejection_carries_its_reason_and_writes_the_lineage() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let created = run_cli_ok(
        &ws.data_dir,
        &["changeset", "new", "--from-gap", "bench/gaps/gap_01.json"],
    );
    let second = field(&created, "changeset").expect("a change set");
    let rejected = run_cli_ok(
        &ws.data_dir,
        &[
            "reject",
            second.as_str(),
            "--reason",
            "the benchmark does not cover the drifting class",
        ],
    );
    assert_eq!(field(&rejected, "decision").as_deref(), Some("reject"));
    assert!(
        rejected.contains("drifting class"),
        "the reason is written down: {rejected}"
    );
    let sql = store(&ws).await;
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM change_sets WHERE status = 'rejected'",
            Params::new()
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM evolution_journal WHERE decision = 'reject'",
            Params::new()
        )
        .await,
        1
    );
    // A rejection with no reason is refused before anything is recorded.
    let refused = run_cli(
        &ws.data_dir,
        &["reject", second.as_str(), "--reason", "   "],
    );
    assert!(
        !refused.status.success(),
        "a rejection without a reason is not a decision"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM promotions WHERE decision = 'reject'",
            Params::new()
        )
        .await,
        1,
        "the refused command recorded nothing"
    );
}
