//! The tool-facing memory service.
//!
//! Explicit remember/recall/forget records live in the unified `memory` graph
//! beside the four-tier pipeline. A record carries the tool contract and is
//! filtered by its payload shape, so the pipeline's own entities are not
//! mistaken for explicit memories.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::RdfEngine;
use crate::memory::contract::{
    ForgetRequest, ForgetSelector, MemoryContractError, MemoryRecord, MemoryScope,
    MemoryScopeCount, MemoryService, MemoryStatus, RecallQuery, RecallResult, RememberRequest,
};
use crate::rdf::{self, new_id, now_ms, MMD_NS};

/// The `memory` graph alias.
pub const MEMORY_GRAPH: &str = "memory";
/// `mm:Memory`.
pub const MM_MEMORY: &str = "https://metamind.dev/ontology#Memory";
const SCHEMA: &str = "mm.memory.record.v1";

/// RDF-backed explicit memory records.
#[derive(Debug, Clone)]
pub struct RdfMemoryService {
    engine: Arc<RdfEngine>,
}

impl RdfMemoryService {
    /// Build over the data engine.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    async fn records(&self) -> Result<Vec<MemoryRecord>, MemoryContractError> {
        let values: Vec<Value> = rdf::list_as(&self.engine, MEMORY_GRAPH, MM_MEMORY)
            .await
            .map_err(store_error)?;
        let mut records: Vec<MemoryRecord> = values
            .into_iter()
            .filter(|value| value.get("schema").and_then(Value::as_str) == Some(SCHEMA))
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect();
        records.sort_by(|a, b| (b.updated_at, &b.id).cmp(&(a.updated_at, &a.id)));
        Ok(records)
    }

    async fn write(&self, record: &MemoryRecord) -> Result<(), MemoryContractError> {
        let mut value = serde_json::to_value(record)
            .map_err(|error| MemoryContractError::Unavailable(format!("encode memory: {error}")))?;
        if let Value::Object(map) = &mut value {
            map.insert("schema".into(), Value::String(SCHEMA.into()));
        }
        rdf::upsert(
            &self.engine,
            &crate::graphs::resolve_graph(MEMORY_GRAPH),
            &format!("{MMD_NS}memory/{}", record.id),
            MM_MEMORY,
            Some(&record.key),
            &value,
        )
        .await
        .map_err(store_error)
    }

    fn active(record: &MemoryRecord) -> bool {
        record.expires_at.is_none_or(|expires| expires > now_ms())
    }
}

fn store_error(error: impl std::fmt::Display) -> MemoryContractError {
    MemoryContractError::Unavailable(error.to_string())
}

#[async_trait]
impl MemoryService for RdfMemoryService {
    async fn recall(&self, query: RecallQuery) -> Result<RecallResult, MemoryContractError> {
        query.validate()?;
        let needle = query.query.to_lowercase();
        let now = now_ms();
        let mut matched: Vec<(f32, MemoryRecord)> = self
            .records()
            .await?
            .into_iter()
            .filter(Self::active)
            .filter(|record| query.scopes.contains(&record.scope))
            .filter(|record| query.kinds.is_empty() || query.kinds.contains(&record.kind))
            .filter_map(|record| {
                let haystack = format!("{} {}", record.key, record.value).to_lowercase();
                if !haystack.contains(&needle) {
                    return None;
                }
                let token_hits = needle
                    .split_whitespace()
                    .filter(|token| haystack.contains(token))
                    .count() as f32;
                let recency =
                    1.0 / (1.0 + ((now - record.updated_at).max(0) as f32 / 86_400_000.0));
                Some((token_hits + record.confidence + recency, record))
            })
            .collect();
        matched.sort_by(|a, b| b.0.total_cmp(&a.0));
        let truncated = matched.len() > query.limit;
        matched.truncate(query.limit);
        Ok(RecallResult {
            records: matched.into_iter().map(|(_, record)| record).collect(),
            truncated,
        })
    }

    async fn remember(
        &self,
        request: RememberRequest,
    ) -> Result<MemoryRecord, MemoryContractError> {
        request.validate()?;
        if crate::privacy::contains_secret(&request.value)
            || crate::privacy::contains_secret(&request.key)
        {
            return Err(MemoryContractError::Invalid(
                "memory content looks like a secret and was refused".into(),
            ));
        }
        let now = now_ms();
        let records = self.records().await?;
        let existing = records
            .iter()
            .find(|record| {
                record.scope == request.scope
                    && record.kind == request.kind
                    && record.key == request.key
                    && Self::active(record)
            })
            .cloned();
        let record = if let Some(mut record) = existing {
            record.value = request.value;
            record.source = request.source;
            record.confidence = request.confidence.unwrap_or(record.confidence);
            record.expires_at = request.expires_at;
            record.updated_at = now;
            record
        } else {
            MemoryRecord {
                id: new_id(),
                scope: request.scope,
                kind: request.kind,
                key: request.key,
                value: request.value,
                source: request.source,
                confidence: request.confidence.unwrap_or(0.5),
                created_at: now,
                updated_at: now,
                expires_at: request.expires_at,
            }
        };
        record.validate()?;
        self.write(&record).await?;
        Ok(record)
    }

    async fn forget(&self, request: ForgetRequest) -> Result<u64, MemoryContractError> {
        if request.reason.trim().is_empty() {
            return Err(MemoryContractError::Invalid(
                "forget requires an audit reason".into(),
            ));
        }
        let now = now_ms();
        let mut records = self.records().await?;
        let mut changed = 0;
        for record in records.iter_mut().filter(|record| Self::active(record)) {
            let selected = match &request.selector {
                ForgetSelector::Id { id } => &record.id == id,
                ForgetSelector::Key { scope, key } => record.scope == *scope && record.key == *key,
            };
            if selected {
                record.expires_at = Some(now);
                record.updated_at = now;
                self.write(record).await?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    async fn status(&self) -> Result<MemoryStatus, MemoryContractError> {
        let records: Vec<MemoryRecord> = self
            .records()
            .await?
            .into_iter()
            .filter(Self::active)
            .collect();
        let mut counts: BTreeMap<MemoryScope, u64> = BTreeMap::new();
        for record in &records {
            *counts.entry(record.scope).or_default() += 1;
        }
        Ok(MemoryStatus {
            records: records.len() as u64,
            by_scope: counts
                .into_iter()
                .map(|(scope, count)| MemoryScopeCount { scope, count })
                .collect(),
            last_updated_at: records.iter().map(|record| record.updated_at).max(),
            backend: "rdf:memory".into(),
        })
    }
}
