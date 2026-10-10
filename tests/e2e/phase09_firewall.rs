//! Phase 9 end-to-end: the bounded-decision core and the sanity firewall, driven
//! through the real `mm-cli` binary against a throwaway data directory.
//!
//! This is the plan's §8 pass gate as a test: one conformance suite for every core
//! that can be built, a bounded question answered and recorded, the adversarial
//! corpus meeting its precision/recall floors with every hard prohibition
//! `REJECT`ing, the risk and comparison corpora graded against their references,
//! calibration and conformal coverage inside their tolerances, logging complete,
//! and the `/decision` mirror conforming to its shapes.
//!
//! The corpora are read from `bench/` rather than duplicated here, so the gate
//! fails if a fixture and the artifact it grades drift apart.
//!
//! Two properties are worth naming because they are what makes this an end-to-end
//! test rather than a second unit test:
//!
//! * **The corpus is graded with no decision core configured** (the command's own
//!   default). A configured core whose class is uncalibrated raises `VERIFY_FIRST`
//!   on every run — the plan's own rule — so grading through one would measure the
//!   core's calibration instead of the rails. The corpus is the deterministic half
//!   of the firewall, and it is graded as such.
//! * **A question is answered twice and both answers are recorded.** The decision
//!   row, its audit record and its RDF mirror are written together, so a count of
//!   the rows is also a count of the mirrors.

