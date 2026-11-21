# Proactive Plugin System

> Extensible, testable proactive behaviors via plugin architecture.

---

## 1. Problem

Proactive behaviors hardcoded in CME:
- Hard to add new behaviors
- Hard to test in isolation
- Can't disable/configure per user
- No versioning or rollback
- Domain-specific behaviors need custom code

---

## 2. Plugin Architecture

### 2.1 Core Trait

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[async_trait]
trait ProactivePlugin: Send + Sync + 'static {
    /// Unique identifier
    fn id(&self) -> &'static str;
    
    /// Human-readable name
    fn name(&self) -> &'static str;
    
    /// Version for compatibility
    fn version(&self) -> &'static str;
    
    /// Description for UI/config
    fn description(&self) -> &'static str;
    
    /// Triggers that activate this plugin
    fn triggers(&self) -> Vec<ProactiveTrigger>;
    
    /// Configuration schema (JSON Schema)
    fn config_schema(&self) -> serde_json::Value;
    
    /// Default configuration
    fn default_config(&self) -> PluginConfig;
    
    /// Evaluate whether to generate proactive items
    async fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem>;
    
    /// Handle user feedback on generated items
    async fn on_feedback(&self, item_id: &str, feedback: ProactiveFeedback);
    
    /// Called when plugin is loaded
    async fn on_load(&self, registry: &PluginRegistry) -> Result<()>;
    
    /// Called when plugin is unloaded
    async fn on_unload(&self) -> Result<()>;
}
```

### 2.2 Trigger System

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ProactiveTrigger {
    /// User message matches keywords
    UserMessage { 
        keywords: Vec<String>,
        min_confidence: f32,
    },
    
    /// Every N turns
    TurnInterval { 
        every_n: u32,
        offset: u32,
    },
    
    /// User idle for N minutes
    IdleTimeout { 
        minutes: u32,
    },
    
    /// Tool result matches criteria
    ToolResult { 
        tool: String,
        outcome: ToolOutcome,
        filters: HashMap<String, String>,
    },
    
    /// Topic mentioned frequently
    TopicFrequency { 
        topic: String,
        threshold: u32,
        window_turns: u32,
    },
    
    /// Scheduled (cron)
    Scheduled { 
        cron: String,
        timezone: String,
    },
    
    /// Memory query returns results
    MemoryMatch { 
        query: String,
        min_score: f32,
        max_results: usize,
    },
    
    /// Custom predicate (WASM-compiled)
    Custom { 
        wasm_module: String,
        function: String,
    },
    
    /// Agent completed a goal
    GoalCompleted { 
        goal_type: String,
    },
    
    /// New artifact created
    ArtifactCreated { 
        kind: ArtifactKind,
    },
    
    /// Error pattern detected
    ErrorPattern { 
        pattern: String,
        min_occurrences: u32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum ToolOutcome { Success, Failure, Partial, Timeout }
```

### 2.3 Context for Evaluation

```rust
struct ProactiveContext<'a> {
    /// Current session
    session: &'a Session,
    
    /// User model (preferences, history)
    user_model: &'a UserModel,
    
    /// Recent conversation turns
    recent_turns: &'a [Turn],
    
    /// Memory store for queries
    memory: &'a MemoryStore,
    
    /// Available tools
    tools: &'a ToolRegistry,
    
    /// Time since last proactive item
    time_since_last_proactive: Duration,
    
    /// Trigger that fired
    trigger: ProactiveTrigger,
    
    /// Current proactive queue state
    queue_state: &'a ProactiveQueueState,
}

struct ProactiveQueueState {
    pending_count: usize,
    last_proactive_turn: Option<u32>,
    items_this_session: usize,
    rejected_items: Vec<ProactiveItemId>,
}
```

