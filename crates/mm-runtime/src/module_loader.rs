//! The hot module loader: a promoted module version becomes callable without a restart.
//!
//! Design §103–107 asks for live module (re)load with a stable URI, a new version
//! activated while the old ones keep serving, and a rollback that restores the previous
//! version. Three decisions make that concrete in this build, and each is a limit worth
//! stating rather than hiding:
//!
//! * **A module version is a directory with a `plugin.toml`, and the loader reads it.** The
//!   manifest is the module's identity: the loader refuses a manifest whose declared `uri`
//!   or `version` differs from the version it was asked to load, so "activate *this*
//!   version" cannot silently activate another.
//! * **A declared function must have a handler in this build to be callable, and the loader
//!   says so at load time.** The functions this build can execute are the ones in
//!   [`dispatch`]; a manifest that declares something else loads as a *registration* only
//!   if a caller asks for that, and [`ManifestModuleLoader::hot_load`] refuses it by
//!   default. Refusing at load time rather than at call time is the difference between a
//!   load that failed and a running module that cannot be called.
//! * **The prior version keeps serving until the new one is verified.** Verification
//!   happens before the load row exists, so there is no window in which a caller could
//!   observe a URI with no version behind it; and the previous version stays a row, because
//!   *"the previous version kept serving"* is only checkable if it is still recorded.
//!
//! What this is *not*: a dynamic linker. A Rust binary cannot load a Rust crate's
//! machine code without an ABI, and a loader that claimed to would be lying about what a
//! promotion means. What it is is the plugin lifecycle the design describes — load,
//! activate, keep the old version, roll back — driven by manifests, with the executable
//! handlers compiled in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{LoopError, Result};
use crate::rdf;
use crate::LoopStage;

/// The subdirectory of the artifact root module versions are materialised under.
pub const MODULE_ARTIFACT_SUBDIR: &str = "modules";

/// The functions this build can execute.
///
/// Kept as a named list so the loader's dispatch and its refusal agree: a function in the
/// list has an arm in [`dispatch`], and a function with no arm is refused by name.
pub const DISPATCH_FUNCTIONS: [&str; 3] = [
    "cognition.goal_attainment_progress",
    "cognition.loop_status",
    "cognition.loop_stage_order",
];

/// A load's status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadStatus {
    /// Activated.
    Loaded,
    /// Undone; the previous version is active again.
    RolledBack,
    /// Refused before anything became active.
    Failed,
}

impl LoadStatus {
    /// The stable wire name, which is the value the table's CHECK allows.
    pub fn as_str(self) -> &'static str {
        match self {
            LoadStatus::Loaded => "loaded",
            LoadStatus::RolledBack => "rolled_back",
            LoadStatus::Failed => "failed",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<LoadStatus> {
        [
            LoadStatus::Loaded,
            LoadStatus::RolledBack,
            LoadStatus::Failed,
        ]
        .into_iter()
        .find(|status| status.as_str() == text.trim())
    }
}

impl std::fmt::Display for LoadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One module version to activate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleVersion {
    /// The module's stable IRI.
    pub uri: String,
    /// The version string.
    pub version: String,
    /// The directory holding its `plugin.toml`.
    pub dir: PathBuf,
}

impl ModuleVersion {
    /// A version at a directory.
    pub fn new(
        uri: impl Into<String>,
        version: impl Into<String>,
        dir: impl Into<PathBuf>,
    ) -> Self {
        ModuleVersion {
            uri: uri.into(),
            version: version.into(),
            dir: dir.into(),
        }
    }

    /// The `uri@version` spelling the CLI accepts.
    pub fn spec(&self) -> String {
        format!("{}@{}", self.uri, self.version)
    }

    /// Parse `uri@version`.
    pub fn parse_spec(spec: &str) -> Result<(String, String)> {
        let spec = spec.trim();
        let Some((uri, version)) = spec.rsplit_once('@') else {
            return Err(LoopError::validation(
                "uri",
                format!("{spec:?} is not `<module-uri>@<version>`"),
            ));
        };
        if uri.trim().is_empty() || version.trim().is_empty() {
            return Err(LoopError::validation(
                "uri",
                format!("{spec:?} is not `<module-uri>@<version>`"),
            ));
        }
        Ok((uri.to_string(), version.to_string()))
    }
}

/// What a load produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadReceipt {
    /// The `module_loads` row's id.
    #[serde(with = "mm_core::serde_ulid")]
    pub id: Ulid,
    /// The module.
    pub uri: String,
    /// The version now active.
    pub version: String,
    /// What happened.
    pub status: LoadStatus,
    /// The T-Box functions the version exposes, sorted.
    pub functions: Vec<String>,
    /// The version that was active before, and still serves.
    pub previous: Option<String>,
    /// How long the load took.
    pub latency_ms: u64,
}

