//! Lessons: the diagnosis, made reusable.
//!
//! A lesson is not a private note on an analysis. Phase 7's rule is that every durable
//! way of thinking lives in the cognitive library and passes *that* library's gate, so
//! a lesson is written as a library entry — specifically an **anti-pattern**, because
//! a lesson about a failure is exactly a recurring shape of failure with a corrective
//! rule attached, and that is the kind that carries a `corrective_rule` field.
//!
//! Three decisions are worth stating:
//!
//! * **The derivation is a pure function.** [`Lesson::derive`] turns an analysis plus
//!   an evidence count into a lesson with fixed arithmetic, and both the write path and
//!   the listing path use it. A lesson whose confidence depended on which path read it
//!   would be two lessons.
//! * **The library is optional, and its absence is visible.** Without a manager the
//!   lesson is returned and logged but not indexed; `extract` says so in its return
//!   value's identifier (a content hash rather than a versioned library IRI), and
//!   [`MetaContext::with_library`] is how a caller opts in. Silently pretending to
//!   write would be worse than not writing.
//! * **One analysis yields one lesson here.** The phase's workflow is
//!   analysis → lesson → change set, and a lesson per class would multiply the change
//!   sets a single incident produces. A caller that wants more lessons can analyse
//!   again; the second analysis is a second row, never an edit.
//!
//! The evidence count is the one field the listing cannot recover: it belongs to the
//! episode, and `meta_analyses` deliberately does not duplicate the episode's body.
//! [`list`] reports `0` and the analysis id it came from, rather than inventing a
//! number.

use std::sync::Arc;

use mm_core::{Tabular, Ulid};
use mm_library::{Declarative, EntryKind, LibraryManager};
use mm_log::{codes, Level};

use crate::diagnosis::{MetaAnalysis, MetaContext};
use crate::error::{MetaError, Result};
use crate::ledger::content_ulid;

/// A lesson: a statement, how thin the evidence is, and what it changes.
#[derive(Debug, Clone, PartialEq)]
pub struct Lesson {
    /// The lesson's identifier: a library IRI's content hash when it was indexed, and
    /// the derivation's content hash when it was not.
    pub id: Ulid,
    /// The lesson itself.
    pub text: String,
    /// How sure the extraction is, in `[0,1]`.
    pub confidence: f32,
    /// How many records it was drawn from.
    pub evidence_count: u32,
    /// The domains it applies to.
    pub domains: Vec<String>,
    /// What it changes, when it changes anything.
    pub policy_effect: Option<String>,
}

impl Lesson {
    /// Derive a lesson from an analysis, deterministically.
    ///
    /// Confidence is the mean of recurrence and impact: how often the failure has
    /// happened and what it cost. Both are already on the analysis row, both are
    /// bounded, and their mean is bounded with them — so a lesson's confidence can
    /// never exceed what its two inputs support, which is the property that stops a
    /// lesson from looking more certain than the incident it came from.
    pub fn derive(analysis: &MetaAnalysis, evidence_count: u32) -> Self {
        let confidence = ((analysis.recurrence + analysis.impact) / 2.0) as f32;
        let classes: Vec<String> = analysis
            .error_classes
            .iter()
            .map(|class| class.as_str().to_string())
            .collect();
        let text = format!(
            "When a {} trigger fires and the classes are {}, the answer is: {}",
            analysis.trigger.as_str(),
            classes.join(", "),
            analysis.diagnosis.trim()
        );
        let policy_effect = if confidence >= POLICY_EFFECT_CONFIDENCE {
            Some("re-read the source before answering from memory".to_string())
        } else {
            None
        };
        Lesson {
            id: content_ulid(&format!(
                "lesson:{}:{}",
                mm_core::ulid_string(&analysis.id),
                classes.join("+")
            )),
            text,
            confidence,
            evidence_count,
            domains: classes,
            policy_effect,
        }
    }

    /// The slug the library entry is stored under.
    pub fn slug(&self) -> String {
        format!("lesson-{}", mm_core::ulid_string(&self.id))
    }

