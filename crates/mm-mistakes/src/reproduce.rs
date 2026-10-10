//! `reproduce`: reduce a recorded incident to the smallest thing a test can assert.
//!
//! Phase 11's requirement is that a mistake becomes a test that fails before the fix and
//! passes after it. Before a test can be written, the incident has to be reduced to a
//! *statement*: what happened, what should have happened instead, and which signals were
//! available and missed. That reduction is this module, and it is deliberately pure
//! arithmetic over the mistake record — no model, no guessing — because the artifact it
//! produces ends up in a test that later phases must be able to reproduce.
//!
//! Three decisions are worth stating:
//!
//! * **The expectation comes from the corrective rule when there is one.** Phase 5
//!   guarantees a mistake always names a rule (it derives one from the failure mode when
//!   the caller supplies none), so the rule is the recorded intent, and the derived
//!   sentence is only the fallback. Inventing an expectation when the being already wrote
//!   one would put our words in its record.
//! * **The slug carries the ULID's tail.** Two mistakes with the same failure mode are
//!   the same *sentence* and different *incidents*; a slug derived from the failure mode
//!   alone would collide, and a colliding case directory would silently overwrite the
//!   first regression test. The last six characters of the ULID make the slug unique
//!   without making it unreadable.
//! * **Cues are the retrieval handle.** The missed signals are what a later episode would
//!   have needed to notice in time, so they are carried verbatim; the distinctive words of
//!   the failure mode are added so a cue search on the incident's own vocabulary finds it.
//!
//! The mistake row is read through the organ's own schema and mapped into
//! `mm_memory::MistakeRecord`, rather than through `SqliteMemoryStore`. That store's read
//! path is written for a *writer* — it needs a logger and an identifier factory to be
//! constructed — and compiling a test needs neither. The columns are the same ones
//! `list_mistakes` selects, and the type is the organ's, so the two cannot drift into
//! disagreeing about what a mistake record is.

use std::collections::BTreeSet;

use mm_core::{Param, Tabular, Ulid};
use mm_memory::MistakeRecord;
use mm_store_sqlite::SqliteStore;

use crate::error::{MistakeError, Result};

/// The longest a slug may be before the ULID tail is appended.
const MAX_SLUG_CHARS: usize = 40;
/// How many characters of the ULID identify the incident.
const ULID_TAIL_CHARS: usize = 6;
/// The longest a normalized sentence may be.
const MAX_SENTENCE_CHARS: usize = 200;
/// The shortest word that can serve as a distinctive cue.
const MIN_CUE_CHARS: usize = 5;

/// A mistake reduced to what a regression test needs.
#[derive(Debug, Clone, PartialEq)]
pub struct MinimizedCase {
    /// The mistake row this came from.
    pub mistake: Ulid,
    /// The directory name the case will live under.
    pub slug: String,
    /// One sentence describing the test, for the case's `description`.
    pub summary: String,
    /// The failure mode, normalized to a single sentence.
    pub subject: String,
    /// What should hold instead.
    pub expectation: String,
    /// The observed behaviour, spelled as the buggy side of the pair.
    pub observed: String,
    /// Signals that were available and missed, plus the incident's own vocabulary.
    pub cues: Vec<String>,
}

impl MinimizedCase {
    /// The token a generated test greps for.
    ///
    /// The longest alphanumeric word of the subject, so the marker is stable for a given
    /// failure mode and short enough to sit inside a `grep -F` argument. When the subject
    /// has no such word (a failure mode written entirely in punctuation or in a script
    /// this crate does not read), the slug is the marker: it is always present and always
    /// shell-safe.
    pub fn marker(&self) -> String {
        let mut best: Option<String> = None;
        for word in self.subject.split_whitespace() {
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if cleaned.len() < MIN_CUE_CHARS {
                continue;
            }
            let longer = best.as_ref().is_none_or(|b| cleaned.len() > b.len());
            if longer {
                best = Some(cleaned);
            }
        }
        best.unwrap_or_else(|| self.slug.clone())
    }
}

/// Collapse a failure mode into one sentence.
///
/// The record is written by a person or a model and may carry newlines, runs of spaces
/// and a trailing period; a test's comment and its `grep` marker should carry none of
/// those.
pub fn normalize(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches(['.', '!', ';']).trim();
    if trimmed.chars().count() <= MAX_SENTENCE_CHARS {
        return trimmed.to_string();
    }
    let mut cut = MAX_SENTENCE_CHARS;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = trimmed[..cut].trim_end();
    format!("{head}...")
}

