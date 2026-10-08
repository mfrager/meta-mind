//! The memory model: the ten classes of design §32 as typed, validated values.
//!
//! Two properties are decided here rather than at the storage edge, because both
//! are semantic and both are easy to lose:
//!
//! 1. **A memory is bi-temporal** (Graphiti). `validity` is *world* time — when
//!    the thing remembered was true — and `recorded_at` is *system* time — when
//!    the kernel learned it. An instance that collapses the two cannot answer
//!    "what did I know, and when did I know it", and Phase 9's sanity firewall
//!    depends on that question having an answer.
//! 2. **A memory carries provenance or it does not exist.** `provenance` is a
//!    required ULID, not an optional annotation, and [`Memory::validate`] refuses
//!    a nil one. The plan's rule is "no memory enters `/memory` without a source";
//!    this is where that rule is enforced rather than hoped for.

use mm_core::{content_hash, Timestamp, Ulid};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{MemoryError, Result};

/// The longest free-text content a memory may carry.
///
/// A bound rather than a truncation: a caller that wants to remember something
/// larger consolidates it into a summary first, which is exactly what the
/// summary tree is for.
pub const MAX_CONTENT_CHARS: usize = 8192;

/// The ten memory classes (design §32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Something that happened, in sequence.
    Episodic,
    /// Something learned that is true independent of the episode.
    Semantic,
    /// How to do something.
    Procedural,
    /// The bounded working set for the current task.
    Working,
    /// The being's own story.
    Autobiographical,
    /// What is known about a particular other.
    Relational,
    /// A recorded expectation, evaluated by a later phase.
    Prediction,
    /// Something that went wrong, with a root cause and a corrective rule.
    Mistake,
    /// Something that nearly went wrong.
    NearMiss,
    /// The immutable developmental ledger.
    Developmental,
}

/// Every kind, in a stable order.
pub const MEMORY_KINDS: [MemoryKind; 10] = [
    MemoryKind::Episodic,
    MemoryKind::Semantic,
    MemoryKind::Procedural,
    MemoryKind::Working,
    MemoryKind::Autobiographical,
    MemoryKind::Relational,
    MemoryKind::Prediction,
    MemoryKind::Mistake,
    MemoryKind::NearMiss,
    MemoryKind::Developmental,
];

impl MemoryKind {
    /// The stable wire name, matching the `memories.kind` CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Procedural => "procedural",
            MemoryKind::Working => "working",
            MemoryKind::Autobiographical => "autobiographical",
            MemoryKind::Relational => "relational",
            MemoryKind::Prediction => "prediction",
            MemoryKind::Mistake => "mistake",
            MemoryKind::NearMiss => "near_miss",
            MemoryKind::Developmental => "developmental",
        }
    }

    /// The `/memory` class this kind is typed as.
    pub fn rdf_class(self) -> &'static str {
        match self {
            MemoryKind::Episodic => "EpisodicMemory",
            MemoryKind::Semantic => "SemanticMemory",
            MemoryKind::Procedural => "ProceduralMemory",
            MemoryKind::Working => "WorkingMemory",
            MemoryKind::Autobiographical => "AutobiographicalMemory",
            MemoryKind::Relational => "RelationalMemory",
            MemoryKind::Prediction => "PredictionMemory",
            MemoryKind::Mistake => "Mistake",
            MemoryKind::NearMiss => "NearMiss",
            MemoryKind::Developmental => "DevelopmentalMemory",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        MEMORY_KINDS.into_iter().find(|k| k.as_str() == s)
    }

    /// Every kind a comma-separated CLI argument names.
    pub fn parse_list(s: &str) -> Option<Vec<Self>> {
        s.split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(MemoryKind::parse)
            .collect()
    }
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The tier a memory currently lives in (plan §2).
///
/// Tiers are *views* over one store, never separate stores: a memory that moved
/// from `recall` to `core` is the same memory with a different salience, and
/// moving it must not change its identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Always in context, bounded, chosen by salience.
    Core,
    /// Everything hybrid search can reach.
    Recall,
    /// Low-retention records kept for provenance, not normally surfaced.
    Archival,
}

/// Every tier, in a stable order.
pub const TIERS: [Tier; 3] = [Tier::Core, Tier::Recall, Tier::Archival];

