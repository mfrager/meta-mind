//! Mistakes and near misses.
//!
//! A mistake is not a failed episode; it is a *lesson with a rule attached*. The
//! plan's requirement is concrete: a seeded mistake must be retrievable and must
//! name a corrective rule. That pushes two decisions into this module:
//!
//! * A mistake is stored as an ordinary `memories` row of kind `mistake`, so the
//!   hybrid retrieval, the retention curve, and the mirror treat it exactly like
//!   any other memory. A separate store would be a second retrieval path to keep
//!   correct.
//! * Its corrective rule is **always present**. When a caller does not supply one,
//!   the rule's ULID is derived from the failure mode, so `mm:correctiveRule`
//!   always resolves and a mistake can never be stored without a lesson.
//!
//! Mistakes carry high confidence and high importance on purpose: they are the
//! records an Ebbinghaus curve should keep longest.

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use serde_json::json;

use crate::error::Result;
use crate::model::{ref_ulid, LinkKind, Memory, MemoryKind, NearMiss, RetrievalCue, TimeInterval};
use crate::store::SqliteMemoryStore;

/// Everything needed to record a mistake.
#[derive(Debug, Clone, PartialEq)]
pub struct MistakeInput {
    /// The memory's ULID; the nil ULID asks the store to mint one.
    pub memory_id: Ulid,
    /// The situation it happened in.
    pub situation: Option<Ulid>,
    /// The decision that led to it.
    pub decision: Option<Ulid>,
    /// The observed outcome.
    pub outcome: Option<Ulid>,
    /// What kind of failure it was.
    pub failure_mode: String,
    /// The root cause.
    pub root_cause: Option<Ulid>,
    /// Signals that were available and missed.
    pub missed_signal: Vec<String>,
    /// The rule that prevents recurrence; derived when absent.
    pub corrective_rule: Option<Ulid>,
    /// The text of the corrective rule.
    pub corrective_rule_text: Option<String>,
    /// How likely it is to happen again, in `[0,1]`.
    pub recurrence_risk: f32,
    /// Override the memory content.
    pub content: Option<String>,
    /// The instant to record it at.
    pub now: Option<Timestamp>,
}

impl MistakeInput {
    /// A mistake with just a failure mode.
    pub fn new(failure_mode: impl Into<String>) -> Self {
        MistakeInput {
            memory_id: Ulid::nil(),
            situation: None,
            decision: None,
            outcome: None,
            failure_mode: failure_mode.into(),
            root_cause: None,
            missed_signal: Vec::new(),
            corrective_rule: None,
            corrective_rule_text: None,
            recurrence_risk: 0.5,
            content: None,
            now: None,
        }
    }

    /// Set the corrective rule's text.
    pub fn with_rule(mut self, text: impl Into<String>) -> Self {
        self.corrective_rule_text = Some(text.into());
        self
    }

    /// Set the recurrence risk.
    pub fn with_risk(mut self, risk: f32) -> Self {
        self.recurrence_risk = risk;
        self
    }

    /// Set the missed signals.
    pub fn with_missed(mut self, signals: Vec<String>) -> Self {
        self.missed_signal = signals;
        self
    }

    /// The rule text, defaulting to a statement about the failure mode.
    fn rule_text(&self) -> String {
        self.corrective_rule_text
            .clone()
            .unwrap_or_else(|| format!("avoid recurring {}", self.failure_mode))
    }
}

