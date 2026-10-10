//! The sandbox: where a candidate is allowed to exist.
//!
//! Phase 11's third invariant is "production is never written by the pipeline", and
//! this module is the half of it that acts: a candidate gets a **git worktree** of its
//! own under `data/sandbox/<ulid>/`, every write goes through [`SandboxDir`], and
//! [`audit_writes`] is the check that says afterwards that nothing else moved.
//!
//! Five decisions are worth stating because they are not visible in the shape of the
//! code:
//!
//! * **A worktree, not a copy.** A copy would be a second checkout with no relation to
//!   the commit the candidate diverges from, so "which tree was this judged on" would
//!   be unanswerable. A worktree is the same repository at a recorded commit, its
//!   diff is `git diff`-readable, and removing it is `git worktree remove`.
//! * **A worktree failure is a refusal, not a fallback.** If `git worktree add` fails,
//!   [`Sandbox::prepare`] returns [`SelfEngError::Git`]. Falling back to a plain
//!   directory would silently judge a candidate against a tree the candidate could
//!   have written anywhere.
//! * **Containment is checked twice.** [`changeset::validate_relative_path`] refuses an
//!   absolute path or a `..` before anything runs; [`is_inside`] then compares the
//!   *resolved* path against the sandbox root, which is the check that has a real root
//!   to compare with. Neither subsumes the other.
//! * **The audit is mtime-based and honest about being incomplete.** `audit_writes`
//!   reports every file outside the sandbox whose modification time is newer than the
//!   instant it is given, and sets `complete = false` when a directory could not be
//!   read — because "I could not check this" and "nothing changed" are different
//!   answers, and a boolean that merged them would be the audit lying.
//! * **Build and test are the project's own commands.** The sandbox runs `cargo build`
//!   and `cargo test` in the worktree rather than a bespoke harness, so a candidate is
//!   judged by the same commands a human runs.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use mm_core::{Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use serde_json::json;

use crate::changeset::ChangeSet;
use crate::error::{Result, SelfEngError};

/// The directory a candidate lives in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxDir {
    /// The change set it was prepared for.
    pub id: Ulid,
    /// Its root, an absolute path.
    pub root: PathBuf,
}

impl SandboxDir {
    /// A sandbox at `root` for `id`.
    pub fn new(id: Ulid, root: impl Into<PathBuf>) -> Self {
        SandboxDir {
            id,
            root: root.into(),
        }
    }

    /// Resolve one patch path inside the sandbox, or refuse.
    pub fn resolve(&self, relative: &Path) -> Result<PathBuf> {
        let candidate = self.root.join(relative);
        ensure_inside(&self.root, &candidate)?;
        Ok(candidate)
    }
}

/// What a build run produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildResult {
    /// The command's exit code.
    pub exit_code: i32,
    /// How many `warning:` lines the output carried. The plan's `sandbox build` record
    /// asks for a warning count, and a build that "passes" with new warnings is the
    /// kind of drift a count makes visible.
    pub warnings: usize,
    /// The last lines of the output, for the log.
    pub stdout_tail: String,
}

impl BuildResult {
    /// True when the build succeeded.
    pub fn succeeded(&self) -> bool {
        self.exit_code == 0
    }
}

/// What a test run produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResult {
    /// The command's exit code.
    pub exit_code: i32,
    /// Tests that passed.
    pub passed: u32,
    /// Tests that failed.
    pub failed: u32,
    /// The last `test result:` line, or the tail of the output.
    pub summary: String,
}

impl TestResult {
    /// True when every test passed.
    pub fn succeeded(&self) -> bool {
        self.exit_code == 0 && self.failed == 0
    }
}

/// A place a candidate can be built and tested.
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Create the sandbox and record it exists.
    async fn prepare(&self, cs: &ChangeSet) -> Result<SandboxDir>;
    /// Build inside it.
    async fn build(&self, d: &SandboxDir) -> Result<BuildResult>;
    /// Test inside it.
    async fn test(&self, d: &SandboxDir) -> Result<TestResult>;
}

/// A git worktree under a sandbox root.
pub struct WorktreeSandbox {
    /// The repository the worktree comes from.
    repo: PathBuf,
    /// Where sandboxes live, normally `data/sandbox`.
    sandbox_root: PathBuf,
    /// Where the runs are recorded.
    logger: Arc<Logger>,
    /// The identifier factory the sandbox directory name comes from.
    ids: Arc<UlidFactory>,
    /// The commit the worktree is detached at.
    commitish: String,
}

