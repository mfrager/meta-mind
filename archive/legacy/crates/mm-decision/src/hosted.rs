//! The hosted decision core: one strict structured call, and nothing else.
//!
//! This is the core for novel or unbounded bounded questions — the ones the table
//! has no row for and no head was distilled for. It programs against the substrate
//! ([`LlmClient`]) rather than against a provider, and it mirrors the metacognitive
//! scan's discipline exactly: one request, one strict schema, validate the text
//! against the schema, decode it, and *then* check that the answer actually answers
//! the question it was asked.
//!
//! Two refusals are kept apart, because they mean different things to an operator:
//!
//! * A provider that cannot be reached, or a schema that was never registered, is
//!   [`DecisionError::Unavailable`] — the core has no answer here, and the firewall
//!   escalates. This is the plan's §9 mitigation: a hosted decision model that is
//!   unavailable must not be papered over with a fabricated decision.
//! * A reply that arrived and did not satisfy the schema, or that named an option
//!   the question never offered, is [`DecisionError::Schema`] — the model answered
//!   badly, which is a defect to be measured rather than a state to degrade from.
//!   Nothing is coerced: a well-formed-looking answer that violates its schema is
//!   worse than no answer, because nothing downstream can tell the two apart.
//!
//! The reply is deliberately *flat* rather than a tagged union. A strict schema in
//! this substrate may not use `oneOf`/`anyOf` or a nullable `type` array — an
//! unsupported construct is a hard error, never a looser check — so the reply
//! carries `answer_kind` plus all three payload fields, and [`HostedDecisionReply::validate_for`]
//! refuses any field the chosen kind did not use.

use std::sync::Arc;

use async_trait::async_trait;
use mm_llm::{
    CacheMode, LlmClient, LlmError, LlmRequest, Message, Purpose, SchemaError, SchemaId,
    SchemaRegistry, StructuredOut,
};
use serde::{Deserialize, Serialize};

use crate::core::{CoreId, DecisionCore};
use crate::error::{DecisionError, Result};
use crate::question::{
    AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState, FeatureVector, QuestionKind,
};

/// The schema every hosted decision reply is validated against.
pub const HOSTED_DECISION_SCHEMA_ID: &str = "mm.decision.answer.v1";

/// The instructions that frame the call. One prompt, one answer, no repair loop.
pub const HOSTED_DECISION_SYSTEM_PROMPT: &str = "\
You are the bounded-decision core of a Metamind. You are given exactly one \
bounded question and the state it is asked against, and you must answer that \
question and nothing else. Reply with JSON only. Set `answer_kind` to `option` \
for a choice, `score` for a scale, `bool` for a yes/no, and `none` when the \
question allows no answer; fill only the payload field your kind uses and leave \
the others at their empty defaults. Set `confidence` to your own probability that \
the answer is correct, in [0,1] — do not inflate it. Use `rationale` for one short \
sentence. If the state does not settle the question, answer `none` rather than \
guessing.";

/// One hosted reply, exactly as the schema constrains it.
///
/// Every field is non-nullable so the schema stays inside the strict subset, which
/// is why the unused payload fields carry empty defaults rather than `null`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostedDecisionReply {
    /// Which payload field carries the answer: `option`, `score`, `bool` or `none`.
    pub answer_kind: String,
    /// The chosen option, when `answer_kind` is `option`; `"none"` for no answer.
    pub option: String,
    /// A normalized score in `[0,1]`, mapped into the question's scale on the way
    /// out. The model never sees the scale's raw numbers, so a scale change cannot
    /// change what the model was asked.
    pub score: f32,
    /// The answer when `answer_kind` is `bool`.
    pub yes: bool,
    /// The model's own probability that the answer is correct, in `[0,1]`.
    pub confidence: f32,
    /// One sentence of justification, for the audit.
    pub rationale: String,
}

