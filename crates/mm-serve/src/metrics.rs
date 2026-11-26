//! Minimal in-process counters.
//!
//! The old metrics registry retired with the controller. These counters are
//! exposure-only and never drive behavior.

use std::sync::atomic::{AtomicU64, Ordering};

/// The counters the health route exposes.
#[derive(Debug, Default)]
pub struct Metrics {
    turns_started: AtomicU64,
    turns_completed: AtomicU64,
    turns_failed: AtomicU64,
}

impl Metrics {
    /// Record a started turn.
    pub fn turn_started(&self) {
        self.turns_started.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a completed turn.
    pub fn turn_completed(&self) {
        self.turns_completed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a failed turn.
    pub fn turn_failed(&self) {
        self.turns_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Turns started.
    pub fn started(&self) -> u64 {
        self.turns_started.load(Ordering::Relaxed)
    }

    /// Turns completed.
    pub fn completed(&self) -> u64 {
        self.turns_completed.load(Ordering::Relaxed)
    }

    /// Turns failed.
    pub fn failed(&self) -> u64 {
        self.turns_failed.load(Ordering::Relaxed)
    }
}
