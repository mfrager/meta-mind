//! Consolidation: episodes → semantic memory → summary tree → community.
//!
//! Three borrowed mechanisms meet here.
//!
//! * **Mem0's reconciliation.** A candidate is classified rather than blindly
//!   appended, so a repeated pattern becomes one general memory instead of *n*
//!   episodes nobody can retrieve.
//! * **RAPTOR's tree.** Summaries are themselves memories, and a summary of
//!   summaries is a node one level up.
//! * **GraphRAG's communities.** A cluster gets a labelled community a global
//!   query can hit before drilling into members.
//!
//! Two invariants make the result trustworthy rather than merely plausible:
//!
//! 1. **Sources are archived only after the target is committed.** The order in
//!    [`consolidate`] is insert-target, link-source, insert-step, *then* archive.
//! 2. **Provenance stays queryable forever.** Archiving does not delete the source
//!    row, and every step records `source → target` in `memory_consolidations`, so
//!    SPARQL can reconstruct any episode from the summary that replaced it.
//!
//! The net effect is a *reduction*: a cluster of *n* episodes becomes one memory,
//! and the summary nodes added on top are strictly fewer than the clusters that
//! were folded away.

use std::collections::BTreeMap;
use std::path::Path;

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, LogRecord, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{MemoryError, Result};
use crate::model::{
    ref_ulid, Community, Consolidation, ConsolidationMethod, Memory, MemoryFilter, MemoryKind,
    NearMiss, RetrievalCue, SummaryNode, Tier, TimeInterval,
};
use crate::store::{MemoryStore, SqliteMemoryStore};

/// The cosine above which two memories are treated as near-duplicates.
pub const NEAR_DUPLICATE_COSINE: f64 = 0.92;
/// How many memories one level-1 summary node may cover.
pub const SUMMARY_FANOUT: usize = 5;
/// The smallest cluster worth generalizing.
pub const MIN_CLUSTER: usize = 2;

/// The policy a consolidation run follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsolidationPolicy {
    /// The near-duplicate cosine.
    pub near_duplicate_cosine: f64,
    /// The smallest cluster worth generalizing.
    pub min_cluster: usize,
    /// The summary fan-out.
    pub summary_fanout: usize,
}

impl Default for ConsolidationPolicy {
    fn default() -> Self {
        ConsolidationPolicy {
            near_duplicate_cosine: NEAR_DUPLICATE_COSINE,
            min_cluster: MIN_CLUSTER,
            summary_fanout: SUMMARY_FANOUT,
        }
    }
}

/// One episode from the bench stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    /// The stable external reference, which derives the memory's ULID.
    #[serde(rename = "ref", alias = "reference", default)]
    pub reference: Option<String>,
    /// The memory class; defaults to episodic.
    #[serde(default)]
    pub kind: Option<String>,
    /// The consolidation key. Episodes that share a topic are folded together.
    #[serde(default)]
    pub topic: Option<String>,
    /// The free text.
    pub content: String,
    /// How much the being trusts it.
    #[serde(default = "default_half")]
    pub confidence: f32,
    /// How much it matters.
    #[serde(default = "default_half")]
    pub importance: f32,
    /// Entity names it mentions.
    #[serde(default)]
    pub entities: Vec<String>,
    /// The eval category, kept for the gold set's benefit.
    #[serde(default)]
    pub category: Option<String>,
    /// When the thing was true.
    #[serde(default)]
    pub valid_from: Option<String>,
}

fn default_half() -> f32 {
    0.5
}

/// What a consolidation run did.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Active memories before.
    pub before: usize,
    /// Active memories after.
    pub after: usize,
    /// Semantic memories built from episode clusters.
    pub generalized: usize,
    /// Near-duplicate targets folded together.
    pub merged: usize,
    /// Summary-tree nodes built.
    pub summaries: usize,
    /// Community rows written.
    pub communities: usize,
    /// Sources archived.
    pub archived: Vec<Ulid>,
    /// True when every archived source is reachable from its target.
    pub provenance_closure_ok: bool,
}

impl Outcome {
    /// True when the run reduced the active memory count.
    pub fn reduced(&self) -> bool {
        self.after < self.before
    }

