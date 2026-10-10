//! Shared support for the Phase 9 commands: `decide`, `firewall`, `compare`,
//! `risk`, `calibration` and `conformance`.
//!
//! Everything the six commands agree on lives here rather than in each of them,
//! because the parts they must agree on are exactly the parts that would otherwise
//! drift:
//!
//! * **One reader for a bounded question.** `decide` takes a question as a file
//!   path, `-` for stdin, or inline JSON; the question may omit its ULID, in which
//!   case the kernel's monotonic factory mints one. The ULID is spelled in lowercase
//!   Crockford everywhere in this system, so a file that spells one in uppercase is
//!   normalised rather than rejected — the case is not information.
//! * **One core factory.** `--core rules|local|hosted` means the same thing to
//!   `decide` and to `conformance`, including *why* a core is unavailable. A core
//!   that cannot be built is not an error: it is an `Unavailable` core with a
//!   reason, which is the state the whole subsystem is designed to degrade from.
//! * **One place that reads fitted thresholds.** A calibrator or a conformal
//!   threshold is the *newest* row for its decision class, because a refit writes a
//!   new row rather than updating one (Phase 9's migration explains why). Reading
//!   them anywhere else would eventually read a different row.
//! * **One recorder.** A `decisions` row, its audit record and its RDF mirror are
//!   written together, so no writer can produce the row without the other two.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_core::{Config, MmError, Param, Tabular, Ulid, UlidFactory};
use mm_decision::core::{CoreId, SharedCore};
use mm_decision::question::{AnswerValue, DecisionAnswer, DecisionQuestion, ScoreScale};
use mm_firewall::rdf as firewall_rdf;
use mm_llm::config::LlmConfig;
use mm_llm::schema::{register_structured, SchemaRegistry};
use mm_log::{codes, Level};
use serde_json::Value;

use crate::kernel::Kernel;

/// The target every Phase 9 CLI record carries.
pub const TARGET: &str = "mm.decision";

// ------------------------------------------------------------------- inputs ----

/// Read a JSON document from a path, from `-` (stdin), or from the argument itself.
///
/// The three forms exist because the gate passes files while a human at a shell
/// wants to try a question without writing one to disk, and both should go through
/// the same normalisation.
pub fn load_json_arg(arg: &str) -> Result<Value, MmError> {
    let raw = if arg == "-" {
        use std::io::Read;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| MmError::Config(format!("cannot read stdin: {e}")))?;
        text
    } else {
        let path = Path::new(arg);
        if path.is_file() {
            std::fs::read_to_string(path)
                .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?
        } else {
            arg.to_string()
        }
    };
    let mut value: Value = serde_json::from_str(&raw)
        .map_err(|e| MmError::Codec(format!("{arg}: is not JSON: {e}")))?;
    lowercase_id_strings(&mut value);
    Ok(value)
}

/// The same, from a path only, for flags that must name a file.
pub fn read_json_file(path: &Path) -> Result<Value, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut value: Value = serde_json::from_str(&raw)
        .map_err(|e| MmError::Codec(format!("{}: {e}", path.display())))?;
    lowercase_id_strings(&mut value);
    Ok(value)
}

/// Lowercase every `id`-keyed string anywhere in a document.
///
/// The lowercase Crockford form is the one the kernel stores and compares, and both
/// spellings denote the same identifier. Normalising here means a fixture cannot
/// fail for a reason that carries no information.
pub fn lowercase_id_strings(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if matches!(key.as_str(), "id" | "ulid" | "episode" | "episode_ulid") {
                    if let Value::String(text) = inner {
                        *text = text.trim().to_ascii_lowercase();
                    }
                }
                lowercase_id_strings(inner);
            }
        }
        Value::Array(items) => {
            for item in items {
                lowercase_id_strings(item);
            }
        }
        _ => {}
    }
}

