//! The table-driven decision core.
//!
//! `RulesCore` is the core that is *always* available: a plain table in
//! `rules/decision_rules.toml`, no model, no head, no I/O at answer time. It exists
//! because the safety-critical, high-volume bounded calls cannot depend on a
//! provider being reachable — an authorization question has to have an answer when
//! the network is down.
//!
//! Three decisions are built into the shape of this module:
//!
//! * **Matching is a conjunction, and the tie-break is fixed.** A rule applies when
//!   its kind equals the question's shape and *every* keyword appears in the prompt;
//!   among the rules that apply, the highest priority wins and equal priorities are
//!   broken by `id` ascending. Without that last clause two runs could pick
//!   different rules for the same question and the core would not be deterministic.
//! * **A mis-configured rule is skipped, not fatal.** If the winning rule names an
//!   option the question never offered, or a score outside its scale, the next rule
//!   in rank order is tried. A table edit that makes one rule unusable must degrade
//!   the answer to the table's fallback, not take the kernel's decision path down.
//! * **A question with no admissible rule has a *stated* fallback.** A `Choice`
//!   that allows none answers "none", a `Score` answers the middle of its scale, a
//!   `YesNo` answers `false` — all at `default_confidence`, which is deliberately
//!   low so that the firewall treats a fallback as almost no evidence. A `Choice`
//!   that forbids "none" has no fallback at all and is refused as
//!   [`DecisionError::Unavailable`], because inventing a choice is the one thing a
//!   core must never do.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::{CoreId, DecisionCore};
use crate::error::{DecisionError, Result};
use crate::question::{
    AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState, FeatureVector, QuestionKind,
};

/// What a rule answers. The variant is tied to the rule's `kind`, and
/// [`RuleTable::validate`] refuses a table where the two disagree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleAnswer {
    /// Choose this option.
    Option {
        /// The option's text, which must be one the question offers.
        value: String,
    },
    /// Answer this point on the question's scale.
    Score {
        /// The score, which must be inside the question's scale.
        value: f32,
    },
    /// Answer yes or no.
    Bool {
        /// The answer.
        value: bool,
    },
    /// Decline: the table has no opinion and the question allows "none".
    None,
}

/// One row of the table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// The rule's stable identifier. It is the tie-break, so it is part of the
    /// contract and not just a label.
    pub id: String,
    /// Higher wins. Ties are broken by `id` ascending.
    pub priority: i64,
    /// The question shape the rule answers.
    pub kind: QuestionKind,
    /// Every one of these must appear (case-insensitively) in the prompt.
    pub keywords: Vec<String>,
    /// What the rule answers.
    pub answer: RuleAnswer,
    /// How sure the table is, in `[0,1]`.
    pub confidence: f32,
    /// Why the rule exists, for the operator reading the file.
    pub notes: Option<String>,
}

/// The whole table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleTable {
    /// The table's version, carried into the log so a decision can be attributed
    /// to the policy that produced it.
    pub version: String,
    /// The confidence a fallback answer carries. Low on purpose.
    pub default_confidence: f32,
    /// The rules, in file order. Rank order is computed, never read off the file.
    pub rules: Vec<Rule>,
}

/// The table compiled into the binary, so a kernel with no data directory can
/// still answer an authorization question.
pub const DEFAULT_RULES_TOML: &str = include_str!("../rules/decision_rules.toml");

impl RuleTable {
    /// Parse a table from TOML text.
    pub fn from_toml(text: &str) -> Result<Self> {
        toml::from_str::<RuleTable>(text)
            .map_err(|e| DecisionError::Codec(format!("decision_rules.toml: {e}")))
    }

