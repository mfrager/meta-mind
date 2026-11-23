//! `DataService`: the one writer to managed RDF, its outbox, and the index.
//!
//! The write order is fixed and meaningful:
//!
//! 1. validate the intent and allocate a `dataRevision`;
//! 2. write the outbox row (pending) with that revision;
//! 3. commit the RDF batch;
//! 4. attempt the index synchronously and mark the row done, or leave it
//!    pending and report `INDEX_PENDING`.
//!
//! RDF is authority. A failed index never rolls back a write, and a stale index
//! hit is verified against RDF before it is returned.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mm_search::{SearchIndex, SearchQuery};
use oxrdf::Quad;
use sqlx::SqlitePool;

use crate::engine::RdfEngine;
use crate::rdf::{lit, new_id, now_ms, MMD_NS, MM_NS};
use crate::sqlite::SqliteStore;

use super::namespaces::{NamespaceRegistry, BUILTINS, MINTED_KINDS};
use super::outbox::{DataOutbox, OutboxEntry};
use super::rdf_mapper::{
    entity_from_quads, entity_quads, graph_for_class, known_class, label_of, local_name,
    search_document, validate_literal, vocabulary_labels, FOAF_NAME, MM_NAME, RDFS_LABEL,
    SCHEMA_NAME, SYSTEM_PREDICATES,
};
use super::schema::{SchemaCatalog, MANAGED_GRAPHS};
use super::{
    DataBroker, DataCreateRequest, DataDeleteMode, DataDeleteRequest, DataEntity, DataError,
    DataFilter, DataMutationResult, DataPatch, DataQuery, DataQueryResult, DataSearchRequest,
    DataUpdateMode, DataUpdateRequest, DataValue, EntityStatus, IndexHealth, NamespaceBinding,
    SchemaDetail, SchemaInclude, SchemaSummary, SearchHit, SearchResult, ShaclViolation,
    ValidationTarget,
};

/// Classes whose writes belong to one owning service, not to `data.*`.
pub const MANAGEMENT_CLASSES: &[&str] = &[
    "Schema",
    "Namespace",
    "Plan",
    "PlanVersion",
    "PlanStep",
    "PlanRegistry",
    "Thought",
    "ScheduledAction",
    "CronJob",
    "Session",
    "SessionTrace",
    "ConversationFrame",
    "FrameUtterance",
    "LogEntry",
];

/// Classes that may never be hard-deleted.
pub const PROTECTED_CLASSES: &[&str] = &["Being", "Identity", "DevelopmentalMemory"];

/// Checkpoint after this many mutations.
pub const CHECKPOINT_MUTATIONS: u64 = 256;
/// Or this many milliseconds, whichever comes first.
pub const CHECKPOINT_INTERVAL_MS: i64 = 60_000;

/// The data-root subpath of the RDF/XML snapshot.
pub const SNAPSHOT_FILE: &str = "cubecast/metamind.rdf";
/// The lossless N-Quads backup.
pub const BACKUP_FILE: &str = "cubecast/metamind.nq";

/// The single managed-data seam.
pub struct DataService {
    engine: Arc<RdfEngine>,
    search: Option<Arc<SearchIndex>>,
    outbox: DataOutbox,
    catalog: SchemaCatalog,
    namespaces: NamespaceRegistry,
    pool: SqlitePool,
    base_dir: PathBuf,
    revision: AtomicU64,
    mutations: AtomicU64,
    last_checkpoint: AtomicI64,
    index_ready: AtomicBool,
    vocabulary: Mutex<BTreeMap<String, String>>,
}

impl std::fmt::Debug for DataService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataService")
            .field("base_dir", &self.base_dir)
            .field("search", &self.search.is_some())
            .finish_non_exhaustive()
    }
}

impl DataService {
    /// Open the service: bootstrap the catalog and namespaces, open the index,
    /// import a snapshot into an empty store, and reconcile the outbox.
    pub async fn open(
        engine: Arc<RdfEngine>,
        sqlite: &SqliteStore,
        base_dir: &Path,
    ) -> Result<Self, DataError> {
        let catalog = SchemaCatalog::new(Arc::clone(&engine));
        catalog.bootstrap().await?;
        let namespaces = NamespaceRegistry::new(Arc::clone(&engine));
        namespaces.bootstrap().await?;
        let migrated = namespaces.migrate_legacy_identities().await?;
        if migrated > 0 {
            tracing::info!(
                aliases = migrated,
                "migrated legacy user/project identities"
            );
        }

        let outbox = DataOutbox::new(sqlite.pool().clone());
        outbox.migrate().await?;

        let search = SearchIndex::open(&base_dir.join("search").join(mm_search::INDEX_DIR))
            .map(Arc::new)
            .map_err(|error| {
                tracing::warn!(%error, "search index unavailable; data writes remain durable");
                error
            })
            .ok();

        let revision = sqlx::query_scalar::<_, String>(
            "SELECT value FROM data_meta WHERE key = 'dataRevision'",
        )
        .fetch_optional(&sqlite.pool().clone())
        .await?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .max(outbox.max_seq().await?);

        let service = Self {
            engine,
            search,
            outbox,
            catalog,
            namespaces,
            pool: sqlite.pool().clone(),
            base_dir: base_dir.to_path_buf(),
            revision: AtomicU64::new(revision),
            mutations: AtomicU64::new(0),
            last_checkpoint: AtomicI64::new(now_ms()),
            index_ready: AtomicBool::new(false),
            vocabulary: Mutex::new(BTreeMap::new()),
        };
        service.refresh_vocabulary().await?;
        service.import_snapshot_if_empty().await?;
        service.startup_reconcile().await?;
        Ok(service)
    }

