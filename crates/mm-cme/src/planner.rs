//! Deterministic turn planning.
//!
//! Planning runs before any model call and never consults one: given the turn
//! context and the session's preferences, [`TurnPlanner::build_plan`] produces the
//! same [`TurnPlan`] every time. That is what makes chunk streaming, routing and
//! tests reproducible.

use serde::{Deserialize, Serialize};

use crate::chunk::ChunkDirective;
use crate::error::Result;
use crate::session::SessionContext;
use crate::turn::{TurnPlan, TurnPlanMetadata};
use crate::voice::VoiceTurnConfig;

/// The shape of the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseModality {
    /// Overview, then deep details and sources.
    DrillDown,
    /// Concept, then an analogy.
    #[default]
    Explain,
    /// Summary, then implications.
    ZoomOut,
    /// A short set of bullets.
    Summarize,
    /// Bulleted answer at the requested detail.
    Bullets,
}

impl ResponseModality {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DrillDown => "drill_down",
            Self::Explain => "explain",
            Self::ZoomOut => "zoom_out",
            Self::Summarize => "summarize",
            Self::Bullets => "bullets",
        }
    }

    /// Parse a stored name.
    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::DrillDown,
            Self::Explain,
            Self::ZoomOut,
            Self::Summarize,
            Self::Bullets,
        ]
        .into_iter()
        .find(|modality| modality.as_str() == value)
    }
}

/// How a plan may use tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolPolicy {
    /// No tools.
    #[default]
    None,
    /// Tools may run without asking.
    Allowed,
    /// Every tool call needs confirmation.
    Confirm,
}

impl ToolPolicy {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Allowed => "allowed",
            Self::Confirm => "confirm",
        }
    }
}

/// The per-turn token and cost allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBudget {
    /// Tokens for the whole turn.
    pub tokens: u64,
    /// Cost ceiling in micros; 0 means no ceiling.
    pub cost_micros: i64,
}

impl Default for TurnBudget {
    fn default() -> Self {
        Self {
            tokens: 8_000,
            cost_micros: 0,
        }
    }
}

/// Everything the planner knows about the turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnContext {
    /// Turn identifier.
    pub turn_id: mm_data::TurnId,
    /// Session identifier.
    pub session_id: mm_data::SessionId,
    /// The user's request.
    pub user_query: String,
    /// True when a proactive region spawned the turn.
    pub is_proactive: bool,
    /// What triggered it (`user_input`, `proactive`, `plugin`).
    pub trigger: String,
    /// Voice configuration when the session is in a voice tier.
    pub voice_config: Option<VoiceTurnConfig>,
    /// Partial input worth pre-warming on.
    pub speculative_hint: Option<String>,
    /// Parent turn, for proactive or plugin children.
    pub parent_turn_id: Option<mm_data::TurnId>,
}

impl TurnContext {
    /// A user-input turn context.
    pub fn user_input(
        turn_id: mm_data::TurnId,
        session_id: mm_data::SessionId,
        query: impl Into<String>,
    ) -> Self {
        Self {
            turn_id,
            session_id,
            user_query: query.into(),
            is_proactive: false,
            trigger: "user_input".to_owned(),
            voice_config: None,
            speculative_hint: None,
            parent_turn_id: None,
        }
    }
}

/// The deterministic planner.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnPlanner;

impl TurnPlanner {
    /// Build the plan for a turn from context and session preferences.
    pub fn build_plan(&self, ctx: &TurnContext, session: &SessionContext) -> Result<TurnPlan> {
        let modality = self.choose_modality(ctx, session);
        let detail_level = self.choose_detail_level(ctx, session);
        let mut chunks = self.plan_chunks(ctx, modality, detail_level);
        let tool_policy = self.plan_tool_policy(ctx, session);
        if tool_policy != ToolPolicy::None {
            chunks.push(ChunkDirective::tool("tool", "tool", "lookup"));
        }
        let budget = self.allocate_budget(&session.budgets, chunks.len());
        Ok(TurnPlan {
            turn_id: ctx.turn_id.clone(),
            session_id: ctx.session_id.clone(),
            user_query: ctx.user_query.clone(),
            modality,
            detail_level,
            chunks,
            tool_policy,
            voice_config: ctx.voice_config,
            budget,
            metadata: TurnPlanMetadata {
                parent_turn_id: ctx.parent_turn_id.clone(),
                is_proactive: ctx.is_proactive,
                trigger: ctx.trigger.clone(),
            },
        })
    }

    /// Pick the response shape. The query's own wording wins; a strong format
    /// preference breaks ties for otherwise unremarkable requests.
    pub fn choose_modality(&self, ctx: &TurnContext, session: &SessionContext) -> ResponseModality {
        let query = ctx.user_query.to_lowercase();
        let has = |needle: &str| query.contains(needle);
        if has("summar") || has("tl;dr") || has("short version") {
            ResponseModality::Summarize
        } else if has("big picture") || has("zoom out") || has("implications") {
            ResponseModality::ZoomOut
        } else if has("detail") || has("drill") || has("deep dive") || has("sources") {
            ResponseModality::DrillDown
        } else if has("bullet")
            || has("list")
            || has("steps")
            || session.preferences.value("format") > 0.4
        {
            ResponseModality::Bullets
        } else {
            ResponseModality::Explain
        }
    }

    /// Map the `detail` preference (−1..1) to 1..5, neutral at 3.
    pub fn choose_detail_level(&self, _ctx: &TurnContext, session: &SessionContext) -> u8 {
        let preference = session.preferences.value("detail").clamp(-1.0, 1.0);
        (3.0 + preference * 2.0).round().clamp(1.0, 5.0) as u8
    }

