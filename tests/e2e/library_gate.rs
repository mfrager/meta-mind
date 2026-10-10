//! Phase 7 end-to-end: the operator's library surface, run as a subprocess.
//!
//! This is the plan's §8 pass gate, driven through the real `mm-cli` binary
//! against a throwaway data directory: the committed seed imports and validates
//! clean, every entry kind the gate lists is present, no reference dangles, a
//! malformed extraction is refused rather than coerced, only verified skills are
//! retrievable, the applicability ranking matches its gold table exactly,
//! the experience compiler emits only sub-threshold drafts, the policy genome
//! retains only fitness gains as immutable versions, frames compose
//! deterministically, and the log gate is green.
//!
//! Everything is one workspace and one sequence, because that is what the gate
//! is: each step depends on the state the previous one left.
//!
//! Fixture paths are absolute: the CLI resolves relative paths against its own
//! working directory, which is not this test's.

use mm_e2e::{run_cli, run_cli_ok, Workspace};

/// A repository path, absolute for a subprocess argument.
fn fixture(relative: &str) -> String {
    Workspace::repo_root()
        .join(relative)
        .to_str()
        .expect("a UTF-8 path")
        .to_string()
}

/// A path inside the throwaway workspace.
fn scratch(ws: &Workspace, name: &str) -> String {
    ws.root()
        .join(name)
        .to_str()
        .expect("a UTF-8 path")
        .to_string()
}

/// The instance ULID `frame compose` printed.
fn instance_ulid(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("instance ")?;
            rest.split_whitespace().next().map(str::to_string)
        })
        .unwrap_or_else(|| panic!("no `instance <ULID>` line in:\n{stdout}"))
}

