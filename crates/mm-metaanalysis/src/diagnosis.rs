//! The bounded diagnosis: one pass, a written explanation, at least one class.
//!
//! Phase 11 asks for *one bounded LLM pass* over the episode context, and it is
//! bounded in three directions: one call, a fixed prompt, and a typed result that must
//! name at least one error class. A diagnosis that classified nothing cannot be acted
//! on — there is no change to make for "something went wrong" — so both diagnosers
//! refuse to produce one.
//!
//! Two diagnosers, and which one runs is a configuration decision, not a fallback that
//! happens silently:
//!
//! * [`LlmDiagnoser`] is the plan's path: one `complete` call whose answer is decoded
//!   into [`Diagnosis`]. It is used when a provider is configured.
//! * [`HeuristicDiagnoser`] is the offline path, and it is a *documented cue table*
//!   rather than a stub: it maps words in the goal and failure text to error classes
//!   with fixed arithmetic, so an environment with no provider still classifies the
//!   seeded failure reproducibly. A gate that needed a key would be a gate that only
//!   runs on one machine.
//!
//! What neither diagnoser does is decide. The analysis is evidence that a change is
//! worth proposing; `mm-selfeng`'s gate is what decides whether it is promoted, and it
//! does not read this module's confidence for anything.

use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};

use crate::calibration::Calibrator;
use crate::error::{MetaError, Result};
use crate::taxonomy::{ErrorClass, Trigger};
use crate::triggers::EpisodeContext;

/// The schema identifier the LLM path asks for.
///
/// The diagnosis is decoded into [`Diagnosis`] and refused when a required field is
/// absent, which is the property the grammar layer would otherwise provide; a
/// one-field schema does not need a registered grammar, and pretending it does would
/// add a registry entry that nothing else reads.
pub const DIAGNOSIS_SCHEMA_ID: &str = "mm.metaanalysis.diagnosis@1";

/// The system prompt of the one bounded pass. Fixed, so two runs of the same episode
/// ask the same question.
pub const DIAGNOSIS_SYSTEM_PROMPT: &str = "You diagnose failures of an autonomous agent. \
Answer with a JSON object: {\"diagnosis\": string, \"error_classes\": [string], \"confidence\": number}. \
The error classes come from this closed list: knowledge, retrieval, interpretation, comparison, \
assumption, causal, planning, decision, execution, verification, social, resource, policy, code, \
data, model. Name at least one class, and explain the failure in one or two sentences. Do not \
speculate beyond the episode you are given.";

/// What one diagnosis produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    /// The written explanation.
    pub text: String,
    /// The classes it classified.
    pub error_classes: Vec<ErrorClass>,
    /// The model that answered, when one did.
    pub model: Option<String>,
    /// A digest of the prompt, so two runs can be compared without storing the text.
    pub prompt_hash: Option<String>,
}

impl Diagnosis {
    /// A refusal unless at least one class was named.
    pub fn validate(&self) -> Result<()> {
        if self.text.trim().is_empty() {
            return Err(MetaError::validation("diagnosis", "must not be empty"));
        }
        if self.error_classes.is_empty() {
            return Err(MetaError::Unclassifiable(
                "the diagnosis named no error class".to_string(),
            ));
        }
        Ok(())
    }

    /// The classes as wire names, for the stored row and the log record.
    pub fn class_names(&self) -> Vec<String> {
        self.error_classes
            .iter()
            .map(|class| class.as_str().to_string())
            .collect()
    }
}

/// The raw shape the model answers with, before the class names are parsed.
#[derive(Debug, Deserialize)]
struct DiagnosisReply {
    diagnosis: String,
    #[serde(default)]
    error_classes: Vec<String>,
}

/// Classify one episode.
#[async_trait]
pub trait Diagnoser: Send + Sync {
    /// Diagnose the episode.
    async fn diagnose(&self, ctx: &EpisodeContext) -> Result<Diagnosis>;

    /// The name the log record carries.
    fn name(&self) -> &'static str;
}

/// The offline diagnoser: a documented cue table, applied in a fixed order.
pub struct HeuristicDiagnoser;

