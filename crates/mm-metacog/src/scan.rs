//! One broad metacognitive scan.
//!
//! The phase's invariant 2 is "one scan, many issues": the controller asks a
//! single structured question — *what is wrong with this provisional model, and
//! what does it not settle?* — rather than running a set of independent per-check
//! prompts. A per-check design would make the controller a checklist, which is
//! exactly the degeneracy the phase's risk table names.
//!
//! The scan is a strict [`StructuredOut`] call, so a malformed or over-broad
//! answer is *rejected*, never coerced: a plausible-looking scan that violated its
//! schema would compile into a program nobody can replay. [`ScanResult::validate`]
//! re-checks the same rules the schema states, so the rejection is the same
//! whether it happened at decode or at ingest.
//!
//! The bounds are part of the contract, not a defensive afterthought. A scan that
//! returns a hundred issues has not found a hundred problems; it has failed to
//! prioritise, and the controller would compile a program it cannot afford.

use std::sync::Arc;

use async_trait::async_trait;
use mm_llm::{
    CacheMode, LlmClient, LlmRequest, Message, Purpose, SchemaError, SchemaId, SchemaRegistry,
    StructuredOut,
};
use serde::{Deserialize, Serialize};

use crate::episode::{Context, EpisodeId};
use crate::error::{MetacogError, Result};

/// The most issues one scan may report.
pub const MAX_ISSUES: usize = 12;
/// The most uncertainties one scan may report.
pub const MAX_UNCERTAINTIES: usize = 12;
/// The most comparison requirements one scan may report.
pub const MAX_COMPARISONS: usize = 8;
/// The most failure modes one scan may report.
pub const MAX_FAILURE_MODES: usize = 12;
/// The most simpler alternatives one scan may offer.
pub const MAX_SIMPLER_ALTERNATIVES: usize = 8;

/// What the scan is asked about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanRequest {
    /// The episode being scanned.
    pub episode: EpisodeId,
    /// The statement of what is being decided.
    pub goal: String,
    /// The provisional model, so the scan can say what it leaves out.
    pub context: Context,
}

impl ScanRequest {
    /// Build a request from the episode's own statement.
    pub fn new(episode: EpisodeId, goal: impl Into<String>, context: Context) -> Self {
        ScanRequest {
            episode,
            goal: goal.into(),
            context,
        }
    }

    /// The user turn sent to the model. Deterministic: the same request renders
    /// the same text, which is what makes the scan cacheable and replayable.
    pub fn prompt(&self) -> String {
        format!(
            "Goal: {}\nNovelty: {:.3}\nTime pressure: {:.3}\n",
            self.goal, self.context.novelty, self.context.time_pressure
        )
    }
}

/// One thing the scan found wrong or unsettled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanIssue {
    /// The kind of issue, e.g. `ambiguous`, `missing_evidence`, `conflict`.
    pub kind: String,
    /// How much it matters, in `[0,1]`.
    pub materiality: f64,
    /// What it is about: a candidate, a claim, or the goal.
    pub target: String,
}

/// Something the scan cannot settle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Uncertainty {
    /// A stable name for this uncertainty.
    pub id: String,
    /// What is uncertain.
    pub description: String,
    /// How much it matters, in `[0,1]`.
    pub magnitude: f64,
}

/// A comparison the answer will need.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonSpec {
    /// The first thing.
    pub subject: String,
    /// The second thing.
    pub versus: String,
    /// What they are compared on.
    pub criterion: String,
}

/// A way the answer could fail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FailureMode {
    /// What could go wrong.
    pub mode: String,
    /// How likely it is, in `[0,1]`, as the caller's belief.
    pub likelihood: f64,
    /// How bad it would be, in `[0,1]`.
    pub impact: f64,
}

/// The result of the one broad scan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanResult {
    /// What is wrong with the provisional model.
    pub issues: Vec<ScanIssue>,
    /// What the model does not settle.
    pub uncertainties: Vec<Uncertainty>,
    /// Comparisons the answer will need.
    pub comparison_requirements: Vec<ComparisonSpec>,
    /// Ways the answer could fail.
    pub failure_modes: Vec<FailureMode>,
    /// Simpler answers the scan thinks would do.
    pub simpler_alternatives: Vec<String>,
    /// What is at stake, in `[0,1]`.
    pub stakes: f64,
    /// How hard the decision is to undo, in `[0,1]`.
    pub irreversibility: f64,
    /// How much establishing the truth is worth here, in `[0,1]`.
    pub verification_value: f64,
}

