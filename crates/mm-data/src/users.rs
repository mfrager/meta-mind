//! User home directories and the FOAF/SKOS profile (memory Tier 4 metadata).
//!
//! Creating a user is filesystem plus RDF: a home tree on disk, `config.ttl`
//! and `profile.ttl` beside it, and one entity in the user index so SPARQL can
//! list users without walking the tree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::RdfEngine;
use crate::error::{DataError, Result};
use crate::rdf::{
    self, legacy_user_iri, now_ms, turtle_escape, user_graph, user_iri, USER_INDEX_GRAPH,
    USER_INDEX_GRAPH as _USER_INDEX,
};

/// What to create.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserSpec {
    /// Display name.
    pub name: String,
    /// Email address.
    pub email: Option<String>,
    /// Role.
    pub role: Option<String>,
    /// Explicit preferences (`verbosity` → −0.5).
    #[serde(default)]
    pub preferences: BTreeMap<String, String>,
}

/// A stored user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserRecord {
    /// Identifier.
    pub user_id: String,
    /// Display name.
    pub name: String,
    /// Email.
    pub email: Option<String>,
    /// Role.
    pub role: Option<String>,
    /// Home directory relative to the data root.
    pub home_dir: String,
    /// Created, Unix milliseconds.
    pub created_at: i64,
}

/// User class.
pub const USER_CLASS: &str = "https://metamind.dev/ontology#User";

/// The user registry.
#[derive(Debug, Clone)]
pub struct UserRegistry {
    engine: Arc<RdfEngine>,
    base_dir: PathBuf,
}

impl UserRegistry {
    /// Build the registry and ensure the users root exists.
    pub fn new(engine: Arc<RdfEngine>, base_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(base_dir.join("users"))?;
        Ok(Self {
            engine,
            base_dir: base_dir.to_path_buf(),
        })
    }

