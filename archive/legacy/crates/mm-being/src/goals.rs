//! Goals (a desire) and commitments (an intention), with an append-only lifecycle.
//!
//! BDI makes the distinction load-bearing: wanting something is not promising it,
//! so the two live in separate types with separate status vocabularies. Both are
//! append-only — a terminal status is history, and the way it was reached is
//! recorded in a transition row rather than by editing the object (the migration
//! enforces the same rule with a trigger, so a caller that bypasses this module
//! still cannot rewrite history).
//!
//! The legality table itself lives here, in one place, so the CLI, the store, and
//! the guard cannot disagree about what a lifecycle allows.

use serde::{Deserialize, Serialize};

use mm_core::{Timestamp, Ulid};

use crate::error::{BeingError, Result, TransitionDenied};

/// Where a goal is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Being pursued.
    Active,
    /// Waiting on something.
    Pending,
    /// Cannot proceed yet.
    Blocked,
    /// Achieved.
    Fulfilled,
    /// Given up deliberately.
    Abandoned,
    /// Replaced by another goal.
    Superseded,
}

impl GoalStatus {
    /// The stable wire name, which is what the `goals.status` CHECK allows.
    pub fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Pending => "pending",
            GoalStatus::Blocked => "blocked",
            GoalStatus::Fulfilled => "fulfilled",
            GoalStatus::Abandoned => "abandoned",
            GoalStatus::Superseded => "superseded",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(GoalStatus::Active),
            "pending" => Some(GoalStatus::Pending),
            "blocked" => Some(GoalStatus::Blocked),
            "fulfilled" => Some(GoalStatus::Fulfilled),
            "abandoned" => Some(GoalStatus::Abandoned),
            "superseded" => Some(GoalStatus::Superseded),
            _ => None,
        }
    }

    /// True when the goal is finished and its row may never change again.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            GoalStatus::Fulfilled | GoalStatus::Abandoned | GoalStatus::Superseded
        )
    }
}

impl std::fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a goal came from. An inferred goal is not a promise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalOrigin {
    /// The user asked for it.
    User,
    /// The being set it for itself.
    Being,
    /// Derived from something else the being knows.
    Inferred,
}

impl GoalOrigin {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            GoalOrigin::User => "user",
            GoalOrigin::Being => "being",
            GoalOrigin::Inferred => "inferred",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(GoalOrigin::User),
            "being" => Some(GoalOrigin::Being),
            "inferred" => Some(GoalOrigin::Inferred),
            _ => None,
        }
    }
}

/// Where a commitment is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentStatus {
    /// Owed and not yet discharged.
    Active,
    /// Discharged.
    Fulfilled,
    /// Withdrawn.
    Revoked,
    /// Replaced by another commitment.
    Superseded,
}

impl CommitmentStatus {
    /// The stable wire name, which is what the `commitments.status` CHECK allows.
    pub fn as_str(self) -> &'static str {
        match self {
            CommitmentStatus::Active => "active",
            CommitmentStatus::Fulfilled => "fulfilled",
            CommitmentStatus::Revoked => "revoked",
            CommitmentStatus::Superseded => "superseded",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(CommitmentStatus::Active),
            "fulfilled" => Some(CommitmentStatus::Fulfilled),
            "revoked" => Some(CommitmentStatus::Revoked),
            "superseded" => Some(CommitmentStatus::Superseded),
            _ => None,
        }
    }

    /// True when the commitment is finished and its row may never change again.
    pub fn is_terminal(self) -> bool {
        !matches!(self, CommitmentStatus::Active)
    }
}

impl std::fmt::Display for CommitmentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A desire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    /// The goal's identifier.
    pub id: Ulid,
    /// Whose goal it is.
    pub owner: Ulid,
    /// What is wanted, in the being's own words.
    pub description: String,
    /// Where it is in its life.
    pub status: GoalStatus,
    /// How much it matters, in `[0, 1]`.
    pub priority: f32,
    /// When it should be done, if it should.
    pub deadline: Option<Timestamp>,
    /// The goal this one refines, if any.
    pub parent: Option<Ulid>,
    /// The evidence that it is worth pursuing.
    pub evidence: Vec<String>,
    /// Where it came from.
    pub origin: GoalOrigin,
}

impl Goal {
    /// Set the priority, clamped into `[0, 1]`.
    pub fn set_priority(&mut self, priority: f32) {
        self.priority = clamp01(priority);
    }
}

/// An intention: something owed to someone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commitment {
    /// The commitment's identifier.
    pub id: Ulid,
    /// The goal it serves, when it serves one.
    pub goal_id: Option<Ulid>,
    /// Who it was made to.
    pub made_to: Ulid,
    /// What was promised.
    pub description: String,
    /// Where it is in its life.
    pub status: CommitmentStatus,
    /// When it is due, if it is.
    pub deadline: Option<Timestamp>,
}

/// Check that a goal may move from one status to another.
///
/// A no-op is allowed: re-asserting the current status is what an idempotent
/// transition looks like, and refusing it would make a retry indistinguishable
/// from an illegal edit.
pub fn transition_goal(from: GoalStatus, to: GoalStatus) -> Result<GoalStatus> {
    if from == to {
        return Ok(to);
    }
    if from.is_terminal() {
        return Err(denied("goal", from.as_str(), to.as_str()));
    }
    let allowed = matches!(
        (from, to),
        (
            GoalStatus::Active | GoalStatus::Pending | GoalStatus::Blocked,
            GoalStatus::Active
                | GoalStatus::Pending
                | GoalStatus::Blocked
                | GoalStatus::Fulfilled
                | GoalStatus::Abandoned
                | GoalStatus::Superseded
        )
    );
    if allowed {
        Ok(to)
    } else {
        Err(denied("goal", from.as_str(), to.as_str()))
    }
}

