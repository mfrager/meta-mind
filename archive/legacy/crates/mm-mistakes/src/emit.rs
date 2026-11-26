//! `emit`: turn a minimized incident into a regression test, and run the suite.
//!
//! The artifact is a *directory*, not a string of code:
//!
//! ```text
//! bench/regression/<slug>/
//!   case.json    the contract — what the case is and which scripts play which part
//!   check.sh     the test; exit 0 when the subject behaves, non-zero while the bug is present
//!   fix.sh       apply the fix
//!   reset.sh     restore the bug
//! ```
//!
//! `bench/regression/README.md` is the format's documentation and the reason it is a
//! directory: a case must be runnable by an operator with no toolchain, resettable, and
//! readable as a diff. A Rust integration test would be none of those three.
//!
//! Four decisions are worth stating:
//!
//! * **The contract is asserted by running it, never by trusting the generator.**
//!   [`compile_mistake`] resets, checks, fixes, checks again and resets — the exact
//!   sequence the phase's gate runs — and refuses to record the case unless it observed a
//!   non-zero exit before the fix and a zero exit after. A compiler that wrote the files
//!   and assumed they worked would be reporting its own intention as evidence.
//! * **A generated subject is a re-enactment, and says so.** This phase cannot synthesize
//!   a patch for arbitrary code, so the generated `subject.sh` re-enacts the recorded
//!   incident — it reproduces the *behaviour* the mistake record describes (the observed
//!   side exits 0 where it should refuse). The interesting half of the artifact is the
//!   contract: `case.json` names the expectation, the marker and the pair, so a later
//!   phase that can localize the real code path replaces the subject and keeps the case.
//!   The comment inside the generated scripts says exactly this, so nobody later mistakes
//!   the re-enactment for the original defect.
//! * **The runner leaves the tree buggy.** `reset.sh` runs first and last, so a case run
//!   twice behaves the same both times and a case left behind is a case that still fails.
//! * **The path is the case's identity.** `regression_tests.path` is `UNIQUE` and the
//!   row is upserted, because recompiling the same mistake re-derives the same artifact;
//!   a second row would make "which test covers this mistake" ambiguous.
//!
//! A script is run with `/bin/sh`, `cwd` set to the case directory, and a ten-second
//! budget. The budget matters: a regression case that hangs is a case nobody runs, and a
//! timeout is reported as a failure with the script named rather than as a hung process.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mm_core::{Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{MistakeError, Result};
use crate::reproduce::{reproduce, MinimizedCase};
use crate::TARGET;

/// How long a case script may run.
pub const SCRIPT_TIMEOUT_SECS: u64 = 10;

/// The directory under `repo` where cases live.
pub const SUITE_DIR: &str = "bench/regression";

/// The heredoc terminators the generated scripts use.
///
/// Distinct from any word a failure mode would plausibly contain, and checked against the
/// subject before a script is written — a subject containing one would end the heredoc
/// early and produce a case that does something other than what it says.
const SUBJECT_TERMINATOR: &str = "MM_SUBJECT_SCRIPT";
const INCIDENT_TERMINATOR: &str = "MM_INCIDENT";

/// `case.json`: the contract of one regression case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseFile {
    /// The case's name; also its directory name.
    pub name: String,
    /// The `mistakes` row it was compiled from. `None` for a hand-written case.
    #[serde(default)]
    pub mistake_ulid: Option<Ulid>,
    /// One sentence an operator can read.
    pub description: String,
    /// The script that tests the subject.
    pub fail_before: String,
    /// The script that applies the fix.
    pub fix: String,
    /// The script that restores the bug.
    pub reset: String,
    /// What the case claims about itself.
    #[serde(default)]
    pub expected: CaseExpectation,
}

/// What a case claims: both halves or it is not a regression test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseExpectation {
    /// The test must fail on the buggy code.
    pub fails_before: bool,
    /// The test must pass once the fix is applied.
    pub passes_after: bool,
}

impl Default for CaseExpectation {
    fn default() -> Self {
        CaseExpectation {
            fails_before: true,
            passes_after: true,
        }
    }
}

