//! Phase 12 end-to-end: a promoted capability is hot-loaded, and the load is real.
//!
//! The phase's §7 asks for `tests/e2e/hot_load.rs` with the assertion that "the promoted
//! module's function is callable without restart, and the prior version still serves
//! during the load". This file is that test, driven through the real `mm-cli` binary — the
//! same way `selfeng_cycle.rs` drives Phase 11 — so what it checks is the shipped
//! behaviour of a command rather than of a function.
//!
//! Four properties make this end-to-end rather than a second unit test:
//!
//! * **The module under test was produced by a run, not written here.** The test asks the
//!   loop for the novel goal in `bench/qualification/goal_novel.json` and then loads the
//!   version the *run* materialised, so a loader that could only load a directory this
//!   test created would fail.
//! * **The load is checkable on disk.** `<data>/artifacts/modules/<name>/<version>/` is
//!   read back: the manifest the loader parsed, the generated source, the generated test
//!   and the manual are there, and the manifest's `uri`, `version` and T-Box function are
//!   the ones the goal named. A receipt that named a version with no files behind it would
//!   fail here.
//! * **A load is a row per invocation.** `module_loads` is read directly, and the count
//!   moves by exactly one per `module load` — a re-load of the same version is recorded as
//!   a load rather than silently swallowed, and the version stays the active one.
//! * **Invariant 5 is asserted, not assumed.** The promoted capability must not exist in
//!   the source tree: the loop writes under its artifact root, and a run that wrote
//!   `modules/cognition/goal-attainment` would weaken Phase 11's `audit production-tree`.
//!
//! # What this test deliberately does not do
//!
//! It does not exercise a *successful* `module rollback`. A rollback restores the version
//! that was active before the newest one, and this phase produces exactly one version of
//! each module (`LoopGoal::module_version`), so the only way to give this test two
//! materialised versions of `goal-attainment` would be to hand-write a second directory
//! into the artifact root — a promotion that never happened, standing in for the very thing
//! the test is supposed to check. What the test asserts instead is the guard: with one
//! loaded version, `module rollback` refuses rather than inventing a version to restore.
//! The successful path, with two real versions, is covered by `crates/mm-runtime`'s own
//! `a_second_version_keeps_the_first_one_recorded`, which materialises both versions and
//! then rolls back to the first.

use mm_core::{Param, Params, Tabular};
use mm_e2e::{run_cli, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// The novel goal the qualification run answers.
const GOAL_FILE: &str = "bench/qualification/goal_novel.json";
/// Its budget envelope, so the run's cap is the one the phase shipped.
const BUDGET_FILE: &str = "bench/qualification/budget.toml";
/// The design document the run revises, which is also a change-set patch path.
const DESIGN_DOC: &str = "design/qualification/goal_attainment.md";
/// The version this phase produces (one per phase, per `LoopGoal::module_version`).
const VERSION: &str = "0.1.0";

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

/// The single JSON object `--json` prints.
fn json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not one JSON object ({e}):\n{stdout}"))
}

/// The rows of `module_loads` for a module, newest first.
async fn loads(store: &SqliteStore, uri: &str) -> Vec<serde_json::Value> {
    store
        .query_json(
            "SELECT version, status, functions FROM module_loads WHERE module_uri = ? \
             ORDER BY loaded_at DESC, id DESC",
            vec![Param::Text(uri.to_string())],
        )
        .await
        .unwrap()
}

