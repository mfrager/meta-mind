//! Where memories live: SQLite rows, an FTS5 shadow index, and the entity graph.
//!
//! The `MemoryStore` trait is the plan's §4.3 interface; [`SqliteMemoryStore`] is
//! its implementation over the Phase 1 [`Tabular`] boundary. Two design choices
//! are worth stating, because both are visible from the outside:
//!
//! * **Lexical retrieval is the primary channel.** FTS5 is kept in step by
//!   triggers, so a memory is searchable the moment it is written and an archived
//!   memory stops matching without a second code path to remember to call.
//! * **Archival is an update, never a delete.** `archive` moves a row to
//!   `archived`. Nothing in this file issues a `DELETE` against `memories`, which
//!   is what makes "the being never deletes what it learned" a property of the
//!   implementation rather than a promise in a comment.
//!
//! Times cross this boundary as integer nanoseconds. [`Timestamp`] is
//! `(seconds, nanos)` and the DDL stores nanoseconds, so the two conversions
//! [`ns`] and [`timestamp_of`] are the only place the representation changes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::Logger;
use mm_store_sqlite::SqliteStore;
use serde_json::Value;

use crate::error::{MemoryError, Result};
use crate::model::{
    Community, Consolidation, CueKind, LinkKind, Memory, MemoryFilter, MemoryKind, MistakeRecord,
    RecordStatus, RetrievalCue, SummaryNode, Tier, TimeInterval,
};

/// The trait the plan specifies: the five operations a memory store must offer.
///
/// `record_access` carries the query hash as well as the trace, because the DDL
/// requires it and because "which query surfaced this memory" is the question a
/// retrieval audit asks first.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store a memory.
    async fn put(&self, memory: &Memory) -> Result<Ulid>;
    /// Fetch a memory.
    async fn get(&self, id: &Ulid) -> Result<Option<Memory>>;
    /// Link two memories.
    async fn link(&self, from: Ulid, to: Ulid, relation: LinkKind, weight: f32) -> Result<()>;
    /// Record that a memory was surfaced.
    async fn record_access(
        &self,
        id: Ulid,
        score: f64,
        query_hash: &str,
        trace: Ulid,
    ) -> Result<()>;
    /// Archive a memory. Never a delete.
    async fn archive(&self, id: Ulid) -> Result<()>;
}

/// The SQLite-backed memory store.
#[derive(Clone)]
pub struct SqliteMemoryStore {
    sqlite: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
}

impl std::fmt::Debug for SqliteMemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteMemoryStore")
            .field("path", &self.sqlite.path())
            .finish_non_exhaustive()
    }
}

impl SqliteMemoryStore {
    /// Build a store over an open SQLite handle.
    pub fn new(sqlite: SqliteStore, logger: Arc<Logger>, ids: Arc<UlidFactory>) -> Self {
        SqliteMemoryStore {
            sqlite,
            logger,
            ids,
        }
    }

    /// The underlying SQLite handle, for a caller that needs it.
    pub fn sqlite(&self) -> &SqliteStore {
        &self.sqlite
    }

    /// The logger this store writes to.
    pub fn logger(&self) -> &Arc<Logger> {
        &self.logger
    }

    /// The identifier factory this store mints ids from.
    pub fn ids(&self) -> &Arc<UlidFactory> {
        &self.ids
    }

    /// Mint a fresh ULID.
    pub fn next_id(&self) -> Ulid {
        self.ids.next()
    }

    async fn query(&self, sql: &str, args: Params) -> Result<Vec<Value>> {
        Tabular::query_json(&self.sqlite, sql, args)
            .await
            .map_err(Into::into)
    }

    async fn exec(&self, sql: &str, args: Params) -> Result<u64> {
        Tabular::execute(&self.sqlite, sql, args)
            .await
            .map_err(Into::into)
    }

    async fn scalar(&self, sql: &str, args: Params) -> Result<i64> {
        let rows = self.query(sql, args).await?;
        Ok(rows
            .first()
            .and_then(Value::as_object)
            .and_then(|map| map.values().next())
            .and_then(Value::as_i64)
            .unwrap_or(0))
    }

    // ------------------------------------------------------------------ writes --

