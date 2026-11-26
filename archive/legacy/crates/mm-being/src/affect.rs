//! Affect: an appraisal-driven control variable that never asserts a fact.
//!
//! MicroPsi's lesson is that emotion is useful to a system exactly as long as it
//! biases *behaviour* and not *belief*. So the two flags that would let affect
//! contaminate state — `affects_internal_state` and `affects_reasoning` — are
//! private fields with no setter, no constructor argument, and no deserializer:
//! [`AffectImpulse`] derives `Serialize` and deliberately not `Deserialize`, so a
//! document cannot smuggle in a `true`. The accessors exist so a caller can *read*
//! the guarantee and a test can assert it.
//!
//! [`appraise`] turns an [`AppraisalEvent`] into an impulse deterministically, and
//! [`decay`] relaxes both the impulse and the affect toward neutral, so a mood
//! cannot outlive the appraisal that produced it.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// An impulse below this intensity is spent and is dropped by [`decay`].
const DECAY_EPSILON: f32 = 1e-3;

/// The shortest decay constant accepted, in seconds, so `dt / decay` cannot
/// divide by zero.
const MIN_DECAY_SECONDS: f32 = 1e-3;

/// The appraisal-derived emotions this substrate recognises.
///
/// The set is closed because each variant maps to a fixed control-signal delta;
/// an open vocabulary would let a caller invent an emotion with no behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Emotion {
    /// A goal advanced, by the being's own action.
    Joy,
    /// A goal was set back, by the being's own action or by circumstance.
    Sadness,
    /// A threat is present and coping looks insufficient.
    Fear,
    /// A goal was blocked by someone else.
    Anger,
    /// Something was unexpected.
    Surprise,
    /// Something is novel and worth exploring.
    Interest,
    /// Someone else advanced a goal.
    Gratitude,
    /// A goal was blocked and the coping was not enough to respond well.
    Frustration,
}

impl Emotion {
    /// The stable wire name, recorded in `affect_impulses.emotion`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Emotion::Joy => "joy",
            Emotion::Sadness => "sadness",
            Emotion::Fear => "fear",
            Emotion::Anger => "anger",
            Emotion::Surprise => "surprise",
            Emotion::Interest => "interest",
            Emotion::Gratitude => "gratitude",
            Emotion::Frustration => "frustration",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "joy" => Some(Emotion::Joy),
            "sadness" => Some(Emotion::Sadness),
            "fear" => Some(Emotion::Fear),
            "anger" => Some(Emotion::Anger),
            "surprise" => Some(Emotion::Surprise),
            "interest" => Some(Emotion::Interest),
            "gratitude" => Some(Emotion::Gratitude),
            "frustration" => Some(Emotion::Frustration),
            _ => None,
        }
    }

    /// How long this emotion lasts, in seconds.
    ///
    /// Surprise is over in a moment; sadness lingers. The constants are the
    /// "personality" of the affect layer and are deliberately few.
    pub fn base_decay(&self) -> f32 {
        match self {
            Emotion::Joy => 3.0,
            Emotion::Sadness => 8.0,
            Emotion::Fear => 6.0,
            Emotion::Anger => 5.0,
            Emotion::Surprise => 1.5,
            Emotion::Interest => 4.0,
            Emotion::Gratitude => 6.0,
            Emotion::Frustration => 4.0,
        }
    }

    /// The per-axis deltas this emotion applies, scaled by intensity.
    ///
    /// Order: `valence, arousal, engagement, warmth, caution, curiosity, energy`.
    fn deltas(&self) -> (f32, f32, f32, f32, f32, f32, f32) {
        match self {
            Emotion::Joy => (0.8, 0.5, 0.5, 0.5, -0.3, 0.2, 0.4),
            Emotion::Sadness => (-0.7, -0.4, -0.5, -0.1, 0.2, -0.2, -0.5),
            Emotion::Fear => (-0.6, 0.8, 0.2, -0.2, 0.9, -0.1, -0.2),
            Emotion::Anger => (-0.6, 0.7, 0.4, -0.4, 0.5, -0.2, 0.3),
            Emotion::Surprise => (0.1, 0.9, 0.6, 0.0, 0.2, 0.5, 0.2),
            Emotion::Interest => (0.3, 0.4, 0.7, 0.2, -0.1, 0.8, 0.3),
            Emotion::Gratitude => (0.7, 0.2, 0.3, 0.7, -0.4, 0.1, 0.2),
            Emotion::Frustration => (-0.5, 0.5, 0.1, -0.3, 0.4, -0.1, -0.2),
        }
    }
}

/// Who caused the outcome being appraised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agency {
    /// The being itself.
    Self_,
    /// Someone or something else.
    Other,
}