### 2.4 Proactive Items (Output)

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProactiveItem {
    id: ProactiveItemId,
    plugin_id: String,
    kind: ProactiveItemKind,
    confidence: f32,              // 0.0–1.0
    priority: ProactivePriority,
    expires_at: Option<Timestamp>,
    metadata: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum ProactiveItemKind {
    /// "Consider X — want me to prototype?"
    Suggestion {
        title: String,
        description: String,
        action: ActionRef,
        alternatives: Vec<String>,
    },
    
    /// "That migration drops a column still used by admin."
    Correction {
        severity: CorrectionSeverity,
        message: String,
        fix_suggestion: Option<String>,
    },
    
    /// "Clean refactor — the new service layer cuts 200 lines."
    Compliment {
        context: String,
        evidence: String,
    },
    
    /// "Fixed. Your code now passes 'works on my machine' cert."
    Joke {
        context: String,
        tone: JokeTone,
    },
    
    /// "Pipeline green. Anything need attention?"
    CheckIn {
        message: String,
        details: Option<Vec<String>>,
    },
    
    /// "Options: 1) Profile first 2) Jump to caching"
    GoalNarrowing {
        vague_goal: String,
        options: Vec<GoalOption>,
    },
    
    /// "Want me to generate the rollback migration?"
    ActionOffer {
        action: ActionRef,
        reason: String,
    },
    
    /// "Last time we were implementing Redis caching..."
    Continuation {
        message: String,
        related_session: Option<SessionId>,
    },
    
    /// "Deploy failed — check logs?"
    Alert {
        severity: AlertSeverity,
        title: String,
        description: String,
        actions: Vec<ActionRef>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum ProactivePriority { Low, Normal, High, Critical }
#[derive(Clone, Debug, Serialize, Deserialize)]
enum CorrectionSeverity { Hint, Warning, Critical }
#[derive(Clone, Debug, Serialize, Deserialize)]
enum JokeTone { Light, Technical, SelfDeprecating, Situational }
#[derive(Clone, Debug, Serialize, Deserialize)]
enum AlertSeverity { Info, Warning, Critical }
```

---

## 3. Built-in Plugins

### 3.1 Deploy Monitor Plugin

```rust
struct DeployMonitorPlugin {
    config: DeployMonitorConfig,
    http_client: reqwest::Client,
}

struct DeployMonitorConfig {
    check_interval_secs: u64,
    services: Vec<String>,
    alert_on_failure: bool,
    auto_rollback_suggestion: bool,
}

#[async_trait]
impl ProactivePlugin for DeployMonitorPlugin {
    fn id(&self) -> &'static str { "deploy_monitor" }
    fn name(&self) -> &'static str { "Deploy Monitor" }
    fn version(&self) -> &'static str { "1.0.0" }
    fn description(&self) -> &'static str { "Monitors deploy health, suggests rollback on failure" }
    
    fn triggers(&self) -> Vec<ProactiveTrigger> {
        vec![
            ProactiveTrigger::ToolResult { 
                tool: "deploy".into(), 
                outcome: ToolOutcome::Failure, 
                filters: HashMap::new() 
            },
            ProactiveTrigger::Scheduled { 
                cron: "*/15 * * * *".into(), 
                timezone: "UTC".into() 
            },
        ]
    }
    
    fn config_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "check_interval_secs": { "type": "integer", "default": 900 },
                "services": { "type": "array", "items": { "type": "string" } },
                "alert_on_failure": { "type": "boolean", "default": true },
                "auto_rollback_suggestion": { "type": "boolean", "default": true },
            }
        })
    }
    
    fn default_config(&self) -> PluginConfig {
        PluginConfig {
            enabled: true,
            cooldown_turns: 5,
            min_confidence: 0.7,
            custom: json!({
                "check_interval_secs": 900,
                "services": [],
                "alert_on_failure": true,
                "auto_rollback_suggestion": true,
            }),
        }
    }
    
    async fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem> {
        let mut items = Vec::new();
        
        match &ctx.trigger {
            ProactiveTrigger::ToolResult { tool, outcome, .. } if tool == "deploy" && *outcome == ToolOutcome::Failure => {
                // Extract failure details from tool result
                if let Some(failure) = ctx.recent_tool_results().find(|r| r.tool == "deploy") {
                    items.push(ProactiveItem {
                        id: ProactiveItemId::new(),
                        plugin_id: self.id().into(),
                        kind: ProactiveItemKind::Alert {
                            severity: AlertSeverity::Critical,
                            title: format!("Deploy Failed: {}", failure.service),
                            description: failure.error.clone(),
                            actions: vec![
                                ActionRef::new("check_logs", json!({"service": failure.service})),
                                ActionRef::new("recent_commits", json!({"service": failure.service})),
                                ActionRef::new("rollback", json!({"service": failure.service})),
                            ],
                        },
                        confidence: 0.95,
                        priority: ProactivePriority::Critical,
                        expires_at: Some(now() + Duration::hours(1)),
                        metadata: HashMap::new(),
                    });
                }
            }
            
            ProactiveTrigger::Scheduled { .. } => {
                // Periodic health check
                if let Ok(health) = self.check_all_deploys().await {
                    if health.has_warnings() {
                        items.push(ProactiveItem {
                            id: ProactiveItemId::new(),
                            plugin_id: self.id().into(),
                            kind: ProactiveItemKind::CheckIn {
                                message: format!("Deploy pipeline: {} warnings", health.warning_count),
                                details: Some(health.warnings),
                            },
                            confidence: 0.8,
                            priority: ProactivePriority::Normal,
                            expires_at: Some(now() + Duration::hours(4)),
                            metadata: HashMap::new(),
                        });
                    }
                }
            }
            
            _ => {}
        }
        
        items
    }
    
    async fn on_feedback(&self, item_id: &str, feedback: ProactiveFeedback) {
        match feedback {
            ProactiveFeedback::Rejected => {
                self.config.check_interval_secs = (self.config.check_interval_secs * 2).min(86400);
            }
            ProactiveFeedback::Accepted { acted_upon: true } => {
                self.config.check_interval_secs = (self.config.check_interval_secs / 2).max(300);
            }
            _ => {}
        }
    }
}
```

### 3.2 Code Health Plugin

```rust
struct CodeHealthPlugin {
    config: CodeHealthConfig,
}

