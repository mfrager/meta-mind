//! Phase 6 end-to-end: the operator's `epistemic` surface, run as a subprocess.
//!
//! This is the plan's §8 pass gate, driven through the real `mm-cli` binary
//! against a throwaway data directory: no forbidden promotion is allowed, one
//! contradiction per seeded pair, the cascade equals the golden expectation,
//! `/world` holds only `OBSERVED`/`VERIFIED`, the SHACL shapes conform, the log
//! gate is green, and replay stays deterministic.
//!
//! `bench/epistemic/cascade/{graph.json,expected.json}` is the golden file the
//! plan names, and it is read here rather than duplicated, so the test fails if
//! the fixture and the computed cascade drift apart.

use mm_e2e::{run_cli_ok, value_after, Workspace};

/// A repository path, absolute for a subprocess argument.
fn fixture(relative: &str) -> String {
    Workspace::repo_root()
        .join(relative)
        .to_str()
        .expect("a UTF-8 path")
        .to_string()
}

/// The affected ULIDs the golden cascade file expects, in order.
fn expected_affected() -> Vec<String> {
    let raw = std::fs::read_to_string(fixture("bench/epistemic/cascade/expected.json"))
        .expect("the golden cascade file");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("golden cascade is JSON");
    value["affected"]
        .as_array()
        .expect("affected is an array")
        .iter()
        .map(|v| v.as_str().expect("a ULID").to_string())
        .collect()
}

/// The root the cascade fixture declares.
fn cascade_root() -> String {
    let raw = std::fs::read_to_string(fixture("bench/epistemic/cascade/graph.json"))
        .expect("the cascade fixture");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("cascade fixture is JSON");
    value["root"].as_str().expect("a root ULID").to_string()
}

/// The `affected` lines a command printed, in order.
fn affected_lines(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| line.trim().strip_prefix("affected: "))
        .map(str::to_string)
        .collect()
}

/// The whole gate, in the plan's order.
#[tokio::test]
async fn the_phase_six_gate_passes_end_to_end() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // 1. The deterministic table allows none of the forbidden transitions.
    let promotable = run_cli_ok(
        &ws.data_dir,
        &[
            "epistemic",
            "promotable",
            "--fixture",
            &fixture("bench/epistemic/promotion_guard/forbidden.jsonl"),
        ],
    );
    assert!(
        promotable.contains("allowed=0"),
        "every forbidden transition must be refused:\n{promotable}"
    );
    assert!(
        !promotable.contains("ALLOWED"),
        "one promotion slipped through:\n{promotable}"
    );

    // 2. One contradiction per seeded pair, and the observations reach `/world`.
    let contradictions = run_cli_ok(
        &ws.data_dir,
        &[
            "epistemic",
            "contradictions",
            "--fixture",
            &fixture("bench/adversarial/contradictions"),
        ],
    );
    assert!(
        contradictions.contains("created=3") && contradictions.contains("seeded=3"),
        "one record per seeded pair:\n{contradictions}"
    );

    // 3. The dependency-directed retraction equals the golden expectation.
    let root = cascade_root();
    let expected = expected_affected();

    // `dependency cascade` names the dependents, `invalidate` reports the whole
    // retracted set including the root — which is what the golden file lists.
    let cascade = run_cli_ok(
        &ws.data_dir,
        &["epistemic", "dependency", "cascade", "--root", &root],
    );
    assert_eq!(
        affected_lines(&cascade),
        expected[1..].to_vec(),
        "{cascade}"
    );

    let invalidated = run_cli_ok(
        &ws.data_dir,
        &[
            "epistemic",
            "invalidate",
            &root,
            "--reason",
            "source retracted",
        ],
    );
    assert_eq!(affected_lines(&invalidated), expected, "{invalidated}");
    assert!(
        invalidated.contains(&format!("affected_count={}", expected.len())),
        "{invalidated}"
    );

    // 4. The data-quality suite finds nothing to report on a clean state.
    let tests = run_cli_ok(
        &ws.data_dir,
        &["epistemic", "data-tests", "--graph", "epistemic"],
    );
    assert!(
        tests.contains("violations=0"),
        "a clean state has no violations:\n{tests}"
    );

    // 5. `/world` holds only OBSERVED/VERIFIED, asked with the plan's own query.
    let world = run_cli_ok(
        &ws.data_dir,
        &[
            "epistemic",
            "world",
            "--query",
            "SELECT ?s WHERE { GRAPH <https://metamind.dev/graph/world> \
             { ?s <https://metamind.dev/ontology#status> ?st } }",
        ],
    );
    for line in world
        .lines()
        .filter_map(|l| l.trim().strip_prefix("status "))
    {
        assert!(
            line == "OBSERVED" || line == "VERIFIED",
            "a non-admissible status reached /world: {line}\n{world}"
        );
    }

    // 6. The shapes conform for both graphs they govern.
    for graph in ["epistemic", "world"] {
        let shaped = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", graph]);
        assert!(
            shaped.contains("conforms: 0 violations"),
            "/{graph} must conform:\n{shaped}"
        );
    }

    // 7. The log gate is green, including the epistemic correlation check.
    let logs = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(logs.contains("epistemic correlation"), "{logs}");
    assert!(logs.contains("logs verify passed"), "{logs}");
    assert!(!logs.contains("[FAIL]"), "{logs}");

    // 8. Replay is deterministic across processes. (The plan writes this as
    //    `replay --check`; the CLI's `replay` prints the same whole-log
    //    `state_hash` and checks the stored checkpoint, which is the property.)
    let first = run_cli_ok(&ws.data_dir, &["replay", "--from", "0"]);
    let second = run_cli_ok(&ws.data_dir, &["replay", "--from", "0"]);
    let h1 = value_after(&first, "state_hash").expect("replay reports state_hash");
    let h2 = value_after(&second, "state_hash").expect("replay reports state_hash");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64, "the state hash is sha256 hex");
}

/// A world claim is reachable only through the barrier, so an ingest that offers
/// no observation evidence leaves `/world` untouched rather than admitting it.
#[tokio::test]
async fn a_report_never_reaches_the_world_graph() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // `conflicts.jsonl` is all REPORTED claims with no observation evidence.
    run_cli_ok(
        &ws.data_dir,
        &[
            "epistemic",
            "ingest",
            "--file",
            &fixture("bench/adversarial/contradictions/conflicts.jsonl"),
        ],
    );

    let world = run_cli_ok(
        &ws.data_dir,
        &[
            "epistemic",
            "world",
            "--query",
            "SELECT ?s WHERE { GRAPH <https://metamind.dev/graph/world> \
             { ?s <https://metamind.dev/ontology#status> ?st } }",
        ],
    );
    assert!(
        world.contains("rows=0"),
        "a report must not reach /world:\n{world}"
    );
}
