# Multi-LLM Routing

> Task-aware model selection for quality, latency, and cost optimization.

---

## 1. Problem

Single backend for all tasks:
- Overpay for simple queries (Haiku could handle)
- Underperform on complex reasoning (need o1/Claude Opus)
- No voice-optimized path
- No fallback chain

---

## 2. Task Classification

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum TaskType {
    // Coding tasks
    CodeGeneration,       // "write a function that..."
    CodeReview,           // "review this PR"
    Debugging,            // "why is this failing?"
    Refactoring,          // "refactor this module"
    
    // Reasoning tasks
    ArchitectureDesign,   // "design a system for..."
    TradeoffAnalysis,     // "compare X vs Y"
    Planning,             // "plan the migration"
    RootCauseAnalysis,    // "why did deploy fail?"
    
    // Creative/Exploratory
    Brainstorming,        // "ideas for..."
    AnalogyMapping,       // "what's this like?"
    Naming,               // "name this service"
    
    // Quick facts
    SyntaxLookup,         // "rust hashmap syntax"
    APIReference,         // "aws sdk method for..."
    FactRetrieval,        // "what's the default port?"
    CommandLookup,        // "kubectl get pods syntax"
    
    // Voice-optimized
    VoiceConversation,    // General chat
    VoiceCommand,         // "deploy the service"
    VoiceQuestion,        // "what's the status?"
    
    // Analysis
    LogAnalysis,          // "analyze these logs"
    ErrorDiagnosis,       // "diagnose this error"
    PerformanceAnalysis,  // "why is this slow?"
    
    // Documentation
    DocWriting,           // "write README for..."
    CommentGeneration,    // "add comments to..."
    ChangelogGeneration,  // "generate changelog"
}

impl TaskType {
    fn all() -> Vec<TaskType> { ... }
    fn category(&self) -> TaskCategory { ... }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskCategory {
    Coding,
    Reasoning,
    Creative,
    QuickFact,
    Voice,
    Analysis,
    Documentation,
}
```

---

## 3. Model Registry

```rust
struct ModelRegistry {
    models: HashMap<ModelId, ModelSpec>,
    routing_table: HashMap<TaskType, ModelId>,
    fallback_chains: HashMap<TaskType, Vec<ModelId>>,
    performance_tracker: PerformanceTracker,
}

struct ModelSpec {
    id: ModelId,
    provider: Provider,
    model_name: String,
    capabilities: ModelCapabilities,
    pricing: Pricing,
    limits: ModelLimits,
}

#[derive(Clone)]
struct ModelCapabilities {
    supports_streaming: bool,
    supports_tools: bool,
    supports_vision: bool,
    supports_audio: bool,
    max_context_tokens: usize,
    max_output_tokens: usize,
    supported_languages: Vec<String>,
    specialties: Vec<TaskCategory>,  // Where this model excels
}

struct Pricing {
    input_per_1k: f64,
    output_per_1k: f64,
    cache_read_per_1k: Option<f64>,
}

struct ModelLimits {
    rpm: u32,
    tpm: u32,
    max_concurrent: u32,
}
```

---

## 4. Default Model Catalog

```rust
fn default_catalog() -> Vec<ModelSpec> {
    vec![
        // === REASONING / CODING (High quality) ===
        ModelSpec {
            id: "claude-3.5-sonnet",
            provider: Provider::Anthropic,
            model_name: "claude-3-5-sonnet-20241022",
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_tools: true,
                supports_vision: true,
                supports_audio: false,
                max_context_tokens: 200_000,
                max_output_tokens: 8192,
                supported_languages: vec!["en".into(), "code".into()],
                specialties: vec![TaskCategory::Coding, TaskCategory::Reasoning, TaskCategory::Documentation],
            },
            pricing: Pricing { input_per_1k: 0.003, output_per_1k: 0.015, cache_read_per_1k: Some(0.0003) },
            limits: ModelLimits { rpm: 1000, tpm: 100_000, max_concurrent: 50 },
        },
        
        ModelSpec {
            id: "gpt-4o",
            provider: Provider::OpenAI,
            model_name: "gpt-4o-2024-08-06",
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_tools: true,
                supports_vision: true,
                supports_audio: true,  // Realtime
                max_context_tokens: 128_000,
                max_output_tokens: 16_384,
                supported_languages: vec!["en".into(), "code".into(), "multi".into()],
                specialties: vec![TaskCategory::Coding, TaskCategory::Reasoning, TaskCategory::Creative, TaskCategory::Voice],
            },
            pricing: Pricing { input_per_1k: 0.0025, output_per_1k: 0.01, cache_read_per_1k: Some(0.00125) },
            limits: ModelLimits { rpm: 500, tpm: 30_000, max_concurrent: 20 },
        },
        