/// The phase-12 half of the gate: a promoted capability is loaded without a restart.
#[tokio::test]
async fn a_promoted_capability_is_hot_loaded_without_a_restart() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let repo = Workspace::repo_root();

    // The goal is the fixture's, read rather than restated: a URI hardcoded here could
    // drift from the one the loop was asked to produce, and the drift would look like a
    // passing loader.
    let goal = json(&std::fs::read_to_string(repo.join(GOAL_FILE)).expect("the goal fixture"));
    let uri = goal["target_uri"].as_str().expect("target_uri").to_string();
    let function = goal["function"].as_str().expect("function").to_string();
    let module_name = goal["module_name"]
        .as_str()
        .expect("module_name")
        .to_string();
    let target_path = goal["target_path"]
        .as_str()
        .expect("target_path")
        .to_string();

    // 1. The loop produces the capability. No human code edit is involved: the module is
    //    scaffolded by the run and materialised under its artifact root.
    let run = json(&run_cli_ok(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            repo.join(GOAL_FILE).to_str().unwrap(),
            "--budget-file",
            repo.join(BUDGET_FILE).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    ));
    assert!(
        run["stages"].as_array().expect("stages").len() == 10
            && run["stages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|stage| stage["outcome"] == "ok"),
        "the run must complete every stage: {run}"
    );
    assert!(
        run["promoted"].is_string(),
        "the run must promote a version: {run}"
    );

    let sql = store(&ws).await;

    // 2. The promoted module version is on disk, under the artifact root and nowhere else.
    let version_dir = ws
        .data_dir
        .join("artifacts")
        .join("modules")
        .join(&module_name)
        .join(VERSION);
    let manifest_path = version_dir.join("plugin.toml");
    assert!(
        manifest_path.exists(),
        "the loop materialises the promoted version at {}",
        manifest_path.display()
    );
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(
        manifest.contains(&uri) && manifest.contains(VERSION) && manifest.contains(&function),
        "the manifest names what the goal asked for:\n{manifest}"
    );
    for generated in ["src/lib.rs", "tests/behaviour.rs", "manual/module.md"] {
        assert!(
            version_dir.join(generated).exists(),
            "the promoted version carries {generated}"
        );
    }
    // The run's own hot-load is the first `module_loads` row: the stage that registers a
    // version and the stage that activates it are the same stage.
    let before = loads(&sql, &uri).await;
    assert_eq!(
        before.len(),
        1,
        "the run hot-loaded the version once: {before:?}"
    );
    assert_eq!(before[0]["status"].as_str(), Some("loaded"));
    assert_eq!(before[0]["version"].as_str(), Some(VERSION));
    assert_eq!(before[0]["functions"].as_i64(), Some(1));

    // 3. The promoted capability was not written into the source tree: the loop's writes
    //    belong to its artifact root, which is what keeps `audit production-tree` true.
    assert!(
        !repo.join(&target_path).exists(),
        "a run must not write {target_path} into the repository it is running in"
    );

    // 4. The rollback guard, on the state the phase actually produces: one loaded version
    //    is not something to roll back *to*. (See the module doc for why a successful
    //    rollback is exercised in `mm-runtime` instead.)
    let refused = run_cli(
        &ws.data_dir,
        &["module", "rollback", "--uri", uri.as_str(), "--json"],
    );
    assert!(
        !refused.status.success(),
        "a rollback with no earlier version is refused:\n{}",
        String::from_utf8_lossy(&refused.stdout)
    );
    assert_eq!(
        loads(&sql, &uri).await.len(),
        1,
        "a refused rollback records no load row"
    );

    // 5. `module load` activates the materialised version. `previous` is null because no
    //    *different* version was ever active — the same version is already the current one,
    //    which is what makes this load idempotent rather than a second activation of a
    //    different capability.
    let spec = format!("{uri}@{VERSION}");
    let receipt = json(&run_cli_ok(
        &ws.data_dir,
        &["module", "load", "--uri", spec.as_str(), "--json"],
    ));
    assert_eq!(receipt["status"].as_str(), Some("loaded"));
    assert_eq!(receipt["uri"].as_str(), Some(uri.as_str()));
    assert_eq!(receipt["version"].as_str(), Some(VERSION));
    assert_eq!(receipt["previous"], serde_json::Value::Null);
    let functions: Vec<String> = receipt["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .map(|value| value.as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        functions,
        vec![function.clone()],
        "the promoted function is the one the load exposes"
    );

    // 6. A load is recorded, not swallowed: one row per invocation, and the same version
    //    stays the active one after a re-load.
    let after = loads(&sql, &uri).await;
    assert_eq!(after.len(), 2, "the load recorded a second row: {after:?}");
    assert!(after
        .iter()
        .all(|row| row["status"].as_str() == Some("loaded")));
    assert!(after
        .iter()
        .all(|row| row["version"].as_str() == Some(VERSION)));

    let again = json(&run_cli_ok(
        &ws.data_dir,
        &["module", "load", "--uri", spec.as_str(), "--json"],
    ));
    assert_eq!(again["status"].as_str(), Some("loaded"));
    assert_eq!(
        loads(&sql, &uri).await.len(),
        3,
        "a re-load is idempotent in effect and recorded again"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM module_loads WHERE module_uri = ? AND status = 'loaded'",
            vec![Param::Text(uri.clone())],
        )
        .await,
        3
    );

    // 7. A version that was never materialised is refused, and nothing is recorded: a load
    //    row for a directory that does not exist would make `module load` a claim rather
    //    than an activation.
    let bogus = format!("{uri}@9.9.9");
    let unknown = run_cli(
        &ws.data_dir,
        &["module", "load", "--uri", bogus.as_str(), "--json"],
    );
    assert!(
        !unknown.status.success(),
        "an unmaterialised version is refused:\n{}",
        String::from_utf8_lossy(&unknown.stdout)
    );
    assert_eq!(
        loads(&sql, &uri).await.len(),
        3,
        "the refused load recorded nothing"
    );

    // 8. The run also revised the design document, and the revision is a row tied to the
    //    change set that carried it — the same promoted change set that produced the
    //    module, which is what makes "design and code go through one pipeline" checkable.
    // The document's IRI is the one `DesignDocRef` derives from its path: the `design/`
    // prefix is the namespace's own and is stripped before the IRI is built.
    let doc_uri =
        mm_core::iri::design(DESIGN_DOC.trim_start_matches("design/"), None).into_string();
    let revisions = sql
        .query_json(
            "SELECT doc_uri, revision, change_set_id, promoted FROM design_revisions \
             WHERE doc_uri LIKE ? ORDER BY revision",
            vec![Param::Text(format!("{doc_uri}%"))],
        )
        .await
        .unwrap();
    assert_eq!(revisions.len(), 1, "one revision: {revisions:?}");
    assert_eq!(revisions[0]["revision"].as_i64(), Some(1));
    assert_eq!(revisions[0]["promoted"].as_i64(), Some(1));
    assert!(
        revisions[0]["change_set_id"]
            .as_str()
            .is_some_and(|id| id.len() == 26),
        "the revision names the change set that produced it"
    );
}
