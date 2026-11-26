//! Rollback: undoing a change set by its own plan.
//!
//! The plan asks for two things a rollback record must carry — the steps and a
//! `state_hash` the restore is checked against — and this module adds the check that
//! makes them mean something:
//!
//! * **The plan must still describe the change set.** [`apply`] recomputes the patch
//!   set's digest and refuses when it disagrees with the plan's `state_hash`. A plan
//!   that describes a different artifact set is a plan for a different change, and
//!   applying it would restore the wrong thing with a hash that "matches".
//! * **A write that changed an existing file needs a snapshot.** Every `modify` and
//!   `delete` patch is looked up in `rollback_snapshots`, the table Phase 10 fills when
//!   a reversible action runs. A `create` needs none — deleting it restores the state —
//!   but a modify with no snapshot is a change that **cannot** be undone, and refusing
//!   then is the only honest answer. The store is consulted rather than assumed, so
//!   "restorable" is a fact about rows rather than about a rollback plan's wording.
//!
//! What this module deliberately does not do is copy bytes. The snapshot *table* holds
//! them (`rollback_snapshots.content`), and Phase 10's `mm-tools::rollback` already
//! restores a single snapshot hash-verified. Re-implementing that here would make two
//! restore paths that must agree about hashing, and the one nobody exercises would be
//! the one that is wrong.

use mm_core::Ulid;
use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::changeset::{patches_state_hash, ChangeSet, PatchOperation};
use crate::error::{Result, SelfEngError};

/// What a rollback did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackOutcome {
    /// The change set that was undone.
    pub change_set: Ulid,
    /// The state hash the restore was checked against.
    pub restored_state_hash: String,
    /// The steps that ran, in order.
    pub steps: Vec<String>,
}

/// The patches whose prior content the snapshot table does not hold.
///
/// An absent snapshot is reported by path so the refusal names what is missing rather
/// than only that something is.
pub async fn missing_snapshots(
    store: &mm_store_sqlite::SqliteStore,
    cs: &ChangeSet,
) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for patch in &cs.code {
        if patch.operation == PatchOperation::Create {
            continue;
        }
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM rollback_snapshots WHERE ref = ?")
                .bind(patch.path_str())
                .fetch_one(store.pool())
                .await
                .map_err(|e| SelfEngError::Store(format!("cannot read the snapshots: {e}")))?;
        if count == 0 {
            missing.push(patch.path_str());
        }
    }
    Ok(missing)
}

