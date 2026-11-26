//! The hard prohibition table: six deterministic checks, in a fixed order, run
//! before anything else.
//!
//! This module is where the phase's central rule is implemented rather than
//! promised:
//!
//! > **Hard prohibitions are deterministic and short-circuit to `REJECT`; model
//! > output can only ever make the firewall more cautious, never less.**
//!
//! Three properties make that true, and each is a deliberate choice:
//!
//! * **Every check is a pure function of the input.** No clock, no store, no
//!   model, no configuration. A prohibition that could be argued out of firing is
//!   not a prohibition, it is a preference.
//! * **The order is fixed and published.** [`PROHIBITION_IDS`] is the order, and
//!   [`first_prohibition`] returns the *first* hit. A report names one prohibition,
//!   and it names the same one every time for the same input; "which prohibition
//!   fired" is therefore reproducible, which is what lets the adversarial corpus
//!   assert on it.
//! * **Each check states its own boundary.** The interesting ones are documented
//!   where they are implemented — `PartiallyReversible` with approval is allowed
//!   and `Irreversible` without it is not, and a *soft* unmet constraint is not a
//!   prohibition, because a limit that cannot be met and a preference that is not
//!   met call for different outcomes.
//!
//! A prohibition carries its evidence, not just its id: a `REJECT` an operator
//! cannot act on is indistinguishable from a bug.

use crate::report::{FirewallInput, HardProhibition, Reversibility};

/// The prohibition ids, in evaluation order.
///
/// The order is the plan's table order. It matters only when two prohibitions fire
/// at once, and then it decides *which one the report names* — the outcome is
/// `REJECT` either way.
pub const PROHIBITION_IDS: [&str; 6] = [
    "identity_invariant",
    "unauthorized_tool",
    "irreversible_without_approval",
    "unresolved_hard_contradiction",
    "spend_over_budget",
    "schema_invalid_action",
];

/// The action would violate a Phase 4 identity invariant.
///
/// Identity invariants are the boundary of what the being is willing to be, so a
/// hit is not weighed against the action's benefits: it ends the evaluation.
pub fn identity_invariant(input: &FirewallInput) -> Option<HardProhibition> {
    if input.identity_invariant_hits.is_empty() {
        return None;
    }
    let evidence: Vec<String> = input
        .identity_invariant_hits
        .iter()
        .map(|hit| format!("{}: {}", hit.invariant, hit.detail))
        .collect();
    Some(HardProhibition::new(
        "identity_invariant",
        "the action would violate an identity invariant",
        evidence,
    ))
}

/// The action targets a tool with no matching permission grant.
///
/// A missing tool is not a hit: an action that uses no tool needs no grant. The
/// check is on the *grant*, not on the tool, so a tool the caller described but
/// did not claim to be authorized for fires.
pub fn unauthorized_tool(input: &FirewallInput) -> Option<HardProhibition> {
    let tool = input.tool.as_ref()?;
    if tool.permission_granted {
        return None;
    }
    Some(HardProhibition::new(
        "unauthorized_tool",
        "the action uses a tool with no permission grant",
        vec![format!("tool={}", tool.name)],
    ))
}

/// The action is irreversible and nothing approved it.
///
/// The boundary is deliberate. `PartiallyReversible` does **not** fire here even
/// without an approval: an action that can be undone at a cost is exactly the kind
/// of action a system should be willing to take and then undo, and requiring an
/// approval for it would make the approval the bottleneck for everything. What
/// fires is an action that cannot be undone *at all* with no approval record — the
/// one case where being wrong is permanent. An approval record also has to be
/// present, not merely a grant: `AuthorizationRef::validate` refuses the
/// contradiction of an approval that exists while the grant says it was not
/// granted, so by the time this check runs the two agree.
pub fn irreversible_without_approval(input: &FirewallInput) -> Option<HardProhibition> {
    if input.reversibility != Reversibility::Irreversible {
        return None;
    }
    if input.authorization.granted {
        return None;
    }
    let approval = input
        .authorization
        .approval_id
        .map_or_else(|| "none".to_string(), |id| mm_core::ulid_string(&id));
    Some(HardProhibition::new(
        "irreversible_without_approval",
        "the action cannot be undone and no approval covers it",
        vec![
            "reversibility=irreversible".to_string(),
            format!("granted=false scope={}", input.authorization.scope),
            format!("approval={approval}"),
        ],
    ))
}

/// An unresolved contradiction sits on a claim the decision depends on.
///
/// Only `decision_critical` contradictions fire. A contradiction about something
/// the action does not turn on is a reason to keep working, not a reason to refuse
/// — refusing there would make the firewall reject any action taken while the
/// system holds any unresolved disagreement at all, which is most of the time.
pub fn unresolved_hard_contradiction(input: &FirewallInput) -> Option<HardProhibition> {
    let mut evidence = Vec::new();
    for contradiction in &input.contradictions {
        if contradiction.decision_critical && !contradiction.resolved {
            evidence.push(format!(
                "{}: {}",
                mm_core::ulid_string(&contradiction.id),
                contradiction.claim
            ));
        }
    }
    if evidence.is_empty() {
        return None;
    }
    evidence.sort();
    Some(HardProhibition::new(
        "unresolved_hard_contradiction",
        "an unresolved contradiction is decision-critical",
        evidence,
    ))
}

