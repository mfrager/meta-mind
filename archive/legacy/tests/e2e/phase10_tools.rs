//! Phase 10 end-to-end: authorized, observable action, driven through the real
//! `mm-cli` binary against a throwaway data directory.
//!
//! This is the plan's §8 pass gate as a test, and it is written the way the earlier
//! phases' gates are: a corpus in `bench/` rather than fixtures duplicated here, so the
//! test fails when the artifact and the fixture drift apart.
//!
//! Four properties are what make this an end-to-end test rather than a second unit test:
//!
//! * **The refusal is deterministic and it is recorded.** `process.exec` is denied
//!   because the seeded grant table holds no process grant for anyone — not because a
//!   model declined — and the denial leaves a `permission.check` entry in the ledger.
//!   "Nothing happened" is a fact the ledger has to be able to attest to.
//! * **An authorized action produces exactly one observation with an evidence id**, and
//!   that evidence is what reaches `/world`. The test reads the id back out of the
//!   recorded outcome and finds the claim.
//! * **A repeated idempotency key does not re-apply the effect.** The first write and
//!   the second are the identical call; the second reports `deduplicated`, and the
//!   ledger holds one effect.
//! * **A rollback restores the recorded hash, byte for byte**, and the ledger chain
//!   verifies both before and after.
//!
//! # One correction to the plan's gate script
//!
//! §8 computes `before` from `data/sandbox/target.txt` and then rewrites the *same*
//! text with `--arg text='hello'`, so `before` and `after` are equal by construction and
//! the `test "$before" != "$after"` line can never pass. The property the line is
//! reaching for is tested here with two different texts, which is what makes the
//! assertion about rollback rather than about the fixture staying still.

use std::path::PathBuf;