    /// The chunk structure for a modality. Identifiers are stable so clients and
    /// tests can address a section.
    pub fn plan_chunks(
        &self,
        ctx: &TurnContext,
        modality: ResponseModality,
        detail: u8,
    ) -> Vec<ChunkDirective> {
        let mut chunks = match modality {
            ResponseModality::DrillDown => vec![
                ChunkDirective::text_at("overview", "overview", detail),
                ChunkDirective::structured("details", "details", detail.saturating_add(1)),
                ChunkDirective::structured("sources", "sources", detail),
            ],
            ResponseModality::Explain => vec![
                ChunkDirective::text_at("concept", "concept", detail),
                ChunkDirective::text_at("analogy", "analogy", detail),
            ],
            ResponseModality::ZoomOut => vec![
                ChunkDirective::text_at("summary", "summary", detail),
                ChunkDirective::structured("implications", "implications", detail),
            ],
            ResponseModality::Summarize | ResponseModality::Bullets => {
                vec![ChunkDirective::text_at("bullets", "bullets", detail)]
            }
        };
        // A voice session emits a matching audio chunk after the prose.
        if let Some(voice) = &ctx.voice_config {
            if voice.tier.streams_audio() {
                chunks.push(ChunkDirective::audio("audio", "audio"));
            }
        }
        chunks
    }

    /// Tools are opt-in by preference and never used for proactive turns. A
    /// `confirm_tools` preference wins over `tools`, so a session that wants a
    /// gate before every call gets one.
    pub fn plan_tool_policy(&self, ctx: &TurnContext, session: &SessionContext) -> ToolPolicy {
        if ctx.is_proactive {
            ToolPolicy::None
        } else if session.preferences.value("confirm_tools") > 0.5 {
            ToolPolicy::Confirm
        } else if session.preferences.value("tools") > 0.5 {
            ToolPolicy::Allowed
        } else {
            ToolPolicy::None
        }
    }

    /// Split the session budget evenly, with a 128-token floor per chunk so a
    /// many-chunk plan cannot starve one section.
    pub fn allocate_budget(
        &self,
        budgets: &crate::session::SessionBudgets,
        chunk_count: usize,
    ) -> TurnBudget {
        let count = chunk_count.max(1) as u64;
        let per_chunk = (budgets.turn_token_budget / count).max(128);
        TurnBudget {
            tokens: per_chunk * count,
            cost_micros: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{MemoryContext, PreferenceProfile, SessionBudgets};

    fn context(query: &str) -> TurnContext {
        TurnContext::user_input(
            mm_data::TurnId::new("t1"),
            mm_data::SessionId::new("s1"),
            query,
        )
    }

    fn session() -> SessionContext {
        SessionContext {
            memory: MemoryContext::default(),
            preferences: PreferenceProfile::default(),
            affect: Default::default(),
            budgets: SessionBudgets::default(),
        }
    }

    #[test]
    fn wording_selects_a_stable_modality() {
        let planner = TurnPlanner;
        let session = session();
        let cases = [
            ("summarize this", ResponseModality::Summarize),
            ("what is the big picture?", ResponseModality::ZoomOut),
            ("drill into the details", ResponseModality::DrillDown),
            ("give me bullets", ResponseModality::Bullets),
            ("hello", ResponseModality::Explain),
        ];
        for (query, expected) in cases {
            assert_eq!(
                planner.choose_modality(&context(query), &session),
                expected,
                "{query}"
            );
        }
    }

    #[test]
    fn preferences_move_detail_and_default_format() {
        let planner = TurnPlanner;
        let mut session = session();
        session.preferences.values.insert("detail".into(), 1.0);
        session.preferences.values.insert("format".into(), 0.9);
        assert_eq!(planner.choose_detail_level(&context("hello"), &session), 5);
        assert_eq!(
            planner.choose_modality(&context("hello"), &session),
            ResponseModality::Bullets
        );
    }

    #[test]
    fn plans_are_deterministic_and_shape_follows_modality() {
        let planner = TurnPlanner;
        let session = session();
        let ctx = context("explain this");
        let a = planner.build_plan(&ctx, &session).unwrap();
        let b = planner.build_plan(&ctx, &session).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.chunks.len(), 2);
        assert_eq!(a.chunks[0].chunk_id.as_str(), "concept");
        assert_eq!(a.chunks[1].chunk_id.as_str(), "analogy");

        let summarize = planner.build_plan(&context("summarize"), &session).unwrap();
        assert_eq!(summarize.chunks.len(), 1);
        assert_eq!(summarize.chunks[0].label, "bullets");
    }

    #[test]
    fn tool_policy_requires_opt_in_and_confirmation_wins() {
        let planner = TurnPlanner;
        let ctx = context("look it up");
        assert_eq!(planner.plan_tool_policy(&ctx, &session()), ToolPolicy::None);

        let mut session = session();
        session.preferences.values.insert("tools".into(), 1.0);
        assert_eq!(
            planner.plan_tool_policy(&ctx, &session),
            ToolPolicy::Allowed
        );
        session
            .preferences
            .values
            .insert("confirm_tools".into(), 1.0);
        assert_eq!(
            planner.plan_tool_policy(&ctx, &session),
            ToolPolicy::Confirm
        );

        let mut proactive = ctx.clone();
        proactive.is_proactive = true;
        assert_eq!(
            planner.plan_tool_policy(&proactive, &session),
            ToolPolicy::None
        );
    }

    #[test]
    fn budget_floors_each_chunk() {
        let planner = TurnPlanner;
        let budgets = SessionBudgets {
            turn_token_budget: 100,
            max_turns: 10,
        };
        let budget = planner.allocate_budget(&budgets, 3);
        assert_eq!(budget.tokens, 384);
    }
}