struct CodeHealthConfig {
    check_on_save: bool,
    check_interval_turns: u32,
    severity_threshold: Severity,
    auto_fix_suggestions: bool,
}

#[async_trait]
impl ProactivePlugin for CodeHealthPlugin {
    fn id(&self) -> &'static str { "code_health" }
    fn name(&self) -> &'static str { "Code Health Monitor" }
    fn triggers(&self) -> Vec<ProactiveTrigger> {
        vec![
            ProactiveTrigger::ToolResult { 
                tool: "write_file".into(), 
                outcome: ToolOutcome::Success, 
                filters: HashMap::from([("ext".into(), "rs".into())]) 
            },
            ProactiveTrigger::TurnInterval { 
                every_n: 10, 
                offset: 0 
            },
        ]
    }
    
    async fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem> {
        let mut items = Vec::new();
        
        match &ctx.trigger {
            ProactiveTrigger::ToolResult { tool, outcome, filters } 
                if tool == "write_file" && *outcome == ToolOutcome::Success 
                && filters.get("ext") == Some(&"rs".into()) =>
            {
                // Get the file that was written
                if let Some(result) = ctx.recent_tool_results().find(|r| r.tool == "write_file") {
                    let file_path = result.args.get("path").unwrap().as_str().unwrap();
                    let content = result.args.get("content").unwrap().as_str().unwrap();
                    
                    // Run linters
                    if let Ok(issues) = self.lint_rust(file_path, content).await {
                        if !issues.is_empty() {
                            let high_severity = issues.iter().filter(|i| i.severity >= self.config.severity_threshold).count();
                            if high_severity > 0 {
                                items.push(ProactiveItem {
                                    id: ProactiveItemId::new(),
                                    plugin_id: self.id().into(),
                                    kind: ProactiveItemKind::Correction {
                                        severity: if high_severity > 3 { CorrectionSeverity::Critical } else { CorrectionSeverity::Warning },
                                        message: format!("{} issues in {} ({} high severity)", issues.len(), file_path, high_severity),
                                        fix_suggestion: if self.config.auto_fix_suggestions {
                                            Some("Run `cargo fix` or apply suggested fixes".into())
                                        } else { None },
                                    },
                                    confidence: 0.9,
                                    priority: ProactivePriority::High,
                                    expires_at: Some(now() + Duration::hours(2)),
                                    metadata: json!({"file": file_path, "issues": issues}).as_object().unwrap().clone(),
                                });
                            }
                        }
                    }
                }
            }
            
            ProactiveTrigger::TurnInterval { .. } => {
                // Periodic project-wide check
                if let Ok(report) = self.project_health_check().await {
                    if report.has_degraded() {
                        items.push(ProactiveItem {
                            id: ProactiveItemId::new(),
                            plugin_id: self.id().into(),
                            kind: ProactiveItemKind::Suggestion {
                                title: "Code Health Degraded".into(),
                                description: format!("{} new issues since last check", report.new_issues),
                                action: ActionRef::new("view_health_report", json!({})),
                                alternatives: vec!["Run `cargo clippy --fix`".into(), "Schedule refactor time".into()],
                            },
                            confidence: 0.8,
                            priority: ProactivePriority::Normal,
                            expires_at: Some(now() + Duration::hours(24)),
                            metadata: HashMap::new(),
                        });
                    }
                }
            }
            
            _ => {}
        }
        
        items
    }
}
```

### 3.3 Security Scanner Plugin

```rust
struct SecurityScannerPlugin {
    config: SecurityScannerConfig,
    cve_client: CveApiClient,
}