    /// Read and parse a table from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| DecisionError::Codec(format!("{}: {e}", path.display())))?;
        Self::from_toml(&text)
    }

    /// The table the binary was built with.
    pub fn embedded() -> Result<Self> {
        Self::from_toml(DEFAULT_RULES_TOML)
    }

    /// Refuse a table that could answer the same question two ways.
    pub fn validate(&self) -> Result<()> {
        if self.version.trim().is_empty() {
            return Err(DecisionError::validation("version", "is empty"));
        }
        check_confidence("default_confidence", self.default_confidence)?;
        if self.rules.is_empty() {
            return Err(DecisionError::validation(
                "rules",
                "the table has no rules, so every question would fall back",
            ));
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for rule in &self.rules {
            if rule.id.trim().is_empty() {
                return Err(DecisionError::validation("rules.id", "is empty"));
            }
            if !seen.insert(rule.id.as_str()) {
                return Err(DecisionError::validation(
                    "rules.id",
                    format!("{:?} appears twice", rule.id),
                ));
            }
            check_confidence("rules.confidence", rule.confidence)?;
            if rule.keywords.is_empty() {
                return Err(DecisionError::validation(
                    "rules.keywords",
                    format!(
                        "{:?} matches on nothing, so it would fire on every prompt",
                        rule.id
                    ),
                ));
            }
            let mut keys: BTreeSet<&str> = BTreeSet::new();
            for keyword in &rule.keywords {
                if keyword.trim().is_empty() {
                    return Err(DecisionError::validation(
                        "rules.keywords",
                        format!("{:?} has an empty keyword", rule.id),
                    ));
                }
                if !keys.insert(keyword.as_str()) {
                    return Err(DecisionError::validation(
                        "rules.keywords",
                        format!("{:?} repeats the keyword {keyword:?}", rule.id),
                    ));
                }
            }
            let answer_kind = match &rule.answer {
                RuleAnswer::Bool { .. } => QuestionKind::YesNo,
                RuleAnswer::Score { .. } => QuestionKind::Score,
                RuleAnswer::Option { .. } | RuleAnswer::None => QuestionKind::Choice,
            };
            if answer_kind != rule.kind {
                return Err(DecisionError::validation(
                    "rules.answer",
                    format!(
                        "{:?} is a {} rule but answers a {}",
                        rule.id, rule.kind, answer_kind
                    ),
                ));
            }
            if let RuleAnswer::Score { value } = &rule.answer {
                if !value.is_finite() {
                    return Err(DecisionError::validation(
                        "rules.answer",
                        format!("{:?} answers a non-finite score", rule.id),
                    ));
                }
            }
        }
        Ok(())
    }

    /// The rules that apply to `question`, in file order.
    pub fn matching<'a>(&'a self, question: &DecisionQuestion) -> Vec<&'a Rule> {
        let haystack = question.prompt().to_lowercase();
        self.rules
            .iter()
            .filter(|rule| rule.kind == question.kind())
            .filter(|rule| {
                rule.keywords
                    .iter()
                    .all(|keyword| haystack.contains(&keyword.to_lowercase()))
            })
            .collect()
    }

    /// The rules that apply, highest priority first, ties broken by `id` ascending.
    ///
    /// The order is total: two rules can never be equally ranked, because their
    /// ids differ. That is what makes the core deterministic without a tie-break
    /// that depends on file order.
    pub fn ranked<'a>(&'a self, question: &DecisionQuestion) -> Vec<&'a Rule> {
        let mut out = self.matching(question);
        out.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// The single highest-ranked rule that applies, if any.
    ///
    /// This is the *ranking* winner, not necessarily the rule the core answers
    /// with: a higher-priority rule whose answer the question cannot admit is
    /// skipped by [`RulesCore::answer`], and `best` will name it anyway. Callers
    /// that need the answered rule should read the answer's features.
    pub fn best<'a>(&'a self, question: &DecisionQuestion) -> Option<&'a Rule> {
        self.ranked(question).into_iter().next()
    }
}

/// Refuse a confidence outside `[0,1]`.
fn check_confidence(field: &'static str, value: f32) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(DecisionError::validation(
            field,
            format!("must be in [0,1], got {value}"),
        ));
    }
    Ok(())
}

/// Turn a rule's answer into one the question admits, or `None` when it does not.
fn admissible(question: &DecisionQuestion, rule: &RuleAnswer) -> Option<AnswerValue> {
    match (question, rule) {
        (DecisionQuestion::Choice { options, .. }, RuleAnswer::Option { value }) => options
            .iter()
            .find(|offered| offered.as_str() == value.as_str())
            .map(|offered| AnswerValue::Option {
                value: offered.clone(),
            }),
        (DecisionQuestion::Choice { allow_none, .. }, RuleAnswer::None) if *allow_none => {
            Some(AnswerValue::None)
        }
        (DecisionQuestion::Score { scale, .. }, RuleAnswer::Score { value }) => {
            if value.is_finite() && *value >= scale.min && *value <= scale.max {
                Some(AnswerValue::Score { value: *value })
            } else {
                None
            }
        }
        (DecisionQuestion::YesNo { .. }, RuleAnswer::Bool { value }) => {
            Some(AnswerValue::Bool { value: *value })
        }
        _ => None,
    }
}