        ModelSpec {
            id: "o1-preview",
            provider: Provider::OpenAI,
            model_name: "o1-preview",
            capabilities: ModelCapabilities {
                supports_streaming: false,
                supports_tools: false,
                supports_vision: false,
                supports_audio: false,
                max_context_tokens: 128_000,
                max_output_tokens: 32_768,
                supported_languages: vec!["en".into(), "code".into()],
                specialties: vec![TaskCategory::Reasoning, TaskCategory::Analysis],
            },
            pricing: Pricing { input_per_1k: 0.015, output_per_1k: 0.06, cache_read_per_1k: None },
            limits: ModelLimits { rpm: 20, tpm: 5_000, max_concurrent: 5 },
        },
        
        // === FAST / CHEAP (High volume) ===
        ModelSpec {
            id: "claude-3.5-haiku",
            provider: Provider::Anthropic,
            model_name: "claude-3-5-haiku-20241022",
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_tools: true,
                supports_vision: true,
                supports_audio: false,
                max_context_tokens: 200_000,
                max_output_tokens: 8192,
                supported_languages: vec!["en".into(), "code".into()],
                specialties: vec![TaskCategory::QuickFact, TaskCategory::Coding],
            },
            pricing: Pricing { input_per_1k: 0.00025, output_per_1k: 0.00125, cache_read_per_1k: Some(0.000025) },
            limits: ModelLimits { rpm: 2000, tpm: 200_000, max_concurrent: 100 },
        },
        
        ModelSpec {
            id: "gpt-4o-mini",
            provider: Provider::OpenAI,
            model_name: "gpt-4o-mini",
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_tools: true,
                supports_vision: true,
                supports_audio: false,
                max_context_tokens: 128_000,
                max_output_tokens: 16_384,
                supported_languages: vec!["en".into(), "code".into(), "multi".into()],
                specialties: vec![TaskCategory::QuickFact, TaskCategory::Analysis],
            },
            pricing: Pricing { input_per_1k: 0.00015, output_per_1k: 0.0006, cache_read_per_1k: Some(0.000075) },
            limits: ModelLimits { rpm: 2000, tpm: 100_000, max_concurrent: 50 },
        },
        
        // === LOCAL MODELS (Privacy, zero marginal cost) ===
        ModelSpec {
            id: "qwen2.5-coder-32b",
            provider: Provider::Local,
            model_name: "qwen2.5-coder-32b-instruct",
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_tools: true,
                supports_vision: false,
                supports_audio: false,
                max_context_tokens: 32_768,
                max_output_tokens: 8192,
                supported_languages: vec!["code".into()],
                specialties: vec![TaskCategory::Coding, TaskCategory::QuickFact],
            },
            pricing: Pricing { input_per_1k: 0.0, output_per_1k: 0.0, cache_read_per_1k: None },
            limits: ModelLimits { rpm: 60, tpm: 10_000, max_concurrent: 4 },
        },
        
        ModelSpec {
            id: "llama-3.1-70b",
            provider: Provider::Local,
            model_name: "llama-3.1-70b-instruct",
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_tools: true,
                supports_vision: false,
                supports_audio: false,
                max_context_tokens: 131_072,
                max_output_tokens: 8192,
                supported_languages: vec!["en".into(), "code".into()],
                specialties: vec![TaskCategory::Reasoning, TaskCategory::Analysis, TaskCategory::Documentation],
            },
            pricing: Pricing { input_per_1k: 0.0, output_per_1k: 0.0, cache_read_per_1k: None },
            limits: ModelLimits { rpm: 30, tpm: 5_000, max_concurrent: 2 },
        },
        
        // === VOICE-NATIVE ===
        ModelSpec {
            id: "gpt-4o-realtime",
            provider: Provider::OpenAI,
            model_name: "gpt-4o-realtime-preview-2024-10-01",
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_tools: true,
                supports_vision: false,
                supports_audio: true,
                max_context_tokens: 128_000,
                max_output_tokens: 16_384,
                supported_languages: vec!["en".into(), "multi".into()],
                specialties: vec![TaskCategory::Voice],
            },
            pricing: Pricing { input_per_1k: 0.005, output_per_1k: 0.02, cache_read_per_1k: None },  // Audio pricing different
            limits: ModelLimits { rpm: 100, tpm: 10_000, max_concurrent: 10 },
        },
    ]
}
```

---

## 5. Task Classifier

```rust
struct TaskClassifier {
    fast_llm: Arc<dyn LlmBackend>,      // Haiku / 4o-mini for classification
    classifier_prompt: String,
    cache: LruCache<String, TaskType>,  // Cache recent classifications
}

