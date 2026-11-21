# Conversation Engine Improvements

> Comprehensive improvements to the CME and proactive conversation designs based on architectural review.

---

## 1. Deterministic Turn Planning (CME Owns Structure)

### Problem
LLM controls chunking, pacing, and turn termination → rambling, inconsistent pacing, missed proactive injections.

### Solution: TurnPlan
CME constructs a **TurnPlan** before calling LLM. LLM only fills content slots.

```rust
struct TurnPlan {
    strategy: ResponseStrategy,
    chunks: Vec<PlannedChunk>,
    proactive_injection_points: Vec<usize>, // after chunk N
    max_duration_ms: u32,
    requires_user_input: bool,
}

struct PlannedChunk {
    role: ChunkRole,           // Acknowledge, Finding, Question, NextStep, Proactive
    directive: String,         // "Acknowledge the deploy failure in 1 sentence"
    max_sentences: u8,
    pause_after_ms: Option<u32>,
}

enum ResponseStrategy {
    Reactive { clarification_needed: bool },
    Proactive { item: ProactiveItem },
    Hybrid { reactive_chunks: usize, proactive_after: usize },
    Questionnaire { question_index: usize },
    Introduction { persona: PersonaProfile },
}
```

### Prompt Template for LLM
```
<turn_plan>
Chunk 1 (Acknowledge, max 1 sentence): "Acknowledge the deploy failure."
Chunk 2 (Finding, max 2 sentences): "State the root cause: migration timeout on add_index_users_email."
Chunk 3 (Proactive, max 2 sentences): "Inject suggestion: 'Want me to generate the rollback migration?'"
Chunk 4 (Question, max 1 sentence): "Ask: 'Rollback or retry?'"
</turn_plan>

Generate content for each chunk. Output format:
CHUNK_1: ...
CHUNK_2: ...
CHUNK_3: ...
CHUNK_4: ...
```

### Benefits
- Guaranteed pacing contracts (≤3 sentences/chunk, pauses at defined points)
- Proactive items injected at precise locations
- Eliminates "LLM ignores directives" failure mode
- Testable: TurnPlan can be validated before LLM call

---

## 2. Speculative Execution During ASR (Tier 2/3)

### Problem
Tier 2/3 latency: user finishes speaking → ASR finalizes → LLM starts → first token.

### Solution: Speculative Prefetch
While ASR streams partial transcripts, predict likely completions and pre-warm LLM.

```rust
struct SpeculativeExecutor {
    predictor: CompletionPredictor,        // lightweight model or heuristic
    llm_backend: LlmBackend,
    active_prefetches: HashMap<PrefetchId, PrefetchState>,
}

struct PrefetchState {
    predicted_completion: String,
    prompt: Prompt,
    kv_cache_handle: Option<KVCacheHandle>, // if backend supports
    status: PrefetchStatus,                 // Running, Ready, Discarded, Used
    created_at: Timestamp,
}

async fn on_asr_interim(partial: &str, session: &Session) {
    let predictions = predictor.predict(partial, session.context(), 3);
    for pred in predictions {
        let prompt = build_prompt(pred.text, session);
        let handle = llm_backend.prefetch(prompt).await;
        prefetches.insert(pred.id, PrefetchState {
            predicted_completion: pred.text,
            prompt,
            kv_cache_handle: Some(handle),
            status: PrefetchStatus::Running,
            created_at: now(),
        });
    }
}

async fn on_asr_final(final_text: &str, session: &Session) {
    // Find matching prefetch
    if let Some(prefetch) = prefetches.values_mut()
        .find(|p| similarity(p.predicted_completion, final_text) > 0.85)
    {
        prefetch.status = PrefetchStatus::Used;
        // Resume from KV cache → first token in ~50ms
        llm_backend.continue_from_cache(prefetch.kv_cache_handle.unwrap(), tx);
    } else {
        // Fallback: normal cold start
        llm_backend.stream_completion(build_prompt(final_text, session), tx);
    }
    // Discard other prefetches
}
```

### Prediction Strategies
| Strategy | Description | Accuracy |
|---|---|---|
| **N-gram + Context** | Complete from common patterns + recent context | ~70% |
| **Lightweight LM** | 50M parameter completion model | ~85% |
| **Heuristic** | "deploy failed" → "check logs, recent commits, health" | ~80% for domain-specific |

