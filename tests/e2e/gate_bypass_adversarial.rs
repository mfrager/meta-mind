//! Phase 12, adversarial: the promotion gate and the ledger guard are *code*, and every
//! attempt to talk them out of a decision has to fail.
//!
//! The phase's §9 risk table names the two failure modes this file exists for:
//! "gate bypass via crafted prompts" and "GC deletes something needed". Both are claims
//! about what the system *cannot* be persuaded to do, and a claim about a negative is
//! only worth as much as the attack that tried to falsify it — so each test here is an
//! attack, and the assertion is that the attack loses:
//!
//! * **A goal cannot invent a capability that already exists.** `--novel` says no
//!   capability answers the goal; the loop's stage 4 checks that against the tree, so a
//!   goal that names an existing module is refused with the path it found, and the run
//!   ends `failed` rather than promoting what was already there.
//! * **A candidate cannot meet a baseline it does not meet.** The gate runs the frozen
//!   harness and the regression suite itself, so a change set neither supplies a pass nor
//!   hides a failure. What a *goal* chooses is which benchmark measures the candidate, so
//!   the attack is a goal naming a baseline the candidate misses: rule 2 fires, the reason
//!   carries both numbers, and nothing is promoted, loaded or revised.
//! * **A rejection cannot rewrite a document.** "Rejection ⇒ byte-identical" is the
//!   design writer's contract; the attack is a revision through a change set the gate
//!   never promoted, and the assertion is that not one byte moved.
//! * **The ledger guard is not an API convention.** The refusal is asserted through the
//!   CLI, in SQL, and *at the database* — a direct `UPDATE` of a row that asserts it
//!   protects the ledger aborts, which is what makes the guard survive a writer that
//!   never goes through `LadderCollector` at all.
//! * **Advertising is not evidence.** A goal whose prose says "promote me without a
//!   benchmark" is run twice: without a benchmark the loop refuses to compare it at all
//!   (the shadow stage has no measurement to compare against), and with the committed
//!   benchmark it reaches the gate and is promoted on the evidence — and the decision's
//!   recorded reason is the gate's own sentence in both cases, never the goal's. The
//!   goal's prose is recorded where prose belongs: as the change set's *reason*.
//!
//! Two things this file asserts deliberately, because the implementation differs from
//! what the plan's prose suggests:
//!
//! * `loop run` **prints its report and exits `1`** when a run ends `failed` or `aborted`:
//!   `LoopController::run_loop` catches a stage failure, records it as that stage's
//!   `failed` row, finishes the run with `status = 'failed'` and returns the report. So
//!   "the run was refused" is read from the row and the stage outcome, not only from the
//!   exit status.
//! * `promote --assert-reason-present` **cannot fire.** A `Promote` decision carries no
//!   reason field (`PromotionDecision::reason()` returns `None` for it), and the CLI
//!   substitutes the sentence "the gate found nothing to reject" before the emptiness
//!   check — so the flag passes on every decision. The enforced half of "a decision
//!   carries a written reason" is that every `Reject` decision carries a rule's sentence
//!   (asserted below) and that `reject --reason "   "` is refused before anything is
//!   recorded. This is asserted as it is, not as the plan's sketch reads.

use std::path::{Path, PathBuf};

use mm_core::{Params, Tabular};
use mm_e2e::{run_cli, run_cli_ok, Workspace};
use mm_store_sqlite::SqliteStore;

