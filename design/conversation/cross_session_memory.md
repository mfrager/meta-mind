# Cross-Session Memory & Continuity

> **⚠️ SUPERSEDED**: This design has been merged into the **[Unified Memory System](unified_memory_system.md)**. The 4-tier hierarchy (Episodic → Compressed Segments → Project Memory → User Memory), compression pipeline, session continuity, multi-device sync, cross-project learning, and privacy controls are now defined in the unified design. This file is kept for historical reference only.

> Persistent conversation memory across sessions, projects, and time — enabling true continuity.

---

## 1. Problem

Current systems treat each session as isolated:
- No memory of past projects, decisions, or preferences
- "Who are you?" every session
- Lost context on disconnect/reconnect
- No cross-project learning

---

## 2. Memory Hierarchy

```
┌─────────────────────────────────────────────────────────────────┐
│                    MEMORY HIERARCHY                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ EPISODIC (Session-level)                                │   │
│  │ • Raw turns, tool results, user messages                │   │
│  │ • TTL: 30 days → compressed                            │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                 │
│                              ▼                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ COMPRESSED SEGMENTS (Project-level)                     │   │
│  │ • Narrative + structured artifacts per 2k tokens        │   │
│  │ • TTL: 1 year → aggregated                             │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                 │
│                              ▼                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ PROJECT MEMORY (Persistent)                             │   │
│  │ • Aggregated decisions, commitments, artifacts          │   │
│  │ • Learned preferences, conventions                      │   │
│  │ • Active goals, open questions                          │   │
│  │ • TTL: Indefinite                                       │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                 │
│                              ▼                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ USER MEMORY (Cross-project)                             │   │
│  │ • Global preferences, identity                          │   │
│  │ • Relationship model, communication style               │   │
│  │ • Cross-project patterns, expertise                     │   │
│  │ • TTL: Indefinite                                       │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. Core Data Structures

### 3.1 Session Memory (Ephemeral → Compressed)

```rust
struct SessionMemory {
    session_id: SessionId,
    project_id: Option<ProjectId>,
    user_id: UserId,
    turns: Vec<Turn>,
    tool_results: Vec<ToolResult>,
    proactive_items: Vec<ProactiveItem>,
    created_at: Timestamp,
    last_activity: Timestamp,
    turn_count: u32,
    status: SessionStatus,
}

enum SessionStatus {
    Active,
    Compressing,
    Compressed,
    Archived,
}
```

### 3.2 Compressed Segment (Structured)

```rust
struct CompressedSegment {
    segment_id: SegmentId,
    session_id: SessionId,
    project_id: Option<ProjectId>,
    turn_range: (TurnId, TurnId),
    token_count: usize,
    
    // Narrative summary (for LLM context)
    narrative: String,
    // "User debugged deploy failure on auth-service. 
    //  Root cause: migration timeout on add_index_users_email (commit 3f2a1). 
    //  Fixed via PR #247. Discussed caching strategy — user chose Redis layer, 
    //  will implement separately. Open: cache invalidation strategy."
    
    // Structured artifacts (queryable)
    decisions: Vec<DecisionRecord>,
    commitments: Vec<Commitment>,
    open_questions: Vec<OpenQuestion>,
    artifacts: Vec<ArtifactRef>,
    goals: Vec<GoalProgress>,
    
    // Semantic retrieval
    embedding: Vector,                    // 384-dim sentence-transformers
    key_topics: Vec<String>,              // ["deploy", "migration", "caching"]
    topic_weights: HashMap<String, f32>,
    
    // Quality
    compression_ratio: f32,
    decision_recall: f32,
    commitment_recall: f32,
    artifact_recall: f32,
    
    created_at: Timestamp,
}
```

### 3.3 Project Memory (Aggregated)

```rust
struct ProjectMemory {
    project_id: ProjectId,
    user_id: UserId,
    name: String,
    description: Option<String>,
    
    // Aggregated views (incrementally updated)
    aggregated_decisions: Vec<DecisionRecord>,
    active_commitments: Vec<Commitment>,
    open_questions: Vec<OpenQuestion>,
    key_artifacts: Vec<ArtifactRef>,
    active_goals: Vec<GoalProgress>,
    