impl Tier {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Core => "core",
            Tier::Recall => "recall",
            Tier::Archival => "archival",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        TIERS.into_iter().find(|t| t.as_str() == s)
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The lifecycle status of a row.
///
/// `Archived` is the only endpoint and it is not deletion: forgetting moves a
/// record out of the retrieval set and leaves its lineage intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    /// Retrievable.
    Active,
    /// Kept for provenance, not surfaced.
    Archived,
}

impl RecordStatus {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            RecordStatus::Active => "active",
            RecordStatus::Archived => "archived",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(RecordStatus::Active),
            "archived" => Some(RecordStatus::Archived),
            _ => None,
        }
    }
}

/// A closed time interval in *world* time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeInterval {
    /// When the thing became true.
    pub from: Timestamp,
    /// When it stopped being true; `None` is open-ended.
    pub until: Option<Timestamp>,
}

impl TimeInterval {
    /// An interval that starts at `from` and has not closed.
    pub fn open(from: Timestamp) -> Self {
        TimeInterval { from, until: None }
    }

    /// A closed interval.
    pub fn closed(from: Timestamp, until: Timestamp) -> Self {
        TimeInterval {
            from,
            until: Some(until),
        }
    }

    /// True when `until` is unset.
    pub fn is_open(&self) -> bool {
        self.until.is_none()
    }

    /// True when `at` falls inside the interval, half-open on the right.
    pub fn contains(&self, at: Timestamp) -> bool {
        at >= self.from && self.until.is_none_or(|end| at < end)
    }

    /// Length in nanoseconds, when the interval is closed.
    pub fn duration_ns(&self) -> Option<u128> {
        self.until.map(|end| end.as_nanos().saturating_sub(self.from.as_nanos()))
    }
}

/// What a cue is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueKind {
    /// A word the lexical channel indexes.
    Keyword,
    /// A vector the index holds.
    Embedding,
    /// An entity the graph channel seeds from.
    Entity,
}

impl CueKind {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            CueKind::Keyword => "keyword",
            CueKind::Embedding => "embedding",
            CueKind::Entity => "entity",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "keyword" => Some(CueKind::Keyword),
            "embedding" => Some(CueKind::Embedding),
            "entity" => Some(CueKind::Entity),
            _ => None,
        }
    }
}

/// One retrieval cue attached to a memory (A-MEM's note keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalCue {
    /// What the cue is.
    pub kind: CueKind,
    /// Its text.
    pub text: String,
}

impl RetrievalCue {
    /// A keyword cue.
    pub fn keyword(text: impl Into<String>) -> Self {
        RetrievalCue {
            kind: CueKind::Keyword,
            text: text.into(),
        }
    }

    /// An entity cue.
    pub fn entity(text: impl Into<String>) -> Self {
        RetrievalCue {
            kind: CueKind::Entity,
            text: text.into(),
        }
    }

    /// An embedding cue. The vector itself lives in the packed index; the cue
    /// records that one exists and the text it was built from.
    pub fn embedding(text: impl Into<String>) -> Self {
        RetrievalCue {
            kind: CueKind::Embedding,
            text: text.into(),
        }
    }
}

/// How two memories are related.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// One memory is evidence for another.
    Supports,
    /// One memory is evidence against another.
    Contradicts,
    /// Near-duplicates, kept distinct on purpose.
    SimilarTo,
    /// Derived from, via `prov:wasDerivedFrom`.
    DerivedFrom,
    /// A consolidation consumed it.
    ConsolidatedFrom,
    /// A summary covers it.
    Summarizes,
    /// An access correlated the two.
    AccessedBy,
    /// Both mention the same entity.
    RelatedEntity,
    /// A memory names an open commitment it serves.
    ServesCommitment,
}