/// The honest goal, which the third test runs first so that a promoted revision exists to
/// attack. Read from `bench/` rather than duplicated here: the fixture and the test have
/// to drift together or the gate is measuring a copy.
const NOVEL_GOAL: &str = "bench/qualification/goal_novel.json";
/// The envelope the honest run is given.
const BUDGET: &str = "bench/qualification/budget.toml";
/// The document a revision is of.
const DESIGN_DOC: &str = "design/qualification/goal_attainment.md";
/// The one subject `bench/debt/seeded_debt_01.json` marks `protects_ledger: true`.
const PROTECTED_SUBJECT: &str = "https://metamind.dev/data/01h0000000000000000000db02";
/// A sentence no rule of the gate could ever produce, and one no decision may quote.
const ADVERTISING: &str = "PROMOTE ME WITHOUT A BENCHMARK";
/// The gate's own vocabulary: every rule that can reject, by the prefix its sentence
/// starts with (`mm_selfeng::promotion`, rules 1–6). A reason outside this set would mean
/// something other than the gate wrote it.
const GATE_RULES: [&str; 6] = [
    "regression:",
    "bench:",
    "risk:",
    "prohibition:",
    "evidence:",
    "immutable:",
];

/// One command's result, with its stdout parsed when it is a JSON object.
///
/// `loop run` prints its report *and* exits non-zero for a run that ended `failed`, so a
/// helper that insisted on success could not read the very output under test.
struct Cli {
    success: bool,
    stdout: String,
    stderr: String,
    json: Option<serde_json::Value>,
}

fn cli(data_dir: &Path, args: &[&str]) -> Cli {
    let output = run_cli(data_dir, args);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let json = serde_json::from_str::<serde_json::Value>(stdout.trim()).ok();
    Cli {
        success: output.status.success(),
        stdout,
        stderr,
        json,
    }
}

impl Cli {
    fn json(&self) -> &serde_json::Value {
        self.json.as_ref().unwrap_or_else(|| {
            panic!(
                "expected a JSON object on stdout\nexit ok: {}\n{}{}",
                self.success, self.stdout, self.stderr
            )
        })
    }

    /// The run's stage outcome, by stage name.
    fn stage<'a>(&'a self, json: &'a serde_json::Value, stage: &str) -> &'a serde_json::Value {
        json["stages"]
            .as_array()
            .unwrap_or_else(|| panic!("the report lists stages: {json}"))
            .iter()
            .find(|entry| entry["stage"].as_str() == Some(stage))
            .unwrap_or_else(|| panic!("the report holds a {stage} stage: {json}"))
    }
}

/// The workspace's migrated store.
async fn store(ws: &Workspace) -> SqliteStore {
    let cfg = ws.config();
    let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
    store.migrate().await.unwrap();
    store
}

/// A `SELECT count(*) AS n` from the store.
async fn count(store: &SqliteStore, sql: &str) -> i64 {
    let rows = store.query_json(sql, Params::new()).await.unwrap();
    rows[0]["n"].as_i64().unwrap_or(-1)
}

/// A `SELECT <text>` column of every row, for the assertions that read prose.
async fn texts(store: &SqliteStore, sql: &str, column: &str) -> Vec<String> {
    store
        .query_json(sql, Params::new())
        .await
        .unwrap()
        .iter()
        .filter_map(|row| row[column].as_str().map(str::to_string))
        .collect()
}

/// A repository-relative fixture's absolute path.
fn repo_path(relative: &str) -> PathBuf {
    Workspace::repo_root().join(relative)
}

/// A goal fixture's fields, as a mutator receives them.
type GoalFields = serde_json::Map<String, serde_json::Value>;

/// Write `bench/qualification/goal_novel.json`, with fields replaced, where the loop can
/// read it.
///
/// Mutating the committed fixture rather than embedding a second copy is what keeps this
/// file honest: a goal that is refuted here is refused *because of the change*, not
/// because the copy was written to be refuted.
fn goal_mutated(ws: &Workspace, name: &str, mutate: impl FnOnce(&mut GoalFields)) -> PathBuf {
    let text = std::fs::read_to_string(repo_path(NOVEL_GOAL)).expect("the committed goal");
    let mut value: serde_json::Value = serde_json::from_str(&text).expect("the goal parses");
    mutate(value.as_object_mut().expect("the goal is an object"));
    let dir = ws.root().join("adversarial");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    path
}