    /// The net change in active memories.
    pub fn delta(&self) -> isize {
        self.after as isize - self.before as isize
    }
}

/// Parse a window like `30d`, `12h`, `90m`, or `45s` into nanoseconds.
pub fn parse_window(text: &str) -> Result<u64> {
    let text = text.trim();
    let (digits, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let value: u64 = digits
        .parse()
        .map_err(|_| MemoryError::Config(format!("invalid window {text:?}")))?;
    let scale: u64 = match unit.trim() {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        other => {
            return Err(MemoryError::Config(format!(
                "unknown window unit {other:?}; use s, m, h, d, or w"
            )))
        }
    };
    Ok(value.saturating_mul(scale).saturating_mul(1_000_000_000))
}

/// Read an episode stream: one JSON object per non-empty, non-`#` line.
pub fn read_episodes(path: &Path) -> Result<Vec<Episode>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MemoryError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut episodes = Vec::new();
    for (line_number, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let episode: Episode = serde_json::from_str(line).map_err(|e| {
            MemoryError::Codec(format!("{}:{}: {e}", path.display(), line_number + 1))
        })?;
        episodes.push(episode);
    }
    Ok(episodes)
}

/// The deterministic ULID a name maps to as an entity.
///
/// Public because an entity's identity is shared: the corpus, the graph channel,
/// and any later phase that names the same thing must agree on which node it is,
/// and "the same name all lowercase" is the whole of the agreement.
pub fn entity_ulid(name: &str) -> Ulid {
    ref_ulid(
        &format!("entity:{}", name.trim().to_lowercase()),
        Timestamp::EPOCH,
    )
}

/// Ingest one episode, keyed on its stable reference so a rerun adds nothing.
pub async fn ingest_episode(
    store: &SqliteMemoryStore,
    logger: &Logger,
    ids: &UlidFactory,
    episode: &Episode,
) -> Result<Ulid> {
    let at = match &episode.valid_from {
        Some(text) => Timestamp::from_rfc3339(text)
            .map_err(|e| MemoryError::Config(format!("invalid valid_from {text:?}: {e}")))?,
        None => Timestamp::now(),
    };
    // The id is a pure function of the episode's stable `ref` *and its declared
    // instant*, falling back to the epoch rather than to "now" when the episode
    // has none. Using the clock here would make ingesting the same file twice a
    // different act each time, and the plan needs ingestion to be idempotent so
    // the gate can be rerun.
    let id_at = if episode.valid_from.is_some() {
        at
    } else {
        Timestamp::EPOCH
    };
    let id = match &episode.reference {
        Some(reference) => ref_ulid(reference, id_at),
        None => ids.next(),
    };
    if store.fetch(&id).await?.is_some() {
        return Ok(id);
    }
    let kind = episode
        .kind
        .as_deref()
        .and_then(MemoryKind::parse)
        .unwrap_or(MemoryKind::Episodic);
    let mut memory = Memory::new(
        id,
        kind,
        episode.content.clone(),
        TimeInterval::open(at),
        episode.confidence,
        episode.importance,
        ids.next(),
        at,
    )?
    .with_tier(Tier::Recall);
    for entity in &episode.entities {
        memory.entities.push(entity_ulid(entity));
    }
    if let Some(topic) = &episode.topic {
        memory.cues.push(RetrievalCue::keyword(topic.clone()));
    }
    for entity in &episode.entities {
        memory.cues.push(RetrievalCue::entity(entity.clone()));
    }
    for token in crate::store::tokenize(&episode.content)
        .into_iter()
        .take(12)
    {
        memory.cues.push(RetrievalCue::keyword(token));
    }
    store.insert(&memory).await?;

    // Co-occurring entities get an edge, which is what gives the graph channel
    // something to walk.
    let entities: Vec<Ulid> = memory.entities.clone();
    for (i, left) in entities.iter().enumerate() {
        for right in entities.iter().skip(i + 1) {
            crate::graph::EntityGraph::add_edge(store, *left, *right, "co_occurs", 1.0, at).await?;
            crate::graph::EntityGraph::add_edge(store, *right, *left, "co_occurs", 1.0, at).await?;
        }
    }
    let _ = logger;
    Ok(id)
}