/// The cue table. Each entry is `(words, class)`; the words are matched
/// case-insensitively against the goal and the failure together.
///
/// The table is the *specification* of the offline classifier, not a hint: a test
/// asserts that the seeded failure classifies as `retrieval`, and the table is what
/// makes that assertion reproducible on any machine.
const CUES: [(&[&str], ErrorClass); 16] = [
    (&["column", "row", "record", "value"], ErrorClass::Data),
    (
        &["schema", "table", "migration", "version"],
        ErrorClass::Knowledge,
    ),
    (
        &["re-read", "reread", "retriev", "lookup", "search", "recall"],
        ErrorClass::Retrieval,
    ),
    (
        &["compar", "versus", "baseline", "unit"],
        ErrorClass::Comparison,
    ),
    (
        &["assum", "presum", "expected that"],
        ErrorClass::Assumption,
    ),
    (
        &["because", "causal", "root cause", "attribut"],
        ErrorClass::Causal,
    ),
    (
        &["plan", "sequence", "step order", "ordering"],
        ErrorClass::Planning,
    ),
    (
        &["choose", "option", "decision", "threshold", "trade"],
        ErrorClass::Decision,
    ),
    (
        &["tool", "execut", "command", "invoke", "run "],
        ErrorClass::Execution,
    ),
    (
        &["verif", "test", "check", "validate", "assert"],
        ErrorClass::Verification,
    ),
    (
        &["user", "operator", "social", "conversation", "tone"],
        ErrorClass::Social,
    ),
    (
        &["cost", "budget", "resource", "timeout", "quota", "latency"],
        ErrorClass::Resource,
    ),
    (
        &["policy", "forbidden", "permission", "grant"],
        ErrorClass::Policy,
    ),
    (
        &[
            "compile",
            "code",
            "crate",
            "module",
            "function",
            "type error",
        ],
        ErrorClass::Code,
    ),
    (
        &["model", "prompt", "temperature", "hallucin", "sample"],
        ErrorClass::Model,
    ),
    (
        &["interpret", "ambigu", "misread", "unclear"],
        ErrorClass::Interpretation,
    ),
];

impl HeuristicDiagnoser {
    /// The classes a text cues to, in table order and without duplicates.
    ///
    /// Never empty: a text that cues nothing is classified `Execution`, because the
    /// action ran and did not work, and a diagnoser that could return an empty set
    /// would be one that can refuse to answer.
    pub fn classify(text: &str) -> Vec<ErrorClass> {
        let haystack = text.to_lowercase();
        let mut classes: Vec<ErrorClass> = Vec::new();
        for (words, class) in CUES {
            if words.iter().any(|word| haystack.contains(word)) && !classes.contains(&class) {
                classes.push(class);
            }
        }
        if classes.is_empty() {
            classes.push(ErrorClass::Execution);
        }
        classes
    }
}

#[async_trait]
impl Diagnoser for HeuristicDiagnoser {
    async fn diagnose(&self, ctx: &EpisodeContext) -> Result<Diagnosis> {
        let classes = Self::classify(&ctx.classifier_text());
        let diagnosis = Diagnosis {
            text: format!(
                "{} failed after {} attempt(s): {}. The error classes are {}.",
                ctx.goal.trim(),
                ctx.attempts,
                ctx.failure.trim(),
                classes
                    .iter()
                    .map(|class| class.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            error_classes: classes,
            model: None,
            prompt_hash: Some(mm_core::content_hash(DIAGNOSIS_SYSTEM_PROMPT.as_bytes())),
        };
        diagnosis.validate()?;
        Ok(diagnosis)
    }

    fn name(&self) -> &'static str {
        "heuristic"
    }
}

/// The plan's path: one bounded call to the substrate.
pub struct LlmDiagnoser {
    client: Arc<dyn mm_llm::LlmClient>,
    max_tokens: u32,
}

impl LlmDiagnoser {
    /// Build a diagnoser over a client.
    pub fn new(client: Arc<dyn mm_llm::LlmClient>, max_tokens: u32) -> Self {
        LlmDiagnoser { client, max_tokens }
    }