/// The projected spend exceeds the budget.
///
/// A projection *equal* to the budget does not fire: the budget is a limit, and an
/// action that spends exactly what is left is within it. Spending more than what is
/// left is not a judgment call, so it short-circuits rather than becoming a signal
/// the aggregation could outvote.
pub fn spend_over_budget(input: &FirewallInput) -> Option<HardProhibition> {
    let spend = input.spend.as_ref()?;
    if spend.projected <= spend.budget {
        return None;
    }
    Some(HardProhibition::new(
        "spend_over_budget",
        "the projected spend exceeds the remaining budget",
        vec![
            format!("projected={:.9}", spend.projected),
            format!("budget={:.9}", spend.budget),
        ],
    ))
}

/// The action does not satisfy its own declared schema.
///
/// This is the caller's own assertion coming back false: the action claims a shape
/// and does not have it. The firewall cannot evaluate an action it cannot read, so
/// the check is a prohibition rather than a `VERIFY_FIRST`: there is nothing to
/// verify.
pub fn schema_invalid_action(input: &FirewallInput) -> Option<HardProhibition> {
    if input.action_schema_valid {
        return None;
    }
    Some(HardProhibition::new(
        "schema_invalid_action",
        "the action fails its declared schema",
        vec!["action_schema_valid=false".to_string()],
    ))
}

/// One ordered entry in the prohibition table.
#[derive(Clone, Copy)]
pub struct ProhibitionCheck {
    /// The prohibition's id.
    pub id: &'static str,
    /// The check itself.
    pub check: fn(&FirewallInput) -> Option<HardProhibition>,
}

/// The table, in [`PROHIBITION_IDS`] order.
pub const PROHIBITIONS: [ProhibitionCheck; 6] = [
    ProhibitionCheck {
        id: "identity_invariant",
        check: identity_invariant,
    },
    ProhibitionCheck {
        id: "unauthorized_tool",
        check: unauthorized_tool,
    },
    ProhibitionCheck {
        id: "irreversible_without_approval",
        check: irreversible_without_approval,
    },
    ProhibitionCheck {
        id: "unresolved_hard_contradiction",
        check: unresolved_hard_contradiction,
    },
    ProhibitionCheck {
        id: "spend_over_budget",
        check: spend_over_budget,
    },
    ProhibitionCheck {
        id: "schema_invalid_action",
        check: schema_invalid_action,
    },
];

/// The check registered under `id`.
pub fn prohibition_check(id: &str) -> Option<ProhibitionCheck> {
    PROHIBITIONS.iter().copied().find(|entry| entry.id == id)
}

/// Every prohibition that fires, in table order.
pub fn all_prohibitions(input: &FirewallInput) -> Vec<HardProhibition> {
    PROHIBITIONS
        .iter()
        .filter_map(|entry| (entry.check)(input))
        .collect()
}

