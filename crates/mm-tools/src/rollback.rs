//! Rollback: undo a reversible action, or refuse to pretend.
//!
//! The third invariant is "every action is auditable, and either reversible or
//! explicitly marked irreversible". This module is the *either*: [`snapshot`] captures
//! what an action is about to change, [`restore`] puts it back, and an
//! [`Reversibility::Irreversible`] tool is refused a snapshot at all — because a
//! snapshot taken for an action that cannot be undone would be a lie with a hash on it.
//!
//! # What a snapshot is
//!
//! The file's *bytes*, in the `rollback_snapshots.content` column. Not a diff, not a
//! copy on disk: a diff can be applied to the wrong base, and a copy can be edited
//! between the snapshot and the restore. Byte-identical restore is then checkable
//! rather than asserted — [`restore`] re-hashes what it wrote and returns
//! [`RollbackError::NotByteIdentical`] naming both hashes when they disagree, so a
//! failed rollback is a typed failure and never a silent success.
//!
//! # A snapshot id is derived, not drawn
//!
//! `snapshot(action_id, path)` is idempotent: the row id is a hash of the action and
//! the path, so calling it twice updates the same row instead of stacking two. That
//! matters because the executor snapshots on the way in and a retry walks the same
//! path again; the older copy would be the *already written* file, and restoring it
//! would restore the change.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::RollbackError;
use crate::spec::Reversibility;
use mm_core::{Timestamp, Ulid};
use mm_store_sqlite::SqliteStore;

/// What kind of thing a snapshot can put back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotKind {
    /// A file's bytes.
    FsPath,
    /// A single row, identified by table and primary key.
    SqlRow,
    /// A quad, identified by its subject.
    GraphQuad,
    /// Nothing: the action changed nothing outside its own call.
    None,
}

/// Every kind, in the order the CLI renders them.
pub const SNAPSHOT_KINDS: [SnapshotKind; 4] = [
    SnapshotKind::FsPath,
    SnapshotKind::SqlRow,
    SnapshotKind::GraphQuad,
    SnapshotKind::None,
];

impl SnapshotKind {
    /// The stable wire name, which is also the `rollback_snapshots.kind` value.
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotKind::FsPath => "fs_path",
            SnapshotKind::SqlRow => "sql_row",
            SnapshotKind::GraphQuad => "graph_quad",
            SnapshotKind::None => "none",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        SNAPSHOT_KINDS
            .into_iter()
            .find(|kind| kind.as_str() == text.trim().to_ascii_lowercase())
    }
}

impl std::fmt::Display for SnapshotKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A snapshot's ULID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnapshotId(pub Ulid);

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&mm_core::ulid_string(&self.0))
    }
}

/// One recorded snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Its id.
    pub id: SnapshotId,
    /// The action it was taken for.
    pub action_id: Ulid,
    /// What kind of thing it is.
    pub kind: SnapshotKind,
    /// The path (or, for the other kinds, the reference) it restores.
    pub path: String,
    /// The hash of the bytes at the moment they were captured.
    pub content_hash: String,
    /// When it was taken.
    pub created_at: Timestamp,
    /// How many bytes were captured.
    pub bytes: u64,
}