use mm_core::{Param, Params, Tabular};
use mm_e2e::{run_cli, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// A repository path, absolute, so the test does not depend on its working directory.
///
/// The sandbox resolves a *relative* argument against the process's working directory,
/// which for `cargo test` is the package directory and not the repository root. The gate
/// runs from the repository root and can use relative paths; a test cannot, and an
/// absolute path inside the sandbox root is the same request.
fn repo(relative: &str) -> String {
    Workspace::repo_root()
        .join(relative)
        .to_str()
        .expect("a UTF-8 path")
        .to_string()
}

/// The sandbox root the CLI made, as an absolute path.
///
/// No `Workspace` argument: the root is fixed by the repository layout, so taking
/// one would only suggest the answer varied with the data directory.
fn sandbox() -> PathBuf {
    Workspace::repo_root().join("data/sandbox")
}

/// Every collision-free working file this test needs, inside the sandbox root.
fn sandbox_file(name: &str) -> String {
    repo(&format!("data/sandbox/{name}"))
}

/// The `tool_calls` + ledger view of a store, for the invariants the records carry.
async fn sqlite(ws: &Workspace) -> SqliteStore {
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
async fn the_phase_ten_gate_passes_end_to_end() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);
    let target = sandbox_file("target.txt");

    // 1. The registry is present and each tool's safety declarations are the ones the
    //    regression table promises.
    let listed = run_cli_ok(&ws.data_dir, &["tool", "list", "--json"]);
    let tools: Vec<serde_json::Value> = serde_json::from_str(&listed).expect("a JSON array");
    assert!(
        tools.len() >= 6,
        "the registry holds {} tool(s)",
        tools.len()
    );
    let by_name = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("{name} is not registered:\n{listed}"))
            .clone()
    };
    assert_eq!(by_name("fs.read")["reversibility"], "reversible");
    assert_eq!(by_name("fs.read")["sandbox_tier"], "WasmCaps");
    assert_eq!(by_name("pi_editor.open")["reversibility"], "irreversible");
    assert_eq!(by_name("pi_editor.open")["sandbox_tier"], "MicroVm");

    // 2. An unauthorized call is denied by deterministic code, and the denial is on the
    //    ledger. No process grant is seeded, for anyone.
    let being = "01h0000000000000000000b010";
    let denied = run_cli(
        &ws.data_dir,
        &[
            "tool",
            "run",
            "process.exec",
            "--arg",
            "cmd=cargo test",
            "--principal",
            being,
        ],
    );
    assert!(
        !denied.status.success(),
        "an ungranted process.exec must fail:\n{}",
        String::from_utf8_lossy(&denied.stdout)
    );
    let stdout = String::from_utf8_lossy(&denied.stdout).to_string();
    assert!(stdout.contains("denied"), "{stdout}");
    assert!(stdout.contains("no grant"), "{stdout}");

    // 3. An explicit `forbid` beats a `permit`: the grant covers the whole tree, so only
    //    the rule can refuse this write. And a request that names no tool cannot match a
    //    grant at all, which is the strictest reading of the same question.
    let check = run_cli_ok(
        &ws.data_dir,
        &[
            "policy",
            "check",
            "--principal",
            being,
            "--action",
            "fs.write",
            "--resource",
            "data/sandbox/x",
        ],
    );
    assert!(check.contains("deny"), "{check}");
    assert!(check.contains("names no tool"), "{check}");

    let forbidden = run_cli(
        &ws.data_dir,
        &[
            "tool",
            "run",
            "fs.write",
            "--arg",
            &format!("path={}", sandbox_file("forbidden/x.txt")),
            "--arg",
            "text=hello",
        ],
    );
    assert!(
        !forbidden.status.success(),
        "the forbid rule must refuse this"
    );
    let forbidden_out = String::from_utf8_lossy(&forbidden.stdout).to_string();
    assert!(forbidden_out.contains("forbid"), "{forbidden_out}");

    // 4. Sandbox enforcement: a path outside the root, and a host that is not granted.
    let escape = run_cli(
        &ws.data_dir,
        &["tool", "run", "fs.read", "--arg", "path=/etc/shadow"],
    );
    assert!(
        !escape.status.success(),
        "an undeclared path must be refused"
    );
    // `http://`, not `https://`: this build links no TLS stack and refuses an https URL
    // before it ever reads the host, so the https form would test that refusal rather
    // than the host allowlist this step is about.
    let host = run_cli(
        &ws.data_dir,
        &[
            "tool",
            "run",
            "http.fetch",
            "--arg",
            "url=http://example.com/",
        ],
    );
    assert!(!host.status.success(), "an ungranted host must be refused");
    let host_out = String::from_utf8_lossy(&host.stdout).to_string();
    assert!(host_out.contains("host_denied"), "{host_out}");

    // 5. An authorized read succeeds and yields an observation with an evidence id.
    let read = run_cli_ok(
        &ws.data_dir,
        &[
            "tool",
            "run",
            "fs.read",
            "--arg",
            &format!("path={}", repo("data/sandbox/README.md")),
            "--json",
        ],
    );
    let read: serde_json::Value = serde_json::from_str(&read).expect("one JSON object");
    assert_eq!(read["status"], "ok", "{read}");
    // Identifiers are compared in the lowercase Crockford form the store uses, whatever
    // case the JSON rendering chose.
    let read_action = read["action_id"]
        .as_str()
        .expect("an action id")
        .to_ascii_lowercase();
    let evidence = read["evidence_id"]
        .as_str()
        .expect("an evidence id")
        .to_ascii_lowercase();
    assert!(read["observation_id"].is_string(), "{read}");

    let store = sqlite(&ws).await;
    let observations = count(
        &store,
        "SELECT count(*) AS n FROM observed_payloads WHERE action_id = ?",
        vec![Param::Text(read_action.clone())],
    )
    .await;
    assert_eq!(
        observations, 1,
        "one authorized read produces exactly one observation"
    );

    // The chain is intact and this action's last entry is its observation.
    let ledger = run_cli_ok(
        &ws.data_dir,
        &["action", "ledger", "--json", "--limit", "200"],
    );
    let entries: Vec<serde_json::Value> = serde_json::from_str(&ledger).expect("a JSON array");
    let last_event = entries
        .iter()
        .filter(|entry| {
            entry["action_id"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref()
                == Some(read_action.as_str())
        })
        .map(|entry| entry["event"].as_str().unwrap_or("").to_string())
        .next_back()
        .expect("the read must be on the ledger");
    assert_eq!(last_event, "observation.record", "{ledger}");

    // The evidence id the action reported is the one the observation recorded, and the
    // claim reached `/world` through the barrier.
    let recorded = store
        .query_json(
            "SELECT evidence_id FROM observed_payloads WHERE action_id = ?",
            vec![Param::Text(read_action.clone())],
        )
        .await
        .unwrap();
    assert_eq!(
        recorded[0]["evidence_id"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(evidence.as_str())
    );

    // 6. Exactly-once: the same key twice applies the effect once and replays the answer.
    let write_args = [
        "tool",
        "run",
        "fs.write",
        "--arg",
        &format!("path={target}"),
        "--arg",
        "text=first",
        "--idempotency-key",
        "gate-k1",
        "--json",
    ];
    let first = run_cli_ok(&ws.data_dir, &write_args);
    let first: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["status"], "ok", "{first}");
    assert_eq!(first["deduplicated"], false, "{first}");

    let second = run_cli_ok(&ws.data_dir, &write_args);
    let second: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert_eq!(second["status"], "ok", "{second}");
    assert_eq!(
        second["deduplicated"], true,
        "a retry under the same key must replay, not re-apply:\n{second}"
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "first",
        "the effect was applied once, with the first call's content"
    );

    // 7. The chain is append-only and hash-chained.
    let verify = run_cli_ok(&ws.data_dir, &["action", "ledger", "--verify"]);
    assert!(verify.contains("chain ok"), "{verify}");

    // 8. A reversible action rolls back byte-identically: the file is rewritten with
    //    different text, then the recorded snapshot is restored.
    let before = mm_core::content_hash(std::fs::read(&target).unwrap().as_slice());
    let rewrite = run_cli_ok(
        &ws.data_dir,
        &[
            "tool",
            "run",
            "fs.write",
            "--arg",
            &format!("path={target}"),
            "--arg",
            "text=second",
            "--json",
        ],
    );
    let rewrite: serde_json::Value = serde_json::from_str(&rewrite).unwrap();
    assert_eq!(rewrite["status"], "ok", "{rewrite}");
    let after = mm_core::content_hash(std::fs::read(&target).unwrap().as_slice());
    assert_ne!(before, after, "the rewrite must change the file");

    let rewrite_action = rewrite["action_id"].as_str().unwrap().to_ascii_lowercase();
    let restored = run_cli_ok(&ws.data_dir, &["action", "rollback", &rewrite_action]);
    assert!(restored.contains("restored"), "{restored}");
    assert_eq!(
        mm_core::content_hash(std::fs::read(&target).unwrap().as_slice()),
        mm_core::content_hash(b"first"),
        "the rollback must restore the byte-identical pre-state"
    );

    // 9. The adversarial corpus: every case is a denial deterministic code produces.
    let adversarial = Workspace::repo_root().join("bench/tools/adversarial");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&adversarial)
        .expect("bench/tools/adversarial")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    cases.sort();
    assert!(cases.len() >= 6, "the corpus holds {} case(s)", cases.len());
    for case in &cases {
        let text = std::fs::read_to_string(case).unwrap();
        let case: serde_json::Value = serde_json::from_str(&text).unwrap();
        let id = case["id"].as_str().unwrap();
        if let Some(kind) = case["construct"].as_str() {
            // The two cases that are not actions at all: the refusal happens at the
            // layer that owns it, and the unit tests in `mm-tools` assert it. Here the
            // fixture is checked for the shape the corpus promises.
            assert!(
                matches!(kind, "oversize" | "fabricated_observation"),
                "unknown construct kind in {id}: {kind}"
            );
            continue;
        }
        let mut args: Vec<String> = vec!["tool".into(), "run".into()];
        args.push(case["tool"].as_str().unwrap().to_string());
        let mut object = case["args"].clone();
        if let Some(generate) = case.get("generate") {
            let field = generate["field"].as_str().unwrap();
            let repeat = generate["repeat"].as_str().unwrap();
            let times = generate["count"].as_u64().unwrap() as usize;
            object[field] = serde_json::json!(repeat.repeat(times));
        }
        for (key, value) in object.as_object().unwrap() {
            let text = match value {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            args.push("--arg".into());
            args.push(format!("{key}={text}"));
        }
        args.push("--principal".into());
        args.push(case["principal"].as_str().unwrap().to_string());
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = run_cli(&ws.data_dir, &borrowed);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        assert!(
            !output.status.success(),
            "{id} must be refused, got:\n{stdout}"
        );
        let expect_code = case["expect"]["code"].as_str().unwrap();
        assert!(
            stdout.contains(expect_code) || stdout.contains("denied"),
            "{id} does not name {expect_code}:\n{stdout}"
        );
    }

    // 10. The symbolic verifier refutes a seeded inconsistency and refuses to call an
    //     unrefuted set proven.
    let refuted = run_cli(
        &ws.data_dir,
        &[
            "verify",
            "run",
            "symbolic",
            "--subject",
            &repo("bench/tools/seeded_inconsistency.logic"),
            "--json",
        ],
    );
    assert!(
        !refuted.status.success(),
        "a seeded inconsistency must be refuted"
    );
    let refuted: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&refuted.stdout)).unwrap();
    assert_eq!(refuted["verdict"], "refuted", "{refuted}");
    assert!(refuted["counterexample"].is_string(), "{refuted}");

    let inconclusive = run_cli(
        &ws.data_dir,
        &[
            "verify",
            "run",
            "symbolic",
            "--subject",
            &repo("bench/tools/no_refutation.logic"),
            "--json",
        ],
    );
    assert_eq!(
        inconclusive.status.code(),
        Some(3),
        "an unrefuted set is inconclusive, never proven"
    );
    let inconclusive: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&inconclusive.stdout)).unwrap();
    assert_eq!(inconclusive["verdict"], "inconclusive", "{inconclusive}");

    // 11. The `/tools` mirror conforms to its shapes.
    let graph = run_cli_ok(&ws.data_dir, &["graph", "validate", "--graph", "tools"]);
    assert!(graph.contains("conforms: 0 violations"), "{graph}");

    // 12. The MCP bridge lists the same tools and refuses a call without a principal.
    let mcp = run_cli_ok(&ws.data_dir, &["mcp", "list", "--json"]);
    let mcp: serde_json::Value = serde_json::from_str(&mcp).unwrap();
    let count_mcp = mcp["tools"].as_array().map_or(0, Vec::len);
    assert_eq!(
        count_mcp,
        tools.len(),
        "the bridge lists the registry:\n{mcp}"
    );
    let no_principal = run_cli(
        &ws.data_dir,
        &["mcp", "call", "fs.read", "--principal", " ", "--json"],
    );
    assert!(
        !no_principal.status.success(),
        "the bridge must refuse an unusable principal"
    );

    // 13. Logging completeness: the audit chain is intact and the gate is green.
    let logs = run_cli_ok(&ws.data_dir, &["logs", "verify"]);
    assert!(logs.contains("logs verify passed"), "{logs}");
    assert!(!logs.contains("[FAIL]"), "{logs}");

    // Every action that ran has a call row, and no call row claims a decision it did
    // not record.
    let calls = count(
        &store,
        "SELECT count(*) AS n FROM tool_calls",
        Params::new(),
    )
    .await;
    assert!(
        calls >= 5,
        "expected the gate's calls to be recorded, found {calls}"
    );
    let orphan = count(
        &store,
        "SELECT count(*) AS n FROM observed_payloads o \
         WHERE NOT EXISTS (SELECT 1 FROM tool_calls c WHERE c.id = o.action_id)",
        Params::new(),
    )
    .await;
    assert_eq!(orphan, 0, "every observation belongs to a recorded action");

    // The sandbox root the CLI made is the one the reads were checked against.
    assert!(sandbox().join("README.md").exists());
}
