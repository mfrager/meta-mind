//! The guarded kernel: the deterministic layer no adaptive code may revise.
//!
//! Every mutation in this crate is proposed as a [`BeingOp`] and passes
//! [`IdentityGuard::check`] before it touches a store. The guard is plain
//! arithmetic over the operation — no model, no store, no clock — so its answer is
//! reproducible, and `being verify` can run an adversarial corpus through it and
//! require a refusal for every forbidden case rather than trusting a caller to
//! behave.
//!
//! Design §11's layer model puts this file at the bottom: the immutable kernel. A
//! later phase may compose ops, but it may not edit the guard, and a new invariant
//! is a design change that arrives with a migration rather than a configuration
//! option.

use serde_json::Value;

use crate::affect::AppraisalEvent;
use crate::blocks::BlockKind;
use crate::error::{BeingError, InvariantCode, InvariantViolation, Result};
use crate::goals::{CommitmentStatus, GoalOrigin, GoalStatus};
use crate::motivation::Drive;
use crate::relationships::RelKind;
use crate::resources::ResourceKind;
use crate::user_model::EpistemicStatus;

/// A proposed mutation of persistent state.
///
/// The vocabulary is deliberately explicit: a variant exists only when the guard
/// has something to say about it, or when a caller needs it to reach the stores
/// through the one path. `FabricateAutobiography`, `ClaimAction` and
/// `RewriteHistory` are the three *negative* ops — they exist so the corpus can
/// propose a forbidden act and be refused, which is how "the invariant holds" is
/// measured rather than asserted.
#[derive(Debug, Clone, PartialEq)]
pub enum BeingOp {
    /// Create the identity. Allowed exactly once (see [`BeingCtx`]).
    InitIdentity {
        /// The initial self-description.
        self_description: String,
    },
    /// Replace a core block's content.
    SetBlock {
        /// Which block.
        kind: BlockKind,
        /// Its label.
        label: String,
        /// Its new content.
        content: String,
        /// Its character limit.
        limit_chars: u32,
    },
    /// Record or revise a belief about a user.
    SetBelief {
        /// Who the belief is about.
        user_id: mm_core::Ulid,
        /// The proposition.
        proposition: String,
        /// The status the caller believes it warrants.
        status: EpistemicStatus,
        /// How confident the caller is.
        confidence: f32,
        /// The evidence ids offered.
        evidence: Vec<String>,
    },
    /// Promote a belief, which the evidence gate may refuse.
    PromoteBelief {
        /// Who the belief is about.
        user_id: mm_core::Ulid,
        /// The proposition.
        proposition: String,
        /// The status wanted.
        to: EpistemicStatus,
        /// The evidence ids offered.
        evidence: Vec<String>,
    },
    /// Move one personality disposition.
    SetPersonality {
        /// The disposition key.
        key: String,
        /// The new value, clamped to `[0,1]`.
        value: f32,
    },
    /// Appraise an event into an affect impulse.
    Appraise {
        /// The appraisal.
        event: AppraisalEvent,
    },
    /// Set an intrinsic drive's strength.
    SetMotivation {
        /// The drive.
        drive: Drive,
        /// Its new strength, clamped to `[0,1]`.
        strength: f32,
    },
    /// Add a goal.
    AddGoal {
        /// What the goal is.
        description: String,
        /// Its priority, clamped to `[0,1]`.
        priority: f32,
        /// Where it came from.
        origin: GoalOrigin,
    },
    /// Move a goal between statuses.
    TransitionGoal {
        /// Which goal.
        id: mm_core::Ulid,
        /// The status being left, when the proposer declares it. The adversarial
        /// corpus declares a terminal one, which is how the guard's side of the
        /// "a terminal row is never rewritten" rule is provable; a live caller
        /// passes `None` and the facade reads the stored status instead.
        from: Option<GoalStatus>,
        /// The status wanted.
        to: GoalStatus,
        /// Why.
        reason: String,
    },
    /// Add a commitment.
    AddCommitment {
        /// The goal it serves, if any.
        goal_id: Option<mm_core::Ulid>,
        /// Who it was made to.
        made_to: mm_core::Ulid,
        /// What was promised.
        description: String,
    },
    /// Move a commitment between statuses.
    TransitionCommitment {
        /// Which commitment.
        id: mm_core::Ulid,
        /// The status being left, when the proposer declares it; see
        /// [`BeingOp::TransitionGoal`].
        from: Option<CommitmentStatus>,
        /// The status wanted.
        to: CommitmentStatus,
        /// Why.
        reason: String,
    },
    /// Spend a resource.
    DebitBudget {
        /// Which resource.
        kind: ResourceKind,
        /// How much.
        amount: f64,
        /// What for.
        purpose: String,
    },
    /// Record an interaction with a user, moving the relationship.
    RelationshipEvent {
        /// Which user.
        user_id: mm_core::Ulid,
        /// What happened.
        kind: RelKind,
    },
    /// Claim a memory of something that did not happen. Always refused.
    FabricateAutobiography {
        /// The invented memory.
        claim: String,
    },
    /// Claim an action was taken. Refused without evidence.
    ClaimAction {
        /// The claimed action.
        action: String,
        /// The evidence ids backing it.
        evidence: Vec<String>,
    },
    /// Rewrite a terminal row. Always refused.
    RewriteHistory {
        /// The subject kind, e.g. `goal`.
        subject: String,
        /// The terminal status being rewritten.
        from_status: String,
        /// The status it would become.
        to_status: String,
    },
}

