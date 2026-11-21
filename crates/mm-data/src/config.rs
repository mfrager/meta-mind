//! Unified data configuration.
//!
//! One `config.toml` under the data root describes the RDF store and every
//! memory tier, mirroring `design/rdf_backend_integration.md` §VII. Paths in
//! the file are relative to the data root; an explicit [`InitConfig::base_dir`]
//! wins over `MM_DATA_DIR`, which wins over `~/.metamind`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{DataError, Result};

/// Environment variable that points at the data root.
pub const DATA_DIR_ENV: &str = "MM_DATA_DIR";
/// Name of the unified configuration file.
pub const CONFIG_FILE: &str = "config.toml";

/// Settings for the RDF engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// Store the graph in RocksDB under `cubecast/main`; otherwise keep it in
    /// memory. Tests use in-memory stores.
    pub persistent: bool,
    /// Enable the multi-graph registry. Named graphs work in a single store
    /// regardless; this flag controls the registry layer.
    pub enable_multi_graph: bool,
    /// Enable the semantic registry artifact directory.
    pub enable_registry: bool,
    /// Enable the cubecast project layer.
    pub enable_projects: bool,
    /// Enable MinerU document extraction.
    pub enable_extractions: bool,
    /// Enable inference at the service layer.
    pub enable_inference: bool,
    /// RocksDB `max_open_files`, when set.
    pub rocksdb_max_open_files: Option<u32>,
    /// Repository ontology directory. `None` resolves from the build tree.
    pub ontology_dir: Option<PathBuf>,
    /// Load the repository ontology on open.
    pub load_ontology: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            persistent: false,
            enable_multi_graph: false,
            enable_registry: false,
            enable_projects: true,
            enable_extractions: false,
            enable_inference: false,
            rocksdb_max_open_files: None,
            ontology_dir: None,
            load_ontology: true,
        }
    }
}

/// Tier 1: raw episodic turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpisodicConfig {
    /// Days a turn is kept before it is eligible for compression.
    pub retention_days: u32,
    /// Compress after this many uncompressed turns.
    pub compression_trigger_turns: u32,
    /// Compress after this many uncompressed tokens.
    pub compression_trigger_tokens: usize,
}

impl Default for EpisodicConfig {
    fn default() -> Self {
        Self {
            retention_days: 30,
            compression_trigger_turns: 10,
            compression_trigger_tokens: 2_000,
        }
    }
}

/// Tier 2: compressed segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SegmentConfig {
    /// Days a segment is kept before project aggregation.
    pub retention_days: u32,
    /// Target tokens per segment.
    pub segment_tokens: usize,
    /// Turns of overlap between adjacent segments.
    pub overlap_turns: usize,
    /// Validate every N segments; zero disables sampling.
    pub validate_every_n: u32,
    /// Minimum acceptable decision recall.
    pub min_decision_recall: f32,
    /// Minimum acceptable commitment recall.
    pub min_commitment_recall: f32,
    /// Minimum acceptable artifact recall.
    pub min_artifact_recall: f32,
    /// Minimum acceptable narrative faithfulness.
    pub min_narrative_faithfulness: f32,
    /// Maximum acceptable hallucination rate.
    pub max_hallucination_rate: f32,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            retention_days: 365,
            segment_tokens: 2_000,
            overlap_turns: 2,
            validate_every_n: 10,
            min_decision_recall: 0.90,
            min_commitment_recall: 0.90,
            min_artifact_recall: 0.85,
            min_narrative_faithfulness: 0.85,
            max_hallucination_rate: 0.02,
        }
    }
}

/// Tier 3: project memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectMemoryConfig {
    /// Merge new segments into project memory.
    pub aggregation_enabled: bool,
    /// Suggest the next step from blocked work and goals.
    pub continuation_suggestion_enabled: bool,
    /// Deduplicate decisions by `(topic, choice)`.
    pub dedup_decisions_by_topic: bool,
    /// Cap on the generated project narrative, in characters.
    pub narrative_max_tokens: usize,
}

impl Default for ProjectMemoryConfig {
    fn default() -> Self {
        Self {
            aggregation_enabled: true,
            continuation_suggestion_enabled: true,
            dedup_decisions_by_topic: true,
            narrative_max_tokens: 500,
        }
    }
}

/// Tier 4: user memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserMemoryConfig {
    /// Transfer project conventions and expertise into user memory.
    pub cross_project_learning_enabled: bool,
    /// Update the merged preference model from observations.
    pub preference_learning_enabled: bool,
    /// Regenerate the global narrative after a sync.
    pub global_narrative_enabled: bool,
}