/// The deterministic row id for an action's snapshot of a path.
fn snapshot_id(action_id: &Ulid, path: &str) -> SnapshotId {
    let digest = mm_core::content_hash(
        format!(
            "mm.tools.snapshot\u{1f}{}\u{1f}{path}",
            mm_core::ulid_string(action_id)
        )
        .as_bytes(),
    );
    let bytes = digest.as_bytes();
    let mut parts = [0u8; 16];
    for (index, slot) in parts.iter_mut().enumerate() {
        *slot = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
    }
    parts[0] &= 0b0000_0111;
    SnapshotId(Ulid::from_bytes(parts))
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

/// Snapshot a path, when the action is reversible at all.
///
/// Returns `Ok(None)` for a path that does not exist yet: there is nothing to restore,
/// and a write that *creates* the file is rolled back by the executor removing it. The
/// distinction is not cosmetic — a caller that treated `None` as "snapshot failed"
/// would refuse every file-creating action.
pub async fn snapshot(
    sql: &SqliteStore,
    action_id: Ulid,
    reversibility: Reversibility,
    path: &str,
) -> Result<Option<Snapshot>, RollbackError> {
    if reversibility == Reversibility::Irreversible {
        return Err(RollbackError::Irreversible {
            action: mm_core::ulid_string(&action_id),
        });
    }
    let resolved = PathBuf::from(path);
    let bytes = match std::fs::read(&resolved) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(RollbackError::Snapshot {
                path: path.to_string(),
                reason: error.to_string(),
            })
        }
    };
    let content_hash = mm_core::content_hash(&bytes);
    let id = snapshot_id(&action_id, path);
    let at = Timestamp::now();
    sqlx::query(
        "INSERT INTO rollback_snapshots \
         (id, action_id, kind, ref, content, content_hash, created_at) \
         VALUES (?, ?, 'fs_path', ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           content = excluded.content, \
           content_hash = excluded.content_hash, \
           created_at = excluded.created_at",
    )
    .bind(id.to_string())
    .bind(mm_core::ulid_string(&action_id))
    .bind(path)
    .bind(bytes.as_slice())
    .bind(&content_hash)
    .bind(at.to_rfc3339())
    .execute(sql.pool())
    .await
    .map_err(|e| RollbackError::Store(e.to_string()))?;
    Ok(Some(Snapshot {
        id,
        action_id,
        kind: SnapshotKind::FsPath,
        path: path.to_string(),
        content_hash,
        created_at: at,
        bytes: bytes.len() as u64,
    }))
}