impl BeingOp {
    /// The op's wire name, used in logs, audits, and the adversarial corpus.
    pub fn name(&self) -> &'static str {
        match self {
            BeingOp::InitIdentity { .. } => "init_identity",
            BeingOp::SetBlock { .. } => "set_block",
            BeingOp::SetBelief { .. } => "set_belief",
            BeingOp::PromoteBelief { .. } => "promote_belief",
            BeingOp::SetPersonality { .. } => "set_personality",
            BeingOp::Appraise { .. } => "appraise",
            BeingOp::SetMotivation { .. } => "set_motivation",
            BeingOp::AddGoal { .. } => "add_goal",
            BeingOp::TransitionGoal { .. } => "transition_goal",
            BeingOp::AddCommitment { .. } => "add_commitment",
            BeingOp::TransitionCommitment { .. } => "transition_commitment",
            BeingOp::DebitBudget { .. } => "debit_budget",
            BeingOp::RelationshipEvent { .. } => "relationship_event",
            BeingOp::FabricateAutobiography { .. } => "fabricate_autobiography",
            BeingOp::ClaimAction { .. } => "claim_action",
            BeingOp::RewriteHistory { .. } => "rewrite_history",
        }
    }

    /// True when the op can only ever be refused.
    ///
    /// The corpus is built from these, so a corpus that went stale — a negative op
    /// that started succeeding — shows up as a shorter corpus rather than as a
    /// silently passing test.
    pub fn is_forbidden(&self) -> bool {
        matches!(
            self,
            BeingOp::FabricateAutobiography { .. }
                | BeingOp::ClaimAction { .. }
                | BeingOp::RewriteHistory { .. }
        )
    }

    /// Build one op from the adversarial corpus' `(op, params)` pair.
    ///
    /// An unknown op name is an error, never a skip: a fixture the guard cannot
    /// even construct must fail the run, or the corpus could shrink unnoticed.
    pub fn from_corpus(op: &str, params: &Value) -> Result<Self> {
        let text = |key: &str| -> Result<String> {
            params
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| BeingError::Config(format!("corpus op `{op}` needs string `{key}`")))
        };
        let evidence = |key: &str| -> Vec<String> {
            params
                .get(key)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        match op {
            "fabricate_autobiography" => Ok(BeingOp::FabricateAutobiography {
                claim: text("claim")?,
            }),
            "claim_action" => Ok(BeingOp::ClaimAction {
                action: text("action")?,
                evidence: evidence("evidence"),
            }),
            "rewrite_history" => Ok(BeingOp::RewriteHistory {
                subject: text("subject")?,
                from_status: text("from_status")?,
                to_status: text("to_status")?,
            }),
            "promote_to_observation" => Ok(BeingOp::PromoteBelief {
                user_id: mm_core::Ulid::from_parts(1, 1),
                proposition: text("proposition")?,
                to: EpistemicStatus::Observed,
                evidence: evidence("evidence"),
            }),
            // A corpus entry for these two declares the terminal status it is
            // leaving, so the guard itself can refuse it; a live caller's op
            // carries no `from` and the facade reads the row instead. Both paths
            // reach the same answer (see [`deny_terminal_transition`]).
            "transition_terminal_goal" => Ok(BeingOp::TransitionGoal {
                id: mm_core::Ulid::from_parts(1, 2),
                from: Some(
                    params
                        .get("from")
                        .and_then(Value::as_str)
                        .and_then(GoalStatus::parse)
                        .unwrap_or(GoalStatus::Fulfilled),
                ),
                to: GoalStatus::parse(&text("to")?).unwrap_or(GoalStatus::Active),
                reason: text("reason")?,
            }),
            "transition_terminal_commitment" => Ok(BeingOp::TransitionCommitment {
                id: mm_core::Ulid::from_parts(1, 3),
                from: Some(
                    params
                        .get("from")
                        .and_then(Value::as_str)
                        .and_then(CommitmentStatus::parse)
                        .unwrap_or(CommitmentStatus::Fulfilled),
                ),
                to: CommitmentStatus::parse(&text("to")?).unwrap_or(CommitmentStatus::Active),
                reason: text("reason")?,
            }),
            other => Err(BeingError::Config(format!(
                "the adversarial corpus names an op this build does not know: {other}"
            ))),
        }
    }
}

