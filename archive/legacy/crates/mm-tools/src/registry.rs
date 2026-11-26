//! The registry: what tools exist, and the context a tool runs in.
//!
//! A [`ToolRegistry`] is a name-keyed map of [`Tool`] implementations, and the
//! `tool_registry` table is its persisted shadow. Both are real: the map is what
//! decides whether a call can resolve, and the table is what makes "which tools were
//! installed when this action ran, and with what contract" answerable after the fact.
//! A row whose implementation is not in the process is loaded as a tool that refuses
//! with [`ToolError::Unsupported`] rather than being invisible — an installed tool
//! that silently disappeared would turn a deployment mistake into a confusing "no
//! such tool".
//!
//! # `ToolSpec::digest` is the contract
//!
//! Re-registering a tool under the same name with a different digest is a *different
//! contract*: the schema, the capabilities or the safety declarations changed, and a
//! policy rule written against the old one should be re-read. [`ToolRegistry::persist`]
//! records the digest in the row's `spec_json`, and [`ToolRegistry::describe`] reports
//! it, so a change is visible in `mm-cli tool describe` rather than only in a diff.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::action::ActionSpec;
use crate::error::{RegistryError, ToolError};
use crate::sandbox::CapabilityGuard;
use crate::spec::{ToolName, ToolSpec};
use mm_core::{Timestamp, Ulid};
use mm_store_sqlite::SqliteStore;

/// A tool call's arguments.
///
/// A newtype rather than a bare [`serde_json::Value`] so that "the arguments were
/// checked against the declared schema" has a place to happen, once, instead of in
/// every tool: [`ToolArgs::new`] refuses a non-object and refuses a missing required
/// name, and a tool may then read its arguments without re-litigating either.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolArgs(pub serde_json::Value);

impl ToolArgs {
    /// Arguments checked against the tool's schema.
    pub fn new(spec: &ToolSpec, value: serde_json::Value) -> Result<Self, ToolError> {
        if !value.is_object() {
            return Err(ToolError::BadArguments {
                tool: spec.name.clone(),
                reason: format!("arguments must be a JSON object, got {}", kind_of(&value)),
            });
        }
        for required in spec.input_schema.required() {
            if value.get(&required).is_none() {
                return Err(ToolError::BadArguments {
                    tool: spec.name.clone(),
                    reason: format!("the required argument {required:?} is missing"),
                });
            }
        }
        Ok(ToolArgs(value))
    }

    /// An empty argument object.
    pub fn empty() -> Self {
        ToolArgs(serde_json::json!({}))
    }

    /// The value.
    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }

    /// A field, or `None`.
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.0.get(name)
    }

    /// A string field, or `None` when absent.
    pub fn opt_str(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(serde_json::Value::as_str)
    }
}

impl From<serde_json::Value> for ToolArgs {
    fn from(value: serde_json::Value) -> Self {
        ToolArgs(value)
    }
}

/// The JSON type of a value, for an error message.
fn kind_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Everything a tool is handed.
///
/// It carries the guard rather than the capabilities, so a tool *cannot* consult the
/// grant and decide for itself: the only way to touch a path is
/// [`ExecCtx::read_path`] or [`ExecCtx::write_path`], and both refuse outside the
/// capability. That is the phase's sandbox claim expressed as a type.
#[derive(Clone, Debug)]
pub struct ExecCtx {
    /// The action's ULID.
    pub action_id: Ulid,
    /// The trace every record of this action shares.
    pub trace_id: Ulid,
    /// The request as it was authorized.
    pub action: ActionSpec,
    /// The resolved tool's spec.
    pub spec: ToolSpec,
    /// The capability guard, derived from the granted intersection.
    pub guard: CapabilityGuard,
    /// The tabular store, for tools that read kernel tables.
    pub sql: SqliteStore,
    /// The RDF graph, for tools that read it.
    pub graph: mm_store_graph::GraphHandle,
    /// The directory a relative argument resolves against.
    pub workdir: PathBuf,
    /// When the action started.
    pub started_at: Timestamp,
    /// True when the caller explicitly confirmed an irreversible action.
    pub confirmed: bool,
}

impl ExecCtx {
    /// The arguments.
    pub fn args(&self) -> &serde_json::Value {
        &self.action.args
    }