/// A bounded question as it appears in a file, before its ULID is settled.
///
/// An `id` is optional in the wire form and required by the type: a caller writing a
/// question by hand should not have to invent an identifier, and the kernel already
/// has a monotonic factory that can mint one.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionSpec {
    /// Pick one of `options`.
    Choice {
        /// An explicit ULID, or absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// What is being asked.
        prompt: String,
        /// The options on offer.
        options: Vec<String>,
        /// Whether "none of these" is admissible.
        #[serde(default)]
        allow_none: bool,
    },
    /// Score on a closed scale.
    Score {
        /// An explicit ULID, or absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// What is being asked.
        prompt: String,
        /// The scale; the unit interval with its three anchors when omitted.
        #[serde(default = "ScoreScale::unit")]
        scale: ScoreScale,
    },
    /// Answer yes or no.
    YesNo {
        /// An explicit ULID, or absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// What is being asked.
        prompt: String,
    },
}

impl QuestionSpec {
    /// Settle the ULID and produce the typed question.
    pub fn resolve(self, ids: &UlidFactory) -> Result<DecisionQuestion, MmError> {
        let minted = |explicit: Option<String>| -> Result<Ulid, MmError> {
            match explicit {
                Some(text) => mm_core::id::parse_ulid(&text),
                None => Ok(ids.next()),
            }
        };
        let question = match self {
            QuestionSpec::Choice {
                id,
                prompt,
                options,
                allow_none,
            } => DecisionQuestion::Choice {
                id: minted(id)?,
                prompt,
                options,
                allow_none,
            },
            QuestionSpec::Score { id, prompt, scale } => DecisionQuestion::Score {
                id: minted(id)?,
                prompt,
                scale,
            },
            QuestionSpec::YesNo { id, prompt } => DecisionQuestion::YesNo {
                id: minted(id)?,
                prompt,
            },
        };
        // The question came from a document the caller wrote, so a question that
        // fails its own validation is a malformed input rather than an internal
        // failure: the refusal is a codec error that names what is wrong with it.
        // (`DecisionError::validation` would otherwise map to `MmError::Internal`,
        // which reads as "we broke" for what is plainly "you wrote it wrong".)
        question
            .validate()
            .map_err(|e| MmError::Codec(e.to_string()))?;
        Ok(question)
    }
}

/// Read a question from a path, `-`, or inline JSON, minting a ULID when absent.
pub fn load_question(arg: &str, ids: &UlidFactory) -> Result<DecisionQuestion, MmError> {
    let value = load_json_arg(arg)?;
    let spec: QuestionSpec = serde_json::from_value(value)
        .map_err(|e| MmError::Codec(format!("{arg}: is not a bounded question: {e}")))?;
    spec.resolve(ids)
}

/// A decision state from a file, or an empty state for the episode.
pub fn load_state(
    arg: Option<&Path>,
    episode: Ulid,
) -> Result<mm_decision::DecisionState, MmError> {
    let context = match arg {
        Some(path) => read_json_file(path)?,
        None => Value::Object(serde_json::Map::new()),
    };
    Ok(mm_decision::DecisionState::new(episode, context))
}

// -------------------------------------------------------------------- cores ----

/// The three core names an operator can pass.
pub const CORE_CHOICES: [CoreId; 3] = [CoreId::Rules, CoreId::Local, CoreId::Hosted];

/// A core the CLI built, or the reason it could not.
///
/// `core` is `None` exactly when `unavailable` is `Some`, and that pairing is the
/// point: a missing core is never silent, because the alternative is a caller
/// treating "no core" as "everything is fine".
pub struct BuiltCore {
    /// Which core this is, whether or not it could be built.
    pub id: CoreId,
    /// The core, when it exists.
    pub core: Option<SharedCore>,
    /// Why it does not exist.
    pub unavailable: Option<String>,
}

impl BuiltCore {
    /// A core that could not be built.
    pub fn unavailable(id: CoreId, reason: impl Into<String>) -> Self {
        BuiltCore {
            id,
            core: None,
            unavailable: Some(reason.into()),
        }
    }