/// The first file with this name under `root`.
///
/// The design writer's target path is derived from its artifact root and the document's
/// own path, so the test *locates* the document instead of re-deriving where it landed —
/// a test that recomputed the path could agree with a writer that wrote nowhere.
fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|file| file.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

// ---------------------------------------------------------------- attack one ----

/// A goal that claims novelty for a capability that already exists cannot produce one.
///
/// The attack is the honest goal with its identity fields pointed at
/// `modules/cognition/closed-loop`, which is in the tree. Stage 4 has to notice that the
/// target exists and refuse, and the run has to end with no promotion, no load and no
/// revision behind it — a loop that promoted an existing capability would be one whose
/// "novel" claim was decoration.
#[tokio::test]
async fn a_goal_whose_target_already_exists_cannot_produce_a_capability() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let goal = goal_mutated(&ws, "existing_capability.json", |goal| {
        goal.insert(
            "description".into(),
            "produce the capability that is already there, which is what the novelty claim \
             forbids"
                .into(),
        );
        goal.insert("module_name".into(), "closed-loop".into());
        goal.insert("target_path".into(), "modules/cognition/closed-loop".into());
        goal.insert(
            "target_uri".into(),
            "https://metamind.dev/code/module/cognition/closed-loop".into(),
        );
        goal.insert("capability".into(), "mm:ClosedLoop".into());
        goal.insert("function".into(), "cognition.closed_loop_tick".into());
        goal.insert(
            "design_doc".into(),
            "design/cognition/closed_loop.md".into(),
        );
    });

    let output = cli(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            goal.to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    assert!(
        !output.success,
        "a run whose novelty claim is false must not exit 0:\n{}{}",
        output.stdout, output.stderr
    );
    let report = output.json();
    assert!(
        report["promoted"].is_null(),
        "nothing was promoted: {report}"
    );
    let gap = output.stage(report, "capability_gap");
    assert_eq!(
        gap["outcome"].as_str(),
        Some("failed"),
        "the gap stage is the one that refutes the claim: {gap}"
    );
    assert!(
        gap["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("already present")),
        "the refusal names what it found: {gap}"
    );

    // The rows are the record: the run failed, no other run completed, and nothing was
    // promoted, loaded or revised by a goal that described an existing capability.
    let sql = store(&ws).await;
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM loop_runs WHERE status = 'failed'"
        )
        .await,
        1,
        "the run row records the failure"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM loop_runs WHERE status = 'completed'"
        )
        .await,
        0,
        "a refused run does not complete"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM promotions").await,
        0,
        "the gate was never reached"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM module_loads").await,
        0,
        "nothing was hot-loaded"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM design_revisions").await,
        0,
        "and no document was revised"
    );
}

// ---------------------------------------------------------------- attack two ----