### Latency Impact
| Scenario | Baseline | With Speculation |
|---|---|---|
| Correct prediction | 800ms | **300ms** (50ms resume + 250ms overlap) |
| Wrong prediction | 800ms | 850ms (discard overhead) |
| **Weighted (80% accuracy)** | 800ms | **~400ms** |

---

## 3. Structured Conversation Compression

### Problem
Sliding window loses semantic structure; summarization loses queryable artifacts.

### Solution: CompressedConversation with Semantic Indexing

```rust
struct CompressedConversation {
    // Narrative summary (for LLM context)
    narrative: String,
    // "User debugged deploy failure on auth-service. Root cause: migration timeout 
    //  on add_index_users_email (commit 3f2a1). Fixed via PR #247. Discussed 
    //  caching strategy — user chose Redis layer, will implement separately."
    
    // Extractable artifacts (queryable)
    decisions: Vec<DecisionRecord>,
    commitments: Vec<Commitment>,
    open_questions: Vec<OpenQuestion>,
    artifacts: Vec<ArtifactRef>,
    
    // Semantic retrieval
    embedding: Vector,              // sentence-transformers embedding of narrative
    key_topics: Vec<String>,        // ["deploy", "migration", "caching", "auth-service"]
    time_range: (Timestamp, Timestamp),
    
    // Provenance
    source_turn_ids: Vec<TurnId>,
    compression_ratio: f32,         // original_tokens / compressed_tokens
}

struct DecisionRecord {
    id: DecisionId,
    topic: String,                  // "caching strategy"
    choice: String,                 // "Redis layer, separate PR"
    rationale: String,              // "Lower risk, incremental"
    alternatives_considered: Vec<String>,
    turn_ids: Vec<TurnId>,
    confidence: f32,
}

struct Commitment {
    id: CommitmentId,
    description: String,            // "Implement Redis caching for org lookup"
    owner: EntityId,                // user or agent
    deadline: Option<Timestamp>,
    status: CommitmentStatus,       // Open, InProgress, Done, Blocked
    related_turns: Vec<TurnId>,
}

struct ArtifactRef {
    kind: ArtifactKind,             // PullRequest, File, Config, Diagram, Test
    identifier: String,             // "PR #247", "src/cache/redis.rs"
    description: String,
    turn_ids: Vec<TurnId>,
}
```

### Compression Algorithm
```rust
async fn compress_conversation(session: &Session, turns: &[Turn]) -> CompressedConversation {
    // 1. Extract structured artifacts via LLM (one call per N turns)
    let extraction_prompt = format!(
        "Extract from this conversation: decisions, commitments, open questions, artifacts. 
         Output JSON schema: {schema}. Conversation: {turns_json}"
    );
    let extraction = llm.complete(extraction_prompt).await;
    
    // 2. Generate narrative summary
    let narrative_prompt = format!(
        "Write a 3-5 sentence narrative summary preserving causal chain. 
         Focus on: problem → investigation → root cause → decision → outcome.
         Conversation: {turns_json}"
    );
    let narrative = llm.complete(narrative_prompt).await;
    
    // 3. Compute embedding
    let embedding = embedder.embed(&narrative).await;
    
    // 4. Extract key topics (TF-IDF + LLM)
    let topics = extract_topics(&narrative, &extraction);
    
    CompressedConversation { ... }
}
```

### Retrieval Interface
```rust
impl ConversationStore {
    // Semantic search across all sessions
    async fn search(&self, query: &str, user_id: UserId, k: usize) -> Vec<CompressedConversation> {
        let q_embedding = embedder.embed(query).await;
        self.vector_index.search(q_embedding, k, filter: user_id).await
    }
    
    // Structured queries
    async fn find_decisions(&self, topic: &str, user_id: UserId) -> Vec<DecisionRecord> { ... }
    async fn find_commitments(&self, owner: EntityId, status: CommitmentStatus) -> Vec<Commitment> { ... }
    async fn find_artifacts(&self, kind: ArtifactKind, user_id: UserId) -> Vec<ArtifactRef> { ... }
}
```