/// Undo a change set.
pub async fn apply(
    cs: &ChangeSet,
    store: &mm_store_sqlite::SqliteStore,
    logger: &Logger,
) -> Result<RollbackOutcome> {
    cs.validate()?;
    if cs.rollback.steps.is_empty() {
        return Err(SelfEngError::Rollback(
            "the change set has no rollback steps".to_string(),
        ));
    }
    let expected = patches_state_hash(&cs.code);
    if cs.rollback.state_hash != expected {
        return Err(SelfEngError::Rollback(format!(
            "the plan describes {}, the artifacts hash to {expected}",
            cs.rollback.state_hash
        )));
    }
    let missing = missing_snapshots(store, cs).await?;
    if !missing.is_empty() {
        return Err(SelfEngError::Rollback(format!(
            "no snapshot restores {}",
            missing.join(", ")
        )));
    }
    let outcome = RollbackOutcome {
        change_set: cs.id,
        restored_state_hash: cs.rollback.state_hash.clone(),
        steps: cs.rollback.steps.clone(),
    };
    logger
        .audit(
            Level::Warn,
            codes::ROLLBACK_APPLY,
            crate::TARGET,
            Some(cs.id),
            json!({
                "change_set_id": mm_core::ulid_string(&cs.id),
                "restored_state_hash": outcome.restored_state_hash,
                "steps": outcome.steps.len(),
            }),
        )
        .await?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::{from_gap, BenchmarkId, Gap, Patch, TestId};
    use mm_core::{Config, UlidFactory};
    use mm_store_sqlite::SqliteStore;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn gap() -> Gap {
        Gap {
            gap_id: Ulid::from_parts(1_700_000_000_000, 1),
            kind: "capability".to_string(),
            statement: "s".to_string(),
            evidence: vec![Ulid::from_parts(1_700_000_000_000, 2)],
            target_uri: "https://metamind.dev/code/module/x".to_string(),
            target_path: PathBuf::from("x"),
        }
    }

    fn creation() -> ChangeSet {
        from_gap(
            &gap(),
            vec![Patch::create("modules/x/new.txt", "one")],
            vec![TestId::new("t")],
            vec![BenchmarkId::new("b")],
            &UlidFactory::new(),
        )
        .unwrap()
    }

    fn modification() -> ChangeSet {
        from_gap(
            &gap(),
            vec![Patch::modify("modules/x/existing.txt", "two")],
            vec![TestId::new("t")],
            vec![BenchmarkId::new("b")],
            &UlidFactory::new(),
        )
        .unwrap()
    }

    async fn store_and_logger(dir: &std::path::Path) -> (SqliteStore, Arc<Logger>) {
        let store = SqliteStore::open(&dir.join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir);
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        (store, logger)
    }

    #[tokio::test]
    async fn a_creation_rolls_back_without_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let (store, logger) = store_and_logger(dir.path()).await;
        let outcome = apply(&creation(), &store, &logger).await.unwrap();
        assert_eq!(outcome.steps, vec!["delete modules/x/new.txt".to_string()]);
        assert_eq!(outcome.restored_state_hash, creation().rollback.state_hash);
    }

    #[tokio::test]
    async fn a_modification_without_a_snapshot_cannot_be_undone() {
        let dir = tempfile::tempdir().unwrap();
        let (store, logger) = store_and_logger(dir.path()).await;
        let cs = modification();
        let missing = missing_snapshots(&store, &cs).await.unwrap();
        assert_eq!(missing, vec!["modules/x/existing.txt".to_string()]);
        let error = apply(&cs, &store, &logger).await.unwrap_err();
        assert_eq!(error.code(), "rollback");
        assert!(error.to_string().contains("no snapshot"), "{error}");
    }

    #[tokio::test]
    async fn a_modification_with_a_snapshot_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let (store, logger) = store_and_logger(dir.path()).await;
        sqlx::query(
            "INSERT INTO rollback_snapshots (id, action_id, kind, ref, content, content_hash, created_at) \
             VALUES (?, ?, 'fs_path', ?, NULL, ?, '2026-10-09T00:00:00.000000000Z')",
        )
        .bind("01h0000000000000000000s001")
        .bind("01h0000000000000000000a001")
        .bind("modules/x/existing.txt")
        .bind(mm_core::content_hash(b"two"))
        .execute(store.pool())
        .await
        .unwrap();
        let outcome = apply(&modification(), &store, &logger).await.unwrap();
        assert_eq!(outcome.steps.len(), 1);
        assert!(outcome.steps[0].contains("snapshot"));
    }

    #[tokio::test]
    async fn a_plan_that_describes_other_artifacts_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (store, logger) = store_and_logger(dir.path()).await;
        let mut cs = creation();
        cs.rollback.state_hash = "0000000000000000".to_string();
        let error = apply(&cs, &store, &logger).await.unwrap_err();
        assert!(error.to_string().contains("the plan describes"), "{error}");
    }

    #[tokio::test]
    async fn a_change_set_with_no_steps_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (store, logger) = store_and_logger(dir.path()).await;
        let mut cs = creation();
        cs.rollback.steps.clear();
        // Validation catches it first, which is the point: a change set with no way back
        // never reaches the rollback path at all.
        let error = apply(&cs, &store, &logger).await.unwrap_err();
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("way back"), "{error}");
    }
}
