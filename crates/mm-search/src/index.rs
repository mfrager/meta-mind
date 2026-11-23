//! The Tantivy index: writer, reader, search, and reindex.

use std::path::{Path, PathBuf};
use std::time::Instant;

use tantivy::collector::TopDocs;
use tantivy::query::Query;
use tantivy::schema::Value;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};
use tokio::sync::Mutex;

use crate::schema::{build_schema, SearchFields};
use crate::{IndexReport, SearchDocument, SearchError, SearchPage, SearchQuery};

/// Commit every this many documents during a reindex.
pub const REINDEX_BATCH: usize = 500;
/// Memory budget for the single writer.
const WRITER_BUDGET: usize = 50_000_000;

/// The derived full-text index.
pub struct SearchIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,
    fields: SearchFields,
    dir: PathBuf,
}

impl std::fmt::Debug for SearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchIndex")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl SearchIndex {
    /// Open an index, creating or rebuilding the directory when its schema does
    /// not match.
    pub fn open(dir: &Path) -> Result<Self, SearchError> {
        std::fs::create_dir_all(dir).map_err(|e| {
            SearchError::Unavailable(format!("cannot create {}: {e}", dir.display()))
        })?;
        let (schema, fields) = build_schema();
        let index = match Index::open_in_dir(dir) {
            Ok(index) if index.schema() == schema => index,
            Ok(_) => {
                // A different schema is a rebuildable read model, not authority.
                clear_dir(dir)?;
                Index::create_in_dir(dir, schema)
                    .map_err(|e| SearchError::Unavailable(format!("cannot create index: {e}")))?
            }
            Err(_) => Index::create_in_dir(dir, schema)
                .map_err(|e| SearchError::Unavailable(format!("cannot open index: {e}")))?,
        };
        let writer = index
            .writer_with_num_threads(1, WRITER_BUDGET)
            .map_err(|e| SearchError::Unavailable(format!("cannot open index writer: {e}")))?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| SearchError::Unavailable(format!("cannot open index reader: {e}")))?;
        Ok(Self {
            index,
            writer: Mutex::new(writer),
            reader,
            fields,
            dir: dir.to_path_buf(),
        })
    }

    /// The index directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The field handles.
    pub fn fields(&self) -> &SearchFields {
        &self.fields
    }

    /// Live documents, committed or not-yet-visible excluded.
    pub async fn num_docs(&self) -> Result<u64, SearchError> {
        let searcher = self.reader.searcher();
        Ok(searcher.num_docs())
    }

    /// Insert or replace one document, committing immediately.
    ///
    /// A replace is a delete of the subject's term followed by the new
    /// document in the same commit, so a search never sees two versions.
    pub async fn upsert(&self, doc: &SearchDocument) -> Result<(), SearchError> {
        let term = Term::from_field_text(self.fields.subject, &doc.subject);
        let document = self.to_document(doc);
        let mut writer = self.writer.lock().await;
        writer.delete_term(term);
        writer
            .add_document(document)
            .map_err(|e| SearchError::Index(format!("add document: {e}")))?;
        writer
            .commit()
            .map_err(|e| SearchError::Index(format!("commit: {e}")))?;
        drop(writer);
        self.reader
            .reload()
            .map_err(|e| SearchError::Index(format!("reader reload: {e}")))?;
        Ok(())
    }

    /// Delete one subject's document.
    pub async fn delete(&self, subject: &str) -> Result<(), SearchError> {
        let term = Term::from_field_text(self.fields.subject, subject);
        let mut writer = self.writer.lock().await;
        writer.delete_term(term);
        writer
            .commit()
            .map_err(|e| SearchError::Index(format!("commit: {e}")))?;
        drop(writer);
        self.reader
            .reload()
            .map_err(|e| SearchError::Index(format!("reader reload: {e}")))?;
        Ok(())
    }

    /// Run a bounded query.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchPage, SearchError> {
        if query.limit == 0 || query.limit > crate::MAX_SEARCH_LIMIT {
            return Err(SearchError::Invalid(format!(
                "search limit must be 1–{}",
                crate::MAX_SEARCH_LIMIT
            )));
        }
        let built: Box<dyn Query> = crate::query::build_query(&self.index, &self.fields, query);
        let searcher = self.reader.searcher();
        let top = searcher
            .search(&*built, &TopDocs::with_limit(query.limit).order_by_score())
            .map_err(|e| SearchError::Index(format!("search: {e}")))?;
        let mut hits = Vec::with_capacity(top.len());
        for (score, address) in top {
            let document: TantivyDocument = searcher
                .doc(address)
                .map_err(|e| SearchError::Index(format!("read document: {e}")))?;
            let subject = text(&document, self.fields.subject);
            if subject.is_empty() {
                continue;
            }
            let graph = text(&document, self.fields.graph);
            let label = text(&document, self.fields.label);
            let body = text(&document, self.fields.body);
            let types = document
                .get_all(self.fields.types)
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect();
            let hit = crate::SearchHit {
                subject,
                graph,
                namespace: text(&document, self.fields.namespace),
                label: if label.is_empty() { None } else { Some(label) },
                types,
                revision: first_u64(&document, self.fields.revision),
                updated_at: first_i64(&document, self.fields.updated_at),
                status: text(&document, self.fields.status),
                score_micros: (f64::from(score) * 1_000_000.0).round() as i64,
                snippet: if query.snippet {
                    crate::query::snippet(&body, &query.text)
                } else {
                    None
                },
            };
            hits.push(hit);
        }
        hits.sort_by(|a, b| {
            b.score_micros
                .cmp(&a.score_micros)
                .then_with(|| a.subject.cmp(&b.subject))
        });
        let has_more = hits.len() == query.limit;
        Ok(SearchPage { hits, has_more })
    }

    /// Rebuild the whole index from a set of documents.
    ///
    /// The index is cleared first: `reindex` means "the RDF state is the truth
    /// now", so a document whose entity disappeared must not survive.
    pub async fn reindex(
        &self,
        documents: Vec<SearchDocument>,
    ) -> Result<IndexReport, SearchError> {
        let started = Instant::now();
        let mut writer = self.writer.lock().await;
        writer
            .delete_all_documents()
            .map_err(|e| SearchError::Index(format!("clear index: {e}")))?;
        for chunk in documents.chunks(REINDEX_BATCH) {
            for doc in chunk {
                let document = self.to_document(doc);
                writer
                    .add_document(document)
                    .map_err(|e| SearchError::Index(format!("add document: {e}")))?;
            }
            writer
                .commit()
                .map_err(|e| SearchError::Index(format!("commit: {e}")))?;
        }
        writer
            .commit()
            .map_err(|e| SearchError::Index(format!("commit: {e}")))?;
        drop(writer);
        self.reader
            .reload()
            .map_err(|e| SearchError::Index(format!("reader reload: {e}")))?;
        Ok(IndexReport {
            documents: documents.len(),
            took_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// The underlying index, for diagnostics.
    pub fn index(&self) -> &Index {
        &self.index
    }

    fn to_document(&self, doc: &SearchDocument) -> TantivyDocument {
        let mut document = TantivyDocument::new();
        document.add_text(self.fields.subject, &doc.subject);
        document.add_text(self.fields.graph, &doc.graph);
        for class in &doc.types {
            document.add_text(self.fields.types, class);
        }
        document.add_text(self.fields.label, &doc.label);
        document.add_text(self.fields.body, &doc.body);
        document.add_text(self.fields.namespace, &doc.namespace);
        document.add_u64(self.fields.revision, doc.revision);
        document.add_i64(self.fields.updated_at, doc.updated_at);
        document.add_text(self.fields.status, &doc.status);
        document
    }
}

fn text(document: &TantivyDocument, field: tantivy::schema::Field) -> String {
    document
        .get_first(field)
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_default()
}

fn first_u64(document: &TantivyDocument, field: tantivy::schema::Field) -> u64 {
    document
        .get_first(field)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn first_i64(document: &TantivyDocument, field: tantivy::schema::Field) -> i64 {
    document
        .get_first(field)
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
}

fn clear_dir(dir: &Path) -> Result<(), SearchError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| SearchError::Unavailable(format!("cannot read {}: {e}", dir.display())))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| SearchError::Unavailable(format!("cannot remove {path:?}: {e}")))?;
        } else {
            std::fs::remove_file(&path)
                .map_err(|e| SearchError::Unavailable(format!("cannot remove {path:?}: {e}")))?;
        }
    }
    Ok(())
}