    /// A string argument, refusing a missing or non-string one.
    pub fn arg_str(&self, name: &str) -> Result<String, ToolError> {
        match self.action.args.get(name) {
            Some(serde_json::Value::String(text)) => Ok(text.clone()),
            Some(other) => {
                Err(self.bad_argument(name, format!("expected a string, got {}", kind_of(other))))
            }
            None => Err(self.bad_argument(name, "it is required".to_string())),
        }
    }

    /// An optional string argument.
    pub fn opt_str(&self, name: &str) -> Option<String> {
        self.action
            .args
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }

    /// An optional unsigned integer argument.
    pub fn opt_u64(&self, name: &str) -> Result<Option<u64>, ToolError> {
        match self.action.args.get(name) {
            None => Ok(None),
            Some(serde_json::Value::Number(number)) => number.as_u64().map(Some).ok_or_else(|| {
                self.bad_argument(name, "expected a non-negative integer".to_string())
            }),
            Some(other) => {
                Err(self.bad_argument(name, format!("expected an integer, got {}", kind_of(other))))
            }
        }
    }

    /// An optional boolean argument.
    pub fn opt_bool(&self, name: &str) -> Result<Option<bool>, ToolError> {
        match self.action.args.get(name) {
            None => Ok(None),
            Some(serde_json::Value::Bool(flag)) => Ok(Some(*flag)),
            Some(other) => {
                Err(self.bad_argument(name, format!("expected a boolean, got {}", kind_of(other))))
            }
        }
    }

    /// Resolve a path for reading through the capability guard.
    pub fn read_path(&self, requested: &str) -> Result<PathBuf, ToolError> {
        self.guard.read_path(requested).map_err(ToolError::Sandbox)
    }

    /// Resolve a path for writing through the capability guard.
    pub fn write_path(&self, requested: &str) -> Result<PathBuf, ToolError> {
        self.guard.write_path(requested).map_err(ToolError::Sandbox)
    }

    /// Refuse a payload larger than the grant allows.
    pub fn check_size(&self, bytes: u64) -> Result<(), ToolError> {
        self.guard.check_size(bytes).map_err(ToolError::Sandbox)
    }

    /// Refuse a host that is not allowlisted.
    pub fn check_host(&self, host: &str) -> Result<(), ToolError> {
        self.guard.check_host(host).map_err(ToolError::Sandbox)
    }

    /// Refuse a subprocess the sandbox does not grant.
    pub fn check_subprocess(&self) -> Result<(), ToolError> {
        self.guard.check_subprocess().map_err(ToolError::Sandbox)
    }

    /// Refuse a command the grant does not cover.
    ///
    /// Checked in addition to [`ExecCtx::check_subprocess`], because the two answer
    /// different questions: whether this caller may spawn a process at all, and whether
    /// *this* command is the one it was granted. The pattern comes from the grant, so a
    /// caller granted `cargo test` cannot run `rm`.
    pub fn check_command(&self, command: &str) -> Result<(), ToolError> {
        self.guard
            .check_command(command)
            .map_err(ToolError::Sandbox)
    }

    /// A [`ToolError::BadArguments`] for this call.
    pub fn bad_argument(&self, name: &str, reason: impl Into<String>) -> ToolError {
        ToolError::BadArguments {
            tool: self.spec.name.clone(),
            reason: format!("argument {name:?}: {}", reason.into()),
        }
    }

    /// A [`ToolError::Failed`] for this call.
    pub fn failed(&self, reason: impl Into<String>) -> ToolError {
        ToolError::Failed {
            tool: self.spec.name.clone(),
            reason: reason.into(),
        }
    }

    /// A [`ToolError::Backing`] for this call.
    pub fn backing(&self, reason: impl Into<String>) -> ToolError {
        ToolError::Backing {
            tool: self.spec.name.clone(),
            reason: reason.into(),
        }
    }

    /// A [`ToolError::Unsupported`] for this call.
    pub fn unsupported(&self, reason: impl Into<String>) -> ToolError {
        ToolError::Unsupported {
            tool: self.spec.name.clone(),
            reason: reason.into(),
        }
    }

    /// An outcome with no artifacts.
    pub fn outcome(&self, output: serde_json::Value, summary: impl Into<String>) -> ToolOutcome {
        ToolOutcome {
            output,
            summary: summary.into(),
            touched_paths: Vec::new(),
            artifacts: Vec::new(),
        }
    }
}