    // Learned project-specific preferences
    learned_conventions: Vec<ConventionRule>,
    tech_stack: TechStack,
    deployment_targets: Vec<DeploymentTarget>,
    
    // Semantic index
    segment_ids: Vec<SegmentId>,
    topic_index: HashMap<String, Vec<SegmentId>>,
    
    // Continuity
    last_session_id: Option<SessionId>,
    suggested_continuation: Option<String>,
    project_narrative: String,
    
    created_at: Timestamp,
    updated_at: Timestamp,
}
```

### 3.4 User Memory (Cross-Project)

```rust
struct UserMemory {
    user_id: UserId,
    identity: UserIdentity,
    
    // Global preferences (learned + explicit)
    preferences: UserPreferenceModel,
    communication_style: CommunicationStyle,
    
    // Relationship & social
    relationship: RelationshipState,
    known_facts: HashMap<String, Fact>,
    
    // Cross-project learning
    expertise_domains: HashMap<String, ExpertiseLevel>,
    communication_patterns: CommunicationPatterns,
    preferred_tools: Vec<ToolPreference>,
    
    // Project registry
    projects: HashMap<ProjectId, ProjectSummary>,
    
    // Global continuity
    global_narrative: String,           // "Senior backend engineer, focuses on Rust/Go, 
                                        //  prefers terse communication, 
                                        //  currently working on auth-service migration"
    
    created_at: Timestamp,
    updated_at: Timestamp,
}
```

---

## 4. Memory Lifecycle

### 4.1 Compression Pipeline

```rust
struct MemoryCompressor {
    llm: Arc<dyn LlmBackend>,
    embedder: Arc<dyn Embedder>,
    config: CompressionConfig,
}

impl MemoryCompressor {
    // Triggered when session reaches token threshold or ends
    async fn compress_session(&self, session: &mut SessionMemory) -> Vec<CompressedSegment> {
        let turns = &session.turns;
        let mut segments = Vec::new();
        
        // Chunk into ~2000 token segments with overlap
        let chunks = self.chunk_turns(turns, self.config.segment_tokens, self.config.overlap_turns);
        
        for chunk in chunks {
            let segment = self.compress_chunk(chunk).await?;
            segments.push(segment);
        }
        
        // Update session status
        session.status = SessionStatus::Compressed;
        session.turns.clear(); // Free memory
        session.turns.shrink_to_fit();
        
        segments
    }
    
    async fn compress_chunk(&self, turns: &[Turn]) -> Result<CompressedSegment> {
        // Stage 1: Structured extraction
        let extraction = self.extract_structured(turns).await?;
        
        // Stage 2: Narrative generation
        let narrative = self.generate_narrative(turns, &extraction).await?;
        
        // Stage 3: Embedding + topics
        let embedding = self.embedder.embed(&narrative).await?;
        let topics = self.extract_topics(&narrative, &extraction);
        
        // Stage 4: Quality validation (sample)
        let quality = if self.config.validate_sample {
            Some(self.validate_quality(turns, &extraction).await?)
        } else { None };
        
        Ok(CompressedSegment { ... })
    }
}
```

### 4.2 Project Memory Aggregation

```rust
struct ProjectMemoryAggregator {
    project_memory: ProjectMemory,
    config: AggregationConfig,
}

impl ProjectMemoryAggregator {
    fn merge_segment(&mut self, segment: CompressedSegment) {
        // Merge decisions (deduplicate by topic+choice)
        for decision in segment.decisions {
            self.upsert_decision(decision);
        }
        
        // Merge commitments
        for commitment in segment.commitments {
            self.upsert_commitment(commitment);
        }
        
        // Merge open questions
        for question in segment.open_questions {
            self.upsert_question(question);
        }
        
        // Merge artifacts
        for artifact in segment.artifacts {
            self.upsert_artifact(artifact);
        }
        
        // Merge goals
        for goal in segment.goals {
            self.upsert_goal(goal);
        }
        
        // Update topic index
        for topic in segment.key_topics {
            self.project_memory.topic_index.entry(topic).or_default().push(segment.segment_id);
        }
        
        // Update narrative
        self.project_memory.project_narrative = self.regenerate_narrative();
        
        // Update continuation suggestion
        self.project_memory.suggested_continuation = self.suggest_continuation();
        
        self.project_memory.updated_at = now();
    }
    