impl HostedDecisionReply {
    /// Map the reply onto the question it was asked, refusing anything the question
    /// did not admit.
    ///
    /// The option comparison is case-insensitive and returns the *question's*
    /// spelling, so a model that answers `Roll Back` still names the offered option
    /// in the record.
    pub fn validate_for(&self, question: &DecisionQuestion) -> Result<AnswerValue> {
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(DecisionError::Schema(format!(
                "confidence {} is outside [0,1]",
                self.confidence
            )));
        }
        let kind = self.answer_kind.trim().to_ascii_lowercase();
        match (question, kind.as_str()) {
            (
                DecisionQuestion::Choice {
                    options,
                    allow_none,
                    ..
                },
                "option",
            ) => {
                let value = self.option.trim();
                if value.eq_ignore_ascii_case("none") {
                    if *allow_none {
                        return Ok(AnswerValue::None);
                    }
                    return Err(DecisionError::Schema(
                        "the question does not allow an empty answer".to_string(),
                    ));
                }
                options
                    .iter()
                    .find(|offered| offered.eq_ignore_ascii_case(value))
                    .map(|offered| AnswerValue::Option {
                        value: offered.clone(),
                    })
                    .ok_or_else(|| {
                        DecisionError::Schema(format!("{value:?} was not offered by the question"))
                    })
            }
            (DecisionQuestion::Choice { allow_none, .. }, "none") => {
                if *allow_none {
                    Ok(AnswerValue::None)
                } else {
                    Err(DecisionError::Schema(
                        "the question does not allow an empty answer".to_string(),
                    ))
                }
            }
            (DecisionQuestion::Score { scale, .. }, "score") => {
                if !self.score.is_finite() || !(0.0..=1.0).contains(&self.score) {
                    return Err(DecisionError::Schema(format!(
                        "score {} is outside [0,1]",
                        self.score
                    )));
                }
                // The clamp repairs float rounding at the top of the scale, which is
                // arithmetic and not a semantic coercion: the value is already inside
                // [0,1] by the check above, and `min + width` need not equal `max` in
                // binary floating point.
                let value = (scale.min + self.score * scale.width()).clamp(scale.min, scale.max);
                Ok(AnswerValue::Score { value })
            }
            (DecisionQuestion::YesNo { .. }, "bool") => Ok(AnswerValue::Bool { value: self.yes }),
            (question, kind) => Err(DecisionError::Schema(format!(
                "a {} question cannot be answered with kind {kind:?}",
                question.kind()
            ))),
        }
    }

    /// The one line of justification, or `None` when the model gave none.
    pub fn rationale(&self) -> Option<&str> {
        let trimmed = self.rationale.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }
}

impl StructuredOut for HostedDecisionReply {
    fn schema_id() -> SchemaId {
        SchemaId::new(HOSTED_DECISION_SCHEMA_ID)
    }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": [
                "answer_kind",
                "option",
                "score",
                "yes",
                "confidence",
                "rationale"
            ],
            "properties": {
                "answer_kind": {
                    "type": "string",
                    "enum": ["option", "score", "bool", "none"]
                },
                "option": { "type": "string" },
                "score": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                "yes": { "type": "boolean" },
                "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                "rationale": { "type": "string" }
            }
        })
    }
}

/// Register the hosted decision schema in a registry.
pub fn register(registry: &mut SchemaRegistry) -> std::result::Result<(), LlmError> {
    mm_llm::schema::register_structured::<HostedDecisionReply>(registry)
}

/// The decision core backed by the substrate's structured output.
pub struct HostedCore<C: LlmClient> {
    client: Arc<C>,
    registry: Arc<SchemaRegistry>,
    max_tokens: u32,
}

impl<C: LlmClient> Clone for HostedCore<C> {
    fn clone(&self) -> Self {
        HostedCore {
            client: Arc::clone(&self.client),
            registry: Arc::clone(&self.registry),
            max_tokens: self.max_tokens,
        }
    }
}

impl<C: LlmClient> HostedCore<C> {
    /// Build a core. The registry must already have the reply schema registered —
    /// [`register`] does that.
    pub fn new(client: Arc<C>, registry: Arc<SchemaRegistry>, max_tokens: u32) -> Self {
        HostedCore {
            client,
            registry,
            max_tokens,
        }
    }