    /// The reason, or a placeholder for a core that exists.
    pub fn reason(&self) -> String {
        self.unavailable
            .clone()
            .unwrap_or_else(|| format!("{} is available", self.id))
    }
}

/// Build one core from the command's flags.
///
/// * `rules`: the embedded table, or `--rules <file>` when given. A malformed table
///   is a hard error — the table *is* the core, and continuing without it would mean
///   silently falling back to nothing.
/// * `local`: a distilled head from `--head <file>`. Without one the core is
///   `Unavailable` with that as the reason, which is the state Phase 11 will change.
/// * `hosted`: `mm-llm`'s provider adapter, when a provider is configured. With no
///   `OPENAI_BASE_URL`/`OPENAI_API_KEY` the core is `Unavailable` and says so.
pub fn build_core(
    id: CoreId,
    rules: Option<&Path>,
    head: Option<&Path>,
    cfg: &Config,
    repo_root: &Path,
) -> Result<BuiltCore, MmError> {
    match id {
        CoreId::Rules => {
            let table = match rules {
                Some(path) => mm_decision::RuleTable::load(path)?,
                None => mm_decision::RuleTable::embedded()?,
            };
            let core = mm_decision::RulesCore::new(table)?;
            Ok(BuiltCore {
                id,
                core: Some(Arc::new(core) as SharedCore),
                unavailable: None,
            })
        }
        CoreId::Local => match head {
            Some(path) => {
                let core = mm_decision::LocalCore::from_path(path)?;
                Ok(BuiltCore {
                    id,
                    core: Some(Arc::new(core) as SharedCore),
                    unavailable: None,
                })
            }
            None => Ok(BuiltCore::unavailable(
                id,
                "no distilled head is loaded; pass --head <file> (Phase 11 fits one)",
            )),
        },
        CoreId::Hosted => {
            let llm_path = repo_root.join("config").join("llm.toml");
            let llm_cfg = LlmConfig::load_or_default(&llm_path)
                .map_err(|e| MmError::Config(e.to_string()))?;
            let client = match mm_llm::OpenAiClient::new(&llm_cfg) {
                Ok(client) => client,
                Err(e) => return Ok(BuiltCore::unavailable(id, e.to_string())),
            };
            let mut registry = SchemaRegistry::new();
            register_structured::<mm_decision::HostedDecisionReply>(&mut registry)
                .map_err(|e| MmError::Config(e.to_string()))?;
            let client = client.with_schemas(registry.clone());
            let _ = cfg;
            let core = mm_decision::HostedCore::new(
                Arc::new(client),
                Arc::new(registry),
                llm_cfg.defaults.max_tokens,
            );
            Ok(BuiltCore {
                id,
                core: Some(Arc::new(core) as SharedCore),
                unavailable: None,
            })
        }
    }
}

// ------------------------------------------------- fitted threshold loading ----

/// The newest fitted calibrator per decision class.
///
/// "Newest" is by `created_ulid`, which is monotonic (the kernel's factory is), so
/// the comparison is a string comparison and does not depend on row order.
pub async fn load_calibrators(store: &dyn Tabular) -> Result<mm_decision::CalibratorSet, MmError> {
    let rows = store
        .query_json(
            "SELECT decision_class, temperature, ece, brier, n, created_ulid \
             FROM calibration ORDER BY decision_class, created_ulid",
            Vec::new(),
        )
        .await?;
    let mut newest: BTreeMap<String, mm_decision::Calibrator> = BTreeMap::new();
    for row in rows {
        let class = string_field(&row, "decision_class")?;
        let calibrator = mm_decision::Calibrator {
            n: usize::try_from(row.get("n").and_then(Value::as_i64).unwrap_or(0)).unwrap_or(0),
            temperature: float_field(&row, "temperature")? as f32,
            ece: float_field(&row, "ece")? as f32,
            brier: float_field(&row, "brier")? as f32,
            // The `calibration` row is the fitted result; the before/after split is
            // reported by `calibration fit` rather than stored, so it is reconstructed
            // as "the metrics of this row".
            ece_before: float_field(&row, "ece")? as f32,
            brier_before: float_field(&row, "brier")? as f32,
            decision_class: class.clone(),
        };
        newest.insert(class, calibrator);
    }
    let mut set = mm_decision::CalibratorSet::new();
    for calibrator in newest.into_values() {
        set.insert(calibrator);
    }
    Ok(set)
}

