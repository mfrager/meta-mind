//! Errors raised while defining or running a state machine.

use thiserror::Error;

/// A state-machine failure.
#[derive(Debug, Error)]
pub enum SmError {
    /// A definition is internally inconsistent.
    #[error("machine {machine:?}: {detail}")]
    InvalidMachine {
        /// The machine whose definition is wrong.
        machine: String,
        /// What is wrong with it.
        detail: String,
    },

    /// A state was declared more than once.
    #[error("machine {machine:?} declares state {state:?} twice")]
    DuplicateState {
        /// The machine.
        machine: String,
        /// The repeated state.
        state: String,
    },

    /// A transition, history entry or snapshot names a state the machine does not declare.
    #[error("machine {machine:?} does not declare state {state:?}")]
    UndeclaredState {
        /// The machine.
        machine: String,
        /// The unknown state.
        state: String,
    },

    /// A region was declared more than once.
    #[error("machine {machine:?} declares region {region:?} twice")]
    DuplicateRegion {
        /// The machine.
        machine: String,
        /// The repeated region.
        region: String,
    },

    /// A transition action failed.
    #[error("machine {machine:?}: action for {from} -> {to} on {event:?} failed: {source}")]
    Action {
        /// The machine.
        machine: String,
        /// The state being left.
        from: String,
        /// The state being entered.
        to: String,
        /// The event that caused the transition.
        event: String,
        /// The action's own error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A snapshot belongs to a different machine.
    #[error("snapshot for machine {expected:?} names machine {actual:?}")]
    SnapshotMachine {
        /// The machine the snapshot is being restored into.
        expected: String,
        /// The machine the snapshot was taken from.
        actual: String,
    },

    /// A snapshot's state is not declared by the machine.
    #[error("snapshot state {state:?} is not declared by machine {machine:?}")]
    SnapshotState {
        /// The machine.
        machine: String,
        /// The unknown state.
        state: String,
    },

    /// A snapshot names a region the machine does not declare.
    #[error("snapshot region {region:?} is not declared by machine {machine:?}")]
    SnapshotRegion {
        /// The machine.
        machine: String,
        /// The unknown region.
        region: String,
    },

    /// A snapshot names a child machine the restore catalog does not contain.
    #[error(
        "snapshot names child machine {machine:?}, which the restore catalog does not contain"
    )]
    UnknownChildMachine {
        /// The missing child machine name.
        machine: String,
    },
}

/// A result carrying [`SmError`].
pub type SmResult<T> = Result<T, SmError>;

impl SmError {
    pub(crate) fn invalid(machine: &str, detail: impl Into<String>) -> Self {
        SmError::InvalidMachine {
            machine: machine.to_string(),
            detail: detail.into(),
        }
    }

    pub(crate) fn undeclared(machine: &str, state: impl Into<String>) -> Self {
        SmError::UndeclaredState {
            machine: machine.to_string(),
            state: state.into(),
        }
    }
}