struct SecurityScannerConfig {
    scan_on_dependency_add: bool,
    daily_scan: bool,
    severity_threshold: CveSeverity,
    auto_patch_suggestions: bool,
}

#[async_trait]
impl ProactivePlugin for SecurityScannerPlugin {
    fn id(&self) -> &'static str { "security_scanner" }
    fn name(&self) -> &'static str { "Security Scanner" }
    fn triggers(&self) -> Vec<ProactiveTrigger> {
        vec![
            ProactiveTrigger::ToolResult { 
                tool: "cargo_add".into(), 
                outcome: ToolOutcome::Success, 
                filters: HashMap::new() 
            },
            ProactiveTrigger::Scheduled { 
                cron: "0 6 * * *".into(), 
                timezone: "UTC".into() 
            },
            ProactiveTrigger::MemoryMatch {
                query: "CVE vulnerability".into(),
                min_score: 0.7,
                max_results: 5,
            },
        ]
    }
    
    async fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem> {
        let mut items = Vec::new();
        
        match &ctx.trigger {
            ProactiveTrigger::ToolResult { tool, .. } if tool == "cargo_add" => {
                if let Some(result) = ctx.recent_tool_results().find(|r| r.tool == "cargo_add") {
                    let crate_name = result.args.get("name").unwrap().as_str().unwrap();
                    let version = result.args.get("version").unwrap().as_str().unwrap();
                    
                    if let Ok(vulns) = self.check_crate(crate_name, version).await {
                        if !vulns.is_empty() {
                            items.push(ProactiveItem {
                                id: ProactiveItemId::new(),
                                plugin_id: self.id().into(),
                                kind: ProactiveItemKind::Alert {
                                    severity: AlertSeverity::Critical,
                                    title: format!("Vulnerability in {}@{}", crate_name, version),
                                    description: vulns.iter().map(|v| format!("{}: {}", v.id, v.summary)).collect::<Vec<_>>().join("\n"),
                                    actions: vec![
                                        ActionRef::new("update_dependency", json!({"crate": crate_name})),
                                        ActionRef::new("find_alternative", json!({"crate": crate_name})),
                                    ],
                                },
                                confidence: 0.95,
                                priority: ProactivePriority::Critical,
                                expires_at: Some(now() + Duration::days(7)),
                                metadata: json!({"crate": crate_name, "version": version, "vulns": vulns}).as_object().unwrap().clone(),
                            });
                        }
                    }
                }
            }
            
            ProactiveTrigger::Scheduled { .. } => {
                // Daily full scan
                if let Ok(report) = self.full_project_scan().await {
                    if report.critical_count > 0 {
                        items.push(ProactiveItem {
                            id: ProactiveItemId::new(),
                            plugin_id: self.id().into(),
                            kind: ProactiveItemKind::Alert {
                                severity: AlertSeverity::Critical,
                                title: "Critical Vulnerabilities Found".into(),
                                description: format!("{} critical, {} high CVEs in dependencies", report.critical_count, report.high_count),
                                actions: vec![
                                    ActionRef::new("view_security_report", json!({})),
                                    ActionRef::new("auto_patch", json!({})),
                                ],
                            },
                            confidence: 0.9,
                            priority: ProactivePriority::Critical,
                            expires_at: Some(now() + Duration::days(1)),
                            metadata: HashMap::new(),
                        });
                    }
                }
            }
            
            _ => {}
        }
        
        items
    }
}
```

### 3.4 Team Convention Plugin

```rust
struct TeamConventionPlugin {
    config: TeamConventionConfig,
    conventions: Vec<ConventionRule>,
}