impl ScanResult {
    /// The mean magnitude of the reported uncertainties, `0` when there are none.
    pub fn mean_uncertainty(&self) -> f64 {
        if self.uncertainties.is_empty() {
            return 0.0;
        }
        let total: f64 = self.uncertainties.iter().map(|u| u.magnitude).sum();
        total / self.uncertainties.len() as f64
    }

    /// The issues, materiality first, ties broken by target.
    ///
    /// The scan's own order is not trusted: a model's ordering is not a ranking
    /// anybody can reproduce, so the controller imposes one.
    pub fn issues_by_materiality(&self) -> Vec<&ScanIssue> {
        let mut out: Vec<&ScanIssue> = self.issues.iter().collect();
        out.sort_by(|a, b| {
            b.materiality
                .partial_cmp(&a.materiality)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.target.cmp(&b.target))
                .then_with(|| a.kind.cmp(&b.kind))
        });
        out
    }

    /// The highest stakes any failure mode implies, `0` when none are reported.
    pub fn worst_failure_impact(&self) -> f64 {
        self.failure_modes
            .iter()
            .map(|f| f.impact)
            .fold(0.0_f64, f64::max)
    }

    /// Refuse a scan that cannot be compiled, naming the field that failed.
    ///
    /// This is the same rule the strict schema states, applied after decode as
    /// well: a value that arrived through a decoder other than the schema one must
    /// still be refused, or the schema would be the only thing standing between a
    /// bad scan and a program.
    pub fn validate(&self) -> Result<()> {
        check_len("issues", self.issues.len(), MAX_ISSUES)?;
        check_len(
            "comparison_requirements",
            self.comparison_requirements.len(),
            MAX_COMPARISONS,
        )?;
        check_len("failure_modes", self.failure_modes.len(), MAX_FAILURE_MODES)?;
        check_len(
            "simpler_alternatives",
            self.simpler_alternatives.len(),
            MAX_SIMPLER_ALTERNATIVES,
        )?;
        // Uncertainties are checked both ways: a scan that bails out with a
        // thousand "I don't know"s has not scanned anything.
        check_len("uncertainties", self.uncertainties.len(), MAX_UNCERTAINTIES)?;

        for (index, issue) in self.issues.iter().enumerate() {
            non_empty(&format!("issues[{index}].kind"), &issue.kind)?;
            non_empty(&format!("issues[{index}].target"), &issue.target)?;
            check_unit_at("issues[{index}].materiality", issue.materiality)?;
        }
        for (index, uncertainty) in self.uncertainties.iter().enumerate() {
            non_empty(&format!("uncertainties[{index}].id"), &uncertainty.id)?;
            non_empty(
                &format!("uncertainties[{index}].description"),
                &uncertainty.description,
            )?;
            check_unit_at(
                &format!("uncertainties[{index}].magnitude"),
                uncertainty.magnitude,
            )?;
        }
        for (index, comparison) in self.comparison_requirements.iter().enumerate() {
            non_empty(
                &format!("comparison_requirements[{index}].subject"),
                &comparison.subject,
            )?;
            non_empty(
                &format!("comparison_requirements[{index}].versus"),
                &comparison.versus,
            )?;
            non_empty(
                &format!("comparison_requirements[{index}].criterion"),
                &comparison.criterion,
            )?;
        }
        for (index, mode) in self.failure_modes.iter().enumerate() {
            non_empty(&format!("failure_modes[{index}].mode"), &mode.mode)?;
            check_unit_at(
                &format!("failure_modes[{index}].likelihood"),
                mode.likelihood,
            )?;
            check_unit_at(&format!("failure_modes[{index}].impact"), mode.impact)?;
        }
        for (index, alternative) in self.simpler_alternatives.iter().enumerate() {
            non_empty(&format!("simpler_alternatives[{index}]"), alternative)?;
        }
        check_unit_at("scan.stakes", self.stakes)?;
        check_unit_at("scan.irreversibility", self.irreversibility)?;
        check_unit_at("scan.verification_value", self.verification_value)
    }
}

/// Refuse a list longer than the scan is allowed to report.
fn check_len(field: &str, len: usize, max: usize) -> Result<()> {
    if len > max {
        return Err(MetacogError::Scan(format!(
            "{field} holds {len} entries; the scan may report at most {max}. \
             A scan that reports everything has prioritised nothing"
        )));
    }
    Ok(())
}