impl Agency {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Agency::Self_ => "self",
            Agency::Other => "other",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "self" => Some(Agency::Self_),
            "other" => Some(Agency::Other),
            _ => None,
        }
    }
}

/// The appraisal variables an event is scored on (MicroPsi-style).
///
/// Four numbers in `[0, 1]` (`goal_conduciveness` is signed) and who acted. The
/// event is *evidence about the situation*, never a statement about the world,
/// which is why nothing here is ever promoted to a fact.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AppraisalEvent {
    /// How much the outcome matters to a goal.
    pub goal_relevance: f32,
    /// Whether it helped (`> 0`) or hindered (`< 0`) the goal.
    pub goal_conduciveness: f32,
    /// How unexpected it was.
    pub novelty: f32,
    /// How well the being expects to cope.
    pub coping_potential: f32,
    /// Who acted.
    pub agency: Agency,
}

impl AppraisalEvent {
    /// Build an appraisal event.
    pub fn new(
        goal_relevance: f32,
        goal_conduciveness: f32,
        novelty: f32,
        coping_potential: f32,
        agency: Agency,
    ) -> Self {
        AppraisalEvent {
            goal_relevance,
            goal_conduciveness,
            novelty,
            coping_potential,
            agency,
        }
    }
}

/// The being's affect, a vector of control signals in `[-1, 1]` with `0` neutral.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Affect {
    /// Pleasant to unpleasant.
    pub valence: f32,
    /// Calm to activated.
    pub arousal: f32,
    /// Disengaged to absorbed.
    pub engagement: f32,
    /// Distant to warm.
    pub warmth: f32,
    /// Bold to careful.
    pub caution: f32,
    /// Settled to inquisitive.
    pub curiosity: f32,
    /// Depleted to energetic.
    pub energy: f32,
}

impl Affect {
    /// The resting point every axis is measured from.
    pub fn neutral() -> Self {
        Affect {
            valence: 0.0,
            arousal: 0.0,
            engagement: 0.0,
            warmth: 0.0,
            caution: 0.0,
            curiosity: 0.0,
            energy: 0.0,
        }
    }

    /// Every axis inside `[-1, 1]`.
    pub fn clamped(&self) -> Self {
        Affect {
            valence: clamp_signed(self.valence),
            arousal: clamp_signed(self.arousal),
            engagement: clamp_signed(self.engagement),
            warmth: clamp_signed(self.warmth),
            caution: clamp_signed(self.caution),
            curiosity: clamp_signed(self.curiosity),
            energy: clamp_signed(self.energy),
        }
    }

    /// Apply an impulse: the emotion's deltas scaled by the impulse intensity.
    pub fn apply(&mut self, impulse: &AffectImpulse) {
        let intensity = impulse.intensity.clamp(0.0, 1.0);
        let (valence, arousal, engagement, warmth, caution, curiosity, energy) =
            impulse.emotion.deltas();
        self.valence += intensity * valence;
        self.arousal += intensity * arousal;
        self.engagement += intensity * engagement;
        self.warmth += intensity * warmth;
        self.caution += intensity * caution;
        self.curiosity += intensity * curiosity;
        self.energy += intensity * energy;
        *self = self.clamped();
    }
}

/// One appraised impulse, decaying toward nothing.
///
/// The two `affects_*` fields are private and there is deliberately no
/// constructor, setter, or `Deserialize` that could set them: the phase's headline
/// invariant is that affect can never assert a fact, and this type enforces it by
/// construction rather than by convention.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AffectImpulse {
    emotion: Emotion,
    intensity: f32,
    decay: f32,
    affects_internal_state: bool,
    affects_reasoning: bool,
}

impl AffectImpulse {
    /// The emotion this impulse carries.
    pub fn emotion(&self) -> Emotion {
        self.emotion
    }

    /// How strongly it applies right now.
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// The time constant of its decay, in seconds.
    pub fn decay(&self) -> f32 {
        self.decay
    }

    /// Always `false`: affect biases behaviour, never state.
    pub fn affects_internal_state(&self) -> bool {
        self.affects_internal_state
    }

    /// Always `false`: affect biases behaviour, never reasoning.
    pub fn affects_reasoning(&self) -> bool {
        self.affects_reasoning
    }

    /// A copy at a different intensity, clamped into `[0, 1]`.
    pub fn with_intensity(&self, intensity: f32) -> Self {
        AffectImpulse {
            intensity: intensity.clamp(0.0, 1.0),
            ..self.clone()
        }
    }

    /// Advance the impulse by `dt`, returning its remaining intensity.
    fn advanced(&self, dt: Duration) -> Self {
        let seconds = dt.as_secs_f32().max(0.0);
        let factor = (-seconds / self.decay.max(MIN_DECAY_SECONDS)).exp();
        AffectImpulse {
            intensity: self.intensity * factor,
            ..self.clone()
        }
    }
}