impl Default for UserMemoryConfig {
    fn default() -> Self {
        Self {
            cross_project_learning_enabled: true,
            preference_learning_enabled: true,
            global_narrative_enabled: true,
        }
    }
}

/// Compression pipeline settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionConfig {
    /// Model used to extract structured artifacts.
    pub extraction_model: String,
    /// Model used to write the narrative.
    pub narrative_model: String,
    /// Model used for embeddings.
    pub embedding_model: String,
    /// Validate a sample of segments.
    pub quality_validation_enabled: bool,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            extraction_model: "heuristic".to_owned(),
            narrative_model: "template".to_owned(),
            embedding_model: "hashing-384".to_owned(),
            quality_validation_enabled: false,
        }
    }
}

/// Vector index settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VectorIndexConfig {
    /// Embedding dimension.
    pub dimension: usize,
    /// Distance metric name.
    pub metric: String,
}

impl Default for VectorIndexConfig {
    fn default() -> Self {
        Self {
            dimension: 384,
            metric: "cosine".to_owned(),
        }
    }
}

/// Synchronisation settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Enable multi-device presence and broadcast.
    pub multi_device_enabled: bool,
    /// Presence lease, in seconds.
    pub presence_ttl_seconds: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            multi_device_enabled: true,
            presence_ttl_seconds: 300,
        }
    }
}

/// Every memory-tier setting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Master switch.
    pub enabled: bool,
    /// Tier 1.
    pub episodic: EpisodicConfig,
    /// Tier 2.
    pub segments: SegmentConfig,
    /// Tier 3.
    pub project: ProjectMemoryConfig,
    /// Tier 4.
    pub user: UserMemoryConfig,
    /// Compression pipeline.
    pub compression: CompressionConfig,
    /// Vector index.
    pub vector_index: VectorIndexConfig,
    /// Multi-device sync.
    pub sync: SyncConfig,
}

/// The file written at `$MM_DATA_DIR/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataConfig {
    /// Identity of this data root.
    pub system: SystemConfig,
    /// Engine settings.
    pub engine: EngineConfig,
    /// Memory tiers.
    pub memory: MemoryConfig,
}

/// System identity stored beside the engine configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    /// Human-readable instance name.
    pub kernel_name: String,
    /// Short codename used in IRIs.
    pub codename: String,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            kernel_name: "metamind".to_owned(),
            codename: "mm".to_owned(),
        }
    }
}

/// How to initialise the data layer.
#[derive(Debug, Clone, Default)]
pub struct InitConfig {
    /// Explicit data root.
    pub base_dir: Option<PathBuf>,
    /// Engine overrides; the file's value wins when the file exists.
    pub engine: EngineConfig,
    /// Memory overrides.
    pub memory: MemoryConfig,
    /// Force an in-memory store even when the configuration says persistent.
    pub in_memory: bool,
}

/// Resolve the data root without touching the filesystem.
pub fn resolve_base_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(value) = std::env::var_os(DATA_DIR_ENV) {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        DataError::Config(format!(
            "no data directory: set {DATA_DIR_ENV} or HOME, or pass base_dir"
        ))
    })?;
    Ok(PathBuf::from(home).join(".metamind"))
}

/// Read `config.toml`, or create it from the supplied defaults.
///
/// A file that already exists is authoritative; the in-memory config only seeds
/// a fresh root. This keeps a reboot from silently changing a user's engine.
pub fn load_or_create_config(base_dir: &Path, init: &InitConfig) -> Result<DataConfig> {
    let path = base_dir.join(CONFIG_FILE);
    if path.exists() {
        let document = std::fs::read_to_string(&path)?;
        return toml::from_str(&document).map_err(Into::into);
    }
    let config = DataConfig {
        system: SystemConfig::default(),
        engine: EngineConfig {
            persistent: !init.in_memory,
            ..init.engine.clone()
        },
        memory: init.memory.clone(),
    };
    let document = toml::to_string_pretty(&config)?;
    std::fs::write(&path, document)?;
    Ok(config)
}

/// The directory structure every data root has, in creation order.
pub const DIRECTORIES: &[&str] = &[
    "cubecast/main",
    "cubecast/graphs/stores",
    "cubecast/registry",
    "cubecast/projects",
    "cubecast/extractions",
    "cubecast/cache",
    "agentstream/sessions",
    "agentstream/events",
    "agentstream/projections",
    "users",
    "projects",
    "apps",
    "datasets",
    "sqlite",
    "logs",
    "temp/uploads",
];

/// Create the data-root layout. Idempotent.
pub fn create_directory_structure(base: &Path) -> Result<()> {
    std::fs::create_dir_all(base)?;
    for dir in DIRECTORIES {
        std::fs::create_dir_all(base.join(dir))?;
    }
    Ok(())
}