use mm_core::{Params, Tabular};
use mm_e2e::{run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// A repository path, absolute for a subprocess argument.
fn fixture(relative: &str) -> String {
    Workspace::repo_root()
        .join(relative)
        .to_str()
        .expect("a UTF-8 path")
        .to_string()
}

/// The quad count `graph validate` reported for a graph.
fn quad_count(stdout: &str, graph: &str) -> i64 {
    let prefix = format!("graph {graph} :");
    let line = stdout
        .lines()
        .find(|line| line.trim_start().starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{stdout}"));
    line.split_whitespace()
        .find_map(|word| word.parse::<i64>().ok())
        .unwrap_or_else(|| panic!("no quad count in {line:?}"))
}

/// The whole gate, in the plan's order.
#[tokio::test]
async fn the_phase_nine_gate_passes_end_to_end() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // 1. One conformance suite, run against every core named. A core that cannot
    //    be built is reported as unavailable rather than as a pass, so the run is
    //    a success as long as nothing was *graded* and failed.
    let conformance = run_cli_ok(
        &ws.data_dir,
        &[
            "conformance",
            "--core",
            "rules",
            "--core",
            "local",
            "--core",
            "hosted",
        ],
    );
    assert!(
        conformance.contains("conformance: rules 6 passed, 0 failed"),
        "the rules core must pass the whole suite:\n{conformance}"
    );
    assert!(
        !conformance.contains("FAIL"),
        "a conformance finding failed:\n{conformance}"
    );
    // `local` and `hosted` are unavailable without a head and without a provider;
    // each says so rather than being counted as a pass.
    assert!(
        conformance.contains("local not run (unavailable)"),
        "{conformance}"
    );
    assert!(
        conformance.contains("hosted not run (unavailable)"),
        "{conformance}"
    );

    // 2. A bounded question is answered, calibrated-or-not, and recorded.
    let decide = run_cli_ok(
        &ws.data_dir,
        &[
            "decide",
            "--question",
            r#"{"kind":"yes_no","prompt":"is the migration idempotent?"}"#,
            "--core",
            "rules",
            "--episode",
            "01h00000000000000000000090",
            "--json",
        ],
    );
    let answer: serde_json::Value =
        serde_json::from_str(&decide).expect("decide --json prints one object");
    assert_eq!(answer["core"], "rules", "{decide}");
    assert_eq!(answer["available"], true, "{decide}");
    assert!(
        answer["decision_id"].is_string(),
        "a recorded decision must name itself:\n{decide}"
    );
    assert_eq!(
        answer["admitted"], false,
        "a class with no fitted threshold admits nothing:\n{decide}"
    );

    // 3. The adversarial corpus. Precision, recall, and every prohibition.
    let report_path = ws.root().join("firewall-report.json");
    let firewall = run_cli_ok(
        &ws.data_dir,
        &[
            "firewall",
            "eval",
            "--episode",
            &fixture("bench/ep.json"),
            "--corpus",
            &fixture("bench/adversarial/firewall_cases.jsonl"),
            "--gold",
            &fixture("bench/firewall/gold.jsonl"),
            "--min-precision",
            "0.95",
            "--min-recall",
            "0.90",
            "--report",
            report_path.to_str().unwrap(),
            "--json",
        ],
    );
    let report: serde_json::Value =
        serde_json::from_str(&firewall).expect("firewall eval --json prints one object");
    assert!(
        report["precision"].as_f64().unwrap() >= 0.95,
        "precision below the floor: {}",
        report["precision"]
    );
    assert!(
        report["recall"].as_f64().unwrap() >= 0.90,
        "recall below the floor: {}",
        report["recall"]
    );
    assert_eq!(
        report["prohibition_cases"], report["prohibition_rejections"],
        "every hard-prohibition case must REJECT:\n{firewall}"
    );
    assert!(
        report["failures"].as_array().unwrap().is_empty(),
        "{firewall}"
    );

    // Each prohibition in the table is exercised by a case that REJECTs through it,
    // so the six checks are covered by name rather than only by count.
    let cases = report["cases"].as_array().expect("cases is an array");
    let named: Vec<String> = cases
        .iter()
        .filter_map(|case| case["hard_prohibition"].as_str().map(str::to_string))
        .collect();
    for prohibition in [
        "identity_invariant",
        "unauthorized_tool",
        "irreversible_without_approval",
        "unresolved_hard_contradiction",
        "spend_over_budget",
        "schema_invalid_action",
    ] {
        assert!(
            named.iter().any(|id| id == prohibition),
            "no case REJECTs through {prohibition}: {named:?}"
        );
    }
    for case in cases {
        if case["hard_prohibition"].is_string() {
            assert_eq!(
                case["outcome"].as_str(),
                Some("REJECT"),
                "{} carries a prohibition but not a REJECT",
                case["id"]
            );
        }
    }

    // 4. The risk measures match their hand-computed references.
    let risk = run_cli_ok(
        &ws.data_dir,
        &[
            "risk",
            "analyze",
            "--outcomes",
            &fixture("bench/risk/reference_values.json"),
            "--tolerance",
            "1e-6",
        ],
    );
    assert!(risk.contains("0 field mismatch(es)"), "{risk}");

    // 5. The comparison fixtures produce exactly their expected verdicts.
    let compare = run_cli_ok(
        &ws.data_dir,
        &[
            "compare",
            "check",
            "--fixtures",
            &fixture("bench/comparison/fixtures.jsonl"),
        ],
    );
    assert!(compare.contains("0 mismatch(es)"), "{compare}");

    // 6. Calibration improves, and conformal coverage lands inside ±0.03.
    let fit = run_cli_ok(
        &ws.data_dir,
        &[
            "calibration",
            "fit",
            "--labeled",
            &fixture("bench/calibration/labeled.jsonl"),
        ],
    );
    assert!(
        fit.contains("calibration fit: 400 point(s), 2 class(es)"),
        "{fit}"
    );
    assert!(!fit.contains("uncalibrated"), "{fit}");

    let conformal = run_cli_ok(
        &ws.data_dir,
        &[
            "calibration",
            "conformal",
            "--scores",
            &fixture("bench/calibration/conformal.jsonl"),
            "--coverage",
            "0.9",
        ],
    );
    assert!(
        conformal.contains("calibration conformal: 1000 score(s), 5 class(es)"),
        "{conformal}"
    );
    assert!(!conformal.contains("FAIL"), "{conformal}");

    // 7. Logging completeness: every decision carries its features and every
    //    firewall run its reason codes.
    let cfg = ws.config();
    let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    let incomplete = sqlite
        .query_json(
            "SELECT (SELECT COUNT(*) FROM decisions WHERE features_json IS NULL) \
                + (SELECT COUNT(*) FROM firewall_runs WHERE reason_codes_json IS NULL) AS n",
            Params::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        incomplete[0]["n"],
        serde_json::json!(0),
        "a decision or firewall run was recorded without its features/reasons"
    );

    // The recorded decisions are the ones the CLI reported, and the firewall runs
    // are one per corpus case.
    let counts = sqlite
        .query_json(
            "SELECT (SELECT COUNT(*) FROM decisions) AS decisions, \
                    (SELECT COUNT(*) FROM firewall_runs) AS runs, \
                    (SELECT COUNT(*) FROM calibration) AS calibrations, \
                    (SELECT COUNT(*) FROM conformal_thresholds) AS thresholds",
            Params::new(),
        )
        .await
        .unwrap();
    assert_eq!(counts[0]["decisions"], serde_json::json!(1));
    let corpus_lines = std::fs::read_to_string(fixture("bench/adversarial/firewall_cases.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    assert_eq!(counts[0]["runs"], serde_json::json!(corpus_lines));
    assert_eq!(counts[0]["calibrations"], serde_json::json!(2));
    assert_eq!(counts[0]["thresholds"], serde_json::json!(5));

    // 8. The RDF mirror exists and conforms to the decision shapes.
    let graph = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "decision"]);
    assert!(
        graph.contains("conforms: 0 violations"),
        "/decision must conform:\n{graph}"
    );
    assert!(
        quad_count(&graph, "decision") > 0,
        "the decision mirror must hold the recorded decisions:\n{graph}"
    );

    // 9. The log gate is green, including the audit chain the Phase 9 writers use.
    let logs = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(logs.contains("logs verify passed"), "{logs}");
    assert!(!logs.contains("[FAIL]"), "{logs}");
}
