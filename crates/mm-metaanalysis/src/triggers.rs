//! The triggers: why anyone looked.
//!
//! Phase 11 is explicit that meta-analysis is **event-triggered, never scheduled**. A
//! periodic introspection pass spends attention whether or not anything happened, and
//! the plan's risk table names "runaway introspection" as the failure mode it would
//! cause. So this module answers one question — *which of the eight triggers have
//! fired since an instant* — by reading records that were written for other reasons.
//!
//! Each trigger has a different source, and the source is the argument for it being a
//! trigger rather than an opinion:
//!
//! | Trigger | Source |
//! |---|---|
//! | `prediction_error` | a prediction whose horizon has passed and which is still unresolved |
//! | `user_correction` | an event whose payload says a correction was made |
//! | `repeated_failure` | two or more `mistake` memories in the window |
//! | `contradiction` | a row in the contradiction table |
//! | `unexpected_outcome` | a resolution that contradicts its staked probability |
//! | `goal_failure` | an event whose payload says a goal failed |
//! | `near_miss` | a `near_miss` memory in the window |
//! | `novel_success` | an event whose payload says a success had no precedent |
//!
//! Two properties matter more than the list. **The scan is deterministic**: every
//! source is a `SELECT count(*)` or a bounded read, and the result is sorted by
//! `(priority, episode)` so two runs over the same state fire in the same order.
//! **An unobserved trigger is absent, not zero-valued**: `fire` returns only what
//! happened, and a caller that sees an empty vector has learned that nothing needs
//! attention, which is different from having learned nothing.
//!
//! `novel_success` is the one trigger whose source does not exist yet in this build:
//! nothing writes `novel_success` events. It is kept in the vocabulary, and its query
//! is written, because the trigger's absence is itself information — an operator
//! asking "has anything worked that never worked before" gets a true answer.

use std::path::Path;

use mm_core::{Param, Params, Tabular, Timestamp, Ulid};
use mm_log::{codes, Level, LogRecord, Logger};

use crate::error::{MetaError, Result};
use crate::taxonomy::{MetaAnalysisPriority, Trigger};

/// One trigger that fired, with the episode it fired on and the priority it earned.
#[derive(Debug, Clone, PartialEq)]
pub struct MetaTrigger {
    /// Which trigger fired.
    pub trigger: Trigger,
    /// The episode it fired on. The fixture's episode, or a content-addressed stand-in
    /// when the source is a table row with no episode of its own.
    pub episode: Ulid,
    /// The five reasons to look, as they were computed.
    pub priority: MetaAnalysisPriority,
    /// When the trigger was observed.
    pub at: Timestamp,
}

impl MetaTrigger {
    /// The priority's total, which is what the queue is ordered by.
    pub fn score(&self) -> f64 {
        self.priority.total()
    }
}

/// An episode under analysis, as `bench/episodes/seeded_failure_01.json` describes it.
///
/// A context is *evidence about one failure*, not a live session: it carries the goal,
/// what actually happened, the attempts, the evidence ULIDs and the conditions. The
/// diagnoser reads it; the lesson reads it for its evidence count.
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeContext {
    /// The episode's identifier.
    pub episode_id: Ulid,
    /// The trigger that put it in the queue.
    pub trigger: Trigger,
    /// What was being attempted.
    pub goal: String,
    /// What went wrong, in prose. The cue table reads this text.
    pub failure: String,
    /// How many attempts it took.
    pub attempts: u32,
    /// The evidence the episode stands on.
    pub evidence: Vec<Ulid>,
    /// The conditions it happened under.
    pub conditions: Vec<String>,
    /// The five components of its priority.
    pub priority: MetaAnalysisPriority,
    /// The error classes the fixture says must be among those diagnosed.
    pub expected_classes: Vec<String>,
    /// How many lessons the fixture says the analysis must yield.
    pub expected_lessons: u32,
}

impl EpisodeContext {
    /// The text a diagnoser classifies: the goal and the failure together, because a
    /// failure's class depends on what was being attempted.
    pub fn classifier_text(&self) -> String {
        format!("{} {}", self.goal, self.failure)
    }
}