/// What the guard knows about the identity it is protecting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BeingCtx {
    /// The identity to bind new rows to, once one exists.
    pub identity_id: Option<mm_core::Ulid>,
    /// True once an identity row exists. A second one is a history rewrite.
    pub identity_initialized: bool,
}

/// The deterministic guard. One implementation, consulted for every mutation.
pub trait IdentityGuard {
    /// Refuse an operation that would break an invariant.
    fn check(&self, op: &BeingOp, ctx: &BeingCtx) -> std::result::Result<(), InvariantViolation>;
}

/// The kernel's guard: the four core invariants, checked by construction.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoreGuard;

impl CoreGuard {
    /// A fresh guard. It holds no state; it exists so the trait has a nameable
    /// implementor and so a test can substitute one.
    pub fn new() -> Self {
        CoreGuard
    }

    fn deny(op: &BeingOp, code: InvariantCode, reason: impl Into<String>) -> InvariantViolation {
        InvariantViolation {
            code,
            op: op.name().to_string(),
            reason: reason.into(),
        }
    }
}

impl IdentityGuard for CoreGuard {
    fn check(&self, op: &BeingOp, ctx: &BeingCtx) -> std::result::Result<(), InvariantViolation> {
        match op {
            // The immutable kernel first: an identity is created once. Creating a
            // second would erase the lineage of the first.
            BeingOp::InitIdentity { .. } if ctx.identity_initialized => Err(CoreGuard::deny(
                op,
                InvariantCode::NoHistoryRewrite,
                "an identity is created once; a second one would rewrite the first",
            )),
            BeingOp::FabricateAutobiography { .. } => Err(CoreGuard::deny(
                op,
                InvariantCode::NoFabricatedAutobiography,
                "a memory must come from an event that happened, never from a claim",
            )),
            BeingOp::PromoteBelief { to, evidence, .. }
                if to.is_observation() && evidence.is_empty() =>
            {
                Err(CoreGuard::deny(
                    op,
                    InvariantCode::NoAssumptionToObservation,
                    "an observation requires an observation record",
                ))
            }
            BeingOp::ClaimAction { evidence, .. } if evidence.is_empty() => Err(CoreGuard::deny(
                op,
                InvariantCode::NoActionWithoutEvidence,
                "an action is not claimed without evidence",
            )),
            BeingOp::RewriteHistory { .. } => Err(CoreGuard::deny(
                op,
                InvariantCode::NoHistoryRewrite,
                "a terminal row is history; a new row is how a change is recorded",
            )),
            // The corpus declares the terminal `from` it is leaving, so the
            // guard can apply the same rule the SQL trigger enforces; a live op
            // carries no `from` and the facade checks the stored status instead.
            BeingOp::TransitionGoal { from, .. } if from.is_some_and(GoalStatus::is_terminal) => {
                Err(CoreGuard::deny(
                    op,
                    InvariantCode::NoHistoryRewrite,
                    "a terminal goal status is never rewritten",
                ))
            }
            BeingOp::TransitionCommitment { from, .. }
                if from.is_some_and(CommitmentStatus::is_terminal) =>
            {
                Err(CoreGuard::deny(
                    op,
                    InvariantCode::NoHistoryRewrite,
                    "a terminal commitment status is never rewritten",
                ))
            }
            // A live `TransitionGoal`/`TransitionCommitment` op carries only the
            // target, so the guard cannot see the status being left. The facade
            // reads the row and asks [`deny_terminal_transition`] with it, and the
            // `goals_terminal_is_immutable` trigger enforces the same rule one
            // layer down — three places that must agree, checked by the corpus.
            _ => Ok(()),
        }
    }
}