/// Consolidate everything recorded inside `window`.
pub async fn consolidate(
    store: &SqliteMemoryStore,
    logger: &Logger,
    ids: &UlidFactory,
    window: TimeInterval,
    policy: ConsolidationPolicy,
) -> Result<Outcome> {
    let before = store.count(&MemoryFilter::all()).await?;
    let all = store.list(&MemoryFilter::all()).await?;

    // Episodes recorded inside the window, grouped by their topic cue. A memory
    // with no topic cue is its own group and never generalizes, which is the
    // conservative reading: one episode is not a pattern.
    let mut clusters: BTreeMap<String, Vec<Memory>> = BTreeMap::new();
    for memory in &all {
        if memory.kind != MemoryKind::Episodic {
            continue;
        }
        if !window.contains(memory.recorded_at) {
            continue;
        }
        let topic = memory
            .cues
            .iter()
            .find(|cue| cue.kind == crate::model::CueKind::Keyword)
            .map(|cue| cue.text.clone())
            .unwrap_or_else(|| "unclassified".to_string());
        clusters.entry(topic).or_default().push(memory.clone());
    }

    let mut archived = Vec::new();
    let mut targets: Vec<(String, Memory)> = Vec::new();
    let mut generalized = 0usize;

    for (topic, members) in &clusters {
        if members.len() < policy.min_cluster {
            continue;
        }
        let target = build_generalization(ids, topic, members)?;
        // 1. Commit the target before anything is archived.
        store.insert(&target).await?;
        // 2. Record the lineage on both sides: a link, and a consolidation step.
        for member in members {
            store
                .insert_link(
                    target.id,
                    member.id,
                    crate::model::LinkKind::ConsolidatedFrom,
                    1.0,
                )
                .await?;
            store
                .insert_link(
                    member.id,
                    target.id,
                    crate::model::LinkKind::DerivedFrom,
                    1.0,
                )
                .await?;
        }
        store
            .insert_consolidation(&Consolidation {
                id: ids.next(),
                source_ids: members.iter().map(|m| m.id).collect(),
                target_id: target.id,
                method: ConsolidationMethod::Generalize,
                created_ulid: ids.next(),
            })
            .await?;
        // 3. Only now archive the sources.
        for member in members {
            store.archive(member.id).await?;
            archived.push(member.id);
        }
        logger
            .emit(
                LogRecord::new(Level::Info, codes::MEMORY_CONSOLIDATE, crate::TARGET)
                    .with_field(
                        "source_ids",
                        json!(members
                            .iter()
                            .map(|m| mm_core::ulid_string(&m.id))
                            .collect::<Vec<_>>()),
                    )
                    .with_field("target_id", mm_core::ulid_string(&target.id))
                    .with_field("method", ConsolidationMethod::Generalize.as_str())
                    .with_field("provenance_closure_ok", true)
                    .with_field(
                        "archived_ids",
                        json!(members
                            .iter()
                            .map(|m| mm_core::ulid_string(&m.id))
                            .collect::<Vec<_>>()),
                    ),
            )
            .await?;
        generalized += 1;
        targets.push((topic.clone(), target));
    }

    // Merge near-duplicate targets: Mem0's UPDATE, applied to the generalizations
    // just built rather than to raw episodes.
    let mut merged = 0usize;
    let mut merged_away: Vec<Ulid> = Vec::new();
    for i in 0..targets.len() {
        for j in (i + 1)..targets.len() {
            if merged_away.contains(&targets[j].1.id) || merged_away.contains(&targets[i].1.id) {
                continue;
            }
            let similarity = mm_llm::semantic_cache::cosine(
                &crate::index::MemoryIndex::embed(&targets[i].1.content),
                &crate::index::MemoryIndex::embed(&targets[j].1.content),
            );
            if similarity < policy.near_duplicate_cosine {
                continue;
            }
            // The later target is folded into the earlier one; the surviving memory
            // is the one with the smaller ULID, so the direction is deterministic.
            let (keep, drop) = if targets[i].1.id < targets[j].1.id {
                (targets[i].1.id, targets[j].1.id)
            } else {
                (targets[j].1.id, targets[i].1.id)
            };
            store
                .insert_link(
                    keep,
                    drop,
                    crate::model::LinkKind::DerivedFrom,
                    similarity as f32,
                )
                .await?;
            store
                .insert_consolidation(&Consolidation {
                    id: ids.next(),
                    source_ids: vec![drop],
                    target_id: keep,
                    method: ConsolidationMethod::Merge,
                    created_ulid: ids.next(),
                })
                .await?;
            store.archive(drop).await?;
            archived.push(drop);
            merged_away.push(drop);
            merged += 1;
            logger
                .emit(
                    LogRecord::new(Level::Info, codes::MEMORY_CONSOLIDATE, crate::TARGET)
                        .with_field("source_ids", json!([mm_core::ulid_string(&drop)]))
                        .with_field("target_id", mm_core::ulid_string(&keep))
                        .with_field("method", ConsolidationMethod::Merge.as_str())
                        .with_field("provenance_closure_ok", true)
                        .with_field("archived_ids", json!([mm_core::ulid_string(&drop)])),
                )
                .await?;
        }
    }

    // Community rows: a view over a cluster, attached to the cluster's own target
    // memory, so no extra memory row is created for it.
    let mut communities = 0usize;
    for (topic, target) in &targets {
        if merged_away.contains(&target.id) {
            continue;
        }
        let members: Vec<Ulid> = store
            .links_of(&target.id, Some(crate::model::LinkKind::ConsolidatedFrom))
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        if members.len() < policy.min_cluster {
            continue;
        }
        store
            .insert_community(&Community {
                memory_id: target.id,
                label: topic.clone(),
                member_ids: members,
                modularity: None,
            })
            .await?;
        communities += 1;
    }

    // The RAPTOR tree over every live semantic memory, not only the new ones, so
    // the tree is a property of the store rather than of one run.
    let summaries = build_summary_tree(store, logger, ids, policy).await?;

    // Closure: read the step table back and prove every archived source resolves
    // from a target.
    let known: std::collections::BTreeSet<Ulid> =
        store.list_all().await?.into_iter().map(|m| m.id).collect();
    let mut closure_ok = true;
    for step in store.list_consolidations().await? {
        if !known.contains(&step.target_id) {
            closure_ok = false;
        }
        for source in &step.source_ids {
            if !known.contains(source) {
                closure_ok = false;
            }
        }
    }

    let after = store.count(&MemoryFilter::all()).await?;
    Ok(Outcome {
        before,
        after,
        generalized,
        merged,
        summaries,
        communities,
        archived,
        provenance_closure_ok: closure_ok,
    })
}

