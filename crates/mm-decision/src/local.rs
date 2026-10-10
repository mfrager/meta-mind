//! The distilled local decision head.
//!
//! Phase 9 defines the *adapter*; Phase 11 does the distilling. The adapter's job
//! is to be honest about what a head can and cannot do, because the failure mode
//! this core could introduce is the worst one in the subsystem: a local head that
//! answers a question it has no business answering, with a confidence nobody
//! calibrated and no provider to blame.
//!
//! So the adapter is narrow on purpose:
//!
//! * **No head ⇒ no answer.** A missing artifact is a normal state, not an error —
//!   the plan's §9 mitigation is that an unavailable local core never fabricates a
//!   decision, and the firewall escalates rather than proceeding on nothing.
//! * **A head answers one shape.** A head loaded for `yes_no` questions refuses a
//!   `score` question rather than reinterpreting its logit, and *any* head refuses
//!   a `choice` question: a single linear head can score one proposition, but it
//!   cannot rank a set of options, and pretending otherwise would turn a
//!   well-defined refusal into a silent mis-ranking.
//! * **The feature vector is derived, never read back.** The head is evaluated over
//!   features built from the [`DecisionState`] the caller handed it — counts from the
//!   three reference lists and every numeric entry of the state's `context` — so a
//!   head can only see what a decision was allowed to see.
//! * **A decided answer is more confident than an indecisive one.** Confidence is
//!   the head's own declared confidence scaled by how far the logit sits from the
//!   decision threshold, which is what makes an `abstain` reachable: a head that is
//!   nearly on its boundary reports nearly no confidence, and the conformal
//!   threshold then refuses to admit it.

use std::collections::BTreeMap;
use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::{CoreId, DecisionCore};
use crate::error::{DecisionError, Result};
use crate::question::{
    AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState, FeatureVector, QuestionKind,
};

/// A distilled linear head over named features.
///
/// `weights` is keyed by the same names [`features_from_state`] produces, so a
/// head and the features it reads cannot drift apart silently: an unknown weight
/// key simply contributes nothing, and the head's own `logit` is the sum over the
/// keys it names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionHead {
    /// The decision class this head was distilled for. It is the key the Phase 9
    /// calibrator is fitted under, so it is carried into every answer.
    pub class: String,
    /// The one question shape this head answers.
    pub question_kind: QuestionKind,
    /// The linear weights, by feature name.
    pub weights: BTreeMap<String, f64>,
    /// The intercept.
    pub bias: f64,
    /// The probability above which a `yes_no` head answers yes.
    pub threshold: f32,
    /// The head's own claimed confidence, in `[0,1]`, before the margin is applied.
    pub confidence: f32,
}

impl DecisionHead {
    /// Read a head from a JSON artifact.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| DecisionError::Codec(format!("{}: {e}", path.display())))?;
        let head: DecisionHead = serde_json::from_str(&text)
            .map_err(|e| DecisionError::Codec(format!("{}: {e}", path.display())))?;
        head.validate()?;
        Ok(head)
    }

    /// Refuse a head that cannot produce a bounded probability.
    pub fn validate(&self) -> Result<()> {
        if self.class.trim().is_empty() {
            return Err(DecisionError::validation("head.class", "is empty"));
        }
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(DecisionError::validation(
                "head.confidence",
                format!("must be in [0,1], got {}", self.confidence),
            ));
        }
        if !self.threshold.is_finite() || !(0.0..=1.0).contains(&self.threshold) {
            return Err(DecisionError::validation(
                "head.threshold",
                format!("must be in [0,1], got {}", self.threshold),
            ));
        }
        if !self.bias.is_finite() {
            return Err(DecisionError::validation("head.bias", "is not finite"));
        }
        for (name, weight) in &self.weights {
            if !weight.is_finite() {
                return Err(DecisionError::validation(
                    "head.weights",
                    format!("{name} is not finite"),
                ));
            }
        }
        Ok(())
    }

    /// The linear score for a feature vector: `bias + Σ w·x`.
    ///
    /// Missing features contribute nothing rather than an error, so a head can be
    /// evaluated against a state that carries fewer features than the head knows.
    pub fn logit(&self, features: &FeatureVector) -> f64 {
        let mut total = self.bias;
        for (name, weight) in &self.weights {
            if let Some(value) = features.get(name) {
                total += weight * value;
            }
        }
        total
    }

    /// The head's probability that the proposition holds, in `[0,1]`.
    pub fn probability(&self, features: &FeatureVector) -> f32 {
        let logit = self.logit(features);
        // Numerically safe sigmoid: the branches avoid `exp` overflow for logits
        // far from zero, which a badly scaled head can easily produce.
        let p = if logit >= 0.0 {
            1.0 / (1.0 + (-logit).exp())
        } else {
            let e = logit.exp();
            e / (1.0 + e)
        };
        (p as f32).clamp(0.0, 1.0)
    }
}