impl WorktreeSandbox {
    /// A sandbox over a repository.
    pub fn new(
        repo: impl Into<PathBuf>,
        sandbox_root: impl Into<PathBuf>,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
    ) -> Self {
        WorktreeSandbox {
            repo: repo.into(),
            sandbox_root: sandbox_root.into(),
            logger,
            ids,
            commitish: "HEAD".to_string(),
        }
    }

    /// Detach the worktree at another revision.
    pub fn at(mut self, commitish: impl Into<String>) -> Self {
        self.commitish = commitish.into();
        self
    }

    /// The root sandboxes are created under.
    pub fn root(&self) -> &Path {
        &self.sandbox_root
    }

    /// Write every patch of a change set into the sandbox.
    ///
    /// Returns the paths it touched, so a caller can record exactly what the candidate
    /// changed rather than what it claimed to change.
    pub async fn apply(&self, d: &SandboxDir, cs: &ChangeSet) -> Result<Vec<PathBuf>> {
        cs.validate()?;
        let mut touched = Vec::new();
        for patch in &cs.code {
            let target = d.resolve(&patch.path)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match patch.operation {
                crate::changeset::PatchOperation::Delete => {
                    // Deleting a path that is already absent is not an error: the patch
                    // states the desired state, and the desired state is "not there".
                    match std::fs::remove_file(&target) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            return Err(SelfEngError::Sandbox(format!(
                                "cannot delete {}: {e}",
                                target.display()
                            )))
                        }
                    }
                }
                _ => {
                    std::fs::write(&target, patch.content.as_bytes()).map_err(|e| {
                        SelfEngError::Sandbox(format!("cannot write {}: {e}", target.display()))
                    })?;
                }
            }
            touched.push(target);
        }
        Ok(touched)
    }

    /// Run one command in the sandbox.
    async fn run(&self, d: &SandboxDir, program: &str, args: &[&str]) -> Result<(i32, String)> {
        let output = tokio::process::Command::new(program)
            .args(args)
            .current_dir(&d.root)
            .output()
            .await
            .map_err(|e| SelfEngError::Sandbox(format!("cannot run {program}: {e}")))?;
        let mut text = String::from_utf8_lossy(&output.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok((output.status.code().unwrap_or(-1), text))
    }
}

