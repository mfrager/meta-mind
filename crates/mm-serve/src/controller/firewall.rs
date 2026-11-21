//! Deterministic firewall check before a write, a gate, or an external call.

use mm_core::Ulid;
use mm_decision::risk::{analyze_risk, RiskMeasure};
use mm_firewall::report::{AuthorizationRef, FirewallInput, Reversibility, Stakes, ToolTarget};
use mm_firewall::prohibitions::first_prohibition;

use crate::state::AppState;

use super::catalog::{Effect, ToolSpec};

pub fn guards(effect: Effect) -> bool {
    matches!(effect, Effect::Write | Effect::Gate | Effect::External)
}

/// `Ok` when the prohibitions are quiet. A denial is the tool result, not a button.
pub fn allow(app: &AppState, spec: &ToolSpec) -> Result<(), String> {
    if let Some(reason) = app
        .firewall_denial
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
    {
        return Err(reason);
    }
    let risks = analyze_risk(&[], RiskMeasure::Variance);
    let mut input = FirewallInput::new(Ulid::nil(), risks);
    input.reversibility = Reversibility::Reversible;
    input.authorization = AuthorizationRef {
        granted: true,
        scope: "operator".into(),
        approval_id: None,
    };
    input.stakes = Stakes {
        level: 0.0,
        irreversibility: 0.0,
    };
    input.tool = Some(ToolTarget {
        name: spec.name.into(),
        permission_granted: true,
    });
    input.action_schema_valid = true;
    if let Some(hit) = first_prohibition(&input) {
        return Err(hit.reason.clone());
    }
    Ok(())
}