    fn upsert_decision(&mut self, new: DecisionRecord) {
        // Find existing by topic similarity
        if let Some(existing) = self.project_memory.aggregated_decisions.iter_mut()
            .find(|d| d.topic == new.topic && d.choice == new.choice) 
        {
            // Update: keep highest confidence, merge provenance
            if new.confidence > existing.confidence {
                *existing = new;
            }
            existing.turn_ids.extend(new.turn_ids);
            existing.related_artifacts.extend(new.related_artifacts);
        } else {
            self.project_memory.aggregated_decisions.push(new);
        }
    }
    
    fn suggest_continuation(&self) -> Option<String> {
        // Priority order:
        // 1. Blocked commitments
        if let Some(blocked) = self.project_memory.active_commitments.iter()
            .find(|c| matches!(c.status, CommitmentStatus::Blocked)) 
        {
            return Some(format!("Blocked: {}. Blocker: {}", blocked.description, blocked.blockers[0]));
        }
        
        // 2. Active goals with next steps
        if let Some(goal) = self.project_memory.active_goals.iter()
            .find(|g| g.status == GoalStatus::Active && !g.next_step.is_empty()) 
        {
            return Some(format!("Last time: {}. Next: {}", goal.recent_progress, goal.next_step));
        }
        
        // 3. High-priority open questions
        if let Some(q) = self.project_memory.open_questions.iter()
            .filter(|q| q.priority == QuestionPriority::High)
            .next() 
        {
            return Some(format!("Open question: {}", q.question));
        }
        
        // 4. Recent artifacts needing follow-up
        if let Some(artifact) = self.project_memory.key_artifacts.iter()
            .filter(|a| a.status == ArtifactStatus::Created || a.status == ArtifactStatus::Modified)
            .next() 
        {
            return Some(format!("Follow up on {}: {}", artifact.kind, artifact.identifier));
        }
        
        None
    }
}
```

---

## 5. Session Continuity

### 5.1 Session Resume with Context

```rust
struct SessionContinuityManager {
    project_memory: Arc<RwLock<ProjectMemory>>,
    user_memory: Arc<RwLock<UserMemory>>,
    compressor: MemoryCompressor,
}

impl SessionContinuityManager {
    async fn resume_or_create_session(
        &self, 
        user_id: UserId, 
        project_id: Option<ProjectId>,
        previous_session_id: Option<SessionId>
    ) -> SessionContext {
        let mut context = SessionContext::new(user_id, project_id);
        
        // 1. Load project memory if available
        if let Some(pid) = project_id {
            if let Some(pmem) = self.load_project_memory(pid).await {
                context.project_context = Some(ProjectContext {
                    project_summary: pmem.project_narrative,
                    active_goals: pmem.active_goals.clone(),
                    open_commitments: pmem.active_commitments.iter()
                        .filter(|c| matches!(c.status, CommitmentStatus::Open | CommitmentStatus::InProgress))
                        .cloned()
                        .collect(),
                    recent_artifacts: pmem.key_artifacts.iter().take(10).cloned().collect(),
                    learned_conventions: pmem.learned_conventions.clone(),
                    suggested_continuation: pmem.suggested_continuation.clone(),
                });
            }
        }
        
        // 2. Load user memory
        if let Some(umem) = self.load_user_memory(user_id).await {
            context.user_context = Some(UserContext {
                preferences: umem.preferences.clone(),
                communication_style: umem.communication_style.clone(),
                relationship: umem.relationship.clone(),
                expertise_domains: umem.expertise_domains.clone(),
                global_narrative: umem.global_narrative.clone(),
            });
        }
        
        // 3. Handle previous session resume
        if let Some(prev_sid) = previous_session_id {
            if let Some(prev_session) = self.load_session(prev_sid).await {
                context.previous_session = Some(PreviousSessionContext {
                    session_id: prev_sid,
                    turn_count: prev_session.turn_count,
                    last_topics: self.extract_topics(&prev_session),
                    last_artifacts: self.extract_recent_artifacts(&prev_session),
                    ended_at: prev_session.last_activity,
                });
            }
        }
        
        // 4. Generate personalized greeting
        context.greeting = self.generate_greeting(&context);
        
        context
    }
    