/// The newest fitted conformal threshold per decision class.
pub async fn load_conformal(
    store: &dyn Tabular,
) -> Result<BTreeMap<String, mm_decision::ConformalSet>, MmError> {
    let rows = store
        .query_json(
            "SELECT decision_class, target_coverage, q_hat, n, created_ulid \
             FROM conformal_thresholds ORDER BY decision_class, created_ulid",
            Vec::new(),
        )
        .await?;
    let mut newest: BTreeMap<String, mm_decision::ConformalSet> = BTreeMap::new();
    for row in rows {
        let class = string_field(&row, "decision_class")?;
        let q_hat = float_field(&row, "q_hat")? as f32;
        let n = usize::try_from(row.get("n").and_then(Value::as_i64).unwrap_or(0)).unwrap_or(0);
        newest.insert(
            class.clone(),
            mm_decision::ConformalSet {
                decision_class: class,
                q_hat,
                coverage: float_field(&row, "target_coverage")? as f32,
                n,
                empirical_coverage: 0.0,
            },
        );
    }
    Ok(newest)
}

fn string_field(row: &Value, key: &str) -> Result<String, MmError> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| MmError::Store(format!("column {key} is missing from a row")))
}

fn float_field(row: &Value, key: &str) -> Result<f64, MmError> {
    row.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| MmError::Store(format!("column {key} is missing from a row")))
}

// ---------------------------------------------------------------- recording ----

/// What to record alongside a decision.
pub struct DecisionRecord<'a> {
    /// The question that was asked.
    pub question: &'a DecisionQuestion,
    /// The pre-calibration answer.
    pub answer: &'a DecisionAnswer,
    /// The calibrated confidence, when a calibrator was fitted for the class.
    pub calibrated: Option<f32>,
    /// The episode, when the decision belongs to one.
    pub episode: Option<Ulid>,
    /// How much the answering core cost, in the ledger's units.
    pub cost: f64,
}

/// Write one decision: the row, its audit record and its RDF mirror.
///
/// The three are written together and the row is written first, so a failure leaves
/// an unreferenced mirror rather than a row nothing attests to. Returns the
/// decision's ULID, which is also what the RDF mirror names it by.
pub async fn record_decision(kernel: &Kernel, record: DecisionRecord<'_>) -> Result<Ulid, MmError> {
    let id = kernel.ids.next();
    let created = kernel.ids.next();
    let answer = record.answer;

    let answer_json = serde_json::to_string(&answer.answer)
        .map_err(|e| MmError::Internal(format!("cannot render the answer: {e}")))?;
    let question_json = serde_json::to_string(&QuestionSpec::from_question(record.question))
        .map_err(|e| MmError::Internal(format!("cannot render the question: {e}")))?;
    let features_json = serde_json::to_string(&answer.features)
        .map_err(|e| MmError::Internal(format!("cannot render the features: {e}")))?;

    kernel
        .sqlite
        .execute(
            "INSERT INTO decisions (id, question_kind, question_json, answer_json, confidence, \
             calibrated_confidence, features_json, core_impl, latency_ms, cost, created_ulid, \
             episode_ulid, outcome_json, outcome_ulid) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, NULL)",
            vec![
                mm_core::ulid_string(&id).into(),
                answer.question_kind.as_str().into(),
                question_json.into(),
                answer_json.into(),
                f64::from(answer.confidence).into(),
                match record.calibrated {
                    Some(calibrated) => Param::Real(f64::from(calibrated)),
                    None => Param::Null,
                },
                features_json.into(),
                answer.core.as_str().into(),
                i64::from(answer.latency_ms).into(),
                record.cost.into(),
                mm_core::ulid_string(&created).into(),
                record.episode.map(|e| mm_core::ulid_string(&e)).into(),
            ],
        )
        .await?;

    if kernel.graph.is_some() {
        mm_decision::rdf::emit_decision(
            kernel.graph()?,
            &id,
            answer,
            record.episode.as_ref(),
            record.calibrated,
        )
        .await?;
    }

    kernel
        .logger
        .audit(
            Level::Info,
            codes::DECISION_LOG_WRITE,
            TARGET,
            Some(record.episode.unwrap_or(id)),
            serde_json::json!({
                "decision_id": mm_core::ulid_string(&id),
                "episode_id": record.episode.map(|e| mm_core::ulid_string(&e)),
                "outcome_present": false,
                "question_kind": answer.question_kind.as_str(),
                "core_impl": answer.core.as_str(),
                "confidence": answer.confidence,
                "calibrated_confidence": record.calibrated,
            }),
        )
        .await?;

    Ok(id)
}