/// Read an episode fixture.
///
/// The document is the gate's, so it is parsed strictly: a missing field is a refusal
/// naming the field rather than a default, because a fixture that silently defaulted
/// its trigger would exercise a path the gate does not.
pub async fn load_episode(path: &Path) -> Result<EpisodeContext> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| MetaError::Record(format!("{} is not an episode: {e}", path.display())))?;
    let get_str = |key: &str| -> Result<String> {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| MetaError::Record(format!("{}: no {key}", path.display())))
    };
    let episode_id = mm_core::id::parse_ulid(&get_str("episode_id")?)
        .map_err(|e| MetaError::Record(format!("{}: episode_id: {e}", path.display())))?;
    let trigger_text = get_str("trigger")?;
    let trigger = Trigger::parse(&trigger_text).ok_or_else(|| {
        MetaError::Record(format!(
            "{}: trigger {trigger_text:?} is not one of the eight",
            path.display()
        ))
    })?;
    let priority_value = value
        .get("priority")
        .ok_or_else(|| MetaError::Record(format!("{}: no priority", path.display())))?;
    let component = |name: &str| -> Result<f64> {
        priority_value
            .get(name)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| MetaError::Record(format!("{}: priority has no {name}", path.display())))
    };
    let priority = MetaAnalysisPriority::new(
        component("novelty")?,
        component("failure")?,
        component("impact")?,
        component("recurrence")?,
        component("uncertainty")?,
    )?;
    let list = |key: &str| -> Result<Vec<String>> {
        Ok(match value.get(key) {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect(),
            Some(_) => {
                return Err(MetaError::Record(format!(
                    "{}: {key} is not an array",
                    path.display()
                )))
            }
        })
    };
    let mut evidence = Vec::new();
    for text in list("evidence")? {
        evidence.push(
            mm_core::id::parse_ulid(&text)
                .map_err(|e| MetaError::Record(format!("{}: evidence: {e}", path.display())))?,
        );
    }
    let expected = value.get("expected");
    let expected_classes: Vec<String> = expected
        .and_then(|e| e.get("error_classes_must_include"))
        .and_then(serde_json::Value::as_str)
        .map(|class| vec![class.to_string()])
        .unwrap_or_default();
    let expected_lessons = expected
        .and_then(|e| e.get("lessons_min"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;

    Ok(EpisodeContext {
        episode_id,
        trigger,
        goal: get_str("goal")?,
        failure: get_str("failure")?,
        attempts: value
            .get("attempts")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32,
        evidence,
        conditions: list("conditions")?,
        priority,
        expected_classes,
        expected_lessons,
    })
}

/// Which triggers have fired since `since`.
///
/// The order is `(priority descending, episode)` so the highest-value analysis is
/// first, and the sort is total: two runs over the same state produce the same vector.
pub async fn fire(
    store: &mm_store_sqlite::SqliteStore,
    logger: &Logger,
    now: Timestamp,
) -> Result<Vec<MetaTrigger>> {
    let since = now.to_rfc3339();
    let mut fired: Vec<MetaTrigger> = Vec::new();

    for (trigger, count) in [
        (
            Trigger::PredictionError,
            count(
                store,
                PREDICTION_ERROR_SQL,
                vec![Param::Text(since.clone())],
            )
            .await?,
        ),
        (
            Trigger::UserCorrection,
            count(store, USER_CORRECTION_SQL, vec![Param::Text(since.clone())]).await?,
        ),
        (
            Trigger::RepeatedFailure,
            count(
                store,
                REPEATED_FAILURE_SQL,
                vec![Param::Text(since.clone())],
            )
            .await?,
        ),
        (
            Trigger::Contradiction,
            count(store, CONTRADICTION_SQL, vec![]).await?,
        ),
        (
            Trigger::UnexpectedOutcome,
            count(store, UNEXPECTED_OUTCOME_SQL, vec![]).await?,
        ),
        (
            Trigger::GoalFailure,
            count(store, GOAL_FAILURE_SQL, vec![Param::Text(since.clone())]).await?,
        ),
        (
            Trigger::NearMiss,
            count(store, NEAR_MISS_SQL, vec![Param::Text(since.clone())]).await?,
        ),
        (
            Trigger::NovelSuccess,
            count(store, NOVEL_SUCCESS_SQL, vec![Param::Text(since.clone())]).await?,
        ),
    ] {
        if count == 0 {
            continue;
        }
        let priority = priority_for(trigger, count);
        let episode = crate::ledger::content_ulid(&format!(
            "trigger:{}:{}:{}",
            trigger.as_str(),
            count,
            since
        ));
        let entry = MetaTrigger {
            trigger,
            episode,
            priority,
            at: now,
        };
        logger
            .emit(
                LogRecord::new(Level::Info, codes::META_TRIGGER, crate::TARGET)
                    .with_field("trigger", trigger.as_str())
                    .with_field("episode_id", mm_core::ulid_string(&episode))
                    .with_field("occurrences", count)
                    .with_field("novelty", priority.novelty)
                    .with_field("failure", priority.failure)
                    .with_field("impact", priority.impact)
                    .with_field("recurrence", priority.recurrence)
                    .with_field("uncertainty", priority.uncertainty),
            )
            .await?;
        fired.push(entry);
    }

    fired.sort_by(|a, b| {
        b.score()
            .partial_cmp(&a.score())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.episode.cmp(&b.episode))
    });
    Ok(fired)
}