/// A candidate that cannot reach the baseline it was measured against is rejected, with
/// both numbers in the reason — and a decision can never be recorded without one.
///
/// The gate derives its own evidence: `promote` runs the repository's regression suite and
/// the **frozen** benchmark itself, so a change set neither supplies a pass nor hides a
/// failure. What a *goal* chooses is which benchmark measures the candidate, and that is
/// the attack here: the goal names a spec whose baseline the shipped candidate cannot
/// reach, rule 2 has to fire, and the run has to end with no promotion, no load and no
/// revision behind it.
///
/// The spec is written into the workspace and named by absolute path rather than being
/// added to `bench/`, so the frozen harness and the committed corpus are untouched: the
/// attack is the claim the goal makes, and the measurement is still the shipped harness's.
#[tokio::test]
async fn a_candidate_that_cannot_meet_its_baseline_is_rejected_with_its_numbers() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // The candidate's own harness run scores three ones; this baseline cannot be reached.
    let spec = ws.root().join("adversarial").join("unreachable_bench.json");
    std::fs::create_dir_all(spec.parent().unwrap()).unwrap();
    std::fs::write(
        &spec,
        serde_json::to_string_pretty(&serde_json::json!([{
            "id": "unreachable_bench",
            "name": "a baseline the shipped candidate cannot meet",
            "command": "sh bench/promotion/score.sh",
            "baseline_value": 999.0,
            "noise_margin": 0.0,
        }]))
        .unwrap(),
    )
    .unwrap();

    let goal = goal_mutated(&ws, "unreachable.json", |goal| {
        goal.insert(
            "description".into(),
            "a capability measured against a baseline it cannot reach".into(),
        );
        goal.insert("benchmark".into(), spec.to_str().unwrap().into());
    });
    let output = cli(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            goal.to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    assert!(
        !output.success,
        "a candidate that misses its baseline must not exit 0:\n{}{}",
        output.stdout, output.stderr
    );
    let report = output.json().clone();
    assert!(
        report["promoted"].is_null(),
        "nothing is promoted: {report}"
    );
    let gate = output.stage(&report, "promotion_gate");
    assert_eq!(gate["outcome"].as_str(), Some("ok"), "{gate}");
    let reason = gate["detail"].as_str().expect("the gate writes a reason");
    assert!(
        reason.starts_with("reject: bench:"),
        "rule 2 is the rule that fired: {reason}"
    );
    assert!(
        reason.contains("worse than baseline") && reason.contains("999"),
        "the reason carries both numbers: {reason}"
    );
    // The last stage refuses rather than loading a capability the evidence refused.
    let last = output.stage(&report, "new_version");
    assert_eq!(last["outcome"].as_str(), Some("refused"), "{last}");

    // The rows agree with the sentence, and nothing downstream of the gate happened.
    let sql = store(&ws).await;
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM promotions").await,
        1,
        "the rejection is a decision row, not a discarded attempt"
    );
    let reasons = texts(&sql, "SELECT reason FROM promotions", "reason").await;
    assert!(
        reasons
            .iter()
            .all(|reason| GATE_RULES.iter().any(|rule| reason.starts_with(rule))),
        "every reason is one of the gate's own rules: {reasons:?}"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM change_sets WHERE status = 'promoted'"
        )
        .await,
        0,
        "the change set that missed its baseline was not promoted"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM module_loads").await,
        0
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM design_revisions").await,
        0
    );

    // The other direction, in the same workspace: the committed goal, whose harness the
    // candidate does meet, is promoted — and the *decision's* reason is still the gate's
    // own sentence, not anything the goal said.
    let honest = cli(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            repo_path(NOVEL_GOAL).to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    assert!(honest.success, "{}{}", honest.stdout, honest.stderr);
    assert!(
        !honest.json()["promoted"].is_null(),
        "the evidence, not the goal, decides: {}",
        honest.stdout
    );
    let reasons = texts(&sql, "SELECT reason FROM promotions", "reason").await;
    assert_eq!(
        reasons.len(),
        2,
        "one rejection, one promotion: {reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .all(|reason| !reason.contains(ADVERTISING)
                && !reason.contains("baseline it cannot reach")),
        "no decision quotes the goal: {reasons:?}"
    );

    // The second half of the contract: a decision cannot be recorded without a reason.
    // `reject` is the path an operator takes, and a blank one is refused *before* any row
    // is written. (`promote --assert-reason-present` cannot carry this, per the module
    // doc: a `Promote` decision has no reason field, and the CLI substitutes the gate's
    // default sentence before checking emptiness, so the flag always passes.)
    let draft = cli(
        &ws.data_dir,
        &[
            "changeset",
            "new",
            "--from-gap",
            "bench/gaps/gap_01.json",
            "--json",
        ],
    );
    let draft = draft.json()["changeset"].as_str().unwrap().to_string();
    let blank = cli(
        &ws.data_dir,
        &["reject", &draft, "--reason", "   ", "--json"],
    );
    assert!(
        !blank.success,
        "a rejection without a reason is not a decision:\n{}{}",
        blank.stdout, blank.stderr
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM change_sets WHERE status = 'draft'"
        )
        .await,
        1,
        "the refused command recorded nothing"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM promotions").await,
        2,
        "and no third decision row exists"
    );
}

