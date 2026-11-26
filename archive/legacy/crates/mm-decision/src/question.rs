//! The bounded question, its answer, and the state both hang off.
//!
//! A decision in this system is never "what should we do". It is a *bounded
//! question* with a closed answer type — pick one of these options, score on this
//! scale, or answer yes/no — asked against a frozen state. That is what lets three
//! very different backings (a rule table, a distilled head, a hosted model) answer
//! the *same* question and be compared by one conformance suite: the shape of the
//! answer is part of the question, and a core that answers outside it is wrong
//! rather than creative.
//!
//! Three properties are enforced here rather than trusted:
//!
//! * **Confidence is bounded by construction.** [`DecisionAnswer::new`] refuses a
//!   confidence outside `[0,1]`; nothing downstream has to clamp or police it.
//! * **The state has a digest.** Every decision is asked against a
//!   [`DecisionState`], and the digest of that state is what a decision cache keys
//!   on, so a cached answer can never be replayed against a state it did not see.
//! * **An answer is canonicalizable.** [`DecisionAnswer::canonical`] is a fixed
//!   rendering with the confidence at nine decimals, so "the same answer" is a
//!   string comparison and the conformance suite can prove determinism.

use std::collections::BTreeMap;

use mm_core::Ulid;
use serde::{Deserialize, Serialize};

use crate::core::CoreId;
use crate::error::{DecisionError, Result};

/// An option a `Choice` question offers. Options are strings, not indices,
/// because the answer is read by humans in an audit and by a caller that has to
/// act on it.
pub type OptionId = String;

/// One anchor on a score scale: what a value *means*.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoreAnchor {
    /// Where on the scale the anchor sits.
    pub at: f32,
    /// What that point means.
    pub label: String,
}

/// A score scale: the closed interval the answer must land in, plus the anchors
/// that make a bare number interpretable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoreScale {
    /// The lowest admissible value.
    pub min: f32,
    /// The highest admissible value.
    pub max: f32,
    /// The labelled points inside the interval.
    pub anchors: Vec<ScoreAnchor>,
}

impl ScoreScale {
    /// The unit interval, with the three anchors every probability-like score
    /// shares.
    pub fn unit() -> Self {
        ScoreScale {
            min: 0.0,
            max: 1.0,
            anchors: vec![
                ScoreAnchor {
                    at: 0.0,
                    label: "no chance".into(),
                },
                ScoreAnchor {
                    at: 0.5,
                    label: "even".into(),
                },
                ScoreAnchor {
                    at: 1.0,
                    label: "certain".into(),
                },
            ],
        }
    }

    /// A scale from `min` to `max` with no anchors.
    pub fn new(min: f32, max: f32) -> Self {
        ScoreScale {
            min,
            max,
            anchors: Vec::new(),
        }
    }

    /// Refuse a scale that cannot be scored on.
    pub fn validate(&self) -> Result<()> {
        if !self.min.is_finite() || !self.max.is_finite() || self.min >= self.max {
            return Err(DecisionError::validation(
                "scale",
                format!("needs a finite min < max, got {}..{}", self.min, self.max),
            ));
        }
        for anchor in &self.anchors {
            if !anchor.at.is_finite() || anchor.at < self.min || anchor.at > self.max {
                return Err(DecisionError::validation(
                    "scale.anchors",
                    format!(
                        "anchor at {} is outside {}..{}",
                        anchor.at, self.min, self.max
                    ),
                ));
            }
            if anchor.label.trim().is_empty() {
                return Err(DecisionError::validation(
                    "scale.anchors",
                    "anchor label is empty",
                ));
            }
        }
        Ok(())
    }

    /// How wide the scale is.
    pub fn width(&self) -> f32 {
        self.max - self.min
    }
}

/// The three question shapes. Nothing else is a bounded question.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionQuestion {
    /// Pick one of `options`, or nothing when `allow_none`.
    Choice {
        /// The question's ULID.
        id: Ulid,
        /// What is being asked.
        prompt: String,
        /// The options on offer.
        options: Vec<OptionId>,
        /// Whether "none of these" is an admissible answer.
        allow_none: bool,
    },
    /// Score on a closed scale with anchors.
    Score {
        /// The question's ULID.
        id: Ulid,
        /// What is being asked.
        prompt: String,
        /// The scale the answer must land in.
        scale: ScoreScale,
    },
    /// Answer yes or no.
    YesNo {
        /// The question's ULID.
        id: Ulid,
        /// What is being asked.
        prompt: String,
    },
}

