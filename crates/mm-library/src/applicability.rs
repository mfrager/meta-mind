//! Deterministic selection: the activation blend, and the applicability score.
//!
//! The plan's invariant 5 is "deterministic selection": `applicability()` and
//! `activation_score()` are pure arithmetic, and the ranking they produce is
//! golden-tested. Nothing here consults a model, a clock, or a store — a state is
//! passed in, a score comes out, and the same state always yields the same rank.
//!
//! Three components, each with one job:
//!
//! * **activation** — an ACT-R-style blend of recent use, how often the entry has
//!   been used, and the entry's own stored utility. The weights are fixed constants
//!   so a change to the formula is a change to a checked artefact.
//! * **fitness** — how well the entry has worked in the domains the state names,
//!   from its per-domain fitness map. An untried domain contributes the neutral
//!   `0.5`: unknown is not the same as bad, and treating it as zero would bury every
//!   new entry.
//! * **contraindication penalty** — a subtraction, because a contraindication is a
//!   warning rather than a fit. It is the only component that can *lower* a score
//!   below what its factors would give, which is what makes it worth stating.

use mm_core::Timestamp;
use serde::{Deserialize, Serialize};

use crate::entry::{Declarative, LibraryEntry};
use crate::error::Result;

/// Weight of recent use in the activation blend.
pub const W_RECENCY: f32 = 0.4;
/// Weight of use frequency in the activation blend.
pub const W_FREQUENCY: f32 = 0.3;
/// Weight of the entry's own utility in the activation blend.
pub const W_UTILITY: f32 = 0.3;
/// The neutral fitness of a domain the entry has never been used in.
pub const NEUTRAL_FITNESS: f32 = 0.5;
/// How much a matched contraindication costs.
pub const CONTRAINDICATION_COST: f32 = 0.4;
/// How many uses count as "used often".
pub const FREQUENCY_SATURATION: usize = 3;
/// Recent use scores this; never having been used scores half of it.
pub const RECENCY_SEEN: f32 = 1.0;
/// The recency of an entry the state has never used.
pub const RECENCY_UNSEEN: f32 = 0.5;
/// The stakes at which "high stakes" becomes part of the state's text.
pub const HIGH_STAKES: f32 = 0.7;

/// The ACT-R-style activation blend.
///
/// Pure, and clamped: a caller cannot make an activation exceed 1 by passing large
/// inputs, so a rank cannot be gamed by inflating one component.
pub fn activation_score(recency: f32, frequency: f32, utility: f32) -> f32 {
    let recency = recency.clamp(0.0, 1.0);
    let frequency = frequency.clamp(0.0, 1.0);
    let utility = utility.clamp(0.0, 1.0);
    (W_RECENCY * recency + W_FREQUENCY * frequency + W_UTILITY * utility).clamp(0.0, 1.0)
}

/// What the being is facing, as the selector sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplicabilityState {
    /// The frames active right now.
    pub frames: Vec<String>,
    /// The goal being pursued.
    pub goal: String,
    /// The domains in play.
    pub domain: Vec<String>,
    /// How uncertain the being is, in `[0,1]`.
    pub uncertainty: f32,
    /// How much is at stake, in `[0,1]`.
    pub stakes: f32,
    /// The entries already used in this episode.
    pub history: Vec<String>,
    /// The instant the state is evaluated at.
    pub now: Timestamp,
}

impl ApplicabilityState {
    /// A state with everything empty and nothing at stake.
    pub fn new(goal: &str, now: Timestamp) -> Self {
        ApplicabilityState {
            frames: Vec::new(),
            goal: goal.to_string(),
            domain: Vec::new(),
            uncertainty: 0.0,
            stakes: 0.0,
            history: Vec::new(),
            now,
        }
    }

    /// Add a domain.
    pub fn with_domain(mut self, domain: &str) -> Self {
        self.domain.push(domain.to_string());
        self
    }

    /// Add an active frame.
    pub fn with_frame(mut self, frame: &str) -> Self {
        self.frames.push(frame.to_string());
        self
    }

    /// Set how much is at stake.
    pub fn with_stakes(mut self, stakes: f32) -> Self {
        self.stakes = stakes.clamp(0.0, 1.0);
        self
    }

    /// Set how uncertain the being is.
    pub fn with_uncertainty(mut self, uncertainty: f32) -> Self {
        self.uncertainty = uncertainty.clamp(0.0, 1.0);
        self
    }

    /// Record that an entry was used.
    pub fn with_history(mut self, iri: &str) -> Self {
        self.history.push(iri.to_string());
        self
    }

    /// The lowercase text a condition is matched against: the frames, the goal and
    /// the domains, plus `high stakes` when the stakes warrant it.
    ///
    /// One place defines the haystack, so a contraindication cannot match one
    /// component and be invisible to another.
    pub fn condition_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.frames.iter().cloned());
        parts.push(self.goal.clone());
        parts.extend(self.domain.iter().cloned());
        if self.stakes >= HIGH_STAKES {
            parts.push("high stakes".to_string());
        }
        parts.join(" ").to_ascii_lowercase()
    }

    /// The state's hash: a function of the ranked inputs, not of the clock.
    pub fn hash(&self) -> String {
        let mut fields: Vec<String> = self.frames.clone();
        fields.push(self.goal.clone());
        fields.extend(self.domain.iter().cloned());
        fields.extend(self.history.iter().cloned());
        fields.push(format!("{:.6}", self.uncertainty));
        fields.push(format!("{:.6}", self.stakes));
        let borrowed: Vec<&str> = fields.iter().map(String::as_str).collect();
        mm_core::hash_fields(&borrowed)
    }
}

