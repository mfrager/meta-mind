//! The nine-step data-layer bootstrap.
//!
//! ```text
//! 1 resolve base dir      2 create layout      3 load/create config.toml
//! 4 open RDF engine       5 load ontology      6 agentstream
//! 7 registries            8 SQLite            9 health check + memory system
//! ```
//!
//! Every step is idempotent. Reopening an existing root reads its
//! `config.toml` rather than overwriting it, so a restart cannot silently
//! change a user's engine.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::agentstream::AgentstreamStore;
use crate::apps::AppRegistry;
use crate::config::{
    create_directory_structure, load_or_create_config, resolve_base_dir, DataConfig, InitConfig,
};
use crate::data::DataService;
use crate::datasets::DatasetCatalog;
use crate::engine::RdfEngine;
use crate::error::Result;
use crate::memory::MemorySystem;
use crate::ontology::{default_ontology_dir, load_metamind_ontology};
use crate::projects::ProjectRegistry;
use crate::rdf::{APP_INDEX_GRAPH, PROJECT_INDEX_GRAPH, USER_INDEX_GRAPH};
use crate::sqlite::SqliteStore;
use crate::users::UserRegistry;

/// Everything the bootstrap produced.
pub struct InitResult {
    /// The open RDF engine.
    pub engine: Arc<RdfEngine>,
    /// Agentstream store.
    pub agentstream: Arc<AgentstreamStore>,
    /// Users.
    pub users: UserRegistry,
    /// Projects.
    pub projects: ProjectRegistry,
    /// Apps.
    pub apps: AppRegistry,
    /// Datasets.
    pub datasets: DatasetCatalog,
    /// Tabular store.
    pub sqlite: SqliteStore,
    /// The managed-data service: RDF instance writes, outbox, schema catalog,
    /// namespaces, and the lexical index.
    pub data_service: Arc<DataService>,
    /// Unified memory. Shared with the server and conversation engine.
    pub memory_system: Arc<MemorySystem>,
    /// The configuration actually in force.
    pub config: DataConfig,
    /// The resolved data root.
    pub base_dir: PathBuf,
    /// Health information from the final step.
    pub health: DataHealth,
}

/// The result of the step-9 probes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataHealth {
    /// Quad count.
    pub quads: usize,
    /// Named graphs seen.
    pub graphs: usize,
    /// Ontology triples loaded.
    pub ontology_triples: usize,
    /// SQLite schema version.
    pub sqlite_schema: String,
}

/// Initialise (or reopen) the data layer.
pub async fn initialize_data_layer(init: InitConfig) -> Result<InitResult> {
    // 1. Resolve the root.
    let base_dir = resolve_base_dir(init.base_dir.as_deref())?;

    // 2. Layout.
    create_directory_structure(&base_dir)?;

    // 3. Configuration.
    let config = load_or_create_config(&base_dir, &init)?;
    let mut engine_config = config.engine.clone();
    if init.in_memory {
        engine_config.persistent = false;
    }

    // 4. Engine.
    let engine = Arc::new(RdfEngine::open(&base_dir, engine_config.clone()).await?);

    // 5. Ontology.
    let ontology_triples = if engine_config.load_ontology {
        let dir = default_ontology_dir(
            config
                .engine
                .ontology_dir
                .as_deref()
                .or(engine_config.ontology_dir.as_deref()),
        );
        load_metamind_ontology(&engine, &dir).await?
    } else {
        0
    };

    // 6. Agentstream.
    let agentstream = Arc::new(AgentstreamStore::new(Arc::clone(&engine), &base_dir)?);

    // 7. Registries.
    let users = UserRegistry::new(Arc::clone(&engine), &base_dir)?;
    let projects = ProjectRegistry::new(Arc::clone(&engine), &base_dir)?;
    let apps = AppRegistry::new(Arc::clone(&engine), &base_dir)?;
    let datasets = DatasetCatalog::new(Arc::clone(&engine), &base_dir)?;
    write_index_stubs(&base_dir)?;

    // 8. SQLite.
    let sqlite = SqliteStore::open(&base_dir.join("sqlite")).await?;
    let sqlite_schema = sqlite.schema_version().await?;
    if !sqlite.is_ready().await? {
        return Err(crate::error::DataError::Config(
            "sqlite readiness probe failed".to_owned(),
        ));
    }

    // 9. Health and memory.
    let memory_system = Arc::new(MemorySystem::new(
        Arc::clone(&engine),
        &base_dir,
        config.memory.clone(),
    )?);
    // Phase 3: the single managed-data seam. It bootstraps the schema catalog
    // and namespaces, opens the Tantivy index, imports a snapshot into an empty
    // store, and reconciles any pending outbox rows from the previous run.
    let data_service = Arc::new(
        DataService::open(Arc::clone(&engine), &sqlite, &base_dir)
            .await
            .map_err(|error| {
                crate::error::DataError::Config(format!("data service failed to open: {error}"))
            })?,
    );
    let quads = engine.quad_count().await?;
    let graphs = engine.named_graphs().await?.len();
    for graph in [USER_INDEX_GRAPH, PROJECT_INDEX_GRAPH, APP_INDEX_GRAPH] {
        // An empty index is healthy; the ask is only a reachability probe.
        let _ = engine.graph_is_ready(graph).await?;
    }
    let health = DataHealth {
        quads,
        graphs,
        ontology_triples,
        sqlite_schema,
    };

    Ok(InitResult {
        engine,
        agentstream,
        users,
        projects,
        apps,
        datasets,
        sqlite,
        data_service,
        memory_system,
        config,
        base_dir,
        health,
    })
}

/// Write the three index documents if they do not exist. They are empty
/// Turtle documents today; keeping them on disk makes the layout inspectable and
/// gives an operator a file to point tools at.
fn write_index_stubs(base_dir: &std::path::Path) -> Result<()> {
    for (dir, name) in [
        ("users", "index.ttl"),
        ("projects", "index.ttl"),
        ("apps", "index.ttl"),
    ] {
        let path = base_dir.join(dir).join(name);
        if !path.exists() {
            std::fs::write(&path, "@prefix mm: <https://metamind.dev/ontology#> .\n")?;
        }
    }
    let catalog = base_dir.join("datasets").join("catalog.ttl");
    if !catalog.exists() {
        std::fs::write(
            &catalog,
            "@prefix dcat: <http://www.w3.org/ns/dcat#> .\n\
@prefix dct: <http://purl.org/dc/terms/> .\n\n\
<urn:metamind:datasets> a dcat:Catalog ;\n\
    dct:title \"MetaMind Datasets Catalog\" .\n",
        )?;
    }
    Ok(())
}