    /// Store a memory, its cues, and its entity mentions.
    pub async fn insert(&self, memory: &Memory) -> Result<()> {
        memory.validate()?;
        self.exec(
            "INSERT INTO memories (id, kind, tier, content, source_ulid, confidence, importance, \
             utility, retention, valid_from, valid_until, recorded_at, recorded_ulid, provenance, \
             protected, status, content_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&memory.id)),
                Param::Text(memory.kind.as_str().to_string()),
                Param::Text(memory.tier.as_str().to_string()),
                Param::Text(memory.content.clone()),
                Param::opt_text(memory.source.map(|s| mm_core::ulid_string(&s))),
                Param::Real(f64::from(memory.confidence)),
                Param::Real(f64::from(memory.importance)),
                Param::Int(ns(memory.validity.from)),
                memory
                    .validity
                    .until
                    .map_or(Param::Null, |t| Param::Int(ns(t))),
                Param::Int(ns(memory.recorded_at)),
                Param::Text(mm_core::ulid_string(&memory.recorded_ulid)),
                Param::Text(mm_core::ulid_string(&memory.provenance)),
                Param::Int(i64::from(memory.protected)),
                Param::Text(memory.status.as_str().to_string()),
                Param::Text(memory.content_hash()),
            ],
        )
        .await?;
        for cue in &memory.cues {
            self.exec(
                "INSERT INTO memory_cues (id, memory_id, cue, cue_kind) VALUES (?, ?, ?, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&self.ids.next())),
                    Param::Text(mm_core::ulid_string(&memory.id)),
                    Param::Text(cue.text.clone()),
                    Param::Text(cue.kind.as_str().to_string()),
                ],
            )
            .await?;
        }
        for entity in &memory.entities {
            self.exec(
                "INSERT INTO memory_entities (id, memory_id, entity_ulid, role) VALUES (?, ?, ?, NULL)",
                vec![
                    Param::Text(mm_core::ulid_string(&self.ids.next())),
                    Param::Text(mm_core::ulid_string(&memory.id)),
                    Param::Text(mm_core::ulid_string(entity)),
                ],
            )
            .await?;
        }
        self.logger
            .audit(
                mm_log::Level::Info,
                mm_log::codes::MEMORY_ADD,
                crate::TARGET,
                Some(memory.id),
                serde_json::json!({
                    "memory_id": mm_core::ulid_string(&memory.id),
                    "kind": memory.kind.as_str(),
                    "tier": memory.tier.as_str(),
                    "content_hash": memory.content_hash(),
                    "confidence": memory.confidence,
                    "importance": memory.importance,
                    "protected": memory.protected,
                    "provenance": mm_core::ulid_string(&memory.provenance),
                }),
            )
            .await?;
        Ok(())
    }

    /// The id of an active memory with this exact content, if one exists.
    ///
    /// The comparison includes `valid_from`, because the same sentence learned
    /// again at a different time is a second episode, not a duplicate. `exclude`
    /// skips one id, which lets a caller screen a candidate that is already
    /// stored against everything *else* rather than against itself.
    pub async fn find_by_hash(
        &self,
        kind: MemoryKind,
        content_hash: &str,
        valid_from: Timestamp,
        exclude: Option<Ulid>,
    ) -> Result<Option<Ulid>> {
        let rows = self
            .query(
                "SELECT id FROM memories WHERE kind = ? AND content_hash = ? AND valid_from = ? \
                 ORDER BY id",
                vec![
                    Param::Text(kind.as_str().to_string()),
                    Param::Text(content_hash.to_string()),
                    Param::Int(ns(valid_from)),
                ],
            )
            .await?;
        for row in &rows {
            let Some(id) = row["id"].as_str() else {
                continue;
            };
            let id = parse_id(id)?;
            if Some(id) != exclude {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Link two memories.
    pub async fn insert_link(
        &self,
        from: Ulid,
        to: Ulid,
        relation: LinkKind,
        weight: f32,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO memory_links (id, from_id, to_id, relation, weight, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.ids.next())),
                Param::Text(mm_core::ulid_string(&from)),
                Param::Text(mm_core::ulid_string(&to)),
                Param::Text(relation.as_str().to_string()),
                Param::Real(f64::from(weight)),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
            ],
        )
        .await?;
        Ok(())
    }

    /// Record an access.
    pub async fn insert_access(
        &self,
        id: Ulid,
        score: f64,
        query_hash: &str,
        trace: Ulid,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO memory_access (id, memory_id, accessed_at, query_hash, score, trace_id) \
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.ids.next())),
                Param::Text(mm_core::ulid_string(&id)),
                Param::Int(ns(Timestamp::now())),
                Param::Text(query_hash.to_string()),
                Param::Real(score),
                Param::Text(mm_core::ulid_string(&trace)),
            ],
        )
        .await?;
        Ok(())
    }

    /// Raise a memory's importance and record the reinforcement as an access.
    pub async fn reinforce(&self, id: Ulid, evidence: f32) -> Result<()> {
        let bump = f64::from(evidence.clamp(0.0, 1.0)) * 0.1;
        let affected = self
            .exec(
                "UPDATE memories SET importance = min(1.0, importance + ?) WHERE id = ?",
                vec![Param::Real(bump), Param::Text(mm_core::ulid_string(&id))],
            )
            .await?;
        if affected == 0 {
            return Err(MemoryError::NotFound(mm_core::ulid_string(&id)));
        }
        let hash = mm_core::hash_fields(&["reinforce", &mm_core::ulid_string(&id)]);
        self.insert_access(id, bump, &hash, id).await?;
        self.logger
            .emit(
                mm_log::LogRecord::new(
                    mm_log::Level::Debug,
                    mm_log::codes::MEMORY_ACCESS,
                    crate::TARGET,
                )
                .with_field("memory_id", mm_core::ulid_string(&id))
                .with_field("score", bump)
                .with_field("query_hash", hash)
                .with_field("trace_id", mm_core::ulid_string(&id)),
            )
            .await?;
        Ok(())
    }

    /// Write the last-computed utility and retention for a memory.
    pub async fn set_scores(
        &self,
        id: Ulid,
        utility: Option<f64>,
        retention: Option<f64>,
    ) -> Result<()> {
        self.exec(
            "UPDATE memories SET utility = COALESCE(?, utility), retention = COALESCE(?, retention) \
             WHERE id = ?",
            vec![
                utility.map_or(Param::Null, Param::Real),
                retention.map_or(Param::Null, Param::Real),
                Param::Text(mm_core::ulid_string(&id)),
            ],
        )
        .await?;
        Ok(())
    }

    /// Record a consolidation step.
    pub async fn insert_consolidation(&self, c: &Consolidation) -> Result<()> {
        let sources: Vec<String> = c.source_ids.iter().map(mm_core::ulid_string).collect();
        self.exec(
            "INSERT INTO memory_consolidations (id, source_ids, target_id, method, created_ulid) \
             VALUES (?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&c.id)),
                Param::Text(serde_json::to_string(&sources)?),
                Param::Text(mm_core::ulid_string(&c.target_id)),
                Param::Text(c.method.as_str().to_string()),
                Param::Text(mm_core::ulid_string(&c.created_ulid)),
            ],
        )
        .await?;
        Ok(())
    }

    /// Record a summary-tree node.
    pub async fn insert_summary(&self, node: &SummaryNode) -> Result<()> {
        let members: Vec<String> = node.member_ids.iter().map(mm_core::ulid_string).collect();
        self.exec(
            "INSERT INTO memory_summaries (id, memory_id, parent_id, level, member_ids, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.ids.next())),
                Param::Text(mm_core::ulid_string(&node.memory_id)),
                Param::opt_text(node.parent_id.map(|p| mm_core::ulid_string(&p))),
                Param::Int(i64::from(node.level)),
                Param::Text(serde_json::to_string(&members)?),
                Param::Text(mm_core::ulid_string(&node.memory_id)),
            ],
        )
        .await?;
        Ok(())
    }

    /// Record a community summary.
    pub async fn insert_community(&self, community: &Community) -> Result<()> {
        let members: Vec<String> = community
            .member_ids
            .iter()
            .map(mm_core::ulid_string)
            .collect();
        self.exec(
            "INSERT INTO memory_communities (id, memory_id, label, member_ids, modularity, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.ids.next())),
                Param::Text(mm_core::ulid_string(&community.memory_id)),
                Param::Text(community.label.clone()),
                Param::Text(serde_json::to_string(&members)?),
                community.modularity.map_or(Param::Null, Param::Real),
                Param::Text(mm_core::ulid_string(&community.memory_id)),
            ],
        )
        .await?;
        Ok(())
    }

    /// Record a mistake's detail row.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_mistake(
        &self,
        id: Ulid,
        memory_id: Ulid,
        situation: Option<Ulid>,
        decision: Option<Ulid>,
        outcome: Option<Ulid>,
        failure_mode: &str,
        root_cause: Option<Ulid>,
        missed_signal: &[String],
        corrective_rule: Option<Ulid>,
        recurrence_risk: f32,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO mistakes (id, memory_id, situation_ulid, decision_ulid, outcome_ulid, \
             failure_mode, root_cause_ulid, missed_signal, corrective_rule_ulid, recurrence_risk, \
             created_ulid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&id)),
                Param::Text(mm_core::ulid_string(&memory_id)),
                Param::opt_text(situation.map(|s| mm_core::ulid_string(&s))),
                Param::opt_text(decision.map(|s| mm_core::ulid_string(&s))),
                Param::opt_text(outcome.map(|s| mm_core::ulid_string(&s))),
                Param::Text(failure_mode.to_string()),
                Param::opt_text(root_cause.map(|s| mm_core::ulid_string(&s))),
                Param::Text(serde_json::to_string(missed_signal)?),
                Param::opt_text(corrective_rule.map(|s| mm_core::ulid_string(&s))),
                Param::Real(f64::from(recurrence_risk)),
                Param::Text(mm_core::ulid_string(&id)),
            ],
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------------- reads --

    /// Fetch one memory by id.
    pub async fn fetch(&self, id: &Ulid) -> Result<Option<Memory>> {
        let rows = self
            .query(
                "SELECT * FROM memories WHERE id = ?",
                vec![Param::Text(mm_core::ulid_string(id))],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let memories = self.hydrate(std::slice::from_ref(row)).await?;
        Ok(memories.into_iter().next())
    }

    /// Every memory matching a filter, ordered by ULID.
    pub async fn list(&self, filter: &MemoryFilter) -> Result<Vec<Memory>> {
        let mut sql = String::from("SELECT * FROM memories WHERE 1 = 1");
        let mut args: Params = Vec::new();
        if !filter.kinds.is_empty() {
            sql.push_str(" AND kind IN (");
            push_placeholders(&mut sql, &mut args, filter.kinds.len());
            for kind in &filter.kinds {
                args.push(Param::Text(kind.as_str().to_string()));
            }
        }
        if !filter.tiers.is_empty() {
            sql.push_str(" AND tier IN (");
            push_placeholders(&mut sql, &mut args, filter.tiers.len());
            for tier in &filter.tiers {
                args.push(Param::Text(tier.as_str().to_string()));
            }
        }
        match filter.status {
            Some(status) => {
                sql.push_str(" AND status = ?");
                args.push(Param::Text(status.as_str().to_string()));
            }
            None => {
                sql.push_str(" AND status = 'active'");
            }
        }
        sql.push_str(" ORDER BY id");
        if let Some(limit) = filter.limit {
            sql.push_str(" LIMIT ?");
            args.push(Param::Int(limit as i64));
        }
        let rows = self.query(&sql, args).await?;
        self.hydrate(&rows).await
    }

    /// Every memory, whatever its status.
    pub async fn list_all(&self) -> Result<Vec<Memory>> {
        let rows = self
            .query("SELECT * FROM memories ORDER BY id", Vec::new())
            .await?;
        self.hydrate(&rows).await
    }

    /// Attach cues and entities to a set of memory rows.
    async fn hydrate(&self, rows: &[Value]) -> Result<Vec<Memory>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let cues = self
            .query(
                "SELECT memory_id, cue, cue_kind FROM memory_cues ORDER BY id",
                Vec::new(),
            )
            .await?;
        let mut cues_by_memory: BTreeMap<String, Vec<RetrievalCue>> = BTreeMap::new();
        for row in &cues {
            let (Some(memory), Some(text), Some(kind)) = (
                row["memory_id"].as_str(),
                row["cue"].as_str(),
                row["cue_kind"].as_str().and_then(CueKind::parse),
            ) else {
                continue;
            };
            cues_by_memory
                .entry(memory.to_string())
                .or_default()
                .push(RetrievalCue {
                    kind,
                    text: text.to_string(),
                });
        }
        let entities = self
            .query(
                "SELECT memory_id, entity_ulid FROM memory_entities ORDER BY id",
                Vec::new(),
            )
            .await?;
        let mut entities_by_memory: BTreeMap<String, Vec<Ulid>> = BTreeMap::new();
        for row in &entities {
            let (Some(memory), Some(entity)) =
                (row["memory_id"].as_str(), row["entity_ulid"].as_str())
            else {
                continue;
            };
            if let Ok(entity) = parse_id(entity) {
                entities_by_memory
                    .entry(memory.to_string())
                    .or_default()
                    .push(entity);
            }
        }

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(memory) = memory_of(row) else {
                continue;
            };
            let key = mm_core::ulid_string(&memory.id);
            let mut memory = memory;
            memory.cues = cues_by_memory.remove(&key).unwrap_or_default();
            memory.entities = entities_by_memory.remove(&key).unwrap_or_default();
            out.push(memory);
        }
        Ok(out)
    }

    /// Lexical candidates from FTS5, best first.
    ///
    /// Returns `(memory id, bm25 rank)`. SQLite's `bm25()` is negative with more
    /// negative meaning a better match, so the order is ascending.
    pub async fn lexical_search(&self, text: &str, limit: usize) -> Result<Vec<(Ulid, f64)>> {
        let Some(query) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let rows = self
            .query(
                "SELECT m.id AS id, bm25(memory_fts) AS rank \
                 FROM memory_fts JOIN memories m ON m.rowid = memory_fts.rowid \
                 WHERE memory_fts MATCH ? AND m.status = 'active' \
                 ORDER BY rank ASC, m.id ASC LIMIT ?",
                vec![Param::Text(query), Param::Int(limit as i64)],
            )
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let (Some(id), Some(rank)) = (row["id"].as_str(), row["rank"].as_f64()) else {
                continue;
            };
            out.push((parse_id(id)?, rank));
        }
        Ok(out)
    }

    /// How many times a memory has been surfaced.
    pub async fn access_count(&self, id: &Ulid) -> Result<u32> {
        let count = self
            .scalar(
                "SELECT count(*) FROM memory_access WHERE memory_id = ?",
                vec![Param::Text(mm_core::ulid_string(id))],
            )
            .await?;
        Ok(count.max(0) as u32)
    }

    /// The links leaving a memory, filtered by relation when one is given.
    pub async fn links_of(
        &self,
        id: &Ulid,
        relation: Option<LinkKind>,
    ) -> Result<Vec<(Ulid, f32)>> {
        let (sql, args) = match relation {
            Some(relation) => (
                "SELECT to_id, weight FROM memory_links WHERE from_id = ? AND relation = ? \
                 ORDER BY to_id",
                vec![
                    Param::Text(mm_core::ulid_string(id)),
                    Param::Text(relation.as_str().to_string()),
                ],
            ),
            None => (
                "SELECT to_id, weight FROM memory_links WHERE from_id = ? ORDER BY to_id",
                vec![Param::Text(mm_core::ulid_string(id))],
            ),
        };
        let rows = self.query(sql, args).await?;
        let mut out = Vec::new();
        for row in &rows {
            let (Some(to), Some(weight)) = (row["to_id"].as_str(), row["weight"].as_f64()) else {
                continue;
            };
            out.push((parse_id(to)?, weight as f32));
        }
        Ok(out)
    }

    /// Every link, for the `/memory` mirror.
    pub async fn list_links(&self) -> Result<Vec<(Ulid, Ulid, LinkKind, f32)>> {
        let rows = self
            .query(
                "SELECT from_id, to_id, relation, weight FROM memory_links ORDER BY id",
                Vec::new(),
            )
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let (Some(from), Some(to), Some(relation)) = (
                row["from_id"].as_str(),
                row["to_id"].as_str(),
                row["relation"].as_str().and_then(LinkKind::parse),
            ) else {
                continue;
            };
            out.push((
                parse_id(from)?,
                parse_id(to)?,
                relation,
                row["weight"].as_f64().unwrap_or(0.0) as f32,
            ));
        }
        Ok(out)
    }

    /// Every entity mention, grouped by entity.
    pub async fn entities_by_entity(&self) -> Result<BTreeMap<Ulid, Vec<Ulid>>> {
        let rows = self
            .query(
                "SELECT entity_ulid, memory_id FROM memory_entities ORDER BY entity_ulid, memory_id",
                Vec::new(),
            )
            .await?;
        let mut out: BTreeMap<Ulid, Vec<Ulid>> = BTreeMap::new();
        for row in &rows {
            let (Some(entity), Some(memory)) =
                (row["entity_ulid"].as_str(), row["memory_id"].as_str())
            else {
                continue;
            };
            out.entry(parse_id(entity)?)
                .or_default()
                .push(parse_id(memory)?);
        }
        Ok(out)
    }

    /// The entities one memory mentions.
    pub async fn entities_of(&self, id: &Ulid) -> Result<Vec<Ulid>> {
        let rows = self
            .query(
                "SELECT entity_ulid FROM memory_entities WHERE memory_id = ? ORDER BY entity_ulid",
                vec![Param::Text(mm_core::ulid_string(id))],
            )
            .await?;
        rows.iter()
            .filter_map(|row| row["entity_ulid"].as_str())
            .map(parse_id)
            .collect()
    }

    /// Every consolidation step.
    pub async fn list_consolidations(&self) -> Result<Vec<Consolidation>> {
        let rows = self
            .query(
                "SELECT id, source_ids, target_id, method, created_ulid FROM memory_consolidations \
                 ORDER BY id",
                Vec::new(),
            )
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let (Some(id), Some(target), Some(method)) = (
                row["id"].as_str(),
                row["target_id"].as_str(),
                row["method"]
                    .as_str()
                    .and_then(crate::model::ConsolidationMethod::parse),
            ) else {
                continue;
            };
            let sources: Vec<String> =
                serde_json::from_str(row["source_ids"].as_str().unwrap_or("[]"))
                    .unwrap_or_default();
            out.push(Consolidation {
                id: parse_id(id)?,
                source_ids: sources
                    .iter()
                    .map(|s| parse_id(s))
                    .collect::<Result<Vec<_>>>()?,
                target_id: parse_id(target)?,
                method,
                created_ulid: row["created_ulid"]
                    .as_str()
                    .map(parse_id)
                    .transpose()?
                    .unwrap_or(Ulid::nil()),
            });
        }
        Ok(out)
    }

    /// Every summary-tree node.
    pub async fn list_summaries(&self) -> Result<Vec<SummaryNode>> {
        let rows = self
            .query(
                "SELECT memory_id, parent_id, level, member_ids FROM memory_summaries ORDER BY id",
                Vec::new(),
            )
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let Some(memory) = row["memory_id"].as_str() else {
                continue;
            };
            let members: Vec<String> =
                serde_json::from_str(row["member_ids"].as_str().unwrap_or("[]"))
                    .unwrap_or_default();
            out.push(SummaryNode {
                memory_id: parse_id(memory)?,
                parent_id: row["parent_id"].as_str().map(parse_id).transpose()?,
                level: row["level"].as_u64().unwrap_or(0) as u32,
                member_ids: members
                    .iter()
                    .map(|s| parse_id(s))
                    .collect::<Result<Vec<_>>>()?,
            });
        }
        Ok(out)
    }

    /// Every community summary.
    pub async fn list_communities(&self) -> Result<Vec<Community>> {
        let rows = self
            .query(
                "SELECT memory_id, label, member_ids, modularity FROM memory_communities ORDER BY id",
                Vec::new(),
            )
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let (Some(memory), Some(label)) = (row["memory_id"].as_str(), row["label"].as_str())
            else {
                continue;
            };
            let members: Vec<String> =
                serde_json::from_str(row["member_ids"].as_str().unwrap_or("[]"))
                    .unwrap_or_default();
            out.push(Community {
                memory_id: parse_id(memory)?,
                label: label.to_string(),
                member_ids: members
                    .iter()
                    .map(|s| parse_id(s))
                    .collect::<Result<Vec<_>>>()?,
                modularity: row["modularity"].as_f64(),
            });
        }
        Ok(out)
    }

    /// Every mistake detail row.
    pub async fn list_mistakes(&self) -> Result<Vec<MistakeRecord>> {
        let rows = self
            .query(
                "SELECT id, memory_id, failure_mode, root_cause_ulid, missed_signal, \
                 corrective_rule_ulid, recurrence_risk FROM mistakes ORDER BY id",
                Vec::new(),
            )
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let (Some(id), Some(memory), Some(mode)) = (
                row["id"].as_str(),
                row["memory_id"].as_str(),
                row["failure_mode"].as_str(),
            ) else {
                continue;
            };
            out.push(MistakeRecord {
                id: parse_id(id)?,
                memory_id: parse_id(memory)?,
                failure_mode: mode.to_string(),
                root_cause: row["root_cause_ulid"].as_str().map(parse_id).transpose()?,
                missed_signal: serde_json::from_str(row["missed_signal"].as_str().unwrap_or("[]"))
                    .unwrap_or_default(),
                corrective_rule: row["corrective_rule_ulid"]
                    .as_str()
                    .map(parse_id)
                    .transpose()?,
                recurrence_risk: row["recurrence_risk"].as_f64().unwrap_or(0.0) as f32,
            });
        }
        Ok(out)
    }

    /// The number of rows in a table, or in `memories` when `None`.
    pub async fn count_of(&self, table: Option<&str>) -> Result<usize> {
        let table = table.unwrap_or("memories");
        let count = self
            .scalar(&format!("SELECT count(*) FROM {table}"), Vec::new())
            .await?;
        Ok(count.max(0) as usize)
    }

    /// The number of rows a filter matches.
    pub async fn count(&self, filter: &MemoryFilter) -> Result<usize> {
        Ok(self.list(filter).await?.len())
    }

    /// Memory ids that are archived even though something protects them.
    ///
    /// Must always be zero. It is checked rather than assumed because archival is
    /// a plain `UPDATE`, and a plain `UPDATE` is exactly the kind of write a later
    /// phase adds a second caller to.
    pub async fn archived_but_protected(&self) -> Result<usize> {
        let count = self
            .scalar(
                "SELECT count(*) FROM memories WHERE status = 'archived' \
                 AND (protected = 1 OR kind = 'developmental')",
                Vec::new(),
            )
            .await?;
        Ok(count.max(0) as usize)
    }

    /// Consolidation sources that name a memory that does not exist.
    pub async fn dangling_consolidation_sources(&self) -> Result<usize> {
        let known: BTreeSet<Ulid> = self.list_all().await?.into_iter().map(|m| m.id).collect();
        let mut dangling = 0usize;
        for step in self.list_consolidations().await? {
            for source in step.source_ids {
                if !known.contains(&source) {
                    dangling += 1;
                }
            }
        }
        Ok(dangling)
    }

    /// An open commitment a memory serves, if any.
    pub async fn open_commitment_for(&self, memory: &Ulid) -> Result<Option<String>> {
        let rows = self
            .query(
                "SELECT c.id AS id FROM memory_links l JOIN commitments c ON c.id = l.to_id \
                 WHERE l.from_id = ? AND l.relation = 'serves_commitment' AND c.status = 'active' \
                 ORDER BY c.id LIMIT 1",
                vec![Param::Text(mm_core::ulid_string(memory))],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row["id"].as_str())
            .map(str::to_string))
    }

    /// The instant a memory was recorded.
    pub async fn recorded_at(&self, id: &Ulid) -> Result<Option<Timestamp>> {
        let rows = self
            .query(
                "SELECT recorded_at FROM memories WHERE id = ?",
                vec![Param::Text(mm_core::ulid_string(id))],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row["recorded_at"].as_i64())
            .map(timestamp_of))
    }

    /// The last-computed retention for a memory.
    pub async fn retention_of(&self, id: &Ulid) -> Result<Option<f64>> {
        let rows = self
            .query(
                "SELECT retention FROM memories WHERE id = ?",
                vec![Param::Text(mm_core::ulid_string(id))],
            )
            .await?;
        Ok(rows.first().and_then(|row| row["retention"].as_f64()))
    }

    /// How many direct sources a memory has through `consolidated_from`.
    pub async fn consolidation_sources(&self, target: &Ulid) -> Result<usize> {
        let mut count = 0usize;
        for step in self.list_consolidations().await? {
            if step.target_id == *target {
                count += step.source_ids.len();
            }
        }
        Ok(count)
    }
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    async fn put(&self, memory: &Memory) -> Result<Ulid> {
        self.insert(memory).await?;
        Ok(memory.id)
    }

    async fn get(&self, id: &Ulid) -> Result<Option<Memory>> {
        self.fetch(id).await
    }

    async fn link(&self, from: Ulid, to: Ulid, relation: LinkKind, weight: f32) -> Result<()> {
        self.insert_link(from, to, relation, weight).await
    }

    async fn record_access(
        &self,
        id: Ulid,
        score: f64,
        query_hash: &str,
        trace: Ulid,
    ) -> Result<()> {
        self.insert_access(id, score, query_hash, trace).await
    }

    async fn archive(&self, id: Ulid) -> Result<()> {
        let affected = self
            .exec(
                "UPDATE memories SET status = 'archived', tier = 'archival' WHERE id = ?",
                vec![Param::Text(mm_core::ulid_string(&id))],
            )
            .await?;
        if affected == 0 {
            return Err(MemoryError::NotFound(mm_core::ulid_string(&id)));
        }
        Ok(())
    }
}

