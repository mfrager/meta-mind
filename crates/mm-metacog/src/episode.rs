//! A cognitive episode: one bounded deliberation, and the provisional model it
//! starts from.
//!
//! Design §88 asks for deliberation to be a *bounded* thing with a budget and a
//! timescale, not an open-ended reflection. This module is that boundary. An
//! episode holds references to records owned by other organs — claims and
//! assumptions from `/epistemic`, frames and techniques from `/library`, a goal
//! from the being — and never a copy of them, so there is one authority for each
//! and no second one can drift.
//!
//! ## What an identifier is here
//!
//! Records with a row in the tabular store are `Ulid`s (claims, assumptions,
//! goals, outcomes — the phase index's identifier decision). Things that are
//! *named* rather than *minted* — a library entry, a tool, a candidate answer —
//! are IRIs stored as `String`, because a library frame's identity is its path
//! and must survive a round trip through Turtle.

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

use crate::budget::CognitiveBudget;
use crate::error::{MetacogError, Result};
use crate::tier::Tier;

/// The identifier of an episode.
pub type EpisodeId = Ulid;
/// The identifier of a compiled program.
pub type ProgramId = Ulid;
/// The identifier of a persisted trace.
pub type TraceId = Ulid;

/// A claim's ULID (`/epistemic`).
pub type ClaimId = Ulid;
/// An assumption's ULID (`/epistemic`).
pub type AssumptionId = Ulid;
/// A goal's ULID (the being).
pub type GoalId = Ulid;
/// An outcome's ULID (`/epistemic`).
pub type OutcomeId = Ulid;

/// A frame's IRI (`/library`).
pub type FrameId = String;
/// A doctrine's IRI (`/library`).
pub type DoctrineId = String;
/// A technique's IRI (`/library`).
pub type TechniqueId = String;
/// A reference class's IRI (`/library`).
pub type RefClassId = String;
/// An evaluation criterion's IRI (`/library`).
pub type CriterionId = String;
/// A tool's identity.
pub type ToolId = String;
/// A candidate answer's identity.
pub type CandidateId = String;
/// An action's identity.
pub type ActionId = String;
/// A scenario's identity.
pub type ScenarioId = String;
/// An observation source's identity.
pub type SourceId = String;
/// A constraint's identity.
pub type ConstraintId = String;
/// A proposition's identity within the epistemic store.
pub type PropositionId = String;
/// A case's IRI (`/library`).
pub type CaseId = String;

/// The timescale an episode runs at.
///
/// The being's scheduler (a later phase) owns multi-timescale control; an episode
/// is the "minutes" scale, and the enum exists so a caller that schedules
/// differently says so rather than being silently reinterpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Timescale {
    /// Under a second.
    Seconds,
    /// The episode's native scale.
    Minutes,
    /// Hours.
    Hours,
    /// Days.
    Days,
}

impl Timescale {
    /// Every scale, shortest first.
    pub const ALL: [Timescale; 4] = [
        Timescale::Seconds,
        Timescale::Minutes,
        Timescale::Hours,
        Timescale::Days,
    ];

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Timescale::Seconds => "seconds",
            Timescale::Minutes => "minutes",
            Timescale::Hours => "hours",
            Timescale::Days => "days",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        Timescale::ALL.into_iter().find(|t| t.as_str() == text)
    }
}

impl std::fmt::Display for Timescale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where an episode is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeStatus {
    /// Opened, nothing compiled yet.
    Open,
    /// A program exists for it.
    Programmed,
    /// The loop stopped before the budget ran out.
    Stopped,
    /// Its outcome is recorded.
    Closed,
}

impl EpisodeStatus {
    /// Every status, in life order.
    pub const ALL: [EpisodeStatus; 4] = [
        EpisodeStatus::Open,
        EpisodeStatus::Programmed,
        EpisodeStatus::Stopped,
        EpisodeStatus::Closed,
    ];

    /// The stable wire name, matching the `episodes.status` check constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            EpisodeStatus::Open => "open",
            EpisodeStatus::Programmed => "programmed",
            EpisodeStatus::Stopped => "stopped",
            EpisodeStatus::Closed => "closed",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        EpisodeStatus::ALL.into_iter().find(|s| s.as_str() == text)
    }
}

