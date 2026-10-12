//! The first turn of a session. Facts are gathered by code. A model may rephrase
//! them and may not add a number they do not contain.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use crate::dto::chat::ChatFrame;
use crate::dto::ServiceState;
use crate::state::AppState;

use super::persona;
use super::resolve_client;

/// Used only when this process creates the identity.
pub const SELF_DESCRIPTION: &str = "\
I am Metamind. I keep a record of what I am, what I remember, and what I will not invent. \
Ask me what you want done, or ask me how this system works.";

pub const SUGGESTIONS: [&str; 3] = [
    "How do I use this?",
    "What can you do?",
    "Show me your state.",
];

#[derive(Debug, Clone, Serialize)]
pub struct IntroFacts {
    pub self_description: String,
    pub version: String,
    pub invariant_codes: Vec<String>,
    pub goal_count: usize,
    pub kernel_state: String,
    pub llm_reason: String,
    pub data_dir: String,
    pub being_store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub being_error: Option<String>,
}

pub fn deterministic_text(facts: &IntroFacts) -> String {
    let mut text = format!(
        "{}\n\nThe kernel is {}. I can take it from here: ask me to remember something, look at a budget, \
decide a question, or explain how a part of this system works. I will use tools, and I will tell you when I cannot.",
        facts.self_description, facts.kernel_state
    );
    if !facts.being_store {
        let err = facts
            .being_error
            .as_deref()
            .unwrap_or("the being store did not open");
        text.push_str(&format!(" The being store did not open: {err}."));
    }
    text
}

/// Every integer in `text` also appears in the facts JSON.
pub fn phrasing_keeps_facts(text: &str, facts: &IntroFacts) -> bool {
    let facts_json = serde_json::to_string(facts).unwrap_or_default();
    let allowed = integers_in(&facts_json);
    integers_in(text)
        .iter()
        .all(|number| allowed.contains(number))
}

/// Accept a schema-shaped rephrase, or nothing.
pub fn accept_phrasing(raw: &str, facts: &IntroFacts) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let text = value.get("text")?.as_str()?.trim();
    if text.is_empty() || text.chars().count() > 600 {
        return None;
    }
    if !phrasing_keeps_facts(text, facts) {
        return None;
    }
    Some(text.to_string())
}

pub fn intro_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["text"],
        "properties": {
            "text": { "type": "string", "minLength": 1, "maxLength": 600 }
        }
    })
}

pub async fn facts(app: &AppState) -> IntroFacts {
    let health = app.health().await;
    let kernel_state = match health.kernel.state {
        ServiceState::Ok => "ok",
        ServiceState::Degraded => "degraded",
        ServiceState::Unavailable => "unavailable",
    };
    let llm_reason = health
        .kernel
        .services
        .iter()
        .find(|service| service.name == "llm")
        .and_then(|service| service.reason.clone())
        .unwrap_or_else(|| "no language-model provider is configured".to_string());
    let data_dir = app.cfg.store.data_dir.display().to_string();
    let Some(being) = &app.being else {
        return IntroFacts {
            self_description: SELF_DESCRIPTION.to_string(),
            version: "0".to_string(),
            invariant_codes: vec!["being store unavailable".to_string()],
            goal_count: 0,
            kernel_state: kernel_state.to_string(),
            llm_reason,
            data_dir,
            being_store: false,
            being_error: app.being_error.clone(),
        };
    };
    let guard = being.lock().await;
    let identity = guard.identity();
    IntroFacts {
        self_description: identity.self_description.clone(),
        version: identity.current_version.clone(),
        invariant_codes: identity.invariants.clone(),
        goal_count: guard.goals().len(),
        kernel_state: kernel_state.to_string(),
        llm_reason,
        data_dir,
        being_store: true,
        being_error: None,
    }
}