impl LinkKind {
    /// The stable wire name, matching the `mm:` predicate.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkKind::Supports => "supports",
            LinkKind::Contradicts => "contradicts",
            LinkKind::SimilarTo => "similar_to",
            LinkKind::DerivedFrom => "derived_from",
            LinkKind::ConsolidatedFrom => "consolidated_from",
            LinkKind::Summarizes => "summarizes",
            LinkKind::AccessedBy => "accessed_by",
            LinkKind::RelatedEntity => "related_entity",
            LinkKind::ServesCommitment => "serves_commitment",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        const ALL: [LinkKind; 9] = [
            LinkKind::Supports,
            LinkKind::Contradicts,
            LinkKind::SimilarTo,
            LinkKind::DerivedFrom,
            LinkKind::ConsolidatedFrom,
            LinkKind::Summarizes,
            LinkKind::AccessedBy,
            LinkKind::RelatedEntity,
            LinkKind::ServesCommitment,
        ];
        ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// One memory: a typed, provenance-bearing trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    /// The memory's ULID; also its instance IRI.
    pub id: Ulid,
    /// Whether the row is retrievable or kept only for provenance.
    ///
    /// The plan's struct lists the tier but not the status; the DDL has both, and
    /// a caller that cannot tell an archived memory from an active one would put
    /// forgotten state back in front of the being.
    pub status: RecordStatus,
    /// Which of the ten classes it is.
    pub kind: MemoryKind,
    /// Which tier it currently lives in.
    pub tier: Tier,
    /// The free text.
    pub content: String,
    /// The event, commitment, or record it was derived from, when there is one.
    pub source: Option<Ulid>,
    /// How much the being trusts it, in `[0,1]`.
    pub confidence: f32,
    /// How much it matters, in `[0,1]`.
    pub importance: f32,
    /// World-time validity.
    pub validity: TimeInterval,
    /// System time: when the kernel recorded it.
    pub recorded_at: Timestamp,
    /// The operation ULID that recorded it.
    pub recorded_ulid: Ulid,
    /// The provenance node (PROV).
    pub provenance: Ulid,
    /// Entities it mentions.
    pub entities: Vec<Ulid>,
    /// Retrieval cues.
    pub cues: Vec<RetrievalCue>,
    /// `true` when it may never be archived.
    pub protected: bool,
}