### Use Cases
- "What did we decide about caching last month?" → `search("caching decision")` → DecisionRecord
- "Show me my open commitments" → `find_commitments(user, Open)` → list
- "Find the PR for the deploy fix" → `find_artifacts(PullRequest, "deploy")` → PR #247

---

## 4. Implicit Preference Learning

### Problem
Questionnaires capture explicit preferences but miss nuanced, evolving patterns.

### Solution: Continuous Preference Inference

```rust
struct PreferenceInferenceEngine {
    dimensions: Vec<PreferenceDimension>,
    evidence_buffer: VecDeque<PreferenceEvidence>,
    user_models: HashMap<UserId, UserPreferenceModel>,
}

#[derive(Clone)]
struct PreferenceDimension {
    id: String,                     // "verbosity", "technical_depth", "initiative", "formality"
    range: (f32, f32),              // (-1.0, 1.0)
    description: String,
    evidence_types: Vec<EvidenceType>,
}

#[derive(Clone)]
struct PreferenceEvidence {
    dimension: String,
    signal: PreferenceSignal,
    weight: f32,                    // 0.1 - 1.0
    turn_id: TurnId,
    timestamp: Timestamp,
}

enum PreferenceSignal {
    // Verbosity
    AcceptedShortReply,             // user didn't interrupt, continued
    InterruptedLongReply,           // user cut off agent
    ExplicitComplaint { text: String },
    
    // Technical Depth
    AskedForDetail { topic: String },
    AcceptedHighLevelSummary,
    ExplicitRequest { level: TechnicalLevel },
    
    // Initiative
    AcceptedProactiveSuggestion,
    RejectedProactiveSuggestion,
    AskedAgentToAct,
    
    // Formality
    UsedCasualLanguage,
    UsedFormalLanguage,
    CorrectedTone { expected: String },
}

struct UserPreferenceModel {
    user_id: UserId,
    preferences: HashMap<String, InferredPreference>,
    last_updated: Timestamp,
    total_evidence_count: u32,
}

struct InferredPreference {
    dimension: String,
    value: f32,                     // -1.0 to 1.0
    confidence: f32,                // 0.0 to 1.0
    evidence_count: u32,
    recent_trend: TrendDirection,   // Stable, Increasing, Decreasing
}
```

### Inference Algorithm (Bayesian Update)
```rust
fn update_preference(model: &mut UserPreferenceModel, evidence: &PreferenceEvidence) {
    let dim = &evidence.dimension;
    let prior = model.preferences.get(dim).cloned()
        .unwrap_or(InferredPreference { value: 0.0, confidence: 0.0, evidence_count: 0, ... });
    
    // Likelihood from signal
    let (signal_value, signal_strength) = signal_to_value(&evidence.signal);
    let weight = evidence.weight * signal_strength;
    
    // Bayesian update: posterior ∝ prior × likelihood
    let new_evidence_count = prior.evidence_count + 1;
    let new_value = (prior.value * prior.confidence * prior.evidence_count as f32 
                     + signal_value * weight) 
                    / (prior.confidence * prior.evidence_count as f32 + weight);
    let new_confidence = (prior.confidence * prior.evidence_count as f32 + weight)
                         / (prior.evidence_count as f32 + 1.0).min(1.0);
    
    model.preferences.insert(dim.clone(), InferredPreference {
        value: new_value.clamp(-1.0, 1.0),
        confidence: new_confidence,
        evidence_count: new_evidence_count,
        recent_trend: compute_trend(prior.value, new_value),
    });
    model.last_updated = now();
    model.total_evidence_count += 1;
}
```

### Signal Mapping Examples
| Signal | Dimension | Value | Weight |
|---|---|---|---|
| InterruptedLongReply | verbosity | -0.8 | 0.9 |
| AcceptedShortReply | verbosity | -0.3 | 0.4 |
| AskedForDetail("auth") | technical_depth | +0.6 | 0.8 |
| AcceptedProactiveSuggestion | initiative | +0.4 | 0.5 |
| RejectedProactiveSuggestion | initiative | -0.5 | 0.7 |
| UsedCasualLanguage | formality | -0.4 | 0.3 |

