//! End-to-end CLI behaviour: the commands an operator runs, including the ones
//! that must *fail*.

use mm_e2e::{run_cli, run_cli_ok, Workspace};
use mm_store_graph::GraphStore;

/// Insert a fixture graph into a named graph, then close the store so the CLI can
/// open it.
async fn seed_graph(ws: &Workspace, graph: &str, turtle: &str) {
    let cfg = ws.config();
    let store = GraphStore::open(&cfg.store.graph_dir, &cfg.shapes_file())
        .await
        .unwrap();
    let inserted = store.handle().insert_turtle(graph, turtle).await.unwrap();
    assert!(inserted > 0, "the fixture must contain triples");
    store.handle().flush().await.unwrap();
    store.shutdown().await.unwrap();
    drop(store);
}

#[tokio::test]
async fn an_empty_being_graph_conforms() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let out = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "being"]);
    assert!(out.contains("conforms: 0 violations"), "{out}");

    // A clean fixture also conforms.
    seed_graph(&ws, "being", &Workspace::fixture("ontology/clean.ttl")).await;
    let out = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "being"]);
    assert!(out.contains("conforms: 0 violations"), "{out}");
    assert!(
        out.contains("3 quad(s)") || out.contains("quad(s)"),
        "{out}"
    );
}

#[tokio::test]
async fn a_broken_being_graph_fails_the_gate_with_named_violations() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    seed_graph(&ws, "being", &Workspace::fixture("ontology/broken.ttl")).await;

    let output = run_cli(&ws.data_dir, &["graph", "validate", "--graph", "being"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        !output.status.success(),
        "validation of a broken graph must exit non-zero:\n{stdout}"
    );
    assert!(stdout.contains("does not conform"), "{stdout}");
    assert!(stdout.contains("violation"), "{stdout}");
    // Both planted defects are reported, with the offending path named.
    assert!(stdout.contains("iri"), "{stdout}");
    assert!(stdout.contains("created"), "{stdout}");
}

#[tokio::test]
async fn an_unknown_graph_name_is_rejected() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let output = run_cli(
        &ws.data_dir,
        &["graph", "validate", "--graph", "not_a_graph"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(stderr.contains("unknown graph"), "{stderr}");
}

#[tokio::test]
async fn logs_verify_passes_and_reports_every_check() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let out = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    for check in [
        "audit chain",
        "audit completeness",
        "event sequence",
        "record schema",
        "redaction",
        "replay determinism",
        "llm accounting",
        "being correlation",
        "memory correlation",
    ] {
        assert!(out.contains(check), "missing check {check:?} in:\n{out}");
    }
    assert!(out.contains("logs verify passed"), "{out}");
    assert!(!out.contains("[FAIL]"), "{out}");
}

#[tokio::test]
async fn logs_tail_and_trace_expose_the_streams() {
    let ws = Workspace::new();
    let doctor = run_cli_ok(&ws.data_dir, &["doctor"]);

    let tail = run_cli_ok(&ws.data_dir, &["logs", "tail", "-n", "3"]);
    let lines: Vec<&str> = tail.lines().filter(|l| l.starts_with('{')).collect();
    assert!(!lines.is_empty() && lines.len() <= 3, "{tail}");
    assert!(lines.iter().all(|l| l.contains("\"event_code\"")));

    // A trace id from the doctor run addresses records that mention it.
    let trace = doctor
        .lines()
        .find_map(|l| l.split("trace ").nth(1).map(str::trim))
        .map(str::to_string);
    if let Some(trace) = trace {
        let out = run_cli(&ws.data_dir, &["logs", "trace", &trace]);
        assert!(out.status.success(), "trace {trace} should resolve");
    }

    // An unknown trace resolves to nothing and says so with a non-zero exit.
    // The ULID is syntactically valid (26 lowercase Crockford chars) but names
    // no record the kernel ever wrote.
    let missing = run_cli(
        &ws.data_dir,
        &["logs", "trace", "01h00000000000000000000000"],
    );
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stdout).contains("0 record(s)"));
}

#[tokio::test]
async fn the_cli_refuses_a_missing_configuration() {
    let output = std::process::Command::new(mm_e2e::mm_cli())
        .args(["--config", "/nonexistent/metamind.toml", "doctor"])
        .output()
        .expect("spawn mm-cli");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(stderr.contains("config"), "{stderr}");
}
