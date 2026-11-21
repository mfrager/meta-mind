//! Phase 7 migration from the legacy `mm-serve` data root.
//!
//! The old console kept a SQLite file at `metamind.db`, an Oxigraph/RocksDB
//! directory at `graph/`, and chat sessions in `chat-sessions.json`. The data
//! layer keeps the same information under `sqlite/metamind.db`,
//! `cubecast/main/` and agentstream sessions. The migration copies before
//! opening the new root,
//! then imports the old chat sessions into agentstream so their identifiers and
//! titles survive a cutover.
//!
//! It is intentionally idempotent: an existing destination file/directory is
//! left alone, and importing an already-present session is a no-op.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agentstream::SessionMetadata;
use crate::config::{create_directory_structure, InitConfig};
use crate::error::{DataError, Result};
use crate::init::initialize_data_layer;

/// What the migration copied or imported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// `metamind.db` became `sqlite/metamind.db`.
    pub sqlite_copied: bool,
    /// The legacy RDF directory became `cubecast/main`.
    pub graph_copied: bool,
    /// Legacy user/project/app home directories copied.
    pub homes_copied: usize,
    /// Chat sessions imported into agentstream.
    pub sessions_imported: usize,
    /// Chat sessions that were already present.
    pub sessions_existing: usize,
}

/// Copy a legacy `mm-serve` data root into a data-layer root.
pub async fn migrate_from_mm_serve(
    old_data_dir: &Path,
    new_base_dir: &Path,
) -> Result<MigrationReport> {
    if old_data_dir == new_base_dir {
        return Err(DataError::Config(
            "old and new data roots must be different".to_owned(),
        ));
    }
    if !old_data_dir.is_dir() {
        return Err(DataError::Config(format!(
            "legacy data root {} does not exist",
            old_data_dir.display()
        )));
    }
    create_directory_structure(new_base_dir)?;

    let sqlite_copied = copy_sqlite(old_data_dir, new_base_dir)?;
    let graph_copied = copy_graph(old_data_dir, new_base_dir)?;
    let mut homes_copied = 0;
    for name in ["users", "projects", "apps"] {
        if copy_tree(&old_data_dir.join(name), &new_base_dir.join(name))? {
            homes_copied += 1;
        }
    }
    let mut report = MigrationReport {
        sqlite_copied,
        graph_copied,
        homes_copied,
        ..MigrationReport::default()
    };

    // The old sessions are read before opening the new root: initialize writes
    // its own config and may create the agentstream index.
    let sessions = read_legacy_sessions(old_data_dir)?;
    let data = initialize_data_layer(InitConfig {
        base_dir: Some(new_base_dir.to_path_buf()),
        ..InitConfig::default()
    })
    .await?;

    for (session_id, title) in sessions {
        if data
            .memory_system
            .agentstream
            .session(&session_id)
            .await?
            .is_some()
        {
            report.sessions_existing += 1;
            continue;
        }
        data.memory_system
            .ensure_session(
                &session_id,
                SessionMetadata {
                    user_id: "console-operator".to_owned(),
                    project_id: None,
                    title,
                },
            )
            .await?;
        report.sessions_imported += 1;
    }

    Ok(report)
}

fn copy_sqlite(old: &Path, new: &Path) -> Result<bool> {
    let source = old.join("metamind.db");
    let destination = new.join("sqlite").join(crate::sqlite::MAIN_DB);
    if !source.is_file() || destination.exists() {
        return Ok(false);
    }
    std::fs::copy(source, destination)?;
    Ok(true)
}

fn copy_graph(old: &Path, new: &Path) -> Result<bool> {
    copy_tree(&old.join("graph"), &new.join("cubecast").join("main"))
}

/// Copy the contents of `source` into `destination` when `destination` is empty.
fn copy_tree(source: &Path, destination: &Path) -> Result<bool> {
    if !source.is_dir() {
        return Ok(false);
    }
    let mut entries = std::fs::read_dir(destination)
        .map(|entries| entries.filter_map(std::result::Result::ok))
        .into_iter()
        .flatten();
    if entries.next().is_some() {
        return Ok(false);
    }
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(true)
}

fn read_legacy_sessions(old: &Path) -> Result<BTreeMap<String, Option<String>>> {
    let path = old.join("chat-sessions.json");
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let disk: LegacyDisk = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(disk
        .sessions
        .into_iter()
        .map(|(id, session)| (id, session.title))
        .collect())
}

#[derive(Deserialize)]
struct LegacyDisk {
    #[serde(default)]
    sessions: BTreeMap<String, LegacySession>,
}

#[derive(Deserialize)]
struct LegacySession {
    #[serde(default)]
    title: Option<String>,
}

/// The paths a migration reads. Kept public for operators writing a preflight
/// report without opening either root.
pub fn legacy_paths(old_data_dir: &Path) -> Vec<PathBuf> {
    vec![
        old_data_dir.join("metamind.db"),
        old_data_dir.join("graph"),
        old_data_dir.join("chat-sessions.json"),
    ]
}