### Application in CME
```rust
fn apply_preferences(turn_plan: &mut TurnPlan, prefs: &UserPreferenceModel) {
    // Verbosity → chunk count, sentence limits
    if let Some(p) = prefs.preferences.get("verbosity") {
        if p.value < -0.5 && p.confidence > 0.6 {
            turn_plan.chunks.iter_mut().for_each(|c| c.max_sentences = 1);
            turn_plan.max_chunks = 2;
        }
    }
    
    // Technical depth → directive detail level
    if let Some(p) = prefs.preferences.get("technical_depth") {
        if p.value > 0.5 && p.confidence > 0.6 {
            turn_plan.chunks.iter_mut()
                .filter(|c| c.role == ChunkRole::Finding)
                .for_each(|c| c.directive.push_str(" Include technical details."));
        }
    }
    
    // Initiative → proactive frequency
    if let Some(p) = prefs.preferences.get("initiative") {
        config.proactive_frequency = 0.15 + 0.2 * (p.value + 1.0) / 2.0; // 0.15–0.35
    }
}
```

---

## 5. Multi-LLM Routing Per Task Type

### Problem
Single backend for all tasks → overpay for simple, underperform on complex.

### Solution: Task-Aware Router

```rust
enum TaskType {
    Coding,           // code gen, review, debug
    Reasoning,        // architecture, tradeoffs, planning
    Creative,         // brainstorming, naming, analogies
    QuickFact,        // lookup, syntax, API reference
    VoiceOptimized,   // low-latency, conversational
    Analysis,         // log analysis, error diagnosis
    Documentation,    // writing docs, comments
}

struct TaskRouter {
    backends: HashMap<ModelId, LlmBackend>,
    routing_table: HashMap<TaskType, ModelId>,
    fallback_chain: Vec<ModelId>,
    cost_tracker: CostTracker,
}

impl TaskRouter {
    fn classify_task(&self, user_input: &str, context: &Context) -> TaskType {
        // Lightweight classifier (can be LLM-based or heuristic)
        let classifier_prompt = format!(
            "Classify this request: {}\nContext: {}\nOutput one: {}",
            user_input,
            context.summary(),
            TaskType::variants().join(", ")
        );
        llm_fast.complete(classifier_prompt).await.parse()
    }
    
    fn route(&self, task: TaskType, session: &Session) -> &LlmBackend {
        let model_id = self.routing_table.get(&task)
            .or_else(|| self.fallback_chain.first())
            .expect("no backend configured");
        self.backends.get(model_id).expect("backend not registered")
    }
}
```

### Default Routing Table
| Task Type | Model | Rationale |
|---|---|---|
| Coding | Claude 3.5 Sonnet / GPT-4o | Best code quality |
| Reasoning | o1 / Claude 3 Opus | Complex multi-step reasoning |
| Creative | GPT-4o / Claude 3.5 Sonnet | Good creativity + coherence |
| QuickFact | Haiku / Local 7B / GPT-4o-mini | Fast, cheap, accurate for facts |
| VoiceOptimized | GPT-4o Realtime / Gemini Live | Native audio, low latency |
| Analysis | o1 / GPT-4o | Pattern recognition in logs/errors |
| Documentation | Claude 3.5 Sonnet | Best technical writing |

### Dynamic Adaptation
```rust
// Track per-task success metrics
struct TaskPerformance {
    task_type: TaskType,
    model_id: ModelId,
    success_rate: f32,
    avg_latency_ms: u32,
    avg_cost_usd: f32,
    user_satisfaction: f32,
}

// Periodically re-evaluate routing
fn optimize_routing(&mut self) {
    for task in TaskType::all() {
        let candidates = self.backends.keys()
            .filter(|m| m.supports_task(task))
            .collect_vec();
        
        let best = candidates.into_iter()
            .max_by_key(|m| self.performance_score(task, m))
            .unwrap();
        
        self.routing_table.insert(task, best);
    }
}

fn performance_score(&self, task: TaskType, model: &ModelId) -> f32 {
    let p = self.metrics.get(&(task, model.clone())).unwrap();
    // Weighted: success (40%), satisfaction (30%), latency (20%), cost (10%)
    p.success_rate * 0.4 + p.user_satisfaction * 0.3 
    + (1.0 - p.avg_latency_ms as f32 / 5000.0) * 0.2 
    + (1.0 - p.avg_cost_usd / 0.10) * 0.1
}
```