/// The answer a table gives when no rule fits.
fn fallback(question: &DecisionQuestion) -> Result<AnswerValue> {
    match question {
        DecisionQuestion::Choice { allow_none, .. } => {
            if *allow_none {
                Ok(AnswerValue::None)
            } else {
                Err(DecisionError::Unavailable(
                    "no rule matched and the question allows no default".to_string(),
                ))
            }
        }
        DecisionQuestion::Score { scale, .. } => Ok(AnswerValue::Score {
            value: scale.min + scale.width() / 2.0,
        }),
        DecisionQuestion::YesNo { .. } => Ok(AnswerValue::Bool { value: false }),
    }
}

/// The decision core backed by a [`RuleTable`].
pub struct RulesCore {
    table: Arc<RuleTable>,
}

impl RulesCore {
    /// Build a core over a table that has already passed validation.
    pub fn new(table: RuleTable) -> Result<Self> {
        table.validate()?;
        Ok(RulesCore {
            table: Arc::new(table),
        })
    }

    /// Build a core over the compiled-in table.
    pub fn embedded() -> Result<Self> {
        Self::new(RuleTable::embedded()?)
    }

    /// The table this core answers from.
    pub fn table(&self) -> &RuleTable {
        &self.table
    }
}

impl std::fmt::Debug for RulesCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RulesCore")
            .field("version", &self.table.version)
            .field("rules", &self.table.rules.len())
            .finish()
    }
}

#[async_trait]
impl DecisionCore for RulesCore {
    fn id(&self) -> CoreId {
        CoreId::Rules
    }