impl CaseFile {
    /// A case for `name` with the conventional script names.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        CaseFile {
            name: name.into(),
            mistake_ulid: None,
            description: description.into(),
            fail_before: "check.sh".to_string(),
            fix: "fix.sh".to_string(),
            reset: "reset.sh".to_string(),
            expected: CaseExpectation::default(),
        }
    }

    /// The mistake a case was compiled from, if any.
    pub fn with_mistake(mut self, mistake: Ulid) -> Self {
        self.mistake_ulid = Some(mistake);
        self
    }

    /// Refuse a contract that is not one.
    ///
    /// A script name must be a plain file name: a path that escapes the case directory
    /// would make the case's behaviour depend on where it is run from, and a case that
    /// cannot be reasoned about locally cannot be trusted as evidence.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(MistakeError::validation("name", "must not be empty"));
        }
        if self.description.trim().is_empty() {
            return Err(MistakeError::validation(
                "description",
                "must not be empty: a case nobody can read is a case nobody runs",
            ));
        }
        for (field, script) in [
            ("fail_before", &self.fail_before),
            ("fix", &self.fix),
            ("reset", &self.reset),
        ] {
            if script.trim().is_empty() || script.contains('/') || script.contains('\\') {
                return Err(MistakeError::validation(
                    field,
                    format!("{script:?} must be a plain file name"),
                ));
            }
        }
        if !self.expected.fails_before || !self.expected.passes_after {
            return Err(MistakeError::validation(
                "expected",
                "a regression case must claim both halves: fail before and pass after",
            ));
        }
        Ok(())
    }
}

/// One case's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    /// The case's name.
    pub name: String,
    /// `check.sh` exited non-zero on the buggy code.
    pub fails_before: bool,
    /// `check.sh` exited zero once the fix was applied.
    pub passes_after: bool,
    /// The exit code of the fail-before run, when the script ran at all.
    pub before_exit: Option<i32>,
    /// The exit code of the pass-after run, when the script ran at all.
    pub after_exit: Option<i32>,
    /// What happened, in one line.
    pub detail: String,
}

impl CaseResult {
    /// True when the case holds both halves of its contract.
    pub fn passed(&self) -> bool {
        self.fails_before && self.passes_after
    }
}

/// The whole suite's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuiteReport {
    /// Every case, in name order.
    pub cases: Vec<CaseResult>,
    /// How many did not hold their contract.
    pub failed: u32,
}

impl SuiteReport {
    /// True when every case held its contract.
    pub fn passed(&self) -> bool {
        self.failed == 0
    }

    /// The names of the cases that failed.
    pub fn failing(&self) -> Vec<String> {
        self.cases
            .iter()
            .filter(|c| !c.passed())
            .map(|c| c.name.clone())
            .collect()
    }
}

/// One compiled regression test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionTest {
    /// The `regression_tests` row.
    pub id: Ulid,
    /// The case directory.
    pub path: PathBuf,
    /// The ULID of the observed fail-before run.
    pub fails_before: Ulid,
    /// The ULID of the observed pass-after run.
    pub passes_after: Ulid,
}