impl Memory {
    /// Build and validate a memory.
    ///
    /// `recorded_ulid` starts equal to `id`; a caller recording the memory on
    /// behalf of another operation overwrites it with that operation's ULID.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Ulid,
        kind: MemoryKind,
        content: impl Into<String>,
        validity: TimeInterval,
        confidence: f32,
        importance: f32,
        provenance: Ulid,
        recorded_at: Timestamp,
    ) -> Result<Self> {
        let memory = Memory {
            id,
            status: RecordStatus::Active,
            kind,
            tier: Tier::Recall,
            content: content.into(),
            source: None,
            confidence,
            importance,
            validity,
            recorded_at,
            recorded_ulid: id,
            provenance,
            entities: Vec::new(),
            cues: Vec::new(),
            protected: false,
        };
        memory.validate_shallow()?;
        Ok(memory)
    }

    /// A memory with the kernel's default confidence and importance, its own
    /// provenance node, and an open interval starting at `from`.
    pub fn draft(id: Ulid, kind: MemoryKind, content: impl Into<String>, from: Timestamp) -> Result<Self> {
        Memory::new(id, kind, content, TimeInterval::open(from), 0.5, 0.5, id, from)
    }

    /// Set the source this memory was derived from.
    pub fn with_source(mut self, source: Ulid) -> Self {
        self.source = Some(source);
        self
    }

    /// Set the tier.
    pub fn with_tier(mut self, tier: Tier) -> Self {
        self.tier = tier;
        self
    }

    /// Set confidence.
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence;
        self
    }

    /// Set importance.
    pub fn with_importance(mut self, importance: f32) -> Self {
        self.importance = importance;
        self
    }

    /// Set the lifecycle status.
    pub fn with_status(mut self, status: RecordStatus) -> Self {
        self.status = status;
        self
    }

    /// Mark the memory protected.
    pub fn with_protected(mut self, protected: bool) -> Self {
        self.protected = protected;
        self
    }

    /// Attach entities.
    pub fn with_entities(mut self, entities: Vec<Ulid>) -> Self {
        self.entities = entities;
        self
    }

    /// Attach one cue.
    pub fn with_cue(mut self, cue: RetrievalCue) -> Self {
        self.cues.push(cue);
        self
    }

    /// Attach cues.
    pub fn with_cues(mut self, cues: Vec<RetrievalCue>) -> Self {
        self.cues.extend(cues);
        self
    }

    /// `sha256(canonical content)` — the identity of what is remembered, which is
    /// what makes duplicate detection possible without reading every row.
    pub fn content_hash(&self) -> String {
        content_hash(self.content.as_bytes())
    }

    /// True when the memory is retrievable.
    pub fn is_active(&self) -> bool {
        self.status == RecordStatus::Active
    }

    /// True when the record is developmental: the immutable ledger.
    pub fn is_developmental(&self) -> bool {
        self.kind == MemoryKind::Developmental
    }

    /// True when this memory may not be archived.
    pub fn is_unforgettable(&self) -> bool {
        self.protected || self.is_developmental()
    }

    /// Refuse a value that cannot be *constructed* faithfully.
    ///
    /// This is the check a constructor runs. It deliberately leaves the
    /// developmental-ledger rule to [`Memory::validate`], so a caller can build a
    /// record and then mark it protected — which is what the store's own tests do.
    pub fn validate_shallow(&self) -> Result<()> {
        if self.id.is_nil() {
            return Err(MemoryError::validation("id", "must not be the nil ULID"));
        }
        if self.provenance.is_nil() {
            return Err(MemoryError::validation(
                "provenance",
                "a memory must name the provenance node it came from",
            ));
        }
        if self.recorded_ulid.is_nil() {
            return Err(MemoryError::validation(
                "recorded_ulid",
                "must not be the nil ULID",
            ));
        }
        let trimmed = self.content.trim();
        if trimmed.is_empty() {
            return Err(MemoryError::validation("content", "must not be empty"));
        }
        if self.content.chars().count() > MAX_CONTENT_CHARS {
            return Err(MemoryError::validation(
                "content",
                format!("must be at most {MAX_CONTENT_CHARS} characters"),
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(MemoryError::validation(
                "confidence",
                format!("must be in [0,1], got {}", self.confidence),
            ));
        }
        if !(0.0..=1.0).contains(&self.importance) {
            return Err(MemoryError::validation(
                "importance",
                format!("must be in [0,1], got {}", self.importance),
            ));
        }
        if let Some(until) = self.validity.until {
            if until < self.validity.from {
                return Err(MemoryError::validation(
                    "validity",
                    "valid_until precedes valid_from",
                ));
            }
        }
        Ok(())
    }

    /// Refuse a memory that must not reach the store.
    ///
    /// The store calls this, not the constructors: a developmental record is part
    /// of the immutable ledger and must carry its do-not-forget mark *before* it is
    /// written, and a caller that forgot the mark gets a refusal rather than a
    /// silently unprotected ledger entry.
    pub fn validate(&self) -> Result<()> {
        self.validate_shallow()?;
        if self.is_developmental() && !self.protected {
            return Err(MemoryError::validation(
                "protected",
                "a developmental memory is part of the immutable ledger and must be protected",
            ));
        }
        Ok(())
    }
}

/// A mistake: what went wrong, why, and what to do instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mistake {
    /// The memory that records it.
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
    /// The rule that prevents recurrence; generated when absent.
    pub corrective_rule: Option<Ulid>,
    /// The text of the corrective rule, used to derive one when absent.
    pub corrective_rule_text: Option<String>,
    /// How likely it is to happen again, in `[0,1]`.
    pub recurrence_risk: f32,
}

/// A stored mistake detail row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MistakeRecord {
    /// The detail row's own ULID.
    pub id: Ulid,
    /// The memory that holds the mistake.
    pub memory_id: Ulid,
    /// What kind of failure it was.
    pub failure_mode: String,
    /// The root cause.
    pub root_cause: Option<Ulid>,
    /// Signals that were available and missed.
    pub missed_signal: Vec<String>,
    /// The rule that prevents recurrence.
    pub corrective_rule: Option<Ulid>,
    /// How likely it is to happen again.
    pub recurrence_risk: f32,
}

/// A near miss: the same shape as a mistake, without the harm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NearMiss {
    /// The memory that records it.
    pub memory_id: Ulid,
    /// What almost happened.
    pub failure_mode: String,
    /// Signals that were available and missed.
    pub missed_signal: Vec<String>,
}