impl TaskClassifier {
    async fn classify(&self, user_input: &str, context: &ClassificationContext) -> TaskType {
        // Check cache
        let cache_key = format!("{}:{}", context.conversation_hash(), hash_input(user_input));
        if let Some(cached) = self.cache.get(&cache_key) {
            return *cached;
        }
        
        let prompt = self.build_prompt(user_input, context);
        let response = self.fast_llm.complete(prompt).await;
        let task_type = self.parse_response(&response);
        
        self.cache.insert(cache_key, task_type);
        task_type
    }
    
    fn build_prompt(&self, input: &str, context: &ClassificationContext) -> String {
        format!(
            r#"Classify this user request into ONE task type.

User: "{}"

Context:
- Recent topics: {}
- Active goal: {}
- Conversation mode: {}
- User role: {}

Task types:
{}

Output ONLY the task type name (e.g., "CodeGeneration", "Debugging", "QuickFact")."#,
            input,
            context.recent_topics.join(", "),
            context.active_goal.unwrap_or("none".into()),
            context.mode,
            context.user_role,
            TaskType::all().iter().map(|t| format!("- {}", t)).collect::<Vec<_>>().join("\n")
        )
    }
}

struct ClassificationContext {
    recent_topics: Vec<String>,
    active_goal: Option<String>,
    mode: ConversationMode,
    user_role: String,
    conversation_hash: u64,
}

enum ConversationMode {
    Text,
    VoiceTier1,
    VoiceTier2,
    VoiceTier3,
}
```

---

## 6. Router with Fallback

```rust
struct TaskRouter {
    registry: ModelRegistry,
    classifier: TaskClassifier,
    session_preferences: HashMap<SessionId, SessionRoutingPrefs>,
}

struct SessionRoutingPrefs {
    preferred_models: HashMap<TaskCategory, ModelId>,
    cost_sensitivity: f32,           // 0.0 = ignore cost, 1.0 = minimize cost
    latency_sensitivity: f32,        // 0.0 = ignore latency, 1.0 = minimize latency
    privacy_mode: bool,              // Prefer local models
}