impl RegressionTest {
    /// The case's name: the last path component.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}

/// The directory a case with this slug lives in.
pub fn case_path(repo: &Path, slug: &str) -> PathBuf {
    repo.join(SUITE_DIR).join(slug)
}

/// The case directories under a suite, in name order.
///
/// A directory counts as a case when it holds a `case.json`; anything else in the suite
/// (a README, a scratch directory) is ignored rather than reported as a broken case.
pub fn cases_in_suite(suite: &Path) -> Result<Vec<String>> {
    let entries = std::fs::read_dir(suite).map_err(|e| {
        MistakeError::suite_unreadable(suite, format!("cannot list the directory: {e}"))
    })?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            MistakeError::suite_unreadable(suite, format!("cannot read an entry: {e}"))
        })?;
        let path = entry.path();
        if path.is_dir() && path.join("case.json").is_file() {
            if let Some(name) = path.file_name() {
                names.push(name.to_string_lossy().to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Read and check one case's contract.
pub fn read_case(dir: &Path) -> Result<CaseFile> {
    let file = dir.join("case.json");
    let raw = std::fs::read_to_string(&file)
        .map_err(|e| MistakeError::case_unreadable(&file, format!("cannot read it: {e}")))?;
    let case: CaseFile = serde_json::from_str(&raw)
        .map_err(|e| MistakeError::case_unreadable(&file, format!("not a case.json: {e}")))?;
    case.validate()?;
    for script in [&case.fail_before, &case.fix, &case.reset] {
        if !dir.join(script).is_file() {
            return Err(MistakeError::MissingScript {
                case: case.name.clone(),
                script: script.clone(),
                path: dir.to_path_buf(),
            });
        }
    }
    Ok(case)
}

/// What one script run produced.
#[derive(Debug, Clone, PartialEq)]
struct ScriptRun {
    exit: Option<i32>,
    success: bool,
    timed_out: bool,
    output: String,
}

impl ScriptRun {
    /// The exit code as a number, for a refusal that names it.
    fn code(&self) -> i32 {
        self.exit.unwrap_or(-1)
    }
}

/// Escape a sentence for the inside of a double-quoted shell string.
///
/// The incident text is user-written, and it is embedded in a generated script; a
/// failure mode containing a quote or a `$` must not be able to end the string or run a
/// command. The escaping is deliberately small and total: backslash first, then the four
/// characters a POSIX shell still interprets inside double quotes.
pub fn escape_for_double_quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    out
}

/// Run one of a case's scripts.
async fn run_script(dir: &Path, script: &str) -> Result<ScriptRun> {
    let path = dir.join(script);
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg(&path).current_dir(dir);
    let future = command.output();
    match tokio::time::timeout(Duration::from_secs(SCRIPT_TIMEOUT_SECS), future).await {
        Err(_) => Ok(ScriptRun {
            exit: None,
            success: false,
            timed_out: true,
            output: format!("{script} did not finish within {SCRIPT_TIMEOUT_SECS}s"),
        }),
        Ok(Err(e)) => Err(MistakeError::Spawn {
            command: format!("/bin/sh {}", path.display()),
            detail: e.to_string(),
        }),
        Ok(Ok(output)) => {
            let mut text = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&stderr);
            }
            Ok(ScriptRun {
                exit: output.status.code(),
                success: output.status.success(),
                timed_out: false,
                output: text.trim().to_string(),
            })
        }
    }
}

/// Every private copy gets its own sequence number, so two runs of the same case in one
/// process — two gate tests in one test binary, say — cannot land in the same directory.
static COPY_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Copy a case directory into a unique temporary directory, and return where it landed.
///
/// The case's own scripts rewrite their subject file (that is what `reset.sh` and `fix.sh`
/// *are*), so running them where they live would make `regression run` a mutating command:
/// two callers would race over the same checked-in file, and a tree under review would be
/// left modified by a read. The copy costs a directory walk and removes both problems.
/// `std::fs::copy` preserves the mode, which is what keeps the scripts executable.
fn copy_case_to_temp(dir: &Path) -> Result<PathBuf> {
    let sequence = COPY_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let target = std::env::temp_dir().join(format!(
        "mm-regression-{}-{}-{}",
        std::process::id(),
        sequence,
        &mm_core::content_hash(dir.display().to_string().as_bytes())[..12]
    ));
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| {
            MistakeError::suite_unreadable(
                target.clone(),
                format!("cannot clear a previous copy: {e}"),
            )
        })?;
    }
    copy_tree(dir, &target)?;
    Ok(target)
}

/// Copy a directory tree, preserving file modes.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| {
        MistakeError::suite_unreadable(to.to_path_buf(), format!("cannot create the copy: {e}"))
    })?;
    let entries = std::fs::read_dir(from)
        .map_err(|e| MistakeError::case_unreadable(from.to_path_buf(), e.to_string()))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| MistakeError::case_unreadable(from.to_path_buf(), e.to_string()))?;
        let destination = to.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|e| MistakeError::case_unreadable(entry.path(), e.to_string()))?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else {
            std::fs::copy(entry.path(), &destination).map_err(|e| {
                MistakeError::case_unreadable(
                    entry.path(),
                    format!("cannot copy into {}: {e}", destination.display()),
                )
            })?;
        }
    }
    Ok(())
}

/// Run one case: reset, check, fix, check, reset.
///
/// The refusals this returns are the ones an operator must act on (an unreadable directory,
/// a script that will not spawn); a case that merely does not hold its contract comes back
/// as a [`CaseResult`] with the halves marked false, so a suite run reports every case
/// rather than stopping at the first one.
///
/// The case runs in a private copy (see [`copy_case_to_temp`]) and the copy is removed
/// afterwards, so the caller's tree is exactly as it was found however the case ended.
pub async fn run_case(dir: &Path) -> Result<CaseResult> {
    let copy = copy_case_to_temp(dir)?;
    let outcome = run_case_in(&copy).await;
    let _ = std::fs::remove_dir_all(&copy);
    outcome
}