/// The wire name of a question shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// A closed choice.
    Choice,
    /// A bounded score.
    Score,
    /// A yes/no judgment.
    YesNo,
}

/// Every question shape, in wire order.
pub const QUESTION_KINDS: [QuestionKind; 3] = [
    QuestionKind::Choice,
    QuestionKind::Score,
    QuestionKind::YesNo,
];

impl QuestionKind {
    /// The stable wire name, which is also the `decisions.question_kind` value.
    pub fn as_str(self) -> &'static str {
        match self {
            QuestionKind::Choice => "choice",
            QuestionKind::Score => "score",
            QuestionKind::YesNo => "yes_no",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        QUESTION_KINDS
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}

impl std::fmt::Display for QuestionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl DecisionQuestion {
    /// The question's ULID.
    pub fn id(&self) -> Ulid {
        match self {
            DecisionQuestion::Choice { id, .. }
            | DecisionQuestion::Score { id, .. }
            | DecisionQuestion::YesNo { id, .. } => *id,
        }
    }

    /// What is being asked.
    pub fn prompt(&self) -> &str {
        match self {
            DecisionQuestion::Choice { prompt, .. }
            | DecisionQuestion::Score { prompt, .. }
            | DecisionQuestion::YesNo { prompt, .. } => prompt,
        }
    }

    /// Which shape this is.
    pub fn kind(&self) -> QuestionKind {
        match self {
            DecisionQuestion::Choice { .. } => QuestionKind::Choice,
            DecisionQuestion::Score { .. } => QuestionKind::Score,
            DecisionQuestion::YesNo { .. } => QuestionKind::YesNo,
        }
    }

    /// The options on offer, empty for a question that has none.
    pub fn options(&self) -> &[OptionId] {
        match self {
            DecisionQuestion::Choice { options, .. } => options,
            _ => &[],
        }
    }

    /// Refuse a question that cannot be answered unambiguously.
    pub fn validate(&self) -> Result<()> {
        if self.prompt().trim().is_empty() {
            return Err(DecisionError::validation("prompt", "is empty"));
        }
        match self {
            DecisionQuestion::Choice {
                options,
                allow_none,
                ..
            } => {
                if options.is_empty() {
                    return Err(DecisionError::validation(
                        "options",
                        "a choice question needs at least one option",
                    ));
                }
                if !allow_none && options.len() == 1 {
                    // A one-option question that forbids "none" is not a question:
                    // the answer is fixed and the caller should not be asking.
                    return Err(DecisionError::validation(
                        "options",
                        "a single-option choice must allow none, or it is not a question",
                    ));
                }
                let mut seen = std::collections::BTreeSet::new();
                for option in options {
                    if option.trim().is_empty() {
                        return Err(DecisionError::validation("options", "an option is empty"));
                    }
                    if !seen.insert(option) {
                        return Err(DecisionError::validation(
                            "options",
                            format!("{option:?} is offered twice"),
                        ));
                    }
                }
            }
            DecisionQuestion::Score { scale, .. } => scale.validate()?,
            DecisionQuestion::YesNo { .. } => {}
        }
        Ok(())
    }

    /// A stable rendering, used for the rules table's matching and the decision
    /// cache key.
    pub fn canonical(&self) -> String {
        match self {
            DecisionQuestion::Choice {
                id,
                prompt,
                options,
                allow_none,
            } => format!(
                "choice:{}:{}:{allow_none}:{}",
                mm_core::ulid_string(id),
                prompt.trim().to_lowercase(),
                options.join("|")
            ),
            DecisionQuestion::Score { id, prompt, scale } => format!(
                "score:{}:{}:{}..{}",
                mm_core::ulid_string(id),
                prompt.trim().to_lowercase(),
                scale.min,
                scale.max
            ),
            DecisionQuestion::YesNo { id, prompt } => format!(
                "yes_no:{}:{}",
                mm_core::ulid_string(id),
                prompt.trim().to_lowercase()
            ),
        }
    }
}

/// A bounded answer. The variant is fixed by the question's kind, and the tag is
/// what makes a mismatched answer a schema error rather than a surprise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerValue {
    /// One of the offered options.
    Option {
        /// The chosen option.
        value: OptionId,
    },
    /// A point on the question's scale.
    Score {
        /// The score.
        value: f32,
    },
    /// Yes or no.
    Bool {
        /// The answer.
        value: bool,
    },
    /// No answer: the core declined rather than guessing.
    None,
}