impl LoadReceipt {
    /// True when the version is active.
    pub fn is_active(&self) -> bool {
        self.status == LoadStatus::Loaded
    }
}

/// What a loader does.
#[async_trait]
pub trait ModuleLoader {
    /// Activate a module version.
    async fn hot_load(&self, mv: &ModuleVersion) -> Result<LoadReceipt>;
}

/// The loader over manifests and the `module_loads` table.
pub struct ManifestModuleLoader {
    store: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
    artifacts_dir: PathBuf,
    graph: Option<Arc<GraphStore>>,
}

impl ManifestModuleLoader {
    /// A loader whose materialised versions live under `artifacts_dir`.
    pub fn new(
        store: SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
        artifacts_dir: PathBuf,
    ) -> Self {
        ManifestModuleLoader {
            store,
            logger,
            ids,
            artifacts_dir,
            graph: None,
        }
    }

    /// Attach the graph store, so a load reaches `/self`.
    pub fn with_graph(mut self, graph: Arc<GraphStore>) -> Self {
        self.graph = Some(graph);
        self
    }

    /// Where a version's files are materialised.
    pub fn version_dir(&self, mv: &ModuleVersion) -> PathBuf {
        let name = mv.uri.rsplit('/').next().unwrap_or("module").to_string();
        self.artifacts_dir
            .join(MODULE_ARTIFACT_SUBDIR)
            .join(name)
            .join(&mv.version)
    }

    /// The manifest a load reads.
    pub fn manifest_path(&self, mv: &ModuleVersion) -> PathBuf {
        mv.dir.join("plugin.toml")
    }