#[async_trait]
impl Sandbox for WorktreeSandbox {
    async fn prepare(&self, cs: &ChangeSet) -> Result<SandboxDir> {
        cs.validate()?;
        std::fs::create_dir_all(&self.sandbox_root)?;
        let id = self.ids.next();
        let root = self.sandbox_root.join(mm_core::ulid_string(&id));
        let output = tokio::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "--detach",
                &root.to_string_lossy(),
                &self.commitish,
            ])
            .current_dir(&self.repo)
            .output()
            .await
            .map_err(|e| SelfEngError::Git {
                command: "worktree add".to_string(),
                status: -1,
                stderr: format!("cannot run git: {e}"),
            })?;
        if !output.status.success() {
            return Err(SelfEngError::Git {
                command: "worktree add".to_string(),
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let sandbox = SandboxDir::new(id, root);
        self.logger
            .audit(
                Level::Info,
                codes::SANDBOX_PREPARE,
                crate::TARGET,
                Some(id),
                json!({
                    "change_set_id": mm_core::ulid_string(&cs.id),
                    "sandbox_id": mm_core::ulid_string(&id),
                    "dir": sandbox.root.display().to_string(),
                    "commit": self.commitish,
                }),
            )
            .await?;
        Ok(sandbox)
    }

    async fn build(&self, d: &SandboxDir) -> Result<BuildResult> {
        let (exit_code, text) = self.run(d, "cargo", &["build"]).await?;
        let warnings = text
            .lines()
            .filter(|line| line.trim_start().starts_with("warning:"))
            .count();
        let result = BuildResult {
            exit_code,
            warnings,
            stdout_tail: tail(&text, 12),
        };
        self.logger
            .audit(
                Level::Info,
                codes::SANDBOX_BUILD,
                crate::TARGET,
                Some(d.id),
                json!({
                    "change_set_id": mm_core::ulid_string(&d.id),
                    "dir": d.root.display().to_string(),
                    "exit_code": result.exit_code,
                    "warnings": result.warnings,
                }),
            )
            .await?;
        Ok(result)
    }

    async fn test(&self, d: &SandboxDir) -> Result<TestResult> {
        let (exit_code, text) = self.run(d, "cargo", &["test"]).await?;
        let (passed, failed) = parse_test_totals(&text);
        // `rfind`, not `filter(..).last()`: the summary wanted is the last
        // `test result:` line, and the double-ended walk finds it from the end
        // rather than streaming the whole log to keep only the tail.
        let summary = text
            .lines()
            .rfind(|line| line.contains("test result:"))
            .map(|line| line.trim().to_string())
            .unwrap_or_else(|| tail(&text, 6));
        let result = TestResult {
            exit_code,
            passed,
            failed,
            summary,
        };
        self.logger
            .audit(
                Level::Info,
                codes::SANDBOX_TEST,
                crate::TARGET,
                Some(d.id),
                json!({
                    "change_set_id": mm_core::ulid_string(&d.id),
                    "dir": d.root.display().to_string(),
                    "exit_code": result.exit_code,
                    "passed": result.passed,
                    "failed": result.failed,
                }),
            )
            .await?;
        Ok(result)
    }
}

/// Sum the `test result:` lines of a cargo test run.
///
/// The line is `test result: ok. 12 passed; 0 failed; 0 ignored; …`, so the count is
/// the last whitespace-separated token *before* the word, not the whole segment:
/// `"test result: ok. 12".parse::<u32>()` fails, and a parser that quietly read zero
/// would report a passing run as an empty one.
fn parse_test_totals(text: &str) -> (u32, u32) {
    fn count_before(segment: &str, word: &str) -> Option<u32> {
        let head = segment.trim().strip_suffix(word)?.trim();
        // `rsplit(char::is_whitespace)`, not `rsplit_whitespace`: `str` has the former
        // and only the forward `split_whitespace`.
        head.rsplit(char::is_whitespace).next()?.parse::<u32>().ok()
    }

    let mut passed = 0_u32;
    let mut failed = 0_u32;
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("test result:") {
            continue;
        }
        for segment in line.split(';') {
            if let Some(value) = count_before(segment, "passed") {
                passed += value;
            }
            if let Some(value) = count_before(segment, "failed") {
                failed += value;
            }
        }
    }
    (passed, failed)
}

/// The last `lines` lines of a text.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

/// What a filesystem audit found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteAudit {
    /// Files outside the sandbox that changed after the instant audited from.
    pub outside: Vec<PathBuf>,
    /// False when some directory could not be read, so "nothing changed" is not a
    /// claim the audit can make.
    pub complete: bool,
}

impl WriteAudit {
    /// True when the audit is both complete and empty.
    pub fn is_clean(&self) -> bool {
        self.complete && self.outside.is_empty()
    }
}

/// Walk the production tree and report anything modified after `since`.
///
/// `data/sandbox`, `target` and `.git` are skipped: the first is where the candidate is
/// *supposed* to write, and the other two are build and version-control scratch that
/// every run touches. Everything else is production, and a production file newer than
/// the instant the promotion started is a write the pipeline did not authorize.
pub fn audit_writes(production_root: &Path, sandbox_root: &Path, since: SystemTime) -> WriteAudit {
    let mut audit = WriteAudit {
        outside: Vec::new(),
        complete: true,
    };
    let mut stack = vec![production_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                // An unreadable directory is not evidence of cleanliness.
                audit.complete = false;
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    audit.complete = false;
                    continue;
                }
            };
            if metadata.is_dir() {
                if name == "target" || name == ".git" || is_inside(sandbox_root, &path) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            match metadata.modified() {
                Ok(modified) => {
                    if modified > since {
                        audit.outside.push(path);
                    }
                }
                Err(_) => audit.complete = false,
            }
        }
    }
    audit.outside.sort();
    audit
}

/// True when `candidate` is `root` or a descendant of it.
///
/// The comparison is lexical: both paths are normalized (`.` removed, `..` folded,
/// redundant separators dropped) and then compared component-wise. Nothing touches the
/// filesystem, so the check works for a path that does not exist yet — which is the
/// case that matters, because a patch writes a file that is not there.
pub fn is_inside(root: &Path, candidate: &Path) -> bool {
    let root = lexical_resolve(root);
    let candidate = lexical_resolve(candidate);
    candidate.starts_with(&root)
}