struct ConventionRule {
    id: String,
    name: String,
    pattern: Regex,
    message: String,
    severity: CorrectionSeverity,
    auto_fix: Option<String>,
    file_patterns: Vec<String>,
}

#[async_trait]
impl ProactivePlugin for TeamConventionPlugin {
    fn id(&self) -> &'static str { "team_conventions" }
    fn name(&self) -> &'static str { "Team Convention Enforcer" }
    fn triggers(&self) -> Vec<ProactiveTrigger> {
        vec![
            ProactiveTrigger::ToolResult { 
                tool: "write_file".into(), 
                outcome: ToolOutcome::Success, 
                filters: HashMap::new() 
            },
        ]
    }
    
    async fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem> {
        let mut items = Vec::new();
        
        if let ProactiveTrigger::ToolResult { tool, outcome, .. } = &ctx.trigger {
            if tool == "write_file" && *outcome == ToolOutcome::Success {
                if let Some(result) = ctx.recent_tool_results().find(|r| r.tool == "write_file") {
                    let file_path = result.args.get("path").unwrap().as_str().unwrap();
                    let content = result.args.get("content").unwrap().as_str().unwrap();
                    
                    for rule in &self.conventions {
                        if rule.file_patterns.iter().any(|p| glob_match(p, file_path)) {
                            for mat in rule.pattern.find_iter(content) {
                                items.push(ProactiveItem {
                                    id: ProactiveItemId::new(),
                                    plugin_id: self.id().into(),
                                    kind: ProactiveItemKind::Correction {
                                        severity: rule.severity,
                                        message: format!("Convention '{}': {}", rule.name, rule.message),
                                        fix_suggestion: rule.auto_fix.clone(),
                                    },
                                    confidence: 0.95,
                                    priority: ProactivePriority::High,
                                    expires_at: Some(now() + Duration::hours(4)),
                                    metadata: json!({
                                        "file": file_path,
                                        "rule": rule.id,
                                        "match": mat.as_str(),
                                        "line": content[..mat.start()].matches('\n').count() + 1,
                                    }).as_object().unwrap().clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
        
        items
    }
}
```

---

## 4. Plugin Registry & Management

```rust
struct PluginRegistry {
    plugins: HashMap<String, Box<dyn ProactivePlugin>>,
    configs: HashMap<String, PluginConfig>,
    load_order: Vec<String>,
}

struct PluginConfig {
    enabled: bool,
    cooldown_turns: u32,
    min_confidence: f32,
    custom: serde_json::Value,  // Plugin-specific config
}

impl PluginRegistry {
    fn register(&mut self, plugin: Box<dyn ProactivePlugin>) -> Result<()> {
        let id = plugin.id();
        if self.plugins.contains_key(id) {
            return Err(anyhow!("Plugin {} already registered", id));
        }
        
        let config = self.configs.get(id).cloned().unwrap_or_else(|| plugin.default_config());
        plugin.on_load(self).await?;
        
        self.plugins.insert(id.into(), plugin);
        self.load_order.push(id.into());
        Ok(())
    }
    
    fn unregister(&mut self, id: &str) -> Result<()> {
        if let Some(mut plugin) = self.plugins.remove(id) {
            plugin.on_unload().await?;
            self.load_order.retain(|i| i != id);
            Ok(())
        } else {
            Err(anyhow!("Plugin {} not found", id))
        }
    }
    
    fn get_enabled(&self) -> Vec<&dyn ProactivePlugin> {
        self.load_order.iter()
            .filter_map(|id| self.plugins.get(id))
            .filter(|p| self.configs.get(p.id()).map(|c| c.enabled).unwrap_or(true))
            .map(|p| p.as_ref())
            .collect()
    }
    
    fn update_config(&mut self, id: &str, config: PluginConfig) -> Result<()> {
        self.configs.insert(id.into(), config);
        Ok(())
    }
}
```

---

## 5. WASM Plugin Support (Dynamic Loading)

```rust
struct WasmPluginWrapper {
    instance: wasmtime::Instance,
    store: wasmtime::Store<WasmPluginState>,
    memory: wasmtime::Memory,
    evaluate_func: wasmtime::Func,
    on_feedback_func: wasmtime::Func,
    config_schema_func: wasmtime::Func,
}

struct WasmPluginState {
    plugin_id: String,
    host_functions: HostFunctionTable,
}

impl WasmPluginWrapper {
    async fn new(wasm_bytes: &[u8], plugin_id: String) -> Result<Self> {
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, wasm_bytes)?;
        let mut linker = wasmtime::Linker::new(&engine);
        
        // Define host functions the plugin can call
        Self::define_host_functions(&mut linker)?;
        
        let mut store = wasmtime::Store::new(&engine, WasmPluginState {
            plugin_id: plugin_id.clone(),
            host_functions: HostFunctionTable::new(),
        });
        
        let instance = linker.instantiate(&mut store, &module)?;
        
        Ok(Self {
            instance,
            store,
            memory: instance.get_memory(&mut store, "memory").unwrap(),
            evaluate_func: instance.get_typed_func(&mut store, "evaluate")?,
            on_feedback_func: instance.get_typed_func(&mut store, "on_feedback")?,
            config_schema_func: instance.get_typed_func(&mut store, "config_schema")?,
        })
    }
    
    fn define_host_functions(linker: &mut wasmtime::Linker<WasmPluginState>) -> Result<()> {
        // memory.query(query: string) -> string (JSON)
        linker.func_wrap("host", "memory_query", |mut caller: wasmtime::Caller<WasmPluginState>, query_ptr: i32, query_len: i32| -> i32 {
            // Read query from WASM memory
            // Execute query via host
            // Write result to WASM memory
            // Return result pointer
            Ok(0)
        })?;
        
        // tools.execute(tool: string, args: string) -> string (JSON)
        linker.func_wrap("host", "tools_execute", |mut caller, tool_ptr, tool_len, args_ptr, args_len| {
            Ok(0)
        })?;
        
        // log(level: string, message: string)
        linker.func_wrap("host", "log", |mut caller, level_ptr, level_len, msg_ptr, msg_len| {
            Ok(())
        })?;
        
        Ok(())
    }
}

#[async_trait]
impl ProactivePlugin for WasmPluginWrapper {
    fn id(&self) -> &'static str { self.plugin_id.leak() }
    // ... delegate to WASM functions
}
```

---

## 6. Plugin Configuration (Per User/Project)

```toml
# ~/.metamind/plugins.toml
[plugins.deploy_monitor]
enabled = true
cooldown_turns = 5
min_confidence = 0.7
custom = { check_interval_secs = 900, services = ["api", "worker", "frontend"], alert_on_failure = true }

[plugins.code_health]
enabled = true
cooldown_turns = 10
min_confidence = 0.8
custom = { check_on_save = true, severity_threshold = "Warning", auto_fix_suggestions = true }

[plugins.security_scanner]
enabled = true
cooldown_turns = 20
min_confidence = 0.9
custom = { scan_on_dependency_add = true, daily_scan = true, severity_threshold = "High" }

[plugins.team_conventions]
enabled = true
cooldown_turns = 3
min_confidence = 0.95
custom = { conventions_file = ".metamind/conventions.yaml" }

# Per-project overrides in .metamind/plugins.toml
```

---

## 7. Evaluation Loop Integration

```rust
impl CME {
    async fn evaluate_proactive_queue(&mut self, ctx: ProactiveContext) {
        let plugins = self.plugin_registry.get_enabled();
        
        for plugin in plugins {
            let config = self.plugin_registry.configs.get(plugin.id()).unwrap();
            
            // Check cooldown
            if ctx.queue_state.time_since_last_proactive < Duration::from_secs(config.cooldown_turns as u64 * 30) {
                continue;
            }
            
            // Check if plugin's triggers match
            if !plugin.triggers().iter().any(|t| t.matches(&ctx)) {
                continue;
            }
            
            // Evaluate
            let items = plugin.evaluate(&ctx).await;
            
            // Filter by confidence
            for item in items {
                if item.confidence >= config.min_confidence {
                    self.proactive_queue.push(item);
                }
            }
        }
        
        // Sort by priority, take top 1
        self.proactive_queue.sort_by_priority();
        if let Some(top) = self.proactive_queue.pop() {
            self.inject_proactive_item(top).await;
        }
    }
}
```

---

## 8. Testing Plugins

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_deploy_monitor_failure_alert() {
        let plugin = DeployMonitorPlugin::default();
        let ctx = create_test_context()
            .with_tool_result("deploy", ToolOutcome::Failure, json!({
                "service": "api",
                "error": "migration timeout"
            }))
            .build();
        
        let items = plugin.evaluate(&ctx).await;
        
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].kind, ProactiveItemKind::Alert { severity: AlertSeverity::Critical, .. }));
        assert!(items[0].confidence > 0.9);
    }
    
    #[tokio::test]
    async fn test_code_health_correction() {
        let plugin = CodeHealthPlugin::default();
        let ctx = create_test_context()
            .with_tool_result("write_file", ToolOutcome::Success, json!({
                "path": "src/main.rs",
                "content": "fn main() { println!(\"hello\") }"  // missing semicolon
            }))
            .build();
        
        let items = plugin.evaluate(&ctx).await;
        
        assert!(!items.is_empty());
        assert!(matches!(items[0].kind, ProactiveItemKind::Correction { .. }));
    }
}
```

---

## 9. Metrics

| Metric | Target |
|---|---|
| Plugin evaluation latency (p99) | < 10ms |
| Proactive item relevance (acceptance rate) | > 60% |
| False positive rate | < 10% |
| Plugin load time (WASM) | < 50ms |
| Config change propagation | < 1s |

---

*The plugin system makes proactive behavior **composable, testable, and user-configurable** — teams can add domain-specific intelligence without touching CME core.*