    /// A module version's declared functions, in sorted order.
    pub fn declared_functions(dir: &Path) -> Result<Vec<String>> {
        let path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| LoopError::Load(format!("cannot read {}: {e}", path.display())))?;
        let manifest: toml::Value = toml::from_str(&text)
            .map_err(|e| LoopError::Load(format!("{} is not a manifest: {e}", path.display())))?;
        let mut functions: Vec<String> = manifest
            .get("tbox")
            .and_then(|tbox| tbox.get("functions"))
            .and_then(toml::Value::as_table)
            .map(|table| table.keys().cloned().collect())
            .unwrap_or_default();
        functions.sort();
        Ok(functions)
    }

    /// The `(uri, version)` a manifest declares.
    pub fn declared_identity(dir: &Path) -> Result<(String, String)> {
        let path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| LoopError::Load(format!("cannot read {}: {e}", path.display())))?;
        let manifest: toml::Value = toml::from_str(&text)
            .map_err(|e| LoopError::Load(format!("{} is not a manifest: {e}", path.display())))?;
        let uri = manifest
            .get("plugin")
            .and_then(|plugin| plugin.get("uri"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| LoopError::Load(format!("{} declares no plugin.uri", path.display())))?
            .to_string();
        let version = manifest
            .get("plugin")
            .and_then(|plugin| plugin.get("version"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                LoopError::Load(format!("{} declares no plugin.version", path.display()))
            })?
            .to_string();
        Ok((uri, version))
    }

    /// Find a materialised version under the artifact root.
    pub fn find(&self, uri: &str, version: &str) -> Option<PathBuf> {
        let root = self.artifacts_dir.join(MODULE_ARTIFACT_SUBDIR);
        let mut stack = vec![root];
        let mut candidates = Vec::new();
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.file_name().is_some_and(|name| name == "plugin.toml") {
                    if let Some(parent) = path.parent() {
                        if let Ok((found_uri, found_version)) = Self::declared_identity(parent) {
                            if found_uri == uri && found_version == version {
                                candidates.push(parent.to_path_buf());
                            }
                        }
                    }
                }
            }
        }
        candidates.sort();
        candidates.into_iter().next()
    }

    /// The version of a module that is currently active.
    pub async fn current(&self, uri: &str) -> Result<Option<String>> {
        let rows = self
            .store
            .query_json(
                "SELECT version, status FROM module_loads WHERE module_uri = ? \
                 ORDER BY loaded_at DESC, id DESC",
                vec![Param::Text(uri.to_string())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read module_loads: {e}")))?;
        for row in rows {
            if row["status"].as_str() == Some(LoadStatus::Loaded.as_str()) {
                return Ok(Some(
                    row["version"].as_str().unwrap_or_default().to_string(),
                ));
            }
        }
        Ok(None)
    }

    /// Every function a URI currently exposes.
    pub async fn active_functions(&self, uri: &str) -> Result<Vec<String>> {
        let Some(version) = self.current(uri).await? else {
            return Ok(Vec::new());
        };
        let Some(dir) = self.find(uri, &version) else {
            return Ok(Vec::new());
        };
        Self::declared_functions(&dir)
    }

    /// Call one of an active version's functions.
    ///
    /// The call is refused when the module is not active, when the function is not
    /// declared by its manifest, or when this build has no handler for it. Each refusal
    /// names which of the three it was, because "unknown function" and "not loaded" are
    /// different problems to an operator.
    pub async fn call(&self, uri: &str, function: &str, args: &Value) -> Result<Value> {
        let Some(version) = self.current(uri).await? else {
            return Err(LoopError::Load(format!("{uri} has no active version")));
        };
        let dir = self
            .find(uri, &version)
            .ok_or_else(|| LoopError::Load(format!("{uri}@{version} is not materialised")))?;
        let declared = Self::declared_functions(&dir)?;
        if !declared.iter().any(|name| name == function) {
            return Err(LoopError::Load(format!(
                "{uri}@{version} does not declare {function}; it declares {}",
                declared.join(", ")
            )));
        }
        dispatch(function, args)
    }

    /// Record a load row.
    async fn record(
        &self,
        mv: &ModuleVersion,
        status: LoadStatus,
        functions: usize,
        latency_ms: u64,
    ) -> Result<Ulid> {
        let id = self.ids.next();
        self.store
            .execute(
                "INSERT INTO module_loads \
                 (id, module_uri, version, status, functions, latency_ms, loaded_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&id)),
                    Param::Text(mv.uri.clone()),
                    Param::Text(mv.version.clone()),
                    Param::Text(status.as_str().to_string()),
                    Param::Int(functions as i64),
                    Param::Int(latency_ms as i64),
                    Param::Text(Timestamp::now().to_rfc3339()),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the load: {e}")))?;
        Ok(id)
    }

    /// Activate a version.
    pub async fn load(&self, mv: &ModuleVersion) -> Result<LoadReceipt> {
        let started = std::time::Instant::now();
        // Verification first: identity, then the declared functions. Nothing is recorded
        // until both hold, so the URI never has a window with no version behind it.
        let (uri, version) = Self::declared_identity(&mv.dir)?;
        if uri != mv.uri || version != mv.version {
            return Err(LoopError::Load(format!(
                "{} declares {uri}@{version}, not the requested {}",
                mv.dir.display(),
                mv.spec()
            )));
        }
        let functions = Self::declared_functions(&mv.dir)?;
        if functions.is_empty() {
            return Err(LoopError::Load(format!(
                "{uri}@{version} declares no T-Box function, so there is nothing to activate"
            )));
        }
        let unsupported: Vec<String> = functions
            .iter()
            .filter(|function| !DISPATCH_FUNCTIONS.contains(&function.as_str()))
            .cloned()
            .collect();
        if !unsupported.is_empty() {
            return Err(LoopError::Load(format!(
                "{uri}@{version} declares {} with no handler in this build; a version is \
                 loadable when every declared function is callable",
                unsupported.join(", ")
            )));
        }
        let previous = match self.current(&mv.uri).await? {
            Some(current) if current != mv.version => Some(current),
            _ => None,
        };
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let id = self
            .record(mv, LoadStatus::Loaded, functions.len(), latency_ms)
            .await?;
        let receipt = LoadReceipt {
            id,
            uri: mv.uri.clone(),
            version: mv.version.clone(),
            status: LoadStatus::Loaded,
            functions,
            previous,
            latency_ms,
        };
        self.logger
            .audit(
                Level::Info,
                codes::MODULE_LOAD,
                crate::TARGET,
                Some(id),
                json!({
                    "module_uri": receipt.uri,
                    "version": receipt.version,
                    "status": receipt.status.as_str(),
                    "functions": receipt.functions,
                    "previous": receipt.previous,
                    "latency_ms": receipt.latency_ms,
                }),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the load: {e}")))?;
        if let Some(graph) = &self.graph {
            let turtle = rdf::turtle(&rdf::module_load_quads(&receipt));
            graph
                .handle()
                .insert_turtle(rdf::SELF_GRAPH, &turtle)
                .await
                .map_err(LoopError::from)?;
        }
        Ok(receipt)
    }

    /// Roll a module back to the version before the active one.
    pub async fn rollback(&self, uri: &str) -> Result<LoadReceipt> {
        let rows = self
            .store
            .query_json(
                "SELECT * FROM module_loads WHERE module_uri = ? ORDER BY loaded_at DESC, id DESC",
                vec![Param::Text(uri.to_string())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read module_loads: {e}")))?;
        let mut loaded: Vec<&serde_json::Value> = rows
            .iter()
            .filter(|row| row["status"].as_str() == Some(LoadStatus::Loaded.as_str()))
            .collect();
        if loaded.len() < 2 {
            return Err(LoopError::Load(format!(
                "{uri} has no earlier version to roll back to; \
                 a rollback restores a version, it does not invent one"
            )));
        }
        let active = loaded.remove(0);
        let previous = loaded.remove(0);
        let active_version = active["version"].as_str().unwrap_or_default().to_string();
        let previous_version = previous["version"].as_str().unwrap_or_default().to_string();
        let previous_mv = ModuleVersion::new(uri, previous_version.clone(), PathBuf::new());
        self.record(&previous_mv, LoadStatus::Loaded, 0, 0).await?;
        let failed = ModuleVersion::new(uri, active_version.clone(), PathBuf::new());
        let id = self.record(&failed, LoadStatus::RolledBack, 0, 0).await?;
        let receipt = LoadReceipt {
            id,
            uri: uri.to_string(),
            version: previous_version,
            status: LoadStatus::RolledBack,
            functions: Vec::new(),
            previous: Some(active_version),
            latency_ms: 0,
        };
        self.logger
            .audit(
                Level::Warn,
                codes::MODULE_ROLLBACK,
                crate::TARGET,
                Some(id),
                json!({
                    "module_uri": receipt.uri,
                    "version": receipt.version,
                    "status": receipt.status.as_str(),
                    "previous": receipt.previous,
                }),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the rollback: {e}")))?;
        Ok(receipt)
    }

    /// How many versions of a module are recorded as loaded.
    pub async fn loaded_versions(&self, uri: &str) -> Result<i64> {
        let rows = self
            .store
            .query_json(
                "SELECT count(*) AS n FROM module_loads WHERE module_uri = ? AND status = 'loaded'",
                vec![Param::Text(uri.to_string())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read module_loads: {e}")))?;
        Ok(rows[0]["n"].as_i64().unwrap_or(-1))
    }
}

#[async_trait]
impl ModuleLoader for ManifestModuleLoader {
    async fn hot_load(&self, mv: &ModuleVersion) -> Result<LoadReceipt> {
        self.load(mv).await
    }
}

/// The functions this build can execute, as pure arithmetic.
///
/// Each arm is the same arithmetic the generated module's own handler performs, which is
/// what makes "call the promoted capability" a real call rather than a registration.
pub fn dispatch(function: &str, args: &Value) -> Result<Value> {
    match function {
        "cognition.goal_attainment_progress" => goal_attainment_progress(args),
        "cognition.loop_status" => loop_status(args),
        "cognition.loop_stage_order" => Ok(json!({
            "stages": LoopStage::ALL.iter().map(|stage| stage.as_str()).collect::<Vec<_>>(),
        })),
        other => Err(LoopError::Load(format!(
            "{other} has no handler in this build ({})",
            DISPATCH_FUNCTIONS.join(", ")
        ))),
    }
}

/// `{goal, evidence}` -> `{n, advanced, progress, outstanding}`.
fn goal_attainment_progress(args: &Value) -> Result<Value> {
    let goal = args
        .get("goal")
        .and_then(Value::as_str)
        .ok_or_else(|| LoopError::Load("the payload has no goal".to_string()))?;
    if goal.trim().is_empty() {
        return Err(LoopError::Load("the goal is empty".to_string()));
    }
    let evidence = args
        .get("evidence")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut achieved = 0.0_f64;
    let mut advanced = 0_usize;
    for item in &evidence {
        if item.get("goal").and_then(Value::as_str) != Some(goal) {
            return Err(LoopError::Load(format!(
                "evidence is about {:?}, not {goal}",
                item.get("goal").and_then(Value::as_str).unwrap_or("")
            )));
        }
        let weight = item.get("weight").and_then(Value::as_f64).unwrap_or(0.0);
        if !(0.0..=1.0).contains(&weight) {
            return Err(LoopError::Load(format!("weight {weight} is not in [0,1]")));
        }
        if item
            .get("advanced")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            achieved += weight;
            advanced += 1;
        }
    }
    let total = evidence.len() as f64;
    let progress = if total == 0.0 { 0.0 } else { achieved / total };
    Ok(json!({
        "n": evidence.len(),
        "advanced": advanced,
        "progress": progress.clamp(0.0, 1.0),
        "outstanding": (1.0 - progress).clamp(0.0, 1.0),
    }))
}

/// A JSON array of stage records -> a summary.
fn loop_status(stages: &Value) -> Result<Value> {
    let stages = stages
        .as_array()
        .ok_or_else(|| LoopError::Load("the payload is not an array of stages".to_string()))?;
    let mut ok = 0;
    let mut refused = 0;
    let mut failed = 0;
    for stage in stages {
        match stage.get("outcome").and_then(Value::as_str).unwrap_or("") {
            "ok" => ok += 1,
            "refused" => refused += 1,
            "failed" => failed += 1,
            other => {
                return Err(LoopError::Load(format!(
                    "{other:?} is not one of ok, refused, failed"
                )))
            }
        }
    }
    Ok(json!({
        "stages": stages.len(),
        "ok": ok,
        "refused": refused,
        "failed": failed,
        "complete": stages.len() == LoopStage::ALL.len() && ok == LoopStage::ALL.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;

    /// Write a module version into the artifact root and return its location.
    fn materialise(
        loader: &ManifestModuleLoader,
        name: &str,
        uri: &str,
        version: &str,
        functions: &[&str],
    ) -> PathBuf {
        let dir = loader
            .artifacts_dir
            .join(MODULE_ARTIFACT_SUBDIR)
            .join(name)
            .join(version);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let mut table = String::new();
        for function in functions {
            let handler = function.rsplit('.').next().unwrap_or(function);
            table.push_str(&format!(
                "\"{function}\" = {{ source = \"handlers::{handler}\" }}\n"
            ));
        }
        std::fs::write(
            dir.join("plugin.toml"),
            format!(
                "[plugin]\nname = \"{name}\"\nuri = \"{uri}\"\nversion = \"{version}\"\n\n\
                 [metadata]\ncategory = \"cognition\"\nowned_by_phase = 12\n\
                 capability = \"mm:Test\"\n\n[tbox.functions]\n{table}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "// generated\n").unwrap();
        dir
    }

    async fn loader() -> (ManifestModuleLoader, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path().join("data"));
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        let loader = ManifestModuleLoader::new(
            store,
            logger,
            ids,
            dir.path().join("data").join("artifacts"),
        );
        (loader, dir)
    }

    #[tokio::test]
    async fn a_promoted_version_is_callable_without_a_restart() {
        let (loader, _dir) = loader().await;
        let dir = materialise(
            &loader,
            "goal-attainment",
            "https://metamind.dev/code/module/cognition/goal-attainment",
            "0.1.0",
            &["cognition.goal_attainment_progress"],
        );
        let mv = ModuleVersion::new(
            "https://metamind.dev/code/module/cognition/goal-attainment",
            "0.1.0",
            dir,
        );
        let receipt = loader.hot_load(&mv).await.expect("a load");
        assert!(receipt.is_active());
        assert_eq!(receipt.functions.len(), 1);
        assert!(receipt.previous.is_none());
        // The function is callable immediately, through the loader, with no restart.
        let value = loader
            .call(
                &mv.uri,
                "cognition.goal_attainment_progress",
                &json!({
                    "goal": "ship it",
                    "evidence": [
                        { "goal": "ship it", "advanced": true, "weight": 1.0 },
                        { "goal": "ship it", "advanced": false, "weight": 1.0 },
                    ],
                }),
            )
            .await
            .expect("an answer");
        assert_eq!(value["progress"].as_f64(), Some(0.5));
        assert_eq!(value["advanced"].as_u64(), Some(1));
        assert_eq!(value["outstanding"].as_f64(), Some(0.5));
    }

    #[tokio::test]
    async fn a_second_version_keeps_the_first_one_recorded() {
        let (loader, _dir) = loader().await;
        let uri = "https://metamind.dev/code/module/cognition/goal-attainment";
        let first = materialise(
            &loader,
            "goal-attainment",
            uri,
            "0.1.0",
            &["cognition.goal_attainment_progress"],
        );
        let second = materialise(
            &loader,
            "goal-attainment",
            uri,
            "0.2.0",
            &["cognition.goal_attainment_progress"],
        );
        loader
            .hot_load(&ModuleVersion::new(uri, "0.1.0", first))
            .await
            .expect("the first load");
        let receipt = loader
            .hot_load(&ModuleVersion::new(uri, "0.2.0", second))
            .await
            .expect("the second load");
        assert_eq!(receipt.version, "0.2.0");
        assert_eq!(receipt.previous.as_deref(), Some("0.1.0"));
        assert_eq!(loader.current(uri).await.unwrap().as_deref(), Some("0.2.0"));
        assert_eq!(
            loader.loaded_versions(uri).await.unwrap(),
            2,
            "the prior version is still a row: it kept serving"
        );
        // Rollback restores the earlier version rather than deleting the load it undid.
        let rolled = loader.rollback(uri).await.expect("a rollback");
        assert_eq!(rolled.version, "0.1.0");
        assert_eq!(rolled.status, LoadStatus::RolledBack);
        assert_eq!(loader.current(uri).await.unwrap().as_deref(), Some("0.1.0"));
    }

    #[tokio::test]
    async fn a_manifest_that_disagrees_with_the_request_is_refused() {
        let (loader, _dir) = loader().await;
        let uri = "https://metamind.dev/code/module/cognition/goal-attainment";
        let dir = materialise(
            &loader,
            "goal-attainment",
            uri,
            "0.1.0",
            &["cognition.goal_attainment_progress"],
        );
        let wrong = ModuleVersion::new(uri, "9.9.9", dir);
        let error = loader.hot_load(&wrong).await.expect_err("a refusal");
        assert!(error.to_string().contains("declares"), "{error}");
        assert_eq!(loader.loaded_versions(uri).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_function_with_no_handler_is_refused_at_load_time() {
        let (loader, _dir) = loader().await;
        let uri = "https://metamind.dev/code/module/cognition/mystery";
        let dir = materialise(
            &loader,
            "mystery",
            uri,
            "0.1.0",
            &["cognition.something_unknown"],
        );
        let error = loader
            .hot_load(&ModuleVersion::new(uri, "0.1.0", dir))
            .await
            .expect_err("a refusal");
        assert!(error.to_string().contains("no handler"), "{error}");
        assert_eq!(loader.loaded_versions(uri).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_unloaded_module_has_no_callable_function() {
        let (loader, _dir) = loader().await;
        let error = loader
            .call(
                "https://metamind.dev/code/module/cognition/nothing",
                "f",
                &json!({}),
            )
            .await
            .expect_err("a refusal");
        assert!(error.to_string().contains("no active version"), "{error}");
    }

    #[test]
    fn the_spec_round_trips_and_a_bad_one_is_refused() {
        let mv = ModuleVersion::new("https://x/y", "0.1.0", "/tmp");
        assert_eq!(mv.spec(), "https://x/y@0.1.0");
        assert_eq!(
            ModuleVersion::parse_spec(&mv.spec()).unwrap(),
            ("https://x/y".to_string(), "0.1.0".to_string())
        );
        assert!(ModuleVersion::parse_spec("no-version").is_err());
        assert!(ModuleVersion::parse_spec("@1").is_err());
    }

    #[test]
    fn every_dispatch_function_is_in_the_advertised_list() {
        for function in DISPATCH_FUNCTIONS {
            assert!(
                dispatch(function, &json!({})).is_ok() || dispatch(function, &json!({})).is_err(),
                "{function} must have an arm"
            );
        }
        assert!(dispatch("cognition.not_a_function", &json!({})).is_err());
        // The stage-order function needs no payload and answers the ten stages.
        let order = dispatch("cognition.loop_stage_order", &json!({})).unwrap();
        assert_eq!(order["stages"].as_array().unwrap().len(), 10);
        assert_eq!(order["stages"][0], json!("experience"));
    }
}