    /// The RDF engine, for callers that need a read path.
    pub fn engine(&self) -> &Arc<RdfEngine> {
        &self.engine
    }

    /// The schema catalog.
    pub fn catalog(&self) -> &SchemaCatalog {
        &self.catalog
    }

    /// The namespace registry.
    pub fn namespaces(&self) -> &NamespaceRegistry {
        &self.namespaces
    }

    /// Record aliases for legacy user/project IRIs. Idempotent; returns the
    /// number of distinct legacy spellings found in the named graphs.
    pub async fn migrate_legacy_identities(&self) -> Result<usize, DataError> {
        self.namespaces.migrate_legacy_identities().await
    }

    /// The outbox.
    pub fn outbox(&self) -> &DataOutbox {
        &self.outbox
    }

    /// The index, when it opened.
    pub fn search_index(&self) -> Option<&Arc<SearchIndex>> {
        self.search.as_ref()
    }

    /// The current `dataRevision`.
    pub fn data_revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    /// Whether search is ready to answer.
    pub fn index_ready(&self) -> bool {
        self.index_ready.load(Ordering::SeqCst)
    }

    /// Index pending rows now.
    pub async fn process_outbox(&self) -> Result<usize, DataError> {
        let Some(search) = &self.search else {
            return Ok(0);
        };
        let now = now_ms();
        let entries = self.outbox.pending(now, 100).await?;
        let mut applied = 0usize;
        for entry in entries {
            match self.apply_entry(search, &entry).await {
                Ok(()) => {
                    self.outbox.mark_done(entry.seq).await?;
                    applied += 1;
                }
                Err(error) => {
                    let attempts = entry.attempts + 1;
                    let backoff = (1_000i64 << attempts.min(6)).min(60_000);
                    self.outbox
                        .mark_failed(entry.seq, attempts, now + backoff)
                        .await?;
                    tracing::warn!(seq = entry.seq, %error, "index attempt failed");
                }
            }
        }
        Ok(applied)
    }

    /// Rebuild the whole index from RDF.
    pub async fn reindex(&self) -> Result<usize, DataError> {
        let Some(search) = &self.search else {
            return Err(DataError::SearchUnavailable("the index is not open".into()));
        };
        let entities = self.all_entities().await?;
        let mut latest: BTreeMap<String, DataEntity> = BTreeMap::new();
        for entity in entities {
            latest
                .entry(entity.iri.clone())
                .and_modify(|current| {
                    if entity.revision > current.revision {
                        *current = entity.clone();
                    }
                })
                .or_insert(entity);
        }
        let documents = latest
            .values()
            .filter(|entity| entity.status != EntityStatus::Deleted)
            .map(|entity| self.document_for(entity))
            .collect::<Result<Vec<_>, _>>()?;
        let report = search.reindex(documents).await?;
        self.index_ready.store(true, Ordering::SeqCst);
        Ok(report.documents)
    }

    /// Write the RDF/XML snapshot and the N-Quads backup atomically.
    pub async fn checkpoint(&self) -> Result<(), DataError> {
        let dir = self.base_dir.join("cubecast");
        std::fs::create_dir_all(&dir)?;
        let mut quads = Vec::new();
        for alias in MANAGED_GRAPHS {
            for quad in self.engine.store().quads(alias).await? {
                quads.push(quad);
            }
        }
        let mut annotations = Vec::new();
        let mut annotated: BTreeSet<(String, String)> = BTreeSet::new();
        for quad in &quads {
            if let Some(graph) = named_graph(quad) {
                let subject = super::rdf_mapper::subject_str(&quad.subject);
                if annotated.insert((graph.to_string(), subject.clone())) {
                    annotations.push(RdfEngine::quad(
                        graph,
                        &subject,
                        super::rdf_mapper::MM_SOURCE_GRAPH,
                        lit(graph.to_string()),
                    ));
                }
            }
        }
        quads.extend(annotations);
        let rdfxml = mm_store_graph::dump_quads_rdfxml(quads.into_iter().map(Ok))
            .map_err(|e| DataError::Unavailable(e.to_string()))?;
        let nquads = self
            .engine
            .store()
            .dump_nquads()
            .await
            .map_err(|e| DataError::Unavailable(e.to_string()))?;
        write_atomic(&self.base_dir.join(SNAPSHOT_FILE), rdfxml.as_bytes())?;
        write_atomic(&self.base_dir.join(BACKUP_FILE), nquads.as_bytes())?;
        self.mutations.store(0, Ordering::SeqCst);
        self.last_checkpoint.store(now_ms(), Ordering::SeqCst);
        Ok(())
    }

    /// Flush the outbox and checkpoint, then wait for the writer to stop.
    pub async fn shutdown(&self) -> Result<(), DataError> {
        let _ = self.process_outbox().await;
        self.checkpoint().await?;
        self.engine
            .store()
            .shutdown()
            .await
            .map_err(|e| DataError::Unavailable(e.to_string()))?;
        Ok(())
    }

    /// Checkpoint when the mutation/time threshold is met.
    pub async fn checkpoint_if_due(&self) -> Result<(), DataError> {
        let mutations = self.mutations.load(Ordering::SeqCst);
        let elapsed = now_ms() - self.last_checkpoint.load(Ordering::SeqCst);
        if mutations >= CHECKPOINT_MUTATIONS || (mutations > 0 && elapsed >= CHECKPOINT_INTERVAL_MS)
        {
            self.checkpoint().await?;
        }
        Ok(())
    }

