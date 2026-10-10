//! Procedural memory: how to do something, and how well it is known.
//!
//! A procedure is stored as a `procedural` memory whose content is a canonical
//! JSON document. That is a deliberate choice: the DDL has no `procedures` table
//! (the plan's §4.1 is explicit about the table list), the structured detail still
//! needs to survive, and a memory whose content is canonical JSON is still
//! lexically searchable and still hashed like any other.
//!
//! **Habit promotion** is the point of tracking proficiency: a procedure that has
//! succeeded repeatedly at high proficiency is a habit, and habits are what a
//! later phase can stop deliberating about. The rule is deterministic — three
//! successes and a proficiency of 0.8 — so "is this a habit" has one answer.

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{MemoryError, Result};
use crate::model::{Memory, MemoryKind, RetrievalCue, TimeInterval};
use crate::store::SqliteMemoryStore;

/// The proficiency at which a proven procedure becomes a habit.
pub const HABIT_PROFICIENCY: f32 = 0.8;
/// The number of recorded successes a habit needs.
pub const HABIT_SUCCESSES: u32 = 3;

/// A skill record: preconditions, steps, outcomes, failure modes, proficiency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Procedure {
    /// The memory's ULID; the nil ULID asks the store to mint one.
    #[serde(default = "nil_ulid")]
    pub memory_id: Ulid,
    /// What the procedure is called.
    pub name: String,
    /// What must hold before it can be attempted.
    #[serde(default)]
    pub preconditions: Vec<String>,
    /// What to do.
    #[serde(default)]
    pub steps: Vec<String>,
    /// What a good outcome looks like.
    #[serde(default)]
    pub outcomes: Vec<String>,
    /// How it goes wrong.
    #[serde(default)]
    pub failure_modes: Vec<String>,
    /// How well it is known, in `[0,1]`.
    #[serde(default)]
    pub proficiency: f32,
    /// Recorded successes.
    #[serde(default)]
    pub successes: u32,
    /// Recorded attempts.
    #[serde(default)]
    pub attempts: u32,
}

fn nil_ulid() -> Ulid {
    Ulid::nil()
}

impl Procedure {
    /// A procedure with just a name.
    pub fn new(name: impl Into<String>) -> Self {
        Procedure {
            memory_id: Ulid::nil(),
            name: name.into(),
            preconditions: Vec::new(),
            steps: Vec::new(),
            outcomes: Vec::new(),
            failure_modes: Vec::new(),
            proficiency: 0.0,
            successes: 0,
            attempts: 0,
        }
    }

    /// Record one successful attempt, which raises proficiency toward 1.
    pub fn record_success(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
        self.successes = self.successes.saturating_add(1);
        self.proficiency = (f64::from(self.proficiency) + 0.15).min(1.0) as f32;
    }

    /// Record one failed attempt, which lowers proficiency but never to zero.
    pub fn record_failure(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
        self.proficiency = (f64::from(self.proficiency) - 0.25).max(0.05) as f32;
    }

    /// True when this procedure has become a habit.
    pub fn is_habit(&self) -> bool {
        self.proficiency >= HABIT_PROFICIENCY && self.successes >= HABIT_SUCCESSES
    }

    /// The canonical JSON that becomes the memory's content.
    pub fn canonical(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| MemoryError::Codec(e.to_string()))
    }
}

/// Record a procedure, returning its memory.
pub async fn record_procedure(
    store: &SqliteMemoryStore,
    logger: &Logger,
    ids: &UlidFactory,
    procedure: &Procedure,
) -> Result<Ulid> {
    let now = Timestamp::now();
    let memory_id = if procedure.memory_id.is_nil() {
        ids.next()
    } else {
        procedure.memory_id
    };
    let mut stored = procedure.clone();
    stored.memory_id = memory_id;
    let content = stored.canonical()?;
    let mut memory = Memory::new(
        memory_id,
        MemoryKind::Procedural,
        content,
        TimeInterval::open(now),
        0.8,
        if stored.is_habit() { 0.9 } else { 0.6 },
        ids.next(),
        now,
    )?;
    memory.cues.push(RetrievalCue::keyword(stored.name.clone()));
    for token in crate::store::tokenize(&stored.name).into_iter().take(8) {
        memory.cues.push(RetrievalCue::keyword(token));
    }
    for precondition in &stored.preconditions {
        memory
            .cues
            .push(RetrievalCue::keyword(precondition.clone()));
    }
    store.insert(&memory).await?;
    logger
        .audit(
            Level::Info,
            codes::MEMORY_ADD,
            crate::TARGET,
            Some(memory_id),
            json!({
                "memory_id": mm_core::ulid_string(&memory_id),
                "kind": MemoryKind::Procedural.as_str(),
                "tier": memory.tier.as_str(),
                "content_hash": memory.content_hash(),
                "confidence": memory.confidence,
                "importance": memory.importance,
                "protected": false,
                "provenance": mm_core::ulid_string(&memory.provenance),
                "habit": stored.is_habit(),
            }),
        )
        .await?;
    let _ = logger;
    let _ = ids;
    Ok(memory_id)
}

/// Read a procedure back out of its memory.
pub fn parse(content: &str) -> Result<Procedure> {
    serde_json::from_str(content).map_err(|e| MemoryError::Codec(format!("procedure: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proficiency_rises_with_success_and_falls_with_failure() {
        let mut procedure = Procedure::new("rebuild the index");
        assert_eq!(procedure.proficiency, 0.0);
        procedure.record_success();
        assert!(procedure.proficiency > 0.0);
        assert_eq!(procedure.successes, 1);
        let before = procedure.proficiency;
        procedure.record_failure();
        assert!(procedure.proficiency < before);
        assert!(procedure.proficiency > 0.0, "proficiency never hits zero");
        assert_eq!(procedure.attempts, 2);
    }

    #[test]
    fn a_habit_needs_both_proficiency_and_repetition() {
        let mut procedure = Procedure::new("run the gate");
        // Three successes at +0.15 each is 0.45 — repetition alone is not enough.
        for _ in 0..3 {
            procedure.record_success();
        }
        assert!(
            !procedure.is_habit(),
            "proficiency {0}",
            procedure.proficiency
        );
        for _ in 0..3 {
            procedure.record_success();
        }
        assert!(procedure.is_habit());
        assert_eq!(procedure.successes, 6);
    }

    #[test]
    fn proficiency_saturates_at_one() {
        let mut procedure = Procedure::new("x");
        for _ in 0..20 {
            procedure.record_success();
        }
        assert_eq!(procedure.proficiency, 1.0);
    }

    #[test]
    fn a_procedure_round_trips_through_canonical_json() {
        let mut procedure = Procedure::new("rebuild");
        procedure.steps = vec!["drop".into(), "rebuild".into()];
        procedure.preconditions = vec!["a clean tree".into()];
        let text = procedure.canonical().unwrap();
        assert_eq!(parse(&text).unwrap(), procedure);
    }

    #[test]
    fn malformed_content_is_a_codec_error() {
        assert_eq!(parse("not json").unwrap_err().code(), "memory.codec");
    }
}
