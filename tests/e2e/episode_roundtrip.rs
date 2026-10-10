//! End-to-end episode round-trip (Phase 8).
//!
//! The metacognitive controller's whole claim is that a deliberation is
//! reproducible: the same episode, the same scan and the same budget compile the
//! same program, score it the same way and trace it the same way, every time. This
//! drives the real CLI the way the pass gate does — `run`, `replay`, `verify`,
//! `budget-audit`, `value` — and checks the claim against the stores rather than
//! against the program's own stdout:
//!
//! * the run persists one episode, one program and one trace per fixture, and the
//!   program each artifact names is the program the store holds;
//! * a replay in a fresh data directory reproduces the artifacts' programs, scores
//!   and traces byte-for-byte, which is what makes the corpus a benchmark rather
//!   than a recording;
//! * and a tampered artifact is refused, so the gate can fail.

use std::path::{Path, PathBuf};

use mm_core::{Params, Tabular};
use mm_e2e::{run_cli, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// The committed corpus directory.
fn corpus() -> PathBuf {
    Workspace::repo_root().join("bench/episodes")
}

/// The committed gold directory.
fn gold() -> PathBuf {
    corpus().join("gold")
}

/// The committed value table.
fn value_table() -> PathBuf {
    corpus().join("operation_value.csv")
}

/// The committed thresholds.
fn thresholds() -> PathBuf {
    corpus().join("thresholds.json")
}

/// Run the corpus into `out`, returning the artifact directory as a string.
fn run_corpus(data_dir: &Path, out: &Path) -> String {
    let input = corpus().to_string_lossy().to_string();
    let out_arg = out.to_string_lossy().to_string();
    run_cli_ok(
        data_dir,
        &["episode", "run", "--input", &input, "--out", &out_arg],
    )
}

/// Every `*.json` artifact in a directory, by file name.
fn artifacts(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    paths
}

/// One artifact's summary fields, read as JSON.
fn summary(path: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()));
    value["outcome"].clone()
}

#[tokio::test]
async fn the_corpus_runs_persists_and_replays_byte_identically() {
    let ws = Workspace::new();
    let out = ws.root().join("artifacts").join("episodes");
    let out_arg = out.to_string_lossy().to_string();
    let run = run_corpus(&ws.data_dir, &out);

    // Every difficulty band ran, which is what the budget audit needs.
    for band in ["trivial", "standard", "hard"] {
        assert!(run.contains(band), "the run skipped {band}:\n{run}");
    }
    let written = artifacts(&out);
    assert!(
        written.len() >= 9,
        "expected one artifact per committed fixture, got {}",
        written.len()
    );

    // The run persisted an episode, a program and a trace per artifact — and the
    // program the artifact names is the program the store holds, not a lookalike.
    let cfg = ws.config();
    let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    let episodes = sqlite
        .query_json("SELECT id, trace_id, status FROM episodes", Params::new())
        .await
        .unwrap();
    assert_eq!(
        episodes.len(),
        written.len(),
        "one persisted episode per artifact"
    );

    for path in &written {
        let summary = summary(path);
        let episode_id = summary["episode_id"].as_str().unwrap().to_string();
        let program_id = summary["program_id"].as_str().unwrap().to_string();
        let dag_hash = summary["dag_hash"].as_str().unwrap().to_string();
        let content_hash = summary["content_hash"].as_str().unwrap().to_string();

        let stored = sqlite
            .query_json(
                "SELECT p.dag_hash FROM programs p JOIN episodes e ON e.id = p.episode_id \
                 WHERE p.id = ? AND e.id = ?",
                vec![program_id.clone().into(), episode_id.clone().into()],
            )
            .await
            .unwrap();
        assert_eq!(
            stored.len(),
            1,
            "program {program_id} for episode {episode_id} is missing from the store"
        );
        assert_eq!(
            stored[0]["dag_hash"].as_str().unwrap(),
            dag_hash,
            "the stored DAG hash disagrees with the artifact's"
        );
        // The trace is stored against the program, so the rows a replay compares
        // are the rows the run committed.
        let trace_rows = sqlite
            .query_json(
                "SELECT count(*) AS n FROM program_traces WHERE program_id = ?",
                vec![program_id.clone().into()],
            )
            .await
            .unwrap();
        let rows = trace_rows[0]["n"].as_i64().unwrap();
        assert!(
            rows >= summary["executed"].as_i64().unwrap(),
            "the trace must hold at least the operations that ran ({rows} rows)"
        );
        assert_eq!(content_hash.len(), 64, "the content hash is sha256 hex");
    }

    // A replay against the artifacts the run wrote must be byte-identical, and it
    // must be reproducible without the store's help: a fresh data directory
    // replays the same corpus to the same bytes.
    let fresh = Workspace::new();
    let replay = run_cli_ok(
        &fresh.data_dir,
        &[
            "episode",
            "replay",
            "--all",
            "--input",
            &corpus().to_string_lossy(),
            "--compare",
            &out_arg,
        ],
    );
    assert!(
        replay.contains("replayed, diff_count 0"),
        "a replay in a fresh data directory diverged:\n{replay}"
    );

    // The rest of the gate, against the same artifacts.
    let verify = run_cli_ok(
        &ws.data_dir,
        &[
            "episode",
            "verify",
            "--artifacts",
            &out_arg,
            "--gold",
            &gold().to_string_lossy(),
        ],
    );
    assert!(
        verify.contains("0 difference(s), gold matches exactly"),
        "verify:\n{verify}"
    );

    let value = run_cli_ok(
        &ws.data_dir,
        &[
            "episode",
            "value",
            "--gold-table",
            &value_table().to_string_lossy(),
        ],
    );
    assert!(value.contains("match"), "value:\n{value}");

    let audit = run_cli_ok(
        &ws.data_dir,
        &[
            "episode",
            "budget-audit",
            "--artifacts",
            &out_arg,
            "--thresholds",
            &thresholds().to_string_lossy(),
        ],
    );
    assert!(audit.contains("0 problem(s)"), "budget audit:\n{audit}");
    assert!(audit.contains("0 over budget"), "budget audit:\n{audit}");

    // The episodes are mirrored into `/epistemic`, and the shapes accept them.
    let graph = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "epistemic"]);
    assert!(
        graph.contains("conforms: 0 violations"),
        "the episode shapes rejected a clean graph:\n{graph}"
    );

    // Every record the deliberation emitted reached the audit chain.
    let logs = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(logs.contains("logs verify passed"), "logs:\n{logs}");
}