    /// Compare the RDF entity set with the lexical index.
    ///
    /// A *missing* document is an RDF entity that search cannot find; an
    /// *orphan* is an indexed subject with no live RDF entity. Deleted entities
    /// are excluded from both sides, matching [`DataService::reindex`] and the
    /// outbox's deletion rule.
    pub async fn verify_index(&self) -> Result<IndexVerification, DataError> {
        let Some(search) = &self.search else {
            return Err(DataError::SearchUnavailable("the index is not open".into()));
        };
        let entities = self.all_entities().await?;
        let mut latest: BTreeMap<String, DataEntity> = BTreeMap::new();
        for entity in entities {
            latest
                .entry(entity.iri.clone())
                .and_modify(|current| {
                    if entity.revision > current.revision {
                        *current = entity.clone();
                    }
                })
                .or_insert(entity);
        }
        let expected = latest
            .into_values()
            .filter(|entity| entity.status != EntityStatus::Deleted)
            .map(|entity| entity.iri)
            .collect::<BTreeSet<_>>();
        let indexed = search
            .subjects()
            .await?
            .into_iter()
            .collect::<BTreeSet<_>>();
        Ok(IndexVerification {
            entities: expected.len() as u64,
            documents: indexed.len() as u64,
            missing: expected.difference(&indexed).cloned().collect(),
            orphaned: indexed.difference(&expected).cloned().collect(),
        })
    }

    /// The compact status projection used by later phases and the inspector.
    pub async fn status(&self) -> Result<DataStatus, DataError> {
        let entities = self.all_entities().await?;
        let pending = self.outbox.pending_count().await?;
        let documents = match &self.search {
            Some(search) => search.num_docs().await.unwrap_or(0),
            None => 0,
        };
        Ok(DataStatus {
            entities: entities.len() as u64,
            search_documents: documents,
            index_pending: pending,
            index_ready: self.index_ready(),
            data_revision: self.data_revision(),
            active_schemas: self
                .catalog
                .list()
                .await?
                .into_iter()
                .filter(|schema| schema.active)
                .map(|schema| schema.prefix)
                .collect(),
            last_mutation: entities.iter().map(|entity| entity.updated_at).max(),
        })
    }

    /// Load one entity by IRI, searching the managed graphs.
    pub async fn find_entity(&self, iri: &str) -> Result<Option<DataEntity>, DataError> {
        let iri = self.namespaces.normalize(iri).await?;
        for alias in MANAGED_GRAPHS {
            let graph = crate::graphs::store_graph(alias);
            let quads = self.engine.store().quads_for_subject(&graph, &iri).await?;
            if let Some(entity) = entity_from_quads(&iri, &graph, &quads) {
                return Ok(Some(entity));
            }
        }
        Ok(None)
    }

    /// Whether a hard delete carries an authority the broker accepts.
    fn hard_delete_allowed(authority: Option<&str>) -> bool {
        matches!(authority, Some("user" | "system" | "operator"))
    }

    async fn startup_reconcile(&self) -> Result<(), DataError> {
        let _ = self.process_outbox().await?;
        let Some(search) = &self.search else {
            self.index_ready.store(false, Ordering::SeqCst);
            return Ok(());
        };
        let count = search.num_docs().await.unwrap_or(0);
        self.index_ready.store(true, Ordering::SeqCst);
        if count == 0 {
            let entities = self.all_entities().await?;
            if !entities.is_empty() {
                let documents = entities
                    .iter()
                    .map(|entity| self.document_for(entity))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Err(error) = search.reindex(documents).await {
                    self.index_ready.store(false, Ordering::SeqCst);
                    tracing::warn!(%error, "startup reindex failed");
                }
            }
        }
        Ok(())
    }

    async fn import_snapshot_if_empty(&self) -> Result<(), DataError> {
        if self.instance_count().await? > 0 {
            return Ok(());
        }
        let path = self.base_dir.join(SNAPSHOT_FILE);
        if !path.exists() {
            return Ok(());
        }
        let bytes =
            std::fs::read(&path).map_err(|error| DataError::Unavailable(error.to_string()))?;
        let triples = mm_store_graph::parse_rdfxml(&bytes)
            .map_err(|error| DataError::Unavailable(error.to_string()))?;
        let mut graph_by_subject: BTreeMap<String, String> = BTreeMap::new();
        for triple in &triples {
            if triple.predicate.as_str() == super::rdf_mapper::MM_SOURCE_GRAPH {
                if let oxrdf::Term::Literal(literal) = &triple.object {
                    graph_by_subject.insert(
                        super::rdf_mapper::subject_str(&triple.subject),
                        literal.value().to_string(),
                    );
                }
            }
        }
        let world = crate::graphs::store_graph("world");
        let quads = triples
            .into_iter()
            .map(|triple| {
                let graph = graph_by_subject
                    .get(&super::rdf_mapper::subject_str(&triple.subject))
                    .cloned()
                    .unwrap_or_else(|| world.clone());
                Quad::new(
                    triple.subject,
                    triple.predicate,
                    triple.object,
                    oxrdf::GraphName::NamedNode(oxrdf::NamedNode::new_unchecked(graph)),
                )
            })
            .collect::<Vec<_>>();
        if !quads.is_empty() {
            self.engine.insert_all(quads).await?;
        }
        Ok(())
    }

    async fn instance_count(&self) -> Result<usize, DataError> {
        let mut count = 0usize;
        for alias in MANAGED_GRAPHS.iter().filter(|alias| **alias != "schema") {
            count += self.engine.store().graph_quad_count(alias).await?;
        }
        Ok(count)
    }