---

## 6. Proactive Plugin System

### Problem
Proactive behaviors hardcoded in CME → hard to extend, test, customize.

### Solution: Plugin Architecture

```rust
trait ProactivePlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn triggers(&self) -> Vec<ProactiveTrigger>;
    fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem>;
    fn on_feedback(&self, item_id: &str, feedback: ProactiveFeedback);
    fn config_schema(&self) -> serde_json::Value;
}

struct ProactiveContext {
    session: &Session,
    user_model: &UserModel,
    recent_turns: &[Turn],
    memory: &MemoryStore,
    tools: &ToolRegistry,
    time_since_last_proactive: Duration,
}

enum ProactiveTrigger {
    UserMessage { keywords: Vec<String> },
    TurnInterval { every_n: u32 },
    IdleTimeout { minutes: u32 },
    ToolResult { tool: String, outcome: Outcome },
    TopicFrequency { topic: String, threshold: u32 },
    Scheduled { cron: String },
    MemoryMatch { query: String, min_score: f32 },
    Custom(fn(&ProactiveContext) -> bool),
}

enum ProactiveFeedback {
    Accepted { acted_upon: bool },
    Rejected,
    Ignored,
    ExplicitRating(i8), // -2 to +2
}

// Built-in plugins
struct DeployMonitorPlugin;
struct CodeHealthPlugin;
struct SecurityScannerPlugin;
struct DependencyWatcherPlugin;
struct CostOptimizerPlugin;
struct TeamConventionPlugin;
```

### Plugin: Deploy Monitor Example
```rust
impl ProactivePlugin for DeployMonitorPlugin {
    fn name(&self) -> &'static str { "deploy_monitor" }
    fn triggers(&self) -> Vec<ProactiveTrigger> {
        vec![
            ProactiveTrigger::ToolResult { tool: "deploy".into(), outcome: Outcome::Failure },
            ProactiveTrigger::Scheduled { cron: "*/15 * * * *".into() }, // every 15 min
        ]
    }
    
    fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem> {
        let mut items = vec![];
        
        // Check recent deploy failures
        if let Some(failure) = ctx.recent_tool_results()
            .find(|r| r.tool == "deploy" && r.outcome == Outcome::Failure)
        {
            items.push(ProactiveItem::Investigation {
                title: "Deploy Failure Detected",
                summary: format!("{} failed: {}", failure.service, failure.error),
                suggested_actions: vec!["check_logs", "recent_commits", "rollback"],
                confidence: 0.9,
            });
        }
        
        // Periodic health check
        if matches!(ctx.trigger, ProactiveTrigger::Scheduled { .. }) {
            if let Ok(health) = check_all_deploys().await {
                if health.has_warnings() {
                    items.push(ProactiveItem::CheckIn {
                        message: format!("Deploy pipeline: {} warnings", health.warning_count),
                        details: health.warnings,
                    });
                }
            }
        }
        
        items
    }
    
    fn on_feedback(&self, item_id: &str, feedback: ProactiveFeedback) {
        // Learn: if user rejects deploy checks, reduce frequency
        match feedback {
            ProactiveFeedback::Rejected => self.increase_cooldown(item_id),
            ProactiveFeedback::Accepted { .. } => self.decrease_cooldown(item_id),
            _ => {}
        }
    }
}
```

### Plugin Registry & Loading
```rust
struct PluginRegistry {
    plugins: HashMap<String, Box<dyn ProactivePlugin>>,
    configs: HashMap<String, PluginConfig>,
}

struct PluginConfig {
    enabled: bool,
    cooldown_turns: u32,
    min_confidence: f32,
    custom_params: serde_json::Value,
}

impl CME {
    fn evaluate_proactive_queue(&mut self, ctx: ProactiveContext) {
        for plugin in self.registry.enabled_plugins() {
            let config = self.registry.configs.get(plugin.name()).unwrap();
            if !config.enabled { continue; }
            if ctx.time_since_last_proactive < config.cooldown_turns { continue; }
            
            let items = plugin.evaluate(&ctx);
            for item in items {
                if item.confidence() >= config.min_confidence {
                    self.proactive_queue.push(item);
                }
            }
        }
    }
}
```

