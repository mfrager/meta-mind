//! Memory region: drives the four-tier pipeline's lifecycle.
//!
//! The transitions are the contract. The work behind each transition is
//! delegated to `mm-data::MemorySystem` by [`MemoryBridge`](crate::memory::MemoryBridge);
//! this region owns whether a compression, aggregation or sync is in flight, not
//! the records themselves.

use mm_sm::{SmEvent, StateMachine, Transition, TransitionRecord};
use serde_json::Value;

use crate::error::Result;
use crate::regions::RegionSm;

/// Memory lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryState {
    /// Raw turns are being buffered.
    Episodic,
    /// The compressor is running.
    Compressing,
    /// Segments are stored.
    Segmented,
    /// Project aggregation is running.
    ProjectAggregating,
    /// Project memory is updated.
    ProjectSynced,
    /// User learning is running.
    UserSyncing,
    /// User memory is updated.
    UserSynced,
    /// Retention cleanup is running.
    Purging,
}

impl MemoryState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Episodic => "episodic",
            Self::Compressing => "compressing",
            Self::Segmented => "segmented",
            Self::ProjectAggregating => "project_aggregating",
            Self::ProjectSynced => "project_synced",
            Self::UserSyncing => "user_syncing",
            Self::UserSynced => "user_synced",
            Self::Purging => "purging",
        }
    }
}

/// Events the memory region reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum MemoryEvent {
    /// A turn completed and should be buffered.
    TurnCompleted,
    /// The compression trigger fired.
    CompressionTriggered,
    /// Compression finished.
    CompressionDone(Vec<String>),
    /// Project aggregation should run.
    ProjectAggregationTriggered,
    /// Project aggregation finished.
    ProjectAggregationDone,
    /// User sync should run.
    UserSyncTriggered,
    /// User sync finished.
    UserSyncDone,
    /// Retention sweep.
    RetentionTick,
    /// Retention sweep finished.
    PurgeCompleted,
}

impl MemoryEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::TurnCompleted => "turn_completed",
            Self::CompressionTriggered => "compression_triggered",
            Self::CompressionDone(_) => "compression_done",
            Self::ProjectAggregationTriggered => "project_aggregation_triggered",
            Self::ProjectAggregationDone => "project_aggregation_done",
            Self::UserSyncTriggered => "user_sync_triggered",
            Self::UserSyncDone => "user_sync_done",
            Self::RetentionTick => "retention_tick",
            Self::PurgeCompleted => "purge_completed",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::CompressionDone(segments) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "segments": segments }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The memory state-machine definition.
pub fn memory_machine() -> StateMachine {
    StateMachine::new("memory", MemoryState::Episodic.as_str())
        .state(MemoryState::Compressing.as_str())
        .state(MemoryState::Segmented.as_str())
        .state(MemoryState::ProjectAggregating.as_str())
        .state(MemoryState::ProjectSynced.as_str())
        .state(MemoryState::UserSyncing.as_str())
        .state(MemoryState::UserSynced.as_str())
        .state(MemoryState::Purging.as_str())
        .transition(Transition::new(
            "episodic",
            "compression_triggered",
            "compressing",
        ))
        .transition(Transition::new(
            "compressing",
            "compression_done",
            "segmented",
        ))
        .transition(Transition::new(
            "segmented",
            "project_aggregation_triggered",
            "project_aggregating",
        ))
        .transition(Transition::new(
            "project_aggregating",
            "project_aggregation_done",
            "project_synced",
        ))
        .transition(Transition::new(
            "project_synced",
            "user_sync_triggered",
            "user_syncing",
        ))
        .transition(Transition::new(
            "user_syncing",
            "user_sync_done",
            "user_synced",
        ))
        .transition(Transition::any("retention_tick", "purging"))
        .transition(Transition::new("purging", "purge_completed", "episodic"))
        .transition(Transition::new(
            "user_synced",
            "compression_triggered",
            "compressing",
        ))
        // A cycle that stops before project aggregation (no project, or an empty
        // segment batch) must still be able to compress again later.
        .transition(Transition::new(
            "segmented",
            "compression_triggered",
            "compressing",
        ))
        .transition(Transition::new(
            "project_synced",
            "compression_triggered",
            "compressing",
        ))
        .transition(Transition::new(
            "segmented",
            "user_sync_triggered",
            "user_syncing",
        ))
}

/// A live memory region.
#[derive(Debug)]
pub struct MemoryRegion {
    /// The region instance.
    pub sm: RegionSm,
}

impl MemoryRegion {
    /// Enter the region.
    pub fn new() -> Result<Self> {
        Ok(Self {
            sm: RegionSm::new("memory", memory_machine())?,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state()
    }

    /// Dispatch one event.
    pub fn dispatch(&mut self, event: MemoryEvent) -> Result<Vec<TransitionRecord>> {
        Ok(self.sm.sm.dispatch(event.into_sm())?)
    }

    /// Dispatch a raw event name, used by the session driver.
    pub fn dispatch_name(&mut self, name: &str, payload: Option<Value>) -> Result<()> {
        self.sm.dispatch_name(name, payload)?;
        Ok(())
    }
}