    /// Create a user home and index entry.
    pub async fn create_user(&self, spec: UserSpec) -> Result<UserRecord> {
        if spec.name.trim().is_empty() {
            return Err(DataError::Invalid {
                kind: "user",
                detail: "name must not be empty".to_owned(),
            });
        }
        let id = rdf::new_id();
        let dir = self.base_dir.join("users").join(&id);
        for sub in ["home", "projects", "apps"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        let home_dir = format!("users/{id}/home");
        let record = UserRecord {
            user_id: id.clone(),
            name: spec.name.clone(),
            email: spec.email.clone(),
            role: spec.role.clone(),
            home_dir,
            created_at: now_ms(),
        };

        std::fs::write(dir.join("config.ttl"), build_config_ttl(&record, &spec))?;
        std::fs::write(dir.join("profile.ttl"), build_profile_ttl(&record))?;

        let payload = serde_json::to_value(&record)?;
        let subject = user_iri(&id);
        rdf::upsert(
            &self.engine,
            &user_graph(&id),
            &subject,
            USER_CLASS,
            Some(&record.name),
            &payload,
        )
        .await?;
        rdf::upsert(
            &self.engine,
            _USER_INDEX,
            &subject,
            USER_CLASS,
            Some(&record.name),
            &payload,
        )
        .await?;
        Ok(record)
    }

    /// One user.
    ///
    /// The canonical `mmd:user/<id>` index entry is authoritative. A legacy
    /// `urn:metamind:user:<id>` entry is still readable during migration, so an
    /// upgrade does not hide users created before Phase 12.
    pub async fn get_user(&self, user_id: &str) -> Result<Option<UserRecord>> {
        if let Some(record) =
            rdf::load_as(&self.engine, USER_INDEX_GRAPH, &user_iri(user_id)).await?
        {
            return Ok(Some(record));
        }
        rdf::load_as(&self.engine, USER_INDEX_GRAPH, &legacy_user_iri(user_id)).await
    }

    /// Require a user.
    pub async fn require_user(&self, user_id: &str) -> Result<UserRecord> {
        self.get_user(user_id)
            .await?
            .ok_or_else(|| DataError::NotFound {
                kind: "user",
                id: user_id.to_owned(),
            })
    }

    /// Every user, by name.
    ///
    /// Both canonical and legacy index entries are listed, deduplicated by the
    /// stable user id. The canonical entry wins whenever both spellings exist,
    /// regardless of graph enumeration order.
    pub async fn list_users(&self) -> Result<Vec<UserRecord>> {
        let subjects = rdf::subjects_of_class(&self.engine, USER_INDEX_GRAPH, USER_CLASS).await?;
        let mut by_id: std::collections::BTreeMap<String, UserRecord> =
            std::collections::BTreeMap::new();
        for subject in subjects {
            if let Some(record) =
                rdf::load_as::<UserRecord>(&self.engine, USER_INDEX_GRAPH, &subject).await?
            {
                let canonical = subject == user_iri(&record.user_id);
                match by_id.entry(record.user_id.clone()) {
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
        let mut users = by_id.into_values().collect::<Vec<_>>();
        users.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.user_id.cmp(&b.user_id)));
        Ok(users)
    }

    /// The absolute home directory of a user.
    pub fn user_home(&self, user_id: &str) -> PathBuf {
        self.base_dir.join("users").join(user_id).join("home")
    }

    /// Link a project into the user's `projects/` directory.
    pub async fn link_project(&self, user_id: &str, project_id: &str) -> Result<()> {
        let target = self.base_dir.join("projects").join(project_id);
        if !target.exists() {
            return Err(DataError::NotFound {
                kind: "project",
                id: project_id.to_owned(),
            });
        }
        let link = self
            .base_dir
            .join("users")
            .join(user_id)
            .join("projects")
            .join(project_id);
        link_symlink(&target, &link)?;
        Ok(())
    }

    /// Link an app into the user's `apps/` directory.
    pub async fn link_app(&self, user_id: &str, app_id: &str) -> Result<()> {
        let target = self.base_dir.join("apps").join(app_id);
        if !target.exists() {
            return Err(DataError::NotFound {
                kind: "app",
                id: app_id.to_owned(),
            });
        }
        let link = self
            .base_dir
            .join("users")
            .join(user_id)
            .join("apps")
            .join(app_id);
        link_symlink(&target, &link)?;
        Ok(())
    }

    /// The number of users.
    pub async fn count(&self) -> Result<usize> {
        Ok(self.list_users().await?.len())
    }
}

fn link_symlink(target: &Path, link: &Path) -> Result<()> {
    if link.exists() {
        return Ok(());
    }
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(link, target.to_string_lossy().as_bytes())?;
    }
    Ok(())
}

fn build_config_ttl(record: &UserRecord, spec: &UserSpec) -> String {
    let mut preferences = String::new();
    for (key, value) in &spec.preferences {
        preferences.push_str(&format!("    mm:{key} \"{}\" ;\n", turtle_escape(value)));
    }
    format!(
        "@prefix mm: <https://metamind.dev/ontology#> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\n\
<{}> a foaf:Person, mm:User ;\n\
    foaf:name \"{}\" ;\n\
    mm:homeDir \"{}\" ;\n\
    mm:created \"{}\"^^xsd:dateTime ;\n\
{}    mm:preferences [\n{}    ] .\n",
        user_iri(&record.user_id),
        turtle_escape(&record.name),
        turtle_escape(&record.home_dir),
        record.created_at,
        spec.email
            .as_ref()
            .map(|email| format!("    foaf:mbox \"{}\" ;\n", turtle_escape(email)))
            .unwrap_or_default(),
        preferences
    )
}

fn build_profile_ttl(record: &UserRecord) -> String {
    format!(
        "@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
@prefix mm: <https://metamind.dev/ontology#> .\n\n\
<{}> a foaf:Person ;\n\
    foaf:name \"{}\" ;\n\
    mm:homeDir \"{}\" .\n",
        user_iri(&record.user_id),
        turtle_escape(&record.name),
        turtle_escape(&record.home_dir)
    )
}