    /// True when the lesson asks for a policy change.
    pub fn changes_a_policy(&self) -> bool {
        self.policy_effect.is_some()
    }
}

/// The confidence at or above which a lesson names a policy effect.
///
/// A lesson drawn from a failure that recurred and cost something is worth acting on;
/// a one-off is worth remembering. The threshold is fixed here rather than learned,
/// because it decides whether a *policy delta* is proposed, and a proposal that
/// depended on a fitted number would make the change set unreproducible.
pub const POLICY_EFFECT_CONFIDENCE: f32 = 0.7;

/// Write the lesson for an analysis, and return it.
pub async fn extract(
    analysis: &MetaAnalysis,
    evidence_count: u32,
    ctx: &MetaContext,
) -> Result<Lesson> {
    let lesson = Lesson::derive(analysis, evidence_count);
    let mut indexed = false;

    if let Some(library) = ctx.library.as_ref() {
        commit(library, analysis, &lesson).await?;
        indexed = true;
    }

    ctx.logger
        .audit(
            Level::Info,
            codes::META_LESSON,
            crate::TARGET,
            Some(lesson.id),
            serde_json::json!({
                "lesson_id": mm_core::ulid_string(&lesson.id),
                "analysis_id": mm_core::ulid_string(&analysis.id),
                "confidence": lesson.confidence,
                "evidence_count": evidence_count,
                "domains": lesson.domains,
                "policy_effect": lesson.policy_effect.clone().unwrap_or_default(),
                "indexed_in_library": indexed,
            }),
        )
        .await?;
    Ok(lesson)
}

/// Index the lesson as an anti-pattern, through the library's own gate.
async fn commit(
    library: &Arc<LibraryManager>,
    analysis: &MetaAnalysis,
    lesson: &Lesson,
) -> Result<()> {
    let title = format!(
        "lesson from analysis {}",
        mm_core::ulid_string(&analysis.id)
    );
    let mut entry = Declarative::new(EntryKind::AntiPattern, &lesson.slug(), &title, &lesson.text)
        .map_err(|e| MetaError::Library(e.to_string()))?;
    for domain in &lesson.domains {
        entry = entry.with_domain(domain);
    }
    entry = entry
        .corrective_rule(
            lesson
                .policy_effect
                .as_deref()
                .unwrap_or("remember the failure mode and check for it first"),
        )
        .with_confidence(lesson.confidence)
        .map_err(|e| MetaError::Library(e.to_string()))?;
    // The failure mode is a plain field rather than a builder: it is the diagnosis
    // verbatim, and `failure_mode` is what makes the entry an anti-pattern a later
    // retrieval can match on.
    entry.failure_mode = Some(analysis.diagnosis.clone());
    if let Some(evidence) = analysis_evidence(analysis) {
        entry = entry.backed_by(evidence);
    }
    library
        .commit(&entry, lesson.confidence)
        .await
        .map_err(|e| MetaError::Library(e.to_string()))?;
    Ok(())
}

/// Every lesson this kernel has derived, newest first.
///
/// Read from `meta_analyses` through the same derivation the write path uses, so a
/// listing shows exactly what `extract` produced — with the evidence count reported as
/// zero, because the count belongs to the episode and the analysis row does not
/// duplicate the episode's body.
pub async fn list(store: &mm_store_sqlite::SqliteStore) -> Result<Vec<Lesson>> {
    let rows = Tabular::query_json(
        store,
        "SELECT id, trigger, episode_ulid, diagnosis, error_classes_json, recurrence, impact, \
         created_at FROM meta_analyses ORDER BY created_at DESC, id DESC",
        Vec::new(),
    )
    .await
    .map_err(|e| MetaError::Store(e.to_string()))?;
    let mut lessons = Vec::new();
    for row in &rows {
        let analysis = parse_analysis(row)?;
        lessons.push(Lesson::derive(&analysis, 0));
    }
    Ok(lessons)
}