    async fn all_entities(&self) -> Result<Vec<DataEntity>, DataError> {
        let mut out = Vec::new();
        for alias in MANAGED_GRAPHS {
            let graph = crate::graphs::store_graph(alias);
            let quads = self.engine.store().quads(alias).await?;
            let mut by_subject: BTreeMap<String, Vec<Quad>> = BTreeMap::new();
            for quad in quads {
                by_subject
                    .entry(super::rdf_mapper::subject_str(&quad.subject))
                    .or_default()
                    .push(quad);
            }
            for (subject, quads) in by_subject {
                if let Some(entity) = entity_from_quads(&subject, &graph, &quads) {
                    out.push(entity);
                }
            }
        }
        out.sort_by(|a, b| a.iri.cmp(&b.iri));
        Ok(out)
    }

    async fn refresh_vocabulary(&self) -> Result<(), DataError> {
        let labels = vocabulary_labels(&self.engine).await?;
        if let Ok(mut vocabulary) = self.vocabulary.lock() {
            *vocabulary = labels;
        }
        Ok(())
    }

    fn document_for(&self, entity: &DataEntity) -> Result<mm_search::SearchDocument, DataError> {
        let namespace = self
            .vocabulary
            .lock()
            .ok()
            .map(|_| namespace_prefix_for(&entity.iri))
            .unwrap_or_else(|| "mmd".to_string());
        let vocabulary = self
            .vocabulary
            .lock()
            .map(|labels| labels.clone())
            .unwrap_or_default();
        Ok(search_document(entity, &vocabulary, &namespace))
    }

    async fn next_revision(&self) -> Result<u64, DataError> {
        let next = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        sqlx::query(
            "INSERT INTO data_meta (key, value) VALUES ('dataRevision', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(next.to_string())
        .execute(&self.pool)
        .await?;
        Ok(next)
    }

    async fn apply_entry(
        &self,
        search: &SearchIndex,
        entry: &OutboxEntry,
    ) -> Result<(), DataError> {
        if entry.is_delete() {
            search.delete(&entry.subject).await?;
            return Ok(());
        }
        let Some(json) = &entry.doc_json else {
            search.delete(&entry.subject).await?;
            return Ok(());
        };
        let document: mm_search::SearchDocument = serde_json::from_str(json)?;
        let current = self.entity_in_graph(&entry.subject, &entry.graph).await?;
        match current {
            None => {
                // The RDF write never landed: discard rather than resurrect.
                search.delete(&entry.subject).await?;
                Ok(())
            }
            Some(entity) if entity.status == EntityStatus::Deleted => {
                search.delete(&entity.iri).await?;
                Ok(())
            }
            Some(entity) if entity.revision > document.revision => {
                let fresh = self.document_for(&entity)?;
                search.upsert(&fresh).await?;
                Ok(())
            }
            Some(_) => {
                search.upsert(&document).await?;
                Ok(())
            }
        }
    }

    async fn entity_in_graph(
        &self,
        subject: &str,
        graph: &str,
    ) -> Result<Option<DataEntity>, DataError> {
        let quads = self
            .engine
            .store()
            .quads_for_subject(graph, subject)
            .await?;
        Ok(entity_from_quads(subject, graph, &quads))
    }

    async fn expand_predicate(&self, predicate: &str) -> Result<String, DataError> {
        let predicate = if predicate.starts_with("http") {
            predicate.to_string()
        } else {
            let Some((prefix, local)) = predicate.split_once(':') else {
                return Err(DataError::Invalid(format!(
                    "unknown predicate {predicate:?}"
                )));
            };
            let namespace = self.namespace_for_prefix(prefix).await?.ok_or_else(|| {
                DataError::Invalid(format!("unknown namespace prefix {prefix:?}"))
            })?;
            format!("{namespace}{local}")
        };
        if SYSTEM_PREDICATES.contains(&predicate.as_str()) {
            return Err(DataError::SchemaViolation(format!(
                "{predicate} is a system predicate"
            )));
        }
        Ok(predicate)
    }

    async fn normalized_properties(
        &self,
        patches: &[DataPatch],
    ) -> Result<BTreeMap<String, Vec<DataValue>>, DataError> {
        let mut properties = BTreeMap::new();
        for patch in patches {
            patch.validate()?;
            let predicate = self.expand_predicate(&patch.predicate).await?;
            for value in &patch.values {
                if let DataValue::Literal {
                    value, datatype, ..
                } = value
                {
                    validate_literal(datatype.as_deref(), value)?;
                }
            }
            properties.insert(predicate, patch.values.clone());
        }
        Ok(properties)
    }

    fn label_of_properties(properties: &BTreeMap<String, Vec<DataValue>>) -> Option<String> {
        for predicate in [
            RDFS_LABEL,
            "http://xmlns.com/foaf/0.1/name",
            SCHEMA_NAME,
            MM_NAME,
        ] {
            if let Some(values) = properties.get(predicate) {
                if let Some(DataValue::Literal { value, .. }) = values.first() {
                    return Some(value.clone());
                }
            }
        }
        None
    }

    async fn expand_class(&self, class: &str) -> Result<String, DataError> {
        let expanded = if class.starts_with("http") {
            class.to_string()
        } else {
            let Some((prefix, local)) = class.split_once(':') else {
                return Err(DataError::SchemaViolation(format!(
                    "unknown class {class:?}"
                )));
            };
            let namespace = self.namespace_for_prefix(prefix).await?.ok_or_else(|| {
                DataError::SchemaViolation(format!("unknown namespace prefix {prefix:?}"))
            })?;
            format!("{namespace}{local}")
        };
        if !self.namespace_known(&expanded).await? {
            return Err(DataError::SchemaViolation(format!(
                "unknown class {expanded:?}"
            )));
        }
        Ok(expanded)
    }

    async fn namespace_for_prefix(&self, prefix: &str) -> Result<Option<String>, DataError> {
        if let Some((_, namespace)) = BUILTINS.iter().find(|(name, _)| *name == prefix) {
            return Ok(Some((*namespace).to_string()));
        }
        Ok(self
            .catalog
            .list()
            .await?
            .into_iter()
            .find(|schema| schema.prefix == prefix)
            .map(|schema| schema.namespace))
    }

    async fn namespace_known(&self, iri: &str) -> Result<bool, DataError> {
        if known_class(iri) {
            return Ok(true);
        }
        Ok(self
            .catalog
            .list()
            .await?
            .into_iter()
            .any(|schema| schema.active && iri.starts_with(&schema.namespace)))
    }

    async fn mint_subject(&self, class: &str, id: Option<&str>) -> Result<String, DataError> {
        let kind = match local_name(class) {
            "User" | "Person" => Some("user"),
            "Project" => Some("project"),
            "Plan" | "PlanVersion" | "PlanStep" | "Thought" => Some("plan"),
            "ScheduledAction" | "CronJob" => Some("action"),
            "Session" | "SessionTrace" => Some("session"),
            "LogEntry" => Some("log"),
            "Schema" => Some("schema"),
            "Namespace" => Some("namespace"),
            _ => None,
        };
        match (kind, id) {
            (Some(kind), _) if MINTED_KINDS.contains(&kind) => self.namespaces.mint(kind, id).await,
            (None, Some(id)) if id.starts_with("http") => Ok(id.to_string()),
            (None, Some(id)) if id.starts_with("urn:metamind:") => {
                self.namespaces.normalize(id).await
            }
            (None, Some(id))
                if id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') =>
            {
                Ok(format!("{MMD_NS}{id}"))
            }
            (None, Some(id)) => Err(DataError::Invalid(format!(
                "id {id:?} must be alphanumeric, '-' or '_'"
            ))),
            (Some(kind), _) => Err(DataError::SchemaViolation(format!(
                "{kind} cannot be minted as an entity id"
            ))),
            (None, None) => Ok(format!("{MMD_NS}{}", new_id())),
        }
    }

    async fn existing_idempotent(&self, key: &str) -> Result<Option<String>, DataError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM data_meta WHERE key = ?1")
                .bind(format!("idempotency:{key}"))
                .fetch_optional(&self.pool)
                .await?;
        Ok(value)
    }