### Dynamic Plugin Loading (WASM)
```rust
// Plugins compiled to WASM, loaded at runtime
async fn load_plugin(wasm_path: &Path, config: PluginConfig) -> Result<Box<dyn ProactivePlugin>> {
    let engine = wasmtime::Engine::default();
    let module = Module::from_file(&engine, wasm_path)?;
    let mut linker = Linker::new(&engine);
    // Link host functions: memory, tools, logging
    let instance = linker.instantiate(&mut store, &module)?;
    let plugin = PluginWrapper::new(instance);
    Ok(Box::new(plugin))
}
```

---

## 7. Voice Prosody Adaptation

### Problem
Flat TTS output doesn't match user's emotional state or conversation context.

### Solution: Prosody Controller

```rust
struct ProsodyController {
    affect_detector: AffectDetector,
    prosody_profiles: HashMap<UserAffect, ProsodyProfile>,
    user_baseline: UserProsodyBaseline,
}

#[derive(Clone)]
struct ProsodyProfile {
    rate_multiplier: f32,        // 0.5–2.0 (1.0 = normal)
    pitch_shift_semitones: f32,  // -12 to +12
    volume_db: f32,              // -10 to +10
    pause_scaling: f32,          // 0.5–2.0
    emphasis_style: EmphasisStyle,
    breathing_pattern: BreathingPattern,
}

#[derive(Clone)]
struct UserProsodyBaseline {
    avg_rate: f32,
    avg_pitch: f32,
    typical_pause_ms: u32,
    learned_from_turns: u32,
}

enum UserAffect {
    Neutral,
    Frustrated,      // slower, lower pitch, longer pauses
    Excited,         // faster, higher pitch, shorter pauses
    Uncertain,       // more hesitant, rising intonation
    Tired,           // slower, lower energy
    Focused,         // steady, precise, minimal filler
}

impl ProsodyController {
    fn compute_prosody(&self, text: &str, ctx: &ProsodyContext) -> ProsodyProfile {
        let affect = self.affect_detector.detect(ctx);
        let mut profile = self.prosody_profiles.get(&affect).cloned()
            .unwrap_or(ProsodyProfile::neutral());
        
        // Adapt to user baseline
        if ctx.user_baseline.learned_from_turns > 10 {
            profile.rate_multiplier *= ctx.user_baseline.avg_rate / profile.rate_multiplier;
            profile.pitch_shift_semitones += ctx.user_baseline.avg_pitch;
        }
        
        // Context adjustments
        if ctx.stakes > 0.7 { profile.rate_multiplier *= 0.9; } // slower for high stakes
        if ctx.is_correction { profile.emphasis_style = EmphasisStyle::Deliberate; }
        if ctx.is_joke { profile.pitch_shift_semitones += 2.0; }
        
        profile
    }
}

struct ProsodyContext {
    user_affect: UserAffect,
    user_baseline: UserProsodyBaseline,
    stakes: f32,
    is_correction: bool,
    is_joke: bool,
    conversation_phase: ConversationPhase,
}
```

### Affect Detection Signals
| Signal | Source | Weight |
|---|---|---|
| Interruption frequency | CME | 0.3 |
| Response latency | CME | 0.2 |
| Explicit language ("ugh", "great", "confused") | ASR transcript | 0.4 |
| Prosody of user speech (pitch, rate, energy) | Audio features | 0.5 |
| Explicit feedback ("too fast", "slow down") | User message | 0.8 |