/// The priority a trigger earns from how often it fired.
///
/// The mapping is fixed arithmetic rather than a model's judgement, because the
/// priority orders a queue and a queue that reorders itself run to run cannot be
/// audited. `novelty` and `uncertainty` are constants per trigger class: a trigger
/// that fires at all is, by construction, a situation the system did not have a
/// settled response to.
fn priority_for(trigger: Trigger, occurrences: i64) -> MetaAnalysisPriority {
    let recurrence = match occurrences {
        0 => 0.0,
        1 => 0.3,
        2 => 0.5,
        3..=5 => 0.7,
        _ => 0.9,
    };
    let (novelty, failure, impact, uncertainty) = match trigger {
        Trigger::PredictionError => (0.4, 0.7, 0.5, 0.8),
        Trigger::UserCorrection => (0.5, 0.6, 0.7, 0.5),
        Trigger::RepeatedFailure => (0.2, 0.9, 0.8, 0.4),
        Trigger::Contradiction => (0.6, 0.5, 0.6, 0.9),
        Trigger::UnexpectedOutcome => (0.5, 0.5, 0.6, 0.9),
        Trigger::GoalFailure => (0.3, 0.8, 0.8, 0.6),
        Trigger::NearMiss => (0.4, 0.3, 0.5, 0.7),
        Trigger::NovelSuccess => (0.9, 0.0, 0.4, 0.6),
    };
    MetaAnalysisPriority {
        novelty,
        failure,
        impact,
        recurrence,
        uncertainty,
    }
}