impl TaskRouter {
    fn route(&self, task: TaskType, session_id: SessionId) -> RoutingResult {
        let prefs = self.session_preferences.get(&session_id)
            .cloned()
            .unwrap_or_default();
        
        let category = task.category();
        
        // 1. Check session preference
        if let Some(model_id) = prefs.preferred_models.get(&category) {
            if let Some(model) = self.registry.models.get(model_id) {
                if self.is_available(model, &prefs) {
                    return RoutingResult::Direct(model_id.clone());
                }
            }
        }
        
        // 2. Check routing table
        if let Some(model_id) = self.registry.routing_table.get(&task) {
            if let Some(model) = self.registry.models.get(model_id) {
                if self.is_available(model, &prefs) {
                    return RoutingResult::Direct(model_id.clone());
                }
            }
        }
        
        // 3. Fallback chain
        if let Some(chain) = self.registry.fallback_chains.get(&task) {
            for model_id in chain {
                if let Some(model) = self.registry.models.get(model_id) {
                    if self.is_available(model, &prefs) {
                        return RoutingResult::Fallback(model_id.clone());
                    }
                }
            }
        }
        
        // 4. Global default
        RoutingResult::Fallback(self.registry.default_model.clone())
    }
    
    fn is_available(&self, model: &ModelSpec, prefs: &SessionRoutingPrefs) -> bool {
        // Privacy mode → prefer local
        if prefs.privacy_mode && model.provider != Provider::Local {
            return false;
        }
        
        // Cost sensitivity
        if prefs.cost_sensitivity > 0.7 && model.pricing.output_per_1k > 0.01 {
            return false;
        }
        
        // Latency sensitivity
        if prefs.latency_sensitivity > 0.7 && !model.capabilities.supports_streaming {
            return false;
        }
        
        // Rate limits
        if self.is_rate_limited(&model.id) {
            return false;
        }
        
        true
    }
}

enum RoutingResult {
    Direct(ModelId),
    Fallback(ModelId),
}
```

---

## 7. Default Routing Table

```rust
fn default_routing_table() -> HashMap<TaskType, ModelId> {
    let mut table = HashMap::new();
    
    // Coding
    table.insert(TaskType::CodeGeneration, "claude-3.5-sonnet");
    table.insert(TaskType::CodeReview, "claude-3.5-sonnet");
    table.insert(TaskType::Debugging, "gpt-4o");
    table.insert(TaskType::Refactoring, "claude-3.5-sonnet");
    
    // Reasoning
    table.insert(TaskType::ArchitectureDesign, "claude-3.5-sonnet");
    table.insert(TaskType::TradeoffAnalysis, "o1-preview");
    table.insert(TaskType::Planning, "gpt-4o");
    table.insert(TaskType::RootCauseAnalysis, "o1-preview");
    
    // Creative
    table.insert(TaskType::Brainstorming, "gpt-4o");
    table.insert(TaskType::AnalogyMapping, "claude-3.5-sonnet");
    table.insert(TaskType::Naming, "gpt-4o-mini");
    
    // Quick facts
    table.insert(TaskType::SyntaxLookup, "claude-3.5-haiku");
    table.insert(TaskType::APIReference, "gpt-4o-mini");
    table.insert(TaskType::FactRetrieval, "claude-3.5-haiku");
    table.insert(TaskType::CommandLookup, "gpt-4o-mini");
    
    // Voice
    table.insert(TaskType::VoiceConversation, "gpt-4o-realtime");
    table.insert(TaskType::VoiceCommand, "gpt-4o-realtime");
    table.insert(TaskType::VoiceQuestion, "gpt-4o-realtime");
    
    // Analysis
    table.insert(TaskType::LogAnalysis, "gpt-4o");
    table.insert(TaskType::ErrorDiagnosis, "o1-preview");
    table.insert(TaskType::PerformanceAnalysis, "o1-preview");
    
    // Documentation
    table.insert(TaskType::DocWriting, "claude-3.5-sonnet");
    table.insert(TaskType::CommentGeneration, "gpt-4o-mini");
    table.insert(TaskType::ChangelogGeneration, "gpt-4o-mini");
    
    table
}
```

### 7.1 Fallback Chains

```rust
fn default_fallback_chains() -> HashMap<TaskType, Vec<ModelId>> {
    let mut chains = HashMap::new();
    
    // High-quality coding → fast coding → local
    chains.insert(TaskType::CodeGeneration, vec![
        "claude-3.5-sonnet",
        "gpt-4o",
        "claude-3.5-haiku",
        "gpt-4o-mini",
        "qwen2.5-coder-32b",
    ]);
    
    // Reasoning → fast reasoning → local
    chains.insert(TaskType::TradeoffAnalysis, vec![
        "o1-preview",
        "claude-3.5-sonnet",
        "gpt-4o",
        "llama-3.1-70b",
    ]);
    
    // Quick facts → faster → local
    chains.insert(TaskType::SyntaxLookup, vec![
        "claude-3.5-haiku",
        "gpt-4o-mini",
        "qwen2.5-coder-32b",
    ]);
    
    // Voice → streaming text → local TTS
    chains.insert(TaskType::VoiceConversation, vec![
        "gpt-4o-realtime",
        "gpt-4o",  // Tier 2 fallback
        "claude-3.5-sonnet",  // Tier 1 fallback
    ]);
    
    chains
}
```

---

## 8. Performance Tracking & Auto-Optimization

```rust
struct PerformanceTracker {
    metrics: DashMap<(TaskType, ModelId), TaskPerformance>,
    optimization_interval: Duration,
}