### TTS Integration
```rust
// Tier 1/2: SSML generation
fn generate_ssml(text: &str, prosody: &ProsodyProfile) -> String {
    format!(
        r#"<speak><prosody rate="{:.0}%" pitch="{:+.1}st" volume="{:+.1}dB">
            {}
        </prosody></speak>"#,
        prosody.rate_multiplier * 100.0,
        prosody.pitch_shift_semitones,
        prosody.volume_db,
        inject_pauses(text, prosody.pause_scaling)
    )
}

// Tier 3: Provider-specific prosody control (OpenAI Realtime `voice` + `instructions`)
fn realtime_instructions(prosody: &ProsodyProfile) -> String {
    format!(
        "Speak at {}x normal rate. Pitch: {} semitones. {} pauses. {}",
        prosody.rate_multiplier,
        prosody.pitch_shift_semitones,
        if prosody.pause_scaling > 1.2 { "Longer" } else { "Normal" },
        match prosody.emphasis_style {
            EmphasisStyle::Deliberate => "Emphasize key words clearly.",
            EmphasisStyle::Casual => "Natural, conversational emphasis.",
            EmphasisStyle::Excited => "Energetic, expressive delivery.",
        }
    )
}
```

---

## 8. Synthetic Conversation Testing Framework

### Problem
Manual testing doesn't catch regressions in pacing, proactivity, interruption handling.

### Solution: Declarative Test Suite

```yaml
# tests/conversations/vague_goal_narrowing.yaml
test_case:
  name: "vague_goal_narrowing"
  description: "Agent narrows vague goal before planning"
  user_persona: "impatient_engineer"
  max_turns: 5
  success_criteria:
    - resolves_in_turns: 3
    - no_wall_of_text: true
    - asks_clarification: true
    - proposes_structured_options: true

  turns:
    - user: "Make the API faster"
      expect_agent:
        chunk_count: 2..3
        contains: ["Faster how?", "Latency", "Throughput", "Cold-start"]
        ends_with_question: true
    
    - user: "Latency"
      expect_agent:
        chunk_count: 2..3
        contains: ["profile first", "caching", "jump to"]
        ends_with_question: true
    
    - user: "Profile first"
      expect_agent:
        chunk_count: 1..2
        action: "run_profiler"
        ends_with: "results_ready"

# tests/conversations/interruption_handling.yaml
test_case:
  name: "interruption_mid_stream"
  description: "Agent handles user interruption gracefully"
  user_persona: "interruptive_user"
  
  turns:
    - user: "Explain the deployment architecture"
      expect_agent:
        chunk_count: 4
        streaming: true
    
    - user: "STOP. Just give me the diagram."
      timing: during_agent_chunk_2
      expect_agent:
        immediate_ack: true
        ack_max_sentences: 1
        no_apology: true
        next_action: "generate_diagram"
```

### Test Runner
```rust
struct ConversationTestRunner {
    cme: CME,
    personas: HashMap<String, UserPersona>,
    assertions: Vec<Assertion>,
}

struct UserPersona {
    name: String,
    response_latency_ms: Distribution,  // e.g., LogNormal(3000, 1.5)
    interruption_probability: f32,
    verbosity: VerbosityLevel,
    technical_level: TechnicalLevel,
    patience: PatienceLevel,
}

impl ConversationTestRunner {
    async fn run(&self, test: &TestCase) -> TestResult {
        let mut session = self.cme.create_session(test.user_persona).await;
        let mut turn = 0;
        
        for expected_turn in &test.turns {
            // Simulate user message
            let user_msg = self.simulate_user(&expected_turn.user, &session).await;
            let agent_response = session.process_message(user_msg).await;
            
            // If interruption specified
            if let Some(interrupt) = expected_turn.interrupt {
                let interrupt_msg = self.simulate_user(interrupt.message, &session).await;
                session.process_message(interrupt_msg).await;
            }
            
            // Assertions
            for assertion in &expected_turn.assertions {
                assertion.check(&agent_response, &session)?;
            }
            turn += 1;
        }
        
        // Global success criteria
        for criterion in &test.success_criteria {
            criterion.verify(&session)?;
        }
        
        TestResult::Passed
    }
}
```

### CI Integration
```yaml
# .github/workflows/conversation-tests.yml
name: Conversation Tests
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run conversation tests
        run: cargo test --test conversation_tests -- --nocapture
      - name: Generate report
        run: cargo test --test conversation_tests -- --format=json > results.json
      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: conversation-test-results
          path: results.json
```

---

## 9. Cross-Session Continuity

### Problem
Each session starts fresh; no memory of past projects, decisions, preferences.