    /// Turn a schema rejection into this crate's refusal, keeping the reason.
    ///
    /// A rejection is `Schema` rather than `Unavailable`: the call happened and the
    /// answer was wrong, and conflating the two would hide a model defect behind a
    /// state the firewall is designed to degrade from.
    pub fn wrap(error: SchemaError) -> DecisionError {
        DecisionError::Schema(error.to_string())
    }

    /// The user turn: the question rendered deterministically, then the state.
    pub fn prompt(question: &DecisionQuestion, state: &DecisionState) -> String {
        let mut out = format!(
            "Question kind: {}\nQuestion: {}\n",
            question.kind(),
            question.prompt()
        );
        match question {
            DecisionQuestion::Choice {
                options,
                allow_none,
                ..
            } => {
                out.push_str(&format!("Options: {}\n", options.join(" | ")));
                out.push_str(&format!("May answer none: {allow_none}\n"));
            }
            DecisionQuestion::Score { scale, .. } => {
                out.push_str(&format!("Scale: {}..{}\n", scale.min, scale.max));
                for anchor in &scale.anchors {
                    out.push_str(&format!("  {} = {}\n", anchor.at, anchor.label));
                }
            }
            DecisionQuestion::YesNo { .. } => {}
        }
        out.push_str("State:\n");
        out.push_str(&state.canonical());
        out
    }
}

#[async_trait]
impl<C: LlmClient + 'static> DecisionCore for HostedCore<C> {
    fn id(&self) -> CoreId {
        CoreId::Hosted
    }

    async fn answer(
        &self,
        question: &DecisionQuestion,
        state: &DecisionState,
    ) -> Result<DecisionAnswer> {
        question.validate()?;

        let schema_id = SchemaId::new(HOSTED_DECISION_SCHEMA_ID);
        // Check registration *before* the call: a call whose output cannot be
        // checked is a call whose output must not be used, and spending the request
        // first would leave the ledger with a decision nobody can validate.
        if self.registry.get(&schema_id).is_none() {
            return Err(DecisionError::Unavailable(format!(
                "schema `{HOSTED_DECISION_SCHEMA_ID}` is not registered; \
                 refusing to answer a decision that cannot be validated"
            )));
        }

        let mut request = LlmRequest::new(
            Purpose::Plan,
            vec![
                Message::system(HOSTED_DECISION_SYSTEM_PROMPT),
                Message::user(Self::prompt(question, state)),
            ],
        )
        .with_schema(schema_id.clone())
        .with_cache(CacheMode::Use);
        request.max_tokens = self.max_tokens;

        let response = self.client.complete(&request).await.map_err(|e| {
            // Unreachable provider, timeout, budget: the core has no answer here.
            DecisionError::Unavailable(format!("hosted decision call failed: {e}"))
        })?;

        let value = self
            .registry
            .validate(&schema_id, &response.text)
            .map_err(Self::wrap)?;
        let reply: HostedDecisionReply = mm_llm::schema::decode(value).map_err(Self::wrap)?;
        let answer = reply.validate_for(question)?;

        let features = FeatureVector::new()
            .with("hosted.tokens_in", f64::from(response.usage.tokens_in))
            .with("hosted.tokens_out", f64::from(response.usage.tokens_out))
            .with("hosted.cost_micros", response.usage.cost_micros as f64)
            .with("hosted.registered_schema", 1.0);

        // `latency_ms` is zero, not the provider's reported latency. The conformance
        // suite compares two answers to the same question byte for byte, and a
        // measured duration would make the hosted core fail a check the other two
        // pass for a reason that has nothing to do with the answer.
        DecisionAnswer::new(
            question,
            answer,
            reply.confidence,
            features,
            CoreId::Hosted,
            0,
        )
    }
}

/// Whether a reply is well-formed for `question`, for the adversarial corpus.
pub fn reply_is_admissible(reply: &HostedDecisionReply, question: &DecisionQuestion) -> bool {
    reply.validate_for(question).is_ok()
}

