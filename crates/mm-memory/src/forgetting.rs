//! Deliberate forgetting: retention decay, with a hard refusal for what must last.
//!
//! This module exists to make one thing false: that memory only ever grows. A
//! body that keeps everything remembers nothing in particular, so records below a
//! retention threshold are *archived* — moved out of retrieval and kept for
//! provenance.
//!
//! What it must never do is delete. Three protections are checked before any
//! archival, and each one is checked rather than assumed:
//!
//! 1. `protected = 1` — the record carries an explicit do-not-forget mark.
//! 2. `kind = developmental` — the immutable ledger Phase 4 forbids rewriting.
//! 3. **An open commitment.** A memory that serves a commitment that has not
//!    reached a terminal status is still needed, however old it is.
//!
//! Every refusal is recorded twice: once as `memory.protected_refusal` naming what
//! protected the record, and once as a `memory.forget` with `action = "skip"`. A
//! dry run computes exactly the same refusals and writes nothing else, which is
//! what makes `memory forget --dry-run` a usable preview.

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, LogRecord, Logger};
use serde_json::json;

use crate::error::{ProtectedRefusal, Result};
use crate::model::{ForgetReport, MemoryFilter};
use crate::store::{MemoryStore, SqliteMemoryStore};
use crate::utility::{retention_score, WEEK_NS};

/// The default retention floor.
pub const DEFAULT_THRESHOLD: f64 = 0.15;
/// The default half-life.
pub const DEFAULT_HALF_LIFE_NS: u64 = WEEK_NS;

/// How a forget cycle decides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForgetPolicy {
    /// Records whose retention falls below this are archived.
    pub min_retention: f64,
    /// The retention half-life, in nanoseconds.
    pub half_life_ns: u64,
    /// When true, nothing is written but the decisions are still made and logged.
    pub dry_run: bool,
}

impl Default for ForgetPolicy {
    fn default() -> Self {
        ForgetPolicy {
            min_retention: DEFAULT_THRESHOLD,
            half_life_ns: DEFAULT_HALF_LIFE_NS,
            dry_run: false,
        }
    }
}

/// Run one forget cycle.
///
/// Returns what was archived and what was refused; in a dry run `archived` lists
/// what *would* be archived and nothing in the store changes.
pub async fn forget_cycle(
    store: &SqliteMemoryStore,
    logger: &Logger,
    ids: &UlidFactory,
    now: Timestamp,
    policy: ForgetPolicy,
) -> Result<ForgetReport> {
    let active = store.list(&MemoryFilter::all()).await?;
    let mut report = ForgetReport::new(now, policy.dry_run);
    report.considered = active.len();

    for memory in &active {
        // The cycle never touches the archival tier's counterpart: a memory already
        // archived is not reconsidered, so a run is idempotent.
        if !memory.is_active() {
            continue;
        }
        let age = now
            .as_nanos()
            .saturating_sub(memory.recorded_at.as_nanos()) as u64;
        let access_count = store.access_count(&memory.id).await?;
        let retention = retention_score(age, policy.half_life_ns, memory.importance, access_count);
        if !policy.dry_run {
            store.set_scores(memory.id, None, Some(retention)).await?;
        }
        if retention >= policy.min_retention {
            report.retained += 1;
            continue;
        }

        // Below threshold. Everything past this point is either a logged refusal or
        // an archival — never a silent skip.
        if let Some(blocking) = protection_of(store, memory).await? {
            let refusal = ProtectedRefusal {
                memory_id: mm_core::ulid_string(&memory.id),
                kind: memory.kind.as_str().to_string(),
                attempted_action: "archive".to_string(),
                blocking_reference: blocking,
                retention: format!("{retention:.6}"),
            };
            logger
                .audit(
                    Level::Warn,
                    codes::MEMORY_PROTECTED_REFUSAL,
                    crate::TARGET,
                    Some(memory.id),
                    json!({
                        "memory_id": refusal.memory_id,
                        "kind": refusal.kind,
                        "attempted_action": refusal.attempted_action,
                        "blocking_reference": refusal.blocking_reference,
                    }),
                )
                .await?;
            logger
                .emit(
                    LogRecord::new(Level::Info, codes::MEMORY_FORGET, crate::TARGET)
                        .with_trace(memory.id)
                        .with_field("memory_id", refusal.memory_id.clone())
                        .with_field("utility", serde_json::Value::Null)
                        .with_field("retention", retention)
                        .with_field("action", "skip")
                        .with_field("reason", refusal.blocking_reference.clone()),
                )
                .await?;
            report.refused.push(refusal);
            continue;
        }

        report.archived.push(memory.id);
        if !policy.dry_run {
            store.archive(memory.id).await?;
        }
        logger
            .audit(
                Level::Info,
                codes::MEMORY_FORGET,
                crate::TARGET,
                Some(memory.id),
                json!({
                    "memory_id": mm_core::ulid_string(&memory.id),
                    "utility": serde_json::Value::Null,
                    "retention": retention,
                    "action": "archive",
                    "reason": "below retention threshold",
                }),
            )
            .await?;
        let _ = ids;
    }
    Ok(report)
}

/// What protects a record from archival, if anything.
async fn protection_of(
    store: &SqliteMemoryStore,
    memory: &crate::model::Memory,
) -> Result<Option<String>> {
    if memory.is_developmental() {
        return Ok(Some("developmental".to_string()));
    }
    if memory.protected {
        return Ok(Some("protected".to_string()));
    }
    if let Some(commitment) = store.open_commitment_for(&memory.id).await? {
        return Ok(Some(format!("commitment:{commitment}")));
    }
    Ok(None)
}

/// Compute the retention of a record without touching the store.
pub fn retention_of(
    recorded_at: Timestamp,
    now: Timestamp,
    half_life_ns: u64,
    importance: f32,
    access_count: u32,
) -> f64 {
    let age = now.as_nanos().saturating_sub(recorded_at.as_nanos()) as u64;
    retention_score(age, half_life_ns, importance, access_count)
}

/// The memories a cycle would archive, without writing anything.
pub async fn preview(
    store: &SqliteMemoryStore,
    now: Timestamp,
    policy: ForgetPolicy,
) -> Result<Vec<(Ulid, f64)>> {
    let active = store.list(&MemoryFilter::all()).await?;
    let mut out = Vec::new();
    for memory in &active {
        let access_count = store.access_count(&memory.id).await?;
        let retention = retention_of(
            memory.recorded_at,
            now,
            policy.half_life_ns,
            memory.importance,
            access_count,
        );
        if retention < policy.min_retention {
            out.push((memory.id, retention));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_policy_is_deliberate_not_aggressive() {
        let policy = ForgetPolicy::default();
        assert!((policy.min_retention - 0.15).abs() < 1e-12);
        assert_eq!(policy.half_life_ns, WEEK_NS);
        assert!(!policy.dry_run);
    }

    #[test]
    fn retention_of_uses_the_same_curve_as_the_utility_module() {
        let at = Timestamp::from_epoch_seconds(1_000_000);
        let now = Timestamp::from_epoch_seconds(1_000_000 + 7 * 86_400);
        assert_eq!(
            retention_of(at, now, WEEK_NS, 0.0, 0),
            retention_score(7 * 86_400 * 1_000_000_000, WEEK_NS, 0.0, 0)
        );
        // A record from the future cannot produce a negative age.
        assert_eq!(retention_of(now, at, WEEK_NS, 1.0, 0), 1.0);
    }
}