/// The body of [`run_case`], in the private copy.
async fn run_case_in(dir: &Path) -> Result<CaseResult> {
    let case = read_case(dir)?;
    let reset = run_script(dir, &case.reset).await?;
    if reset.timed_out {
        return Err(MistakeError::Timeout {
            case: case.name.clone(),
            script: case.reset.clone(),
            seconds: SCRIPT_TIMEOUT_SECS,
        });
    }
    let before = run_script(dir, &case.fail_before).await?;

    let fix = run_script(dir, &case.fix).await?;
    if fix.timed_out {
        return Err(MistakeError::Timeout {
            case: case.name.clone(),
            script: case.fix.clone(),
            seconds: SCRIPT_TIMEOUT_SECS,
        });
    }
    let after = if fix.success {
        run_script(dir, &case.fail_before).await?
    } else {
        ScriptRun {
            exit: fix.exit,
            success: false,
            timed_out: false,
            output: format!("{} failed: {}", case.fix, fix.output),
        }
    };

    // Leave the case buggy, so a second run sees what the first one did.
    let _ = run_script(dir, &case.reset).await;

    let fails_before = !before.success && !before.timed_out;
    let passes_after = after.success;
    let detail = if before.timed_out {
        format!("{} timed out", case.fail_before)
    } else if !fails_before {
        format!(
            "{} passed on the buggy code (exit {}); a test that never failed is not evidence",
            case.fail_before,
            before.code()
        )
    } else if !passes_after {
        let reason = if after.output.is_empty() {
            format!("exit {}", after.code())
        } else {
            after.output.clone()
        };
        format!("{} still fails after the fix: {reason}", case.fail_before)
    } else {
        format!(
            "failed before (exit {}) and passed after (exit {})",
            before.code(),
            after.code()
        )
    };
    Ok(CaseResult {
        name: case.name,
        fails_before,
        passes_after,
        before_exit: before.exit,
        after_exit: after.exit,
        detail,
    })
}

/// Run every case in a suite.
pub async fn run_suite(suite: &Path) -> Result<SuiteReport> {
    let mut cases = Vec::new();
    for name in cases_in_suite(suite)? {
        cases.push(run_case(&suite.join(name)).await?);
    }
    let failed = cases.iter().filter(|c| !c.passed()).count() as u32;
    Ok(SuiteReport { cases, failed })
}

// --------------------------------------------------------------------- generation --

/// Render the `case.json` for a minimized incident.
pub fn render_case_json(case: &MinimizedCase) -> String {
    let file = CaseFile {
        name: case.slug.clone(),
        mistake_ulid: Some(case.mistake),
        description: case.summary.clone(),
        fail_before: "check.sh".to_string(),
        fix: "fix.sh".to_string(),
        reset: "reset.sh".to_string(),
        expected: CaseExpectation::default(),
    };
    // Pretty-printed with a trailing newline: the file is committed and reviewed as a
    // diff, and a one-line blob of JSON is not reviewable.
    let mut text = serde_json::to_string_pretty(&file).unwrap_or_else(|_| "{}".to_string());
    text.push('\n');
    text
}

/// Render `reset.sh`: restore the bug.
pub fn render_reset_script(case: &MinimizedCase) -> String {
    format!(
        "#!/bin/sh\n\
         # Restore the bug, so the case fails before its fix.\n\
         # OBSERVED: {observed}\n\
         set -eu\n\
         dir=$(dirname \"$0\")\n\
         cat > \"$dir/subject.sh\" <<'{terminator}'\n\
         #!/bin/sh\n\
         # subject.sh — a re-enactment of the recorded incident, not the original code path.\n\
         # The mistake record says the operation reported success where it should have\n\
         # refused; the fixed half of this case refuses. A later phase that can localize\n\
         # the real defect replaces this subject and keeps case.json unchanged.\n\
         set -u\n\
         input=${{1:-incident.txt}}\n\
         cat \"$input\"\n\
         exit 0\n\
         {terminator}\n\
         chmod +x \"$dir/subject.sh\"\n",
        observed = case.observed,
        terminator = SUBJECT_TERMINATOR,
    )
}