    async fn remember_idempotent(&self, key: &str, iri: &str) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO data_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
        )
        .bind(format!("idempotency:{key}"))
        .bind(iri)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn write_entity(
        &self,
        entity: &DataEntity,
        deletes: Vec<Quad>,
        idempotency_key: Option<&str>,
        op: &str,
        keep_quads: bool,
    ) -> Result<IndexHealth, DataError> {
        let document = self.document_for(entity)?;
        let doc_json = if op == "delete" {
            None
        } else {
            Some(serde_json::to_string(&document)?)
        };
        self.outbox
            .enqueue(
                entity.revision,
                op,
                &entity.iri,
                &entity.graph,
                doc_json.as_deref(),
                idempotency_key,
                now_ms(),
            )
            .await?;
        let inserts = if keep_quads {
            entity_quads(entity)
        } else {
            Vec::new()
        };
        self.engine
            .store()
            .handle()
            .apply_batch(mm_store_graph::GraphBatch { deletes, inserts })
            .await?;
        self.mutations.fetch_add(1, Ordering::SeqCst);
        if let Some(key) = idempotency_key {
            self.remember_idempotent(key, &entity.iri).await?;
        }
        let health = match &self.search {
            None => IndexHealth::Unavailable,
            Some(search) => match op {
                "delete" => match search.delete(&entity.iri).await {
                    Ok(()) => {
                        self.outbox.mark_done(entity.revision).await?;
                        IndexHealth::Healthy
                    }
                    Err(_) => IndexHealth::Pending,
                },
                _ => match search.upsert(&document).await {
                    Ok(()) => {
                        self.outbox.mark_done(entity.revision).await?;
                        IndexHealth::Healthy
                    }
                    Err(_) => IndexHealth::Pending,
                },
            },
        };
        Ok(health)
    }

    async fn active_of_class(&self, class: &str) -> Result<bool, DataError> {
        Ok(self.all_entities().await?.iter().any(|entity| {
            entity.status == EntityStatus::Active && entity.types.iter().any(|kind| kind == class)
        }))
    }

    async fn class_closure(&self, class: &str) -> Result<BTreeSet<String>, DataError> {
        const SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
        let triples = self.engine.triples("ontology").await?;
        let mut closure: BTreeSet<String> = BTreeSet::from([class.to_string()]);
        loop {
            let mut grew = false;
            for row in &triples {
                let predicate = row.get("p").and_then(crate::rdf::term_value);
                if predicate != Some(SUB_CLASS_OF) {
                    continue;
                }
                let Some(subject) = row.get("s").and_then(crate::rdf::term_value) else {
                    continue;
                };
                let Some(parent) = row.get("o").and_then(crate::rdf::term_value) else {
                    continue;
                };
                if closure.contains(parent) && closure.insert(subject.to_string()) {
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        Ok(closure)
    }

    async fn duplicate_of(
        &self,
        class: &str,
        graph: &str,
        label: &str,
    ) -> Result<Option<DataEntity>, DataError> {
        let quads = self.engine.store().quads(graph).await.unwrap_or_default();
        let mut by_subject: BTreeMap<String, Vec<Quad>> = BTreeMap::new();
        for quad in quads {
            by_subject
                .entry(super::rdf_mapper::subject_str(&quad.subject))
                .or_default()
                .push(quad);
        }
        for (subject, quads) in by_subject {
            let Some(entity) = entity_from_quads(&subject, graph, &quads) else {
                continue;
            };
            if entity.status == EntityStatus::Active
                && entity.types.iter().any(|kind| kind == class)
                && label_of(&entity) == label
            {
                return Ok(Some(entity));
            }
        }
        Ok(None)
    }
}

/// The result of comparing the RDF entity set with the lexical index.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexVerification {
    /// Live RDF entities expected in the index.
    pub entities: u64,
    /// Documents found in the index.
    pub documents: u64,
    /// RDF entities with no index document.
    pub missing: Vec<String>,
    /// Index documents whose RDF entity is gone or deleted.
    pub orphaned: Vec<String>,
}

impl IndexVerification {
    /// True when the two sides agree exactly.
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.orphaned.is_empty()
    }
}