    fn generate_greeting(&self, ctx: &SessionContext) -> String {
        // Returning user with project context
        if let Some(proj) = &ctx.project_context {
            if let Some(cont) = &proj.suggested_continuation {
                return format!("Back at it. {}", cont);
            }
            if let Some(prev) = &ctx.previous_session {
                let hours_ago = (now() - prev.ended_at).as_secs() / 3600;
                if hours_ago < 24 {
                    return format!("Welcome back. Still on {}?", proj.project_summary.split('.').next().unwrap_or("that"));
                }
            }
            return format!("Back to {}. What's next?", proj.project_summary.split('.').next().unwrap_or("the project"));
        }
        
        // Returning user, no project
        if ctx.user_context.is_some() {
            return "Back at it. What are we working on?".into();
        }
        
        // New user
        "I'm Metamind. What should I call you?".into()
    }
}
```

### 5.2 Cross-Device / Multi-UI Sync

```rust
struct MultiDeviceSync {
    session_store: Arc<SessionStore>,
    presence: PresenceManager,
}

impl MultiDeviceSync {
    // User opens web UI while Telegram session active
    async fn on_channel_connect(&self, session_id: SessionId, channel: UiChannel) {
        // 1. Fetch current session state
        let session = self.session_store.get(session_id).await.unwrap();
        
        // 2. Send current state to new channel
        let state = SessionStateSnapshot {
            turn_count: session.turn_count,
            recent_turns: session.recent_turns(5),
            active_goal: session.active_goal.clone(),
            project_context: session.project_id,
            user_name: session.user_name,
        };
        channel.send(UiMessage::SessionState(state)).await;
        
        // 3. Register presence
        self.presence.register(session_id, channel).await;
    }
    
    // User sends message on one channel, appears on all
    async fn broadcast_turn(&self, session_id: SessionId, turn: Turn, origin: UiChannel) {
        let channels = self.presence.get_channels(session_id).await;
        for channel in channels {
            if channel != origin {
                channel.send(UiMessage::Turn(turn.clone())).await;
            }
        }
    }
    
    // Graceful handoff: user switches from voice to text
    async fn handoff(&self, session_id: SessionId, from: UiChannel, to: UiChannel) {
        // Preserve voice context (prosody, affect) in session
        let session = self.session_store.get(session_id).await.unwrap();
        session.voice_context = session.voice_context.map(|vc| VoiceContext {
            last_affect: vc.last_affect,
            prosody_profile: vc.prosody_profile,
            tier: vc.tier,
        });
        
        // Notify new channel of modality change
        to.send(UiMessage::ModalityChanged { 
            from: Modality::Voice, 
            to: Modality::Text,
            preserved_context: session.voice_context.clone(),
        }).await;
    }
}
```

---

## 6. Cross-Project Learning

```rust
struct CrossProjectLearner {
    user_memory: Arc<RwLock<UserMemory>>,
}

impl CrossProjectLearner {
    async fn learn_from_project_completion(&self, project_id: ProjectId) {
        let pmem = self.load_project_memory(project_id).await.unwrap();
        let mut umem = self.user_memory.write().await;
        
        // 1. Extract tech stack expertise
        for tech in &pmem.tech_stack.languages {
            let entry = umem.expertise_domains.entry(tech.clone()).or_default();
            *entry = (*entry as u32 + 1).min(100);
        }
        
        // 2. Learn conventions
        for convention in &pmem.learned_conventions {
            if convention.confidence > 0.8 {
                umem.preferred_tools.push(ToolPreference {
                    tool: convention.tool.clone(),
                    context: convention.context.clone(),
                    confidence: convention.confidence,
                });
            }
        }
        
        // 3. Update communication patterns
        umem.communication_patterns.update_from_project(&pmem);
        
        // 4. Update global narrative
        umem.global_narrative = self.regenerate_global_narrative(&umem);
    }
    
