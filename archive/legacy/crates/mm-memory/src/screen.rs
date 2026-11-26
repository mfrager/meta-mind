//! Admission screening: what a candidate write is, before it is trusted.
//!
//! Plan §9 names *retrieval poisoning* as a risk, and §7 requires that
//! duplicate, near-duplicate, stale, contradictory, orphan-entity, and
//! missing-provenance writes are "flagged, never silently accepted". This module
//! is where that requirement becomes code rather than an intention.
//!
//! Every rule is deterministic and cheap, and each one answers a question a human
//! reviewer would ask about a candidate:
//!
//! | Flag | Question |
//! |---|---|
//! | `MissingProvenance` | Where did this come from? |
//! | `Duplicate` | Have I already been told exactly this? |
//! | `NearDuplicate` | Have I been told almost exactly this? |
//! | `Stale` | Has something newer already replaced this? |
//! | `Contradiction` | Does something I believe say the opposite? |
//! | `OrphanEntity` | Is this about anything I can relate it to? |
//!
//! Screening is a *read*: it reports, and only two flags block a write —
//! `MissingProvenance`, because a memory with no source cannot be reasoned about,
//! and `Duplicate`, because storing the same fact twice would double its weight in
//! every ranking. The rest are recorded and linked, so a reviewer (or a later
//! phase) can see that the write was noticed.
//!
//! Screening works on a [`Candidate`] rather than a [`Memory`] on purpose: the
//! whole point of `MissingProvenance` is to describe something that cannot legally
//! become a `Memory`, so a rule that could only run after construction could never
//! report it.

use std::collections::BTreeSet;

use mm_core::{Timestamp, Ulid};

use crate::error::Result;
use crate::graph::EntityGraph;
use crate::index::MemoryIndex;
use crate::model::{Memory, MemoryKind, TimeInterval};
use crate::store::{tokenize, SqliteMemoryStore};

/// A proposed write, before it is a `Memory`.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The id it would take, when it already has one.
    pub id: Option<Ulid>,
    /// The class it would join.
    pub kind: MemoryKind,
    /// The free text.
    pub content: String,
    /// Its provenance node; `None` means it has none.
    pub provenance: Option<Ulid>,
    /// The entities it mentions.
    pub entities: Vec<Ulid>,
    /// When it would become true.
    pub valid_from: Timestamp,
    /// When it would stop being true.
    pub valid_until: Option<Timestamp>,
}

impl Candidate {
    /// A candidate from a memory that is about to be written.
    pub fn from_memory(memory: &Memory) -> Self {
        Candidate {
            id: Some(memory.id),
            kind: memory.kind,
            content: memory.content.clone(),
            provenance: if memory.provenance.is_nil() {
                None
            } else {
                Some(memory.provenance)
            },
            entities: memory.entities.clone(),
            valid_from: memory.validity.from,
            valid_until: memory.validity.until,
        }
    }

    /// The interval it would be valid over.
    pub fn validity(&self) -> TimeInterval {
        TimeInterval {
            from: self.valid_from,
            until: self.valid_until,
        }
    }

    /// The words that carry meaning, for the similarity rules.
    pub fn significant(&self) -> BTreeSet<String> {
        significant(&self.content)
    }
}

/// What screening found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FlagKind {
    /// The candidate names no provenance node.
    MissingProvenance,
    /// An identical memory already exists.
    Duplicate,
    /// A near-identical memory exists.
    NearDuplicate,
    /// Something newer on the same subject already exists.
    Stale,
    /// An existing memory says the opposite.
    Contradiction,
    /// The candidate mentions an entity nothing else relates to.
    OrphanEntity,
}

/// Every flag, in the order screening reports them.
pub const FLAG_KINDS: [FlagKind; 6] = [
    FlagKind::MissingProvenance,
    FlagKind::Duplicate,
    FlagKind::NearDuplicate,
    FlagKind::Stale,
    FlagKind::Contradiction,
    FlagKind::OrphanEntity,
];