/// Refuse a path outside the sandbox root.
pub fn ensure_inside(root: &Path, candidate: &Path) -> Result<()> {
    if is_inside(root, candidate) {
        Ok(())
    } else {
        Err(SelfEngError::Escape {
            path: candidate.display().to_string(),
        })
    }
}

/// Fold `.` and `..` out of a path without touching the filesystem.
///
/// `..` at the root is dropped rather than kept: `/..` is `/`, and keeping it would
/// make `/..` compare as a prefix of nothing while still being an ancestor of
/// everything.
pub fn lexical_resolve(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    // Nothing to pop: the parent of the root is the root.
                }
            }
            Component::Normal(part) => out.push(part),
            Component::RootDir => out.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::{from_gap, BenchmarkId, Gap, Patch, TestId};
    use mm_core::Config;

    /// A logger over the test directory. These tests exercise `prepare` (which records
    /// a `sandbox.prepare` audit record only on success) and `apply` (which records
    /// nothing), so no audit writer is needed: the git-failure case must refuse before
    /// it writes anything at all.
    fn logger(dir: &Path) -> Arc<Logger> {
        let cfg = Config::for_data_dir(dir);
        Arc::new(Logger::from_config(&cfg.log, None).unwrap())
    }

    fn change() -> ChangeSet {
        let gap = Gap {
            gap_id: Ulid::from_parts(1_700_000_000_000, 1),
            kind: "capability".to_string(),
            statement: "s".to_string(),
            evidence: vec![Ulid::from_parts(1_700_000_000_000, 2)],
            target_uri: "https://metamind.dev/code/module/x".to_string(),
            target_path: PathBuf::from("x"),
        };
        from_gap(
            &gap,
            vec![
                Patch::create("modules/x/one.txt", "one"),
                Patch::create("modules/x/nested/two.txt", "two"),
            ],
            vec![TestId::new("t")],
            vec![BenchmarkId::new("b")],
            &UlidFactory::new(),
        )
        .unwrap()
    }

    #[test]
    fn containment_is_lexical_and_catches_the_escape_attempts() {
        let root = Path::new("/repo/data/sandbox/01h");
        assert!(is_inside(root, Path::new("/repo/data/sandbox/01h/a/b.txt")));
        assert!(is_inside(root, root));
        assert!(!is_inside(
            root,
            Path::new("/repo/data/sandbox/01h/../other")
        ));
        assert!(!is_inside(root, Path::new("/repo/data/sandbox")));
        assert!(!is_inside(root, Path::new("/etc/passwd")));
        assert!(!is_inside(
            root,
            Path::new("/repo/data/sandbox/01h/./../../production.txt")
        ));
    }

    #[test]
    fn a_sandbox_dir_refuses_a_path_that_resolves_outside_it() {
        let dir = SandboxDir::new(
            Ulid::from_parts(1_700_000_000_000, 1),
            "/repo/data/sandbox/01h",
        );
        assert!(dir.resolve(Path::new("modules/x/a.txt")).is_ok());
        let error = dir
            .resolve(Path::new("../02h/target.txt"))
            .expect_err("an escape must be refused");
        assert_eq!(error.code(), "sandbox.escape");
        assert!(dir.resolve(Path::new("../../production.txt")).is_err());
    }

    #[test]
    fn lexical_resolution_folds_dots_without_touching_the_filesystem() {
        assert_eq!(
            lexical_resolve(Path::new("/a/./b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(lexical_resolve(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(
            lexical_resolve(Path::new("a/b/../../../c")),
            PathBuf::from("c")
        );
    }

    #[test]
    fn the_test_totals_are_summed_over_every_target() {
        let text = "running 3 tests\ntest a ... ok\n\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n\n\ntest result: ok. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s\n";
        let (passed, failed) = parse_test_totals(text);
        assert_eq!(passed, 7);
        assert_eq!(failed, 1);
        assert_eq!(parse_test_totals("no tests ran"), (0, 0));
    }

    #[test]
    fn a_tail_is_bounded() {
        let text = (0..50)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(tail(&text, 3), "47\n48\n49");
        assert_eq!(tail("one line", 5), "one line");
    }

    #[tokio::test]
    async fn applying_a_change_set_writes_only_inside_the_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data/sandbox");
        let sandbox = WorktreeSandbox::new(
            dir.path(),
            &root,
            logger(dir.path()),
            Arc::new(UlidFactory::new()),
        );
        let target = SandboxDir::new(
            Ulid::from_parts(1_700_000_000_000, 7),
            root.join("01h00000000000000000000007"),
        );
        std::fs::create_dir_all(&target.root).unwrap();
        let touched = sandbox.apply(&target, &change()).await.unwrap();
        assert_eq!(touched.len(), 2);
        assert!(target.root.join("modules/x/one.txt").exists());
        assert!(target.root.join("modules/x/nested/two.txt").exists());
        assert_eq!(
            std::fs::read_to_string(target.root.join("modules/x/one.txt")).unwrap(),
            "one"
        );
        // Nothing outside the sandbox was created by `apply`: the temp root holds only the
        // sandbox tree and this test's own log directory (`Config::for_data_dir` puts the
        // JSONL sink at `<root>/logs`, and the harness built the logger).
        let stray: Vec<PathBuf> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                name != "data" && name != "logs"
            })
            .collect();
        assert!(stray.is_empty(), "{stray:?}");
    }

    #[tokio::test]
    async fn a_delete_patch_removes_the_path_and_tolerates_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        let sandbox = WorktreeSandbox::new(
            dir.path(),
            &root,
            logger(dir.path()),
            Arc::new(UlidFactory::new()),
        );
        let target = SandboxDir::new(Ulid::from_parts(1_700_000_000_000, 8), root.clone());
        std::fs::create_dir_all(&target.root).unwrap();
        std::fs::write(target.root.join("gone.txt"), "x").unwrap();

        let mut change = change();
        change.code = vec![Patch::delete("gone.txt"), Patch::delete("never-there.txt")];
        change.rollback.state_hash = crate::changeset::patches_state_hash(&change.code);
        sandbox.apply(&target, &change).await.unwrap();
        assert!(!target.root.join("gone.txt").exists());
    }

    #[test]
    fn a_worktree_prepare_reports_git_failure_rather_than_falling_back() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let sandbox = WorktreeSandbox::new(
            dir.path(),
            dir.path().join("sandbox"),
            logger(dir.path()),
            Arc::new(UlidFactory::new()),
        );
        // `dir` is not a git repository, so `git worktree add` fails and the refusal
        // must be the git one — never a silently created plain directory.
        let error = runtime.block_on(sandbox.prepare(&change())).unwrap_err();
        assert_eq!(error.code(), "git");
        assert!(error.to_string().contains("worktree"), "{error}");
    }

    #[test]
    fn the_audit_reports_a_write_outside_the_sandbox_and_skips_build_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let production = dir.path().join("repo");
        let sandbox = production.join("data/sandbox/01h");
        std::fs::create_dir_all(&sandbox).unwrap();
        std::fs::create_dir_all(production.join("target")).unwrap();
        std::fs::create_dir_all(production.join("crates")).unwrap();
        std::fs::create_dir_all(production.join(".git")).unwrap();
        std::fs::write(production.join("crates/production.rs"), "// old").unwrap();

        // Everything was just created, so an audit from "now" sees nothing, and an
        // audit from before the files existed sees the production file but neither the
        // sandbox write nor the build output.
        let now = SystemTime::now();
        let clean = audit_writes(&production, &sandbox, now);
        assert!(clean.outside.is_empty(), "{clean:?}");
        assert!(clean.complete);

        let past = now - std::time::Duration::from_secs(3600);
        std::fs::write(sandbox.join("candidate.txt"), "new").unwrap();
        std::fs::write(production.join("target/artifact.bin"), "build").unwrap();
        std::fs::write(production.join(".git/index.lock"), "git").unwrap();
        let audit = audit_writes(&production, &sandbox, past);
        assert_eq!(
            audit.outside,
            vec![production.join("crates/production.rs")],
            "only production is reported"
        );
        assert!(audit.complete);
        assert!(!audit.is_clean());
        // And on an unchanged tree the same audit is clean, so the check is about
        // change rather than about the tree being empty.
        let after = audit_writes(&production, &sandbox, now);
        assert!(after.is_clean(), "{after:?}");
    }
}