    /// The prompt one episode is asked about.
    pub fn prompt(ctx: &EpisodeContext) -> String {
        format!(
            "Trigger: {}\nGoal: {}\nFailure: {}\nAttempts: {}\nConditions: {}",
            ctx.trigger.as_str(),
            ctx.goal,
            ctx.failure,
            ctx.attempts,
            ctx.conditions.join("; ")
        )
    }
}

#[async_trait]
impl Diagnoser for LlmDiagnoser {
    async fn diagnose(&self, ctx: &EpisodeContext) -> Result<Diagnosis> {
        let mut request = mm_llm::LlmRequest::new(
            mm_llm::Purpose::Plan,
            vec![
                mm_llm::Message::system(DIAGNOSIS_SYSTEM_PROMPT),
                mm_llm::Message::user(Self::prompt(ctx)),
            ],
        )
        .with_cache(mm_llm::CacheMode::Use);
        request.max_tokens = self.max_tokens;

        let response = self
            .client
            .complete(&request)
            .await
            .map_err(|e| MetaError::Diagnosis(e.to_string()))?;
        let reply: DiagnosisReply = serde_json::from_str(&response.text)
            .map_err(|e| MetaError::Diagnosis(format!("the answer is not a diagnosis: {e}")))?;
        let mut classes = Vec::new();
        for name in &reply.error_classes {
            let class = ErrorClass::parse(&name.trim().to_lowercase()).ok_or_else(|| {
                MetaError::Diagnosis(format!("{name:?} is not one of the sixteen error classes"))
            })?;
            if !classes.contains(&class) {
                classes.push(class);
            }
        }
        let diagnosis = Diagnosis {
            text: reply.diagnosis.trim().to_string(),
            error_classes: classes,
            model: Some(response.model.clone()),
            prompt_hash: Some(mm_core::content_hash(
                format!("{DIAGNOSIS_SYSTEM_PROMPT}{}", Self::prompt(ctx)).as_bytes(),
            )),
        };
        diagnosis.validate()?;
        Ok(diagnosis)
    }

    fn name(&self) -> &'static str {
        "llm"
    }
}

/// One diagnosis, persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct MetaAnalysis {
    /// The row's identifier.
    pub id: Ulid,
    /// The trigger that put the episode in the queue.
    pub trigger: Trigger,
    /// The episode analysed.
    pub episode: Ulid,
    /// The written diagnosis.
    pub diagnosis: String,
    /// The classes it named.
    pub error_classes: Vec<ErrorClass>,
    /// How often the failure has recurred.
    pub recurrence: f64,
    /// How much it cost.
    pub impact: f64,
    /// When the analysis ran.
    pub created_at: Timestamp,
}

/// Everything the diagnosis needs, assembled by the caller.
///
/// The library is optional and its absence is documented rather than silently
/// tolerated: without a manager the lesson is returned and logged but is not indexed
/// in the cognitive library, which is a state an operator should be able to see.
pub struct MetaContext {
    /// The tabular store the analysis is written to.
    pub sql: mm_store_sqlite::SqliteStore,
    /// The logger.
    pub logger: Arc<Logger>,
    /// The identifier factory.
    pub ids: Arc<UlidFactory>,
    /// The diagnoser to run.
    pub diagnoser: Arc<dyn Diagnoser>,
    /// The library the lesson is written to, when one is open.
    pub library: Option<Arc<mm_library::LibraryManager>>,
    /// The baseline the calibration gate compares against.
    pub baseline_brier: f64,
}

impl MetaContext {
    /// A context with the offline diagnoser and no library.
    pub fn local(
        sql: mm_store_sqlite::SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
    ) -> Self {
        MetaContext {
            sql,
            logger,
            ids,
            diagnoser: Arc::new(HeuristicDiagnoser),
            library: None,
            baseline_brier: 0.30,
        }
    }

    /// Attach the cognitive library the lesson is written to.
    pub fn with_library(mut self, library: Arc<mm_library::LibraryManager>) -> Self {
        self.library = Some(library);
        self
    }

    /// Attach a calibrator's baseline, for the report that accompanies the analysis.
    pub fn with_baseline(mut self, baseline_brier: f64) -> Self {
        self.baseline_brier = baseline_brier;
        self
    }
}