/// Refuse an empty string field, naming the indexed path.
///
/// Scan refusals carry a `String` rather than the `&'static str` a `Validation`
/// refusal names its field with, because the paths here are built at run time
/// (`issues[3].target`). A leaked `&'static str` would keep the leak invisible until
/// it mattered.
fn non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(MetacogError::Scan(format!("{field} must not be empty")));
    }
    Ok(())
}

/// Check a unit-interval scan field, naming the indexed path.
fn check_unit_at(field: &str, value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(MetacogError::Scan(format!(
            "{field} must be in [0,1], got {value}"
        )));
    }
    Ok(())
}

/// A driver that answers the scan. Injectable so the controller can be exercised
/// without a model.
#[async_trait]
pub trait ScanDriver: Send + Sync {
    /// Run the one broad scan.
    async fn scan(&self, request: &ScanRequest) -> Result<ScanResult>;
}

/// The instructions that frame the scan. One prompt, one pass.
pub const SCAN_SYSTEM_PROMPT: &str = "\
You are the metacognitive scan of a bounded deliberation. Read the goal and the \
provisional model, and answer with JSON only. Report what is wrong with the model \
(issues), what it does not settle (uncertainties), the comparisons the answer will \
need, the ways it could fail, and any simpler answer that would do. Then state \
three beliefs in [0,1]: what is at stake, how hard the decision is to undo, and \
how much establishing the truth is worth. Report the *most material* issues only; \
a scan that lists everything has prioritised nothing.";

impl StructuredOut for ScanResult {
    fn schema_id() -> SchemaId {
        SchemaId::new("metacog.scan.v1")
    }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": [
                "issues",
                "uncertainties",
                "comparison_requirements",
                "failure_modes",
                "simpler_alternatives",
                "stakes",
                "irreversibility",
                "verification_value"
            ],
            "properties": {
                "issues": {
                    "type": "array",
                    "maxItems": MAX_ISSUES,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "materiality", "target"],
                        "properties": {
                            "kind": { "type": "string", "minLength": 1 },
                            "materiality": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                            "target": { "type": "string", "minLength": 1 }
                        }
                    }
                },
                "uncertainties": {
                    "type": "array",
                    "maxItems": MAX_UNCERTAINTIES,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["id", "description", "magnitude"],
                        "properties": {
                            "id": { "type": "string", "minLength": 1 },
                            "description": { "type": "string", "minLength": 1 },
                            "magnitude": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                        }
                    }
                },
                "comparison_requirements": {
                    "type": "array",
                    "maxItems": MAX_COMPARISONS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["subject", "versus", "criterion"],
                        "properties": {
                            "subject": { "type": "string", "minLength": 1 },
                            "versus": { "type": "string", "minLength": 1 },
                            "criterion": { "type": "string", "minLength": 1 }
                        }
                    }
                },
                "failure_modes": {
                    "type": "array",
                    "maxItems": MAX_FAILURE_MODES,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["mode", "likelihood", "impact"],
                        "properties": {
                            "mode": { "type": "string", "minLength": 1 },
                            "likelihood": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                            "impact": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                        }
                    }
                },
                "simpler_alternatives": {
                    "type": "array",
                    "maxItems": MAX_SIMPLER_ALTERNATIVES,
                    "items": { "type": "string", "minLength": 1 }
                },
                "stakes": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                "irreversibility": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                "verification_value": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
            }
        })
    }
}

/// A driver that returns a fixed scan. Used by the golden corpus and by tests.
#[derive(Debug, Clone)]
pub struct MockScanDriver {
    result: ScanResult,
}

impl MockScanDriver {
    /// A driver that always answers with `result`, refusing an invalid one now
    /// rather than at every call.
    pub fn new(result: ScanResult) -> Result<Self> {
        result.validate()?;
        Ok(MockScanDriver { result })
    }

    /// The scan this driver answers with.
    pub fn result(&self) -> &ScanResult {
        &self.result
    }
}

#[async_trait]
impl ScanDriver for MockScanDriver {
    async fn scan(&self, _request: &ScanRequest) -> Result<ScanResult> {
        Ok(self.result.clone())
    }
}

/// A driver over the LLM substrate: one request, one strict schema, no repair.
pub struct LlmScanDriver<C: LlmClient> {
    client: Arc<C>,
    registry: Arc<SchemaRegistry>,
    max_tokens: u32,
}

impl<C: LlmClient> Clone for LlmScanDriver<C> {
    fn clone(&self) -> Self {
        LlmScanDriver {
            client: Arc::clone(&self.client),
            registry: Arc::clone(&self.registry),
            max_tokens: self.max_tokens,
        }
    }
}