struct TaskPerformance {
    task_type: TaskType,
    model_id: ModelId,
    total_calls: u64,
    success_count: u64,
    error_count: u64,
    total_latency_ms: u64,
    total_tokens_in: u64,
    total_tokens_out: u64,
    total_cost_usd: f64,
    user_satisfaction_sum: f32,  // From explicit + implicit feedback
    user_satisfaction_count: u32,
    last_updated: Timestamp,
}

impl PerformanceTracker {
    fn record_call(&self, task: TaskType, model: ModelId, result: CallResult) {
        self.metrics.entry((task, model)).and_modify(|p| {
            p.total_calls += 1;
            if result.success { p.success_count += 1; } else { p.error_count += 1; }
            p.total_latency_ms += result.latency_ms;
            p.total_tokens_in += result.tokens_in;
            p.total_tokens_out += result.tokens_out;
            p.total_cost_usd += result.cost_usd;
            if let Some(sat) = result.user_satisfaction {
                p.user_satisfaction_sum += sat;
                p.user_satisfaction_count += 1;
            }
            p.last_updated = now();
        }).or_insert(TaskPerformance { ... });
    }
    
    fn performance_score(&self, task: TaskType, model: ModelId) -> f32 {
        let p = self.metrics.get(&(task, model)).unwrap();
        
        let success_rate = p.success_count as f32 / p.total_calls as f32;
        let avg_latency = p.total_latency_ms as f32 / p.total_calls as f32;
        let avg_cost = p.total_cost_usd / p.total_calls as f64;
        let satisfaction = if p.user_satisfaction_count > 0 {
            p.user_satisfaction_sum / p.user_satisfaction_count as f32
        } else { 0.5 };
        
        // Weighted score: success (40%), satisfaction (30%), latency (20%), cost (10%)
        success_rate * 0.4 
        + satisfaction * 0.3 
        + (1.0 - (avg_latency / 5000.0).min(1.0)) * 0.2 
        + (1.0 - (avg_cost / 0.10).min(1.0) as f32) * 0.1
    }
}

impl TaskRouter {
    async fn optimize_routing(&mut self) {
        for task in TaskType::all() {
            let candidates: Vec<ModelId> = self.registry.models.keys()
                .filter(|m| m.supports_task(task))
                .cloned()
                .collect();
            
            if candidates.is_empty() { continue; }
            
            let best = candidates.into_iter()
                .max_by_key(|m| self.tracker.performance_score(task, m))
                .unwrap();
            
            self.registry.routing_table.insert(task, best);
        }
    }
}
```

---

## 9. Session-Level Preferences

```rust
// User can override per session
struct SessionRoutingPrefs {
    preferred_models: HashMap<TaskCategory, ModelId>,
    cost_sensitivity: f32,
    latency_sensitivity: f32,
    privacy_mode: bool,
    max_cost_per_turn_usd: Option<f64>,
}