/// Render `fix.sh`: apply the corrective rule.
pub fn render_fix_script(case: &MinimizedCase) -> String {
    format!(
        "#!/bin/sh\n\
         # Apply the fix, so the case passes after it.\n\
         # EXPECTED: {expectation}\n\
         set -eu\n\
         dir=$(dirname \"$0\")\n\
         cat > \"$dir/subject.sh\" <<'{terminator}'\n\
         #!/bin/sh\n\
         # subject.sh — the corrective rule, applied: an input carrying the incident's\n\
         # marker is refused instead of reported as success.\n\
         set -u\n\
         input=${{1:-incident.txt}}\n\
         if grep -q -F -- '{marker}' \"$input\"; then\n\
         \x20 echo \"refused: {message}\" >&2\n\
         \x20 exit 2\n\
         fi\n\
         exit 0\n\
         {terminator}\n\
         chmod +x \"$dir/subject.sh\"\n",
        expectation = case.expectation,
        message = escape_for_double_quotes(&case.subject),
        marker = case.marker(),
        terminator = SUBJECT_TERMINATOR,
    )
}

/// Render `check.sh`: assert both halves.
///
/// Both halves matter. A test that only asserted "the incident is refused" would pass for a
/// subject that refuses *everything*, which is the classic way a regression test becomes a
/// test of the fix's shape rather than of the bug's absence.
pub fn render_check_script(case: &MinimizedCase) -> String {
    format!(
        "#!/bin/sh\n\
         # The test. It asserts both halves of the contract: the recorded incident is\n\
         # refused, and an ordinary input still succeeds.\n\
         set -eu\n\
         dir=$(dirname \"$0\")\n\
         work=$(mktemp -d)\n\
         trap 'rm -rf \"$work\"' EXIT\n\
         \n\
         cat > \"$work/incident.txt\" <<'{incident_terminator}'\n\
         {subject}\n\
         {incident_terminator}\n\
         if \"$dir/subject.sh\" \"$work/incident.txt\" >/dev/null 2>&1; then\n\
         \x20 echo \"the recorded incident was reported as success\" >&2\n\
         \x20 exit 1\n\
         fi\n\
         \n\
         printf '%s\\n' 'an ordinary input' > \"$work/clean.txt\"\n\
         if ! \"$dir/subject.sh\" \"$work/clean.txt\" >/dev/null 2>&1; then\n\
         \x20 echo \"an ordinary input was refused\" >&2\n\
         \x20 exit 1\n\
         fi\n",
        subject = case.subject,
        incident_terminator = INCIDENT_TERMINATOR,
    )
}

