//! The kernel context every command opens.
//!
//! Opening is ordered so that a failure leaves nothing half-built: configuration,
//! then the tabular store and its migrations, then the logger (which needs the
//! store as its audit writer), then the event log, and only then the graph store —
//! which is the slowest and the only one a read-only command can skip.

use std::path::PathBuf;
use std::sync::Arc;

use mm_core::{Config, MmError, UlidFactory};
use mm_eventlog::EventLog;
use mm_log::Logger;
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

/// An open kernel.
pub struct Kernel {
    /// The resolved configuration.
    pub cfg: Config,
    /// The tabular store.
    pub sqlite: SqliteStore,
    /// The RDF store, when the command needs it.
    pub graph: Option<GraphStore>,
    /// The logger, wired to the audit chain.
    pub logger: Arc<Logger>,
    /// The event log.
    pub events: EventLog,
    /// The monotonic identifier factory. Exposed so a projection that needs new
    /// primary keys (the codex index) shares the kernel's watermark instead of
    /// opening a second factory over the same file.
    pub ids: Arc<UlidFactory>,
}

impl Kernel {
    /// Resolve configuration from `--data-dir`, `--config`, or the repository.
    pub fn resolve_config(
        config: Option<PathBuf>,
        data_dir: Option<PathBuf>,
    ) -> Result<Config, MmError> {
        if let Some(dir) = data_dir {
            return Ok(Config::for_data_dir(dir));
        }
        match config {
            Some(path) => Config::load(&path),
            None => Config::load_default(),
        }
    }

    /// Open the kernel, optionally including the RDF store.
    pub async fn open(cfg: Config, with_graph: bool) -> Result<Self, MmError> {
        std::fs::create_dir_all(&cfg.store.data_dir)?;

        let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await?;
        sqlite.migrate().await?;

        let logger = Arc::new(Logger::from_config(
            &cfg.log,
            Some(Arc::new(sqlite.clone())),
        )?);

        let ids = Arc::new(UlidFactory::open(&cfg.ulid_watermark_path())?);
        let events = EventLog::new(sqlite.clone(), Arc::clone(&logger), Arc::clone(&ids));

        logger
            .emit(
                mm_log::LogRecord::new(mm_log::Level::Info, mm_log::codes::LOG_INIT, "mm.cli")
                    .with_field("sinks", logger.sink_names())
                    .with_field("level", logger.min_level().as_str())
                    .with_field("log_dir", cfg.log.dir.display().to_string()),
            )
            .await?;

        let graph = if with_graph {
            Some(GraphStore::open(&cfg.store.graph_dir, &cfg.shapes_file()).await?)
        } else {
            None
        };

        Ok(Kernel {
            cfg,
            sqlite,
            graph,
            logger,
            events,
            ids,
        })
    }

    /// The graph store, or an error explaining that this command needs it.
    pub fn graph(&self) -> Result<&GraphStore, MmError> {
        self.graph
            .as_ref()
            .ok_or_else(|| MmError::Graph("this kernel was opened without the RDF store".into()))
    }

    /// Load the ontology files into the default graph.
    ///
    /// Returns how many triples were added (0 when the ontology was already
    /// loaded), so a caller can avoid recording an event for a no-op.
    pub async fn load_ontology(&self) -> Result<usize, MmError> {
        let files = self.cfg.ontology_files();
        for file in &files {
            if !file.exists() {
                return Err(MmError::Config(format!(
                    "ontology file missing: {}",
                    file.display()
                )));
            }
        }
        self.graph()?.handle().load_ontology(files).await
    }
}