/// The question shape a reply claims to answer, as a [`QuestionKind`], so a
/// mismatch can be reported without a second parse.
pub fn claimed_kind(reply: &HostedDecisionReply) -> Option<QuestionKind> {
    match reply.answer_kind.trim().to_ascii_lowercase().as_str() {
        "option" | "none" => Some(QuestionKind::Choice),
        "score" => Some(QuestionKind::Score),
        "bool" => Some(QuestionKind::YesNo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question::ScoreScale;
    use mm_core::Ulid;
    use mm_llm::mock::{MockClient, ScriptedResponse};

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn yes_no() -> DecisionQuestion {
        DecisionQuestion::YesNo {
            id: ulid(1),
            prompt: "Is this action authorized?".into(),
        }
    }

    fn score() -> DecisionQuestion {
        DecisionQuestion::Score {
            id: ulid(2),
            prompt: "How much residual risk is there?".into(),
            scale: ScoreScale::new(0.0, 4.0),
        }
    }

    fn choice() -> DecisionQuestion {
        DecisionQuestion::Choice {
            id: ulid(3),
            prompt: "Which mitigation?".into(),
            options: vec!["roll back".into(), "fail over".into()],
            allow_none: true,
        }
    }

    fn state() -> DecisionState {
        DecisionState::new(ulid(9), serde_json::json!({"stakes": 0.9}))
    }

    fn registered() -> Arc<SchemaRegistry> {
        let mut registry = SchemaRegistry::new();
        register(&mut registry).expect("the reply schema is strict");
        Arc::new(registry)
    }

    fn core_for(registry: Arc<SchemaRegistry>, text: &str) -> HostedCore<MockClient> {
        HostedCore::new(
            Arc::new(MockClient::script(vec![ScriptedResponse::text(text)])),
            registry,
            512,
        )
    }

    fn reply(kind: &str) -> String {
        format!(
            r#"{{"answer_kind":"{kind}","option":"","score":0.0,"yes":false,"confidence":0.0,"rationale":""}}"#
        )
    }

    #[test]
    fn the_reply_schema_is_strict_enough_to_register() {
        let mut registry = SchemaRegistry::new();
        register(&mut registry).expect("registration must succeed");
        assert!(registry
            .get(&SchemaId::new(HOSTED_DECISION_SCHEMA_ID))
            .is_some());
        // Registering twice is refused rather than silently replacing the schema.
        assert!(register(&mut registry).is_err());
    }

    #[tokio::test]
    async fn a_well_formed_reply_becomes_a_bounded_answer() {
        let text = r#"{"answer_kind":"bool","option":"","score":0.0,"yes":true,"confidence":0.72,"rationale":"a grant matches"}"#;
        let core = core_for(registered(), text);
        assert_eq!(core.id(), CoreId::Hosted);
        let answer = core.answer(&yes_no(), &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::Bool { value: true });
        assert!((answer.confidence - 0.72).abs() < 1e-6);
        assert_eq!(answer.core, CoreId::Hosted);
        assert_eq!(answer.latency_ms, 0);
        assert_eq!(answer.features.get("hosted.registered_schema"), Some(1.0));
        assert!(answer.features.get("hosted.tokens_in").is_some());
    }

    #[tokio::test]
    async fn a_reply_of_the_wrong_kind_is_a_schema_refusal() {
        let text = reply("bool");
        let core = core_for(registered(), &text);
        // The question is a choice; a bool reply is not an answer to it.
        let err = core.answer(&choice(), &state()).await.unwrap_err();
        assert_eq!(err.code(), "schema");
        assert!(!err.is_unavailable());
    }

    #[tokio::test]
    async fn an_option_that_was_never_offered_is_a_schema_refusal() {
        let text = r#"{"answer_kind":"option","option":"rewrite it","score":0.0,"yes":false,"confidence":0.5,"rationale":""}"#;
        let core = core_for(registered(), text);
        let err = core.answer(&choice(), &state()).await.unwrap_err();
        assert_eq!(err.code(), "schema");
        assert!(err.to_string().contains("rewrite it"), "{err}");

        // The same reply against a question that does offer it is accepted, and the
        // question's own spelling is what the answer carries.
        let text = r#"{"answer_kind":"option","option":"Roll Back","score":0.0,"yes":false,"confidence":0.5,"rationale":""}"#;
        let core = core_for(registered(), text);
        let answer = core.answer(&choice(), &state()).await.unwrap();
        assert_eq!(
            answer.answer,
            AnswerValue::Option {
                value: "roll back".into()
            }
        );
    }

    #[tokio::test]
    async fn a_score_is_mapped_into_the_questions_scale() {
        let text = r#"{"answer_kind":"score","option":"","score":0.25,"yes":false,"confidence":0.6,"rationale":""}"#;
        let core = core_for(registered(), text);
        let answer = core.answer(&score(), &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::Score { value: 1.0 });

        // The top of the normalized scale lands exactly on the scale's maximum even
        // when the maximum is not exactly representable as `min + width`.
        let text = r#"{"answer_kind":"score","option":"","score":1.0,"yes":false,"confidence":0.6,"rationale":""}"#;
        let core = core_for(registered(), text);
        let odd = DecisionQuestion::Score {
            id: ulid(4),
            prompt: "How risky?".into(),
            scale: ScoreScale::new(0.1, 0.9),
        };
        let answer = core.answer(&odd, &state()).await.unwrap();
        assert_eq!(answer.answer, AnswerValue::Score { value: 0.9 });
    }

    #[tokio::test]
    async fn an_unregistered_schema_refuses_rather_than_answering_unvalidated() {
        let core = core_for(Arc::new(SchemaRegistry::new()), &reply("bool"));
        let err = core.answer(&yes_no(), &state()).await.unwrap_err();
        assert_eq!(err.code(), "unavailable");
        assert!(err.to_string().contains("not registered"), "{err}");
    }

    #[tokio::test]
    async fn a_provider_that_cannot_be_reached_is_unavailable_not_wrong() {
        let client = Arc::new(MockClient::script(vec![ScriptedResponse::failing(
            "connection reset",
        )]));
        let core = HostedCore::new(client, registered(), 512);
        let err = core.answer(&yes_no(), &state()).await.unwrap_err();
        assert_eq!(err.code(), "unavailable");
        assert!(err.to_string().contains("connection reset"), "{err}");
    }

    #[tokio::test]
    async fn text_that_violates_the_schema_is_rejected_not_repaired() {
        // A missing required field and an out-of-range confidence, in one reply.
        let text = r#"{"answer_kind":"bool","yes":true,"confidence":7.5}"#;
        let core = core_for(registered(), text);
        let err = core.answer(&yes_no(), &state()).await.unwrap_err();
        assert_eq!(err.code(), "schema");

        let text = r#"{"answer_kind":"bool","option":"","score":0.0,"yes":true,"confidence":0.5,"rationale":"","extra":1}"#;
        let core = core_for(registered(), text);
        assert_eq!(
            core.answer(&yes_no(), &state()).await.unwrap_err().code(),
            "schema"
        );
    }

    #[tokio::test]
    async fn a_confidence_outside_the_unit_interval_is_refused_after_decode() {
        // The schema bounds `confidence`, but a caller can construct a reply without
        // going through the decoder, so `validate_for` checks it again.
        let reply = HostedDecisionReply {
            answer_kind: "bool".into(),
            option: String::new(),
            score: 0.0,
            yes: true,
            confidence: 2.0,
            rationale: String::new(),
        };
        assert!(reply.validate_for(&yes_no()).is_err());
    }

    #[test]
    fn a_reply_with_no_answer_names_the_kind_it_claimed() {
        let reply = HostedDecisionReply {
            answer_kind: "option".into(),
            option: "roll back".into(),
            score: 0.0,
            yes: false,
            confidence: 0.4,
            rationale: "  ".into(),
        };
        assert_eq!(claimed_kind(&reply), Some(QuestionKind::Choice));
        assert!(reply.rationale().is_none());
        assert!(reply_is_admissible(&reply, &choice()));
        assert!(!reply_is_admissible(&reply, &yes_no()));
    }

    #[test]
    fn the_prompt_states_the_kind_the_options_and_the_state() {
        let rendered = HostedCore::<MockClient>::prompt(&choice(), &state());
        assert!(rendered.contains("Question kind: choice"), "{rendered}");
        assert!(rendered.contains("roll back | fail over"), "{rendered}");
        assert!(rendered.contains("stakes"), "{rendered}");
    }
}