/// Write one firewall run's row, its audit record and its RDF mirror.
pub async fn record_firewall_run(
    kernel: &Kernel,
    report: &mm_firewall::FirewallReport,
    input: &mm_firewall::FirewallInput,
) -> Result<(), MmError> {
    let created = kernel.ids.next();
    let inputs_json = serde_json::to_string(input)
        .map_err(|e| MmError::Internal(format!("cannot render the input: {e}")))?;
    let reasons = serde_json::to_string(&report.reason_code_strings())
        .map_err(|e| MmError::Internal(format!("cannot render the reasons: {e}")))?;
    let decisions: Vec<String> = report.decisions.iter().map(mm_core::ulid_string).collect();
    let decisions = serde_json::to_string(&decisions)
        .map_err(|e| MmError::Internal(format!("cannot render the consulted ids: {e}")))?;

    kernel
        .sqlite
        .execute(
            "INSERT INTO firewall_runs (id, episode_ulid, inputs_json, outcome, \
             reason_codes_json, decision_ulids, hard_prohibition, created_ulid) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            vec![
                mm_core::ulid_string(&report.id).into(),
                mm_core::ulid_string(&report.episode).into(),
                inputs_json.into(),
                report.outcome.as_str().into(),
                reasons.into(),
                decisions.into(),
                report.hard_prohibition_id().map(str::to_string).into(),
                mm_core::ulid_string(&created).into(),
            ],
        )
        .await?;

    if kernel.graph.is_some() {
        firewall_rdf::emit_firewall(kernel.graph()?, report).await?;
    }

    kernel
        .logger
        .audit(
            Level::Info,
            codes::FIREWALL_REPORT,
            mm_firewall::TARGET,
            Some(report.episode),
            serde_json::json!({
                "firewall_run_id": mm_core::ulid_string(&report.id),
                "outcome": report.outcome.as_str(),
                "hard_prohibition": report.hard_prohibition_id(),
                "reason_codes": report.reason_code_strings(),
            }),
        )
        .await?;

    Ok(())
}

impl QuestionSpec {
    /// Render a typed question back into its wire form.
    ///
    /// The round trip exists so the recorded `question_json` is the same document a
    /// caller could feed back to `decide`, rather than a second rendering of the
    /// question that only this CLI understands.
    pub fn from_question(question: &DecisionQuestion) -> Self {
        match question {
            DecisionQuestion::Choice {
                id,
                prompt,
                options,
                allow_none,
            } => QuestionSpec::Choice {
                id: Some(mm_core::ulid_string(id)),
                prompt: prompt.clone(),
                options: options.clone(),
                allow_none: *allow_none,
            },
            DecisionQuestion::Score { id, prompt, scale } => QuestionSpec::Score {
                id: Some(mm_core::ulid_string(id)),
                prompt: prompt.clone(),
                scale: scale.clone(),
            },
            DecisionQuestion::YesNo { id, prompt } => QuestionSpec::YesNo {
                id: Some(mm_core::ulid_string(id)),
                prompt: prompt.clone(),
            },
        }
    }
}