impl AnswerValue {
    /// Which question shape this answers.
    pub fn kind(&self) -> QuestionKind {
        match self {
            AnswerValue::Option { .. } => QuestionKind::Choice,
            AnswerValue::Score { .. } => QuestionKind::Score,
            AnswerValue::Bool { .. } => QuestionKind::YesNo,
            AnswerValue::None => QuestionKind::Choice,
        }
    }

    /// A stable rendering, so "the same answer" is a string comparison.
    pub fn canonical(&self) -> String {
        match self {
            AnswerValue::Option { value } => format!("option:{value}"),
            AnswerValue::Score { value } => format!("score:{value:.6}"),
            AnswerValue::Bool { value } => format!("bool:{value}"),
            AnswerValue::None => "none".to_string(),
        }
    }

    /// True when the core declined to answer.
    pub fn is_none(&self) -> bool {
        matches!(self, AnswerValue::None)
    }
}

/// The state a question is asked against: what the system already holds.
///
/// This is deliberately a *narrow* view — the ids the episode carries and a JSON
/// context — rather than the whole kernel, because a decision core must not be
/// able to reach back into the stores and answer from something the caller did not
/// hand it. The digest is what a decision cache keys on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionState {
    /// The episode the decision belongs to.
    pub episode: Ulid,
    /// Claim ULIDs in evidence.
    pub facts: Vec<Ulid>,
    /// Assumption ULIDs in play.
    pub assumptions: Vec<Ulid>,
    /// Constraint identifiers in play.
    pub constraints: Vec<String>,
    /// Anything else the caller wants the core to see, as JSON.
    pub context: serde_json::Value,
}

impl DecisionState {
    /// A state with no facts, assumptions or constraints.
    pub fn new(episode: Ulid, context: serde_json::Value) -> Self {
        DecisionState {
            episode,
            facts: Vec::new(),
            assumptions: Vec::new(),
            constraints: Vec::new(),
            context,
        }
    }

    /// A state with the three reference lists filled in.
    pub fn with_refs(
        episode: Ulid,
        facts: Vec<Ulid>,
        assumptions: Vec<Ulid>,
        constraints: Vec<String>,
        context: serde_json::Value,
    ) -> Self {
        DecisionState {
            episode,
            facts,
            assumptions,
            constraints,
            context,
        }
    }

    /// A digest of the state, stable across runs and independent of list order.
    pub fn digest(&self) -> String {
        mm_core::content_hash(self.canonical().as_bytes())
    }

    /// The canonical rendering the digest is taken over.
    pub fn canonical(&self) -> String {
        let mut facts: Vec<String> = self.facts.iter().map(mm_core::ulid_string).collect();
        let mut assumptions: Vec<String> =
            self.assumptions.iter().map(mm_core::ulid_string).collect();
        let mut constraints = self.constraints.clone();
        facts.sort();
        assumptions.sort();
        constraints.sort();
        format!(
            "episode={}\nfacts={}\nassumptions={}\nconstraints={}\ncontext={}",
            mm_core::ulid_string(&self.episode),
            facts.join(","),
            assumptions.join(","),
            constraints.join(","),
            crate::canonical_json(&self.context)
        )
    }
}

/// The features a decision was made from, keyed by name.
///
/// A map rather than a struct because Phase 11 learns from these: the keys are the
/// contract a calibration model is fitted on, and a new signal must not be a
/// breaking change to the type. Values are `f64` so a feature can be a count, a
/// rate, a flag or a score without a second type.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FeatureVector(pub BTreeMap<String, f64>);

impl FeatureVector {
    /// An empty vector.
    pub fn new() -> Self {
        FeatureVector(BTreeMap::new())
    }

    /// Add or replace a feature.
    pub fn with(mut self, key: impl Into<String>, value: f64) -> Self {
        self.insert(key, value);
        self
    }