/// What a tool produced.
///
/// The output is the machine-readable result, `summary` is the one sentence the
/// observation carries, and the two path lists are what the ledger and the rollback
/// layer need: `touched_paths` is what a snapshot must cover, `artifacts` is what a
/// later step may read. A tool that writes a file and does not say so is a tool whose
/// action cannot be rolled back, which is why the lists exist rather than being
/// inferred from the output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    /// The result.
    pub output: serde_json::Value,
    /// One sentence describing what happened, for the observation.
    pub summary: String,
    /// The paths this call may have changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched_paths: Vec<String>,
    /// The files or resources it produced, for a later step to read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
}

impl ToolOutcome {
    /// An outcome from a value and a summary.
    pub fn new(output: serde_json::Value, summary: impl Into<String>) -> Self {
        ToolOutcome {
            output,
            summary: summary.into(),
            touched_paths: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    /// The same outcome with the paths it touched.
    pub fn touching(mut self, paths: Vec<String>) -> Self {
        self.touched_paths = paths;
        self
    }

    /// The same outcome with the artifacts it produced.
    pub fn with_artifacts(mut self, artifacts: Vec<String>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// A stable hash of the result, for the ledger payload and replay comparison.
    pub fn hash(&self) -> String {
        mm_core::hash_fields(&[
            &crate::canonical_json(&self.output),
            &self.summary,
            &self.touched_paths.join(","),
            &self.artifacts.join(","),
        ])
    }
}

/// A tool.
///
/// `&self` and `Send + Sync`, because a tool is registered once and called many times:
/// a tool with per-call state would make two concurrent actions share it, and the
/// third invariant (every action is auditable and attributable) requires that the
/// state an action touched belongs to that action alone.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The tool's published contract.
    fn spec(&self) -> &ToolSpec;

    /// Do the work. Every failure is a typed [`ToolError`]; a panic would be a bug in
    /// the tool, not a result.
    async fn invoke(&self, ctx: &ExecCtx, args: ToolArgs) -> Result<ToolOutcome, ToolError>;
}

/// A tool whose implementation is not in this process.
///
/// Loaded from `tool_registry` when a row names a tool the running build does not
/// provide. It refuses every call rather than disappearing, so `mm-cli tool list`
/// shows the installed set and a call says why it cannot proceed.
#[derive(Clone, Debug)]
pub struct PersistedTool {
    spec: ToolSpec,
}

impl PersistedTool {
    /// A tool that only knows its spec.
    pub fn new(spec: ToolSpec) -> Self {
        PersistedTool { spec }
    }
}

#[async_trait]
impl Tool for PersistedTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, _ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        Err(ToolError::Unsupported {
            tool: self.spec.name.clone(),
            reason: format!(
                "version {} is registered in tool_registry but its implementation is not in this \
                 build",
                self.spec.version
            ),
        })
    }
}

/// The registry.
///
/// `Debug` is written by hand rather than derived: a `dyn Tool` is not `Debug`, and
/// printing one would print a tool's whole spec anyway. The length is what a debug
/// rendering of a registry is for.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Arc<RwLock<BTreeMap<ToolName, Arc<dyn Tool>>>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.len())
            .finish()
    }
}

