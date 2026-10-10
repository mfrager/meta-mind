//! The design writer: a design document changes only through a promoted change set.
//!
//! Design §67–69 asks the being to continue its own *design*. The dangerous version of
//! that is a writer that edits documents directly: an unreviewed design change is an
//! architectural change nobody proposed, and the promotion gate is the only place a
//! proposal is judged. So this module is deliberately narrow:
//!
//! * **The content is the change set's, not the caller's.** [`ChangeSetDesignWriter::revise`]
//!   takes a change set and a document reference and looks the content up in the change
//!   set's own patches. There is no `write(text)` entry point, because an entry point that
//!   took text would be the direct edit this module exists to prevent.
//! * **A rejection is byte-identical, and that is a property of the code, not a
//!   convention.** The status is read *before* anything is written, and a change set that
//!   is not `promoted` returns an error with the document untouched. The test for it writes
//!   a document, then attempts a revision from a rejected change set, then re-reads the
//!   bytes.
//! * **The document is written under the artifact root, not into the source tree.** A
//!   run must not be able to mutate the repository it is running in — that is what Phase
//!   11's `audit production-tree` asserts — so a promoted revision lands in
//!   `<data>/artifacts/design/…` and the *row* plus the `/self` triples are what make it
//!   visible. [`DESIGN_ARTIFACT_SUBDIR`] is the one place the subdirectory is spelled.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_selfeng::changeset::{ChangeSet, ChangeSetStatus, ChangeSetStore};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{LoopError, Result};
use crate::rdf;

/// The subdirectory of the artifact root design documents are written under.
pub const DESIGN_ARTIFACT_SUBDIR: &str = "design";

/// Which document a revision is of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignDocRef {
    /// Its stable IRI, `https://metamind.dev/design/{path}`.
    pub uri: String,
    /// Its repository-relative path, which is also the change set patch's path.
    pub path: PathBuf,
}

impl DesignDocRef {
    /// A reference to a document, with its IRI derived from the path.
    ///
    /// The `design/` directory is the `DESIGN` namespace's own, so it is stripped from
    /// the path before the IRI is built; without that, `design/x.md` would become
    /// `https://metamind.dev/design/design/x.md`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let relative = path.strip_prefix("design").unwrap_or(&path);
        DesignDocRef {
            uri: mm_core::iri::design(&relative.to_string_lossy(), None).into_string(),
            path,
        }
    }
}

/// One registered revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// Its ULID.
    #[serde(with = "mm_core::serde_ulid")]
    pub id: Ulid,
    /// The document's IRI.
    pub doc_uri: String,
    /// The document's path.
    pub path: PathBuf,
    /// The revision number, from 1.
    pub revision: i64,
    /// The change set that produced it.
    #[serde(with = "mm_core::serde_ulid")]
    pub change_set: Ulid,
    /// Whether the bytes were written.
    pub applied: bool,
}

/// What a design writer does.
#[async_trait]
pub trait DesignWriter {
    /// Revise a document to the content a change set carries for it.
    async fn revise(&self, doc: &DesignDocRef, cs: &ChangeSet) -> Result<Revision>;
}

/// The writer over the change-set pipeline.
pub struct ChangeSetDesignWriter {
    store: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
    change_sets: ChangeSetStore,
    artifacts_dir: PathBuf,
    graph: Option<Arc<GraphStore>>,
}

impl ChangeSetDesignWriter {
    /// A writer that materialises a document under `artifacts_dir`.
    pub fn new(
        store: SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
        artifacts_dir: PathBuf,
    ) -> Self {
        let change_sets = ChangeSetStore::new(store.clone(), logger.clone(), ids.clone());
        ChangeSetDesignWriter {
            store,
            logger,
            ids,
            change_sets,
            artifacts_dir,
            graph: None,
        }
    }

    /// Attach the graph store, so a revision reaches `/self`.
    pub fn with_graph(mut self, graph: Arc<GraphStore>) -> Self {
        self.graph = Some(graph);
        self
    }

