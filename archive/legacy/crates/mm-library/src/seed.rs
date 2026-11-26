//! Seed extraction: prose-shaped candidates in, validated entries out.
//!
//! The plan's §4.5 says `library extract` turns candidate entries into library
//! content and that **an unvalidated entry is never committed**. Two ideas carry
//! that here:
//!
//! * **The gate is the manager's, not this module's.** [`extract`] never writes an
//!   entry itself: it builds a [`Declarative`] and hands it to
//!   [`LibraryManager::commit_declarative`], which runs the committed SHACL shapes,
//!   the content-hash duplicate check and the orphan check and *writes nothing* when
//!   any of them refuses. Extraction can therefore not grow a second, weaker door
//!   into `/library`.
//! * **A refusal is a record, not an exception.** A line that cannot be read, or an
//!   entry the gate refuses, lands in the *rejection ledger* with its 1-based line
//!   number and a reason. The good lines are still extracted; the caller (the CLI,
//!   and the gate script) decides the exit code from
//!   [`ExtractionReport::has_rejections`]. A batch that silently dropped half its
//!   input would pass a gate it should fail.
//!
//! ## Line schema
//!
//! One JSON object per line:
//!
//! | key | required | meaning |
//! |---|---|---|
//! | `kind` | yes | a declarative [`EntryKind`] name, case-insensitive |
//! | `slug` | yes | the IRI slug; must satisfy [`crate::entry::iri::is_slug`] |
//! | `title` | yes | the entry's title |
//! | `purpose` | yes | what the entry is for |
//! | `domains` | no | array of domain names |
//! | `applicable_when` | no | array of conditions (strings) |
//! | `evidence` | no | array of IRIs |
//! | `examples` | no | array of IRIs |
//! | `corrective_rule` | required for `AntiPattern` | what to do instead |
//! | `confidence` | no | number in `[0,1]` |
//!
//! **An unknown key is a hard error**, matching the entry codec's rule: a field the
//! kernel does not understand is either a typo that silently loses data or a
//! feature that was never implemented, and both are worse as a silent no-op than as
//! a refusal.
//!
//! ## What the fixtures assume
//!
//! `bench/library/extraction/raw.jsonl` names seed entries
//! (`…/library/case/outage-rollback` and friends) as its evidence, so the gate
//! imports `ontology/seed/library_seed.ttl` **before** extracting. On a store with
//! no seed those entries are genuinely orphaned and are refused — which is the
//! orphan rule doing its job, not a broken fixture.

use mm_core::{NamedNode, Ulid};
use mm_log::{codes, Level, LogRecord};

use crate::entry::{Declarative, EntryKind};
use crate::error::{LibraryError, Result};
use crate::manager::{LibraryManager, TARGET};

/// The keys a candidate line may carry. Anything else is refused.
pub const CANDIDATE_KEYS: [&str; 10] = [
    "kind",
    "slug",
    "title",
    "purpose",
    "domains",
    "applicable_when",
    "evidence",
    "examples",
    "corrective_rule",
    "confidence",
];

/// The kinds extraction may produce: the ones prose can carry.
///
/// A skill (an artefact and a test), a frame (roles and slots), a case (problem,
/// solution, outcome) and a workflow (an induced step list) have structure of
/// their own and are *registered* from that structure rather than extracted from a
/// sentence; allowing them here would mean inventing their required fields.
pub fn extractable_kinds() -> Vec<EntryKind> {
    crate::entry::ENTRY_KINDS
        .into_iter()
        .filter(|kind| kind.is_declarative())
        .collect()
}

/// One line the extraction could not commit, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rejection {
    /// The 1-based line in the source document.
    pub line: usize,
    /// The slug, when the line parsed far enough to have one.
    pub iri_or_slug: String,
    /// Why it was refused.
    pub reason: String,
}

/// What an extraction run did.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionReport {
    /// The job's ULID, which is also the trace id of its records.
    pub job: Ulid,
    /// The source document, as the caller named it.
    pub source: String,
    /// The versioned IRIs the gate accepted, in input order.
    pub accepted: Vec<String>,
    /// The lines the gate or the parser refused, in line order.
    pub rejected: Vec<Rejection>,
    /// The accepted entries, one JSON object per line.
    pub accepted_jsonl: String,
}

