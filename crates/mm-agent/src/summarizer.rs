//! Per-turn summarization for long downstream agent turns.
//!
//! A raw turn is retained in agentstream. The outer conversation manager sees a
//! compact summary so a long job does not consume its context. When no model is
//! available, a deterministic extractor runs and marks the summary degraded.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use mm_llm::{LlmClient, LlmRequest, Message, Purpose};
use mm_prompt::PromptStore;
use serde_json::Value;

use crate::{AgentArtifact, AgentToolSummary, AgentTurnSummary};

/// Turns longer than this many characters are summarized.
pub const SUMMARY_THRESHOLD_CHARS: usize = 24_000;
/// Turns longer than this many completion tokens are summarized.
pub const SUMMARY_THRESHOLD_TOKENS: u64 = 6_000;
/// A summarizer call is bounded.
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(30);

/// The turn summarizer.
#[derive(Clone)]
pub struct TurnSummarizer {
    client: Option<Arc<dyn LlmClient>>,
    prompts: Arc<PromptStore>,
    model: Option<String>,
    threshold_chars: usize,
    threshold_tokens: u64,
}

impl TurnSummarizer {
    /// Build a summarizer. Without a client, the deterministic extractor is used.
    pub fn new(
        client: Option<Arc<dyn LlmClient>>,
        prompts: Arc<PromptStore>,
        model: Option<String>,
    ) -> Self {
        Self {
            client,
            prompts,
            model,
            threshold_chars: SUMMARY_THRESHOLD_CHARS,
            threshold_tokens: SUMMARY_THRESHOLD_TOKENS,
        }
    }

    /// Override the thresholds. Tests use this.
    pub fn with_thresholds(mut self, chars: usize, tokens: u64) -> Self {
        self.threshold_chars = chars;
        self.threshold_tokens = tokens;
        self
    }

    /// Summarize a long turn, or return the raw message unchanged.
    pub async fn maybe_summarize(
        &self,
        raw: &str,
        previous_summary: Option<&str>,
        artifacts: &[AgentArtifact],
        tools: &[AgentToolSummary],
        completion_tokens: u64,
    ) -> (String, Option<AgentTurnSummary>) {
        if raw.chars().count() <= self.threshold_chars && completion_tokens <= self.threshold_tokens
        {
            return (raw.to_owned(), None);
        }
        if let Some(summary) = self.model_summary(raw, previous_summary, artifacts).await {
            let message = display_message(&summary);
            return (message, Some(summary));
        }
        let summary = deterministic_summary(raw, artifacts, tools);
        let message = display_message(&summary);
        (message, Some(summary))
    }

    async fn model_summary(
        &self,
        raw: &str,
        previous_summary: Option<&str>,
        artifacts: &[AgentArtifact],
    ) -> Option<AgentTurnSummary> {
        let client = self.client.as_ref()?;
        let spec = self.prompts.strategy("agent.summarize.v1")?;
        let mut variables: BTreeMap<String, String> = spec
            .slots
            .iter()
            .map(|slot| (slot.clone(), String::new()))
            .collect();
        variables.insert("agent_turn".into(), raw.chars().take(80_000).collect());
        variables.insert(
            "previous_summary".into(),
            previous_summary.unwrap_or_default().to_owned(),
        );
        let prompt = self.prompts.render("agent.summarize.v1", &variables).ok()?;
        let mut request = LlmRequest::new(
            Purpose::Summarize,
            vec![
                Message::system(prompt.text),
                Message::user(format!(
                    "Summarize this downstream agent turn as JSON:\n\n{raw}"
                )),
            ],
        );
        request.max_tokens = 700;
        request.temperature = 0.0;
        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        let response = tokio::time::timeout(SUMMARY_TIMEOUT, client.complete(&request))
            .await
            .ok()?
            .ok()?;
        let mut summary = parse_summary(&response.text)?;
        if summary.artifacts.is_empty() {
            summary.artifacts = artifacts.to_vec();
        }
        Some(summary)
    }
}

fn parse_summary(text: &str) -> Option<AgentTurnSummary> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim_end_matches("```")
        .trim();
    let value: Value = serde_json::from_str(trimmed).ok()?;
    let headline = value.get("headline")?.as_str()?.trim().to_owned();
    if headline.is_empty() {
        return None;
    }
    Some(AgentTurnSummary {
        headline: headline.chars().take(200).collect(),
        did: string_list(value.get("did"), 8),
        learned: string_list(value.get("learned"), 8),
        artifacts: Vec::new(),
        blockers: string_list(value.get("blockers"), 8),
        next_recommended: value
            .get("nextRecommended")
            .and_then(Value::as_str)
            .map(|value| value.chars().take(300).collect()),
        confidence: value
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.5)
            .clamp(0.0, 1.0) as f32,
        summary_failed: false,
    })
}

fn string_list(value: Option<&Value>, limit: usize) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| item.chars().take(300).collect())
                .take(limit)
                .collect()
        })
        .unwrap_or_default()
}

fn deterministic_summary(
    raw: &str,
    artifacts: &[AgentArtifact],
    tools: &[AgentToolSummary],
) -> AgentTurnSummary {
    let headline = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Agent turn completed")
        .trim()
        .chars()
        .take(200)
        .collect::<String>();
    let mut did = tools
        .iter()
        .map(|tool| format!("called {}", tool.name))
        .collect::<Vec<_>>();
    for line in raw.lines() {
        if did.len() >= 8 {
            break;
        }
        let line = line.trim();
        if line.len() > 4 && !line.starts_with('#') && !did.iter().any(|item| item.contains(line)) {
            did.push(line.chars().take(200).collect());
        }
    }
    let blockers = raw
        .lines()
        .filter(|line| {
            let lowered = line.to_ascii_lowercase();
            lowered.contains("error") || lowered.contains("failed") || line.trim().ends_with('?')
        })
        .take(8)
        .map(|line| line.trim().chars().take(300).collect())
        .collect();
    AgentTurnSummary {
        headline,
        did,
        learned: Vec::new(),
        artifacts: artifacts.to_vec(),
        blockers,
        next_recommended: None,
        confidence: 0.2,
        summary_failed: true,
    }
}

fn display_message(summary: &AgentTurnSummary) -> String {
    match &summary.next_recommended {
        Some(next) if !next.is_empty() => format!("{} Next: {next}", summary.headline),
        _ => summary.headline.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_turns_are_not_summarized() {
        let store = Arc::new(
            PromptStore::load(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
            )
            .unwrap(),
        );
        let summarizer = TurnSummarizer::new(None, store, None);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (message, summary) =
            runtime.block_on(summarizer.maybe_summarize("short", None, &[], &[], 0));
        assert_eq!(message, "short");
        assert!(summary.is_none());
    }

    #[test]
    fn fallback_marks_the_summary_degraded_and_keeps_artifacts() {
        let summary = deterministic_summary(
            "read the file\nthe test failed with error 42",
            &[AgentArtifact {
                kind: "file".into(),
                id: "src/auth.rs".into(),
                label: None,
            }],
            &[],
        );
        assert!(summary.summary_failed);
        assert_eq!(summary.artifacts.len(), 1);
        assert!(summary.blockers.iter().any(|line| line.contains("failed")));
    }
}