impl std::fmt::Display for EpisodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the controller knows about the situation before it thinks.
///
/// `novelty` and `time_pressure` enter from the caller — the phase's invariant 6
/// says nothing here computes a probability or a judgement, so a caller that does
/// not know says `0.0` rather than being guessed at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Context {
    /// One-line statement of what is being decided.
    pub summary: String,
    /// How unlike anything already handled this is, in `[0,1]`.
    pub novelty: f64,
    /// How little time there is, in `[0,1]`.
    pub time_pressure: f64,
}

impl Context {
    /// A context with just a summary.
    pub fn new(summary: impl Into<String>) -> Self {
        Context {
            summary: summary.into(),
            novelty: 0.0,
            time_pressure: 0.0,
        }
    }

    /// How novel the situation is.
    pub fn with_novelty(mut self, novelty: f64) -> Self {
        self.novelty = novelty;
        self
    }

    /// How pressed for time the being is.
    pub fn with_time_pressure(mut self, pressure: f64) -> Self {
        self.time_pressure = pressure;
        self
    }

    /// Refuse a context that cannot be reasoned about.
    pub fn validate(&self) -> Result<()> {
        if self.summary.trim().is_empty() {
            return Err(MetacogError::validation(
                "context.summary",
                "must not be empty, it is what `Recall` is asked for",
            ));
        }
        check_unit("context.novelty", self.novelty)?;
        check_unit("context.time_pressure", self.time_pressure)
    }
}

/// A bounded deliberation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CognitiveEpisode {
    /// The episode's ULID. Its `trace_id` in the log is the same value.
    pub id: EpisodeId,
    /// The goal being pursued.
    pub goal: GoalId,
    /// The provisional model of the problem.
    pub context: Context,
    /// The claims the deliberation may rely on.
    pub claims: Vec<ClaimId>,
    /// The assumptions the deliberation rests on.
    pub assumptions: Vec<AssumptionId>,
    /// Identifiers of the uncertainties the caller already knows about.
    pub uncertainties: Vec<String>,
    /// The constraints the answer must respect.
    pub constraints: Vec<ConstraintId>,
    /// The frames that are active.
    pub active_frames: Vec<FrameId>,
    /// The doctrines that are active.
    pub active_doctrines: Vec<DoctrineId>,
    /// The techniques that are active.
    pub active_techniques: Vec<TechniqueId>,
    /// The candidate answers under consideration.
    pub candidates: Vec<CandidateId>,
    /// The evidence already gathered.
    pub evidence: Vec<String>,
    /// The risks already named.
    pub risks: Vec<String>,
    /// The decision, once one is taken.
    pub decision: Option<String>,
    /// The budget this deliberation may spend.
    pub budget: CognitiveBudget,
    /// The policy tier. `T0` until the selector picks one.
    pub tier: Tier,
    /// The scale this episode runs at.
    pub timescale: Timescale,
}

impl CognitiveEpisode {
    /// Open an episode. It starts `open`, at `T0`, with the given budget.
    pub fn new(
        id: EpisodeId,
        goal: GoalId,
        context: Context,
        budget: CognitiveBudget,
    ) -> Result<Self> {
        let episode = CognitiveEpisode {
            id,
            goal,
            context,
            claims: Vec::new(),
            assumptions: Vec::new(),
            uncertainties: Vec::new(),
            constraints: Vec::new(),
            active_frames: Vec::new(),
            active_doctrines: Vec::new(),
            active_techniques: Vec::new(),
            candidates: Vec::new(),
            evidence: Vec::new(),
            risks: Vec::new(),
            decision: None,
            budget,
            tier: Tier::T0,
            timescale: Timescale::Minutes,
        };
        episode.validate()?;
        Ok(episode)
    }

    /// The claims the deliberation may rely on.
    pub fn with_claims(mut self, claims: Vec<ClaimId>) -> Self {
        self.claims = claims;
        self
    }

    /// The assumptions the deliberation rests on.
    pub fn with_assumptions(mut self, assumptions: Vec<AssumptionId>) -> Self {
        self.assumptions = assumptions;
        self
    }

    /// The candidate answers.
    pub fn with_candidates(mut self, candidates: Vec<CandidateId>) -> Self {
        self.candidates = candidates;
        self
    }

    /// The active frames.
    pub fn with_frames(mut self, frames: Vec<FrameId>) -> Self {
        self.active_frames = frames;
        self
    }

    /// The active techniques.
    pub fn with_techniques(mut self, techniques: Vec<TechniqueId>) -> Self {
        self.active_techniques = techniques;
        self
    }