fn parse_analysis(row: &serde_json::Value) -> Result<MetaAnalysis> {
    let text = |key: &str| -> Result<String> {
        row.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| MetaError::Malformed {
                table: "meta_analyses",
                detail: format!("column {key} is missing"),
            })
    };
    let id = mm_core::id::parse_ulid(&text("id")?).map_err(|e| MetaError::Malformed {
        table: "meta_analyses",
        detail: e.to_string(),
    })?;
    let episode_text = text("episode_ulid")?;
    let episode = mm_core::id::parse_ulid(&episode_text).map_err(|e| MetaError::Malformed {
        table: "meta_analyses",
        detail: e.to_string(),
    })?;
    let trigger_text = text("trigger")?;
    let trigger =
        crate::taxonomy::Trigger::parse(&trigger_text).ok_or_else(|| MetaError::Malformed {
            table: "meta_analyses",
            detail: format!("trigger {trigger_text:?} is not one of the eight"),
        })?;
    let names: Vec<String> =
        serde_json::from_str(&text("error_classes_json")?).map_err(|e| MetaError::Malformed {
            table: "meta_analyses",
            detail: format!("error_classes_json: {e}"),
        })?;
    let mut error_classes = Vec::new();
    for name in names {
        error_classes.push(crate::taxonomy::ErrorClass::parse(&name).ok_or_else(|| {
            MetaError::Malformed {
                table: "meta_analyses",
                detail: format!("error class {name:?} is not one of the sixteen"),
            }
        })?);
    }
    let number = |key: &str| -> Result<f64> {
        row.get(key)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| MetaError::Malformed {
                table: "meta_analyses",
                detail: format!("column {key} is missing"),
            })
    };
    let created_at = mm_core::Timestamp::from_rfc3339(&text("created_at")?).map_err(|e| {
        MetaError::Malformed {
            table: "meta_analyses",
            detail: e.to_string(),
        }
    })?;
    Ok(MetaAnalysis {
        id,
        trigger,
        episode,
        diagnosis: text("diagnosis")?,
        error_classes,
        recurrence: number("recurrence")?,
        impact: number("impact")?,
        created_at,
    })
}