/// Diagnose one episode and write exactly one `meta_analyses` row.
///
/// One row per call is the contract: the row is the record that something was
/// analysed, and a second call about the same episode is a second look, not an edit.
pub async fn analyze(ctx: &MetaContext, episode: &EpisodeContext) -> Result<MetaAnalysis> {
    let diagnosis = ctx.diagnoser.diagnose(episode).await?;
    let id = ctx.ids.next();
    let at = Timestamp::now();
    let classes = serde_json::to_string(&diagnosis.class_names())?;
    Tabular::execute(
        &ctx.sql,
        "INSERT INTO meta_analyses (id, trigger, episode_ulid, diagnosis, \
         error_classes_json, recurrence, impact, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        vec![
            Param::Text(mm_core::ulid_string(&id)),
            Param::Text(episode.trigger.as_str().to_string()),
            Param::Text(mm_core::ulid_string(&episode.episode_id)),
            Param::Text(diagnosis.text.clone()),
            Param::Text(classes),
            Param::Real(episode.priority.recurrence),
            Param::Real(episode.priority.impact),
            Param::Text(at.to_rfc3339()),
        ],
    )
    .await
    .map_err(|e| MetaError::Store(e.to_string()))?;

    ctx.logger
        .audit(
            Level::Info,
            codes::META_DIAGNOSE,
            crate::TARGET,
            Some(id),
            serde_json::json!({
                "analysis_id": mm_core::ulid_string(&id),
                "error_classes": diagnosis.class_names(),
                "model": diagnosis.model.clone().unwrap_or_else(|| ctx.diagnoser.name().to_string()),
                "prompt_hash": diagnosis.prompt_hash.clone().unwrap_or_default(),
                "recurrence": episode.priority.recurrence,
                "impact": episode.priority.impact,
            }),
        )
        .await?;

    Ok(MetaAnalysis {
        id,
        trigger: episode.trigger,
        episode: episode.episode_id,
        diagnosis: diagnosis.text,
        error_classes: diagnosis.error_classes,
        recurrence: episode.priority.recurrence,
        impact: episode.priority.impact,
        created_at: at,
    })
}

/// Diagnose the episode a fixture describes.
pub async fn analyze_fixture(path: &std::path::Path, ctx: &MetaContext) -> Result<MetaAnalysis> {
    let episode = crate::triggers::load_episode(path).await?;
    analyze(ctx, &episode).await
}

/// The target precision a calibrator is asked for by default, re-exported so a caller
/// that builds a context can score a set without importing the calibration module.
pub fn default_target_precision() -> f64 {
    crate::calibration::DEFAULT_TARGET_PRECISION
}

/// Score a labeled set with the context's baseline, for the report a CLI prints
/// alongside an analysis.
pub fn score(
    labeled: &[crate::calibration::Labeled],
    ctx: &MetaContext,
) -> crate::calibration::CalibrationReport {
    let calibrator = crate::calibration::MetricsCalibrator::fit(
        labeled,
        ctx.baseline_brier,
        default_target_precision(),
    );
    calibrator.score(labeled)
}

/// The trait object a calibrator is used through, for callers that want to inject one.
pub type SharedDiagnoser = Arc<dyn Diagnoser>;

#[cfg(test)]
mod tests {
    use super::*;

    fn episode() -> EpisodeContext {
        EpisodeContext {
            episode_id: Ulid::from_parts(1_700_000_000_000, 1),
            trigger: Trigger::RepeatedFailure,
            goal: "answer the operator's question about the shape of the tool registry".into(),
            failure: "the answer cited a column that no longer exists, because the retrieval \
                      step did not re-read the schema after the migration"
                .into(),
            attempts: 3,
            evidence: vec![Ulid::from_parts(1_700_000_000_000, 2)],
            conditions: vec!["the question names a table".into()],
            priority: crate::taxonomy::MetaAnalysisPriority::new(0.2, 0.7, 0.6, 0.8, 0.4).unwrap(),
            expected_classes: vec!["retrieval".into()],
            expected_lessons: 1,
        }
    }

    #[tokio::test]
    async fn the_seeded_failure_classifies_as_retrieval_among_others() {
        let diagnosis = HeuristicDiagnoser.diagnose(&episode()).await.unwrap();
        assert!(!diagnosis.error_classes.is_empty());
        assert!(
            diagnosis.error_classes.contains(&ErrorClass::Retrieval),
            "{:?}",
            diagnosis.error_classes
        );
        assert!(diagnosis.error_classes.contains(&ErrorClass::Data));
        assert!(diagnosis.model.is_none());
        assert!(diagnosis.prompt_hash.is_some());
        assert!(diagnosis.text.contains("retrieval"));
    }

    #[test]
    fn a_text_that_cues_nothing_still_classifies() {
        let classes = HeuristicDiagnoser::classify("it did not work");
        assert_eq!(classes, vec![ErrorClass::Execution]);
    }

