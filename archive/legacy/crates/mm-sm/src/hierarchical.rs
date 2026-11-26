//! Hierarchical composition: a running instance may spawn children.
//!
//! Children carry their parent's identifier and receive every event the parent
//! dispatches. A child's state is independent; the parent only observes the
//! transitions a dispatch reports.

use crate::error::SmResult;
use crate::machine::{SmInstance, StateMachine};

impl SmInstance {
    /// Spawn a child machine and return a handle to it.
    ///
    /// The child starts in its initial state with null data and records this
    /// instance as its parent. It is dispatched to after the parent and its
    /// orthogonal regions have seen the event.
    pub fn spawn_child(&mut self, machine: StateMachine) -> SmResult<&mut SmInstance> {
        let mut child = Self::start(machine)?;
        child.parent = Some(self.id.clone());
        self.children.push(child);
        Ok(self.children.last_mut().expect("a child was just pushed"))
    }

    /// The child with `id`, if it is still attached.
    pub fn child(&self, id: &str) -> Option<&SmInstance> {
        self.children.iter().find(|child| child.id == id)
    }

    /// The child with `id`, mutably.
    pub fn child_mut(&mut self, id: &str) -> Option<&mut SmInstance> {
        self.children.iter_mut().find(|child| child.id == id)
    }

    /// Every live descendant of this instance, depth first.
    pub fn descendants(&self) -> Vec<&SmInstance> {
        let mut out = Vec::new();
        for child in &self.children {
            out.push(child);
            out.extend(child.descendants());
        }
        out
    }

    /// Detach and return the child with `id`.
    pub fn remove_child(&mut self, id: &str) -> Option<SmInstance> {
        let index = self.children.iter().position(|child| child.id == id)?;
        Some(self.children.remove(index))
    }
}