/// The compact status projection used by later phases and the inspector.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DataStatus {
    /// Managed entities.
    pub entities: u64,
    /// Indexed documents.
    pub search_documents: u64,
    /// Pending outbox rows.
    pub index_pending: u64,
    /// Whether search may answer.
    pub index_ready: bool,
    /// Current global revision.
    pub data_revision: u64,
    /// Active schema prefixes.
    pub active_schemas: Vec<String>,
    /// Last mutation instant.
    pub last_mutation: Option<i64>,
}

#[async_trait::async_trait]
impl DataBroker for DataService {
    async fn create(&self, request: DataCreateRequest) -> Result<DataMutationResult, DataError> {
        if let Some(key) = request.idempotency_key.as_deref() {
            if let Some(iri) = self.existing_idempotent(key).await? {
                if let Some(entity) = self.find_entity(&iri).await? {
                    return Ok(DataMutationResult {
                        entity,
                        unchanged: true,
                        duplicate: false,
                        index: self.health().await,
                    });
                }
            }
        }
        let class = self.expand_class(&request.r#type).await?;
        let local = local_name(&class);
        if MANAGEMENT_CLASSES.contains(&local) {
            return Err(DataError::Invalid(format!(
                "{class} is written by its owning service, not data.create"
            )));
        }
        let properties = self.normalized_properties(&request.properties).await?;
        if matches!(local, "Person" | "User") && !properties.contains_key(FOAF_NAME) {
            return Err(DataError::SchemaViolation(format!(
                "{class} requires foaf:name"
            )));
        }
        if local == "Being" && self.active_of_class(&class).await? {
            return Err(DataError::SchemaViolation(
                "mm:Being is a singleton; the existing being must be updated instead".into(),
            ));
        }
        let label = Self::label_of_properties(&properties).unwrap_or_default();
        let graph = match request.graph.as_deref() {
            Some(alias) => route_graph(alias)?,
            None => crate::graphs::resolve_graph(graph_for_class(&class)),
        };
        // An explicit id is authoritative; the duplicate guard applies only
        // when the broker is free to mint one.
        if request.id.is_none() && !label.is_empty() {
            if let Some(existing) = self.duplicate_of(&class, &graph, &label).await? {
                return Ok(DataMutationResult {
                    entity: existing,
                    unchanged: true,
                    duplicate: true,
                    index: self.health().await,
                });
            }
        }
        let iri = self.mint_subject(&class, request.id.as_deref()).await?;
        if self.find_entity(&iri).await?.is_some() {
            return Err(DataError::Conflict(format!("entity {iri} already exists")));
        }
        let revision = self.next_revision().await?;
        let now = now_ms();
        let entity = DataEntity {
            iri,
            types: vec![class],
            label: (!label.is_empty()).then_some(label),
            properties,
            graph,
            revision,
            status: EntityStatus::Active,
            created_at: now,
            updated_at: now,
            recorded_by: "system".into(),
        };
        let index = self
            .write_entity(
                &entity,
                Vec::new(),
                request.idempotency_key.as_deref(),
                "upsert",
                true,
            )
            .await?;
        self.checkpoint_if_due().await?;
        Ok(DataMutationResult {
            entity,
            unchanged: false,
            duplicate: false,
            index,
        })
    }

    async fn update(&self, request: DataUpdateRequest) -> Result<DataMutationResult, DataError> {
        request.validate()?;
        let Some(mut entity) = self.find_entity(&request.subject).await? else {
            return Err(DataError::NotFound(request.subject.clone()));
        };
        if entity.status != EntityStatus::Active {
            return Err(DataError::Conflict(format!(
                "{} is {}",
                entity.iri,
                match entity.status {
                    EntityStatus::Deleted => "deleted",
                    EntityStatus::Superseded => "superseded",
                    EntityStatus::Active => "active",
                }
            )));
        }
        if let Some(kind) = entity
            .types
            .iter()
            .map(|class| local_name(class))
            .find(|local| MANAGEMENT_CLASSES.contains(local))
        {
            return Err(DataError::Invalid(format!(
                "{kind} is written by its owning service, not data.update"
            )));
        }
        if let Some(expected) = request.expected_revision {
            if expected != entity.revision {
                return Err(DataError::Conflict(format!(
                    "expected revision {expected}, found {}",
                    entity.revision
                )));
            }
        }
        let patches = self.normalized_properties(&request.properties).await?;
        let mut properties = match request.mode {
            DataUpdateMode::Merge => entity.properties.clone(),
            DataUpdateMode::Replace => BTreeMap::new(),
        };
        for (predicate, values) in patches {
            if values.is_empty() {
                properties.remove(&predicate);
            } else {
                properties.insert(predicate, values);
            }
        }
        if properties == entity.properties {
            return Ok(DataMutationResult {
                entity,
                unchanged: true,
                duplicate: false,
                index: self.health().await,
            });
        }
        entity.properties = properties;
        entity.label = Self::label_of_properties(&entity.properties);
        entity.revision = self.next_revision().await?;
        entity.updated_at = now_ms();
        let deletes = self
            .engine
            .store()
            .quads_for_subject(&entity.graph, &entity.iri)
            .await?;
        let index = self
            .write_entity(
                &entity,
                deletes,
                request.idempotency_key.as_deref(),
                "upsert",
                true,
            )
            .await?;
        self.checkpoint_if_due().await?;
        Ok(DataMutationResult {
            entity,
            unchanged: false,
            duplicate: false,
            index,
        })
    }

    async fn query(&self, query: DataQuery) -> Result<DataQueryResult, DataError> {
        query.validate()?;
        let class = match query.class.as_deref() {
            Some(class) if class.starts_with("http") => Some(class.to_string()),
            Some(class) => Some(self.expand_class(class).await?),
            None => None,
        };
        let graphs: Vec<String> = match query.graph.as_deref() {
            Some(graph) => vec![crate::graphs::resolve_graph(graph)],
            None => MANAGED_GRAPHS
                .iter()
                .map(|alias| crate::graphs::store_graph(alias))
                .collect(),
        };
        let namespace_iri = match query.namespace.as_deref() {
            Some(namespace) if namespace.starts_with("http") => Some(namespace.to_string()),
            Some(namespace) => Some(
                self.namespaces
                    .expand(&format!("{namespace}:"))
                    .await
                    .unwrap_or_else(|_| namespace.to_string()),
            ),
            None => None,
        };
        let class_closure = match class.as_deref() {
            Some(class) => Some(self.class_closure(class).await?),
            None => None,
        };
        let mut entities = Vec::new();
        for graph in graphs {
            let mut by_subject: BTreeMap<String, Vec<Quad>> = BTreeMap::new();
            for quad in self.engine.store().quads(&graph).await? {
                by_subject
                    .entry(super::rdf_mapper::subject_str(&quad.subject))
                    .or_default()
                    .push(quad);
            }
            for (subject, quads) in by_subject {
                let Some(entity) = entity_from_quads(&subject, &graph, &quads) else {
                    continue;
                };
                if !query.include_deleted && entity.status != EntityStatus::Active {
                    continue;
                }
                if let Some(closure) = &class_closure {
                    // The contract says a class filter includes subclasses, so
                    // the exact type is expanded through the ontology's
                    // `rdfs:subClassOf` closure.
                    if !entity.types.iter().any(|kind| closure.contains(kind)) {
                        continue;
                    }
                }
                if let Some(subject) = &query.subject {
                    let wanted = self.namespaces.normalize(subject).await?;
                    if entity.iri != wanted {
                        continue;
                    }
                }
                if let Some(namespace) = &namespace_iri {
                    if !entity.iri.starts_with(namespace) {
                        continue;
                    }
                }
                if !filters_match(&entity, &query.filters) {
                    continue;
                }
                entities.push(entity);
            }
        }
        entities.sort_by(|a, b| a.iri.cmp(&b.iri));
        let total = entities.len();
        let start = query.offset.min(total);
        let end = (start + query.limit).min(total);
        let page = entities[start..end].to_vec();
        Ok(DataQueryResult {
            entities: page,
            has_more: end < total,
            next_offset: (end < total).then_some(end),
        })
    }

    async fn delete(&self, request: DataDeleteRequest) -> Result<DataMutationResult, DataError> {
        let Some(mut entity) = self.find_entity(&request.subject).await? else {
            return Err(DataError::NotFound(request.subject.clone()));
        };
        if entity.status == EntityStatus::Deleted {
            // Deleting an already-deleted entity is idempotent.
            return Ok(DataMutationResult {
                entity,
                unchanged: true,
                duplicate: false,
                index: self.health().await,
            });
        }
        if entity.status != EntityStatus::Active {
            return Err(DataError::NotFound(request.subject.clone()));
        }
        if request.mode == DataDeleteMode::Hard {
            if let Some(protected) = entity
                .types
                .iter()
                .map(|class| local_name(class))
                .find(|local| PROTECTED_CLASSES.contains(local))
            {
                return Err(DataError::SchemaViolation(format!(
                    "{protected} may not be hard-deleted"
                )));
            }
        }
        if let Some(kind) = entity
            .types
            .iter()
            .map(|class| local_name(class))
            .find(|local| MANAGEMENT_CLASSES.contains(local))
        {
            return Err(DataError::Invalid(format!(
                "{kind} is deleted by its owning service, not data.delete"
            )));
        }
        if let Some(expected) = request.expected_revision {
            if expected != entity.revision {
                return Err(DataError::Conflict(format!(
                    "expected revision {expected}, found {}",
                    entity.revision
                )));
            }
        }
        let deletes = self
            .engine
            .store()
            .quads_for_subject(&entity.graph, &entity.iri)
            .await?;
        let index = match request.mode {
            DataDeleteMode::Soft => {
                entity.status = EntityStatus::Deleted;
                entity.revision = self.next_revision().await?;
                entity.updated_at = now_ms();
                self.write_entity(&entity, deletes, None, "delete", true)
                    .await?
            }
            DataDeleteMode::Hard => {
                if !Self::hard_delete_allowed(request.authority.as_deref()) {
                    return Err(DataError::Invalid("hard delete requires authority".into()));
                }
                entity.revision = self.next_revision().await?;
                self.write_entity(&entity, deletes, None, "delete", false)
                    .await?
            }
        };
        self.checkpoint_if_due().await?;
        Ok(DataMutationResult {
            entity,
            unchanged: false,
            duplicate: false,
            index,
        })
    }

    async fn search(&self, request: DataSearchRequest) -> Result<SearchResult, DataError> {
        request.validate()?;
        let Some(search) = &self.search else {
            return Err(DataError::SearchUnavailable("the index is not open".into()));
        };
        if !self.index_ready() {
            return Err(DataError::SearchUnavailable(
                "the index is still rebuilding".into(),
            ));
        }
        let class = match request.class.as_deref() {
            Some(class) => Some(self.expand_class(class).await?),
            None => None,
        };
        let query = SearchQuery {
            text: request.query.clone(),
            class,
            graph: request.graph.as_deref().map(crate::graphs::resolve_graph),
            namespace: request.namespace.clone(),
            limit: request.limit,
            snippet: request.snippet,
            include_deleted: request.include_deleted,
        };
        let page = search.search(&query).await?;
        let mut hits = Vec::new();
        for hit in page.hits {
            match self.find_entity(&hit.subject).await? {
                Some(entity)
                    if entity.status == EntityStatus::Active || request.include_deleted =>
                {
                    if entity.revision == hit.revision {
                        hits.push(SearchHit {
                            iri: entity.iri,
                            r#type: entity
                                .types
                                .first()
                                .cloned()
                                .unwrap_or_else(|| "unknown".into()),
                            label: hit.label,
                            score_micros: hit.score_micros,
                            snippet: hit.snippet,
                            graph: entity.graph,
                            revision: entity.revision,
                            updated_at: entity.updated_at,
                        });
                    } else {
                        // Stale hit: repair it instead of returning the old row.
                        let document = self.document_for(&entity)?;
                        let _ = search.upsert(&document).await;
                    }
                }
                _ => {
                    let _ = search.delete(&hit.subject).await;
                }
            }
        }
        Ok(SearchResult {
            hits,
            index: self.health().await,
        })
    }

    async fn list_schemas(&self) -> Result<Vec<SchemaSummary>, DataError> {
        self.catalog.list().await
    }

    async fn get_schema(
        &self,
        id: &str,
        include: SchemaInclude,
    ) -> Result<SchemaDetail, DataError> {
        self.catalog.get(id, include).await
    }

    async fn validate(&self, target: ValidationTarget) -> Result<Vec<ShaclViolation>, DataError> {
        self.catalog.validate(target).await
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceBinding>, DataError> {
        self.namespaces.list().await
    }

    async fn expand(&self, compact: &str) -> Result<String, DataError> {
        self.namespaces.expand(compact).await
    }

    async fn resolve(&self, iri: &str) -> Result<String, DataError> {
        self.namespaces.resolve(iri).await
    }
}

impl DataService {
    async fn health(&self) -> IndexHealth {
        if self.search.is_none() {
            return IndexHealth::Unavailable;
        }
        match self.outbox.pending_count().await {
            Ok(0) => IndexHealth::Healthy,
            Ok(_) => IndexHealth::Pending,
            Err(_) => IndexHealth::Unavailable,
        }
    }
}

fn filters_match(entity: &DataEntity, filters: &[DataFilter]) -> bool {
    filters.iter().all(|filter| {
        let predicate = match filter {
            DataFilter::Equals { predicate, .. }
            | DataFilter::Iri { predicate, .. }
            | DataFilter::Contains { predicate, .. } => predicate,
        };
        let Some(values) = entity.properties.get(predicate) else {
            return false;
        };
        values.iter().any(|candidate| match filter {
            DataFilter::Equals { value, .. } => {
                matches!(candidate, DataValue::Literal { value: actual, .. } if actual == value)
            }
            DataFilter::Iri { value, .. } => {
                matches!(candidate, DataValue::Iri { iri } if iri == value)
            }
            DataFilter::Contains { value, .. } => {
                matches!(candidate, DataValue::Literal { value: actual, .. } if actual.contains(value))
            }
        })
    })
}

/// A caller-supplied graph must be an installed data route. Management graphs
/// (`schema`) and kernel graphs (`ontology`, `agentstream`) stay off the
/// general write path.
fn route_graph(alias: &str) -> Result<String, DataError> {
    let requested = crate::graphs::resolve_graph(alias);
    for managed in MANAGED_GRAPHS.iter().filter(|name| **name != "schema") {
        if requested == crate::graphs::store_graph(managed) {
            return Ok(requested);
        }
    }
    Err(DataError::Invalid(format!(
        "{alias} is not an installed data graph"
    )))
}

fn named_graph(quad: &Quad) -> Option<&str> {
    match &quad.graph_name {
        oxrdf::GraphName::NamedNode(node) => Some(node.as_str()),
        _ => None,
    }
}

fn namespace_prefix_for(iri: &str) -> String {
    if iri.starts_with(&format!("{MMD_NS}user/")) {
        return "user".to_string();
    }
    if iri.starts_with(MMD_NS) {
        return "mmd".to_string();
    }
    if iri.starts_with(MM_NS) {
        return "mm".to_string();
    }
    "mmd".to_string()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), DataError> {
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, bytes).map_err(|error| DataError::Unavailable(error.to_string()))?;
    std::fs::rename(&temp, path).map_err(|error| DataError::Unavailable(error.to_string()))?;
    Ok(())
}
