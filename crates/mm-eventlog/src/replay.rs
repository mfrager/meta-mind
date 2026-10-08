//! Replay.
//!
//! Replaying means: abort anything left `provisional`, then apply every
//! committed event in `seq` order and fold their hashes into one value. The fold
//! is a pure function of the log, so replaying the same log always yields the
//! same `log_hash` — which is what makes "the projection is reproducible"
//! checkable rather than assumed.

use mm_core::{MmError, Ulid};

use crate::record::EventRecord;

/// A projection that can be rebuilt from the log.
///
/// `apply` must be deterministic and idempotent per event: replay calls it for
/// each committed event exactly once, and the same event must always produce the
/// same state.
pub trait EventApplier {
    /// Apply one committed event.
    fn apply(&mut self, record: &EventRecord) -> Result<(), MmError>;

    /// A hash of the projection's own state, if it can compute one.
    ///
    /// Returning an empty string means "this applier has no independent state
    /// hash"; the log fold is still checked. An applier that *can* hash its state
    /// should, because then a replay failure localizes to the projection rather
    /// than to the log.
    fn state_hash(&self) -> String {
        String::new()
    }
}

/// An applier that only counts what it was given. Used by `mm-cli replay` and by
/// tests that check the log itself rather than a projection.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountingApplier {
    /// Every event applied, in order.
    pub applied: Vec<Ulid>,
    /// Every event sequence applied, in order.
    pub sequences: Vec<i64>,
}

impl CountingApplier {
    /// A fresh applier.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many events were applied.
    pub fn len(&self) -> usize {
        self.applied.len()
    }

    /// Whether nothing was applied.
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }
}

impl EventApplier for CountingApplier {
    fn apply(&mut self, record: &EventRecord) -> Result<(), MmError> {
        self.applied.push(record.id);
        self.sequences.push(record.seq);
        Ok(())
    }

    fn state_hash(&self) -> String {
        mm_core::content_hash(
            self.sequences
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",")
                .as_bytes(),
        )
    }
}

/// The result of replaying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSnapshot {
    /// How many events were applied.
    pub applied: u64,
    /// The last sequence applied.
    pub last_seq: i64,
    /// The fold of every applied event's hash.
    pub log_hash: String,
    /// The applier's own state hash (empty when it does not provide one).
    pub applier_hash: String,
    /// Events that were `provisional` and had to be aborted first.
    pub aborted: Vec<Ulid>,
}

/// The fold: `h(0) = sha256("")`, `h(n) = sha256(h(n-1) | event.hash)`.
///
/// Seeded with the empty content hash so an empty log has a defined, non-empty
/// value rather than the empty string.
pub fn fold_hashes<'a>(hashes: impl IntoIterator<Item = &'a str>) -> String {
    let mut current = mm_core::content_hash(b"");
    for hash in hashes {
        current = mm_core::hash_fields(&[&current, hash]);
    }
    current
}

/// The `log_hash` of an already-applied event sequence.
pub fn fold_records(records: &[EventRecord]) -> String {
    fold_hashes(records.iter().map(|r| r.hash.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fold_is_order_sensitive_and_seeded() {
        let a = fold_hashes(["a", "b"]);
        let b = fold_hashes(["b", "a"]);
        let none = fold_hashes(std::iter::empty());
        assert_ne!(a, b);
        assert_ne!(a, none);
        assert_eq!(none.len(), 64);
        assert_eq!(a, fold_hashes(["a", "b"]), "the fold must be deterministic");
    }

    #[test]
    fn counting_applier_records_order_and_hashes_it() {
        let mut applier = CountingApplier::new();
        assert!(applier.is_empty());
        let ids = mm_core::UlidFactory::new();
        let record = |seq: i64| EventRecord {
            seq,
            id: ids.next(),
            kind: mm_core::EventKind::Custom,
            payload: serde_json::json!({}),
            status: crate::record::EventStatus::Committed,
            correlation: None,
            system_from: mm_core::Timestamp::EPOCH,
            created_at: mm_core::Timestamp::EPOCH,
            hash: format!("h{seq}"),
        };
        applier.apply(&record(1)).unwrap();
        applier.apply(&record(2)).unwrap();
        assert_eq!(applier.len(), 2);
        assert_eq!(applier.sequences, vec![1, 2]);
        assert_ne!(applier.state_hash(), CountingApplier::new().state_hash());
    }
}