    fn regenerate_global_narrative(&self, umem: &UserMemory) -> String {
        let top_expertise = umem.expertise_domains.iter()
            .max_by_key(|(_, &v)| v)
            .map(|(k, _)| k.clone())
            .unwrap_or("generalist".into());
        
        let style = match umem.preferences.preferences.get("verbosity") {
            Some(p) if p.value < -0.5 => "terse",
            Some(p) if p.value > 0.5 => "verbose",
            _ => "balanced",
        };
        
        let current_project = umem.projects.values()
            .max_by_key(|p| p.last_activity)
            .map(|p| p.name.clone())
            .unwrap_or("none".into());
        
        format!(
            "{} {} engineer, prefers {} communication. Current project: {}.",
            umem.identity.role.unwrap_or("Software".into()),
            top_expertise,
            style,
            current_project
        )
    }
}
```

---

## 7. Privacy & Data Control

```rust
struct PrivacyController {
    user_id: UserId,
    policies: PrivacyPolicies,
}

struct PrivacyPolicies {
    // Retention
    session_retention_days: u32,
    compressed_retention_days: u32,
    project_retention_days: Option<u32>,
    
    // Granular control
    allow_cross_project_learning: bool,
    allow_preference_learning: bool,
    allow_voice_recording: bool,
    allow_analytics: bool,
    
    // Export/Delete
    export_format: ExportFormat,
    auto_delete_on_inactivity_days: Option<u32>,
}

impl PrivacyController {
    // User requests full data export
    async fn export_all(&self, format: ExportFormat) -> ExportPackage {
        // Collect all memory tiers
        // Apply redaction (PII, secrets)
        // Package in requested format
    }
    
    // User requests deletion
    async fn delete_all(&self) -> DeletionReport {
        // Cascade delete: sessions → segments → project memory → user memory
        // Audit log
    }
    
    // Per-project privacy
    async fn set_project_privacy(&self, project_id: ProjectId, policy: ProjectPrivacyPolicy) {
        // Override global settings for specific project
    }
    
    // Ephemeral mode: no persistence
    fn enable_ephemeral_mode(&mut self) {
        self.policies.session_retention_days = 0;
        self.policies.compressed_retention_days = 0;
        self.policies.allow_cross_project_learning = false;
        self.policies.allow_preference_learning = false;
    }
}
```

---

## 8. Configuration

```toml
[cross_session_memory]
enabled = true

# Session
session_token_threshold = 2000
segment_tokens = 2000
segment_overlap_turns = 2
session_ttl_days = 30

# Compression
compression_model = "gpt-4o-mini"
embedding_model = "sentence-transformers/all-MiniLM-L6-v2"
validate_every_n = 10
min_decision_recall = 0.9
min_commitment_recall = 0.9

# Project
project_aggregation_enabled = true
project_narrative_max_tokens = 500
continuation_suggestion_enabled = true

# User
cross_project_learning_enabled = true
preference_learning_enabled = true
global_narrative_enabled = true

# Retention
session_retention_days = 30
compressed_retention_days = 365
project_retention_days = 0  # 0 = indefinite
user_retention_days = 0

# Sync
multi_device_sync_enabled = true
presence_ttl_seconds = 300
state_sync_interval_ms = 100

# Privacy
allow_cross_project_learning = true
allow_preference_learning = true
export_format = "json"
```

---

## 9. Metrics

| Metric | Target |
|---|---|
| Session resume latency | < 500ms |
| Context relevance (user rated) | > 4.0/5 |
| Continuation suggestion acceptance | > 60% |
| Cross-project preference transfer accuracy | > 75% |
| Memory compression ratio | 15–25x |
| Query latency (semantic search) | < 100ms |

---

## 10. Example Continuity Scenarios

| Scenario | Behavior |
|---|---|
| User returns tomorrow | "Back at it. Last time we fixed the migration timeout. Next: Redis caching?" |
| User switches from Telegram to Web | Seamless — same session, full history visible |
| User starts new project | "New project! What should I call you?" (or uses saved name) |
| User completes project | Project archived, learnings transferred to user memory |
| User asks "What did we decide about caching?" | Instant retrieval from project memory |
| User says "Same conventions as last project" | Applies learned conventions automatically |
| Voice → Text handoff | Prosody context preserved, affect detected |

---

*Cross-session memory transforms the agent from a **stateless chatbot** into a **long-term thinking partner** — it remembers what matters, learns what you like, and picks up exactly where you left off.*