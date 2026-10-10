//! Kernel configuration.
//!
//! Paths in `config/metamind.toml` are written relative to the repository root
//! and are resolved against it, so the same file works from any working
//! directory. `MM_CONFIG` and `MM_DATA_DIR` override discovery for tests and CI.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::MmError;

/// The whole kernel configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Identity of the being whose kernel this is.
    pub kernel: KernelConfig,
    /// Where state lives.
    pub store: StoreConfig,
    /// Where records go.
    pub log: LogConfig,
    /// The repository root that relative paths resolved against. Not part of the
    /// file: it is derived from where the file was found.
    #[serde(skip, default)]
    pub root: PathBuf,
}

/// Who this kernel belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelConfig {
    /// Machine name of the being.
    pub name: String,
    /// Short codename used in IRIs and crate names.
    pub codename: String,
}

/// Store locations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreConfig {
    /// Root of all runtime state.
    pub data_dir: PathBuf,
    /// The SQLite database file.
    pub sqlite_file: PathBuf,
    /// The Oxigraph/RocksDB directory.
    pub graph_dir: PathBuf,
}

/// Logging destinations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogConfig {
    /// Minimum level: `error`, `warn`, `info`, `debug`, or `trace`.
    pub level: String,
    /// Directory for JSONL and audit files.
    pub dir: PathBuf,
    /// Whether records are also written to stdout.
    #[serde(default = "default_true")]
    pub console: bool,
    /// Whether records are also appended to `<dir>/mm.jsonl`.
    #[serde(default = "default_true")]
    pub jsonl: bool,
}

fn default_true() -> bool {
    true
}

/// The path worth trying first when no explicit config is given.
pub const DEFAULT_CONFIG_PATH: &str = "config/metamind.toml";

