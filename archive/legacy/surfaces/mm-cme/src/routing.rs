//! Conversation-aware routing over `mm-llm`'s deterministic router.
//!
//! The base router knows purpose, complexity, stakes, precision and budgets. The
//! conversation engine adds the response modality, the active voice tier, whether
//! the turn is proactive, and the learned preference profile. A small
//! [`RoutingTable`] may pin a model per case; otherwise the cheapest sufficient
//! configured model wins, exactly as in `ThresholdRouter`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use mm_llm::config::{Complexity, Precision, Stakes};
use mm_llm::{Purpose, Router, RoutingDecision, RoutingRequest};
use serde::{Deserialize, Serialize};

use crate::error::{CmeError, Result};
use crate::planner::ResponseModality;
use crate::session::PreferenceProfile;
use crate::voice::VoiceTier;

/// Optional pinning table, loaded from `config/routing_table.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RoutingTable {
    /// Fallback pin when no more specific entry matches.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Modality → model.
    #[serde(default)]
    pub modality: BTreeMap<String, String>,
    /// Voice tier (`tier1`, `tier2`, `tier3`) → model.
    #[serde(default)]
    pub voice: BTreeMap<String, String>,
    /// Proactive turn pin.
    #[serde(default)]
    pub proactive_model: Option<String>,
}

impl RoutingTable {
    /// Parse a table document.
    pub fn from_toml_str(raw: &str) -> Result<Self> {
        toml::from_str(raw)
            .map_err(|error| CmeError::RoutingTable(format!("invalid routing table: {error}")))
    }

    /// Read a routing table from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            CmeError::RoutingTable(format!("cannot read {}: {error}", path.display()))
        })?;
        Self::from_toml_str(&raw)
    }

    /// The pinned model for a case, in precedence order: voice, proactive,
    /// modality, default.
    pub fn model_for(
        &self,
        modality: ResponseModality,
        voice_tier: Option<VoiceTier>,
        is_proactive: bool,
    ) -> Option<&str> {
        if let Some(tier) = voice_tier {
            if let Some(model) = self.voice.get(tier.as_str()) {
                return Some(model);
            }
        }
        if is_proactive {
            if let Some(model) = &self.proactive_model {
                return Some(model);
            }
        }
        self.modality
            .get(modality.as_str())
            .map(String::as_str)
            .or(self.default_model.as_deref())
    }
}

/// A routing request enriched with conversation state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationRoutingRequest {
    /// The turn's query, used for a deterministic complexity estimate.
    pub query: String,
    /// Response shape.
    pub modality: ResponseModality,
    /// Active voice tier, when in a voice session.
    pub voice_tier: Option<VoiceTier>,
    /// True for proactive turns.
    pub is_proactive: bool,
    /// The cost ceiling in micros; 0 means none.
    pub cost_budget_micros: i64,
    /// Learned preferences.
    pub preference_profile: PreferenceProfile,
}

impl ConversationRoutingRequest {
    /// A user-input text request.
    pub fn text(query: impl Into<String>, modality: ResponseModality) -> Self {
        Self {
            query: query.into(),
            modality,
            voice_tier: None,
            is_proactive: false,
            cost_budget_micros: 0,
            preference_profile: PreferenceProfile::default(),
        }
    }

    /// Attach the voice tier.
    pub fn with_voice_tier(mut self, tier: VoiceTier) -> Self {
        self.voice_tier = Some(tier);
        self
    }

    /// Mark the turn proactive.
    pub fn proactive(mut self) -> Self {
        self.is_proactive = true;
        self
    }

    /// Attach a cost ceiling.
    pub fn with_cost_budget(mut self, micros: i64) -> Self {
        self.cost_budget_micros = micros;
        self
    }

    /// Estimate complexity deterministically from the query's length and shape.
    fn complexity(&self) -> Complexity {
        let words = self.query.split_whitespace().count();
        let mut complexity = if words > 40 {
            Complexity::High
        } else if words > 12 {
            Complexity::Medium
        } else {
            Complexity::Low
        };
        // A drill-down or a strong quality preference raises the bar.
        if self.modality == ResponseModality::DrillDown
            || self.preference_profile.value("quality") > 0.5
        {
            complexity = complexity.max(Complexity::High);
        }
        complexity
    }