/// One node of the RAPTOR summary tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryNode {
    /// The summary memory's ULID.
    pub memory_id: Ulid,
    /// The node one level up, if this is not the root.
    pub parent_id: Option<Ulid>,
    /// The depth: the leaves are 0.
    pub level: u32,
    /// Everything this node covers.
    pub member_ids: Vec<Ulid>,
}

/// A GraphRAG-style community summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Community {
    /// The community memory's ULID.
    pub memory_id: Ulid,
    /// A stable label for the cluster.
    pub label: String,
    /// Its members.
    pub member_ids: Vec<Ulid>,
    /// The cluster's modularity, when the detector reported one.
    pub modularity: Option<f64>,
}

/// What a consolidation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationMethod {
    /// A new memory was added with no existing equivalent.
    Add,
    /// Near-duplicates were folded into one.
    Merge,
    /// A memory superseded one whose validity had ended.
    Supersede,
    /// Episodes were generalised into a semantic memory.
    Generalize,
    /// A summary node was built.
    Summarize,
    /// A community summary was built.
    Community,
}

impl ConsolidationMethod {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            ConsolidationMethod::Add => "add",
            ConsolidationMethod::Merge => "merge",
            ConsolidationMethod::Supersede => "supersede",
            ConsolidationMethod::Generalize => "generalize",
            ConsolidationMethod::Summarize => "summarize",
            ConsolidationMethod::Community => "community",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        [
            ConsolidationMethod::Add,
            ConsolidationMethod::Merge,
            ConsolidationMethod::Supersede,
            ConsolidationMethod::Generalize,
            ConsolidationMethod::Summarize,
            ConsolidationMethod::Community,
        ]
        .into_iter()
        .find(|m| m.as_str() == s)
    }
}

/// One provenance-bearing consolidation step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Consolidation {
    /// The step's own ULID.
    pub id: Ulid,
    /// Everything that fed it.
    pub source_ids: Vec<Ulid>,
    /// What it produced.
    pub target_id: Ulid,
    /// Which method produced it.
    pub method: ConsolidationMethod,
    /// The operation that created it.
    pub created_ulid: Ulid,
}

/// The result of a forget cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetReport {
    /// The instant retention was measured at.
    pub now: Timestamp,
    /// How many active records were considered.
    pub considered: usize,
    /// Records that were archived (or would be, in a dry run).
    pub archived: Vec<Ulid>,
    /// Records the cycle refused to archive.
    pub refused: Vec<crate::error::ProtectedRefusal>,
    /// Records that were above the threshold and untouched.
    pub retained: usize,
    /// True when nothing was actually written.
    pub dry_run: bool,
}

impl ForgetReport {
    /// An empty report for `now`.
    pub fn new(now: Timestamp, dry_run: bool) -> Self {
        ForgetReport {
            now,
            considered: 0,
            archived: Vec::new(),
            refused: Vec::new(),
            retained: 0,
            dry_run,
        }
    }
}

/// The free-text query a recall answers.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallQuery {
    /// What to look for.
    pub text: String,
    /// How many hits to return.
    pub k: usize,
    /// Restrict to these kinds; empty means every kind.
    pub kinds: Vec<MemoryKind>,
    /// Restrict to these tiers; empty means every tier.
    pub tiers: Vec<Tier>,
    /// The instant `recency` is measured from.
    pub now: Timestamp,
}

impl RecallQuery {
    /// A query for the `k` best hits as of `now`.
    pub fn new(text: impl Into<String>, k: usize, now: Timestamp) -> Self {
        RecallQuery {
            text: text.into(),
            k,
            kinds: Vec::new(),
            tiers: Vec::new(),
            now,
        }
    }

    /// Restrict the query to `kinds`.
    pub fn with_kinds(mut self, kinds: Vec<MemoryKind>) -> Self {
        self.kinds = kinds;
        self
    }

    /// Restrict the query to `tiers`.
    pub fn with_tiers(mut self, tiers: Vec<Tier>) -> Self {
        self.tiers = tiers;
        self
    }
}

/// One channel of the hybrid score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// FTS5/BM25.
    Lexical,
    /// Cosine similarity in the packed index.
    Vector,
    /// Entity PPR expansion.
    Graph,
    /// Time decay.
    Recency,
    /// The memory's declared importance.
    Importance,
}