impl FlagKind {
    /// The stable wire name, matching the fixtures' `expect_flag`.
    pub fn as_str(self) -> &'static str {
        match self {
            FlagKind::MissingProvenance => "missing_provenance",
            FlagKind::Duplicate => "duplicate",
            FlagKind::NearDuplicate => "near_duplicate",
            FlagKind::Stale => "stale",
            FlagKind::Contradiction => "contradiction",
            FlagKind::OrphanEntity => "orphan_entity",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        FLAG_KINDS.into_iter().find(|kind| kind.as_str() == text)
    }

    /// True when the flag alone makes a candidate inadmissible.
    pub fn blocks(self) -> bool {
        matches!(self, FlagKind::MissingProvenance | FlagKind::Duplicate)
    }
}

impl std::fmt::Display for FlagKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One flag, and what raised it.
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    /// Which rule fired.
    pub kind: FlagKind,
    /// The existing memory it concerns, when there is one.
    pub existing: Option<Ulid>,
    /// A numeric measure for the rule, when it has one: the cosine for a
    /// near-duplicate, otherwise absent.
    pub score: Option<f64>,
    /// A human-readable detail.
    pub detail: String,
}

/// Everything screening found about one candidate.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScreenReport {
    /// Every flag, in [`FLAG_KINDS`] order.
    pub flags: Vec<Flag>,
}

impl ScreenReport {
    /// True when no rule fired.
    pub fn is_clean(&self) -> bool {
        self.flags.is_empty()
    }

    /// True when a flag makes the candidate inadmissible.
    pub fn is_blocked(&self) -> bool {
        self.flags.iter().any(|flag| flag.kind.blocks())
    }

    /// The first blocking flag, if any.
    pub fn blocking(&self) -> Option<&Flag> {
        self.flags.iter().find(|flag| flag.kind.blocks())
    }

    /// True when this flag was raised.
    pub fn flagged(&self, kind: FlagKind) -> bool {
        self.flags.iter().any(|flag| flag.kind == kind)
    }

    /// A flag of this kind, if any.
    pub fn get(&self, kind: FlagKind) -> Option<&Flag> {
        self.flags.iter().find(|flag| flag.kind == kind)
    }

    /// The flags as wire names, for a log record.
    pub fn names(&self) -> Vec<&'static str> {
        self.flags.iter().map(|flag| flag.kind.as_str()).collect()
    }
}

/// Words too common to carry a subject.
const STOPWORDS: [&str; 22] = [
    "the", "a", "an", "and", "or", "is", "are", "was", "were", "be", "been", "to", "of", "for",
    "in", "on", "at", "it", "its", "that", "this", "with",
];

/// How many meaningful words two memories must share before their disagreement is
/// read as a contradiction rather than as coincidence.
const CONTRADICTION_SHARED: usize = 3;

/// The meaning-bearing words of a text.
pub fn significant(text: &str) -> BTreeSet<String> {
    tokenize(text)
        .into_iter()
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .collect()
}

/// How many meaningful words two texts share.
pub fn shared_words(left: &BTreeSet<String>, right: &BTreeSet<String>) -> usize {
    left.intersection(right).count()
}

/// True when each text says something the other does not.
///
/// The condition is symmetric on purpose. If one text's meaningful words are a
/// subset of the other's, the second is the first with an extra qualifier — a
/// paraphrase or an elaboration, which is a *near*-duplicate, not an opposing
/// claim. Two texts that each add something are making different claims about the
/// same subject.
fn says_something_the_other_does_not(left: &BTreeSet<String>, right: &BTreeSet<String>) -> bool {
    !left.is_subset(right) && !right.is_subset(left)
}