    #[test]
    fn the_cue_table_is_deterministic_and_deduplicated() {
        let text = "the tool ran, the tool failed";
        let first = HeuristicDiagnoser::classify(text);
        let second = HeuristicDiagnoser::classify(text);
        assert_eq!(first, second);
        let unique: std::collections::BTreeSet<&ErrorClass> = first.iter().collect();
        assert_eq!(unique.len(), first.len());
    }

    #[test]
    fn every_class_is_reachable_from_some_cue() {
        // A cue table that cannot produce a class is a class nothing can be told apart
        // by, which would make the taxonomy decorative.
        let mut seen = std::collections::BTreeSet::new();
        for (words, class) in CUES {
            assert!(!words.is_empty());
            seen.insert(class);
        }
        assert_eq!(seen.len(), ErrorClass::ALL.len());
    }

    #[tokio::test]
    async fn an_analysis_writes_one_row_and_records_its_classes() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let ctx = MetaContext::local(
            store.clone(),
            crate::test_logger(&store),
            Arc::new(UlidFactory::new()),
        );
        let analysis = analyze(&ctx, &episode()).await.unwrap();
        assert_eq!(analysis.trigger, Trigger::RepeatedFailure);
        assert!(analysis.error_classes.contains(&ErrorClass::Retrieval));
        assert_eq!(store.row_count("meta_analyses").await.unwrap(), 1);

        // A second look is a second row, not an edit.
        analyze(&ctx, &episode()).await.unwrap();
        assert_eq!(store.row_count("meta_analyses").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn a_diagnosis_that_names_nothing_is_refused() {
        let empty = Diagnosis {
            text: "something went wrong".into(),
            error_classes: Vec::new(),
            model: None,
            prompt_hash: None,
        };
        let error = empty.validate().unwrap_err();
        assert_eq!(error.code(), "unclassifiable");

        let silent = Diagnosis {
            text: "   ".into(),
            error_classes: vec![ErrorClass::Execution],
            model: None,
            prompt_hash: None,
        };
        assert!(silent.validate().is_err());
    }

    #[tokio::test]
    async fn the_fixture_path_produces_one_row_with_at_least_one_class() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let ctx = MetaContext::local(
            store.clone(),
            crate::test_logger(&store),
            Arc::new(UlidFactory::new()),
        );
        let path = mm_core::Config::repo_root().join("bench/episodes/seeded_failure_01.json");
        let analysis = analyze_fixture(&path, &ctx).await.unwrap();
        assert_eq!(analysis.trigger, Trigger::RepeatedFailure);
        assert!(!analysis.error_classes.is_empty());
        assert_eq!(store.row_count("meta_analyses").await.unwrap(), 1);
    }

    #[test]
    fn the_llm_prompt_is_fixed_and_content_addressed() {
        let ctx = episode();
        let prompt = LlmDiagnoser::prompt(&ctx);
        assert!(prompt.contains("Trigger: repeated_failure"), "{prompt}");
        assert!(prompt.contains(&ctx.failure), "{prompt}");
        let prompt_hash =
            mm_core::content_hash(format!("{DIAGNOSIS_SYSTEM_PROMPT}{prompt}").as_bytes());
        assert_eq!(prompt_hash.len(), 64);
        // A reply that names a class outside the closed list is parseable as JSON and
        // refused by the class parser, which is what keeps the vocabulary closed.
        let reply: DiagnosisReply =
            serde_json::from_str("{\"diagnosis\":\"x\",\"error_classes\":[\"nonsense\"]}").unwrap();
        assert!(ErrorClass::parse(&reply.error_classes[0]).is_none());
        // A reply with no classes parses and is then refused by `validate`.
        let empty: DiagnosisReply = serde_json::from_str("{\"diagnosis\":\"x\"}").unwrap();
        assert!(empty.error_classes.is_empty());
    }

    #[tokio::test]
    async fn scoring_a_labeled_set_uses_the_context_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let ctx = MetaContext::local(
            store.clone(),
            crate::test_logger(&store),
            Arc::new(UlidFactory::new()),
        )
        .with_baseline(0.25);
        let labeled: Vec<crate::calibration::Labeled> = (0..20)
            .map(|index| crate::calibration::Labeled {
                class: "safety".into(),
                predicted: 0.9,
                label: index < 16,
            })
            .collect();
        let report = score(&labeled, &ctx);
        assert_eq!(report.n, 20);
        assert!((report.baseline_brier - 0.25).abs() < 1e-12);
    }
}