    /// Add or replace a feature in place.
    pub fn insert(&mut self, key: impl Into<String>, value: f64) {
        self.0.insert(key.into(), value);
    }

    /// One feature's value.
    pub fn get(&self, key: &str) -> Option<f64> {
        self.0.get(key).copied()
    }

    /// The feature names, sorted.
    pub fn keys(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }

    /// How many features there are.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A fixed rendering: sorted keys, nine decimals. Two decisions made from the
    /// same features render identically whatever order the features arrived in.
    pub fn canonical(&self) -> String {
        self.0
            .iter()
            .map(|(key, value)| format!("{key}={value:.9}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The names joined for a log field.
    pub fn keys_field(&self) -> String {
        self.keys().join(",")
    }
}

/// One core's answer to one bounded question.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionAnswer {
    /// The question this answers.
    pub question: Ulid,
    /// The question's shape, carried so a stored answer is self-describing.
    pub question_kind: QuestionKind,
    /// The answer.
    pub answer: AnswerValue,
    /// The pre-calibration confidence, in `[0,1]` by construction.
    pub confidence: f32,
    /// The features the answer was made from.
    pub features: FeatureVector,
    /// Which core answered.
    pub core: CoreId,
    /// How long the core took.
    pub latency_ms: u32,
}

impl DecisionAnswer {
    /// Build an answer, refusing one that is not bounded.
    ///
    /// The question is checked against the answer here — right shape, an option
    /// that was actually offered, a score inside the scale — because a core that
    /// answers outside its question is the failure mode the whole conformance
    /// suite exists to catch, and catching it at construction means no core has to
    /// remember to.
    pub fn new(
        question: &DecisionQuestion,
        answer: AnswerValue,
        confidence: f32,
        features: FeatureVector,
        core: CoreId,
        latency_ms: u32,
    ) -> Result<Self> {
        question.validate()?;
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(DecisionError::validation(
                "confidence",
                format!("must be in [0,1], got {confidence}"),
            ));
        }
        let answer = match (question, answer) {
            (DecisionQuestion::Choice { options, .. }, AnswerValue::Option { value }) => {
                if !options.iter().any(|option| option == &value) {
                    return Err(DecisionError::Schema(format!(
                        "{value:?} was not offered by the question"
                    )));
                }
                AnswerValue::Option { value }
            }
            (DecisionQuestion::Choice { allow_none, .. }, AnswerValue::None) => {
                if !allow_none {
                    return Err(DecisionError::Schema(
                        "the question does not allow an empty answer".to_string(),
                    ));
                }
                AnswerValue::None
            }
            (DecisionQuestion::Score { scale, .. }, AnswerValue::Score { value }) => {
                if !value.is_finite() || value < scale.min || value > scale.max {
                    return Err(DecisionError::Schema(format!(
                        "score {value} is outside {}..{}",
                        scale.min, scale.max
                    )));
                }
                AnswerValue::Score { value }
            }
            (DecisionQuestion::YesNo { .. }, AnswerValue::Bool { value }) => {
                AnswerValue::Bool { value }
            }
            (question, answer) => {
                return Err(DecisionError::Schema(format!(
                    "a {} question cannot be answered with a {} answer",
                    question.kind(),
                    answer.canonical()
                )))
            }
        };
        Ok(DecisionAnswer {
            question: question.id(),
            question_kind: question.kind(),
            answer,
            confidence,
            features,
            core,
            latency_ms,
        })
    }

    /// A stable rendering of the answer, the confidence and the features.
    pub fn canonical(&self) -> String {
        format!(
            "question={},kind={},answer={},confidence={:.9},core={},features=[{}]",
            mm_core::ulid_string(&self.question),
            self.question_kind.as_str(),
            self.answer.canonical(),
            self.confidence,
            self.core.as_str(),
            self.features.canonical()
        )
    }

    /// The content hash of the answer, so two runs compare as strings.
    pub fn content_hash(&self) -> String {
        mm_core::content_hash(self.canonical().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CoreId;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn choice() -> DecisionQuestion {
        DecisionQuestion::Choice {
            id: ulid(1),
            prompt: "Which mitigation?".into(),
            options: vec!["roll back".into(), "fail over".into()],
            allow_none: true,
        }
    }

    #[test]
    fn an_answer_outside_its_question_is_refused() {
        let question = choice();
        let err = DecisionAnswer::new(
            &question,
            AnswerValue::Option {
                value: "rewrite it".into(),
            },
            0.5,
            FeatureVector::new(),
            CoreId::Rules,
            0,
        )
        .unwrap_err();
        assert_eq!(err.code(), "schema");

        let err = DecisionAnswer::new(
            &question,
            AnswerValue::Bool { value: true },
            0.5,
            FeatureVector::new(),
            CoreId::Rules,
            0,
        )
        .unwrap_err();
        assert_eq!(err.code(), "schema");

        let score = DecisionQuestion::Score {
            id: ulid(2),
            prompt: "How risky?".into(),
            scale: ScoreScale::unit(),
        };
        assert!(DecisionAnswer::new(
            &score,
            AnswerValue::Score { value: 1.5 },
            0.5,
            FeatureVector::new(),
            CoreId::Rules,
            0,
        )
        .is_err());
        assert!(DecisionAnswer::new(
            &score,
            AnswerValue::Score { value: 0.5 },
            0.5,
            FeatureVector::new(),
            CoreId::Rules,
            0,
        )
        .is_ok());
    }

    #[test]
    fn confidence_is_bounded_by_construction() {
        let question = choice();
        for bad in [-0.01_f32, 1.01, f32::NAN, f32::NEG_INFINITY] {
            assert!(
                DecisionAnswer::new(
                    &question,
                    AnswerValue::None,
                    bad,
                    FeatureVector::new(),
                    CoreId::Rules,
                    0,
                )
                .is_err(),
                "{bad} was accepted as a confidence"
            );
        }
        for good in [0.0_f32, 0.5, 1.0] {
            assert!(
                DecisionAnswer::new(
                    &question,
                    AnswerValue::None,
                    good,
                    FeatureVector::new(),
                    CoreId::Rules,
                    0,
                )
                .is_ok(),
                "{good} was refused as a confidence"
            );
        }
    }

    #[test]
    fn a_state_digest_ignores_list_order() {
        let a = DecisionState::with_refs(
            ulid(1),
            vec![ulid(2), ulid(3)],
            vec![ulid(4)],
            vec!["b".into(), "a".into()],
            serde_json::json!({"x": 1, "y": [2, 3]}),
        );
        let b = DecisionState::with_refs(
            ulid(1),
            vec![ulid(3), ulid(2)],
            vec![ulid(4)],
            vec!["a".into(), "b".into()],
            serde_json::json!({"y": [2, 3], "x": 1}),
        );
        assert_eq!(a.digest(), b.digest());
        assert_eq!(a.digest().len(), 64);
    }

    #[test]
    fn a_question_that_is_not_a_question_is_refused() {
        let fixed = DecisionQuestion::Choice {
            id: ulid(1),
            prompt: "Proceed?".into(),
            options: vec!["yes".into()],
            allow_none: false,
        };
        assert!(fixed.validate().is_err());

        let duplicated = DecisionQuestion::Choice {
            id: ulid(1),
            prompt: "Proceed?".into(),
            options: vec!["yes".into(), "yes".into()],
            allow_none: true,
        };
        assert!(duplicated.validate().is_err());

        let empty_prompt = DecisionQuestion::YesNo {
            id: ulid(1),
            prompt: "   ".into(),
        };
        assert!(empty_prompt.validate().is_err());

        let bad_scale = DecisionQuestion::Score {
            id: ulid(1),
            prompt: "How much?".into(),
            scale: ScoreScale {
                min: 1.0,
                max: 0.0,
                anchors: Vec::new(),
            },
        };
        assert!(bad_scale.validate().is_err());
    }

    #[test]
    fn the_feature_vector_renders_independently_of_insertion_order() {
        let a = FeatureVector::new()
            .with("danger", 1.0)
            .with("matched", 0.5);
        let b = FeatureVector::new()
            .with("matched", 0.5)
            .with("danger", 1.0);
        assert_eq!(a.canonical(), b.canonical());
        assert_eq!(a.keys(), vec!["danger".to_string(), "matched".to_string()]);
    }
}
