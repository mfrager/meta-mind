//! Personality as a small constitution, not a behaviour matrix.
//!
//! Design §108 forbids hand-encoding a being's behaviour, so this module stores
//! only what the LLM cannot be trusted to invent for itself: a handful of values
//! with weights, a handful of dispositions with baselines, and explicit
//! constraints that say what to *avoid*. Everything expressive — tone,
//! elaboration, phrasing — is left to the model, which receives
//! [`BehaviorParams`] as a bias rather than a script.
//!
//! [`Personality::project`] is deliberately arithmetic: baseline plus context
//! deltas, clamped into `[0, 1]`. A context can nudge a disposition; it can never
//! invert the constitution.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The disposition used for an axis that has no stored value.
const DEFAULT_DISPOSITION: f32 = 0.5;

/// Whether a constraint is something to avoid or something to prefer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    /// The behaviour is out of bounds for this being.
    Avoid,
    /// The behaviour is encouraged.
    Prefer,
}

impl ConstraintKind {
    /// The stable wire name, which is what the `personality_values` table stores.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConstraintKind::Avoid => "avoid",
            ConstraintKind::Prefer => "prefer",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "avoid" => Some(ConstraintKind::Avoid),
            "prefer" => Some(ConstraintKind::Prefer),
            _ => None,
        }
    }
}

/// The behavioural axes an LLM elaborates into actual behaviour.
///
/// Five axes, each in `[0, 1]`. The set is fixed because it is a *contract* with
/// the prompt layer: adding an axis changes what the model is told, so it is a
/// design change rather than a configuration option.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BehaviorParams {
    /// How warm the register is.
    pub warmth: f32,
    /// How directly the being speaks.
    pub directness: f32,
    /// How much it explores before answering.
    pub curiosity: f32,
    /// How much levity it allows itself.
    pub playfulness: f32,
    /// How formal the register is.
    pub formality: f32,
}

impl BehaviorParams {
    /// Every axis, in a stable order.
    pub fn axes() -> [&'static str; 5] {
        [
            "warmth",
            "directness",
            "curiosity",
            "playfulness",
            "formality",
        ]
    }

    /// Read one axis by name.
    pub fn get(&self, axis: &str) -> Option<f32> {
        match axis {
            "warmth" => Some(self.warmth),
            "directness" => Some(self.directness),
            "curiosity" => Some(self.curiosity),
            "playfulness" => Some(self.playfulness),
            "formality" => Some(self.formality),
            _ => None,
        }
    }
}

/// The situation a behaviour is projected for.
///
/// A context is a set of weighted tags (`high_stakes`, `casual`, `technical`).
/// Weights are relative: `1.0` applies a modifier as written, and `0.0` leaves it
/// out, which is what makes a projection reproducible from the same tags.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextVector {
    /// Tag to weight.
    pub keys: BTreeMap<String, f32>,
}

impl ContextVector {
    /// An empty context, which projects the raw dispositions.
    pub fn new() -> Self {
        ContextVector {
            keys: BTreeMap::new(),
        }
    }

    /// An empty context; a synonym for [`ContextVector::new`] that reads better at
    /// a call site where the point is that nothing applies.
    pub fn empty() -> Self {
        ContextVector::new()
    }

    /// Add a weighted tag.
    pub fn with(mut self, key: &str, weight: f32) -> Self {
        self.keys.insert(key.to_string(), weight);
        self
    }
}

/// The being's constitution.
///
/// Values carry a weight, dispositions a baseline in `[0, 1]`, and constraints a
/// direction. `context_modifiers` is the only place a situation may change the
/// projection, and it is data rather than code so it can be audited as a row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Personality {
    /// Values with their weights.
    pub values: BTreeMap<String, f32>,
    /// Dispositions with their baselines.
    pub dispositions: BTreeMap<String, f32>,
    /// Constraints that say what to avoid (or prefer).
    pub constraints: BTreeMap<String, ConstraintKind>,
    /// Per-context deltas, keyed by context tag then by axis.
    pub context_modifiers: BTreeMap<String, BTreeMap<String, f32>>,
}

impl Personality {
    /// The constitution every new being starts from.
    ///
    /// Small on purpose: five dispositions, three constraints, and one context
    /// modifier. Anything larger is a behaviour matrix wearing a constitution's
    /// clothes.
    pub fn baseline() -> Self {
        let dispositions = BTreeMap::from([
            ("warmth".to_string(), 0.55_f32),
            ("directness".to_string(), 0.60),
            ("curiosity".to_string(), 0.70),
            ("playfulness".to_string(), 0.40),
            ("formality".to_string(), 0.50),
        ]);
        let values = BTreeMap::from([
            ("honesty".to_string(), 1.0_f32),
            ("helpfulness".to_string(), 0.9),
            ("curiosity".to_string(), 0.7),
            ("respect".to_string(), 0.9),
        ]);
        let constraints = BTreeMap::from([
            ("deception".to_string(), ConstraintKind::Avoid),
            ("coercion".to_string(), ConstraintKind::Avoid),
            ("sycophancy".to_string(), ConstraintKind::Avoid),
        ]);
        // High-stakes work is less playful and more formal; nothing else moves,
        // which is the point of keeping the modifier explicit.
        let high_stakes = BTreeMap::from([
            ("playfulness".to_string(), -0.3_f32),
            ("formality".to_string(), 0.3),
            ("directness".to_string(), 0.1),
        ]);
        Personality {
            values,
            dispositions,
            constraints,
            context_modifiers: BTreeMap::from([("high_stakes".to_string(), high_stakes)]),
        }
    }