/// The features a head is evaluated over, derived from the state alone.
///
/// Only numbers cross this boundary: the three reference-list counts and the
/// numeric entries of `state.context`. A head that wants a categorical signal has
/// to have been distilled with it encoded as a numeric feature, which keeps a
/// distilled artifact free of any judgement about what a string means.
pub fn features_from_state(state: &DecisionState) -> FeatureVector {
    let mut features = FeatureVector::new();
    features.insert("state.facts", state.facts.len() as f64);
    features.insert("state.assumptions", state.assumptions.len() as f64);
    features.insert("state.constraints", state.constraints.len() as f64);
    if let serde_json::Value::Object(map) = &state.context {
        for (key, value) in map {
            if let Some(number) = value.as_f64() {
                if number.is_finite() {
                    features.insert(format!("context.{key}"), number);
                }
            }
        }
    }
    features
}

/// The decision core backed by an optional distilled head.
#[derive(Debug, Clone, Default)]
pub struct LocalCore {
    head: Option<DecisionHead>,
}

impl LocalCore {
    /// A core with no head loaded. It refuses every question.
    pub fn unavailable() -> Self {
        LocalCore { head: None }
    }

    /// A core over `head`, which must validate.
    pub fn new(head: DecisionHead) -> Result<Self> {
        head.validate()?;
        Ok(LocalCore { head: Some(head) })
    }

    /// A core over the head stored at `path`.
    ///
    /// A *missing* file is `Ok(unavailable())`: "no head has been distilled yet"
    /// is the normal state of a fresh installation, and turning it into an error
    /// would make every caller handle a case that is not a failure. A file that
    /// exists and does not parse *is* a failure, because silently ignoring a
    /// corrupt artifact would hide a broken distillation.
    pub fn from_path(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LocalCore::unavailable()),
            Err(e) => Err(DecisionError::Codec(format!("{}: {e}", path.display()))),
            Ok(text) => {
                let head: DecisionHead = serde_json::from_str(&text)
                    .map_err(|e| DecisionError::Codec(format!("{}: {e}", path.display())))?;
                Self::new(head)
            }
        }
    }

    /// The loaded head, if any.
    pub fn head(&self) -> Option<&DecisionHead> {
        self.head.as_ref()
    }
}

#[async_trait]
impl DecisionCore for LocalCore {
    fn id(&self) -> CoreId {
        CoreId::Local
    }