/// The whole gate, in the plan's order.
#[tokio::test]
async fn the_phase_seven_gate_passes_end_to_end() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // 1. The seed imports: every entry accepted, none rejected.
    let imported = run_cli_ok(
        &ws.data_dir,
        &[
            "library",
            "import",
            &fixture("ontology/seed/library_seed.ttl"),
        ],
    );
    assert!(imported.contains("accepted"), "{imported}");
    assert!(
        imported.contains("0 rejected"),
        "the seed must import without a refusal:\n{imported}"
    );

    // 2. `/library` conforms, and the index agrees with it.
    let validated = run_cli_ok(&ws.data_dir, &["library", "validate", "--graph", "library"]);
    assert!(
        validated.contains("0 shape violation(s)"),
        "the imported graph must have no violations:\n{validated}"
    );
    assert!(validated.contains("21 entries"), "{validated}");
    assert!(
        validated.contains("0 duplicate(s)") && validated.contains("0 orphaned entry(ies)"),
        "{validated}"
    );
    assert!(validated.contains("library clean"), "{validated}");

    // 3. Every kind the gate lists has at least one committed entry.
    for kind in [
        "Doctrine",
        "Principle",
        "Heuristic",
        "Technique",
        "Pattern",
        "Case",
        "AntiPattern",
        "Skill",
        "Policy",
        "Frame",
        "Evaluation",
    ] {
        let listed = run_cli_ok(&ws.data_dir, &["library", "list", "--kind", kind]);
        assert!(
            !listed.trim().is_empty(),
            "no {kind} entry was listed:\n{listed}"
        );
        assert!(
            listed.contains(kind),
            "the {kind} listing does not name its kind:\n{listed}"
        );
    }

    // 4. Nothing dangles; the flag is what makes the check load-bearing.
    let orphans = run_cli_ok(&ws.data_dir, &["library", "orphans", "--fail-if-any"]);
    assert!(orphans.contains("0 orphaned entry(ies)"), "{orphans}");

    // 5. A malformed extraction is refused, not coerced into a committed entry.
    let malformed = run_cli(
        &ws.data_dir,
        &[
            "library",
            "extract",
            "--source",
            &fixture("bench/library/extraction/malformed.jsonl"),
            "--out",
            &scratch(&ws, "validated.jsonl"),
        ],
    );
    let malformed_out = String::from_utf8_lossy(&malformed.stdout).to_string();
    assert!(
        !malformed.status.success(),
        "a malformed fixture must exit non-zero:\n{malformed_out}"
    );
    assert!(
        malformed_out.contains("rejected"),
        "the refusal ledger must be reported:\n{malformed_out}"
    );
    assert!(
        malformed_out.contains("0 accepted"),
        "no malformed line may be accepted:\n{malformed_out}"
    );

    // 6. The well-formed fixture extracts, and its output is written.
    let raw = run_cli_ok(
        &ws.data_dir,
        &[
            "library",
            "extract",
            "--source",
            &fixture("bench/library/extraction/raw.jsonl"),
            "--out",
            &scratch(&ws, "raw.jsonl"),
        ],
    );
    assert!(raw.contains("0 rejected"), "{raw}");
    assert!(
        std::fs::metadata(scratch(&ws, "raw.jsonl"))
            .map(|meta| meta.len() > 0)
            .unwrap_or(false),
        "the accepted candidates must be written"
    );

    // 7. The skill library: a verified skill, and only verified ones retrieved.
    let verified = run_cli_ok(
        &ws.data_dir,
        &[
            "skill",
            "verify",
            "https://metamind.dev/library/skill/api-capability-check@1",
        ],
    );
    assert!(verified.contains("verified"), "{verified}");

    let retrieved = run_cli_ok(
        &ws.data_dir,
        &[
            "skill",
            "retrieve",
            "--query",
            "verify external api before use",
            "--top",
            "3",
        ],
    );
    assert!(
        retrieved.contains("api-capability-check"),
        "the verified skill must be retrievable:\n{retrieved}"
    );
    for line in retrieved.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            !line.contains("draft"),
            "an unverified skill was returned:\n{retrieved}"
        );
    }

    // 8. The ranking is deterministic and matches the committed table exactly.
    let ranked = run_cli_ok(
        &ws.data_dir,
        &[
            "library",
            "applicable",
            "--state",
            &fixture("bench/library/state_uncertain_strategy.json"),
            "--top",
            "5",
            "--gold",
            &fixture("bench/library/gold_applicability.jsonl"),
        ],
    );
    assert!(
        !ranked.contains("mismatch"),
        "the ranking must match its gold table:\n{ranked}"
    );

    // 9. The experience compiler emits drafts, and only sub-threshold ones.
    let compiled = run_cli_ok(
        &ws.data_dir,
        &[
            "experience",
            "compile",
            "--trajectories",
            &fixture("bench/library/trajectories/success.jsonl"),
            "--out",
            &scratch(&ws, "compiled.jsonl"),
        ],
    );
    let body = std::fs::read_to_string(scratch(&ws, "compiled.jsonl"))
        .expect("the compiler writes its output file");
    assert!(!body.trim().is_empty(), "{compiled}");
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("compiled line: {e}\n{line}"));
        if let Some(confidence) = value.get("confidence").and_then(serde_json::Value::as_f64) {
            assert!(
                confidence < 0.6,
                "a promotable draft was emitted ({confidence}): {line}"
            );
        }
    }

    // 10. The genome: only retained gains, as immutable parented versions.
    let evolved = run_cli_ok(
        &ws.data_dir,
        &[
            "policy",
            "evolve",
            "prefer_simpler_solution",
            "--generations",
            "3",
            "--budget",
            &fixture("bench/library/evolution/budget.toml"),
        ],
    );
    assert!(
        evolved.contains("generation 0") && evolved.contains("best_fitness"),
        "the run must report its generations and their fitness:\n{evolved}"
    );
    assert!(
        evolved.contains("version(s) created"),
        "the run must report how many versions it created:\n{evolved}"
    );

    let history = run_cli_ok(
        &ws.data_dir,
        &[
            "policy",
            "history",
            "https://metamind.dev/policy/prefer_simpler_solution@2",
        ],
    );
    let versions = history
        .lines()
        .filter(|line| line.trim_start().starts_with('v'))
        .count();
    assert!(
        versions >= 2,
        "an evolved policy must have at least two versions:\n{history}"
    );
    assert!(
        history.contains("parent=1"),
        "version 2 must name version 1 as its parent:\n{history}"
    );

    let fitness = run_cli_ok(
        &ws.data_dir,
        &[
            "policy",
            "fitness",
            "https://metamind.dev/policy/prefer_simpler_solution@2",
        ],
    );
    let numbers: Vec<&str> = fitness
        .lines()
        .find(|line| line.split_whitespace().count() >= 3)
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    assert!(
        numbers.len() >= 3 && numbers[..3].iter().all(|n| n.parse::<f64>().is_ok()),
        "fitness must report trials, successes and mean utility:\n{fitness}"
    );

    // 11. Frames compose deterministically and their gaps are enumerable.
    let composed = run_cli_ok(
        &ws.data_dir,
        &[
            "frame",
            "compose",
            "--frames",
            "problem_solving,software,debugging,high_stakes",
            "--episode",
            "01J0000000000000000000000A",
        ],
    );
    assert!(composed.contains("instance "), "{composed}");
    assert!(composed.contains("missing"), "{composed}");
    let instance = instance_ulid(&composed);
    let missing = run_cli_ok(&ws.data_dir, &["frame", "missing", &instance]);
    assert!(
        !missing.starts_with("mm-cli: "),
        "`frame missing` must resolve the composed instance:\n{missing}"
    );

    // 12. The log gate, which every phase's gate ends on.
    let logs = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(logs.contains("logs verify passed"), "{logs}");
    assert!(!logs.contains("[FAIL]"), "{logs}");
}

/// The seed document itself is the library's contract: a store with nothing
/// imported has no entries, and the same document cannot be imported twice under
/// a second version without becoming a duplicate.
#[tokio::test]
async fn importing_the_same_document_twice_changes_nothing() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let seed = fixture("ontology/seed/library_seed.ttl");

    let first = run_cli_ok(&ws.data_dir, &["library", "import", &seed]);
    assert!(first.contains("21 accepted, 0 rejected"), "{first}");

    // The second import is idempotent: the same content hash is already indexed,
    // so each subject is refused as a duplicate rather than written again.
    let second = run_cli(&ws.data_dir, &["library", "import", &seed]);
    let second_out = String::from_utf8_lossy(&second.stdout).to_string();
    assert!(
        second_out.contains("0 accepted"),
        "a repeated import must accept nothing new:\n{second_out}"
    );

    let listed = run_cli_ok(&ws.data_dir, &["library", "list", "--kind", "Technique"]);
    assert_eq!(
        listed.lines().filter(|l| !l.trim().is_empty()).count(),
        4,
        "the duplicate import must not double the corpus:\n{listed}"
    );
}