### Solution: Project-Level Conversation Memory

```rust
struct ProjectConversationMemory {
    project_id: ProjectId,
    user_id: UserId,
    sessions: Vec<SessionSummary>,
    compressed: CompressedConversation,
    active_goals: Vec<Goal>,
    open_commitments: Vec<Commitment>,
    key_artifacts: Vec<ArtifactRef>,
    learned_preferences: UserPreferenceModel,
    updated_at: Timestamp,
}

struct SessionSummary {
    session_id: SessionId,
    started_at: Timestamp,
    duration_ms: u64,
    turn_count: u32,
    primary_topics: Vec<String>,
    outcome: SessionOutcome,
    key_decisions: Vec<DecisionId>,
}

impl ProjectConversationMemory {
    fn get_context_for_new_session(&self) -> NewSessionContext {
        NewSessionContext {
            project_summary: self.compressed.narrative,
            active_goals: self.active_goals.clone(),
            open_commitments: self.open_commitments.clone(),
            recent_artifacts: self.key_artifacts.iter().take(5).cloned().collect(),
            learned_preferences: self.learned_preferences.clone(),
            suggested_continuation: self.suggest_continuation(),
        }
    }
    
    fn suggest_continuation(&self) -> Option<String> {
        // "Last time we were implementing Redis caching for org lookup. 
        //  PR #247 merged. Next: cache invalidation strategy?"
        self.active_goals.first().map(|g| format!(
            "Last time: {}. Next: {}",
            g.recent_progress,
            g.next_step
        ))
    }
}
```

### Integration with CME Session Start
```rust
async fn on_session_start(cme: &CME, user_id: UserId, project_id: Option<ProjectId>) -> Session {
    let mut session = Session::new(user_id);
    
    if let Some(project_id) = project_id {
        if let Some(memory) = cme.project_memory.load(project_id).await {
            let context = memory.get_context_for_new_session();
            
            // Inject into first turn
            session.initial_context = Some(context);
            
            // Personalized greeting
            if let Some(cont) = context.suggested_continuation {
                session.proactive_queue.push(ProactiveItem::Continuation {
                    message: format!("Back at it. {}", cont),
                });
            } else {
                session.proactive_queue.push(ProactiveItem::Greeting {
                    message: "Back at it. Where were we?".into(),
                });
            }
        }
    }
    
    session
}
```

---

## 10. Implementation Priority Matrix

| Improvement | Effort | Impact | Dependencies | Phase |
|---|---|---|---|---|
| Deterministic Turn Planning | Medium | High | None | 1 |
| Speculative Execution | Medium | High | Tier 2/3 ASR streaming | 2 |
| Structured Compression | High | High | Vector store, embedding model | 3 |
| Implicit Preference Learning | Medium | High | Evidence collection pipeline | 2 |
| Multi-LLM Routing | Low | Medium | Multiple backend contracts | 1 |
| Proactive Plugin System | Medium | High | Plugin registry, WASM runtime | 3 |
| Voice Prosody Adaptation | Medium | Medium | Tier 3, affect detector | 4 |
| Synthetic Testing Framework | Medium | High | Test runner, personas | 2 |
| Cross-Session Continuity | High | High | Project memory store | 3 |

---

## 11. New Design Documents to Create

| Document | Scope |
|---|---|
| `turn_planning.md` | Deterministic TurnPlan spec, prompt templates, validation |
| `speculative_execution.md` | ASR-LLM overlap, prediction models, KV cache reuse |
| `conversation_compression.md` | CompressedConversation schema, extraction prompts, retrieval |
| `preference_learning.md` | Bayesian inference, evidence types, application in CME |
| `multi_llm_routing.md` | Task classification, routing table, performance optimization |
| `proactive_plugin_system.md` | Plugin trait, built-in plugins, WASM loading, config |
| `voice_prosody.md` | Affect detection, prosody profiles, SSML/realtime integration |
| `synthetic_testing.md` | Test DSL, runner, CI integration, persona library |
| `cross_session_memory.md` | ProjectConversationMemory, continuation suggestions, privacy |

---

*These improvements transform the CME from a chat wrapper into a **cognitive conversation controller** — structured, learnable, testable, and extensible.*