/// Appraise an event into an impulse, deterministically.
///
/// The decision table is small and total, so the same event always produces the
/// same emotion: a stochastic mood would make `being show` unreplayable.
pub fn appraise(e: &AppraisalEvent) -> AffectImpulse {
    let relevance = e.goal_relevance.clamp(0.0, 1.0);
    let conduciveness = clamp_signed(e.goal_conduciveness);
    let novelty = e.novelty.clamp(0.0, 1.0);
    let coping = e.coping_potential.clamp(0.0, 1.0);
    let magnitude = conduciveness.abs();

    let emotion = if relevance < 0.15 {
        // Nothing is at stake: the only sensible signal is interest in the novel.
        Emotion::Interest
    } else if conduciveness > 0.05 {
        match e.agency {
            Agency::Self_ => Emotion::Joy,
            Agency::Other => Emotion::Gratitude,
        }
    } else if conduciveness < -0.05 {
        match (e.agency, coping < 0.3) {
            (Agency::Other, true) => Emotion::Frustration,
            (Agency::Other, false) => Emotion::Anger,
            (Agency::Self_, true) => Emotion::Fear,
            (Agency::Self_, false) => Emotion::Sadness,
        }
    } else if novelty >= 0.5 {
        Emotion::Surprise
    } else {
        Emotion::Interest
    };

    let intensity = if relevance < 0.15 {
        (0.1 + 0.3 * novelty).clamp(0.0, 1.0)
    } else {
        (0.6 * relevance + 0.4 * magnitude + 0.2 * novelty * (1.0 - coping)).clamp(0.0, 1.0)
    };

    AffectImpulse {
        emotion,
        intensity,
        decay: emotion.base_decay(),
        // Not parameters: the invariant. There is no code path that writes `true`.
        affects_internal_state: false,
        affects_reasoning: false,
    }
}

/// Advance time: relax the affect toward neutral and decay every impulse.
///
/// The relaxation uses the fastest per-impulse time constant, so a brief surprise
/// does not pin a mood that a slower sadness would. An impulse whose intensity
/// falls below [`DECAY_EPSILON`] is removed rather than kept as a ghost.
pub fn decay(a: &mut Affect, impulses: &mut Vec<AffectImpulse>, dt: Duration) {
    let seconds = dt.as_secs_f32();
    if !seconds.is_finite() || seconds <= 0.0 {
        return;
    }

    let mut tau = f32::INFINITY;
    for impulse in impulses.iter() {
        tau = tau.min(impulse.decay.max(MIN_DECAY_SECONDS));
    }
    if !tau.is_finite() {
        // No impulse is active; a mood still fades, at the default time constant.
        tau = 1.0;
    }

    let relax = 1.0 - (-seconds / tau).exp();
    a.valence *= 1.0 - relax;
    a.arousal *= 1.0 - relax;
    a.engagement *= 1.0 - relax;
    a.warmth *= 1.0 - relax;
    a.caution *= 1.0 - relax;
    a.curiosity *= 1.0 - relax;
    a.energy *= 1.0 - relax;
    *a = a.clamped();

    for impulse in impulses.iter_mut() {
        *impulse = impulse.advanced(dt);
    }
    impulses.retain(|impulse| impulse.intensity >= DECAY_EPSILON);
}