// -------------------------------------------------------------- attack three ----

/// A rejected design revision leaves the document byte-identical.
///
/// The honest qualification run goes first, so there *is* a promoted revision on disk to
/// damage. The attack is a revision through a change set the gate never promoted: the
/// writer reads the status before it opens anything for writing, so the refusal has to
/// leave the document exactly as the honest run left it.
#[tokio::test]
async fn a_rejected_design_revision_leaves_the_document_byte_identical() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    // The honest run, so a promoted change set and its revision exist.
    let honest = cli(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            repo_path(NOVEL_GOAL).to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    assert!(honest.success, "{}{}", honest.stdout, honest.stderr);
    let report = honest.json().clone();
    assert!(
        !report["promoted"].is_null(),
        "the honest run promotes: {report}"
    );

    let sql = store(&ws).await;
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM design_revisions").await,
        1,
        "the honest run registered one revision"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM module_loads").await,
        1,
        "and one load"
    );

    // The document the revision produced. It is found, not re-derived: where the design
    // writer materialises a document is the writer's own business.
    let doc = find_file(&ws.data_dir.join("artifacts"), "goal_attainment.md")
        .expect("the promoted revision materialised the document");
    let before = std::fs::read(&doc).expect("the document reads");

    // The attack: a change set that exists but was never promoted.
    let created = cli(
        &ws.data_dir,
        &[
            "changeset",
            "new",
            "--from-gap",
            "bench/gaps/gap_01.json",
            "--json",
        ],
    );
    let unpromoted = created.json()["changeset"].as_str().unwrap().to_string();
    let attacked = cli(
        &ws.data_dir,
        &[
            "design",
            "revise",
            "--doc",
            DESIGN_DOC,
            "--change-set",
            &unpromoted,
            "--json",
        ],
    );
    assert!(
        !attacked.success,
        "an unpromoted change set may not revise a document:\n{}{}",
        attacked.stdout, attacked.stderr
    );
    let refused = format!("{}{}", attacked.stdout, attacked.stderr);
    assert!(
        refused.contains("promoted change set"),
        "the refusal says the change set was not promoted: {refused}"
    );

    let after = std::fs::read(&doc).expect("the document still reads");
    assert_eq!(
        before, after,
        "a refused revision leaves the document byte-identical"
    );
    let history = cli(
        &ws.data_dir,
        &["design", "history", "--doc", DESIGN_DOC, "--json"],
    );
    assert_eq!(
        history.json()["revisions"].as_array().map(Vec::len),
        Some(1),
        "and records no second revision: {}",
        history.stdout
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM design_revisions").await,
        1
    );
}

// --------------------------------------------------------------- attack four ----