/// Every channel, in the order the score sums them.
pub const CHANNELS: [Channel; 5] = [
    Channel::Lexical,
    Channel::Vector,
    Channel::Graph,
    Channel::Recency,
    Channel::Importance,
];

impl Channel {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Lexical => "lexical",
            Channel::Vector => "vector",
            Channel::Graph => "graph",
            Channel::Recency => "recency",
            Channel::Importance => "importance",
        }
    }
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One channel's contribution to a hit's score.
///
/// The parts are kept so a caller can audit *why* a memory was retrieved: a hit
/// that is right for the wrong reason is a retrieval bug that a bare score hides.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScorePart {
    /// Which channel.
    pub channel: Channel,
    /// The channel's value before weighting.
    pub raw: f64,
    /// The weight it was multiplied by.
    pub weight: f64,
    /// `raw * weight`.
    pub contribution: f64,
}

/// One recalled memory.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallHit {
    /// The memory.
    pub memory: Memory,
    /// Its hybrid score.
    pub score: f64,
    /// The per-channel breakdown.
    pub parts: Vec<ScorePart>,
}

/// The inputs to a utility score (plan §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UtilityInputs {
    /// How much keeping it changes future behavior, in `[0,1]`.
    pub future_behavior_impact: f32,
    /// How likely it is to be needed, in `[0,1]`.
    pub retrieval_probability: f32,
    /// How much it can be relied on, in `[0,1]`.
    pub reliability: f32,
    /// What it costs to keep, in the caller's unit; must be `> 0`.
    pub storage_cost: f64,
}

/// A filter over stored memories.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryFilter {
    /// Only these kinds; empty means all.
    pub kinds: Vec<MemoryKind>,
    /// Only these tiers; empty means all.
    pub tiers: Vec<Tier>,
    /// Only this status; `None` means active only.
    pub status: Option<RecordStatus>,
    /// At most this many rows.
    pub limit: Option<usize>,
}

impl MemoryFilter {
    /// Every active memory.
    pub fn all() -> Self {
        MemoryFilter::default()
    }

    /// Every memory, whatever its status.
    pub fn every_status() -> Self {
        MemoryFilter {
            status: None,
            ..MemoryFilter::default()
        }
    }

    /// Active memories of one kind.
    pub fn of_kind(kind: MemoryKind) -> Self {
        MemoryFilter {
            kinds: vec![kind],
            ..MemoryFilter::default()
        }
    }