    /// Where a document is materialised.
    ///
    /// The `design/` directory is the `DESIGN` namespace's own, which
    /// [`DesignDocRef::new`] already accounts for when it builds the IRI;
    /// [`DESIGN_ARTIFACT_SUBDIR`] now names the *kind*, so joining a path that still
    /// carries its own `design/` would write `artifacts/design/design/…` — a second
    /// `design` that is an artefact of the layout rather than a directory that means
    /// anything.
    pub fn target_path(&self, doc: &DesignDocRef) -> PathBuf {
        let relative = doc.path.strip_prefix("design").unwrap_or(&doc.path);
        self.artifacts_dir
            .join(DESIGN_ARTIFACT_SUBDIR)
            .join(relative)
    }

    /// The next revision number for a document, which is one past the highest recorded.
    async fn next_revision(&self, doc: &DesignDocRef) -> Result<i64> {
        let rows = self
            .store
            .query_json(
                "SELECT COALESCE(MAX(revision), 0) AS n FROM design_revisions WHERE doc_uri = ?",
                vec![Param::Text(doc.uri.clone())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read design_revisions: {e}")))?;
        Ok(rows[0]["n"].as_i64().unwrap_or(0) + 1)
    }

    /// Revise a document, refusing anything but a promoted change set.
    pub async fn revise_promoted(&self, doc: &DesignDocRef, cs: &ChangeSet) -> Result<Revision> {
        // The status is read first, and nothing is written before it is known to be
        // `promoted`. This ordering is the whole guarantee: a rejection leaves the
        // document byte-identical because no byte was ever opened for writing.
        let status = self
            .change_sets
            .status(cs.id)
            .await
            .map_err(LoopError::from)?;
        let status = status.unwrap_or(ChangeSetStatus::Draft);
        if status != ChangeSetStatus::Promoted {
            return Err(LoopError::Design(format!(
                "change set {} is {}, and only a promoted change set may revise a \
                 document; it is byte-identical",
                mm_core::ulid_string(&cs.id),
                status.as_str()
            )));
        }
        let patch = cs
            .code
            .iter()
            .find(|patch| patch.path == doc.path)
            .ok_or_else(|| {
                LoopError::Design(format!(
                    "change set {} carries no patch for {}",
                    mm_core::ulid_string(&cs.id),
                    doc.path.display()
                ))
            })?;
        if patch.operation.as_str() == "delete" {
            return Err(LoopError::Design(format!(
                "change set {} deletes {}, which is not a revision",
                mm_core::ulid_string(&cs.id),
                doc.path.display()
            )));
        }

        let revision = self.next_revision(doc).await?;
        let target = self.target_path(doc);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                LoopError::Design(format!("cannot create {}: {e}", parent.display()))
            })?;
        }
        std::fs::write(&target, &patch.content)
            .map_err(|e| LoopError::Design(format!("cannot write {}: {e}", target.display())))?;