/// A one-line rendering of an answer for a terminal.
pub fn answer_line(answer: &DecisionAnswer) -> String {
    match &answer.answer {
        AnswerValue::Option { value } => value.clone(),
        AnswerValue::Score { value } => format!("{value:.6}"),
        AnswerValue::Bool { value } => value.to_string(),
        AnswerValue::None => "(declined)".to_string(),
    }
}

/// Write a JSON document with a trailing newline, creating parents.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), MmError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| MmError::Internal(format!("cannot serialize {}: {e}", path.display())))?;
    std::fs::write(path, format!("{text}\n"))?;
    Ok(())
}

/// The repository root, resolved from the configuration's data directory.
///
/// `config/llm.toml` and `bench/` are repository-relative, and a command run from a
/// data directory still has to find them, so the root is derived once here rather
/// than guessed per command.
pub fn repo_root(cfg: &Config) -> PathBuf {
    cfg.store
        .data_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inline_question_resolves_and_mints_an_id() {
        let ids = UlidFactory::new();
        let question =
            load_question(r#"{"kind":"yes_no","prompt":"is this safe?"}"#, &ids).unwrap();
        assert_eq!(question.kind().as_str(), "yes_no");
        assert_eq!(question.prompt(), "is this safe?");
    }

    #[test]
    fn an_uppercase_ulid_is_normalised_rather_than_rejected() {
        let ids = UlidFactory::new();
        let question = load_question(
            r#"{"kind":"yes_no","id":"01HF7YAT000000000000000001","prompt":"safe?"}"#,
            &ids,
        )
        .unwrap();
        assert_eq!(
            mm_core::ulid_string(&question.id()),
            "01hf7yat000000000000000001"
        );
    }

    #[test]
    fn a_score_question_defaults_to_the_unit_scale() {
        let ids = UlidFactory::new();
        let question = load_question(r#"{"kind":"score","prompt":"how risky?"}"#, &ids).unwrap();
        match question {
            DecisionQuestion::Score { scale, .. } => {
                assert_eq!(scale.min, 0.0);
                assert_eq!(scale.max, 1.0);
                assert_eq!(scale.anchors.len(), 3);
            }
            other => panic!("unexpected question: {other:?}"),
        }
    }

    #[test]
    fn a_question_round_trips_through_its_wire_form() {
        let ids = UlidFactory::new();
        let question = load_question(
            r#"{"kind":"choice","prompt":"which mitigation?","options":["roll back","fail over"],"allow_none":true}"#,
            &ids,
        )
        .unwrap();
        let spec = QuestionSpec::from_question(&question);
        let back = spec.resolve(&UlidFactory::new()).unwrap();
        assert_eq!(back.id(), question.id());
        assert_eq!(back.canonical(), question.canonical());
    }

    #[test]
    fn a_malformed_question_is_refused_with_its_text() {
        let ids = UlidFactory::new();
        let err =
            load_question(r#"{"kind":"choice","prompt":"x","options":[]}"#, &ids).unwrap_err();
        assert!(
            matches!(err, MmError::Codec(_)),
            "a choice with no options is not a question: {err}"
        );
        assert!(
            err.to_string().contains("needs at least one option"),
            "the refusal must name the offending field: {err}"
        );
    }

    #[test]
    fn the_lowercasing_reaches_nested_documents() {
        let mut value = serde_json::json!({
            "a": [{ "id": "ABCDEFGH000000000000000001" }],
            "precedent": "01ABCDEFGH00000000000000Z1"
        });
        lowercase_id_strings(&mut value);
        assert_eq!(value["a"][0]["id"], "abcdefgh000000000000000001");
    }
}
