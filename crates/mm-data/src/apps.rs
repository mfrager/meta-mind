//! App home directories, manifests, plugins and app data.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::RdfEngine;
use crate::error::{DataError, Result};
use crate::rdf::{self, app_graph, app_iri, now_ms, turtle_escape, APP_INDEX_GRAPH};

/// What to create.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSpec {
    /// Display name.
    pub name: String,
    /// Version.
    pub version: Option<String>,
    /// Owner user id.
    pub owner: Option<String>,
}

/// A stored app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppRecord {
    /// Identifier.
    pub app_id: String,
    /// Display name.
    pub name: String,
    /// Version.
    pub version: Option<String>,
    /// Home directory relative to the data root.
    pub home_dir: String,
    /// Plugins.
    #[serde(default)]
    pub plugins: Vec<String>,
    /// Created, Unix milliseconds.
    pub created_at: i64,
}

/// App class.
pub const APP_CLASS: &str = "https://metamind.dev/ontology#App";

/// The app registry.
#[derive(Debug, Clone)]
pub struct AppRegistry {
    engine: Arc<RdfEngine>,
    base_dir: PathBuf,
}

impl AppRegistry {
    /// Build the registry.
    pub fn new(engine: Arc<RdfEngine>, base_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(base_dir.join("apps"))?;
        Ok(Self {
            engine,
            base_dir: base_dir.to_path_buf(),
        })
    }

    /// Create an app home and manifest.
    pub async fn create_app(&self, spec: AppSpec) -> Result<AppRecord> {
        if spec.name.trim().is_empty() {
            return Err(DataError::Invalid {
                kind: "app",
                detail: "name must not be empty".to_owned(),
            });
        }
        let id = rdf::new_id();
        let dir = self.base_dir.join("apps").join(&id);
        for sub in ["home", "data", "plugins"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        let record = AppRecord {
            app_id: id.clone(),
            name: spec.name.clone(),
            version: spec.version.clone(),
            home_dir: format!("apps/{id}/home"),
            plugins: Vec::new(),
            created_at: now_ms(),
        };
        std::fs::write(dir.join("manifest.ttl"), build_manifest_ttl(&record))?;
        std::fs::write(
            dir.join("config.ttl"),
            format!(
                "@prefix mm: <https://metamind.dev/ontology#> .\n\n<{}> a mm:App .\n",
                app_iri(&id)
            ),
        )?;
        let payload = serde_json::to_value(&record)?;
        let subject = app_iri(&id);
        rdf::upsert(
            &self.engine,
            &app_graph(&id),
            &subject,
            APP_CLASS,
            Some(&record.name),
            &payload,
        )
        .await?;
        rdf::upsert(
            &self.engine,
            APP_INDEX_GRAPH,
            &subject,
            APP_CLASS,
            Some(&record.name),
            &payload,
        )
        .await?;
        Ok(record)
    }

    /// One app.
    pub async fn get_app(&self, app_id: &str) -> Result<Option<AppRecord>> {
        rdf::load_as(&self.engine, APP_INDEX_GRAPH, &app_iri(app_id)).await
    }

    /// Require an app.
    pub async fn require_app(&self, app_id: &str) -> Result<AppRecord> {
        self.get_app(app_id)
            .await?
            .ok_or_else(|| DataError::NotFound {
                kind: "app",
                id: app_id.to_owned(),
            })
    }

    /// Every app.
    pub async fn list_apps(&self) -> Result<Vec<AppRecord>> {
        let mut apps: Vec<AppRecord> =
            rdf::list_as(&self.engine, APP_INDEX_GRAPH, APP_CLASS).await?;
        apps.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.app_id.cmp(&b.app_id)));
        Ok(apps)
    }

    /// Register a plugin directory.
    pub async fn add_plugin(&self, app_id: &str, plugin: &str) -> Result<AppRecord> {
        let mut record = self.require_app(app_id).await?;
        record.plugins.push(plugin.to_owned());
        record.plugins.sort();
        record.plugins.dedup();
        std::fs::create_dir_all(
            self.base_dir
                .join("apps")
                .join(app_id)
                .join("plugins")
                .join(plugin),
        )?;
        let payload = serde_json::to_value(&record)?;
        let subject = app_iri(&record.app_id);
        for graph in [app_graph(&record.app_id), APP_INDEX_GRAPH.to_owned()] {
            rdf::upsert(
                &self.engine,
                &graph,
                &subject,
                APP_CLASS,
                Some(&record.name),
                &payload,
            )
            .await?;
        }
        Ok(record)
    }

    /// The absolute app home.
    pub fn app_home(&self, app_id: &str) -> PathBuf {
        self.base_dir.join("apps").join(app_id).join("home")
    }

    /// The number of apps.
    pub async fn count(&self) -> Result<usize> {
        Ok(self.list_apps().await?.len())
    }
}

fn build_manifest_ttl(record: &AppRecord) -> String {
    format!(
        "@prefix doap: <http://usefulinc.com/ns/doap#> .\n\
@prefix mm: <https://metamind.dev/ontology#> .\n\n\
<{}> a doap:Project, mm:App ;\n\
    doap:name \"{}\" ;\n\
{}    mm:homeDir \"{}\" .\n",
        app_iri(&record.app_id),
        turtle_escape(&record.name),
        record
            .version
            .as_ref()
            .map(|version| format!("    doap:version \"{}\" ;\n", turtle_escape(version)))
            .unwrap_or_default(),
        turtle_escape(&record.home_dir)
    )
}
