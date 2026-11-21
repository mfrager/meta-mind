//! Dataset catalog: DCAT metadata, versions and provenance.
//!
//! Datasets are separate from conversation memory. They are versioned data
//! artifacts with checksums; projects may reference them but do not own them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::RdfEngine;
use crate::error::{DataError, Result};
use crate::rdf::{self, dataset_graph, dataset_iri, now_ms, turtle_escape, DATASET_CATALOG_GRAPH};

/// What to create.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatasetSpec {
    /// Title.
    pub title: String,
    /// Description.
    pub description: Option<String>,
    /// Default distribution format (`text/turtle`).
    pub format: Option<String>,
}

/// One version of a dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetVersion {
    /// Version label.
    pub version: String,
    /// Files, relative to the version directory.
    pub files: Vec<DatasetFile>,
    /// When it was created.
    pub created_at: i64,
}

/// A file in a dataset version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetFile {
    /// File name.
    pub name: String,
    /// Byte size.
    pub bytes: u64,
    /// `sha256:<hex>`.
    pub checksum: String,
}

/// A stored dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetRecord {
    /// Identifier.
    pub dataset_id: String,
    /// Title.
    pub title: String,
    /// Description.
    pub description: Option<String>,
    /// Format.
    pub format: String,
    /// Directory relative to the data root.
    pub data_dir: String,
    /// Versions, in insertion order.
    #[serde(default)]
    pub versions: Vec<DatasetVersion>,
    /// Created, Unix milliseconds.
    pub created_at: i64,
}

/// Dataset class.
pub const DATASET_CLASS: &str = "https://metamind.dev/ontology#Dataset";

/// The dataset catalog.
#[derive(Debug, Clone)]
pub struct DatasetCatalog {
    engine: Arc<RdfEngine>,
    base_dir: PathBuf,
}

impl DatasetCatalog {
    /// Build the catalog.
    pub fn new(engine: Arc<RdfEngine>, base_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(base_dir.join("datasets"))?;
        Ok(Self {
            engine,
            base_dir: base_dir.to_path_buf(),
        })
    }

    /// Create a dataset with the standard directory layout.
    pub async fn create_dataset(&self, spec: DatasetSpec) -> Result<DatasetRecord> {
        if spec.title.trim().is_empty() {
            return Err(DataError::Invalid {
                kind: "dataset",
                detail: "title must not be empty".to_owned(),
            });
        }
        let id = rdf::new_id();
        let dir = self.base_dir.join("datasets").join(&id);
        for sub in [
            "data/raw",
            "data/processed",
            "data/exports",
            "versions",
            "provenance",
            "catalog",
            "metadata",
        ] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        let record = DatasetRecord {
            dataset_id: id.clone(),
            title: spec.title.clone(),
            description: spec.description.clone(),
            format: spec
                .format
                .clone()
                .unwrap_or_else(|| "text/turtle".to_owned()),
            data_dir: format!("datasets/{id}/data"),
            versions: Vec::new(),
            created_at: now_ms(),
        };
        std::fs::write(dir.join("manifest.ttl"), build_manifest_ttl(&record))?;
        self.save(&record).await?;
        Ok(record)
    }

    /// Add a version by copying files into `versions/<version>/`.
    pub async fn add_version(
        &self,
        dataset_id: &str,
        version: &str,
        files: &[PathBuf],
    ) -> Result<DatasetRecord> {
        let mut record = self.require_dataset(dataset_id).await?;
        if record
            .versions
            .iter()
            .any(|existing| existing.version == version)
        {
            return Err(DataError::Invalid {
                kind: "dataset version",
                detail: format!("version {version} already exists"),
            });
        }
        let version_dir = self
            .base_dir
            .join("datasets")
            .join(dataset_id)
            .join("versions")
            .join(version);
        std::fs::create_dir_all(&version_dir)?;
        let mut recorded = Vec::with_capacity(files.len());
        for file in files {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| DataError::Invalid {
                    kind: "dataset file",
                    detail: format!("{} has no file name", file.display()),
                })?
                .to_owned();
            let destination = version_dir.join(&name);
            std::fs::copy(file, &destination)?;
            let bytes = std::fs::metadata(&destination)?.len();
            let checksum = sha256_file(&destination)?;
            recorded.push(DatasetFile {
                name,
                bytes,
                checksum,
            });
        }
        record.versions.push(DatasetVersion {
            version: version.to_owned(),
            files: recorded,
            created_at: now_ms(),
        });
        self.save(&record).await?;
        Ok(record)
    }

    /// One dataset.
    pub async fn get_dataset(&self, dataset_id: &str) -> Result<Option<DatasetRecord>> {
        rdf::load_as(
            &self.engine,
            DATASET_CATALOG_GRAPH,
            &dataset_iri(dataset_id),
        )
        .await
    }

    /// Require a dataset.
    pub async fn require_dataset(&self, dataset_id: &str) -> Result<DatasetRecord> {
        self.get_dataset(dataset_id)
            .await?
            .ok_or_else(|| DataError::NotFound {
                kind: "dataset",
                id: dataset_id.to_owned(),
            })
    }

    /// The catalog, by title.
    pub async fn list_datasets(&self) -> Result<Vec<DatasetRecord>> {
        let mut datasets: Vec<DatasetRecord> =
            rdf::list_as(&self.engine, DATASET_CATALOG_GRAPH, DATASET_CLASS).await?;
        datasets.sort_by(|a, b| {
            a.title
                .cmp(&b.title)
                .then_with(|| a.dataset_id.cmp(&b.dataset_id))
        });
        Ok(datasets)
    }

    /// A version's directory.
    pub fn version_dir(&self, dataset_id: &str, version: &str) -> PathBuf {
        self.base_dir
            .join("datasets")
            .join(dataset_id)
            .join("versions")
            .join(version)
    }

    /// The number of datasets.
    pub async fn count(&self) -> Result<usize> {
        Ok(self.list_datasets().await?.len())
    }

    async fn save(&self, record: &DatasetRecord) -> Result<()> {
        let payload = serde_json::to_value(record)?;
        let subject = dataset_iri(&record.dataset_id);
        for graph in [
            dataset_graph(&record.dataset_id),
            DATASET_CATALOG_GRAPH.to_owned(),
        ] {
            rdf::upsert(
                &self.engine,
                &graph,
                &subject,
                DATASET_CLASS,
                Some(&record.title),
                &payload,
            )
            .await?;
        }
        Ok(())
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn build_manifest_ttl(record: &DatasetRecord) -> String {
    format!(
        "@prefix dcat: <http://www.w3.org/ns/dcat#> .\n\
@prefix dct: <http://purl.org/dc/terms/> .\n\
@prefix mm: <https://metamind.dev/ontology#> .\n\n\
<{}> a dcat:Dataset, mm:Dataset ;\n\
    dct:title \"{}\" ;\n\
{}    dct:format \"{}\" ;\n\
    mm:dataDir \"{}\" .\n",
        dataset_iri(&record.dataset_id),
        turtle_escape(&record.title),
        record
            .description
            .as_ref()
            .map(|description| format!(
                "    dct:description \"{}\" ;\n",
                turtle_escape(description)
            ))
            .unwrap_or_default(),
        turtle_escape(&record.format),
        turtle_escape(&record.data_dir)
    )
}