impl ExtractionReport {
    /// True when anything at all was refused, so the caller can exit non-zero.
    pub fn has_rejections(&self) -> bool {
        !self.rejected.is_empty()
    }

    /// The one-line summary the CLI prints.
    pub fn summary(&self) -> String {
        format!(
            "{} accepted, {} rejected (source {})",
            self.accepted.len(),
            self.rejected.len(),
            self.source
        )
    }
}

/// Parse one candidate line into an entry that is ready for the gate.
pub fn parse_line(line: &str) -> Result<Declarative> {
    let value: serde_json::Value = serde_json::from_str(line.trim())
        .map_err(|e| LibraryError::validation("json", format!("{e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| LibraryError::validation("json", "a candidate must be a JSON object"))?;

    for key in object.keys() {
        if !CANDIDATE_KEYS.contains(&key.as_str()) {
            return Err(LibraryError::validation(
                "json",
                format!(
                    "unknown key `{key}`; known keys are {}",
                    CANDIDATE_KEYS.join(", ")
                ),
            ));
        }
    }

    let kind_text = required_string(object, "kind")?;
    let kind = EntryKind::parse(&kind_text)
        .ok_or_else(|| LibraryError::validation("kind", format!("unknown kind `{kind_text}`")))?;
    if !kind.is_declarative() {
        return Err(LibraryError::validation(
            "kind",
            format!("{kind} is not extracted from prose; the declarative kinds are extractable"),
        ));
    }

    let slug = required_string(object, "slug")?;
    let title = required_string(object, "title")?;
    let purpose = required_string(object, "purpose")?;

    let mut entry = Declarative::new(kind, &slug, &title, &purpose)?;
    for domain in string_list(object, "domains")? {
        entry = entry.with_domain(&domain);
    }
    for condition in string_list(object, "applicable_when")? {
        entry = entry.when(crate::doctrine::when(&condition)?);
    }
    for iri in parse_evidence(&value)? {
        entry = entry.backed_by(iri);
    }
    for iri in iri_list(object, "examples")? {
        entry = entry.illustrated_by(iri);
    }
    if let Some(rule) = optional_string(object, "corrective_rule")? {
        entry = entry.corrective_rule(&rule);
    }
    if let Some(confidence) = optional_number(object, "confidence")? {
        entry = entry.with_confidence(confidence as f32)?;
    }
    Ok(entry)
}

/// The `evidence` IRIs of a candidate object.
pub fn parse_evidence(json: &serde_json::Value) -> Result<Vec<NamedNode>> {
    match json.as_object() {
        Some(object) => iri_list(object, "evidence"),
        None => Err(LibraryError::validation(
            "json",
            "a candidate must be a JSON object",
        )),
    }
}

/// Parse every candidate in a document, keeping the refusals in line order.
///
/// Pure and deterministic: no clock, no store, no model. The same document always
/// produces the same entries and the same ledger, which is what makes a golden
/// comparison of a rejection set meaningful.
pub fn extract_candidates(jsonl: &str) -> (Vec<Declarative>, Vec<Rejection>) {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for (line, label, parsed) in scan(jsonl) {
        match parsed {
            Ok(entry) => accepted.push(entry),
            Err(e) => rejected.push(Rejection {
                line,
                iri_or_slug: label,
                reason: e.to_string(),
            }),
        }
    }
    (accepted, rejected)
}

/// Extract a document through the commit gate, keeping a rejection ledger.
///
/// The job ULID comes from the store's factory, so a run's two log records and any
/// entry it commits share one trace id.
pub async fn extract(
    manager: &LibraryManager,
    source: &str,
    jsonl: &str,
) -> Result<ExtractionReport> {
    let job = manager.store().next_id();
    let job_text = mm_core::ulid_string(&job);
    let lines = scan(jsonl);
    let candidates = lines.len();

    manager
        .logger()
        .emit(
            LogRecord::new(Level::Info, codes::LIBRARY_EXTRACT_JOB_START, TARGET)
                .with_field("job_ulid", job_text.clone())
                .with_field("source", source.to_string())
                .with_field("model", "deterministic")
                .with_field("candidates", candidates),
        )
        .await
        .map_err(LibraryError::from)?;

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut accepted_lines: Vec<String> = Vec::new();

    for (line, label, parsed) in lines {
        let entry = match parsed {
            Ok(entry) => entry,
            Err(e) => {
                rejected.push(Rejection {
                    line,
                    iri_or_slug: label,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        // The manager's gate, not this module's: an entry the shapes, the duplicate
        // check or the orphan check refuses is ledgered and nothing is written.
        match manager.commit_declarative(&entry).await {
            Ok(outcome) => {
                accepted_lines.push(accepted_line(&entry));
                accepted.push(outcome.version_iri);
            }
            Err(e) => rejected.push(Rejection {
                line,
                iri_or_slug: entry.slug(),
                reason: e.to_string(),
            }),
        }
    }

    let report = ExtractionReport {
        job,
        source: source.to_string(),
        accepted,
        rejected,
        accepted_jsonl: accepted_lines.join("\n"),
    };

    manager
        .logger()
        .emit(
            LogRecord::new(Level::Info, codes::LIBRARY_EXTRACT_JOB_END, TARGET)
                .with_field("job_ulid", job_text)
                .with_field("source", source.to_string())
                .with_field("model", "deterministic")
                .with_field("accepted", report.accepted.len())
                .with_field("rejected", report.rejected.len()),
        )
        .await
        .map_err(LibraryError::from)?;

    Ok(report)
}

/// Parse a document line by line, keeping the 1-based line number and a label for
/// each attempt. Blank lines are skipped rather than ledgered: whitespace is not a
/// candidate.
fn scan(jsonl: &str) -> Vec<(usize, String, Result<Declarative>)> {
    let mut out = Vec::new();
    for (index, raw) in jsonl.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let line = index + 1;
        let parsed = parse_line(raw);
        let label = match &parsed {
            Ok(entry) => entry.slug(),
            Err(_) => label_of(raw),
        };
        out.push((line, label, parsed));
    }
    out
}

/// The best label a line can offer: its slug, or the line itself, truncated.
fn label_of(line: &str) -> String {
    let slug = serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("slug")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| line.trim().to_string());
    slug.chars().take(60).collect()
}

/// The JSONL rendering of an accepted entry.
fn accepted_line(entry: &Declarative) -> String {
    serde_json::json!({
        "kind": entry.kind.as_str(),
        "iri": entry.iri.as_str(),
        "slug": entry.slug(),
        "title": entry.title.clone(),
        "confidence": entry.confidence,
    })
    .to_string()
}

fn required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<String> {
    match object.get(key) {
        Some(serde_json::Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
        Some(serde_json::Value::String(_)) => {
            Err(LibraryError::validation(key, "must not be blank"))
        }
        Some(_) => Err(LibraryError::validation(key, "must be a string")),
        None => Err(LibraryError::validation(key, "is required")),
    }
}

fn optional_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<Option<String>> {
    match object.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.clone())),
        Some(_) => Err(LibraryError::validation(key, "must be a non-empty string")),
    }
}

fn optional_number(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<Option<f64>> {
    match object.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => match value.as_f64() {
            Some(number) if number.is_finite() => Ok(Some(number)),
            _ => Err(LibraryError::validation(key, "must be a number")),
        },
    }
}

fn string_list(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<Vec<String>> {
    match object.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| match item.as_str() {
                Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
                _ => Err(LibraryError::validation(
                    key,
                    "must contain non-empty strings",
                )),
            })
            .collect(),
        Some(_) => Err(LibraryError::validation(key, "must be an array")),
    }
}

fn iri_list(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<Vec<NamedNode>> {
    let mut out = Vec::new();
    for text in string_list(object, key)? {
        if !(text.starts_with("http://") || text.starts_with("https://")) {
            return Err(LibraryError::validation(
                key,
                format!("{text:?} is not an IRI"),
            ));
        }
        out.push(NamedNode::new_unchecked(text));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::LibraryEntry;

    const TECHNIQUE: &str = r#"{"kind":"Technique","slug":"cache-negative-results","title":"Cache negative results","purpose":"Stop paying twice for a lookup that fails.","domains":["software"],"applicable_when":["the same external lookup repeats"],"evidence":["https://metamind.dev/library/case/api-latency-misattribution"],"examples":["https://metamind.dev/library/case/api-latency-misattribution"],"confidence":0.55}"#;

    #[test]
    fn a_valid_line_becomes_an_entry_whose_iri_names_its_kind() {
        let entry = parse_line(TECHNIQUE).unwrap();
        assert_eq!(entry.kind, EntryKind::Technique);
        assert_eq!(
            entry.version_iri().as_str(),
            "https://metamind.dev/library/technique/cache-negative-results@1"
        );
        assert_eq!(
            entry.head_iri().as_str(),
            "https://metamind.dev/library/technique/cache-negative-results"
        );
        assert_eq!(entry.applicable_when.len(), 1);
        assert_eq!(entry.evidence.len(), 1);
        assert_eq!(entry.examples.len(), 1);
        assert_eq!(entry.confidence, Some(0.55));
        assert_eq!(entry.domain, vec!["software".to_string()]);
    }

    #[test]
    fn an_unknown_key_is_a_typed_error_that_names_it() {
        let line = r#"{"kind":"Principle","slug":"p","title":"T","purpose":"P","surprise":1}"#;
        let error = parse_line(line).unwrap_err();
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("surprise"), "{error}");
    }

    #[test]
    fn a_non_declarative_kind_is_refused() {
        let line = r#"{"kind":"Frame","slug":"f","title":"T","purpose":"P"}"#;
        let error = parse_line(line).unwrap_err();
        assert!(error.to_string().contains("Frame"), "{error}");
    }

    #[test]
    fn a_missing_required_key_is_refused_by_name() {
        let line = r#"{"kind":"Technique","slug":"t","purpose":"P"}"#;
        let error = parse_line(line).unwrap_err();
        assert!(error.to_string().contains("title"), "{error}");
    }

    #[test]
    fn a_bad_slug_is_refused() {
        let line = r#"{"kind":"Technique","slug":"Not A Slug","title":"T","purpose":"P"}"#;
        assert!(parse_line(line).is_err());
    }

    #[test]
    fn an_anti_pattern_may_carry_its_corrective_rule() {
        let line = r#"{"kind":"AntiPattern","slug":"retry-without-backoff","title":"Retry without backoff","purpose":"Retrying a saturated dependency.","evidence":["https://metamind.dev/library/case/outage-rollback"],"corrective_rule":"add jittered backoff"}"#;
        let entry = parse_line(line).unwrap();
        assert_eq!(
            entry.corrective_rule.as_deref(),
            Some("add jittered backoff")
        );
        entry.validate().unwrap();
    }

    #[test]
    fn a_bad_line_is_ledgered_with_its_line_number_and_the_good_lines_still_parse() {
        let document = format!("{TECHNIQUE}\nnot json at all\n{TECHNIQUE}");
        let (accepted, rejected) = extract_candidates(&document);
        assert_eq!(accepted.len(), 2);
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].line, 2);
        assert!(
            rejected[0].reason.contains("json"),
            "{}",
            rejected[0].reason
        );
    }

    #[test]
    fn blank_lines_are_not_candidates() {
        let document = format!("\n{TECHNIQUE}\n\n");
        let (accepted, rejected) = extract_candidates(&document);
        assert_eq!(accepted.len(), 1);
        assert!(rejected.is_empty());
    }

    #[test]
    fn two_runs_produce_identical_ledgers() {
        let document = format!("{TECHNIQUE}\n{{\"kind\":\"Nope\"}}\n");
        let first = extract_candidates(&document);
        let second = extract_candidates(&document);
        assert_eq!(first, second);
    }

    #[test]
    fn the_accepted_rendering_is_one_json_object_per_line() {
        let entry = parse_line(TECHNIQUE).unwrap();
        let rendered = accepted_line(&entry);
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["kind"], "Technique");
        assert_eq!(value["slug"], "cache-negative-results");
        assert!(!rendered.contains('\n'));
    }

    #[test]
    fn every_extractable_kind_is_declarative() {
        assert!(!extractable_kinds().is_empty());
        assert!(extractable_kinds().iter().all(|kind| kind.is_declarative()));
    }
}
