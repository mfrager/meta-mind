//! Phase 12 end-to-end: the closed developmental loop, driven through the real `mm-cli`
//! binary against a throwaway data directory.
//!
//! This is the plan's §8 pass gate as a test, in the shape the earlier phases' gates have:
//! the fixtures live in `bench/qualification/` and the assertions read what the commands
//! wrote, so the test fails when the artifact and the gate drift apart rather than when a
//! copy of the fixture does.
//!
//! The phase's claim is specific and this file checks it step by step:
//!
//! * **A novel goal becomes a capability with no human code edit.** The goal is absent
//!   from every module in the tree, and the run produces a design revision, a scaffolded
//!   module with a test, a benchmark comparison, a promotion with a written reason and a
//!   hot-load — the six things §8's criterion 1 names.
//! * **The loop decides, not a model.** Every stage outcome, the promotion and the reason
//!   are rows; a run that "completed" without judging anything fails here.
//! * **Budgets are hard and the run is deterministic.** Two independent runs of the same
//!   goal in two workspaces finish with the *same* remaining envelope, stage for stage,
//!   because the debits are constants and no stage spends what it cannot afford. The
//!   digest, by contrast, is per-run (it folds that run's own ULIDs) — which is why
//!   `loop replay` compares a run against *itself* and this test asserts both halves.
//! * **The developmental ledger is immutable.** The seeded debt corpus names a protected
//!   subject; `gc --apply` collects the ordinary findings and refuses that one, the refusal
//!   is a row that asserts protection, and the database refuses to rewrite or erase it.
//! * **Production is written by promotion only.** The promoted capability lands under the
//!   artifact root and not in the repository the loop is running in.
//!
//! # One deviation from §3's file list, recorded rather than taken silently
//!
//! The plan sketches `tests/e2e/{qualification,hot_load,gate_bypass_adversarial}.rs`. This
//! repository names a phase's gate after the phase (`phase09_firewall.rs`,
//! `phase10_tools.rs`, `selfeng_cycle.rs` for Phase 11), so the gate is
//! `phase12_autonomy.rs`. `hot_load.rs` keeps the plan's name because it is one specific
//! property rather than the whole gate.
//!
//! # What this test does not do
//!
//! It does not run `mm-loop` as a background process or drive its `--ticks`: a supervised
//! service is a deployment property, and `mm-runtime`'s own tests plus
//! `design/planning/full/phase_12_autonomy_build_plan.md` §10's readiness/ticks contract
//! are exercised by `mm-runtime`'s unit tests and by the binary's own flags. What the gate
//! needs from the process is that the *same* controller runs a goal to a promoted,
//! hot-loaded capability, which is what `loop run` here is.

use std::path::PathBuf;

use mm_core::{Param, Params, Tabular};
use mm_e2e::{run_cli, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// The novel goal the qualification run answers.
const GOAL: &str = "bench/qualification/goal_novel.json";
/// The hard envelope the run is given.
const BUDGET: &str = "bench/qualification/budget.toml";
/// The document the run revises.
const DESIGN_DOC: &str = "design/qualification/goal_attainment.md";
/// The eight kinds of debt, in the corpus's expected order.
const DEBT_KINDS: [&str; 6] = [
    "unused_capability",
    "duplicate_policy",
    "conflicting_policy",
    "stale_memory",
    "unused_schema",
    "obsolete_technique",
];
/// The ten stages, in order — the loop's own contract, restated here so a run that dropped
/// one and still called itself complete fails.
const STAGES: [&str; 10] = [
    "experience",
    "event_log",
    "meta_analysis",
    "capability_gap",
    "change_set",
    "self_engineering",
    "test_benchmark",
    "shadow",
    "promotion_gate",
    "new_version",
];

/// A repository-relative fixture's absolute path.
fn repo_path(relative: &str) -> PathBuf {
    Workspace::repo_root().join(relative)
}

/// The single JSON object a `--json` command prints.
fn json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not one JSON object ({e}):\n{stdout}"))
}