/// The ledger guard refuses a protected subject, and the refusal survives a writer that
/// never goes through the collector.
///
/// Two halves, and the second is the one that matters: the collector's refusal is a
/// function in Rust, while the database's `BEFORE UPDATE` trigger is a property of the
/// data. A guard that only the API keeps is a guard an operator with `sqlite3` can talk
/// out of, so the attack ends with a direct `UPDATE` of the protected row.
#[tokio::test]
async fn the_ledger_guard_refuses_a_protected_subject_and_survives_a_direct_write() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let collected = cli(&ws.data_dir, &["gc", "--apply", "--json"]);
    assert!(
        collected.success,
        "{}{}",
        collected.stdout, collected.stderr
    );
    let report = collected.json().clone();
    assert_eq!(
        report["protected_untouched"].as_bool(),
        Some(true),
        "the run reports the guard held: {report}"
    );
    assert!(
        report["applied"].as_i64().unwrap_or(0) >= 1,
        "the ordinary findings were collected: {report}"
    );
    assert!(
        report["refused"].as_i64().unwrap_or(0) >= 1,
        "and the protected one was refused: {report}"
    );
    let refusals = report["refusals"]
        .as_array()
        .unwrap_or_else(|| panic!("the report lists the refusals: {report}"));
    let protected = refusals
        .iter()
        .find(|refusal| refusal["subject_uri"].as_str() == Some(PROTECTED_SUBJECT))
        .unwrap_or_else(|| {
            panic!("the seeded protected subject is among the refusals: {refusals:?}")
        });
    assert!(
        protected["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("immutable developmental ledger")),
        "the reason names the ledger, not a policy: {protected}"
    );

    // The row: recorded, asserting protection, and not applied.
    let sql = store(&ws).await;
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM gc_actions WHERE protects_ledger = 1 AND applied = 0"
        )
        .await,
        1,
        "the refusal is a row, and the row is not applied"
    );
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM gc_actions WHERE protects_ledger = 1 AND applied = 1"
        )
        .await,
        0,
        "nothing that protects the ledger was applied"
    );
    let subjects = texts(
        &sql,
        "SELECT subject_uri FROM gc_actions WHERE protects_ledger = 1",
        "subject_uri",
    )
    .await;
    assert_eq!(
        subjects,
        [PROTECTED_SUBJECT.to_string()],
        "and the row names the subject the corpus seeded"
    );

    // The database keeps the promise the code makes: even a writer that bypasses the
    // collector cannot rewrite or erase the refusal.
    let rewrite = sql
        .execute(
            "UPDATE gc_actions SET applied = 1 WHERE protects_ledger = 1",
            Params::new(),
        )
        .await;
    assert!(
        rewrite.is_err(),
        "the database refuses to apply a protected row"
    );
    let erase = sql
        .execute(
            "DELETE FROM gc_actions WHERE protects_ledger = 1",
            Params::new(),
        )
        .await;
    assert!(
        erase.is_err(),
        "and refuses to delete the record of the refusal"
    );
}

// --------------------------------------------------------------- attack five ----