async fn count(store: &mm_store_sqlite::SqliteStore, sql: &str, args: Params) -> Result<i64> {
    let rows = Tabular::query_json(store, sql, args)
        .await
        .map_err(|e| MetaError::Store(e.to_string()))?;
    Ok(rows
        .first()
        .and_then(|row| row.get("n"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0))
}

/// A prediction came due and is still unresolved. `created_at` and `horizon_seconds`
/// are the ledger's own columns, so "past due" is arithmetic on the record.
const PREDICTION_ERROR_SQL: &str = "SELECT count(*) AS n FROM prediction_ledger p \
     LEFT JOIN prediction_outcomes o ON o.prediction_ulid = p.id \
     WHERE o.prediction_ulid IS NULL \
       AND datetime(p.created_at, '+' || p.horizon_seconds || ' seconds') <= datetime(?)";

/// Someone said the answer was wrong. The record is written by whichever surface took
/// the correction; the payload names it, and the payload is the only thing this scan
/// reads.
const USER_CORRECTION_SQL: &str = "SELECT count(*) AS n FROM events \
     WHERE created_at >= ? AND payload LIKE '%user_correction%'";

/// Two or more mistakes recorded in the window. One mistake is a mistake; two of them
/// is a pattern, and the pattern is what a corrective change is worth making for.
const REPEATED_FAILURE_SQL: &str = "SELECT count(*) AS n FROM memories \
     WHERE kind = 'mistake' AND datetime(recorded_at / 1000000000, 'unixepoch') >= datetime(?)";

/// A contradiction exists. The row is Phase 6's: two claims that cannot both hold.
const CONTRADICTION_SQL: &str = "SELECT count(*) AS n FROM contradictions";

/// A resolution that contradicts the probability that was staked. A 0.7-or-more claim
/// that did not come true, or a 0.3-or-less claim that did, is the definition of an
/// outcome the system did not expect from its own numbers.
const UNEXPECTED_OUTCOME_SQL: &str = "SELECT count(*) AS n FROM prediction_outcomes o \
     JOIN prediction_ledger p ON p.id = o.prediction_ulid \
     WHERE (p.probability >= 0.7 AND o.observed = 0) OR (p.probability <= 0.3 AND o.observed = 1)";

/// A goal failed. Goals live in Phase 4's tables and their transitions are events.
const GOAL_FAILURE_SQL: &str = "SELECT count(*) AS n FROM events \
     WHERE created_at >= ? AND payload LIKE '%goal%' AND payload LIKE '%fail%'";

/// A near miss was recorded. Nothing was harmed, and the shape of the failure was the
/// same one that would have been.
const NEAR_MISS_SQL: &str = "SELECT count(*) AS n FROM memories \
     WHERE kind = 'near_miss' AND datetime(recorded_at / 1000000000, 'unixepoch') >= datetime(?)";

/// Something worked that had never worked before. Nothing writes this record yet, so
/// the honest answer today is zero — and the query is here so that the day something
/// does write it, the trigger fires without a code change.
const NOVEL_SUCCESS_SQL: &str = "SELECT count(*) AS n FROM events \
     WHERE created_at >= ? AND payload LIKE '%novel_success%'";

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::{Param, Tabular};

    async fn store() -> (tempfile::TempDir, mm_store_sqlite::SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        (dir, store)
    }

    fn repo_path(relative: &str) -> std::path::PathBuf {
        mm_core::Config::repo_root().join(relative)
    }

    #[tokio::test]
    async fn the_seeded_episode_parses_exactly_as_written() {
        let ctx = load_episode(&repo_path("bench/episodes/seeded_failure_01.json"))
            .await
            .unwrap();
        assert_eq!(ctx.trigger, Trigger::RepeatedFailure);
        assert_eq!(ctx.attempts, 3);
        assert_eq!(ctx.evidence.len(), 2);
        assert_eq!(ctx.conditions.len(), 2);
        assert!((ctx.priority.recurrence - 0.8).abs() < 1e-12);
        assert!((ctx.priority.total() - 2.7).abs() < 1e-12);
        assert!(ctx.failure.contains("retrieval"));
        assert_eq!(ctx.expected_classes, vec!["retrieval".to_string()]);
        assert_eq!(ctx.expected_lessons, 1);
    }

    #[tokio::test]
    async fn a_fixture_with_no_trigger_is_refused_rather_than_defaulted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "{\"episode_id\":\"01h0000000000000000000e110\"}").unwrap();
        let error = load_episode(&path).await.unwrap_err();
        assert_eq!(error.code(), "record");
    }

    #[tokio::test]
    async fn an_untouched_kernel_fires_nothing() {
        let (_dir, store) = store().await;
        let fired = fire(&store, &crate::test_logger(&store), Timestamp::now())
            .await
            .unwrap();
        assert!(
            fired.is_empty(),
            "a fresh kernel has nothing to analyse: {fired:?}"
        );
    }

    #[tokio::test]
    async fn an_unresolved_prediction_past_its_horizon_fires_prediction_error() {
        let (_dir, store) = store().await;
        // A prediction staked at the epoch with a one-second horizon is long past due.
        Tabular::execute(
            &store,
            "INSERT INTO prediction_ledger (id, class, proposition, probability, \
             horizon_seconds, conditions_json, episode_ulid, created_at) \
             VALUES ('01h0000000000000000000p110', 'safety', 'x', 0.8, 1, '[]', NULL, ?)",
            vec![Param::Text(
                Timestamp::from_epoch_seconds(1_700_000_000).to_rfc3339(),
            )],
        )
        .await
        .unwrap();

        let fired = fire(&store, &crate::test_logger(&store), Timestamp::now())
            .await
            .unwrap();
        assert_eq!(fired.len(), 1, "{fired:?}");
        assert_eq!(fired[0].trigger, Trigger::PredictionError);
        assert!(fired[0].score() > 0.0);
    }

    #[tokio::test]
    async fn a_resolution_that_contradicts_its_probability_fires_unexpected_outcome() {
        let (_dir, store) = store().await;
        Tabular::execute(
            &store,
            "INSERT INTO prediction_ledger (id, class, proposition, probability, \
             horizon_seconds, conditions_json, episode_ulid, created_at) \
             VALUES ('01h0000000000000000000p111', 'safety', 'x', 0.9, 3600, '[]', NULL, ?)",
            vec![Param::Text(Timestamp::now().to_rfc3339())],
        )
        .await
        .unwrap();
        Tabular::execute(
            &store,
            "INSERT INTO prediction_outcomes (prediction_ulid, observed, resolved_at, \
             evidence_ulid) VALUES ('01h0000000000000000000p111', 0, ?, \
             '01h0000000000000000000e111')",
            vec![Param::Text(Timestamp::now().to_rfc3339())],
        )
        .await
        .unwrap();

        let fired = fire(&store, &crate::test_logger(&store), Timestamp::now())
            .await
            .unwrap();
        let kinds: Vec<Trigger> = fired.iter().map(|t| t.trigger).collect();
        assert!(kinds.contains(&Trigger::UnexpectedOutcome), "{kinds:?}");
    }

    #[test]
    fn the_priority_ordering_is_total_and_deterministic() {
        let a = MetaTrigger {
            trigger: Trigger::RepeatedFailure,
            episode: Ulid::from_parts(1, 1),
            priority: priority_for(Trigger::RepeatedFailure, 4),
            at: Timestamp::EPOCH,
        };
        let b = MetaTrigger {
            trigger: Trigger::NearMiss,
            episode: Ulid::from_parts(1, 2),
            priority: priority_for(Trigger::NearMiss, 1),
            at: Timestamp::EPOCH,
        };
        assert!(a.score() > b.score(), "{} vs {}", a.score(), b.score());
    }

    #[test]
    fn recurrence_rises_with_occurrences_and_is_bounded() {
        for (occurrences, expected) in [(1, 0.3), (2, 0.5), (4, 0.7), (40, 0.9)] {
            let p = priority_for(Trigger::RepeatedFailure, occurrences);
            assert!((p.recurrence - expected).abs() < 1e-12, "{occurrences}");
            assert!(p.validate().is_ok());
        }
    }
}