/// Screen a candidate against the store.
pub async fn screen(
    store: &SqliteMemoryStore,
    index: &MemoryIndex,
    candidate: &Candidate,
    now: Timestamp,
    near_duplicate_cosine: f64,
) -> Result<ScreenReport> {
    let mut flags: Vec<Flag> = Vec::new();
    let content_hash = mm_core::content_hash(candidate.content.as_bytes());

    // --- MissingProvenance -------------------------------------------------
    if candidate.provenance.is_none() {
        flags.push(Flag {
            kind: FlagKind::MissingProvenance,
            existing: None,
            score: None,
            detail: "the candidate names no provenance node".to_string(),
        });
    }

    // --- Duplicate / NearDuplicate -----------------------------------------
    if let Some(existing) = store
        .find_by_hash(
            candidate.kind,
            &content_hash,
            candidate.valid_from,
            candidate.id,
        )
        .await?
    {
        flags.push(Flag {
            kind: FlagKind::Duplicate,
            existing: Some(existing),
            score: None,
            detail: "an identical memory already exists".to_string(),
        });
    } else {
        // Two, not one: a candidate that is already stored is its own nearest
        // neighbour, so the closest *other* memory is the second hit.
        let vector = MemoryIndex::embed(&candidate.content);
        if let Some((existing, cosine)) = index
            .knn(&vector, 2)
            .into_iter()
            .find(|(id, cosine)| Some(*id) != candidate.id && *cosine >= near_duplicate_cosine)
        {
            flags.push(Flag {
                kind: FlagKind::NearDuplicate,
                existing: Some(existing),
                score: Some(cosine),
                detail: format!("cosine {cosine:.4} with an existing memory"),
            });
        }
    }

    // --- Stale / Contradiction ---------------------------------------------
    //
    // Both rules are statements about one *subject*. Two memories are about the
    // same subject when they share the entities they are grounded in; that is a
    // narrower test than "they use similar words", and it is what keeps a
    // disagreement about a policy distinct from two sentences that happen to
    // mention the same nouns.
    let active = store.list(&crate::model::MemoryFilter::all()).await?;
    let mine = candidate.significant();
    let my_entities: BTreeSet<Ulid> = candidate.entities.iter().copied().collect();
    let mut stale: Option<(Ulid, Timestamp)> = None;
    let mut contradiction: Option<Ulid> = None;
    for existing in &active {
        if existing.kind != candidate.kind || Some(existing.id) == candidate.id {
            continue;
        }
        let theirs = significant(&existing.content);
        let shared = shared_words(&mine, &theirs);
        if shared >= 2 && existing.validity.from > candidate.valid_from {
            let replace = stale
                .map(|(_, at)| existing.validity.from > at)
                .unwrap_or(true);
            if replace {
                stale = Some((existing.id, existing.validity.from));
            }
        }
        if contradiction.is_none()
            && !my_entities.is_empty()
            && existing
                .entities
                .iter()
                .any(|entity| my_entities.contains(entity))
            && shared >= CONTRADICTION_SHARED
            && says_something_the_other_does_not(&mine, &theirs)
        {
            contradiction = Some(existing.id);
        }
    }
    if let Some((id, from)) = stale {
        flags.push(Flag {
            kind: FlagKind::Stale,
            existing: Some(id),
            score: None,
            detail: format!(
                "a newer memory on the same subject is valid from {}",
                from.to_rfc3339()
            ),
        });
    }
    if let Some(until) = candidate.valid_until {
        if until <= now && !flags.iter().any(|flag| flag.kind == FlagKind::Stale) {
            flags.push(Flag {
                kind: FlagKind::Stale,
                existing: None,
                score: None,
                detail: format!("its validity ended at {}", until.to_rfc3339()),
            });
        }
    }
    if let Some(id) = contradiction {
        flags.push(Flag {
            kind: FlagKind::Contradiction,
            existing: Some(id),
            score: None,
            detail: "an existing memory on the same subject reads the other way".to_string(),
        });
    }

    // --- OrphanEntity ------------------------------------------------------
    //
    // An orphan is an entity *nothing else* relates to. The candidate's own
    // mention does not count — otherwise every new entity would look related to
    // itself and the rule could never fire.
    if !candidate.entities.is_empty() {
        let mentions = store.entities_by_entity().await?;
        let graph = EntityGraph::load(store, None).await?;
        let mut related: BTreeSet<Ulid> = BTreeSet::new();
        for edge in graph.edges() {
            related.insert(edge.from);
            related.insert(edge.to);
        }
        for (entity, memories) in &mentions {
            if memories.iter().any(|memory| Some(*memory) != candidate.id) {
                related.insert(*entity);
            }
        }
        let orphans: Vec<String> = candidate
            .entities
            .iter()
            .filter(|entity| !related.contains(entity))
            .map(mm_core::ulid_string)
            .collect();
        if !orphans.is_empty() {
            flags.push(Flag {
                kind: FlagKind::OrphanEntity,
                existing: None,
                score: None,
                detail: format!("unknown entity(ies): {}", orphans.join(", ")),
            });
        }
    }

    flags.sort_by_key(|flag| (flag.kind, flag.existing));
    Ok(ScreenReport { flags })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_flags_are_exactly_provenance_and_duplication() {
        for kind in FLAG_KINDS {
            let expected = matches!(kind, FlagKind::MissingProvenance | FlagKind::Duplicate);
            assert_eq!(kind.blocks(), expected, "{kind}");
        }
        let report = ScreenReport {
            flags: vec![Flag {
                kind: FlagKind::Stale,
                existing: None,
                score: None,
                detail: String::new(),
            }],
        };
        assert!(!report.is_blocked());
        assert!(report.flagged(FlagKind::Stale));
        assert_eq!(report.names(), vec!["stale"]);
    }

    #[test]
    fn flag_kinds_round_trip_their_fixture_names() {
        for kind in FLAG_KINDS {
            assert_eq!(FlagKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(FlagKind::parse("nonsense"), None);
        // The names are the ones the adversarial fixtures use.
        assert_eq!(FlagKind::Duplicate.as_str(), "duplicate");
        assert_eq!(FlagKind::OrphanEntity.as_str(), "orphan_entity");
    }

    #[test]
    fn significant_words_drop_stopwords_and_punctuation() {
        let words = significant("The Toolchain policy, pins Rust!");
        assert!(words.contains("toolchain"));
        assert!(words.contains("policy"));
        assert!(!words.contains("the"));
        assert_eq!(
            shared_words(&words, &significant("the toolchain policy")),
            2
        );
    }

    #[test]
    fn two_opposing_claims_each_say_something_the_other_does_not() {
        let left =
            significant("the toolchain policy pins rust to exactly 1.96.1 for reproducibility");
        let right =
            significant("the toolchain policy allows any stable rust release for reproducibility");
        assert!(shared_words(&left, &right) >= CONTRADICTION_SHARED);
        assert!(says_something_the_other_does_not(&left, &right));
        // The same shape with the day swapped is also a disagreement.
        let monday = significant("the deploy window is tuesday morning");
        let thursday = significant("the deploy window is thursday morning");
        assert!(says_something_the_other_does_not(&monday, &thursday));
    }

    #[test]
    fn a_paraphrase_is_not_a_contradiction() {
        let base =
            significant("the toolchain policy now pins rust to exactly 1.96.1 for reproducibility");
        // One text is the other with an extra qualifier, so neither is a claim the
        // other contradicts — it is a near-duplicate.
        let elaborated = significant(
            "the toolchain policy now pins rust to exactly 1.96.1 for full reproducibility",
        );
        assert!(!says_something_the_other_does_not(&base, &elaborated));
        // The same words twice is a paraphrase, not a disagreement.
        assert!(!says_something_the_other_does_not(&base, &base.clone()));
    }

    #[test]
    fn a_candidate_from_a_memory_reports_its_provenance() {
        let id = Ulid::from_parts(1_700_000_000_000, 1);
        let memory = Memory::draft(id, MemoryKind::Semantic, "a claim", Timestamp::EPOCH).unwrap();
        let candidate = Candidate::from_memory(&memory);
        assert_eq!(candidate.provenance, Some(id));
        assert_eq!(candidate.id, Some(id));
        assert!(candidate.validity().is_open());
    }

    #[test]
    fn a_nil_provenance_memory_is_reported_not_swallowed() {
        // A candidate screened *before* it becomes a memory can carry no
        // provenance, which is exactly the case `MissingProvenance` exists for.
        let candidate = Candidate {
            id: None,
            kind: MemoryKind::Semantic,
            content: "an unsourced assertion".to_string(),
            provenance: None,
            entities: Vec::new(),
            valid_from: Timestamp::EPOCH,
            valid_until: None,
        };
        assert!(candidate.significant().contains("unsourced"));
        assert!(FlagKind::MissingProvenance.blocks());
    }
}
