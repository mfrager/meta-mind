//! Phase 4 end-to-end: the operator's `being` surface, run as a subprocess.
//!
//! These drive the real `mm-cli` binary and then read what it left behind, so they
//! check the whole path — guard, store, audit chain, and `/being` mirror — rather
//! than one crate's view of it.

use mm_e2e::{run_cli, run_cli_ok, Workspace};
use mm_store_graph::GraphStore;

/// `being verify` is the Phase 4 gate and must be clean on a fresh kernel.
#[tokio::test]
async fn being_verify_reports_a_clean_kernel() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let out = run_cli_ok(&ws.data_dir, &["being", "verify"]);
    assert!(out.contains("4 enforced"), "{out}");
    assert!(out.contains("denied of"), "{out}");
    // Every adversarial case is denied, and nothing else is.
    let denied = out
        .lines()
        .find(|l| l.contains("adversarial"))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(str::to_string);
    let total = out
        .lines()
        .find(|l| l.contains("adversarial"))
        .and_then(|l| l.split_whitespace().nth(4))
        .map(str::to_string);
    assert_eq!(denied, total, "not every forbidden op was denied:\n{out}");
    assert!(out.contains("0 violation(s)"), "{out}");
    assert!(!out.contains("FAIL"), "{out}");
}

/// A goal added and transitioned through the CLI is visible in `/being`, with the
/// status the transition reached.
#[tokio::test]
async fn a_goal_added_through_the_cli_reaches_the_being_graph() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let added = run_cli_ok(
        &ws.data_dir,
        &[
            "being",
            "goal",
            "add",
            "--description",
            "ship Phase 4",
            "--priority",
            "0.9",
        ],
    );
    let goal = added
        .trim()
        .rsplit(' ')
        .next()
        .expect("the command prints the new goal's ULID")
        .to_string();
    assert_eq!(goal.len(), mm_core::ULID_LEN, "{added}");
    let goal_id = mm_core::id::parse_ulid(&goal).unwrap();

    run_cli_ok(
        &ws.data_dir,
        &[
            "being",
            "goal",
            "transition",
            "--id",
            &goal,
            "--to",
            "fulfilled",
        ],
    );

    // Read the mirror without the CLI: the projection is what was actually written.
    let cfg = ws.config();
    let store = GraphStore::open(&cfg.store.graph_dir, &cfg.shapes_file())
        .await
        .unwrap();
    let rows = store.triples("being").await.unwrap();
    let iri = mm_core::iri::data(&goal_id).into_string();
    let status = rows
        .iter()
        .find(|row| {
            row["s"].as_str() == Some(iri.as_str())
                && row["p"]
                    .as_str()
                    .is_some_and(|p| p.ends_with("#goalStatus"))
        })
        .map(|row| match &row["o"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(map) => map
                .get("value")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            _ => String::new(),
        });
    store.shutdown().await.unwrap();
    // The store holds the graph directory's lock until it is dropped, and the CLI
    // is about to open it.
    drop(store);
    assert_eq!(status.as_deref(), Some("fulfilled"));

    // The graph with a committed goal still conforms to the being shapes.
    let out = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "being"]);
    assert!(out.contains("conforms: 0 violations"), "{out}");
}

/// A terminal goal cannot be reopened, and the refusal names the invariant.
#[tokio::test]
async fn a_terminal_goal_is_not_reopened_through_the_cli() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let added = run_cli_ok(
        &ws.data_dir,
        &["being", "goal", "add", "--description", "an abandoned plan"],
    );
    let goal = added.trim().rsplit(' ').next().unwrap().to_string();
    run_cli_ok(
        &ws.data_dir,
        &[
            "being",
            "goal",
            "transition",
            "--id",
            &goal,
            "--to",
            "abandoned",
        ],
    );

    let output = run_cli(
        &ws.data_dir,
        &[
            "being",
            "goal",
            "transition",
            "--id",
            &goal,
            "--to",
            "active",
        ],
    );
    assert!(
        !output.status.success(),
        "reopening a terminal goal must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(stderr.contains("no_history_rewrite"), "{stderr}");
}