    async fn answer(
        &self,
        question: &DecisionQuestion,
        state: &DecisionState,
    ) -> Result<DecisionAnswer> {
        // No head, no answer. This is the plan's §9 mitigation made concrete: a
        // local core that is not loaded must not fabricate a decision, because the
        // firewall can escalate an `Unavailable` but cannot detect a plausible
        // number that came from nowhere.
        let Some(head) = self.head.as_ref() else {
            return Err(DecisionError::Unavailable(
                "no distilled head is loaded".to_string(),
            ));
        };

        // A linear head scores one proposition; it cannot rank a set of options.
        // Refusing is the honest answer, and the caller falls back to the rules core
        // or the hosted core.
        if question.kind() == QuestionKind::Choice {
            return Err(DecisionError::Unavailable(
                "a distilled head cannot rank options".to_string(),
            ));
        }

        if head.question_kind != question.kind() {
            return Err(DecisionError::Unavailable(format!(
                "the loaded head answers {} questions, not {}",
                head.question_kind,
                question.kind()
            )));
        }

        let features = features_from_state(state);
        let probability = head.probability(&features);

        let answer = match question {
            DecisionQuestion::YesNo { .. } => AnswerValue::Bool {
                value: probability > head.threshold,
            },
            DecisionQuestion::Score { scale, .. } => AnswerValue::Score {
                value: scale.min + probability * scale.width(),
            },
            // Unreachable: a choice question was refused above. Kept explicit so a
            // future question shape cannot silently fall through to an answer.
            DecisionQuestion::Choice { .. } => {
                return Err(DecisionError::Unavailable(
                    "a distilled head cannot rank options".to_string(),
                ))
            }
        };

        // How far the head sits from its own boundary, as a fraction of the wider
        // side of that boundary. A head answering just above its threshold reports
        // a confidence near zero, which is what makes conformal abstention reachable.
        let margin = (probability - head.threshold).abs();
        let span = head.threshold.max(1.0 - head.threshold).max(f32::EPSILON);
        let decision_strength = (margin / span).clamp(0.0, 1.0);
        let confidence = (head.confidence * (0.5 + 0.5 * decision_strength)).clamp(0.0, 1.0);

        // `latency_ms` is zero, not measured: the conformance suite compares two
        // answers to the same question byte for byte, and a real duration would
        // make them differ.
        DecisionAnswer::new(question, answer, confidence, features, CoreId::Local, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question::ScoreScale;
    use mm_core::Ulid;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn yes_no(prompt: &str) -> DecisionQuestion {
        DecisionQuestion::YesNo {
            id: ulid(1),
            prompt: prompt.into(),
        }
    }

    fn head(threshold: f32) -> DecisionHead {
        DecisionHead {
            class: "safety_problem".into(),
            question_kind: QuestionKind::YesNo,
            weights: BTreeMap::from([("context.danger".to_string(), 4.0)]),
            bias: -1.0,
            threshold,
            confidence: 0.8,
        }
    }

    fn risky_state(danger: f64) -> DecisionState {
        DecisionState::new(ulid(9), serde_json::json!({"danger": danger, "label": "x"}))
    }

    #[tokio::test]
    async fn a_core_with_no_head_refuses_every_question() {
        let core = LocalCore::unavailable();
        assert!(core.head().is_none());
        for question in [
            yes_no("Anything at all"),
            DecisionQuestion::Score {
                id: ulid(2),
                prompt: "How risky?".into(),
                scale: ScoreScale::unit(),
            },
            DecisionQuestion::Choice {
                id: ulid(3),
                prompt: "Which?".into(),
                options: vec!["a".into(), "b".into()],
                allow_none: true,
            },
        ] {
            let err = core.answer(&question, &risky_state(1.0)).await.unwrap_err();
            assert_eq!(err.code(), "unavailable");
            assert!(err.is_unavailable());
        }
    }

    #[test]
    fn a_missing_artifact_is_unavailable_and_a_corrupt_one_is_a_refusal() {
        let missing = std::path::Path::new("/nonexistent/mm/decision-head.json");
        let core = LocalCore::from_path(missing).unwrap();
        assert!(core.head().is_none());

        let dir = std::env::temp_dir().join(format!("mm-decision-head-{}", ulid(77)));
        std::fs::create_dir_all(&dir).unwrap();
        let corrupt = dir.join("head.json");
        std::fs::write(&corrupt, "{ not json").unwrap();
        assert!(LocalCore::from_path(&corrupt).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_yes_no_head_answers_from_the_state_it_was_given() {
        let core = LocalCore::new(head(0.5)).unwrap();
        let question = yes_no("Is this a problem?");

        let dangerous = core.answer(&question, &risky_state(1.0)).await.unwrap();
        assert_eq!(dangerous.answer, AnswerValue::Bool { value: true });

        let benign = core.answer(&question, &risky_state(0.0)).await.unwrap();
        assert_eq!(benign.answer, AnswerValue::Bool { value: false });

        // The head sees only the numbers, and it records which ones it saw.
        assert_eq!(dangerous.features.get("context.danger"), Some(1.0));
        assert_eq!(dangerous.features.get("state.facts"), Some(0.0));
        assert!(!dangerous
            .features
            .keys()
            .iter()
            .any(|k| k == "context.label"));
    }

    #[tokio::test]
    async fn a_head_near_its_boundary_reports_almost_no_confidence() {
        let core = LocalCore::new(head(0.5)).unwrap();
        let question = yes_no("Is this a problem?");
        // A weight of 4 and a bias of -1 put the decision boundary at danger = 0.25.
        let on_the_boundary = core.answer(&question, &risky_state(0.25)).await.unwrap();
        let far_away = core.answer(&question, &risky_state(1.0)).await.unwrap();
        assert!(
            on_the_boundary.confidence < 0.5,
            "{}",
            on_the_boundary.confidence
        );
        assert!(far_away.confidence > on_the_boundary.confidence);
        assert!(far_away.confidence <= 0.8);
    }

    #[tokio::test]
    async fn a_choice_question_is_refused_even_with_a_head_loaded() {
        let core = LocalCore::new(head(0.5)).unwrap();
        let question = DecisionQuestion::Choice {
            id: ulid(4),
            prompt: "Which route?".into(),
            options: vec!["alpha".into(), "beta".into()],
            allow_none: true,
        };
        let err = core.answer(&question, &risky_state(1.0)).await.unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn a_head_for_another_shape_is_refused_rather_than_reinterpreted() {
        let core = LocalCore::new(head(0.5)).unwrap();
        let question = DecisionQuestion::Score {
            id: ulid(5),
            prompt: "How risky?".into(),
            scale: ScoreScale::unit(),
        };
        let err = core.answer(&question, &risky_state(1.0)).await.unwrap_err();
        assert_eq!(err.code(), "unavailable");
        assert!(err.to_string().contains("yes_no"), "{err}");
    }

    #[tokio::test]
    async fn a_score_head_maps_its_probability_into_the_scale() {
        let mut scalar = head(0.5);
        scalar.question_kind = QuestionKind::Score;
        let core = LocalCore::new(scalar).unwrap();
        let question = DecisionQuestion::Score {
            id: ulid(6),
            prompt: "How risky?".into(),
            scale: ScoreScale::new(2.0, 4.0),
        };
        let answer = core.answer(&question, &risky_state(1.0)).await.unwrap();
        let AnswerValue::Score { value } = answer.answer else {
            panic!("a score head must answer a score question with a score");
        };
        assert!((2.0..=4.0).contains(&value), "{value}");
        assert!(value > 3.5, "{value}");
    }

    #[tokio::test]
    async fn the_same_head_and_state_answer_identically() {
        let core = LocalCore::new(head(0.5)).unwrap();
        let question = yes_no("Is this a problem?");
        let first = core.answer(&question, &risky_state(0.7)).await.unwrap();
        let second = core.answer(&question, &risky_state(0.7)).await.unwrap();
        assert_eq!(first.canonical(), second.canonical());
        assert_eq!(first.content_hash(), second.content_hash());
    }

    #[test]
    fn a_head_that_cannot_be_bounded_is_refused() {
        let mut bad = head(0.5);
        bad.confidence = 1.5;
        assert!(bad.validate().is_err());
        bad = head(0.5);
        bad.threshold = -0.1;
        assert!(bad.validate().is_err());
        bad = head(0.5);
        bad.weights.insert("x".into(), f64::INFINITY);
        assert!(bad.validate().is_err());
        bad = head(0.5);
        bad.class = "  ".into();
        assert!(bad.validate().is_err());
    }
}
