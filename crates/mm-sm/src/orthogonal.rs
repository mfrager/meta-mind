//! Orthogonal regions: independently-running machines composed in parallel.
//!
//! Regions are declared on a [`StateMachine`](crate::machine::StateMachine) and
//! started with the instance. Every event dispatched to the instance reaches all
//! of its regions (and their own regions), so unrelated events are normal: a
//! region ignores what it has no transition for.

use crate::error::{SmError, SmResult};
use crate::machine::{SmInstance, StateId, StateMachine};

impl SmInstance {
    /// Add a region to a running instance.
    ///
    /// The region is started immediately in its initial state. Region names are
    /// unique per instance.
    pub fn enter_region(&mut self, machine: StateMachine) -> SmResult<&mut SmInstance> {
        if self.regions.contains_key(&machine.name) {
            return Err(SmError::DuplicateRegion {
                machine: self.machine.name.clone(),
                region: machine.name,
            });
        }
        let name = machine.name.clone();
        let mut region = Self::start(machine)?;
        region.parent = Some(self.id.clone());
        self.regions.insert(name.clone(), region);
        Ok(self
            .regions
            .get_mut(&name)
            .expect("the region was just inserted"))
    }

    /// The current state of `region`, if it exists.
    pub fn orthogonal_current(&self, region: &str) -> Option<&StateId> {
        self.regions.get(region).map(SmInstance::state)
    }

    /// Every region's `(name, current state)` pair, in name order.
    pub fn orthogonal_states(&self) -> Vec<(&str, &StateId)> {
        self.regions
            .iter()
            .map(|(name, region)| (name.as_str(), region.state()))
            .collect()
    }

    /// The region named `name`.
    pub fn region(&self, name: &str) -> Option<&SmInstance> {
        self.regions.get(name)
    }

    /// The region named `name`, mutably.
    pub fn region_mut(&mut self, name: &str) -> Option<&mut SmInstance> {
        self.regions.get_mut(name)
    }

    /// Detach and return the region named `name`.
    pub fn remove_region(&mut self, name: &str) -> Option<SmInstance> {
        self.regions.remove(name)
    }
}