impl ToolRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        ToolRegistry::default()
    }

    /// Register a tool.
    ///
    /// A duplicate name is refused rather than replaced: two tools under one name would
    /// mean the policy rules and the audit records written against that name described
    /// whichever implementation happened to register last.
    pub fn register(&self, tool: Arc<dyn Tool>) -> Result<(), RegistryError> {
        let spec = tool.spec().clone().checked()?;
        let name = spec.name.clone();
        let mut tools = self.write();
        if tools.contains_key(&name) {
            return Err(RegistryError::Duplicate(name.0));
        }
        tools.insert(name, tool);
        Ok(())
    }

    /// The tool with this name.
    pub fn resolve(&self, name: &ToolName) -> Result<Arc<dyn Tool>, RegistryError> {
        self.read()
            .get(name)
            .cloned()
            .ok_or_else(|| RegistryError::Unknown(name.0.clone()))
    }

    /// Every registered spec, sorted by name.
    pub fn list(&self) -> Vec<ToolSpec> {
        self.read()
            .values()
            .map(|tool| tool.spec().clone())
            .collect()
    }

    /// Every registered name, sorted.
    pub fn names(&self) -> Vec<ToolName> {
        self.read().keys().cloned().collect()
    }

    /// One spec.
    pub fn describe(&self, name: &ToolName) -> Result<ToolSpec, RegistryError> {
        self.read()
            .get(name)
            .map(|tool| tool.spec().clone())
            .ok_or_else(|| RegistryError::Unknown(name.0.clone()))
    }

    /// How many tools are registered.
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// True when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// True when the name is registered.
    pub fn contains(&self, name: &ToolName) -> bool {
        self.read().contains_key(name)
    }

    /// Replace a registered tool *only* when the name is free, returning what it
    /// replaced. Used by `load` to layer persisted specs under live implementations.
    fn insert_or_replace(&self, tool: Arc<dyn Tool>) -> Option<Arc<dyn Tool>> {
        let name = tool.spec().name.clone();
        self.write().insert(name, tool)
    }

    /// Write every registered spec into `tool_registry`, one row per tool.
    ///
    /// `INSERT OR REPLACE` on the unique name: a tool re-registered with a new version
    /// is the same installation, advanced, and the row's `spec_json` is what carries the
    /// contract that changed.
    pub async fn persist(&self, sql: &SqliteStore) -> Result<usize, RegistryError> {
        let at = Timestamp::now().to_rfc3339();
        let specs = self.list();
        for spec in &specs {
            let id = row_id(spec.name.as_str());
            let registered = row_id(&format!("{}@{}", spec.name, spec.version));
            let spec_json =
                serde_json::to_string(spec).map_err(|e| RegistryError::InvalidSpec {
                    tool: spec.name.0.clone(),
                    reason: e.to_string(),
                })?;
            sqlx::query(
                "INSERT INTO tool_registry \
                 (id, name, version, module_uri, spec_json, sandbox_tier, reversibility, active, \
                  registered_ulid, system_from, system_to, valid_from, valid_to) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?, NULL, ?, NULL) \
                 ON CONFLICT(name) DO UPDATE SET \
                   version = excluded.version, \
                   module_uri = excluded.module_uri, \
                   spec_json = excluded.spec_json, \
                   sandbox_tier = excluded.sandbox_tier, \
                   reversibility = excluded.reversibility, \
                   active = 1, \
                   valid_from = excluded.valid_from",
            )
            .bind(&id)
            .bind(spec.name.as_str())
            .bind(&spec.version)
            .bind(&spec.module_uri)
            .bind(&spec_json)
            .bind(spec.sandbox_tier.as_str())
            .bind(spec.reversibility.as_str())
            .bind(&registered)
            .bind(&at)
            .bind(&at)
            .execute(sql.pool())
            .await
            .map_err(|e| RegistryError::InvalidSpec {
                tool: spec.name.0.clone(),
                reason: format!("could not persist: {e}"),
            })?;
        }
        Ok(specs.len())
    }

    /// Read the persisted specs.
    pub async fn load(sql: &SqliteStore) -> Result<Self, RegistryError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, spec_json FROM tool_registry WHERE active = 1 ORDER BY name",
        )
        .fetch_all(sql.pool())
        .await
        .map_err(|e| RegistryError::InvalidSpec {
            tool: "tool_registry".into(),
            reason: format!("could not read tool_registry: {e}"),
        })?;
        let registry = ToolRegistry::new();
        for (name, spec_json) in rows {
            let spec: ToolSpec =
                serde_json::from_str(&spec_json).map_err(|e| RegistryError::InvalidSpec {
                    tool: name.clone(),
                    reason: format!("stored spec is not a ToolSpec: {e}"),
                })?;
            registry.insert_or_replace(Arc::new(PersistedTool::new(spec)));
        }
        Ok(registry)
    }

    /// Layer every *live* tool over the persisted rows.
    ///
    /// The order matters: the persisted row says what is installed, and the live
    /// implementation is what can actually run it. A tool that is live but not
    /// persisted is still callable — the boot path persists before it serves — but it
    /// will be written by the next `persist`.
    pub fn with_live(&self, live: &ToolRegistry) -> Self {
        let merged = self.clone();
        for name in live.names() {
            if let Ok(tool) = live.resolve(&name) {
                merged.insert_or_replace(tool);
            }
        }
        merged
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<ToolName, Arc<dyn Tool>>> {
        self.tools.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<ToolName, Arc<dyn Tool>>> {
        self.tools.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// A deterministic ULID-shaped row id for a name.
///
/// Derived rather than drawn, so re-persisting a tool updates its row instead of
/// appending a second one, and `tool_registry.id` stays the same across restarts.
pub fn row_id(key: &str) -> String {
    let digest = mm_core::content_hash(key.as_bytes());
    let bytes = digest.as_bytes();
    let mut parts = [0u8; 16];
    for (index, slot) in parts.iter_mut().enumerate() {
        let high = hex_nibble(bytes[index * 2]) << 4;
        let low = hex_nibble(bytes[index * 2 + 1]);
        *slot = high | low;
    }
    parts[0] &= 0b0000_0111;
    mm_core::ulid_string(&Ulid::from_bytes(parts))
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{PermissionAction, PermissionReq, Principal};
    use crate::sandbox::{SandboxCapabilities, SandboxTier};
    use crate::spec::{JsonSchema, Reversibility, SideEffectClass, ToolAnnotations};

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: ToolName::new(name).unwrap(),
            version: "0.1.0".into(),
            description: "a tool".into(),
            input_schema: JsonSchema::object_with_strings(&["path"]),
            output_schema: JsonSchema::any_object(),
            permissions: vec![PermissionReq::new(
                "fs:read:data/sandbox/**",
                PermissionAction::Read,
            )],
            reversibility: Reversibility::Reversible,
            // A read-only tool has no side effects: `TOOL_REGISTRY`'s read tool reads
            // and nothing else, and the spec check refuses `read_only` beside a local
            // effect. The write tool below is the one that touches the disk.
            side_effects: SideEffectClass::None,
            annotations: ToolAnnotations {
                read_only: true,
                destructive: false,
                idempotent: true,
                open_world: false,
            },
            module_uri: "https://metamind.dev/code/module/tools/fs-read".into(),
            sandbox_tier: SandboxTier::WasmCaps,
        }
    }

    fn tool(name: &str) -> Arc<dyn Tool> {
        Arc::new(EchoTool { spec: spec(name) })
    }

    struct EchoTool {
        spec: ToolSpec,
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn spec(&self) -> &ToolSpec {
            &self.spec
        }

        async fn invoke(&self, ctx: &ExecCtx, args: ToolArgs) -> Result<ToolOutcome, ToolError> {
            Ok(ctx.outcome(args.0.clone(), "echoed"))
        }
    }

    /// A context over a throwaway store and an in-memory graph.
    ///
    /// Both are leaked deliberately: the graph's writer thread lives inside
    /// `GraphStore`, which the handle borrows from, and a test that dropped it would
    /// hand the handle a closed mailbox. The shapes file is the epistemic one because
    /// these tests exercise the tool protocol and never validate against the tools
    /// shapes.
    async fn ctx(spec: ToolSpec) -> ExecCtx {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let sql = SqliteStore::open(&dir.path().join("mm-tools.db"))
            .await
            .unwrap();
        sql.migrate().await.unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl");
        let graph = Box::leak(Box::new(
            mm_store_graph::GraphStore::in_memory(&shapes)
                .await
                .unwrap(),
        ));
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 1),
            trace_id: Ulid::from_parts(1_700_000_000_000, 2),
            action: ActionSpec::new(
                spec.name.clone(),
                serde_json::json!({ "path": "data/sandbox/x" }),
                Principal::system(),
            ),
            guard: CapabilityGuard::new(
                SandboxCapabilities::default(),
                spec.sandbox_tier,
                dir.path(),
            ),
            spec: spec.clone(),
            sql,
            graph: graph.handle().clone(),
            workdir: dir.path().to_path_buf(),
            started_at: Timestamp::now(),
            confirmed: false,
        }
    }

    #[tokio::test]
    async fn a_duplicate_name_is_refused() {
        let registry = ToolRegistry::new();
        registry.register(tool("fs.read")).unwrap();
        let error = registry.register(tool("fs.read")).unwrap_err();
        assert_eq!(error.code(), "registry.duplicate");
        assert_eq!(registry.len(), 1);
    }

    #[tokio::test]
    async fn an_unknown_name_does_not_resolve() {
        let registry = ToolRegistry::new();
        // `err()` rather than `unwrap_err()`: the Ok variant is `Arc<dyn Tool>`, which is
        // deliberately not `Debug` (a tool's debug rendering would be its whole spec).
        let error = registry
            .resolve(&ToolName::new("fs.read").unwrap())
            .err()
            .expect("the name is not registered");
        assert_eq!(error.code(), "registry.unknown");
    }

    #[tokio::test]
    async fn register_list_and_describe_agree() {
        let registry = ToolRegistry::new();
        registry.register(tool("fs.read")).unwrap();
        registry.register(tool("graph.query")).unwrap();
        let names: Vec<String> = registry.names().iter().map(|n| n.0.clone()).collect();
        assert_eq!(
            names,
            vec!["fs.read".to_string(), "graph.query".to_string()]
        );
        assert_eq!(registry.list().len(), 2);
        assert_eq!(
            registry
                .describe(&ToolName::new("fs.read").unwrap())
                .unwrap()
                .name
                .as_str(),
            "fs.read"
        );
    }

    #[tokio::test]
    async fn an_invalid_spec_is_refused_at_registration() {
        struct BadTool {
            spec: ToolSpec,
        }
        #[async_trait]
        impl Tool for BadTool {
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
            async fn invoke(
                &self,
                _ctx: &ExecCtx,
                _args: ToolArgs,
            ) -> Result<ToolOutcome, ToolError> {
                unreachable!()
            }
        }
        let mut bad = spec("fs.read");
        // read_only with a Local side effect is a contradiction.
        bad.side_effects = SideEffectClass::Local;
        assert!(ToolRegistry::new()
            .register(Arc::new(BadTool { spec: bad }))
            .is_err());
    }

    #[tokio::test]
    async fn args_are_checked_against_the_declared_schema() {
        let spec = spec("fs.read");
        assert!(ToolArgs::new(&spec, serde_json::json!({ "path": "x" })).is_ok());
        let missing = ToolArgs::new(&spec, serde_json::json!({})).unwrap_err();
        assert_eq!(missing.code(), "tool.bad_arguments");
        let not_object = ToolArgs::new(&spec, serde_json::json!([1, 2])).unwrap_err();
        assert!(not_object.to_string().contains("JSON object"));
    }

    #[tokio::test]
    async fn the_outcome_hash_tracks_the_result() {
        let one = ToolOutcome::new(serde_json::json!({ "n": 1 }), "one");
        let same = ToolOutcome::new(serde_json::json!({ "n": 1 }), "one");
        let other = ToolOutcome::new(serde_json::json!({ "n": 2 }), "one");
        assert_eq!(one.hash(), same.hash());
        assert_ne!(one.hash(), other.hash());
        assert_eq!(
            ToolOutcome::new(serde_json::json!({ "n": 1 }), "one")
                .touching(vec!["a".into()])
                .hash(),
            ToolOutcome::new(serde_json::json!({ "n": 1 }), "one")
                .touching(vec!["a".into()])
                .hash()
        );
    }

    #[tokio::test]
    async fn a_persisted_tool_refuses_rather_than_disappearing() {
        let persisted = PersistedTool::new(spec("fs.read"));
        let context = ctx(spec("fs.read")).await;
        let error = persisted
            .invoke(&context, ToolArgs::empty())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "tool.unsupported");
        assert!(error.to_string().contains("not in this build"));
    }

    #[tokio::test]
    async fn the_row_id_is_stable_and_ulid_shaped() {
        assert_eq!(row_id("fs.read"), row_id("fs.read"));
        assert_ne!(row_id("fs.read"), row_id("fs.write"));
        assert_eq!(row_id("fs.read").len(), mm_core::ULID_LEN);
    }

    #[tokio::test]
    async fn live_tools_layer_over_persisted_rows() {
        let persisted = ToolRegistry::new();
        persisted.insert_or_replace(Arc::new(PersistedTool::new(spec("fs.read"))));
        let live = ToolRegistry::new();
        live.register(tool("fs.read")).unwrap();
        let context = ctx(spec("fs.read")).await;
        let merged = persisted.with_live(&live);
        let resolved = merged.resolve(&ToolName::new("fs.read").unwrap()).unwrap();
        assert!(
            resolved.invoke(&context, ToolArgs::empty()).await.is_ok(),
            "the live implementation must win"
        );
    }
}