    async fn answer(
        &self,
        question: &DecisionQuestion,
        state: &DecisionState,
    ) -> Result<DecisionAnswer> {
        let ranked = self.table.ranked(question);

        // Walk the ranking and take the first rule the question can admit. A rule
        // that fires on the wrong shape is skipped rather than fatal: a table edit
        // must not be able to remove the core's ability to answer.
        let mut selected: Option<(&Rule, AnswerValue)> = None;
        for rule in &ranked {
            if let Some(answer) = admissible(question, &rule.answer) {
                selected = Some((rule, answer));
                break;
            }
        }

        let (answer, confidence, priority, matched) = match selected {
            Some((rule, answer)) => (answer, rule.confidence, rule.priority, true),
            None => (fallback(question)?, self.table.default_confidence, 0, false),
        };

        // Features are what Phase 11 learns from, so they describe *why* this
        // answer was given: how many rules applied, whether one fired at all,
        // which one won, and how much evidence the state carried.
        let features = FeatureVector::new()
            .with("rules.matched", ranked.len() as f64)
            .with("rules.fired", if matched { 1.0 } else { 0.0 })
            .with("rules.priority", priority as f64)
            .with("rules.declared_confidence", f64::from(confidence))
            .with("state.facts", state.facts.len() as f64)
            .with("state.assumptions", state.assumptions.len() as f64)
            .with("state.constraints", state.constraints.len() as f64);

        // `latency_ms` is zero, not measured. A real duration would make the answer
        // differ between two runs of the same question, and the conformance suite's
        // determinism check compares answers byte for byte.
        DecisionAnswer::new(question, answer, confidence, features, CoreId::Rules, 0)
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

    fn state() -> DecisionState {
        DecisionState::with_refs(
            ulid(9),
            vec![ulid(10)],
            vec![ulid(11), ulid(12)],
            vec!["no-network".into()],
            serde_json::json!({"stakes": 0.8}),
        )
    }

    fn table(spec: &str) -> RuleTable {
        RuleTable::from_toml(spec).expect("table parses")
    }

    #[test]
    fn the_embedded_table_parses_and_validates() {
        let table = RuleTable::embedded().expect("embedded table parses");
        table.validate().expect("embedded table validates");
        assert!(
            table.rules.len() >= 20,
            "the compiled-in table must cover the firewall's judgments, got {}",
            table.rules.len()
        );
        assert_eq!(table.version, "1");
        // Every rule in the shipped table must answer a shape its own kind declares.
        for rule in &table.rules {
            assert!(!rule.keywords.is_empty(), "{} has no keywords", rule.id);
        }
    }

    #[test]
    fn a_duplicate_rule_id_is_refused() {
        let spec = r#"
version = "1"
default_confidence = 0.2

[[rules]]
id = "a"
priority = 1
kind = "yes_no"
keywords = ["x"]
answer = { kind = "bool", value = true }
confidence = 0.5

[[rules]]
id = "a"
priority = 2
kind = "yes_no"
keywords = ["y"]
answer = { kind = "bool", value = false }
confidence = 0.5
"#;
        let err = table(spec).validate().unwrap_err();
        assert_eq!(err.code(), "validation");
        assert!(err.to_string().contains("\"a\""), "{err}");
    }

    #[test]
    fn an_answer_of_the_wrong_shape_is_refused() {
        let spec = r#"
version = "1"
default_confidence = 0.2

[[rules]]
id = "mismatched"
priority = 1
kind = "yes_no"
keywords = ["x"]
answer = { kind = "score", value = 0.5 }
confidence = 0.5
"#;
        assert!(table(spec).validate().is_err());
    }

    #[test]
    fn a_rule_with_no_keywords_is_refused() {
        let spec = r#"
version = "1"
default_confidence = 0.2

[[rules]]
id = "matches-everything"
priority = 1
kind = "yes_no"
keywords = []
answer = { kind = "bool", value = true }
confidence = 0.5
"#;
        assert!(table(spec).validate().is_err());
    }

    #[test]
    fn ties_break_by_id_ascending() {
        let spec = r#"
version = "1"
default_confidence = 0.2

[[rules]]
id = "zebra"
priority = 7
kind = "yes_no"
keywords = ["tie"]
answer = { kind = "bool", value = false }
confidence = 0.3

[[rules]]
id = "alpha"
priority = 7
kind = "yes_no"
keywords = ["tie"]
answer = { kind = "bool", value = true }
confidence = 0.7
"#;
        let table = table(spec);
        let ranked = table.ranked(&yes_no("a tie"));
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].id, "alpha");
        assert_eq!(ranked[1].id, "zebra");
        assert_eq!(table.best(&yes_no("a tie")).unwrap().id, "alpha");
    }

    #[test]
    fn matching_is_a_case_insensitive_conjunction_on_the_kind() {
        let table = RuleTable::embedded().unwrap();
        let hit = table.matching(&yes_no(
            "Is this action IRREVERSIBLE with no APPROVAL record?",
        ));
        assert!(hit
            .iter()
            .any(|r| r.id == "safety.irreversible_without_approval"));

        // Subject alone is not enough: the negative marker has to be there too.
        let subject_only = table.matching(&yes_no("Is this action irreversible?"));
        assert!(!subject_only
            .iter()
            .any(|r| r.id == "safety.irreversible_without_approval"));

        // A yes_no rule can never match a score question.
        let score = DecisionQuestion::Score {
            id: ulid(2),
            prompt: "Is this action irreversible with no approval record?".into(),
            scale: ScoreScale::unit(),
        };
        assert!(!table
            .matching(&score)
            .iter()
            .any(|r| r.id == "safety.irreversible_without_approval"));
    }

    #[tokio::test]
    async fn the_rules_core_answers_a_matching_rule_with_its_own_confidence() {
        let core = RulesCore::embedded().unwrap();
        assert_eq!(core.id(), CoreId::Rules);
        let question = yes_no("Is this action irreversible with no approval record?");
        let answer = core.answer(&question, &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::Bool { value: true });
        assert_eq!(answer.confidence, 0.9);
        assert_eq!(answer.core, CoreId::Rules);
        assert_eq!(answer.latency_ms, 0);
        assert_eq!(answer.features.get("rules.matched"), Some(1.0));
        assert_eq!(answer.features.get("rules.priority"), Some(100.0));
        assert_eq!(answer.features.get("state.facts"), Some(1.0));
        assert_eq!(answer.features.get("state.constraints"), Some(1.0));
    }

    #[tokio::test]
    async fn a_fallback_is_low_confidence_and_shape_appropriate() {
        let core = RulesCore::embedded().unwrap();

        let yn = yes_no("Something no rule mentions at all");
        let answer = core.answer(&yn, &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::Bool { value: false });
        assert_eq!(answer.confidence, core.table().default_confidence);
        assert_eq!(answer.features.get("rules.matched"), Some(0.0));

        let score = DecisionQuestion::Score {
            id: ulid(3),
            prompt: "Something no rule mentions at all".into(),
            scale: ScoreScale::new(2.0, 4.0),
        };
        let answer = core.answer(&score, &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::Score { value: 3.0 });
        assert_eq!(answer.confidence, core.table().default_confidence);
    }

    #[tokio::test]
    async fn a_choice_that_forbids_none_is_refused_when_no_rule_fits() {
        let core = RulesCore::embedded().unwrap();
        let question = DecisionQuestion::Choice {
            id: ulid(4),
            prompt: "Unrecognised choice".into(),
            options: vec!["alpha".into(), "beta".into()],
            allow_none: false,
        };
        let err = core.answer(&question, &state()).await.unwrap_err();
        assert_eq!(err.code(), "unavailable");
        assert!(err.is_unavailable());
    }

    #[tokio::test]
    async fn a_choice_that_allows_none_answers_none_at_the_default_confidence() {
        let core = RulesCore::embedded().unwrap();
        let question = DecisionQuestion::Choice {
            id: ulid(5),
            prompt: "Unrecognised choice".into(),
            options: vec!["alpha".into(), "beta".into()],
            allow_none: true,
        };
        let answer = core.answer(&question, &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::None);
        assert_eq!(answer.confidence, core.table().default_confidence);
    }

    #[tokio::test]
    async fn a_choice_rule_is_skipped_when_its_option_is_not_offered() {
        let spec = r#"
version = "1"
default_confidence = 0.2

[[rules]]
id = "high"
priority = 10
kind = "choice"
keywords = ["route"]
answer = { kind = "option", value = "not on the menu" }
confidence = 0.9

[[rules]]
id = "low"
priority = 1
kind = "choice"
keywords = ["route"]
answer = { kind = "option", value = "beta" }
confidence = 0.4
"#;
        let core = RulesCore::new(table(spec)).unwrap();
        let question = DecisionQuestion::Choice {
            id: ulid(6),
            prompt: "Which route?".into(),
            options: vec!["alpha".into(), "beta".into()],
            allow_none: true,
        };
        let answer = core.answer(&question, &state()).await.unwrap();
        assert_eq!(
            answer.answer,
            AnswerValue::Option {
                value: "beta".into()
            }
        );
        assert_eq!(answer.confidence, 0.4);
        assert_eq!(answer.features.get("rules.matched"), Some(2.0));
    }

    #[tokio::test]
    async fn the_same_question_and_state_answer_identically() {
        let core = RulesCore::embedded().unwrap();
        let question = yes_no("Is this action irreversible with no approval record?");
        let first = core.answer(&question, &state()).await.unwrap();
        let second = core.answer(&question, &state()).await.unwrap();
        assert_eq!(first.canonical(), second.canonical());
        assert_eq!(first.content_hash(), second.content_hash());
    }

    #[test]
    fn every_answer_shape_round_trips_through_toml() {
        // The four variants are written as inline tables in the shipped file, so a
        // parser that handled only some of them would make part of the policy
        // unreadable. One table exercises all four.
        let spec = r#"
version = "1"
default_confidence = 0.2

[[rules]]
id = "a.bool"
priority = 4
kind = "yes_no"
keywords = ["bool"]
answer = { kind = "bool", value = true }
confidence = 0.5

[[rules]]
id = "a.score"
priority = 3
kind = "score"
keywords = ["score"]
answer = { kind = "score", value = 0.25 }
confidence = 0.5

[[rules]]
id = "a.option"
priority = 2
kind = "choice"
keywords = ["option"]
answer = { kind = "option", value = "beta" }
confidence = 0.5

[[rules]]
id = "a.none"
priority = 1
kind = "choice"
keywords = ["none"]
answer = { kind = "none" }
confidence = 0.5
"#;
        let table = table(spec);
        table.validate().expect("all four shapes validate");
        assert_eq!(table.rules[0].answer, RuleAnswer::Bool { value: true });
        assert_eq!(table.rules[1].answer, RuleAnswer::Score { value: 0.25 });
        assert_eq!(
            table.rules[2].answer,
            RuleAnswer::Option {
                value: "beta".into()
            }
        );
        assert_eq!(table.rules[3].answer, RuleAnswer::None);
    }

    #[tokio::test]
    async fn a_none_rule_declines_an_answer_the_question_allows() {
        let spec = r#"
version = "1"
default_confidence = 0.2

[[rules]]
id = "decline"
priority = 5
kind = "choice"
keywords = ["unsure"]
answer = { kind = "none" }
confidence = 0.3
"#;
        let core = RulesCore::new(table(spec)).unwrap();
        let question = DecisionQuestion::Choice {
            id: ulid(7),
            prompt: "Which one, if any? Unsure.".into(),
            options: vec!["alpha".into(), "beta".into()],
            allow_none: true,
        };
        let answer = core.answer(&question, &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::None);
        assert_eq!(answer.confidence, 0.3);
    }

    #[test]
    fn a_table_that_is_not_toml_is_a_codec_refusal() {
        let err = RuleTable::from_toml("version = ").unwrap_err();
        assert_eq!(err.code(), "codec");
    }
}