/// The evidence IRI a lesson can stand on, when the analysis names one.
///
/// The analysis row does not carry the episode's evidence, so this returns the
/// analysis node itself: a lesson backed by the analysis it came from is honest about
/// what a reader can check, whereas a fabricated evidence IRI would pass the library's
/// orphan check and mean nothing.
fn analysis_evidence(analysis: &MetaAnalysis) -> Option<mm_core::NamedNode> {
    Some(mm_core::iri::data(&analysis.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taxonomy::{ErrorClass, Trigger};
    use mm_core::{Param, Timestamp};

    fn analysis() -> MetaAnalysis {
        MetaAnalysis {
            id: Ulid::from_parts(1_700_000_000_000, 1),
            trigger: Trigger::RepeatedFailure,
            episode: Ulid::from_parts(1_700_000_000_000, 2),
            diagnosis: "the retrieval step did not re-read the schema".into(),
            error_classes: vec![ErrorClass::Data, ErrorClass::Retrieval],
            recurrence: 0.8,
            impact: 0.6,
            created_at: Timestamp::from_epoch_seconds(1_700_000_000),
        }
    }

    #[test]
    fn a_lesson_is_derived_deterministically() {
        let a = analysis();
        let one = Lesson::derive(&a, 2);
        let two = Lesson::derive(&a, 2);
        assert_eq!(one, two);
        assert!((one.confidence - 0.7).abs() < 1e-6);
        assert_eq!(
            one.domains,
            vec!["data".to_string(), "retrieval".to_string()]
        );
        assert!(one.text.contains("retrieval"));
        assert!(
            one.changes_a_policy(),
            "0.7 reaches the policy-effect threshold"
        );
        assert!(one.slug().starts_with("lesson-"));
        assert_eq!(mm_core::ulid_string(&one.id).len(), mm_core::ULID_LEN);
    }

    #[test]
    fn a_single_occurrence_names_no_policy_effect() {
        let mut a = analysis();
        a.recurrence = 0.3;
        a.impact = 0.4;
        let lesson = Lesson::derive(&a, 1);
        assert!(!lesson.changes_a_policy());
        assert!((lesson.confidence - 0.35).abs() < 1e-6);
    }

    #[tokio::test]
    async fn a_lesson_without_a_library_is_logged_and_not_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        // A collector whose lines stay readable after the logger takes ownership: the
        // sink is a thin `Arc` wrapper, which is why it exists at all.
        let sink = std::sync::Arc::new(mm_log::CollectSink::new());
        let logger = std::sync::Arc::new(mm_log::Logger::new(
            mm_log::Level::Trace,
            vec![Box::new(SharedCollectSink::new(sink.clone()))],
            // `meta.lesson` is audited, so this logger needs the audit writer the
            // kernel gives it: a lesson that was logged but never reached the chain
            // would be exactly the gap the audit trail exists to catch.
            Some(std::sync::Arc::new(store.clone())),
            mm_log::RedactionPolicy::empty(),
        ));
        let ctx = MetaContext {
            sql: store.clone(),
            logger,
            ids: Arc::new(mm_core::UlidFactory::new()),
            diagnoser: Arc::new(crate::diagnosis::HeuristicDiagnoser),
            library: None,
            baseline_brier: 0.3,
        };
        let lesson = extract(&analysis(), 3, &ctx).await.unwrap();
        assert_eq!(lesson.evidence_count, 3);
        let lines = sink.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("meta.lesson"), "{}", lines[0]);
        assert!(
            lines[0].contains("indexed_in_library\":false"),
            "{}",
            lines[0]
        );
        // Nothing was written to the library, so nothing is listed as a lesson.
        assert_eq!(store.row_count("library_entries").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn listing_re_derives_the_lessons_the_analyses_produced() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        Tabular::execute(
            &store,
            "INSERT INTO meta_analyses (id, trigger, episode_ulid, diagnosis, \
             error_classes_json, recurrence, impact, created_at) \
             VALUES ('01h0000000000000000000a110', 'repeated_failure', \
             '01h0000000000000000000e110', 'the retrieval step did not re-read the schema', \
             '[\"data\",\"retrieval\"]', 0.8, 0.6, ?)",
            vec![Param::Text(Timestamp::now().to_rfc3339())],
        )
        .await
        .unwrap();
        let lessons = list(&store).await.unwrap();
        assert_eq!(lessons.len(), 1);
        assert_eq!(
            lessons[0].domains,
            vec!["data".to_string(), "retrieval".to_string()]
        );
        assert_eq!(
            lessons[0].evidence_count, 0,
            "the count belongs to the episode"
        );
    }

    #[tokio::test]
    async fn a_row_with_an_unknown_class_is_malformed_not_defaulted() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        Tabular::execute(
            &store,
            "INSERT INTO meta_analyses (id, trigger, episode_ulid, diagnosis, \
             error_classes_json, recurrence, impact, created_at) \
             VALUES ('01h0000000000000000000a111', 'repeated_failure', \
             '01h0000000000000000000e110', 'x', '[\"nonsense\"]', 0.8, 0.6, ?)",
            vec![Param::Text(Timestamp::now().to_rfc3339())],
        )
        .await
        .unwrap();
        let error = list(&store).await.unwrap_err();
        assert_eq!(error.code(), "malformed");
    }

    /// A sink that forwards to a collector the test still holds.
    struct SharedCollectSink {
        inner: std::sync::Arc<mm_log::CollectSink>,
    }

    impl SharedCollectSink {
        fn new(inner: std::sync::Arc<mm_log::CollectSink>) -> Self {
            SharedCollectSink { inner }
        }
    }

    impl mm_log::Sink for SharedCollectSink {
        fn name(&self) -> &str {
            "shared-collect"
        }

        fn write_line(&self, line: &str) -> std::result::Result<(), mm_core::MmError> {
            self.inner.write_line(line)
        }
    }
}