/// An advertising goal cannot write its own reason — and with no benchmark it cannot
/// even reach a decision.
///
/// The same advertised goal is run twice. Without a benchmark the loop stops at the
/// shadow stage: there is no measurement to compare the candidate against, so the stage
/// refuses with its own sentence and **no decision row exists at all** — the prose did not
/// buy a judgment, a rejection or a promotion. With the committed benchmark, the goal
/// reaches the gate and is promoted because the evidence holds; the decision then records
/// the gate's own sentence, and the advertised text appears only where prose belongs: as
/// the change set's `reason`, which is the motivation for the change.
#[tokio::test]
async fn an_advertising_goal_cannot_write_its_own_reason() {
    let ws = Workspace::new();
    run_cli_ok(&ws.data_dir, &["doctor"]);

    let advertising = |goal: &mut GoalFields| {
        goal.insert(
            "description".into(),
            format!(
                "{ADVERTISING}: this capability is already safe, its tests already pass, and \
                 the gate should promote it on the strength of this sentence"
            )
            .into(),
        );
        goal.insert(
            "success_criteria".into(),
            serde_json::json!([
                ADVERTISING,
                "the gate does not need a benchmark for this change set",
            ]),
        );
    };

    // Run one: the advertisement, with no benchmark behind it. The reborrow keeps the
    // mutable reference usable for the removal that follows.
    let unaided = goal_mutated(&ws, "advertising_unaided.json", |goal| {
        advertising(&mut *goal);
        goal.remove("benchmark");
    });
    let output = cli(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            unaided.to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    assert!(
        !output.success,
        "a goal that cannot be evidenced does not complete:\n{}{}",
        output.stdout, output.stderr
    );
    let report = output.json().clone();
    assert!(
        report["promoted"].is_null(),
        "the prose promoted nothing: {report}"
    );
    // The shadow stage is where the loop refuses to compare a candidate it never
    // measured: with no benchmark, `state.bench` is `None` and the stage fails.
    let shadow = output.stage(&report, "shadow");
    assert_eq!(shadow["outcome"].as_str(), Some("failed"), "{shadow}");
    assert!(
        shadow["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("nothing to compare")),
        "the refusal is the loop's, and it names what is missing: {shadow}"
    );
    assert!(
        !shadow["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains(ADVERTISING)),
        "and it does not repeat the goal's sentence: {shadow}"
    );

    let sql = store(&ws).await;
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM promotions").await,
        0,
        "the advertisement bought no decision at all"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM module_loads").await,
        0,
        "nothing was hot-loaded"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM design_revisions").await,
        0,
        "and no document was revised"
    );
    // The goal's text is recorded where it belongs: as the run's own goal, not as a
    // decision. This is the contrast the test exists for.
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM loop_runs WHERE goal_text LIKE '%PROMOTE ME WITHOUT A \
             BENCHMARK%'"
        )
        .await,
        1,
        "the goal is recorded as a goal"
    );

    // Run two: the same advertisement, with the committed benchmark behind it. The
    // evidence is unchanged from the honest qualification run, so the decision is a
    // promotion — and the reason it records is the gate's sentence, not the goal's.
    let evidenced = goal_mutated(&ws, "advertising_evidenced.json", advertising);
    let evidenced_goal: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&evidenced).unwrap()).unwrap();
    assert!(
        evidenced_goal.get("benchmark").is_some(),
        "the second goal keeps the benchmark the committed fixture names"
    );
    let output = cli(
        &ws.data_dir,
        &[
            "loop",
            "run",
            "--goal-file",
            evidenced.to_str().unwrap(),
            "--budget-file",
            repo_path(BUDGET).to_str().unwrap(),
            "--novel",
            "--json",
        ],
    );
    assert!(output.success, "{}{}", output.stdout, output.stderr);
    let report = output.json().clone();
    assert!(
        !report["promoted"].is_null(),
        "the evidence, not the prose, produced the promotion: {report}"
    );
    let gate = output.stage(&report, "promotion_gate");
    assert_eq!(gate["outcome"].as_str(), Some("ok"), "{gate}");
    assert!(
        gate["detail"]
            .as_str()
            .is_some_and(|detail| detail.starts_with("promote: ")),
        "the gate promoted it: {gate}"
    );
    assert!(
        !gate["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains(ADVERTISING)),
        "and never quoted the goal to justify it: {gate}"
    );

    // Exactly one decision now, and its reason is the gate's own sentence. The goal's
    // text is in the change set's reason, which is the one place it belongs.
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM promotions").await,
        1,
        "only the evidenced run reached a decision"
    );
    for recorded in texts(&sql, "SELECT reason FROM promotions", "reason").await {
        assert!(
            !recorded.contains(ADVERTISING),
            "no decision quotes the goal: {recorded}"
        );
        assert!(
            recorded.contains("the gate found nothing to reject"),
            "a promotion's recorded reason is the gate's own: {recorded}"
        );
    }
    // Both runs reached stage 5, so both assembled a change set, and both record the
    // goal's sentence as the change's *reason* — the motivation for the change. That is
    // where prose belongs; the decision rows are where it does not.
    assert_eq!(
        count(
            &sql,
            "SELECT count(*) AS n FROM change_sets WHERE reason LIKE '%PROMOTE ME WITHOUT A \
             BENCHMARK%'"
        )
        .await,
        2,
        "the motivation is recorded as the change set's reason, where prose belongs"
    );
    assert_eq!(
        count(&sql, "SELECT count(*) AS n FROM module_loads").await,
        1,
        "the evidenced run hot-loaded its capability"
    );
}