/// Record a mistake, returning the memory that holds it.
pub async fn record_mistake(
    store: &SqliteMemoryStore,
    logger: &Logger,
    ids: &UlidFactory,
    input: &MistakeInput,
) -> Result<Ulid> {
    let now = input.now.unwrap_or_else(Timestamp::now);
    let memory_id = if input.memory_id.is_nil() {
        ids.next()
    } else {
        input.memory_id
    };
    let rule_text = input.rule_text();
    let rule_id = input
        .corrective_rule
        .unwrap_or_else(|| ref_ulid(&format!("rule:{}", rule_text.trim().to_lowercase()), now));

    let content = input.content.clone().unwrap_or_else(|| {
        format!(
            "Mistake: {}. Root cause: {}. Missed signals: {}. Corrective rule: {}.",
            input.failure_mode,
            input
                .root_cause
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_else(|| "unrecorded".to_string()),
            if input.missed_signal.is_empty() {
                "none recorded".to_string()
            } else {
                input.missed_signal.join(", ")
            },
            rule_text
        )
    });

    let mut memory = Memory::new(
        memory_id,
        MemoryKind::Mistake,
        content,
        TimeInterval::open(now),
        0.9,
        0.95,
        ids.next(),
        now,
    )?;
    memory
        .cues
        .push(RetrievalCue::keyword(input.failure_mode.clone()));
    for signal in &input.missed_signal {
        memory.cues.push(RetrievalCue::keyword(signal.clone()));
    }
    for token in crate::store::tokenize(&input.failure_mode)
        .into_iter()
        .take(8)
    {
        memory.cues.push(RetrievalCue::keyword(token));
    }
    store.insert(&memory).await?;

    store
        .insert_mistake(
            ids.next(),
            memory_id,
            input.situation,
            input.decision,
            input.outcome,
            &input.failure_mode,
            input.root_cause,
            &input.missed_signal,
            Some(rule_id),
            input.recurrence_risk,
        )
        .await?;

    // The rule is reachable from the memory, so `retrieve the mistake and name its
    // rule` is one link lookup.
    store
        .insert_link(memory_id, rule_id, LinkKind::Supports, 1.0)
        .await?;

    logger
        .audit(
            Level::Warn,
            codes::MEMORY_MISTAKE_CREATE,
            crate::TARGET,
            Some(memory_id),
            json!({
                "mistake_id": mm_core::ulid_string(&memory_id),
                "failure_mode": input.failure_mode,
                "root_cause": input
                    .root_cause
                    .map(|id| mm_core::ulid_string(&id))
                    .unwrap_or_else(|| "-".to_string()),
                "corrective_rule": mm_core::ulid_string(&rule_id),
                "recurrence_risk": input.recurrence_risk,
            }),
        )
        .await?;
    Ok(memory_id)
}

/// Record a near miss: the same shape, without the harm.
pub async fn record_near_miss(
    store: &SqliteMemoryStore,
    logger: &Logger,
    ids: &UlidFactory,
    input: &NearMiss,
) -> Result<Ulid> {
    let now = Timestamp::now();
    let memory_id = if input.memory_id.is_nil() {
        ids.next()
    } else {
        input.memory_id
    };
    let near = NearMiss {
        memory_id,
        failure_mode: input.failure_mode.clone(),
        missed_signal: input.missed_signal.clone(),
    };
    let memory = crate::consolidate::near_miss_memory(ids, &near, now)?;
    store.insert(&memory).await?;
    logger
        .audit(
            Level::Info,
            codes::MEMORY_NEAR_MISS_CREATE,
            crate::TARGET,
            Some(memory_id),
            json!({
                "near_miss_id": mm_core::ulid_string(&memory_id),
                "failure_mode": input.failure_mode,
                "missed_signal": input.missed_signal,
            }),
        )
        .await?;
    Ok(memory_id)
}

/// Every stored mistake detail row.
pub async fn mistake_rows(store: &SqliteMemoryStore) -> Result<Vec<crate::model::MistakeRecord>> {
    store.list_mistakes().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_text_is_derived_when_absent() {
        let input = MistakeInput::new("merged without running the tests");
        assert_eq!(
            input.rule_text(),
            "avoid recurring merged without running the tests"
        );
        let input = MistakeInput::new("x").with_rule("always run the full suite");
        assert_eq!(input.rule_text(), "always run the full suite");
    }

    #[test]
    fn the_derived_rule_ulid_is_stable_and_content_sensitive() {
        let at = Timestamp::from_epoch_seconds(1_700_000_000);
        let a = ref_ulid("rule:always run the suite", at);
        let b = ref_ulid("rule:always run the suite", at);
        let c = ref_ulid("rule:never push on friday", at);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn a_risk_outside_the_unit_interval_is_visible_to_validation() {
        let input = MistakeInput::new("x").with_risk(1.5);
        assert!(input.recurrence_risk > 1.0);
    }
}