    /// The constraints the answer must respect.
    pub fn with_constraints(mut self, constraints: Vec<ConstraintId>) -> Self {
        self.constraints = constraints;
        self
    }

    /// The scale this episode runs at.
    pub fn with_timescale(mut self, timescale: Timescale) -> Self {
        self.timescale = timescale;
        self
    }

    /// The statement of what is being decided, which is what `Recall` asks for.
    pub fn goal_text(&self) -> &str {
        &self.context.summary
    }

    /// How many candidates are under consideration.
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// Refuse an episode that cannot be compiled.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil() {
            return Err(MetacogError::validation("id", "must not be the nil ULID"));
        }
        if self.goal.is_nil() {
            return Err(MetacogError::validation("goal", "must not be the nil ULID"));
        }
        self.context.validate()?;
        self.budget.validate()?;
        Ok(())
    }

    /// Set the running tier: the situation's own selection, raised to the tier the
    /// caller pinned when the caller pinned one.
    ///
    /// A caller may only raise the tier, never lower it. `T0` means "let the
    /// situation decide", so the common case never pins anything; a pinned tier is
    /// an operator overriding the selector, which the budget still bounds.
    pub fn select_tier(&mut self, scan: &crate::scan::ScanResult) -> Tier {
        self.tier = self.tier.max(Tier::select(self, scan));
        self.tier
    }

    /// The instant the episode's bitemporal interval opens.
    pub fn valid_from(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A unit-interval field check shared by the types in this crate.
pub(crate) fn check_unit(field: &'static str, value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(MetacogError::validation(
            field,
            format!("must be in [0,1], got {value}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::ScanResult;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn episode() -> CognitiveEpisode {
        CognitiveEpisode::new(
            ulid(1),
            ulid(2),
            Context::new("decide whether to roll back the deploy"),
            CognitiveBudget::from_spec(8, 4, 0.5, 60_000).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn an_episode_starts_open_at_t0_with_its_goal_text() {
        let ep = episode();
        assert_eq!(ep.tier, Tier::T0);
        assert_eq!(ep.timescale, Timescale::Minutes);
        assert_eq!(ep.goal_text(), "decide whether to roll back the deploy");
        ep.validate().unwrap();
    }

    #[test]
    fn a_nil_id_or_goal_is_refused() {
        let err = CognitiveEpisode::new(
            Ulid::nil(),
            ulid(2),
            Context::new("x"),
            CognitiveBudget::from_spec(1, 1, 1.0, 1).unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "validation");

        let err = CognitiveEpisode::new(
            ulid(1),
            Ulid::nil(),
            Context::new("x"),
            CognitiveBudget::from_spec(1, 1, 1.0, 1).unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("goal"));
    }

    #[test]
    fn an_empty_summary_is_refused_because_recall_would_have_nothing_to_ask_for() {
        let err = CognitiveEpisode::new(
            ulid(1),
            ulid(2),
            Context::new("   "),
            CognitiveBudget::from_spec(1, 1, 1.0, 1).unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("summary"), "{err}");
    }

    #[test]
    fn the_selector_raises_the_tier_and_a_pinned_tier_is_a_floor() {
        let mut ep = episode().with_candidates(vec!["a".into(), "b".into()]);
        let scan = ScanResult {
            stakes: 0.9,
            irreversibility: 0.9,
            ..ScanResult::default()
        };
        let chosen = ep.select_tier(&scan);
        assert!(chosen > Tier::T0, "high stakes must raise the tier");
        assert_eq!(chosen, ep.tier);

        // Pinning a tier raises but never lowers: the situation may still ask for
        // more, and an operator override is a floor rather than a ceiling.
        let mut pinned = episode();
        pinned.tier = Tier::T5;
        assert_eq!(pinned.select_tier(&ScanResult::default()), Tier::T5);
        let mut calm = episode();
        assert_eq!(calm.select_tier(&ScanResult::default()), Tier::T0);
    }

    #[test]
    fn timescale_and_status_round_trip() {
        for scale in Timescale::ALL {
            assert_eq!(Timescale::parse(scale.as_str()), Some(scale));
        }
        for status in EpisodeStatus::ALL {
            assert_eq!(EpisodeStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(EpisodeStatus::parse("nope"), None);
        assert_eq!(Timescale::parse("nope"), None);
    }
}