    /// Build the base substrate request, before the table's pin is applied.
    pub fn base_request(&self) -> RoutingRequest {
        let mut request = RoutingRequest::new(Purpose::Converse)
            .with_complexity(self.complexity())
            .with_stakes(if self.is_proactive {
                Stakes::Low
            } else {
                Stakes::Medium
            })
            .with_precision(if self.modality == ResponseModality::DrillDown {
                Precision::High
            } else {
                Precision::Normal
            })
            .with_cost_budget(self.cost_budget_micros);
        if let Some(tier) = self.voice_tier {
            request = request.with_latency_budget(tier.latency_budget_ms());
        }
        // A speed preference tightens the latency budget for text turns too.
        if self.voice_tier.is_none() && self.preference_profile.value("speed") > 0.5 {
            request = request.with_latency_budget(800);
        }
        request
    }
}

/// Routes conversation turns through a base router and an optional pin table.
#[derive(Clone)]
pub struct ConversationRouter {
    inner: Arc<dyn Router>,
    table: RoutingTable,
}

impl std::fmt::Debug for ConversationRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationRouter")
            .field("table", &self.table)
            .finish_non_exhaustive()
    }
}

impl ConversationRouter {
    /// A router with no pinning.
    pub fn new(inner: Arc<dyn Router>) -> Self {
        Self {
            inner,
            table: RoutingTable::default(),
        }
    }

    /// A router with a pinning table.
    pub fn with_table(inner: Arc<dyn Router>, table: RoutingTable) -> Self {
        Self { inner, table }
    }

    /// The pin table.
    pub fn table(&self) -> &RoutingTable {
        &self.table
    }

    /// Choose a model for a conversation turn.
    pub fn choose(&self, request: &ConversationRoutingRequest) -> Result<RoutingDecision> {
        let mut base = request.base_request();
        if let Some(pinned) =
            self.table
                .model_for(request.modality, request.voice_tier, request.is_proactive)
        {
            base.pinned_model = Some(pinned.to_owned());
        }
        Ok(self.inner.choose(&base)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_llm::{ModelSpec, ThresholdRouter};

    fn router() -> ThresholdRouter {
        ThresholdRouter::from_specs(
            vec![
                ModelSpec {
                    name: "fast".into(),
                    input_cost_micros_per_1k: 100,
                    output_cost_micros_per_1k: 400,
                    typical_latency_ms: 100,
                    complexity_ceiling: Complexity::Medium,
                    mock: false,
                },
                ModelSpec {
                    name: "deep".into(),
                    input_cost_micros_per_1k: 1_000,
                    output_cost_micros_per_1k: 4_000,
                    typical_latency_ms: 900,
                    complexity_ceiling: Complexity::High,
                    mock: false,
                },
            ],
            Some("fast".into()),
        )
    }

    #[test]
    fn drill_down_raises_precision_and_complexity() {
        let request = ConversationRoutingRequest::text("short", ResponseModality::DrillDown);
        let base = request.base_request();
        assert_eq!(base.required_precision, Precision::High);
        assert_eq!(base.purpose, Purpose::Converse);
        assert_eq!(base.complexity, Complexity::High);
    }

    #[test]
    fn voice_tier_sets_the_latency_budget() {
        let request = ConversationRoutingRequest::text("hello", ResponseModality::Explain)
            .with_voice_tier(VoiceTier::Tier3);
        assert_eq!(request.base_request().latency_budget_ms, 300);
    }

    #[test]
    fn table_pins_voice_over_modality_and_defaults() {
        let table = RoutingTable::from_toml_str(
            r#"
            default_model = "fast"
            proactive_model = "fast"

            [modality]
            drill_down = "deep"

            [voice]
            tier3 = "deep"
            "#,
        )
        .unwrap();
        let router = ConversationRouter::with_table(Arc::new(router()), table);
        let drill = router
            .choose(&ConversationRoutingRequest::text(
                "x",
                ResponseModality::DrillDown,
            ))
            .unwrap();
        assert_eq!(drill.model, "deep");
        let voice = router
            .choose(
                &ConversationRoutingRequest::text("x", ResponseModality::Explain)
                    .with_voice_tier(VoiceTier::Tier3),
            )
            .unwrap();
        assert_eq!(voice.model, "deep");
        let proactive = router
            .choose(&ConversationRoutingRequest::text("x", ResponseModality::Explain).proactive())
            .unwrap();
        assert_eq!(proactive.model, "fast");
    }

    #[test]
    fn without_a_pin_the_cheapest_sufficient_model_wins() {
        let router = ConversationRouter::new(Arc::new(router()));
        let decision = router
            .choose(&ConversationRoutingRequest::text(
                "hello",
                ResponseModality::Explain,
            ))
            .unwrap();
        assert_eq!(decision.model, "fast"); // Explain at low complexity is sufficient on `fast`.
    }
}