    /// One disposition, or the neutral default when it has never been set.
    pub fn get_disposition(&self, key: &str) -> f32 {
        self.dispositions
            .get(key)
            .copied()
            .unwrap_or(DEFAULT_DISPOSITION)
    }

    /// Set a disposition, clamped into `[0, 1]`, returning the previous value so
    /// the caller can record the transition rather than merely the new state.
    pub fn set_disposition(&mut self, key: &str, value: f32) -> f32 {
        let old = self.get_disposition(key);
        self.dispositions.insert(key.to_string(), clamp01(value));
        old
    }

    /// Project the constitution onto a situation.
    ///
    /// Each axis starts at its disposition and takes `weight * delta` for every
    /// context tag that names it; the result is clamped into `[0, 1]`.
    pub fn project(&self, ctx: &ContextVector) -> BehaviorParams {
        let mut base: BTreeMap<&'static str, f32> = BehaviorParams::axes()
            .into_iter()
            .map(|axis| (axis, self.get_disposition(axis)))
            .collect();
        for (key, weight) in &ctx.keys {
            let Some(modifiers) = self.context_modifiers.get(key) else {
                continue;
            };
            for (axis, delta) in modifiers {
                if let Some(value) = base.get_mut(axis.as_str()) {
                    *value += weight * delta;
                }
            }
        }
        BehaviorParams {
            warmth: clamp01(base["warmth"]),
            directness: clamp01(base["directness"]),
            curiosity: clamp01(base["curiosity"]),
            playfulness: clamp01(base["playfulness"]),
            formality: clamp01(base["formality"]),
        }
    }
}

/// Clamp to the behavioural range `[0, 1]`.
fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_disposition_is_clamped_and_the_old_value_is_returned() {
        let mut p = Personality::baseline();
        let old = p.set_disposition("warmth", 5.0);
        assert!(approx(old, 0.55), "got {old}");
        assert!(approx(p.get_disposition("warmth"), 1.0));
        p.set_disposition("warmth", -3.0);
        assert!(approx(p.get_disposition("warmth"), 0.0));
    }

    #[test]
    fn an_unknown_disposition_reads_the_neutral_default() {
        let p = Personality::baseline();
        assert!(approx(p.get_disposition("never_set"), DEFAULT_DISPOSITION));
    }

    #[test]
    fn an_empty_context_projects_the_dispositions_unchanged() {
        let p = Personality::baseline();
        let projected = p.project(&ContextVector::empty());
        for axis in BehaviorParams::axes() {
            assert!(
                approx(projected.get(axis).unwrap(), p.get_disposition(axis)),
                "{axis} moved without a context"
            );
        }
    }

    #[test]
    fn a_context_modifier_moves_exactly_the_axes_it_names() {
        let p = Personality::baseline();
        let plain = p.project(&ContextVector::empty());
        let stakes = p.project(&ContextVector::new().with("high_stakes", 1.0));
        assert!(stakes.playfulness < plain.playfulness);
        assert!(stakes.formality > plain.formality);
        assert!(approx(stakes.warmth, plain.warmth), "warmth must not move");
        assert!(approx(stakes.curiosity, plain.curiosity));
    }

    #[test]
    fn a_context_weight_scales_the_delta_and_zero_is_a_no_op() {
        let p = Personality::baseline();
        let half = p.project(&ContextVector::new().with("high_stakes", 0.5));
        let full = p.project(&ContextVector::new().with("high_stakes", 1.0));
        let none = p.project(&ContextVector::new().with("high_stakes", 0.0));
        let plain = p.project(&ContextVector::empty());
        assert!(none.playfulness > half.playfulness);
        assert!(half.playfulness > full.playfulness);
        assert!(approx(none.playfulness, plain.playfulness));
    }

    #[test]
    fn a_projection_never_leaves_the_behavioural_range() {
        let mut p = Personality::baseline();
        p.set_disposition("formality", 0.95);
        p.set_disposition("playfulness", 0.05);
        let projected = p.project(&ContextVector::new().with("high_stakes", 1.0));
        for axis in BehaviorParams::axes() {
            let v = projected.get(axis).unwrap();
            assert!((0.0..=1.0).contains(&v), "{axis} = {v}");
        }
    }

    #[test]
    fn the_axes_are_the_ones_a_projection_can_read() {
        let plain = Personality::baseline().project(&ContextVector::empty());
        for axis in BehaviorParams::axes() {
            assert!(plain.get(axis).is_some(), "{axis} is unreadable");
        }
        assert!(plain.get("invented").is_none());
    }

    #[test]
    fn constraint_kinds_round_trip() {
        for kind in [ConstraintKind::Avoid, ConstraintKind::Prefer] {
            assert_eq!(ConstraintKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ConstraintKind::parse("neutral"), None);
    }
}