/// Check that a commitment may move from one status to another.
pub fn transition_commitment(
    from: CommitmentStatus,
    to: CommitmentStatus,
) -> Result<CommitmentStatus> {
    if from == to {
        return Ok(to);
    }
    if from.is_terminal() {
        return Err(denied("commitment", from.as_str(), to.as_str()));
    }
    // The only starting point is `active`, because every other status is terminal.
    let allowed = matches!(
        to,
        CommitmentStatus::Fulfilled | CommitmentStatus::Revoked | CommitmentStatus::Superseded
    );
    if allowed {
        Ok(to)
    } else {
        Err(denied("commitment", from.as_str(), to.as_str()))
    }
}

fn denied(subject: &str, from: &str, to: &str) -> BeingError {
    let reason = if GoalStatus::parse(from).is_some_and(GoalStatus::is_terminal)
        || CommitmentStatus::parse(from).is_some_and(CommitmentStatus::is_terminal)
    {
        "a terminal status is never rewritten"
    } else {
        "the lifecycle does not allow that transition"
    };
    BeingError::Transition(TransitionDenied {
        subject: subject.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        reason: reason.to_string(),
    })
}

fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_GOAL: [GoalStatus; 6] = [
        GoalStatus::Active,
        GoalStatus::Pending,
        GoalStatus::Blocked,
        GoalStatus::Fulfilled,
        GoalStatus::Abandoned,
        GoalStatus::Superseded,
    ];

    const ALL_COMMITMENT: [CommitmentStatus; 4] = [
        CommitmentStatus::Active,
        CommitmentStatus::Fulfilled,
        CommitmentStatus::Revoked,
        CommitmentStatus::Superseded,
    ];

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    #[test]
    fn a_terminal_goal_refuses_every_change() {
        for terminal in ALL_GOAL.into_iter().filter(|s| s.is_terminal()) {
            for other in ALL_GOAL {
                if other == terminal {
                    continue;
                }
                let err = transition_goal(terminal, other).unwrap_err();
                assert_eq!(err.kind(), "transition_denied");
                assert!(
                    err.to_string().contains("terminal"),
                    "{terminal} -> {other}: {err}"
                );
            }
            // Re-asserting the terminal status is still a no-op, not an edit.
            assert_eq!(transition_goal(terminal, terminal).unwrap(), terminal);
        }
    }

    #[test]
    fn the_open_statuses_move_freely_and_an_impossible_move_is_denied() {
        assert_eq!(
            transition_goal(GoalStatus::Active, GoalStatus::Fulfilled).unwrap(),
            GoalStatus::Fulfilled
        );
        assert_eq!(
            transition_goal(GoalStatus::Blocked, GoalStatus::Pending).unwrap(),
            GoalStatus::Pending
        );
        // Every open status can reach every other, so the only denial is terminal
        // -> anything, which the test above covers. Confirm by construction too.
        for from in ALL_GOAL.into_iter().filter(|s| !s.is_terminal()) {
            for to in ALL_GOAL {
                assert!(transition_goal(from, to).is_ok(), "{from} -> {to}");
            }
        }
    }

    #[test]
    fn a_fulfilled_goal_may_not_be_reopened() {
        let err = transition_goal(GoalStatus::Fulfilled, GoalStatus::Active).unwrap_err();
        assert_eq!(err.kind(), "transition_denied");
        assert_eq!(err.code(), "mm.being.transition_denied");
    }

    #[test]
    fn a_terminal_commitment_refuses_every_change() {
        for terminal in ALL_COMMITMENT.into_iter().filter(|s| s.is_terminal()) {
            for other in ALL_COMMITMENT {
                if other == terminal {
                    continue;
                }
                assert!(transition_commitment(terminal, other).is_err());
            }
        }
        assert_eq!(
            transition_commitment(CommitmentStatus::Active, CommitmentStatus::Revoked).unwrap(),
            CommitmentStatus::Revoked
        );
        assert_eq!(
            transition_commitment(CommitmentStatus::Active, CommitmentStatus::Active).unwrap(),
            CommitmentStatus::Active
        );
    }

    #[test]
    fn statuses_round_trip_and_priority_is_clamped() {
        for status in ALL_GOAL {
            assert_eq!(GoalStatus::parse(status.as_str()), Some(status));
        }
        for status in ALL_COMMITMENT {
            assert_eq!(CommitmentStatus::parse(status.as_str()), Some(status));
        }
        for origin in [GoalOrigin::User, GoalOrigin::Being, GoalOrigin::Inferred] {
            assert_eq!(GoalOrigin::parse(origin.as_str()), Some(origin));
        }
        assert_eq!(GoalStatus::parse("frobbed"), None);

        let mut goal = Goal {
            id: id(1),
            owner: id(2),
            description: "ship Phase 4".into(),
            status: GoalStatus::Active,
            priority: 0.5,
            deadline: None,
            parent: None,
            evidence: vec!["plan".into()],
            origin: GoalOrigin::User,
        };
        goal.set_priority(4.2);
        assert!((goal.priority - 1.0).abs() < f32::EPSILON);
        goal.set_priority(-3.0);
        assert!(goal.priority.abs() < f32::EPSILON);

        let commitment = Commitment {
            id: id(3),
            goal_id: Some(id(1)),
            made_to: id(2),
            description: "report by Friday".into(),
            status: CommitmentStatus::Active,
            deadline: None,
        };
        assert!(!commitment.status.is_terminal());
    }
}