impl Default for SessionRoutingPrefs {
    fn default() -> Self {
        Self {
            preferred_models: HashMap::new(),
            cost_sensitivity: 0.3,
            latency_sensitivity: 0.3,
            privacy_mode: false,
            max_cost_per_turn_usd: None,
        }
    }
}

// User commands that affect routing
fn parse_routing_command(input: &str, prefs: &mut SessionRoutingPrefs) -> Option<String> {
    let lower = input.to_lowercase();
    
    if lower.starts_with("use ") {
        let model = lower.trim_start_matches("use ").trim();
        if let Some(id) = resolve_model_alias(model) {
            prefs.preferred_models.insert(TaskCategory::Coding, id);
            return Some(format!("Using {} for coding tasks", id));
        }
    }
    
    if lower.contains("cheap") || lower.contains("cost") {
        prefs.cost_sensitivity = (prefs.cost_sensitivity + 0.2).min(1.0);
        return Some("Increased cost sensitivity".into());
    }
    
    if lower.contains("fast") || lower.contains("speed") {
        prefs.latency_sensitivity = (prefs.latency_sensitivity + 0.2).min(1.0);
        return Some("Increased latency sensitivity".into());
    }
    
    if lower.contains("private") || lower.contains("local") {
        prefs.privacy_mode = true;
        return Some("Privacy mode enabled — preferring local models".into());
    }
    
    None
}
```

---

## 10. Configuration

```toml
[multi_llm_routing]
enabled = true

# Classification
classifier_model = "gpt-4o-mini"       # Fast, cheap classifier
classification_cache_ttl_secs = 300
classification_cache_size = 1000

# Routing
default_routing_table = "config/routing_table.yaml"
fallback_chains = "config/fallback_chains.yaml"
optimize_interval_minutes = 30
min_calls_before_optimize = 50

# Performance tracking
track_user_satisfaction = true
satisfaction_weight_explicit = 1.0
satisfaction_weight_implicit = 0.3

# Session defaults
default_cost_sensitivity = 0.3
default_latency_sensitivity = 0.3
default_privacy_mode = false

# Rate limiting
global_rpm_limit = 1000
per_model_rpm = { "gpt-4o" = 500, "claude-3.5-sonnet" = 1000, "local" = 60 }

# Fallback
max_fallback_depth = 3
fallback_timeout_ms = 5000
```

---

## 11. Metrics

| Metric | Target |
|---|---|
| Classification accuracy | > 90% |
| Routing latency (p99) | < 50ms |
| Fallback rate | < 5% |
| Cost reduction vs single-model | > 40% |
| Latency improvement vs single-model | > 30% |
| User satisfaction (routed) | ≥ baseline |
| Local model utilization (privacy mode) | > 80% |

---

## 12. Example Routing Decisions

| User Input | Classified Task | Routed Model | Reason |
|---|---|---|---|
| "Write a Rust async HTTP client" | CodeGeneration | claude-3.5-sonnet | Best coding quality |
| "Why is my deploy failing?" | RootCauseAnalysis | o1-preview | Complex reasoning |
| "What's the kubectl syntax for logs?" | CommandLookup | gpt-4o-mini | Cheap, fast, accurate |
| "Compare Redis vs Memcached" | TradeoffAnalysis | o1-preview | Multi-factor reasoning |
| "Ideas for onboarding flow" | Brainstorming | gpt-4o | Creative |
| "Debug this stack trace" | Debugging | gpt-4o | Good at error analysis |
| "Plan the migration" | Planning | gpt-4o | Structured planning |
| (Voice) "Deploy the service" | VoiceCommand | gpt-4o-realtime | Native audio + tools |
| "Write doc for API" | DocWriting | claude-3.5-sonnet | Best technical writing |
| "Analyze these logs for errors" | LogAnalysis | gpt-4o | Pattern recognition |

---

*Multi-LLM routing makes the agent **model-agnostic** — each task gets the right tool, automatically optimizing for quality, speed, and cost.*