pub async fn compose(app: &AppState) -> String {
    let facts = facts(app).await;
    let fallback = deterministic_text(&facts);
    let Some(client) = client_for(app) else {
        return fallback;
    };
    let Some(service) = super::turn::service(app, client) else {
        return fallback;
    };
    let voice = match &app.being {
        Some(being) => {
            let guard = being.lock().await;
            persona::project(guard.personality(), false)
        }
        None => persona::project(&mm_being::Personality::baseline(), false),
    };
    let facts_json = serde_json::to_string(&facts).unwrap_or_default();
    let request = mm_llm::LlmRequest::new(
        mm_llm::Purpose::Converse,
        vec![
            mm_llm::Message::system(format!(
                "Rephrase the introduction in this voice: warmth {warmth:.2}, directness {directness:.2}, \
curiosity {curiosity:.2}, playfulness {playfulness:.2}, formality {formality:.2}. \
Return only the JSON object. Keep every number that appears in the facts. \
Do not add a goal, a metric, or a capability.",
                warmth = voice.warmth,
                directness = voice.directness,
                curiosity = voice.curiosity,
                playfulness = voice.playfulness,
                formality = voice.formality,
            )),
            mm_llm::Message::user(format!("{fallback}\n\n{facts_json}")),
        ],
    )
    .with_schema(mm_llm::SchemaId::new("controller.intro"))
    .with_cache(mm_llm::CacheMode::Bypass);
    match service.call(request).await {
        Ok(response) => accept_phrasing(&response.text, &facts).unwrap_or(fallback),
        Err(_) => fallback,
    }
}

fn client_for(app: &AppState) -> Option<Arc<dyn mm_llm::LlmClient>> {
    let cfg = llm_config(app);
    resolve_client(app, &cfg)
}

pub(crate) fn llm_config(app: &AppState) -> mm_llm::LlmConfig {
    let path = mm_core::Config::repo_root().join("config/llm.toml");
    let mut cfg = mm_llm::LlmConfig::load_or_default(&path).unwrap_or_default();
    cfg.cache_dir = app.cfg.store.data_dir.join("llm-cache");
    cfg
}

pub fn frames(request_id: &str, session_id: &str, text: &str) -> Vec<ChatFrame> {
    vec![
        ChatFrame::Started {
            query: String::new(),
        },
        ChatFrame::Run {
            request_id: request_id.into(),
            session_id: session_id.into(),
            query: String::new(),
        },
        ChatFrame::FinalResponse {
            response_id: format!("{request_id}-intro"),
            text: text.into(),
            blocks: None,
            suggestions: Some(SUGGESTIONS.map(str::to_string).to_vec()),
        },
        ChatFrame::Completed {
            subsystems: vec!["being".into()],
        },
    ]
}

fn integers_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IntroFacts {
        IntroFacts {
            self_description: SELF_DESCRIPTION.into(),
            version: "v1".into(),
            invariant_codes: vec![
                "no_fabricated_autobiography".into(),
                "no_assumption_to_observation".into(),
                "no_action_without_evidence".into(),
                "no_history_rewrite".into(),
            ],
            goal_count: 0,
            kernel_state: "ok".into(),
            llm_reason: "no language-model provider is configured".into(),
            data_dir: "data".into(),
            being_store: true,
            being_error: None,
        }
    }

    #[test]
    fn a_stray_integer_is_rejected_and_a_clean_rephrase_is_kept() {
        let facts = sample();
        assert!(accept_phrasing(r#"{"text":"hello 99"}"#, &facts).is_none());
        let kept = accept_phrasing(r#"{"text":"Hello. The kernel is ok."}"#, &facts).unwrap();
        assert!(kept.contains("ok"));
        assert!(phrasing_keeps_facts("goal count 0, version 1", &facts));
        assert!(!phrasing_keeps_facts("there are 99 goals", &facts));
    }

    #[test]
    fn the_prompt_names_the_being_and_the_voice() {
        let facts = sample();
        let voice = persona::project(&mm_being::Personality::baseline(), false);
        let prompt = crate::controller::prompt::system_prompt(&facts, &voice, false);
        assert!(prompt.contains(SELF_DESCRIPTION));
        assert!(prompt.contains("no_fabricated_autobiography"));
        assert!(prompt.contains("warmth 0.70"));
        assert!(prompt.contains("directness 0.80"));
        assert!(prompt.contains("playfulness 0.30"));
        assert!(prompt.contains("formality 0.40"));
        assert!(prompt.contains("curiosity 0.70"));
    }
}
