//! `mm-sm` — the state-machine substrate under the conversation engine.
//!
//! A machine is a named set of states, transitions keyed on event names, optional
//! guards and actions, orthogonal regions that run in parallel, spawned children,
//! and history states whose predecessor is remembered.
//!
//! The engine is deliberately synchronous: guards and actions complete before
//! `dispatch` returns, which keeps a turn's state transitions deterministic and
//! makes them easy to test. Asynchronous work (model calls, tool execution) runs
//! in the driver that owns the instance; it dispatches the resulting events.
//!
//! ```
//! use mm_sm::{SmInstance, StateMachine, Transition};
//!
//! let machine = StateMachine::new("job", "idle")
//!     .state("running")
//!     .transition(Transition::new("idle", "start", "running"));
//! let mut instance = SmInstance::start(machine)?;
//! instance.dispatch("start")?;
//! assert!(instance.is_in("running"));
//! # Ok::<(), mm_sm::SmError>(())
//! ```

#![forbid(unsafe_code)]

pub mod error;
pub mod machine;
pub mod snapshot;

mod hierarchical;
mod orthogonal;
mod tracing;

pub use error::{SmError, SmResult};
pub use machine::{
    Action, ActionContext, ActionResult, Destination, Guard, GuardContext, SmAction, SmEvent,
    SmGuard, SmInstance, StateId, StateMachine, Transition, TransitionRecord,
};
pub use snapshot::{RegionSnapshot, SmSnapshot};