    /// Restrict to a status.
    pub fn with_status(mut self, status: RecordStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Restrict to tiers.
    pub fn with_tiers(mut self, tiers: Vec<Tier>) -> Self {
        self.tiers = tiers;
        self
    }

    /// Restrict to kinds.
    pub fn with_kinds(mut self, kinds: Vec<MemoryKind>) -> Self {
        self.kinds = kinds;
        self
    }

    /// Cap the result count.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// The deterministic ULID for a stable external reference.
///
/// The bench corpus names its episodes with a stable `ref`, and the eval harness
/// has to map a gold answer back onto the memory it names without a lookup table.
/// Deriving the ULID from the reference makes that mapping a pure function, so a
/// gold file stays valid across runs and stores.
pub fn ref_ulid(reference: &str, at: Timestamp) -> Ulid {
    let mut hasher = Sha256::new();
    hasher.update(b"mm.memory.ref\0");
    hasher.update(reference.as_bytes());
    let digest = hasher.finalize();
    let mut random = [0u8; 16];
    random.copy_from_slice(&digest[..16]);
    let ms = at
        .seconds
        .saturating_mul(1000)
        .saturating_add(u64::from(at.nanos / 1_000_000));
    Ulid::from_parts(ms, u128::from_be_bytes(random))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(seconds: u64) -> Timestamp {
        Timestamp::from_epoch_seconds(seconds)
    }

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    #[test]
    fn every_kind_round_trips_its_wire_name_and_has_a_class() {
        for kind in MEMORY_KINDS {
            assert_eq!(MemoryKind::parse(kind.as_str()), Some(kind));
            assert!(!kind.rdf_class().is_empty());
        }
        assert_eq!(MemoryKind::parse("nonsense"), None);
        assert_eq!(
            MemoryKind::parse_list("semantic, episodic").unwrap(),
            vec![MemoryKind::Semantic, MemoryKind::Episodic]
        );
    }

    #[test]
    fn a_draft_memory_is_valid_and_self_provenanced() {
        let memory = Memory::draft(id(1), MemoryKind::Episodic, "a thing happened", ts(10)).unwrap();
        assert_eq!(memory.provenance, memory.id);
        assert_eq!(memory.recorded_ulid, memory.id);
        assert_eq!(memory.tier, Tier::Recall);
        assert!(memory.validity.is_open());
        assert_eq!(memory.content_hash().len(), 64);
    }

    #[test]
    fn validation_refuses_the_things_that_would_lose_provenance() {
        let nil = Ulid::nil();
        assert_eq!(
            Memory::draft(nil, MemoryKind::Semantic, "x", ts(1))
                .unwrap_err()
                .code(),
            "memory.validation"
        );
        let mut memory = Memory::draft(id(2), MemoryKind::Semantic, "x", ts(1)).unwrap();
        memory.provenance = nil;
        assert!(memory.validate().is_err());
        memory.provenance = memory.id;
        memory.content = "   ".into();
        assert!(memory.validate().is_err());
        memory.content = "x".repeat(MAX_CONTENT_CHARS + 1);
        assert!(memory.validate().is_err());
        memory.content = "ok".into();
        memory.confidence = 1.5;
        assert!(memory.validate().is_err());
        memory.confidence = 0.5;
        memory.importance = -0.1;
        assert!(memory.validate().is_err());
    }

    #[test]
    fn a_developmental_memory_must_be_protected() {
        let mut memory = Memory::draft(id(3), MemoryKind::Developmental, "I exist", ts(1)).unwrap();
        assert!(memory.validate().is_err(), "unprotected developmental memory");
        memory.protected = true;
        assert!(memory.validate().is_ok());
        assert!(memory.is_unforgettable());
    }

    #[test]
    fn a_closed_interval_must_not_run_backwards() {
        let memory = Memory::new(
            id(4),
            MemoryKind::Episodic,
            "x",
            TimeInterval {
                from: ts(10),
                until: Some(ts(5)),
            },
            0.5,
            0.5,
            id(4),
            ts(10),
        );
        assert!(memory.is_err());
    }

    #[test]
    fn intervals_are_half_open_on_the_right() {
        let interval = TimeInterval::closed(ts(10), ts(20));
        assert!(interval.contains(ts(10)));
        assert!(interval.contains(ts(19)));
        assert!(!interval.contains(ts(20)));
        assert_eq!(interval.duration_ns().unwrap(), 10 * 1_000_000_000);
        assert!(TimeInterval::open(ts(1)).contains(ts(1_000)));
    }

    #[test]
    fn link_kinds_round_trip() {
        for name in [
            "supports",
            "contradicts",
            "similar_to",
            "derived_from",
            "consolidated_from",
            "summarizes",
            "accessed_by",
            "related_entity",
            "serves_commitment",
        ] {
            assert_eq!(LinkKind::parse(name).unwrap().as_str(), name);
        }
        assert_eq!(LinkKind::parse("nope"), None);
    }

    #[test]
    fn tier_and_status_round_trip() {
        for tier in TIERS {
            assert_eq!(Tier::parse(tier.as_str()), Some(tier));
        }
        assert_eq!(RecordStatus::parse("archived"), Some(RecordStatus::Archived));
        assert_eq!(RecordStatus::parse("gone"), None);
        assert_eq!(
            ConsolidationMethod::parse("generalize"),
            Some(ConsolidationMethod::Generalize)
        );
    }

    #[test]
    fn a_reference_derives_a_stable_ulid() {
        let a = ref_ulid("ep-001", ts(1_700_000_000));
        let b = ref_ulid("ep-001", ts(1_700_000_000));
        let c = ref_ulid("ep-002", ts(1_700_000_000));
        assert_eq!(a, b, "the same reference must derive the same ULID");
        assert_ne!(a, c);
        // The ULID's timestamp part comes from the instant, so ordering is
        // meaningful rather than arbitrary.
        assert!(a < ref_ulid("ep-001", ts(1_700_000_100)));
    }
}
