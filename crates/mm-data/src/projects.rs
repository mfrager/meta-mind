//! Project home directories, DOAP manifest and membership (Tier 3 metadata).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::RdfEngine;
use crate::error::{DataError, Result};
use crate::rdf::{
    self, legacy_project_iri, now_ms, project_graph, project_iri, turtle_escape,
    PROJECT_INDEX_GRAPH,
};

/// What to create.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectSpec {
    /// Display name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Owning user id.
    pub owner: Option<String>,
}

/// A stored project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    /// Identifier.
    pub project_id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Home directory relative to the data root.
    pub home_dir: String,
    /// Members.
    #[serde(default)]
    pub members: Vec<String>,
    /// Linked datasets.
    #[serde(default)]
    pub datasets: Vec<String>,
    /// Created, Unix milliseconds.
    pub created_at: i64,
}

/// Project class.
pub const PROJECT_CLASS: &str = "https://metamind.dev/ontology#Project";

/// The project registry.
#[derive(Debug, Clone)]
pub struct ProjectRegistry {
    engine: Arc<RdfEngine>,
    base_dir: PathBuf,
}

impl ProjectRegistry {
    /// Build the registry.
    pub fn new(engine: Arc<RdfEngine>, base_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(base_dir.join("projects"))?;
        Ok(Self {
            engine,
            base_dir: base_dir.to_path_buf(),
        })
    }