/// `episode replay --all --compare <out>`'s stdout and status.
fn replay_against(data_dir: &Path, out: &Path) -> (bool, String) {
    let output = run_cli(
        data_dir,
        &[
            "episode",
            "replay",
            "--all",
            "--input",
            &corpus().to_string_lossy(),
            "--compare",
            &out.to_string_lossy(),
        ],
    );
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

/// `episode verify`'s status and stdout, for the same artifacts.
fn verify_against(data_dir: &Path, out: &Path) -> (bool, String) {
    let output = run_cli(
        data_dir,
        &[
            "episode",
            "verify",
            "--artifacts",
            &out.to_string_lossy(),
            "--gold",
            &gold().to_string_lossy(),
        ],
    );
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

/// Edit an artifact in place through `edit`, and return its path.
fn tamper(out: &Path, edit: impl FnOnce(&mut serde_json::Value)) -> PathBuf {
    let target = artifacts(out).into_iter().next().expect("one artifact");
    let raw = std::fs::read_to_string(&target).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    edit(&mut value);
    std::fs::write(&target, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    target
}

#[test]
fn a_tampered_program_is_refused() {
    let ws = Workspace::new();
    let out = ws.root().join("artifacts").join("episodes");
    run_corpus(&ws.data_dir, &out);

    // Move one score by hand. A replay that only checked that the program *runs*
    // would pass this; a replay that compares bytes cannot, and neither can a
    // verify that recomputes the hash the program is named by.
    tamper(&out, |value| {
        let steps = value["program"]["steps"].as_array_mut().expect("steps");
        let last = steps.len() - 1;
        let score = steps[last]["value"]["probability_change"].as_f64().unwrap();
        steps[last]["value"]["probability_change"] = serde_json::json!(score / 2.0);
    });

    let (ok, stdout) = replay_against(&ws.data_dir, &out);
    assert!(!ok, "a tampered program must fail the replay:\n{stdout}");
    assert!(
        stdout.contains("program content hash") || stdout.contains("row "),
        "the failure must name what diverged:\n{stdout}"
    );

    let (ok, stdout) = verify_against(&ws.data_dir, &out);
    assert!(!ok, "a tampered program must not pass verify:\n{stdout}");
    assert!(
        stdout.contains("hashes to"),
        "verify must say the body disagrees with its own hash:\n{stdout}"
    );
}

#[test]
fn a_tampered_trace_is_refused() {
    let ws = Workspace::new();
    let out = ws.root().join("artifacts").join("episodes");
    run_corpus(&ws.data_dir, &out);

    // The trace is half of a replay comparison, and the only place its own bytes
    // are checked: an operation's worth moved by hand is a different deliberation.
    tamper(&out, |value| {
        let rows = value["trace"]["rows"].as_array_mut().expect("trace rows");
        let last = rows.len() - 1;
        let score = rows[last]["value"]["probability_change"].as_f64().unwrap();
        rows[last]["value"]["probability_change"] = serde_json::json!(score / 2.0);
    });

    let (ok, stdout) = replay_against(&ws.data_dir, &out);
    assert!(!ok, "a tampered trace must fail the replay:\n{stdout}");
    assert!(
        stdout.contains("trace canonical") || stdout.contains("row "),
        "the failure must name the trace:\n{stdout}"
    );
}
