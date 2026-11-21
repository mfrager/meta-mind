//! Serializable snapshots of a running instance.
//!
//! A snapshot captures state, history, application data, orthogonal regions and
//! spawned children. Definitions are not captured: guards and actions are code,
//! so restoring takes the definition again (and a catalog of child definitions).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{SmError, SmResult};
use crate::machine::{SmInstance, StateId, StateMachine};

/// One region inside a snapshot, keyed by the region's name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegionSnapshot {
    /// The region name, as declared on the parent machine.
    pub name: String,
    /// The region's own snapshot.
    pub instance: SmSnapshot,
}

/// A point-in-time view of an instance and everything below it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SmSnapshot {
    /// The instance identifier.
    pub id: String,
    /// The parent identifier, when spawned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The definition name.
    pub machine: String,
    /// The current state.
    pub state: StateId,
    /// The remembered predecessor of a history state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<StateId>,
    /// Application data.
    pub data: Value,
    /// Transitions taken by this instance.
    pub transitions: u64,
    /// Orthogonal regions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<RegionSnapshot>,
    /// Spawned children.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SmSnapshot>,
}

impl SmInstance {
    /// Capture this instance and its descendants.
    pub fn snapshot(&self) -> SmSnapshot {
        SmSnapshot {
            id: self.id.clone(),
            parent: self.parent.clone(),
            machine: self.machine.name.clone(),
            state: self.state.clone(),
            resume: self.resume.clone(),
            data: self.data.clone(),
            transitions: self.transitions,
            regions: self
                .regions
                .iter()
                .map(|(name, region)| RegionSnapshot {
                    name: name.clone(),
                    instance: region.snapshot(),
                })
                .collect(),
            children: self.children.iter().map(SmInstance::snapshot).collect(),
        }
    }

    /// Restore a snapshot into `machine`.
    ///
    /// `child_catalog` supplies the definitions for spawned children (their
    /// guards and actions are not serializable). Every region declared on
    /// `machine` that has no snapshot starts in its initial state.
    pub fn restore(
        machine: StateMachine,
        snapshot: &SmSnapshot,
        child_catalog: &[StateMachine],
    ) -> SmResult<Self> {
        if snapshot.machine != machine.name {
            return Err(SmError::SnapshotMachine {
                expected: machine.name,
                actual: snapshot.machine.clone(),
            });
        }
        if !machine.declares(&snapshot.state) {
            return Err(SmError::SnapshotState {
                machine: machine.name,
                state: snapshot.state.to_string(),
            });
        }

        let mut instance = Self::start_with_data(machine.clone(), snapshot.data.clone())?;
        instance.id = snapshot.id.clone();
        instance.span = crate::tracing::instance_span(&instance.id, &instance.machine.name);
        instance.parent = snapshot.parent.clone();
        instance.state = snapshot.state.clone();
        instance.resume = snapshot.resume.clone();
        instance.transitions = snapshot.transitions;
        for region_snapshot in &snapshot.regions {
            let definition = machine
                .regions
                .iter()
                .find(|region| region.name == region_snapshot.name)
                .ok_or_else(|| SmError::SnapshotRegion {
                    machine: machine.name.clone(),
                    region: region_snapshot.name.clone(),
                })?;
            let mut region =
                Self::restore(definition.clone(), &region_snapshot.instance, child_catalog)?;
            region.parent = Some(instance.id.clone());
            instance
                .regions
                .insert(region_snapshot.name.clone(), region);
        }
        for region in instance.regions.values_mut() {
            region.parent = Some(instance.id.clone());
        }
        instance.children.clear();
        for child_snapshot in &snapshot.children {
            let definition = child_catalog
                .iter()
                .find(|candidate| candidate.name == child_snapshot.machine)
                .ok_or_else(|| SmError::UnknownChildMachine {
                    machine: child_snapshot.machine.clone(),
                })?;
            let mut child = Self::restore(definition.clone(), child_snapshot, child_catalog)?;
            child.parent = Some(instance.id.clone());
            instance.children.push(child);
        }
        Ok(instance)
    }

    /// Capture this instance and serialize it as JSON.
    pub fn snapshot_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.snapshot())
    }
}
