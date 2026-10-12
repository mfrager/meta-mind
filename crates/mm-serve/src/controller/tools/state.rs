//! The five sections the state board and `state.show` share.

use serde_json::{json, Value};

use crate::dto::{HealthEnvelope, ServiceState};
use crate::state::AppState;

use super::super::exec;
use super::super::persona;

pub async fn snapshot(app: &AppState) -> Value {
    let health = app.health().await;
    json!({
        "kernel": kernel_section(&health),
        "being": being_section(app).await,
        "goals": exec::goal_list(app).await.data,
        "budgets": exec::budget_show(app).await.data,
        "last_action": Value::Null,
    })
}

fn kernel_section(health: &HealthEnvelope) -> Value {
    let ok_names: Vec<&str> = health
        .kernel
        .services
        .iter()
        .filter(|service| service.state == ServiceState::Ok)
        .map(|service| service.name.as_str())
        .collect();
    let problems: Vec<Value> = health
        .kernel
        .services
        .iter()
        .filter(|service| service.state != ServiceState::Ok)
        .map(|service| {
            json!({
                "name": service.name,
                "state": service_name(service.state),
                "reason": service.reason,
            })
        })
        .collect();
    let llm_reason = health
        .kernel
        .services
        .iter()
        .find(|service| service.name == "llm")
        .and_then(|service| service.reason.clone())
        .unwrap_or_else(|| "no language-model provider is configured".into());
    json!({
        "state": service_name(health.kernel.state),
        "reason": health.kernel.reason,
        "ok_services": ok_names.join(", "),
        "llm_reason": llm_reason,
        "services": problems,
    })
}

async fn being_section(app: &AppState) -> Value {
    let Some(being) = &app.being else {
        return json!({
            "ok": false,
            "reason": app.being_error.clone().unwrap_or_else(|| "the being store did not open".into()),
        });
    };
    let guard = being.lock().await;
    let identity = guard.identity();
    let projected = persona::project(guard.personality(), false);
    json!({
        "ok": true,
        "self_description": identity.self_description,
        "version": identity.current_version,
        "invariants": identity.invariants,
        "goal_count": guard.goals().len(),
        "voice": {
            "warmth": projected.warmth,
            "directness": projected.directness,
            "curiosity": projected.curiosity,
            "playfulness": projected.playfulness,
            "formality": projected.formality,
        }
    })
}

fn service_name(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Ok => "ok",
        ServiceState::Degraded => "degraded",
        ServiceState::Unavailable => "unavailable",
    }
}