        let id = self.ids.next();
        self.store
            .execute(
                "INSERT INTO design_revisions \
                 (id, doc_uri, doc_path, revision, change_set_id, promoted, created_at) \
                 VALUES (?, ?, ?, ?, ?, 1, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&id)),
                    Param::Text(doc.uri.clone()),
                    Param::Text(doc.path.to_string_lossy().to_string()),
                    Param::Int(revision),
                    Param::Text(mm_core::ulid_string(&cs.id)),
                    Param::Text(Timestamp::now().to_rfc3339()),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the revision: {e}")))?;
        self.logger
            .audit(
                Level::Info,
                codes::DESIGN_REVISION,
                crate::TARGET,
                Some(id),
                json!({
                    "doc_uri": doc.uri,
                    "doc_path": doc.path.to_string_lossy(),
                    "revision": revision,
                    "change_set_id": mm_core::ulid_string(&cs.id),
                    "target": target.display().to_string(),
                }),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the revision: {e}")))?;
        self.mirror(&id, doc, revision, &cs.id).await?;
        Ok(Revision {
            id,
            doc_uri: doc.uri.clone(),
            path: doc.path.clone(),
            revision,
            change_set: cs.id,
            applied: true,
        })
    }

    /// Mirror a revision into `/self`.
    pub async fn mirror(
        &self,
        id: &Ulid,
        doc: &DesignDocRef,
        revision: i64,
        change_set: &Ulid,
    ) -> Result<usize> {
        let Some(graph) = &self.graph else {
            return Ok(0);
        };
        let quads = rdf::design_revision_quads(id, &doc.uri, revision, change_set, true);
        graph
            .handle()
            .insert_turtle(rdf::SELF_GRAPH, &rdf::turtle(&quads))
            .await
            .map_err(LoopError::from)
    }

    /// Read the change set and revise the document it carries.
    ///
    /// The CLI's entry point: `design revise --doc PATH --change-set ULID` names a path and
    /// a change set, and this finds the patch. A caller cannot pass content, which is the
    /// point.
    pub async fn revise_from_change_set(
        &self,
        doc: &DesignDocRef,
        change_set: Ulid,
    ) -> Result<Revision> {
        let cs = self
            .change_sets
            .get(change_set)
            .await
            .map_err(LoopError::from)?
            .ok_or_else(|| {
                LoopError::Design(format!(
                    "no change set {}",
                    mm_core::ulid_string(&change_set)
                ))
            })?;
        self.revise_promoted(doc, &cs).await
    }

    /// Every revision of a document, oldest first.
    pub async fn history(&self, doc_uri: &str) -> Result<Vec<Revision>> {
        let rows = self
            .store
            .query_json(
                "SELECT * FROM design_revisions WHERE doc_uri = ? ORDER BY revision",
                vec![Param::Text(doc_uri.to_string())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read design_revisions: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(Revision {
                id: mm_core::id::parse_ulid(row["id"].as_str().unwrap_or_default())
                    .map_err(LoopError::from)?,
                doc_uri: row["doc_uri"].as_str().unwrap_or_default().to_string(),
                path: PathBuf::from(row["doc_path"].as_str().unwrap_or_default()),
                revision: row["revision"].as_i64().unwrap_or(0),
                change_set: mm_core::id::parse_ulid(
                    row["change_set_id"].as_str().unwrap_or_default(),
                )
                .map_err(LoopError::from)?,
                applied: row["promoted"].as_i64().unwrap_or(0) == 1,
            });
        }
        Ok(out)
    }
}

#[async_trait]
impl DesignWriter for ChangeSetDesignWriter {
    async fn revise(&self, doc: &DesignDocRef, cs: &ChangeSet) -> Result<Revision> {
        self.revise_promoted(doc, cs).await
    }
}

/// A path's document reference, so a caller that has a path does not build the IRI by hand.
pub fn doc_ref(path: impl AsRef<Path>) -> DesignDocRef {
    DesignDocRef::new(path.as_ref().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;
    use mm_selfeng::changeset::{from_gap, BenchmarkId, Gap, Patch, TestId};

    fn gap() -> Gap {
        Gap {
            gap_id: Ulid::from_parts(1_700_000_000_000, 1),
            kind: "capability".to_string(),
            statement: "no module reports goal attainment".to_string(),
            evidence: vec![Ulid::from_parts(1_700_000_000_000, 2)],
            target_uri: "https://metamind.dev/code/module/cognition/goal-attainment".to_string(),
            target_path: PathBuf::from("modules/cognition/goal-attainment"),
        }
    }

    fn change_set(ids: &UlidFactory, content: &str) -> ChangeSet {
        from_gap(
            &gap(),
            vec![
                Patch::create(
                    "modules/cognition/goal-attainment/src/lib.rs",
                    "// scaffold\n",
                ),
                Patch::create("design/qualification/goal_attainment.md", content),
            ],
            vec![TestId::new("bench/regression/seeded_bug_01")],
            vec![BenchmarkId::new("qualification_bench")],
            ids,
        )
        .expect("a change set")
    }

    struct Fixture {
        writer: ChangeSetDesignWriter,
        store: SqliteStore,
        logger: Arc<Logger>,
    }

    async fn fixture() -> (Fixture, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path().join("data"));
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        let artifacts = dir.path().join("data").join("artifacts");
        (
            Fixture {
                writer: ChangeSetDesignWriter::new(store.clone(), logger.clone(), ids, artifacts),
                store,
                logger,
            },
            dir,
        )
    }

    async fn record(fixture: &Fixture, cs: &ChangeSet, status: ChangeSetStatus) {
        let change_sets = ChangeSetStore::new(
            fixture.store.clone(),
            fixture.logger.clone(),
            Arc::new(UlidFactory::new()),
        );
        change_sets.create(cs).await.unwrap();
        change_sets.set_status(cs.id, status).await.unwrap();
    }

    #[tokio::test]
    async fn a_promoted_change_set_revises_the_document_through_the_pipeline() {
        let (fixture, _dir) = fixture().await;
        let ids = UlidFactory::new();
        let cs = change_set(&ids, "# design v1\n");
        record(&fixture, &cs, ChangeSetStatus::Promoted).await;
        let doc = doc_ref("design/qualification/goal_attainment.md");
        let revision = fixture.writer.revise(&doc, &cs).await.expect("a revision");
        assert_eq!(revision.revision, 1);
        assert!(revision.applied);
        assert_eq!(revision.change_set, cs.id);
        let written = std::fs::read_to_string(fixture.writer.target_path(&doc)).unwrap();
        assert_eq!(written, "# design v1\n");
        assert_eq!(fixture.writer.history(&doc.uri).await.unwrap().len(), 1);
        // A second promoted change set for the same document numbers itself 2.
        let second = change_set(&ids, "# design v2\n");
        record(&fixture, &second, ChangeSetStatus::Promoted).await;
        let again = fixture
            .writer
            .revise(&doc, &second)
            .await
            .expect("a revision");
        assert_eq!(
            again.revision, 2,
            "the second revision of one document is 2"
        );
        assert_eq!(fixture.writer.history(&doc.uri).await.unwrap().len(), 2);
        let target = fixture.writer.target_path(&doc);
        assert!(
            target.starts_with(_dir.path().join("data").join("artifacts")),
            "a revision lands under the artifact root: {}",
            target.display()
        );
    }

    #[tokio::test]
    async fn a_rejected_change_set_leaves_the_document_byte_identical() {
        let (fixture, _dir) = fixture().await;
        let ids = UlidFactory::new();
        let doc = doc_ref("design/qualification/goal_attainment.md");

        let promoted = change_set(&ids, "# original\n");
        record(&fixture, &promoted, ChangeSetStatus::Promoted).await;
        fixture
            .writer
            .revise(&doc, &promoted)
            .await
            .expect("the first revision");
        let before = std::fs::read(fixture.writer.target_path(&doc)).unwrap();

        let rejected = change_set(&ids, "# rejected rewrite\n");
        record(&fixture, &rejected, ChangeSetStatus::Rejected).await;
        let error = fixture
            .writer
            .revise(&doc, &rejected)
            .await
            .expect_err("a rejected change set may not revise");
        assert!(matches!(error, LoopError::Design(_)), "{error}");
        let after = std::fs::read(fixture.writer.target_path(&doc)).unwrap();
        assert_eq!(before, after, "the document is byte-identical");
        assert_eq!(fixture.writer.history(&doc.uri).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_change_set_with_no_patch_for_the_document_is_refused() {
        let (fixture, _dir) = fixture().await;
        let ids = UlidFactory::new();
        let cs = change_set(&ids, "# whatever\n");
        record(&fixture, &cs, ChangeSetStatus::Promoted).await;
        let other = doc_ref("design/other.md");
        assert!(fixture.writer.revise(&other, &cs).await.is_err());
        assert!(!fixture.writer.target_path(&other).exists());
    }

    #[test]
    fn a_document_reference_derives_its_iri_from_the_path() {
        let doc = doc_ref("design/qualification/goal_attainment.md");
        assert_eq!(
            doc.uri,
            "https://metamind.dev/design/qualification/goal_attainment.md"
        );
        assert_eq!(
            doc.path,
            PathBuf::from("design/qualification/goal_attainment.md")
        );
    }
}