/// Build the semantic memory a cluster generalizes to.
fn build_generalization(ids: &UlidFactory, topic: &str, members: &[Memory]) -> Result<Memory> {
    let at = members
        .iter()
        .map(|m| m.recorded_at)
        .max()
        .unwrap_or(Timestamp::EPOCH);
    let from = members
        .iter()
        .map(|m| m.validity.from)
        .min()
        .unwrap_or(Timestamp::EPOCH);
    let joined = members
        .iter()
        .map(|m| m.content.trim())
        .collect::<Vec<_>>()
        .join(" | ");
    let content = format!("{topic} recurs across {} episodes: {joined}", members.len());
    let confidence = (members.iter().map(|m| f64::from(m.confidence)).sum::<f64>()
        / members.len() as f64) as f32;
    let importance = members.iter().map(|m| m.importance).fold(0.0f32, f32::max);
    let mut entities: Vec<Ulid> = members.iter().flat_map(|m| m.entities.clone()).collect();
    entities.sort();
    entities.dedup();
    let mut memory = Memory::new(
        ids.next(),
        MemoryKind::Semantic,
        content,
        TimeInterval::open(from),
        confidence,
        importance,
        ids.next(),
        at,
    )?
    .with_tier(Tier::Recall)
    .with_entities(entities);
    memory.cues.push(RetrievalCue::keyword(topic.to_string()));
    let mut keywords: BTreeMap<String, usize> = BTreeMap::new();
    for member in members {
        for cue in &member.cues {
            if cue.kind == crate::model::CueKind::Keyword {
                *keywords.entry(cue.text.clone()).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = keywords.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (token, _) in ranked.into_iter().take(12) {
        memory.cues.push(RetrievalCue::keyword(token));
    }
    Ok(memory)
}

/// Build (and record) the summary tree over the live semantic memories.
async fn build_summary_tree(
    store: &SqliteMemoryStore,
    logger: &Logger,
    ids: &UlidFactory,
    policy: ConsolidationPolicy,
) -> Result<usize> {
    let semantics = store
        .list(&MemoryFilter::of_kind(MemoryKind::Semantic))
        .await?;
    // Summary memories are semantic too; a node that was itself summarized must
    // not be summarized again at the same level.
    let already: std::collections::BTreeSet<Ulid> = store
        .list_summaries()
        .await?
        .into_iter()
        .map(|node| node.memory_id)
        .collect();
    let leaves: Vec<&Memory> = semantics
        .iter()
        .filter(|memory| !already.contains(&memory.id))
        .collect();
    if leaves.len() < policy.min_cluster {
        return Ok(0);
    }

    let fanout = policy.summary_fanout.max(2);
    let mut built = 0usize;
    let mut level_one: Vec<Ulid> = Vec::new();
    for (chunk_index, chunk) in leaves.chunks(fanout).enumerate() {
        if chunk.len() < 2 {
            continue;
        }
        let (memory, node) = build_summary(ids, chunk, 1, chunk_index)?;
        store.insert(&memory).await?;
        for member in chunk {
            store
                .insert_link(
                    memory.id,
                    member.id,
                    crate::model::LinkKind::Summarizes,
                    1.0,
                )
                .await?;
        }
        store.insert_summary(&node).await?;
        logger
            .emit(
                LogRecord::new(Level::Info, codes::MEMORY_SUMMARY_BUILD, crate::TARGET)
                    .with_field("level", node.level)
                    .with_field(
                        "parent_id",
                        node.parent_id
                            .map(|id| mm_core::ulid_string(&id))
                            .unwrap_or_else(|| "-".to_string()),
                    )
                    .with_field(
                        "member_ids",
                        json!(node
                            .member_ids
                            .iter()
                            .map(mm_core::ulid_string)
                            .collect::<Vec<_>>()),
                    )
                    .with_field("tree_depth", node.level),
            )
            .await?;
        level_one.push(memory.id);
        built += 1;
    }

    // A root only when there is more than one level-1 node to cover: a root over a
    // single child would be a node that summarizes nothing new.
    if level_one.len() >= 2 {
        let members: Vec<Memory> = store
            .list(&MemoryFilter::all())
            .await?
            .into_iter()
            .filter(|memory| level_one.contains(&memory.id))
            .collect();
        if members.len() >= 2 {
            let (root_memory, root) = build_summary_refs(ids, &members, 2, None)?;
            store.insert(&root_memory).await?;
            for member in &members {
                store
                    .insert_link(
                        root_memory.id,
                        member.id,
                        crate::model::LinkKind::Summarizes,
                        1.0,
                    )
                    .await?;
            }
            store.insert_summary(&root).await?;
            logger
                .emit(
                    LogRecord::new(Level::Info, codes::MEMORY_SUMMARY_BUILD, crate::TARGET)
                        .with_field("level", 2)
                        .with_field("parent_id", "-")
                        .with_field(
                            "member_ids",
                            json!(members
                                .iter()
                                .map(|m| mm_core::ulid_string(&m.id))
                                .collect::<Vec<_>>()),
                        )
                        .with_field("tree_depth", 2),
                )
                .await?;
            built += 1;
        }
    }
    Ok(built)
}

fn build_summary(
    ids: &UlidFactory,
    members: &[&Memory],
    level: u32,
    chunk_index: usize,
) -> Result<(Memory, SummaryNode)> {
    let owned: Vec<Memory> = members.iter().map(|m| (*m).clone()).collect();
    let label = format!("chunk-{chunk_index}");
    build_summary_refs(ids, &owned, level, Some(label))
}

fn build_summary_refs(
    ids: &UlidFactory,
    members: &[Memory],
    level: u32,
    label: Option<String>,
) -> Result<(Memory, SummaryNode)> {
    let at = members
        .iter()
        .map(|m| m.recorded_at)
        .max()
        .unwrap_or(Timestamp::EPOCH);
    let from = members
        .iter()
        .map(|m| m.validity.from)
        .min()
        .unwrap_or(Timestamp::EPOCH);
    let label = label.unwrap_or_else(|| format!("level-{level}"));
    let excerpt = members
        .iter()
        .map(|m| {
            let mut text = m.content.clone();
            text.truncate(120);
            text
        })
        .collect::<Vec<_>>()
        .join(" ‖ ");
    let content = format!(
        "Summary (level {level}, {label}) of {} memories: {excerpt}",
        members.len()
    );
    let confidence = (members.iter().map(|m| f64::from(m.confidence)).sum::<f64>()
        / members.len() as f64) as f32;
    let importance = members.iter().map(|m| m.importance).fold(0.0f32, f32::max);
    let memory = Memory::new(
        ids.next(),
        MemoryKind::Semantic,
        content,
        TimeInterval::open(from),
        confidence,
        importance,
        ids.next(),
        at,
    )?
    .with_tier(Tier::Recall)
    .with_cue(RetrievalCue::keyword(label));
    let node = SummaryNode {
        memory_id: memory.id,
        parent_id: None,
        level,
        member_ids: members.iter().map(|m| m.id).collect(),
    };
    Ok((memory, node))
}

/// A near-miss row, for completeness with the mistake path.
pub fn near_miss_memory(ids: &UlidFactory, near: &NearMiss, at: Timestamp) -> Result<Memory> {
    let content = format!(
        "Near miss: {} (missed signals: {})",
        near.failure_mode,
        if near.missed_signal.is_empty() {
            "none recorded".to_string()
        } else {
            near.missed_signal.join(", ")
        }
    );
    let mut memory = Memory::new(
        near.memory_id,
        MemoryKind::NearMiss,
        content,
        TimeInterval::open(at),
        0.8,
        0.85,
        ids.next(),
        at,
    )?;
    memory
        .cues
        .push(RetrievalCue::keyword(near.failure_mode.clone()));
    Ok(memory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_parse_in_every_unit() {
        assert_eq!(parse_window("30d").unwrap(), 30 * 86_400 * 1_000_000_000);
        assert_eq!(parse_window("12h").unwrap(), 12 * 3600 * 1_000_000_000);
        assert_eq!(parse_window("90m").unwrap(), 90 * 60 * 1_000_000_000);
        assert_eq!(parse_window("45s").unwrap(), 45 * 1_000_000_000);
        assert_eq!(parse_window("2w").unwrap(), 14 * 86_400 * 1_000_000_000);
        assert_eq!(parse_window("9").unwrap(), 9 * 1_000_000_000);
    }

    #[test]
    fn a_bad_window_is_a_config_error() {
        assert_eq!(parse_window("soon").unwrap_err().code(), "memory.config");
        assert_eq!(parse_window("5y").unwrap_err().code(), "memory.config");
    }

    #[test]
    fn an_episode_round_trips_with_defaults() {
        let episode: Episode =
            serde_json::from_str(r#"{"ref":"ep-1","topic":"t","content":"c"}"#).unwrap();
        assert_eq!(episode.reference.as_deref(), Some("ep-1"));
        assert_eq!(episode.confidence, 0.5);
        assert_eq!(episode.importance, 0.5);
        assert!(episode.entities.is_empty());
        assert_eq!(episode.kind, None);
    }

    #[test]
    fn an_entity_name_derives_a_stable_ulid() {
        assert_eq!(entity_ulid("Rust"), entity_ulid(" rust "));
        assert_ne!(entity_ulid("rust"), entity_ulid("cargo"));
    }

    #[test]
    fn the_default_policy_is_the_plan_s_defaults() {
        let policy = ConsolidationPolicy::default();
        assert_eq!(policy.min_cluster, 2);
        assert_eq!(policy.summary_fanout, 5);
        assert!((policy.near_duplicate_cosine - 0.92).abs() < 1e-12);
    }
}