/// Clamp to the affect range `[-1, 1]`.
fn clamp_signed(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    fn typical() -> AppraisalEvent {
        AppraisalEvent::new(0.8, 0.5, 0.3, 0.7, Agency::Self_)
    }

    #[test]
    fn an_impulse_can_never_affect_state_or_reasoning() {
        let inputs = [
            typical(),
            AppraisalEvent::new(1.0, -0.9, 0.9, 0.0, Agency::Other),
            AppraisalEvent::new(0.0, 0.0, 0.0, 0.0, Agency::Self_),
            AppraisalEvent::new(0.5, 0.0, 1.0, 0.5, Agency::Other),
        ];
        for event in inputs {
            let impulse = appraise(&event);
            assert!(
                !impulse.affects_internal_state(),
                "affect must never change state"
            );
            assert!(
                !impulse.affects_reasoning(),
                "affect must never change reasoning"
            );
            // Even a re-scaled copy keeps the guarantee: the flags are copied, not
            // set, because there is no way to set them.
            assert!(!impulse.with_intensity(1.0).affects_reasoning());
        }
    }

    #[test]
    fn appraisal_is_deterministic_and_total() {
        let first = appraise(&typical());
        let second = appraise(&typical());
        assert_eq!(first, second);
        assert!((0.0..=1.0).contains(&first.intensity()));
        assert!(first.intensity() > 0.0);
        assert!(first.decay() > 0.0);
    }

    #[test]
    fn the_decision_table_picks_the_expected_emotion() {
        let cases = [
            (0.8, 0.5, Agency::Self_, Emotion::Joy),
            (0.8, 0.5, Agency::Other, Emotion::Gratitude),
            (0.8, -0.5, Agency::Other, Emotion::Anger),
            (0.8, -0.5, Agency::Self_, Emotion::Sadness),
            (0.8, -0.5, Agency::Self_, Emotion::Sadness),
            (0.05, 0.0, Agency::Self_, Emotion::Interest),
        ];
        for (relevance, conduciveness, agency, expected) in cases {
            let event = AppraisalEvent::new(relevance, conduciveness, 0.1, 0.8, agency);
            assert_eq!(appraise(&event).emotion(), expected, "{event:?}");
        }

        // Low coping changes the response, not merely the intensity.
        let low_coping = AppraisalEvent::new(0.8, -0.5, 0.2, 0.1, Agency::Self_);
        assert_eq!(appraise(&low_coping).emotion(), Emotion::Fear);
        let low_coping_other = AppraisalEvent::new(0.8, -0.5, 0.2, 0.1, Agency::Other);
        assert_eq!(appraise(&low_coping_other).emotion(), Emotion::Frustration);

        // A relevant but neutral outcome is surprise only when it is novel.
        let novel = AppraisalEvent::new(0.6, 0.0, 0.9, 0.5, Agency::Self_);
        assert_eq!(appraise(&novel).emotion(), Emotion::Surprise);
        let familiar = AppraisalEvent::new(0.6, 0.0, 0.1, 0.5, Agency::Self_);
        assert_eq!(appraise(&familiar).emotion(), Emotion::Interest);
    }

    #[test]
    fn applying_an_impulse_stays_in_range_and_moves_the_expected_axis() {
        let mut affect = Affect::neutral();
        affect.apply(&appraise(&typical()));
        assert!(affect.valence > 0.0);
        assert!((0.0..=1.0).contains(&affect.valence));

        // Even a saturated impulse cannot leave the range.
        let mut saturated = Affect {
            valence: 1.0,
            ..Affect::neutral()
        };
        saturated.apply(&appraise(&typical()));
        assert!(approx(saturated.valence, 1.0));
        assert_eq!(saturated, saturated.clamped());
    }

    #[test]
    fn decay_is_monotone_toward_neutral_and_eventually_removes_the_impulse() {
        let mut affect = Affect::neutral();
        let impulse = appraise(&typical());
        affect.apply(&impulse);
        let mut impulses = vec![impulse.clone()];

        let mut previous = affect.valence;
        assert!(previous > 0.0);
        let mut steps = 0;
        while !impulses.is_empty() && steps < 1000 {
            decay(&mut affect, &mut impulses, Duration::from_secs(1));
            assert!(
                affect.valence <= previous + 1e-6,
                "valence moved away from neutral"
            );
            assert!(affect.valence >= 0.0);
            previous = affect.valence;
            steps += 1;
        }
        assert!(impulses.is_empty(), "an impulse must eventually be spent");
        assert!(affect.valence < 0.05, "valence must relax to neutral");
    }

    #[test]
    fn decay_ignores_a_zero_or_negative_step() {
        let mut affect = Affect {
            arousal: 0.5,
            ..Affect::neutral()
        };
        let before = affect;
        let mut impulses = vec![appraise(&typical())];
        let count = impulses.len();
        decay(&mut affect, &mut impulses, Duration::from_secs(0));
        assert_eq!(affect, before);
        assert_eq!(impulses.len(), count);
    }

    #[test]
    fn with_intensity_clamps_and_never_touches_the_flags() {
        let impulse = appraise(&typical());
        assert!(approx(impulse.with_intensity(9.0).intensity(), 1.0));
        assert!(approx(impulse.with_intensity(-4.0).intensity(), 0.0));
        assert!(!impulse.with_intensity(0.2).affects_internal_state());
    }

    #[test]
    fn wire_names_round_trip() {
        for emotion in [
            Emotion::Joy,
            Emotion::Sadness,
            Emotion::Fear,
            Emotion::Anger,
            Emotion::Surprise,
            Emotion::Interest,
            Emotion::Gratitude,
            Emotion::Frustration,
        ] {
            assert_eq!(Emotion::parse(emotion.as_str()), Some(emotion));
            assert!(emotion.base_decay() > 0.0);
        }
        assert_eq!(Emotion::parse("elation"), None);
        assert_eq!(Agency::parse(Agency::Self_.as_str()), Some(Agency::Self_));
        assert_eq!(Agency::parse(Agency::Other.as_str()), Some(Agency::Other));
        assert_eq!(Agency::parse("nobody"), None);
    }
}