/// Write a script and make it executable.
fn write_script(path: &Path, body: &str) -> Result<()> {
    std::fs::write(path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

/// Write a case's four files, refusing a subject a heredoc could not carry verbatim.
pub fn write_case(dir: &Path, case: &MinimizedCase) -> Result<()> {
    for text in [&case.subject, &case.observed, &case.expectation] {
        if text.contains(SUBJECT_TERMINATOR) || text.contains(INCIDENT_TERMINATOR) {
            return Err(MistakeError::validation(
                "failure_mode",
                "the incident text contains a script terminator and cannot be embedded verbatim",
            ));
        }
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("case.json"), render_case_json(case))?;
    write_script(&dir.join("check.sh"), &render_check_script(case))?;
    write_script(&dir.join("fix.sh"), &render_fix_script(case))?;
    write_script(&dir.join("reset.sh"), &render_reset_script(case))?;
    Ok(())
}

/// Compile a mistake into a regression test, and record it only once it has been observed
/// to fail before and pass after.
pub async fn compile_mistake(
    mistake: Ulid,
    repo: &Path,
    store: &SqliteStore,
    logger: &Logger,
    ids: &UlidFactory,
) -> Result<RegressionTest> {
    let case = reproduce(mistake, store).await?;
    let dir = case_path(repo, &case.slug);
    write_case(&dir, &case)?;

    let result = run_case(&dir).await?;
    if !result.fails_before {
        return Err(MistakeError::NotFailingBefore {
            case: result.name,
            code: result.before_exit.unwrap_or(-1),
        });
    }
    if !result.passes_after {
        return Err(MistakeError::NotPassingAfter {
            case: result.name,
            code: result.after_exit.unwrap_or(-1),
        });
    }

    let id = ids.next();
    let fails_before = ids.next();
    let passes_after = ids.next();
    let at = Timestamp::now().to_rfc3339();
    let relative = PathBuf::from(SUITE_DIR).join(&case.slug);
    // `path` is UNIQUE: recompiling the same mistake re-derives the same artifact, so the
    // row is replaced rather than duplicated.
    store
        .execute(
            "INSERT INTO regression_tests \
             (id, mistake_ulid, path, fails_before_ulid, passes_after_ulid, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(path) DO UPDATE SET \
               mistake_ulid = excluded.mistake_ulid, \
               fails_before_ulid = excluded.fails_before_ulid, \
               passes_after_ulid = excluded.passes_after_ulid, \
               created_at = excluded.created_at",
            vec![
                mm_core::Param::Text(mm_core::ulid_string(&id)),
                mm_core::Param::Text(mm_core::ulid_string(&case.mistake)),
                mm_core::Param::Text(relative.to_string_lossy().to_string()),
                mm_core::Param::Text(mm_core::ulid_string(&fails_before)),
                mm_core::Param::Text(mm_core::ulid_string(&passes_after)),
                mm_core::Param::Text(at),
            ],
        )
        .await?;

    logger
        .audit(
            Level::Info,
            codes::MISTAKE_REGRESS,
            TARGET,
            Some(id),
            json!({
                "mistake_id": mm_core::ulid_string(&case.mistake),
                "test_path": relative.to_string_lossy(),
                "fails_before": mm_core::ulid_string(&fails_before),
                "passes_after": mm_core::ulid_string(&passes_after),
                "slug": case.slug,
                "cursor": case.cues.len(),
            }),
        )
        .await?;

    Ok(RegressionTest {
        id,
        path: relative,
        fails_before,
        passes_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reproduce::minimize;
    use mm_log::{JsonlSink, RedactionPolicy};
    use std::sync::Arc;

    fn a_case() -> MinimizedCase {
        let (observed, expectation, cues) = minimize(
            "the record counter reported an empty success on a stream it could not read",
            &["a missing file looked like an empty one".to_string()],
        );
        MinimizedCase {
            mistake: Ulid::from_parts(1_700_000_000_000, 42),
            slug: "the_record_counter_reported_000042".to_string(),
            summary: "a regression test for: the record counter reported an empty success"
                .to_string(),
            subject: "the record counter reported an empty success on a stream it could not read"
                .to_string(),
            expectation,
            observed,
            cues,
        }
    }

    fn repo_root() -> PathBuf {
        mm_core::Config::repo_root()
    }

    /// Copy a committed case into a throwaway directory.
    ///
    /// The runner *writes* (it resets and fixes), so a test that ran a case where it is
    /// committed would mutate the repository.
    fn copy_case(name: &str, into: &Path) -> PathBuf {
        let source = repo_root().join(SUITE_DIR).join(name);
        let target = into.join(name);
        std::fs::create_dir_all(&target).unwrap();
        for file in ["case.json", "check.sh", "fix.sh", "reset.sh"] {
            let text = std::fs::read_to_string(source.join(file))
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", source.join(file).display()));
            let path = target.join(file);
            std::fs::write(&path, text).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&path).unwrap().permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(&path, permissions).unwrap();
            }
        }
        target
    }

    async fn harness() -> (
        tempfile::TempDir,
        SqliteStore,
        Arc<Logger>,
        Arc<UlidFactory>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = mm_core::Config::for_data_dir(dir.path());
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let sink = JsonlSink::open(&dir.path().join("mm.jsonl")).unwrap();
        let logger = Arc::new(Logger::new(
            Level::Trace,
            vec![Box::new(sink)],
            Some(Arc::new(store.clone())),
            RedactionPolicy::kernel_default(),
        ));
        let ids = Arc::new(UlidFactory::open(&cfg.ulid_watermark_path()).unwrap());
        (dir, store, logger, ids)
    }

    async fn record_mistake(store: &SqliteStore, id: Ulid, mode: &str) {
        store
            .execute(
                "INSERT INTO mistakes (id, memory_id, failure_mode, missed_signal, \
                 recurrence_risk, created_ulid) VALUES (?, ?, ?, ?, ?, ?)",
                vec![
                    mm_core::Param::Text(mm_core::ulid_string(&id)),
                    mm_core::Param::Text(mm_core::ulid_string(&Ulid::from_parts(1, 1))),
                    mm_core::Param::Text(mode.to_string()),
                    mm_core::Param::Text(
                        "[\"a missing file looked like an empty one\"]".to_string(),
                    ),
                    mm_core::Param::Real(0.7),
                    mm_core::Param::Text(mm_core::ulid_string(&id)),
                ],
            )
            .await
            .unwrap();
    }

    #[test]
    fn a_contract_must_be_a_contract() {
        let mut case = CaseFile::new("c", "one sentence");
        assert!(case.validate().is_ok());
        case.name = "  ".into();
        assert!(case.validate().is_err());
        case.name = "c".into();
        case.description = String::new();
        assert!(case.validate().is_err());
        case.description = "d".into();
        case.fail_before = "../check.sh".into();
        assert!(case.validate().is_err());
        case.fail_before = "check.sh".into();
        case.expected = CaseExpectation {
            fails_before: true,
            passes_after: false,
        };
        let error = case.validate().unwrap_err();
        assert_eq!(error.code(), "mistake.validation");
    }

    #[test]
    fn the_generated_scripts_carry_the_incident_and_both_halves() {
        let case = a_case();
        let check = render_check_script(&case);
        let fix = render_fix_script(&case);
        let reset = render_reset_script(&case);
        assert!(check.contains(&case.subject), "{check}");
        assert!(check.contains("the recorded incident was reported as success"));
        assert!(check.contains("an ordinary input was refused"));
        assert!(fix.contains(&case.marker()), "{fix}");
        assert!(fix.contains("refused:"), "{fix}");
        // A failure mode cannot break out of the generated script's string.
        let hostile = MinimizedCase {
            subject: "a quote \" and a $(command) and a `tick`".to_string(),
            ..case.clone()
        };
        let escaped = render_fix_script(&hostile);
        assert!(escaped.contains("\\$(command)"), "{escaped}");
        // The invariant is about the *interpolated* text, not about the whole script: the
        // generated shell's own plumbing legitimately opens a substitution
        // (`dir=$(dirname "$0")`), so "no `$(` anywhere" would be a rule the generator
        // could only satisfy by not being a shell script. What must hold is that no line
        // carrying the incident's text opens one unescaped.
        for line in escaped.lines().filter(|line| line.contains("command")) {
            for (index, _) in line.match_indices("$(") {
                assert_eq!(line.as_bytes()[index - 1], b'\\', "{line}");
            }
        }
        assert_eq!(
            escape_for_double_quotes("a \"quote\", a $(command) and a `tick`"),
            "a \\\"quote\\\", a \\$(command) and a \\`tick\\`"
        );
        assert_eq!(escape_for_double_quotes("back\\slash"), "back\\\\slash");
        assert!(reset.contains(&case.observed), "{reset}");
        for script in [&check, &fix, &reset] {
            assert!(script.starts_with("#!/bin/sh\n"), "{script}");
            assert!(script.contains("set -eu"), "{script}");
        }
    }

    #[test]
    fn a_case_json_names_the_mistake_and_its_scripts() {
        let case = a_case();
        let rendered = render_case_json(&case);
        let parsed: CaseFile = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed.name, case.slug);
        assert_eq!(parsed.mistake_ulid, Some(case.mistake));
        assert_eq!(parsed.fail_before, "check.sh");
        assert_eq!(parsed.expected, CaseExpectation::default());
        assert!(rendered.ends_with('\n'));
    }

    #[tokio::test]
    async fn a_generated_case_fails_before_and_passes_after() {
        let dir = tempfile::tempdir().unwrap();
        let case = a_case();
        let target = dir.path().join(&case.slug);
        write_case(&target, &case).unwrap();
        for file in ["case.json", "check.sh", "fix.sh", "reset.sh"] {
            assert!(target.join(file).is_file(), "{file} was not written");
        }
        let result = run_case(&target).await.unwrap();
        assert!(result.fails_before, "{result:?}");
        assert!(result.passes_after, "{result:?}");
        assert!(result.passed());
        assert_eq!(result.before_exit.map(|c| c != 0), Some(true));
        assert_eq!(result.after_exit, Some(0));
        // Idempotent: the runner left the case buggy and reset it afterwards.
        let again = run_case(&target).await.unwrap();
        assert_eq!(result, again);
    }

    #[tokio::test]
    async fn the_committed_case_holds_its_contract() {
        let dir = tempfile::tempdir().unwrap();
        let case_dir = copy_case("seeded_bug_01", dir.path());
        let result = run_case(&case_dir).await.unwrap();
        assert!(result.passed(), "{result:?}");
        assert_eq!(result.name, "seeded_bug_01");
        let suite = run_suite(dir.path()).await.unwrap();
        assert!(suite.passed(), "{suite:?}");
        assert_eq!(suite.failed, 0);
        assert_eq!(suite.cases.len(), 1);
    }

    #[tokio::test]
    async fn the_repository_suite_lists_the_committed_cases() {
        let suite = repo_root().join(SUITE_DIR);
        let names = cases_in_suite(&suite).unwrap();
        assert!(names.contains(&"seeded_bug_01".to_string()), "{names:?}");
    }

    #[tokio::test]
    async fn a_case_that_passes_on_the_bug_is_reported_as_failing_before() {
        let dir = tempfile::tempdir().unwrap();
        let case = a_case();
        let target = dir.path().join(&case.slug);
        write_case(&target, &case).unwrap();
        // Replace the buggy subject generator with the fixed one, so check.sh runs against
        // a subject that already behaves: this is the "test never failed" failure mode.
        std::fs::write(target.join("reset.sh"), render_fix_script(&case)).unwrap();
        let result = run_case(&target).await.unwrap();
        assert!(!result.fails_before, "{result:?}");
        assert!(!result.passed());
        assert!(result.detail.contains("never failed"), "{}", result.detail);
    }

    #[tokio::test]
    async fn a_case_with_a_missing_script_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let case = CaseFile::new("broken", "names a script that is not there");
        std::fs::write(
            dir.path().join("case.json"),
            serde_json::to_string(&case).unwrap(),
        )
        .unwrap();
        let error = read_case(dir.path()).unwrap_err();
        assert_eq!(error.code(), "regression.missing_script");
        assert!(error.to_string().contains("check.sh"), "{error}");
    }

    #[tokio::test]
    async fn a_directory_that_is_not_a_case_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("case.json"), "{ not json").unwrap();
        let error = read_case(dir.path()).unwrap_err();
        assert_eq!(error.code(), "regression.case_unreadable");
        let missing = read_case(&dir.path().join("nowhere")).unwrap_err();
        assert_eq!(missing.code(), "regression.case_unreadable");
    }

    #[tokio::test]
    async fn compiling_a_mistake_writes_the_case_runs_it_and_records_it_once() {
        let (dir, store, logger, ids) = harness().await;
        let mistake = Ulid::from_parts(1_700_000_000_000, 77);
        record_mistake(
            &store,
            mistake,
            "the parser accepted an empty record stream",
        )
        .await;

        let repo = dir.path().join("repo");
        let test = compile_mistake(mistake, &repo, &store, &logger, &ids)
            .await
            .unwrap();
        let target = repo.join(&test.path);
        assert!(target.join("case.json").is_file());
        assert!(target.join("check.sh").is_file());
        assert_eq!(test.name(), target.file_name().unwrap().to_string_lossy());
        assert_ne!(test.fails_before, test.passes_after);

        let rows = store
            .query_json("SELECT count(*) AS n FROM regression_tests", Vec::new())
            .await
            .unwrap();
        assert_eq!(rows[0]["n"].as_i64(), Some(1));

        // Recompiling the same mistake upserts rather than duplicating.
        compile_mistake(mistake, &repo, &store, &logger, &ids)
            .await
            .unwrap();
        let rows = store
            .query_json("SELECT count(*) AS n FROM regression_tests", Vec::new())
            .await
            .unwrap();
        assert_eq!(rows[0]["n"].as_i64(), Some(1));

        let log = std::fs::read_to_string(dir.path().join("mm.jsonl")).unwrap();
        assert!(log.contains(codes::MISTAKE_REGRESS), "{log}");
        assert!(log.contains("test_path"), "{log}");
    }

    #[tokio::test]
    async fn a_case_whose_fix_does_not_fix_is_a_named_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let case = a_case();
        let target = dir.path().join(&case.slug);
        write_case(&target, &case).unwrap();
        // A fix script that does nothing leaves the bug in place.
        std::fs::write(target.join("fix.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        let result = run_case(&target).await.unwrap();
        assert!(result.fails_before, "{result:?}");
        assert!(!result.passes_after, "{result:?}");
        assert!(result.detail.contains("still fails"), "{}", result.detail);
    }
}