/// The file-system-safe name of a case.
///
/// Lowercase, with every run of characters that is not `[a-z0-9]` replaced by a single
/// `_`, truncated to `MAX_SLUG_CHARS`, with the last `ULID_TAIL_CHARS` characters of the
/// mistake's ULID appended. The tail is what keeps two incidents with the same failure
/// mode apart — see the module docs.
pub fn slug_for(failure_mode: &str, mistake: Ulid) -> String {
    let id = mm_core::ulid_string(&mistake);
    let tail: String = id
        .chars()
        .rev()
        .take(ULID_TAIL_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut slug = String::new();
    let mut last_was_separator = true;
    for ch in failure_mode.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('_');
            last_was_separator = true;
        }
        if slug.chars().count() >= MAX_SLUG_CHARS {
            break;
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        format!("incident_{tail}")
    } else {
        format!("{slug}_{tail}")
    }
}

/// Reduce an incident to `(observed, expectation, cues)`.
///
/// Pure, so the reduction of a given mistake is the same in the compiled test as it is in
/// the record: nothing here reads a clock, a file, or a model.
pub fn minimize(failure_mode: &str, missed: &[String]) -> (String, String, Vec<String>) {
    let subject = normalize(failure_mode);
    let observed = format!("the recorded incident: {subject}");
    let expectation =
        format!("the operation is refused rather than reported as success: {subject}");
    let mut cues: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for signal in missed {
        let cue = normalize(signal).to_lowercase();
        if cue.is_empty() || !seen.insert(cue.clone()) {
            continue;
        }
        cues.push(cue);
    }
    for word in subject.split_whitespace() {
        let cleaned: String = word
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if cleaned.len() < MIN_CUE_CHARS || !seen.insert(cleaned.clone()) {
            continue;
        }
        cues.push(cleaned);
    }
    (observed, expectation, cues)
}

/// The mistake's detail row, or a refusal naming the ULID.
///
/// `mistakes.id` is the identifier the rest of Phase 11 uses: `regression_tests.mistake_ulid`
/// targets this column (INDEX.md D5), not the memory the mistake is stored in. A caller
/// that hands in the *memory* ULID gets [`MistakeError::NoSuchMistake`] rather than a
/// silent empty case, which is the failure that would be hardest to notice.
pub async fn fetch_mistake(mistake: Ulid, store: &SqliteStore) -> Result<MistakeRecord> {
    let rows = store
        .query_json(
            "SELECT id, memory_id, failure_mode, root_cause_ulid, missed_signal, \
             corrective_rule_ulid, recurrence_risk FROM mistakes WHERE id = ?",
            vec![Param::Text(mm_core::ulid_string(&mistake))],
        )
        .await?;
    let Some(row) = rows.first() else {
        return Err(MistakeError::NoSuchMistake(mm_core::ulid_string(&mistake)));
    };
    let text = |key: &str| -> Result<Ulid> {
        let raw = row[key]
            .as_str()
            .ok_or_else(|| MistakeError::Store(format!("mistakes.{key} is not text")))?;
        mm_core::id::parse_ulid(raw)
            .map_err(|e| MistakeError::Store(format!("mistakes.{key}: {e}")))
    };
    let optional = |key: &str| -> Result<Option<Ulid>> {
        match row[key].as_str() {
            Some(raw) => mm_core::id::parse_ulid(raw)
                .map(Some)
                .map_err(|e| MistakeError::Store(format!("mistakes.{key}: {e}"))),
            None => Ok(None),
        }
    };
    let missed: Vec<String> = serde_json::from_str(row["missed_signal"].as_str().unwrap_or("[]"))?;
    Ok(MistakeRecord {
        id: text("id")?,
        memory_id: text("memory_id")?,
        failure_mode: row["failure_mode"]
            .as_str()
            .ok_or_else(|| MistakeError::Store("mistakes.failure_mode is not text".into()))?
            .to_string(),
        root_cause: optional("root_cause_ulid")?,
        missed_signal: missed,
        corrective_rule: optional("corrective_rule_ulid")?,
        recurrence_risk: row["recurrence_risk"].as_f64().unwrap_or(0.0) as f32,
    })
}