/// The score, and the components it was built from.
///
/// The components are carried rather than folded away because a rank a caller
/// cannot explain is a rank a caller cannot argue with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ApplicabilityScore {
    /// The final score, in `[0,1]`.
    pub total: f32,
    /// The activation blend.
    pub activation: f32,
    /// The domain fitness.
    pub fitness: f32,
    /// What the contraindications cost.
    pub contraindication_penalty: f32,
}

impl ApplicabilityScore {
    /// The score as the log records it.
    pub fn components(&self) -> serde_json::Value {
        serde_json::json!({
            "activation": self.activation,
            "fitness": self.fitness,
            "contraindication_penalty": self.contraindication_penalty,
        })
    }
}

/// How well the entry has worked in the state's domains.
///
/// The mean of the entry's fitness over the state's domains, with
/// [`NEUTRAL_FITNESS`] for a domain it has never been used in. A state with no
/// domains gets the neutral value rather than zero: nothing is known, which is not
/// the same as nothing being good.
pub fn fitness(entry: &Declarative, state: &ApplicabilityState) -> f32 {
    if state.domain.is_empty() {
        return NEUTRAL_FITNESS;
    }
    let total: f32 = state
        .domain
        .iter()
        .map(|domain| {
            entry
                .domain_fitness
                .get(domain)
                .copied()
                .unwrap_or(NEUTRAL_FITNESS)
        })
        .sum();
    (total / state.domain.len() as f32).clamp(0.0, 1.0)
}

/// What the entry's contraindications cost against this state.
pub fn contraindication_penalty(entry: &Declarative, state: &ApplicabilityState) -> f32 {
    if entry.contraindications.is_empty() {
        return 0.0;
    }
    let text = state.condition_text();
    let matched = entry
        .contraindications
        .iter()
        .filter(|condition| condition.matches(&text))
        .count();
    CONTRAINDICATION_COST * (matched as f32 / entry.contraindications.len() as f32)
}

/// The entry's applicability to a state.
pub fn applicability(entry: &Declarative, state: &ApplicabilityState) -> ApplicabilityScore {
    let iri = entry.version_iri().into_string();
    let uses = state.history.iter().filter(|used| **used == iri).count();
    let recency = if uses > 0 {
        RECENCY_SEEN
    } else {
        RECENCY_UNSEEN
    };
    let frequency = (uses as f32 / FREQUENCY_SATURATION as f32).clamp(0.0, 1.0);
    let activation = activation_score(recency, frequency, entry.activation_score);
    let fitness = fitness(entry, state);
    let penalty = contraindication_penalty(entry, state);
    let total = (0.5 * activation + 0.5 * fitness - penalty).clamp(0.0, 1.0);
    ApplicabilityScore {
        total,
        activation,
        fitness,
        contraindication_penalty: penalty,
    }
}

/// One ranked entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    /// The entry's versioned IRI.
    pub iri: String,
    /// Its title.
    pub title: String,
    /// Its score.
    pub score: ApplicabilityScore,
}

/// Rank the applicable entries, best first.
///
/// Deterministic: ties break on the IRI, so two runs over the same state produce
/// the same list in the same order. Entries whose `applicable_when` no condition
/// matches are ranked *below* those that match rather than dropped — a caller that
/// asked for the top five gets five, with the reason visible in the score — but the
/// match is part of the arithmetic, so a matched entry outranks an unmatched one of
/// equal fitness.
pub fn rank(entries: &[Declarative], state: &ApplicabilityState, top: usize) -> Vec<Ranked> {
    let text = state.condition_text();
    let mut ranked: Vec<(bool, Ranked)> = entries
        .iter()
        .map(|entry| {
            let mut score = applicability(entry, state);
            let matched = entry.applicable_when.is_empty()
                || entry
                    .applicable_when
                    .iter()
                    .any(|condition| condition.matches(&text));
            if !matched {
                // A condition that is stated but unmet is evidence against, and it
                // costs the same as half a contraindication.
                score.contraindication_penalty =
                    (score.contraindication_penalty + CONTRAINDICATION_COST / 2.0).min(1.0);
                score.total = (score.total - CONTRAINDICATION_COST / 2.0).clamp(0.0, 1.0);
            }
            (
                matched,
                Ranked {
                    iri: entry.version_iri().into_string(),
                    title: entry.title.clone(),
                    score,
                },
            )
        })
        .collect();
    ranked.sort_by(|(a_matched, a), (b_matched, b)| {
        b_matched
            .cmp(a_matched)
            .then_with(|| {
                b.score
                    .total
                    .partial_cmp(&a.score.total)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.iri.cmp(&b.iri))
    });
    ranked
        .into_iter()
        .map(|(_, ranked)| ranked)
        .take(top)
        .collect()
}

/// Rank the techniques and heuristics in an entry set: the kinds a selector may
/// pick, in the order the plan lists them.
pub fn selectable(entries: &[Declarative]) -> Vec<&Declarative> {
    entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                crate::entry::EntryKind::Technique
                    | crate::entry::EntryKind::Heuristic
                    | crate::entry::EntryKind::Pattern
            )
        })
        .collect()
}

/// The state a fixture file describes.
pub fn state_from_json(value: &serde_json::Value) -> Result<ApplicabilityState> {
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let list = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let number = |key: &str| value.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let now = match text("now") {
        Some(text) => Timestamp::from_rfc3339(&text)
            .map_err(|e| crate::error::LibraryError::Config(format!("state now: {e}")))?,
        None => Timestamp::EPOCH,
    };
    Ok(ApplicabilityState {
        frames: list("frames"),
        goal: text("goal").unwrap_or_default(),
        domain: list("domain"),
        uncertainty: number("uncertainty"),
        stakes: number("stakes"),
        history: list("history"),
        now,
    })
}