impl Config {
    /// Read and validate a configuration file.
    ///
    /// Relative paths inside the file resolve against the file's *grandparent*
    /// directory: `config/metamind.toml` therefore declares `data/metamind.db`
    /// meaning `<repository root>/data/metamind.db`.
    pub fn load(path: &Path) -> Result<Self, MmError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| MmError::Config(format!("cannot read config {}: {e}", path.display())))?;
        let root = resolve_root(path);
        let mut cfg: Config = toml::from_str(&raw)
            .map_err(|e| MmError::Config(format!("invalid config {}: {e}", path.display())))?;
        cfg.root = root.clone();
        cfg.store.data_dir = absolutize(&root, &cfg.store.data_dir);
        cfg.store.sqlite_file = absolutize(&root, &cfg.store.sqlite_file);
        cfg.store.graph_dir = absolutize(&root, &cfg.store.graph_dir);
        cfg.log.dir = absolutize(&root, &cfg.log.dir);
        cfg.validate()?;
        Ok(cfg)
    }

    /// Load `MM_CONFIG`, else `config/metamind.toml` from the working directory.
    pub fn load_default() -> Result<Self, MmError> {
        if let Ok(explicit) = std::env::var("MM_CONFIG") {
            return Config::load(Path::new(&explicit));
        }
        Config::load(Path::new(DEFAULT_CONFIG_PATH))
    }

    /// A configuration whose every path lives under `data_dir`.
    ///
    /// Used by tests and by `mm-cli --data-dir`; it never touches the on-disk
    /// config, so a test can never write into the real store.
    pub fn for_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        let base = data_dir.into();
        Config {
            kernel: KernelConfig {
                name: "metamind".to_string(),
                codename: "mm".to_string(),
            },
            store: StoreConfig {
                sqlite_file: base.join("metamind.db"),
                graph_dir: base.join("graph"),
                data_dir: base.clone(),
            },
            log: LogConfig {
                level: "info".to_string(),
                dir: base.join("logs"),
                console: false,
                jsonl: true,
            },
            // The ontology lives with the source, not with the data, so it stays
            // resolvable from a temporary data directory.
            root: Config::repo_root(),
        }
    }

    /// The repository root this build was compiled in.
    ///
    /// Used only as the ontology root for a data-directory-only configuration
    /// (tests, `--data-dir`); a loaded config always uses its own root.
    pub fn repo_root() -> PathBuf {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        crate_dir
            .join("..")
            .join("..")
            .canonicalize()
            .unwrap_or_else(|_| crate_dir.to_path_buf())
    }

    /// The ontology directory.
    pub fn ontology_dir(&self) -> PathBuf {
        self.root.join("ontology")
    }

    /// `ontology/mm.ttl`.
    pub fn ontology_file(&self) -> PathBuf {
        self.ontology_dir().join("mm.ttl")
    }

    /// `ontology/code.ttl`.
    pub fn ontology_code_file(&self) -> PathBuf {
        self.ontology_dir().join("code.ttl")
    }

    /// `ontology/shapes/mm-shapes.ttl`.
    pub fn shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("mm-shapes.ttl")
    }

    /// `ontology/shapes/mmc-shapes.ttl` — the `/code` graph's shape set.
    pub fn mmc_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("mmc-shapes.ttl")
    }

    /// `ontology/shapes/being.shacl.ttl` — the `/being` graph's shape set.
    pub fn being_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("being.shacl.ttl")
    }

    /// `ontology/shapes/memory.ttl` — the `/memory` graph's shape set.
    ///
    /// The phase-5 plan names this file `memory.ttl` where the other graph shape
    /// sets carry a `.shacl.ttl` suffix; the plan's spelling is kept so the gate's
    /// `graph validate --graph memory` reads the file the plan says it reads.
    pub fn memory_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("memory.ttl")
    }

    /// `ontology/shapes/epistemic_shapes.ttl` — the `/epistemic` graph's shape set.
    ///
    /// `/world` is governed by the same shapes: it holds a subset of the same
    /// records (only `OBSERVED`/`VERIFIED` claims), so a second shape set would be
    /// the same constraints kept in two places.
    pub fn epistemic_shapes_file(&self) -> PathBuf {
        self.ontology_dir()
            .join("shapes")
            .join("epistemic_shapes.ttl")
    }

    /// `ontology/shapes/library.shacl.ttl` — the `/library` graph's shape set.
    ///
    /// This is the one shape set the phase-7 plan names with a `.shacl.ttl`
    /// suffix, and it is kept: the gate's `graph validate --graph library` reads
    /// the file the plan says it reads.
    pub fn library_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("library.shacl.ttl")
    }

    /// `ontology/shapes/decision.ttl` — the `/decision` graph's shape set.
    ///
    /// Phase 9's graph holds decisions, comparisons, risk profiles, uncertainty
    /// measurements and firewall runs. It gets its own set rather than sharing
    /// `mm.ttl`'s because `mm:Decision` is also an epistemic class: a shape that
    /// required `mm:questionKind` on every `mm:Decision` would fire on the
    /// claim-level decisions Phase 6 writes into `/epistemic`.
    pub fn decision_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("decision.ttl")
    }

    /// `ontology/shapes/self_model.ttl` — the `/self` graph's shape set.
    ///
    /// Phase 12's graph holds the numeric self-model reports and their divergence
    /// dimensions, the debt findings, the GC actions, the hot-loaded module versions
    /// and the design revisions. It gets its own set for the same reason `/decision`,
    /// `/tools` and `/selfeng` do: `mm:Divergence` and `mm:GcAction` are Phase 12's own
    /// classes, and the plan's §4.3 places their shapes in this file.
    pub fn self_model_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("self_model.ttl")
    }

    /// `ontology/self_model.ttl` — the autonomy/self-model T-Box.
    ///
    /// It shares the `mm:` namespace with `mm.ttl` and only adds classes and properties
    /// (`mm:SelfModel`, `mm:SelfModelReport`, `mm:Divergence`, `mm:DebtFinding`,
    /// `mm:GcAction`, `mm:ModuleLoad`, `mm:DesignRevision` and their predicates), so
    /// loading it cannot change the meaning of a graph that was already validated.
    ///
    /// The phase plan's §3 writes these classes into `mm.ttl` and only names
    /// `shapes/self_model.ttl`; since Phase 7 this repository has kept one ontology file
    /// *per phase graph* (`selfeng.ttl`, `tools.ttl`), and `Config::ontology_files`
    /// lists them explicitly. The vocabulary therefore lives in its own file and the
    /// deviation is recorded here rather than taken silently.
    pub fn ontology_self_model_file(&self) -> PathBuf {
        self.ontology_dir().join("self_model.ttl")
    }

    /// `ontology/shapes/tools.ttl` — the `/tools` graph's shape set.
    ///
    /// Phase 10's graph holds tool contracts, action records and action observations.
    /// It gets its own set for the same reason `/decision` does: `mm:Observation` is
    /// already an epistemic claim class, so the observation class there is
    /// `mm:ActionObservation` and its shapes are not the kernel ones.
    pub fn tools_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("tools.ttl")
    }

    /// `ontology/shapes/selfeng.ttl` — the `/selfeng` graph's shape set.
    ///
    /// Phase 11's graph holds the calibrated predictions, the calibration runs, the
    /// meta-analyses and lessons, the change sets, the promotion decisions and the
    /// evolution lineage. It gets its own set for the same reason `/decision` and
    /// `/tools` do: `mm:Prediction` is already the epistemic claim class of a Phase 6
    /// prediction, so the ledger's class here is `mm:CalibratedPrediction` and its
    /// constraints are not the kernel ones.
    pub fn selfeng_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("selfeng.ttl")
    }

    /// The shapes file that governs a named graph.
    ///
    /// The `code` graph is described by `mmc:`, `being`, `memory`, `epistemic`,
    /// `library`, `decision` and `tools` by their own sets, and every other graph by the
    /// kernel `mm:` shapes; validating one graph against another's shapes would
    /// report nonsense.
    pub fn shapes_file_for(&self, graph: &str) -> PathBuf {
        let bare = graph.strip_prefix(crate::iri::GRAPH).unwrap_or(graph);
        match bare {
            "code" => self.mmc_shapes_file(),
            "being" => self.being_shapes_file(),
            "memory" => self.memory_shapes_file(),
            "epistemic" | "world" => self.epistemic_shapes_file(),
            "library" => self.library_shapes_file(),
            "decision" => self.decision_shapes_file(),
            "tools" => self.tools_shapes_file(),
            "selfeng" => self.selfeng_shapes_file(),
            "self" => self.self_model_shapes_file(),
            _ => self.shapes_file(),
        }
    }

    /// `ontology/llm.ttl` — the `mm:` vocabulary of the LLM call ledger.
    ///
    /// It shares the `mm:` namespace with `mm.ttl` and only adds classes and
    /// properties, so loading it cannot change an existing graph's meaning.
    pub fn ontology_llm_file(&self) -> PathBuf {
        self.ontology_dir().join("llm.ttl")
    }

    /// `ontology/library.ttl` — the cognitive library's T-Box.
    ///
    /// It shares the `mm:` namespace with `mm.ttl` and only adds classes and
    /// properties, so loading it cannot change an existing graph's meaning.
    pub fn ontology_library_file(&self) -> PathBuf {
        self.ontology_dir().join("library.ttl")
    }

    /// `ontology/metacog.ttl` — the metacognitive controller's T-Box.
    ///
    /// It shares the `mm:` namespace with `mm.ttl` and only adds classes and
    /// properties, so loading it cannot change an existing graph's meaning.
    pub fn ontology_metacog_file(&self) -> PathBuf {
        self.ontology_dir().join("metacog.ttl")
    }

    /// `ontology/selfeng.ttl` — the self-engineering T-Box.
    ///
    /// It shares the `mm:` namespace with `mm.ttl` and only adds classes and
    /// properties (`mm:CalibratedPrediction`, `mm:PredictionOutcome`,
    /// `mm:CalibrationRun`, `mm:MetaAnalysis`, `mm:Lesson`, `mm:ChangeSet`,
    /// `mm:EvolutionEvent` and their predicates), so loading it cannot change the
    /// meaning of a graph that was already validated.
    pub fn ontology_selfeng_file(&self) -> PathBuf {
        self.ontology_dir().join("selfeng.ttl")
    }

    /// `ontology/tools.ttl` — the tool execution T-Box.
    ///
    /// It shares the `mm:` namespace with `mm.ttl` and only adds classes and
    /// properties (`mm:Tool`, `mm:Action`, `mm:ActionObservation` and the predicates
    /// `mm-tools::rdf` emits), so loading it cannot change the meaning of a graph that
    /// was already validated against the kernel shapes.
    pub fn ontology_tools_file(&self) -> PathBuf {
        self.ontology_dir().join("tools.ttl")
    }

    /// `ontology/shapes/episode.ttl` — the shapes for cognitive episodes and the
    /// programs compiled from them.
    ///
    /// Episodes are emitted into `/epistemic`, so these shapes are validated
    /// alongside that graph's own set rather than replacing it; see
    /// [`Config::extra_shapes_files_for`].
    pub fn episode_shapes_file(&self) -> PathBuf {
        self.ontology_dir().join("shapes").join("episode.ttl")
    }

    /// The additional shapes that also govern a named graph.
    ///
    /// `/epistemic` holds the metacognitive controller's episodes as well as the
    /// claims, and the two sets were written separately. Validating the graph
    /// against only one of them would silently skip half of it, so both are read.
    pub fn extra_shapes_files_for(&self, graph: &str) -> Vec<PathBuf> {
        let bare = graph.strip_prefix(crate::iri::GRAPH).unwrap_or(graph);
        match bare {
            "epistemic" | "world" => vec![self.episode_shapes_file()],
            _ => Vec::new(),
        }
    }

    /// Every ontology file the kernel loads, in a deterministic order.
    pub fn ontology_files(&self) -> Vec<PathBuf> {
        vec![
            self.ontology_file(),
            self.ontology_code_file(),
            self.ontology_llm_file(),
            self.ontology_library_file(),
            self.ontology_metacog_file(),
            self.ontology_tools_file(),
            self.ontology_selfeng_file(),
            self.ontology_self_model_file(),
        ]
    }

    /// The `modules/` directory the registry lives in.
    pub fn modules_dir(&self) -> PathBuf {
        self.root.join("modules")
    }

    /// `modules/registry.json` — generated by `codex scan`, committed.
    pub fn registry_path(&self) -> PathBuf {
        self.modules_dir().join("registry.json")
    }

    /// `codex.lock` at the repository root — generated, committed.
    pub fn codex_lock_path(&self) -> PathBuf {
        self.root.join("codex.lock")
    }

    /// Apply the `MM_DATA_DIR` override, if set.
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(dir) = std::env::var("MM_DATA_DIR") {
            let dir = PathBuf::from(dir);
            self.store.sqlite_file = dir.join("metamind.db");
            self.store.graph_dir = dir.join("graph");
            self.log.dir = dir.join("logs");
            self.store.data_dir = dir;
        }
        self
    }

    /// The JSONL record stream.
    pub fn jsonl_path(&self) -> PathBuf {
        self.log.dir.join("mm.jsonl")
    }

    /// The audit JSONL stream (mirrors the SQLite `audit_log`).
    pub fn audit_path(&self) -> PathBuf {
        self.log.dir.join("audit.jsonl")
    }

    /// The identifier watermark file.
    pub fn ulid_watermark_path(&self) -> PathBuf {
        self.store.data_dir.join("ulid.watermark")
    }

    fn validate(&self) -> Result<(), MmError> {
        if self.kernel.codename.is_empty() {
            return Err(MmError::Config("kernel.codename must not be empty".into()));
        }
        if self.store.sqlite_file.as_os_str().is_empty() {
            return Err(MmError::Config("store.sqlite_file must be set".into()));
        }
        if self.store.graph_dir.as_os_str().is_empty() {
            return Err(MmError::Config("store.graph_dir must be set".into()));
        }
        match self.log.level.as_str() {
            "error" | "warn" | "info" | "debug" | "trace" => Ok(()),
            other => Err(MmError::Config(format!(
                "log.level must be one of error|warn|info|debug|trace, got {other:?}"
            ))),
        }
    }
}