impl<C: LlmClient> LlmScanDriver<C> {
    /// Build a driver. The registry must already have [`ScanResult`] registered.
    pub fn new(client: Arc<C>, registry: Arc<SchemaRegistry>, max_tokens: u32) -> Self {
        LlmScanDriver {
            client,
            registry,
            max_tokens,
        }
    }

    /// Turn a schema rejection into this crate's refusal, keeping the reason.
    pub fn rejection(error: SchemaError) -> MetacogError {
        MetacogError::Scan(error.to_string())
    }
}

#[async_trait]
impl<C: LlmClient + 'static> ScanDriver for LlmScanDriver<C> {
    async fn scan(&self, request: &ScanRequest) -> Result<ScanResult> {
        let mut llm_request = LlmRequest::new(
            Purpose::Plan,
            vec![
                Message::system(SCAN_SYSTEM_PROMPT),
                Message::user(request.prompt()),
            ],
        )
        .with_schema(ScanResult::schema_id())
        .with_cache(CacheMode::Use);
        llm_request.max_tokens = self.max_tokens;

        let response = self
            .client
            .complete(&llm_request)
            .await
            .map_err(|e| MetacogError::Scan(e.to_string()))?;
        let value = self
            .registry
            .validate(&ScanResult::schema_id(), &response.text)
            .map_err(Self::rejection)?;
        let result: ScanResult = mm_llm::schema::decode(value).map_err(Self::rejection)?;
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mm_llm::{DecoderKind, LlmError, LlmResponse, SchemaRegistry as Registry, Usage};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A provider adapter that answers with whatever text it was handed.
    struct StubClient {
        text: String,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl LlmClient for StubClient {
        async fn complete(&self, _req: &LlmRequest) -> std::result::Result<LlmResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(LlmResponse {
                call_id: mm_core::Ulid::from_parts(1, 1),
                text: self.text.clone(),
                model: "stub".into(),
                usage: Usage::default(),
                cached: false,
                schema_valid: true,
                decoder: DecoderKind::None,
            })
        }

        fn provider(&self) -> &str {
            "stub"
        }
    }

    fn request() -> ScanRequest {
        ScanRequest::new(
            mm_core::Ulid::from_parts(1, 1),
            "roll back the deploy",
            Context::new("roll back the deploy"),
        )
    }

    fn good_scan() -> ScanResult {
        ScanResult {
            issues: vec![ScanIssue {
                kind: "missing_evidence".into(),
                materiality: 0.7,
                target: "rollback".into(),
            }],
            uncertainties: vec![Uncertainty {
                id: "u1".into(),
                description: "does the window hold".into(),
                magnitude: 0.6,
            }],
            comparison_requirements: vec![ComparisonSpec {
                subject: "rollback".into(),
                versus: "roll forward".into(),
                criterion: "downtime".into(),
            }],
            failure_modes: vec![FailureMode {
                mode: "partial rollback".into(),
                likelihood: 0.2,
                impact: 0.8,
            }],
            simpler_alternatives: vec!["ask the on-call".into()],
            stakes: 0.8,
            irreversibility: 0.6,
            verification_value: 0.7,
        }
    }

    #[test]
    fn the_schema_is_strict_and_registers() {
        let mut registry = SchemaRegistry::new();
        mm_llm::schema::register_structured::<ScanResult>(&mut registry).unwrap();
        let text = serde_json::to_string(&good_scan()).unwrap();
        let value = registry.validate(&ScanResult::schema_id(), &text).unwrap();
        let decoded: ScanResult = mm_llm::schema::decode(value).unwrap();
        assert_eq!(decoded, good_scan());
    }

    #[test]
    fn a_malformed_scan_is_rejected_not_coerced() {
        let mut registry = SchemaRegistry::new();
        mm_llm::schema::register_structured::<ScanResult>(&mut registry).unwrap();
        let id = ScanResult::schema_id();

        // Missing a required aggregate.
        let mut missing = serde_json::to_value(good_scan()).unwrap();
        missing.as_object_mut().unwrap().remove("stakes");
        assert!(registry.validate(&id, &missing.to_string()).is_err());

        // An undeclared field.
        let mut extra = serde_json::to_value(good_scan()).unwrap();
        extra["confidence"] = serde_json::json!(0.9);
        assert!(registry.validate(&id, &extra.to_string()).is_err());

        // Out of range.
        let mut out_of_range = serde_json::to_value(good_scan()).unwrap();
        out_of_range["stakes"] = serde_json::json!(1.5);
        assert!(registry.validate(&id, &out_of_range.to_string()).is_err());

        // Not JSON at all.
        assert!(registry.validate(&id, "definitely not json").is_err());
        assert!(registry.validate(&id, "").is_err());
    }

    #[test]
    fn an_over_broad_scan_is_rejected_by_both_the_schema_and_validate() {
        let mut registry = SchemaRegistry::new();
        mm_llm::schema::register_structured::<ScanResult>(&mut registry).unwrap();
        let mut broad = good_scan();
        broad.issues = (0..MAX_ISSUES + 1)
            .map(|i| ScanIssue {
                kind: "k".into(),
                materiality: 0.5,
                target: format!("t{i}"),
            })
            .collect();
        assert!(broad.validate().is_err());
        assert!(registry
            .validate(
                &ScanResult::schema_id(),
                &serde_json::to_string(&broad).unwrap()
            )
            .is_err());
    }

    #[test]
    fn validate_names_the_field_it_refused() {
        let mut scan = good_scan();
        scan.uncertainties[0].magnitude = 2.0;
        let err = scan.validate().unwrap_err();
        assert_eq!(err.code(), "scan");
        assert!(
            err.to_string().contains("uncertainties[0].magnitude"),
            "{err}"
        );

        let mut scan = good_scan();
        scan.issues[0].target = String::new();
        assert!(scan.validate().unwrap_err().to_string().contains("target"));

        let mut scan = good_scan();
        scan.stakes = f64::NAN;
        assert!(scan.validate().is_err());
    }

    #[test]
    fn the_scan_imposes_its_own_ranking() {
        let mut scan = good_scan();
        scan.issues = vec![
            ScanIssue {
                kind: "a".into(),
                materiality: 0.2,
                target: "z".into(),
            },
            ScanIssue {
                kind: "b".into(),
                materiality: 0.9,
                target: "y".into(),
            },
            ScanIssue {
                kind: "c".into(),
                materiality: 0.9,
                target: "x".into(),
            },
        ];
        let ranked = scan.issues_by_materiality();
        assert_eq!(ranked[0].target, "x", "ties break by target");
        assert_eq!(ranked[1].target, "y");
        assert_eq!(ranked[2].target, "z");
        assert_eq!(scan.mean_uncertainty(), 0.6);
        assert_eq!(scan.worst_failure_impact(), 0.8);
    }

    #[test]
    fn the_mock_driver_returns_its_bounded_scan() {
        let driver = MockScanDriver::new(good_scan()).unwrap();
        assert_eq!(driver.result().issues.len(), 1);
        let answer = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(driver.scan(&request()))
            .unwrap();
        assert_eq!(answer, good_scan());

        let mut invalid = good_scan();
        invalid.stakes = -1.0;
        assert!(MockScanDriver::new(invalid).is_err());
    }

    #[test]
    fn the_llm_driver_decodes_a_schema_valid_answer() {
        let mut registry = Registry::new();
        mm_llm::schema::register_structured::<ScanResult>(&mut registry).unwrap();
        let client = Arc::new(StubClient {
            text: serde_json::to_string(&good_scan()).unwrap(),
            calls: AtomicUsize::new(0),
        });
        let driver = LlmScanDriver::new(Arc::clone(&client), Arc::new(registry), 512);
        let answer = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(driver.scan(&request()))
            .unwrap();
        assert_eq!(answer, good_scan());
        assert_eq!(client.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_llm_driver_rejects_rather_than_repairs() {
        let mut registry = Registry::new();
        mm_llm::schema::register_structured::<ScanResult>(&mut registry).unwrap();
        for text in [
            "not json".to_string(),
            serde_json::json!({"stakes": 0.5}).to_string(),
        ] {
            let driver = LlmScanDriver::new(
                Arc::new(StubClient {
                    text,
                    calls: AtomicUsize::new(0),
                }),
                Arc::new(registry.clone()),
                512,
            );
            let err = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(driver.scan(&request()))
                .unwrap_err();
            assert_eq!(err.code(), "scan");
        }
    }

    #[test]
    fn the_prompt_is_deterministic() {
        let first = request().prompt();
        let second = request().prompt();
        assert_eq!(first, second);
        assert!(first.contains("roll back the deploy"));
        assert!(SCAN_SYSTEM_PROMPT.contains("one") || SCAN_SYSTEM_PROMPT.contains("Report"));
    }
}