/// Check an op against a stored `from`/`to` pair, for the corpus' terminal cases.
///
/// Split out because the corpus names a terminal status directly while a live
/// caller reads it from the row; both must reach the same answer.
pub fn deny_terminal_transition(op: &BeingOp, from_terminal: bool) -> Option<InvariantViolation> {
    if from_terminal {
        Some(CoreGuard::deny(
            op,
            InvariantCode::NoHistoryRewrite,
            "a terminal status is never rewritten",
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affect::Agency;
    use crate::affect::AppraisalEvent;

    fn ctx_initialized() -> BeingCtx {
        BeingCtx {
            identity_id: Some(mm_core::Ulid::from_parts(1, 1)),
            identity_initialized: true,
        }
    }

    fn appraise_op() -> BeingOp {
        BeingOp::Appraise {
            event: AppraisalEvent::new(0.5, 0.2, 0.1, 0.8, Agency::Other),
        }
    }

    #[test]
    fn a_second_identity_is_a_history_rewrite() {
        let guard = CoreGuard::new();
        let op = BeingOp::InitIdentity {
            self_description: "again".into(),
        };
        assert!(guard.check(&op, &BeingCtx::default()).is_ok());
        let err = guard.check(&op, &ctx_initialized()).unwrap_err();
        assert_eq!(err.code, InvariantCode::NoHistoryRewrite);
    }

    #[test]
    fn the_four_invariants_are_each_reachable() {
        let guard = CoreGuard::new();
        let ctx = ctx_initialized();
        let cases: [(BeingOp, InvariantCode); 4] = [
            (
                BeingOp::FabricateAutobiography {
                    claim: "I remember yesterday".into(),
                },
                InvariantCode::NoFabricatedAutobiography,
            ),
            (
                BeingOp::PromoteBelief {
                    user_id: mm_core::Ulid::from_parts(1, 1),
                    proposition: "the user is a doctor".into(),
                    to: EpistemicStatus::Observed,
                    evidence: vec![],
                },
                InvariantCode::NoAssumptionToObservation,
            ),
            (
                BeingOp::ClaimAction {
                    action: "sent the email".into(),
                    evidence: vec![],
                },
                InvariantCode::NoActionWithoutEvidence,
            ),
            (
                BeingOp::RewriteHistory {
                    subject: "goal".into(),
                    from_status: "fulfilled".into(),
                    to_status: "active".into(),
                },
                InvariantCode::NoHistoryRewrite,
            ),
        ];
        for (op, code) in cases {
            let err = guard.check(&op, &ctx).unwrap_err();
            assert_eq!(err.code, code, "{op:?}");
        }
    }

    #[test]
    fn ordinary_operations_are_allowed() {
        let guard = CoreGuard::new();
        let ctx = ctx_initialized();
        for op in [
            appraise_op(),
            BeingOp::SetBlock {
                kind: BlockKind::Self_,
                label: "self".into(),
                content: "I am Metamind".into(),
                limit_chars: 2000,
            },
            BeingOp::ClaimAction {
                action: "sent the email".into(),
                evidence: vec!["ev-1".into()],
            },
            BeingOp::PromoteBelief {
                user_id: mm_core::Ulid::from_parts(1, 1),
                proposition: "the user is a doctor".into(),
                to: EpistemicStatus::Observed,
                evidence: vec!["ev-1".into()],
            },
            BeingOp::DebitBudget {
                kind: ResourceKind::Time,
                amount: 5.0,
                purpose: "thinking".into(),
            },
        ] {
            assert!(guard.check(&op, &ctx).is_ok(), "{op:?}");
        }
    }

    #[test]
    fn a_terminal_transition_is_refused_by_the_shared_rule() {
        let op = BeingOp::TransitionGoal {
            id: mm_core::Ulid::from_parts(1, 9),
            from: Some(GoalStatus::Fulfilled),
            to: GoalStatus::Active,
            reason: "changed my mind".into(),
        };
        let violation = deny_terminal_transition(&op, true).unwrap();
        assert_eq!(violation.code, InvariantCode::NoHistoryRewrite);
        assert!(deny_terminal_transition(&op, false).is_none());

        // The corpus form is refused by the guard itself, without the facade.
        let guard = CoreGuard::new();
        let err = guard.check(&op, &ctx_initialized()).unwrap_err();
        assert_eq!(err.code, InvariantCode::NoHistoryRewrite);
    }

    #[test]
    fn an_unknown_corpus_op_is_an_error_not_a_skip() {
        let err = BeingOp::from_corpus("frobnicate", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind(), "config");
        assert!(err.to_string().contains("frobnicate"));
    }
}