/// Reduce a recorded mistake to the statement a regression test is written from.
pub async fn reproduce(mistake: Ulid, store: &SqliteStore) -> Result<MinimizedCase> {
    let record = fetch_mistake(mistake, store).await?;
    if record.failure_mode.trim().is_empty() {
        return Err(MistakeError::validation(
            "failure_mode",
            "a mistake with no failure mode cannot be reduced to a test",
        ));
    }
    let (observed, derived, cues) = minimize(&record.failure_mode, &record.missed_signal);
    let subject = normalize(&record.failure_mode);
    // The recorded rule is the being's own words; the derived sentence is the fallback.
    // The record's rule *text* lives with the memory (Phase 5 keeps the detail row's
    // `corrective_rule_ulid`), so when the detail row names a rule but the text is not on
    // this row, the derived sentence stands and says so.
    let expectation = match record.corrective_rule {
        Some(rule) => format!(
            "{derived} (corrective rule {})",
            mm_core::ulid_string(&rule)
        ),
        None => derived,
    };
    let slug = slug_for(&record.failure_mode, record.id);
    let summary = format!("a regression test for: {subject}");
    Ok(MinimizedCase {
        mistake: record.id,
        slug,
        summary,
        subject,
        expectation,
        observed,
        cues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Timestamp;

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    #[test]
    fn normalization_collapses_whitespace_and_trailing_punctuation() {
        assert_eq!(
            normalize("  the counter   reported\nan empty success.  "),
            "the counter reported an empty success"
        );
        assert_eq!(normalize(""), "");
        let long = "word ".repeat(80);
        let normalized = normalize(&long);
        assert!(normalized.ends_with("..."), "{normalized}");
        assert!(normalized.chars().count() <= MAX_SENTENCE_CHARS + 3);
    }

    #[test]
    fn the_slug_is_safe_unique_and_readable() {
        let a = slug_for("The record counter reported an EMPTY success!", id(1));
        let b = slug_for("The record counter reported an EMPTY success!", id(2));
        let before = slug_for("The record counter reported an EMPTY success!", id(1));
        assert_eq!(a, before, "the slug must be a pure function of its inputs");
        assert_ne!(a, b, "two incidents must not share a directory");
        assert!(
            a.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{a}"
        );
        assert!(a.starts_with("the_record_counter"), "{a}");
        assert_eq!(a.rsplit('_').next().unwrap().len(), ULID_TAIL_CHARS);
        // A failure mode with nothing usable still yields a name.
        let empty = slug_for("!!!", id(3));
        assert!(empty.starts_with("incident_"), "{empty}");
    }

    #[test]
    fn minimization_keeps_the_missed_signals_and_adds_the_incidents_vocabulary() {
        let (observed, expectation, cues) = minimize(
            "the retrieval step did not re-read the schema",
            &[
                "the migration had run".to_string(),
                "a stale index".to_string(),
            ],
        );
        assert!(observed.contains("retrieval"), "{observed}");
        assert!(expectation.contains("refused"), "{expectation}");
        assert!(
            cues.contains(&"the migration had run".to_string()),
            "{cues:?}"
        );
        assert!(cues.contains(&"retrieval".to_string()), "{cues:?}");
        // Deduplicated: "retrieval" appears in the failure mode once only.
        assert_eq!(cues.iter().filter(|c| c.as_str() == "retrieval").count(), 1);
    }

    #[test]
    fn the_marker_is_the_longest_usable_word_or_the_slug() {
        let case = MinimizedCase {
            mistake: id(1),
            slug: "a_b_c_01h000".to_string(),
            summary: "s".to_string(),
            subject: "the retrieval step did not re-read the schema".to_string(),
            expectation: "e".to_string(),
            observed: "o".to_string(),
            cues: Vec::new(),
        };
        assert_eq!(case.marker(), "retrieval");
        let punctuated = MinimizedCase {
            subject: "!!! ???".to_string(),
            slug: "incident_01h000".to_string(),
            ..case
        };
        assert_eq!(punctuated.marker(), "incident_01h000");
    }

    #[tokio::test]
    async fn a_recorded_mistake_reproduces_into_a_case() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let mistake = id(7);
        let memory = id(8);
        let created = Timestamp::from_epoch_seconds(1_700_000_000).to_rfc3339();
        store
            .execute(
                "INSERT INTO mistakes (id, memory_id, failure_mode, root_cause_ulid, \
                 missed_signal, corrective_rule_ulid, recurrence_risk, created_ulid) \
                 VALUES (?, ?, ?, NULL, ?, ?, ?, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&mistake)),
                    Param::Text(mm_core::ulid_string(&memory)),
                    Param::Text("the parser accepted an empty stream".to_string()),
                    Param::Text(
                        serde_json::to_string(&vec!["no record parsed".to_string()]).unwrap(),
                    ),
                    Param::Text(mm_core::ulid_string(&id(9))),
                    Param::Real(0.8),
                    // The row's creation instant, not the mistake's id twice: the column is
                    // `created_ulid`, and reading back the failure mode is what this test
                    // actually asserts.
                    Param::Text(created),
                ],
            )
            .await
            .unwrap();
        let case = reproduce(mistake, &store).await.unwrap();
        assert_eq!(case.mistake, mistake);
        assert_eq!(case.subject, "the parser accepted an empty stream");
        assert!(
            case.expectation.contains("corrective rule"),
            "{}",
            case.expectation
        );
        assert!(case.summary.contains("regression test"), "{}", case.summary);
        assert!(case.cues.contains(&"parser".to_string()), "{:?}", case.cues);
        assert!(case.marker().len() >= MIN_CUE_CHARS);
    }

    #[tokio::test]
    async fn an_unrecorded_mistake_is_a_named_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let error = reproduce(id(11), &store).await.unwrap_err();
        assert_eq!(error.code(), "mistake.not_found");
        assert!(error.to_string().contains("01h"), "{error}");
    }
}