/// The workspace's migrated store.
async fn store(ws: &Workspace) -> SqliteStore {
    let cfg = ws.config();
    let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    store.migrate().await.unwrap();
    store
}

/// A `SELECT count(*) AS n` from the store.
async fn count(store: &SqliteStore, sql: &str, params: Params) -> i64 {
    let rows = store.query_json(sql, params).await.unwrap();
    rows[0]["n"].as_i64().unwrap_or(-1)
}

/// Run the qualification goal and return its report.
fn qualify(ws: &Workspace) -> (serde_json::Value, String) {
    let stdout = run_cli_ok(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            repo_path(GOAL).to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    let report = json(&stdout);
    assert!(
        !report["promoted"].is_null(),
        "the run must promote a version: {report}"
    );
    let run_id = report["run_id"]
        .as_str()
        .expect("the report names the run")
        .to_string();
    (report, run_id)
}

/// The whole gate, in the plan's §8 order.
#[tokio::test]
async fn the_phase_twelve_gate_passes_end_to_end() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // 1. The novel goal, run to a promoted capability. Ten stages, all of them `ok`: a
    //    run that aborted would have printed a report too, so the stages are asserted by
    //    name and position rather than counted.
    let (report, run_id) = qualify(&ws);
    let stages = report["stages"]
        .as_array()
        .expect("the report lists stages");
    assert_eq!(stages.len(), STAGES.len(), "{report}");
    for (expected, stage) in STAGES.iter().zip(stages) {
        assert_eq!(stage["stage"].as_str(), Some(*expected), "{report}");
        assert_eq!(
            stage["outcome"].as_str(),
            Some("ok"),
            "stage {expected} must complete: {stage}"
        );
    }
    let digest = report["digest"].as_str().expect("a digest");
    assert_eq!(digest.len(), 64, "the digest is a hash: {digest}");
    let remaining = report["budget_used"].clone();

    // 2. A second, independent run of the same goal reaches the same remaining envelope.
    //    This is the budget half of determinism: the debits are constants per stage, so a
    //    run that spent a different amount is a run whose arithmetic depended on something
    //    other than the stage list. (The *digest* is per-run by construction — it folds the
    //    run's own ULIDs — which is why determinism is asserted in two different ways.)
    let other = Workspace::new();
    run_cli_ok(&other.data_dir, &["doctor"]);
    let (second, second_id) = qualify(&other);
    assert_ne!(run_id, second_id, "two runs are two runs");
    assert_eq!(
        second["budget_used"], remaining,
        "the same goal spends the same budget"
    );
    let second_stages: Vec<Option<&str>> = second["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stage| stage["stage"].as_str())
        .collect();
    assert_eq!(
        second_stages,
        STAGES.iter().map(|stage| Some(*stage)).collect::<Vec<_>>(),
        "and runs the same stages"
    );

    // 3. `loop status` reads the run's row, its committed stages and its digest — and the
    //    digest it recomputes from the log has to be the one the row recorded.
    let status = json(&run_cli_ok(
        &ws.data_dir,
        &["loop", "status", "--run", &run_id, "--json"],
    ));
    assert_eq!(
        status["row"]["status"].as_str(),
        Some("completed"),
        "the run's row says completed: {status}"
    );
    assert_eq!(status["row"]["novel"].as_i64(), Some(1), "{status}");
    assert_eq!(
        status["digest_identical"].as_bool(),
        Some(true),
        "the recorded digest recomputes: {status}"
    );
    assert_eq!(status["recorded_digest"].as_str(), Some(digest));
    assert_eq!(
        status["stages"].as_array().map(Vec::len),
        Some(STAGES.len()),
        "every stage has a committed row: {status}"
    );

    // 4. `loop artifacts` returns the whole provenance chain the phase's criterion 2
    //    names: PiSession → Edit → ModuleVersion → ChangeSet → Promotion.
    let artifacts = json(&run_cli_ok(
        &ws.data_dir,
        &["loop", "artifacts", "--run", &run_id, "--json"],
    ));
    assert_eq!(
        artifacts["chain_complete"].as_bool(),
        Some(true),
        "the chain is complete: {artifacts}"
    );
    assert_eq!(
        artifacts["chain_rendered"].as_str(),
        Some("mmc:PiSession -> mmc:Edit -> mmc:ModuleVersion -> mm:ChangeSet -> mm:Promotion")
    );
    assert!(
        artifacts["chain"]["pi_session"].is_string(),
        "the session that authored the candidate is named: {artifacts}"
    );
    assert!(
        artifacts["chain"]["edits"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0)
            > 0,
        "and the files it edited: {artifacts}"
    );
    assert!(artifacts["chain"]["module"].is_string(), "{artifacts}");
    assert!(artifacts["chain"]["change_set"].is_string(), "{artifacts}");
    assert!(artifacts["chain"]["promotion"].is_string(), "{artifacts}");
    let kinds: Vec<&str> = artifacts["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .filter_map(|artifact| artifact["kind"].as_str())
        .collect();
    // An artifact's `kind` is the stage that registered it, which is why the benchmark is
    // `test_benchmark` here: the stage that ran the harness is the stage that owns the
    // result, and an artifact that named its own kind could disagree with the stage that
    // wrote it.
    for kind in [
        "experience",
        "meta_analysis",
        "capability_gap",
        "change_set",
        "self_engineering",
        "test_benchmark",
        "promotion_gate",
        "new_version",
    ] {
        assert!(
            kinds.contains(&kind),
            "the run registered its {kind} artifact: {kinds:?}"
        );
    }

    // 5. `loop replay` re-folds the run's own events and compares them with the digest the
    //    run recorded. This is the phase's criterion 3.
    let replay = json(&run_cli_ok(
        &ws.data_dir,
        &["loop", "replay", "--run", &run_id, "--json"],
    ));
    assert_eq!(replay["identical"].as_bool(), Some(true), "{replay}");
    assert_eq!(replay["recorded"].as_str(), Some(digest));
    assert_eq!(replay["recomputed"].as_str(), Some(digest));
    assert_eq!(
        replay["stages"].as_array().map(Vec::len),
        Some(STAGES.len())
    );

    // 6. The self-model measured the run: three numeric divergences, each in [0,1].
    //    Criterion 4's first half.
    let report_json = json(&run_cli_ok(
        &ws.data_dir,
        &["self-model", "report", "--run", &run_id, "--json"],
    ));
    assert_eq!(report_json["run_id"].as_str(), Some(run_id.as_str()));
    let dims = report_json["dims"]
        .as_array()
        .expect("the report lists dimensions");
    assert_eq!(dims.len(), 3, "exactly three divergences: {report_json}");
    for dim in dims {
        let value = dim["value"].as_f64().expect("a numeric divergence");
        assert!(
            (0.0..=1.0).contains(&value),
            "a divergence is a fraction: {dim}"
        );
        assert!(!dim["dimension"].as_str().unwrap_or_default().is_empty());
    }
    for scalar in ["actual_model", "actual_ideal", "model_ideal"] {
        assert!(
            report_json[scalar].as_f64().is_some(),
            "{scalar} is a number: {report_json}"
        );
    }

    // 7. The debt scan finds the seeded corpus, including the protected subject — and the
    //    sweep that follows collects the ordinary findings and refuses that one.
    //    Criterion 4's second half.
    let scan = json(&run_cli_ok(&ws.data_dir, &["debt", "scan", "--json"]));
    let findings = scan["findings"].as_array().expect("findings");
    assert!(
        findings.len() >= DEBT_KINDS.len(),
        "the six seeded kinds are found: {scan}"
    );
    for kind in DEBT_KINDS {
        assert!(
            findings
                .iter()
                .any(|finding| finding["kind"].as_str() == Some(kind)),
            "the seeded {kind} is found: {findings:?}"
        );
    }
    assert_eq!(scan["protected"].as_i64(), Some(1), "{scan}");
    assert!(
        findings.iter().all(|finding| {
            finding["evidence"]
                .as_array()
                .map(|evidence| !evidence.is_empty())
                .unwrap_or(false)
        }),
        "a finding with no evidence is an opinion: {findings:?}"
    );

    let collected = json(&run_cli_ok(&ws.data_dir, &["gc", "--apply", "--json"]));
    assert_eq!(
        collected["protected_untouched"].as_bool(),
        Some(true),
        "the ledger guard held: {collected}"
    );
    assert!(
        collected["applied"].as_i64().unwrap_or(0) >= 1,
        "the ordinary findings were collected: {collected}"
    );
    assert_eq!(
        collected["refused"].as_i64(),
        Some(1),
        "and exactly the protected subject was refused: {collected}"
    );
    assert_eq!(
        collected["dry_run"].as_bool(),
        Some(false),
        "--apply is not a dry run: {collected}"
    );

    // 8. The promoted capability is loaded, its function is exposed, and the load is a row.
    let goal = json(&std::fs::read_to_string(repo_path(GOAL)).expect("the goal fixture"));
    let uri = goal["target_uri"].as_str().expect("target_uri");
    let function = goal["function"].as_str().expect("function");
    let spec = format!("{uri}@0.1.0");
    let receipt = json(&run_cli_ok(
        &ws.data_dir,
        &["module", "load", "--uri", &spec, "--json"],
    ));
    assert_eq!(receipt["status"].as_str(), Some("loaded"), "{receipt}");
    assert_eq!(receipt["uri"].as_str(), Some(uri));
    assert_eq!(receipt["version"].as_str(), Some("0.1.0"));
    assert_eq!(
        receipt["functions"].as_array().map(|functions| functions
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>()),
        Some(vec![function]),
        "the capability the goal named is the one the load exposes: {receipt}"
    );

    // 9. The design revision went through the same promoted change set as the code.
    let history = json(&run_cli_ok(
        &ws.data_dir,
        &["design", "history", "--doc", DESIGN_DOC, "--json"],
    ));
    let revisions = history["revisions"].as_array().expect("revisions");
    assert_eq!(revisions.len(), 1, "{history}");
    assert_eq!(revisions[0]["revision"].as_i64(), Some(1));
    assert_eq!(revisions[0]["applied"].as_bool(), Some(true));
    assert!(
        revisions[0]["change_set"]
            .as_str()
            .is_some_and(|id| id.len() == 26),
        "the revision names its change set: {history}"
    );

    // 10. The `/self` graph the run mirrored into conforms to its own shapes.
    let validated = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "self"]);
    assert!(
        validated.contains("conforms") || validated.contains("0 violation"),
        "{validated}"
    );

    // 11. The rows are the ones the commands claimed. A command that printed a ULID and
    //     wrote nothing would pass everything above and fail here.
    let sql = store(&ws).await;
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM loop_runs WHERE id = ?",
            vec![Param::Text(run_id.clone())]
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(DISTINCT idx) AS n FROM loop_iterations WHERE run_id = ?",
            vec![Param::Text(run_id.clone())]
        )
        .await,
        STAGES.len() as i64,
        "the idempotency key is unique per stage"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM self_model_reports WHERE run_id = ?",
            vec![Param::Text(run_id.clone())]
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM divergence_metrics",
            Params::new()
        )
        .await,
        3,
        "a report is a vector, not a number"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM change_sets WHERE status = 'promoted'",
            Params::new()
        )
        .await,
        1,
        "the run's change set ended promoted"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM promotions WHERE decision = 'promote'",
            Params::new()
        )
        .await,
        1,
        "and the decision is a row with a written reason"
    );
    let reasons: Vec<Option<String>> = sql
        .query_json("SELECT reason FROM promotions", Params::new())
        .await
        .unwrap()
        .iter()
        .map(|row| row["reason"].as_str().map(str::to_string))
        .collect();
    assert!(
        reasons.iter().all(|reason| reason
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())),
        "every decision carries a reason: {reasons:?}"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM design_revisions WHERE revision = 1 AND promoted = 1",
            Params::new()
        )
        .await,
        1
    );

    // 12. The ledger guard is a property of the data, not only of the collector. The
    //     refusal has to survive a writer that never goes through the API.
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM gc_actions WHERE protects_ledger = 1 AND applied = 0",
            Params::new()
        )
        .await,
        1
    );
    let rewrite = sql
        .execute(
            "UPDATE gc_actions SET applied = 1 WHERE protects_ledger = 1",
            Params::new(),
        )
        .await;
    assert!(rewrite.is_err(), "the protected row is append-only in SQL");
    let erase = sql
        .execute(
            "DELETE FROM gc_actions WHERE protects_ledger = 1",
            Params::new(),
        )
        .await;
    assert!(erase.is_err(), "and cannot be erased");

    // 13. The run did not write the capability into the tree it was running in. This is
    //     the same invariant `audit production-tree` asserts, checked from the outside.
    let target_path = goal["target_path"].as_str().expect("target_path");
    assert!(
        !Workspace::repo_root().join(target_path).exists(),
        "the promoted capability must not appear in the repository tree"
    );
    let revision = ws
        .data_dir
        .join("artifacts")
        .join("design")
        .join("qualification")
        .join("goal_attainment.md");
    assert!(
        revision.exists(),
        "the revision is materialised under the design kind directory: {}",
        revision.display()
    );
    // A module version is materialised under its *name*, not its repository path: the
    // artifact root is where a promoted capability lives, and `modules/cognition/x` is a
    // source-tree path. The design revision, by contrast, mirrors the document's own path
    // under the `design` kind directory.
    let module_name = goal["module_name"].as_str().expect("module_name");
    let version_dir = ws
        .data_dir
        .join("artifacts")
        .join("modules")
        .join(module_name)
        .join("0.1.0");
    assert!(
        version_dir.join("plugin.toml").exists(),
        "it is materialised under the artifact root instead: {}",
        version_dir.display()
    );

    // 14. The log and audit streams verify over everything the run wrote, and the identity
    //     invariants are intact. Criteria 5 and 7.
    let verified = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(
        verified.contains("passed") || verified.contains("[ok]"),
        "{verified}"
    );
    let being = run_cli_ok(&ws.data_dir, &["being", "verify"]);
    assert!(being.contains("being verify"), "{being}");
}