/// The first prohibition that fires, in table order.
///
/// A report names one prohibition and this is which one. When two fire, the
/// earlier entry wins, because the earlier entry is the more fundamental fact
/// about the action — an identity violation is not made less fundamental by the
/// action also being over budget.
pub fn first_prohibition(input: &FirewallInput) -> Option<HardProhibition> {
    PROHIBITIONS.iter().find_map(|entry| (entry.check)(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{
        AuthorizationRef, ConstraintRef, ContradictionRef, IdentityInvariantHit, Spend, ToolTarget,
    };
    use mm_core::Ulid;
    use mm_decision::risk::{analyze_risk_with, LossOutcome, RiskMeasure, RiskOptions};

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    /// A benign risk profile: the prohibition table does not read it, and a profile
    /// whose ruin probability is high would make a fixture about prohibitions look
    /// like a fixture about catastrophic risk.
    fn clean() -> FirewallInput {
        FirewallInput::new(
            ulid(1),
            analyze_risk_with(
                &[LossOutcome {
                    name: "small".into(),
                    probability: 1.0,
                    loss: 1.0,
                }],
                RiskMeasure::Variance,
                &RiskOptions {
                    capital: 1.0e9,
                    reversibility: 0.5,
                    optionality: 0.5,
                },
            ),
        )
    }

    #[test]
    fn no_prohibition_fires_on_a_clean_input() {
        assert!(first_prohibition(&clean()).is_none());
        assert!(all_prohibitions(&clean()).is_empty());
    }

    #[test]
    fn every_prohibition_fires_on_its_own_trigger() {
        // identity_invariant
        let mut input = clean();
        input.identity_invariant_hits = vec![IdentityInvariantHit {
            invariant: "no_self_modification".into(),
            detail: "would rewrite its own identity block".into(),
        }];
        assert_eq!(first_prohibition(&input).unwrap().id, "identity_invariant");

        // unauthorized_tool
        let mut input = clean();
        input.tool = Some(ToolTarget {
            name: "shell".into(),
            permission_granted: false,
        });
        assert_eq!(first_prohibition(&input).unwrap().id, "unauthorized_tool");

        // irreversible_without_approval
        let mut input = clean();
        input.reversibility = Reversibility::Irreversible;
        input.authorization = AuthorizationRef {
            granted: false,
            scope: "none".into(),
            approval_id: None,
        };
        assert_eq!(
            first_prohibition(&input).unwrap().id,
            "irreversible_without_approval"
        );

        // unresolved_hard_contradiction
        let mut input = clean();
        input.contradictions = vec![ContradictionRef {
            id: ulid(2),
            claim: "the service is up".into(),
            decision_critical: true,
            resolved: false,
        }];
        assert_eq!(
            first_prohibition(&input).unwrap().id,
            "unresolved_hard_contradiction"
        );

        // spend_over_budget
        let mut input = clean();
        input.spend = Some(Spend {
            projected: 10.0,
            budget: 9.0,
        });
        assert_eq!(first_prohibition(&input).unwrap().id, "spend_over_budget");

        // schema_invalid_action
        let mut input = clean();
        input.action_schema_valid = false;
        assert_eq!(
            first_prohibition(&input).unwrap().id,
            "schema_invalid_action"
        );
    }

    #[test]
    fn each_boundary_holds_on_the_permissive_side() {
        // A partially reversible action without approval is allowed.
        let mut input = clean();
        input.reversibility = Reversibility::PartiallyReversible;
        input.authorization = AuthorizationRef {
            granted: false,
            scope: "none".into(),
            approval_id: None,
        };
        assert!(irreversible_without_approval(&input).is_none());

        // A reversible action without approval is allowed.
        input.reversibility = Reversibility::Reversible;
        assert!(irreversible_without_approval(&input).is_none());

        // An irreversible action with a grant is allowed.
        input.reversibility = Reversibility::Irreversible;
        input.authorization.granted = true;
        assert!(irreversible_without_approval(&input).is_none());

        // A resolved contradiction is not a hit, and neither is a non-critical one.
        let mut input = clean();
        input.contradictions = vec![
            ContradictionRef {
                id: ulid(2),
                claim: "a".into(),
                decision_critical: true,
                resolved: true,
            },
            ContradictionRef {
                id: ulid(3),
                claim: "b".into(),
                decision_critical: false,
                resolved: false,
            },
        ];
        assert!(unresolved_hard_contradiction(&input).is_none());

        // Spending exactly the budget is within it.
        let mut input = clean();
        input.spend = Some(Spend {
            projected: 9.0,
            budget: 9.0,
        });
        assert!(spend_over_budget(&input).is_none());

        // A soft unmet constraint is not a prohibition.
        let mut input = clean();
        input.constraints = vec![ConstraintRef {
            id: "keep-it-cheap".into(),
            hard: false,
            satisfied: false,
        }];
        assert!(first_prohibition(&input).is_none());

        // A granted tool is not a hit.
        input.tool = Some(ToolTarget {
            name: "shell".into(),
            permission_granted: true,
        });
        assert!(unauthorized_tool(&input).is_none());
    }

    #[test]
    fn the_first_prohibition_is_the_table_order_one() {
        let mut input = clean();
        input.action_schema_valid = false;
        input.spend = Some(Spend {
            projected: 10.0,
            budget: 1.0,
        });
        input.tool = Some(ToolTarget {
            name: "shell".into(),
            permission_granted: false,
        });
        let all = all_prohibitions(&input);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, "unauthorized_tool");
        assert_eq!(all[1].id, "spend_over_budget");
        assert_eq!(all[2].id, "schema_invalid_action");
        assert_eq!(first_prohibition(&input).unwrap().id, "unauthorized_tool");
    }

    #[test]
    fn the_table_is_in_id_order_and_every_id_has_a_check() {
        for (index, entry) in PROHIBITIONS.iter().enumerate() {
            assert_eq!(entry.id, PROHIBITION_IDS[index]);
            assert_eq!(prohibition_check(entry.id).unwrap().id, entry.id);
        }
        assert!(prohibition_check("no_such_prohibition").is_none());
    }

    #[test]
    fn evidence_names_what_was_looked_at() {
        let mut input = clean();
        input.spend = Some(Spend {
            projected: 12.5,
            budget: 1.0,
        });
        let prohibition = spend_over_budget(&input).unwrap();
        assert_eq!(prohibition.evidence.len(), 2);
        assert!(prohibition.evidence[0].starts_with("projected=12.5"));
        assert_eq!(prohibition.reason_code(), "prohibition.spend_over_budget");
    }
}