// --------------------------------------------------------------------- helpers --

/// Parse a ULID out of a column, as a memory error rather than a kernel one.
pub fn parse_id(text: &str) -> Result<Ulid> {
    mm_core::id::parse_ulid(text)
        .map_err(|e| MemoryError::Codec(format!("invalid ULID {text:?}: {e}")))
}

/// Nanoseconds since the epoch, as the DDL stores them.
pub fn ns(time: Timestamp) -> i64 {
    time.as_nanos() as i64
}

/// Rebuild a timestamp from the nanoseconds the DDL stores.
pub fn timestamp_of(nanos: i64) -> Timestamp {
    let nanos = nanos.max(0) as u128;
    Timestamp {
        seconds: (nanos / 1_000_000_000) as u64,
        nanos: (nanos % 1_000_000_000) as u32,
    }
}

fn push_placeholders(sql: &mut String, args: &mut Params, count: usize) {
    for i in 0..count {
        if i > 0 {
            sql.push_str(", ");
        }
        sql.push('?');
    }
    sql.push(')');
    let _ = args;
}

/// Turn free text into an FTS5 `MATCH` expression, or `None` when there is
/// nothing searchable in it.
///
/// Each token is double-quoted, so a query containing `AND`, `*`, or a stray
/// parenthesis is a term rather than syntax. Titles are normalized to lowercase
/// ASCII alphanumerics, which is exactly how the index tokenized them.
pub fn fts_query(text: &str) -> Option<String> {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// Lowercase ASCII-alphanumeric tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn memory_of(row: &Value) -> Option<Memory> {
    let id = row["id"].as_str().and_then(|s| parse_id(s).ok())?;
    let kind = row["kind"].as_str().and_then(MemoryKind::parse)?;
    let tier = row["tier"].as_str().and_then(Tier::parse)?;
    let status = row["status"]
        .as_str()
        .and_then(RecordStatus::parse)
        .unwrap_or(RecordStatus::Active);
    let valid_from = timestamp_of(row["valid_from"].as_i64()?);
    let valid_until = row["valid_until"].as_i64().map(timestamp_of);
    Some(Memory {
        id,
        status,
        kind,
        tier,
        content: row["content"].as_str()?.to_string(),
        source: row["source_ulid"].as_str().map(parse_id).transpose().ok()?,
        confidence: row["confidence"].as_f64().unwrap_or(0.0) as f32,
        importance: row["importance"].as_f64().unwrap_or(0.0) as f32,
        validity: TimeInterval {
            from: valid_from,
            until: valid_until,
        },
        recorded_at: timestamp_of(row["recorded_at"].as_i64()?),
        recorded_ulid: row["recorded_ulid"]
            .as_str()
            .and_then(|s| parse_id(s).ok())?,
        provenance: row["provenance"].as_str().and_then(|s| parse_id(s).ok())?,
        entities: Vec::new(),
        cues: Vec::new(),
        protected: row["protected"].as_i64().unwrap_or(0) != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nanoseconds_round_trip() {
        let time = Timestamp::from_rfc3339("2026-10-08T00:00:00.123456789Z").unwrap();
        assert_eq!(timestamp_of(ns(time)), time);
        assert_eq!(ns(Timestamp::EPOCH), 0);
        // A negative value can only come from a corrupt row; it clamps rather than
        // wrapping into a wild instant.
        assert_eq!(timestamp_of(-5), Timestamp::EPOCH);
    }

    #[test]
    fn an_fts_query_quotes_every_term() {
        let query = fts_query("Rust build, pinned!").unwrap();
        assert_eq!(query, "\"rust\" OR \"build\" OR \"pinned\"");
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query("!!!"), None);
    }

    #[test]
    fn a_query_with_fts_syntax_is_not_syntax() {
        // Anything that would be an operator becomes a phrase, so a user cannot
        // (and an attacker cannot) inject FTS5 grammar through a recall query.
        let query = fts_query("NEAR(a b) OR *").unwrap();
        assert!(!query.contains('('), "{query}");
        assert!(!query.contains('*'), "{query}");
    }

    #[test]
    fn tokenizing_normalizes_case_and_punctuation() {
        assert_eq!(tokenize("Hello, World_2!"), vec!["hello", "world", "2"]);
        assert!(tokenize("---").is_empty());
    }
}