/// The snapshots recorded for an action, oldest first.
pub async fn snapshots_for(
    sql: &SqliteStore,
    action_id: &Ulid,
) -> Result<Vec<Snapshot>, RollbackError> {
    let rows: Vec<(String, String, String, String, Option<i64>, String, String)> = sqlx::query_as(
        "SELECT id, action_id, kind, ref, length(content), content_hash, created_at \
         FROM rollback_snapshots WHERE action_id = ? ORDER BY created_at ASC, id ASC",
    )
    .bind(mm_core::ulid_string(action_id))
    .fetch_all(sql.pool())
    .await
    .map_err(|e| RollbackError::Store(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(
            |(id, action, kind, path, bytes, content_hash, created_at)| Snapshot {
                id: SnapshotId(mm_core::id::parse_ulid(&id).unwrap_or_else(|_| Ulid::nil())),
                action_id: mm_core::id::parse_ulid(&action).unwrap_or_else(|_| Ulid::nil()),
                kind: SnapshotKind::parse(&kind).unwrap_or(SnapshotKind::None),
                path,
                content_hash,
                created_at: Timestamp::from_rfc3339(&created_at).unwrap_or(Timestamp::EPOCH),
                bytes: bytes.unwrap_or(0).max(0) as u64,
            },
        )
        .collect())
}

/// Put one snapshot back, returning the hash of what was written.
pub async fn restore(sql: &SqliteStore, id: &SnapshotId) -> Result<String, RollbackError> {
    let row: Option<(String, String, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT kind, ref, content FROM rollback_snapshots WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(sql.pool())
            .await
            .map_err(|e| RollbackError::Store(e.to_string()))?;
    let Some((kind, path, content)) = row else {
        return Err(RollbackError::Unknown(id.to_string()));
    };
    let Some(bytes) = content else {
        return Err(RollbackError::UnsupportedKind(kind));
    };
    let target = PathBuf::from(&path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| RollbackError::Snapshot {
                path: path.clone(),
                reason: e.to_string(),
            })?;
        }
    }
    std::fs::write(&target, &bytes).map_err(|e| RollbackError::Snapshot {
        path: path.clone(),
        reason: e.to_string(),
    })?;
    let found = mm_core::content_hash(&bytes);
    let expected: String =
        sqlx::query_scalar("SELECT content_hash FROM rollback_snapshots WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(sql.pool())
            .await
            .map_err(|e| RollbackError::Store(e.to_string()))?;
    if found != expected {
        return Err(RollbackError::NotByteIdentical {
            path,
            expected,
            found,
        });
    }
    Ok(found)
}

/// Restore every snapshot an action took, newest first, returning the first hash.
pub async fn rollback(sql: &SqliteStore, action_id: &Ulid) -> Result<String, RollbackError> {
    let mut snapshots = snapshots_for(sql, action_id).await?;
    if snapshots.is_empty() {
        return Err(RollbackError::Unknown(mm_core::ulid_string(action_id)));
    }
    snapshots.reverse();
    let mut restored = String::new();
    for snapshot in &snapshots {
        restored = restore(sql, &snapshot.id).await?;
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store(dir: &tempfile::TempDir) -> SqliteStore {
        let sql = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        sql.migrate().await.unwrap();
        sql
    }

    fn action() -> Ulid {
        Ulid::from_parts(1_700_000_000_000, 9)
    }

    #[tokio::test]
    async fn a_reversible_action_restores_byte_identically() {
        let dir = tempfile::tempdir().unwrap();
        let sql = store(&dir).await;
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"before").unwrap();
        let path = target.to_string_lossy().to_string();

        let snap = snapshot(&sql, action(), Reversibility::Reversible, &path)
            .await
            .unwrap()
            .expect("the file exists, so there is something to restore");
        assert_eq!(snap.kind, SnapshotKind::FsPath);
        assert_eq!(snap.bytes, 6);
        std::fs::write(&target, b"after-and-much-longer").unwrap();
        assert_ne!(
            mm_core::content_hash(&std::fs::read(&target).unwrap()),
            snap.content_hash
        );

        let restored = restore(&sql, &snap.id).await.unwrap();
        assert_eq!(restored, snap.content_hash);
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
    }

    #[tokio::test]
    async fn snapshoting_a_missing_file_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sql = store(&dir).await;
        let missing = dir.path().join("not-yet.txt");
        let taken = snapshot(
            &sql,
            action(),
            Reversibility::Reversible,
            &missing.to_string_lossy(),
        )
        .await
        .unwrap();
        assert!(
            taken.is_none(),
            "a file that does not exist has nothing to restore"
        );
        assert!(snapshots_for(&sql, &action()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_irreversible_action_is_refused_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let sql = store(&dir).await;
        let error = snapshot(
            &sql,
            action(),
            Reversibility::Irreversible,
            &dir.path().join("x").to_string_lossy(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), "rollback.irreversible");
    }

    #[tokio::test]
    async fn the_snapshot_id_is_derived_so_a_retry_updates_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let sql = store(&dir).await;
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"one").unwrap();
        let path = target.to_string_lossy().to_string();

        let first = snapshot(&sql, action(), Reversibility::Reversible, &path)
            .await
            .unwrap()
            .unwrap();
        std::fs::write(&target, b"two-longer").unwrap();
        let second = snapshot(&sql, action(), Reversibility::Reversible, &path)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            first.id, second.id,
            "the same action and path is one snapshot"
        );
        assert_ne!(first.content_hash, second.content_hash);
        assert_eq!(snapshots_for(&sql, &action()).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn restoring_an_unknown_snapshot_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let sql = store(&dir).await;
        let error = restore(&sql, &SnapshotId(Ulid::from_parts(1_700_000_000_000, 77)))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "rollback.unknown");
    }

    #[tokio::test]
    async fn rolling_back_an_action_with_no_snapshots_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let sql = store(&dir).await;
        let error = rollback(&sql, &action()).await.unwrap_err();
        assert_eq!(error.code(), "rollback.unknown");
    }

    #[tokio::test]
    async fn rolling_back_an_action_restores_every_path_it_touched() {
        let dir = tempfile::tempdir().unwrap();
        let sql = store(&dir).await;
        let one = dir.path().join("one.txt");
        let two = dir.path().join("two.txt");
        std::fs::write(&one, b"one").unwrap();
        std::fs::write(&two, b"two").unwrap();
        for path in [&one, &two] {
            snapshot(
                &sql,
                action(),
                Reversibility::Reversible,
                &path.to_string_lossy(),
            )
            .await
            .unwrap();
        }
        std::fs::write(&one, b"changed").unwrap();
        std::fs::write(&two, b"changed").unwrap();

        rollback(&sql, &action()).await.unwrap();
        assert_eq!(std::fs::read(&one).unwrap(), b"one");
        assert_eq!(std::fs::read(&two).unwrap(), b"two");
    }

    #[test]
    fn kinds_round_trip() {
        for kind in SNAPSHOT_KINDS {
            assert_eq!(SnapshotKind::parse(kind.as_str()), Some(kind));
            assert_eq!(kind.to_string(), kind.as_str());
        }
        assert_eq!(SnapshotKind::parse("FS_PATH"), Some(SnapshotKind::FsPath));
        assert_eq!(SnapshotKind::parse("nonsense"), None);
    }
}