/// `config/metamind.toml` -> the directory holding `config/`.
fn resolve_root(config_path: &Path) -> PathBuf {
    let parent = config_path.parent().unwrap_or(Path::new("."));
    if parent.file_name().is_some_and(|n| n == "config") {
        parent.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        parent.to_path_buf()
    }
}

fn absolutize(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_config_resolves_paths_against_the_repo_root() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/metamind.toml");
        let cfg = Config::load(&path).expect("config/metamind.toml must load");
        assert_eq!(cfg.kernel.codename, "mm");
        assert!(cfg.store.sqlite_file.is_absolute());
        assert!(
            cfg.store.sqlite_file.ends_with("data/metamind.db"),
            "got {}",
            cfg.store.sqlite_file.display()
        );
        assert!(cfg
            .store
            .sqlite_file
            .starts_with(path.parent().unwrap().parent().unwrap()));
        assert_eq!(cfg.jsonl_path().file_name().unwrap(), "mm.jsonl");
    }

    #[test]
    fn invalid_level_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.toml");
        std::fs::write(
            &file,
            "[kernel]\nname=\"m\"\ncodename=\"mm\"\n[store]\ndata_dir=\"d\"\nsqlite_file=\"s\"\ngraph_dir=\"g\"\n[log]\nlevel=\"verbose\"\ndir=\"l\"\n",
        )
        .unwrap();
        let err = Config::load(&file).unwrap_err();
        assert!(matches!(err, MmError::Config(_)), "got {err:?}");
    }

    #[test]
    fn missing_file_is_a_config_error_not_a_panic() {
        let err = Config::load(Path::new("/nonexistent/metamind.toml")).unwrap_err();
        assert!(matches!(err, MmError::Config(_)));
    }

    #[test]
    fn for_data_dir_keeps_state_inside_and_finds_the_ontology() {
        let cfg = Config::for_data_dir("/tmp/mm-test");
        assert_eq!(
            cfg.store.sqlite_file,
            PathBuf::from("/tmp/mm-test/metamind.db")
        );
        assert_eq!(cfg.store.graph_dir, PathBuf::from("/tmp/mm-test/graph"));
        assert!(cfg.log.dir.starts_with("/tmp/mm-test"));
        assert!(cfg.ontology_file().ends_with("ontology/mm.ttl"));
        assert!(cfg.shapes_file().ends_with("ontology/shapes/mm-shapes.ttl"));
        assert!(cfg
            .being_shapes_file()
            .ends_with("ontology/shapes/being.shacl.ttl"));
        assert!(cfg
            .memory_shapes_file()
            .ends_with("ontology/shapes/memory.ttl"));
        assert!(cfg
            .epistemic_shapes_file()
            .ends_with("ontology/shapes/epistemic_shapes.ttl"));
        assert!(cfg
            .library_shapes_file()
            .ends_with("ontology/shapes/library.shacl.ttl"));
        assert!(cfg
            .ontology_library_file()
            .ends_with("ontology/library.ttl"));
        assert!(cfg
            .ontology_metacog_file()
            .ends_with("ontology/metacog.ttl"));
        assert!(cfg
            .episode_shapes_file()
            .ends_with("ontology/shapes/episode.ttl"));
        assert!(cfg
            .decision_shapes_file()
            .ends_with("ontology/shapes/decision.ttl"));
        assert!(cfg
            .tools_shapes_file()
            .ends_with("ontology/shapes/tools.ttl"));
        assert!(cfg.ontology_tools_file().ends_with("ontology/tools.ttl"));
        assert!(cfg
            .selfeng_shapes_file()
            .ends_with("ontology/shapes/selfeng.ttl"));
        assert!(cfg
            .ontology_selfeng_file()
            .ends_with("ontology/selfeng.ttl"));
    }

    #[test]
    fn the_epistemic_graph_is_governed_by_two_shape_sets() {
        let cfg = Config::for_data_dir("/tmp/mm-test");
        let extra = cfg.extra_shapes_files_for("epistemic");
        assert_eq!(extra.len(), 1);
        assert!(extra[0].ends_with("ontology/shapes/episode.ttl"));
        // `/world` holds the same records, and every other graph has one set.
        assert_eq!(cfg.extra_shapes_files_for("world").len(), 1);
        assert!(cfg.extra_shapes_files_for("library").is_empty());
        assert!(cfg.extra_shapes_files_for("being").is_empty());
    }

    #[test]
    fn each_named_graph_gets_its_own_shapes() {
        let cfg = Config::for_data_dir("/tmp/mm-test");
        assert!(cfg.shapes_file_for("code").ends_with("mmc-shapes.ttl"));
        assert!(cfg.shapes_file_for("being").ends_with("being.shacl.ttl"));
        assert!(cfg.shapes_file_for("memory").ends_with("memory.ttl"));
        assert!(cfg
            .shapes_file_for("epistemic")
            .ends_with("epistemic_shapes.ttl"));
        // `/world` holds a subset of the same records, so it shares the shapes.
        assert!(cfg
            .shapes_file_for("world")
            .ends_with("epistemic_shapes.ttl"));
        // A full graph IRI is accepted as well as a bare name.
        assert!(cfg
            .shapes_file_for("https://metamind.dev/graph/being")
            .ends_with("being.shacl.ttl"));
        assert!(cfg
            .shapes_file_for("https://metamind.dev/graph/world")
            .ends_with("epistemic_shapes.ttl"));
        assert!(cfg
            .shapes_file_for("library")
            .ends_with("library.shacl.ttl"));
        // Phase 9's graph has its own set, in both spellings.
        assert!(cfg.shapes_file_for("decision").ends_with("decision.ttl"));
        assert!(cfg
            .shapes_file_for("https://metamind.dev/graph/decision")
            .ends_with("decision.ttl"));
        // Phase 10's graph, in both spellings.
        assert!(cfg.shapes_file_for("tools").ends_with("tools.ttl"));
        assert!(cfg
            .shapes_file_for("https://metamind.dev/graph/tools")
            .ends_with("tools.ttl"));
        // Phase 11's graph, in both spellings. `/provenance` keeps the kernel shapes,
        // because Phase 11 does not write there.
        assert!(cfg.shapes_file_for("selfeng").ends_with("selfeng.ttl"));
        assert!(cfg
            .shapes_file_for("https://metamind.dev/graph/selfeng")
            .ends_with("selfeng.ttl"));
        assert!(cfg.shapes_file_for("provenance").ends_with("mm-shapes.ttl"));
    }
}