    /// Create a project home and manifest.
    pub async fn create_project(&self, spec: ProjectSpec) -> Result<ProjectRecord> {
        if spec.name.trim().is_empty() {
            return Err(DataError::Invalid {
                kind: "project",
                detail: "name must not be empty".to_owned(),
            });
        }
        let id = rdf::new_id();
        let dir = self.base_dir.join("projects").join(&id);
        for sub in ["home", "members", "datasets", "agents", "logs"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        let record = ProjectRecord {
            project_id: id.clone(),
            name: spec.name.clone(),
            description: spec.description.clone(),
            home_dir: format!("projects/{id}/home"),
            members: spec.owner.clone().into_iter().collect(),
            datasets: Vec::new(),
            created_at: now_ms(),
        };
        std::fs::write(dir.join("manifest.ttl"), build_manifest_ttl(&record))?;
        std::fs::write(
            dir.join("config.ttl"),
            format!(
                "@prefix mm: <https://metamind.dev/ontology#> .\n\n<{}> a mm:Project .\n",
                project_iri(&id)
            ),
        )?;

        let payload = serde_json::to_value(&record)?;
        let subject = project_iri(&id);
        rdf::upsert(
            &self.engine,
            &project_graph(&id),
            &subject,
            PROJECT_CLASS,
            Some(&record.name),
            &payload,
        )
        .await?;
        rdf::upsert(
            &self.engine,
            PROJECT_INDEX_GRAPH,
            &subject,
            PROJECT_CLASS,
            Some(&record.name),
            &payload,
        )
        .await?;
        Ok(record)
    }

    /// One project.
    ///
    /// Reads the canonical `mmd:project/<id>` entry first, then the legacy
    /// `urn:metamind:project:<id>` entry so pre-Phase-12 projects remain visible.
    pub async fn get_project(&self, project_id: &str) -> Result<Option<ProjectRecord>> {
        if let Some(record) =
            rdf::load_as(&self.engine, PROJECT_INDEX_GRAPH, &project_iri(project_id)).await?
        {
            return Ok(Some(record));
        }
        rdf::load_as(
            &self.engine,
            PROJECT_INDEX_GRAPH,
            &legacy_project_iri(project_id),
        )
        .await
    }

    /// Require a project.
    pub async fn require_project(&self, project_id: &str) -> Result<ProjectRecord> {
        self.get_project(project_id)
            .await?
            .ok_or_else(|| DataError::NotFound {
                kind: "project",
                id: project_id.to_owned(),
            })
    }

    /// Every project, by name.
    ///
    /// Canonical and legacy index entries are deduplicated by project id; the
    /// canonical spelling wins regardless of graph enumeration order.
    pub async fn list_projects(&self) -> Result<Vec<ProjectRecord>> {
        let subjects =
            rdf::subjects_of_class(&self.engine, PROJECT_INDEX_GRAPH, PROJECT_CLASS).await?;
        let mut by_id: std::collections::BTreeMap<String, ProjectRecord> =
            std::collections::BTreeMap::new();
        for subject in subjects {
            if let Some(record) =
                rdf::load_as::<ProjectRecord>(&self.engine, PROJECT_INDEX_GRAPH, &subject).await?
            {
                let canonical = subject == project_iri(&record.project_id);
                match by_id.entry(record.project_id.clone()) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(record);
                    }
                    std::collections::btree_map::Entry::Occupied(mut slot) => {
                        if canonical {
                            slot.insert(record);
                        }
                    }
                }
            }
        }
        let mut projects = by_id.into_values().collect::<Vec<_>>();
        projects.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.project_id.cmp(&b.project_id))
        });
        Ok(projects)
    }

    /// Add a member and link the project into their directory.
    pub async fn add_member(&self, project_id: &str, user_id: &str) -> Result<ProjectRecord> {
        let mut record = self.require_project(project_id).await?;
        if !record.members.iter().any(|member| member == user_id) {
            record.members.push(user_id.to_owned());
            self.save(&record).await?;
            let user_link = self
                .base_dir
                .join("users")
                .join(user_id)
                .join("projects")
                .join(project_id);
            let target = self.base_dir.join("projects").join(project_id);
            if let Some(parent) = user_link.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                if !user_link.exists() {
                    std::os::unix::fs::symlink(&target, &user_link)?;
                }
            }
        }
        Ok(record)
    }

    /// Link a dataset to a project.
    pub async fn add_dataset(&self, project_id: &str, dataset_id: &str) -> Result<ProjectRecord> {
        let mut record = self.require_project(project_id).await?;
        if !record.datasets.iter().any(|dataset| dataset == dataset_id) {
            record.datasets.push(dataset_id.to_owned());
            self.save(&record).await?;
            let link = self
                .base_dir
                .join("projects")
                .join(project_id)
                .join("datasets")
                .join(dataset_id);
            let target = self.base_dir.join("datasets").join(dataset_id);
            if let Some(parent) = link.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                if !link.exists() {
                    std::os::unix::fs::symlink(&target, &link)?;
                }
            }
        }
        Ok(record)
    }

    /// The absolute project home.
    pub fn project_home(&self, project_id: &str) -> PathBuf {
        self.base_dir.join("projects").join(project_id).join("home")
    }

    /// The project directory.
    pub fn project_dir(&self, project_id: &str) -> PathBuf {
        self.base_dir.join("projects").join(project_id)
    }

    async fn save(&self, record: &ProjectRecord) -> Result<()> {
        let payload = serde_json::to_value(record)?;
        let subject = project_iri(&record.project_id);
        for graph in [
            project_graph(&record.project_id),
            PROJECT_INDEX_GRAPH.to_owned(),
        ] {
            rdf::upsert(
                &self.engine,
                &graph,
                &subject,
                PROJECT_CLASS,
                Some(&record.name),
                &payload,
            )
            .await?;
        }
        Ok(())
    }

    /// The number of projects.
    pub async fn count(&self) -> Result<usize> {
        Ok(self.list_projects().await?.len())
    }
}

fn build_manifest_ttl(record: &ProjectRecord) -> String {
    format!(
        "@prefix doap: <http://usefulinc.com/ns/doap#> .\n\
@prefix dct: <http://purl.org/dc/terms/> .\n\
@prefix mm: <https://metamind.dev/ontology#> .\n\n\
<{}> a doap:Project, mm:Project ;\n\
    doap:name \"{}\" ;\n\
{}    mm:homeDir \"{}\" .\n",
        project_iri(&record.project_id),
        turtle_escape(&record.name),
        record
            .description
            .as_ref()
            .map(|description| format!(
                "    doap:description \"{}\" ;\n",
                turtle_escape(description)
            ))
            .unwrap_or_default(),
        turtle_escape(&record.home_dir)
    )
}
