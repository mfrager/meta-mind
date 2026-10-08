//! Shared helpers for the end-to-end tests.
//!
//! The tests drive the kernel the way an operator does: open the stores in a
//! throwaway data directory, and run the real `mm-cli` binary against it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mm_core::Config;

/// A throwaway data directory.
pub struct Workspace {
    dir: tempfile::TempDir,
    /// The data directory inside it.
    pub data_dir: PathBuf,
}

impl Workspace {
    /// Create a fresh workspace.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().join("data");
        Workspace { dir, data_dir }
    }

    /// Kernel configuration pointing at this workspace.
    pub fn config(&self) -> Config {
        Config::for_data_dir(&self.data_dir)
    }

    /// The workspace root (where the data directory lives).
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The repository root, used to locate fixtures and the CLI binary.
    pub fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .canonicalize()
            .expect("repository root")
    }

    /// A fixture file's contents.
    pub fn fixture(relative: &str) -> String {
        let path = Workspace::repo_root().join("tests/fixtures").join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace::new()
    }
}

/// The path to the built `mm-cli` binary.
///
/// Derived from this test binary's own location (`target/<profile>/deps/…`), so
/// it works for `cargo test` without depending on the caller's working directory.
pub fn mm_cli() -> PathBuf {
    if let Ok(explicit) = std::env::var("MM_CLI_BIN") {
        return PathBuf::from(explicit);
    }
    let exe = std::env::current_exe().expect("current exe");
    let target_profile = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>")
        .to_path_buf();
    let candidate = target_profile.join("mm-cli");
    assert!(
        candidate.exists(),
        "{} is missing; run `cargo build --workspace` first",
        candidate.display()
    );
    candidate
}

/// Run `mm-cli` with a data directory and return its output.
pub fn run_cli(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(mm_cli())
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .expect("spawn mm-cli")
}

/// Run `mm-cli` and require success.
pub fn run_cli_ok(data_dir: &Path, args: &[&str]) -> String {
    let output = run_cli(data_dir, args);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "mm-cli {args:?} failed ({}):\n{stdout}\n{stderr}",
        output.status
    );
    stdout
}

/// The value of a `  key   value` line in command output.
pub fn field(stdout: &str, key: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let trimmed = line.trim_start();
        let rest = trimmed.strip_prefix(key)?;
        Some(rest.trim().to_string())
    })
}

/// Parse a `state_hash <hex>` style line.
pub fn value_after(stdout: &str, key: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let first = parts.next()?;
        (first == key).then(|| parts.next().unwrap_or_default().to_string())
    })
}