/// The other direction of the same gate: a run whose stage refuses is a *recorded*
/// refusal, not a silent success.
///
/// The attack is the honest goal with its target pointed at a module that exists, which is
/// what stage 4 checks `novel` against. The point of putting it here, rather than only in
/// the adversarial file, is that the gate's positive path is only meaningful if the
/// negative path leaves a row: a loop that could not distinguish "produced a capability"
/// from "found one already there" would promote the second while printing the first.
#[tokio::test]
async fn a_run_that_cannot_produce_its_capability_records_the_refusal() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let mut goal = json(&std::fs::read_to_string(repo_path(GOAL)).expect("the goal fixture"));
    goal["target_path"] = serde_json::Value::String("modules/cognition/closed-loop".into());
    goal["target_uri"] =
        serde_json::Value::String("https://metamind.dev/code/module/cognition/closed-loop".into());
    let goal_file = ws.root().join("existing_capability.json");
    std::fs::write(&goal_file, serde_json::to_string_pretty(&goal).unwrap()).unwrap();

    let output = run_cli(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            goal_file.to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    assert!(
        !output.status.success(),
        "a refused run must not exit zero:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&String::from_utf8_lossy(&output.stdout));
    assert!(report["promoted"].is_null(), "{report}");
    let gap = report["stages"]
        .as_array()
        .expect("stages")
        .iter()
        .find(|stage| stage["stage"].as_str() == Some("capability_gap"))
        .expect("the gap stage is in the report");
    assert_ne!(
        gap["outcome"].as_str(),
        Some("ok"),
        "the gap stage is the one that refuses: {gap}"
    );
    assert!(
        gap["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("already present")),
        "the refusal says what it found: {gap}"
    );

    // The rows are the record: the run is `failed`, and nothing was promoted, loaded or
    // revised by a goal that described an existing capability.
    let sql = store(&ws).await;
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM loop_runs WHERE status = 'failed'",
            Params::new()
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM module_loads",
            Params::new()
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM design_revisions",
            Params::new()
        )
        .await,
        0
    );
}
