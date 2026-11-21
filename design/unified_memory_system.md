# Unified Memory System — MetaMind

**Status**: Consolidated design merging `cross_session_memory.md`, `conversation_compression.md`, Memory Region SM, and RDF backend user/project/app homes
**Scope**: Single unified memory architecture for all subsystems
**Replaces**: `cross_session_memory.md`, `conversation_compression.md`, Memory Region in `conversation_engine_design.md`, memory sections in `rdf_backend_integration.md`

---

## Executive Summary

This design unifies all memory subsystems into a **single coherent architecture**:

| Subsystem | Before | After (Unified) |
|-----------|--------|-----------------|
| **Cross-session continuity** | `cross_session_memory.md` | Unified Memory Hierarchy (4 tiers) |
| **Conversation compression** | `conversation_compression.md` | Compression Pipeline (stage in hierarchy) |
| **Memory Region SM** | `conversation_engine_design.md` | Memory Region state machine (drives hierarchy) |
| **User/Project/App homes** | `rdf_backend_integration.md` | RDF-backed storage layer for hierarchy |
| **Agentstream logs** | `rdf_backend_integration.md` | Episodic tier (raw turns + events) |

**Single source of truth**: `mm-data` crate owns the unified memory implementation; all consumers (CME, agentstream, registries) use its API.

---

## I. Unified Memory Hierarchy (4 Tiers)

```
┌─────────────────────────────────────────────────────────────────────────────────────┐
│                           UNIFIED MEMORY HIERARCHY                                  │
├─────────────────────────────────────────────────────────────────────────────────────┤
│                                                                                     │
│  ┌─────────────────────────────────────────────────────────────────────────────┐   │
│  │ TIER 1: EPISODIC (Session-scoped, Raw)                                       │   │
│  │ ───────────────────────────────────────────────────────────────────────────── │   │
│  │ • Raw turns, tool calls, user messages, agent responses                     │   │
│  │ • Voice audio chunks, prosody, barge-in events                              │   │
│  │ • Agentstream event log (append-only N-Triples)                             │   │
│  │ • Proactive items executed, plugin outputs                                  │   │
│  │ • Affect traces, modality switches                                          │   │
│  │ • TTL: 30 days (configurable) → triggers compression                        │   │
│  │ • Storage: Agentstream named graphs + SQLite (chat frames)                  │   │
│  └─────────────────────────────────────────────────────────────────────────────┘   │
│                                      │                                             │
│                                      ▼ Compression Pipeline                       │
│  ┌─────────────────────────────────────────────────────────────────────────────┐   │
│  │ TIER 2: COMPRESSED SEGMENTS (Session-scoped, Structured)                    │   │
│  │ ───────────────────────────────────────────────────────────────────────────── │   │
│  │ • Narrative summary (3-5 sentences, causal chain)                           │   │
│  │ • Structured artifacts: decisions, commitments, open questions, artifacts,  │   │
│  │   goals, preferences (with provenance to turn IDs)                          │   │
│  │ • Embeddings (384-dim sentence-transformers) + key topics                   │   │
│  │ • Quality metrics: decision_recall, commitment_recall, artifact_recall      │   │
│  │ • TTL: 1 year → triggers project aggregation                                │   │
│  │ • Storage: RDF (cubecast) — CompressedSegment graphs + vector index         │   │
│  └─────────────────────────────────────────────────────────────────────────────┘   │
│                                      │                                             │
│                                      ▼ Project Aggregation                        │
│  ┌─────────────────────────────────────────────────────────────────────────────┐   │
│  │ TIER 3: PROJECT MEMORY (Project-scoped, Aggregated)                         │   │
│  │ ───────────────────────────────────────────────────────────────────────────── │   │
│  │ • Aggregated decisions (deduped by topic+choice)                            │   │
│  │ • Active commitments, open questions, key artifacts, active goals           │   │
│  │ • Learned project conventions, tech stack, deployment targets               │   │
│  │ • Project narrative + continuation suggestions                              │   │
│  │ • Topic index → segment IDs for semantic retrieval                          │   │
│  │ • TTL: Indefinite (project lifetime)                                        │   │
│  │ • Storage: RDF (cubecast project layer) — ProjectMemory graph               │   │
│  └─────────────────────────────────────────────────────────────────────────────┘   │
│                                      │                                             │
│                                      ▼ Cross-Project Learning                     │
│  ┌─────────────────────────────────────────────────────────────────────────────┐   │
│  │ TIER 4: USER MEMORY (User-scoped, Global)                                   │   │
│  │ ───────────────────────────────────────────────────────────────────────────── │   │
│  │ • Identity, global preferences (learned + explicit)                         │   │
│  │ • Communication style, relationship state, known facts                      │   │
│  │ • Expertise domains, communication patterns, preferred tools                │   │
│  │ • Project registry (ProjectSummary per project)                             │   │
│  │ • Global narrative: "Senior Rust engineer, prefers terse communication,     │   │
│  │   currently working on auth-service migration"                              │   │
│  │ • TTL: Indefinite (user lifetime)                                           │   │
│  │ • Storage: RDF (cubecast user layer) — UserMemory graph + FOAF profile      │   │
│  └─────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                     │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

---

## II. Core Data Structures (Unified)

### II.1 Tier 1: Episodic (Raw)

```rust
// mm-data/src/memory/episodic.rs

/// Raw turn captured during conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicTurn {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub project_id: Option<ProjectId>,
    pub user_id: UserId,
    pub turn_number: u32,
    pub timestamp: Timestamp,
    
    // User input
    pub user_query: Option<String>,
    pub user_modality: InputModality,      // Text | Voice | Structured
    pub voice_context: Option<VoiceContext>, // prosody, affect, tier
    
    // Agent response
    pub agent_response: AgentResponse,
    pub modality: ResponseModality,
    pub detail_level: u8,
    pub chunks: Vec<EmittedChunk>,
    
    // Tool interactions
    pub tool_calls: Vec<ToolCallRecord>,
    pub tool_results: Vec<ToolResultRecord>,
    
    // Proactive
    pub proactive_items: Vec<ProactiveItemRecord>,
    
    // State
    pub affect_snapshot: AffectState,
    pub preferences_applied: PreferenceSnapshot,
}

/// Agentstream event log entry (append-only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentstreamEvent {
    pub event_id: EventId,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub timestamp: Timestamp,
    pub event_type: EventType,             // UserMessage | AgentMessage | ToolCall | ToolResult | Proactive | BargeIn | ModalitySwitch
    pub payload: Value,                    // JSON serialized
    pub provenance: EventProvenance,
}
```

### II.2 Tier 2: Compressed Segments (Structured)

```rust
// mm-data/src/memory/compressed.rs

/// Compressed segment — output of compression pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedSegment {
    pub segment_id: SegmentId,
    pub session_id: SessionId,
    pub project_id: Option<ProjectId>,
    pub user_id: UserId,
    pub turn_range: (TurnId, TurnId),
    pub token_count: usize,
    pub created_at: Timestamp,
    
    // Narrative (for LLM context injection)
    pub narrative: String,
    // "User debugged deploy failure on auth-service (session abc123).
    //  Root cause: migration timeout on add_index_users_email (commit 3f2a1).
    //  Fixed via PR #247. Discussed caching strategy — user chose Redis layer,
    //  will implement separately. Open: cache invalidation strategy."
    
    // Structured artifacts (queryable)
    pub decisions: Vec<DecisionRecord>,
    pub commitments: Vec<Commitment>,
    pub open_questions: Vec<OpenQuestion>,
    pub artifacts: Vec<ArtifactRef>,
    pub goals: Vec<GoalProgress>,
    pub inferred_preferences: Vec<InferredPreference>,
    
    // Semantic retrieval
    pub embedding: Vector,                 // 384-dim sentence-transformers
    pub key_topics: Vec<String>,           // ["deploy", "migration", "caching"]
    pub topic_weights: HashMap<String, f32>,
    
    // Quality
    pub compression_ratio: f32,
    pub decision_recall: f32,
    pub commitment_recall: f32,
    pub artifact_recall: f32,
    pub narrative_faithfulness: f32,
    pub hallucination_rate: f32,
    
    // Provenance
    pub source_turn_ids: Vec<TurnId>,
    pub compression_model: String,
    pub extraction_model: String,
    pub embedding_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: DecisionId,
    pub topic: String,                     // "caching strategy for org lookup"
    pub choice: String,                    // "Redis layer, separate PR"
    pub rationale: String,                 // "Lower risk, incremental, team knows Redis"
    pub alternatives: Vec<Alternative>,
    pub confidence: f32,                   // 0.0–1.0
    pub status: DecisionStatus,            // Decided | Tentative | Reversed | Superseded
    pub decided_by: EntityId,              // user or agent
    pub turn_ids: Vec<TurnId>,             // Provenance
    pub related_artifacts: Vec<ArtifactRef>,
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub commitment_id: CommitmentId,
    pub description: String,               // "Implement Redis caching for org lookup"
    pub owner: EntityId,                   // user or agent
    pub deadline: Option<Timestamp>,
    pub status: CommitmentStatus,          // Open | InProgress | Done | Blocked | Cancelled
    pub progress: f32,                     // 0.0–1.0
    pub blockers: Vec<String>,
    pub created_turn: TurnId,
    pub updated_turn: Option<TurnId>,
    pub related_decisions: Vec<DecisionId>,
    pub related_artifacts: Vec<ArtifactRef>,
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenQuestion {
    pub question_id: QuestionId,
    pub question: String,                  // "What cache invalidation strategy?"
    pub context: String,                   // "Redis layer chosen, need TTL strategy"
    pub asked_by: EntityId,
    pub asked_at: Timestamp,
    pub turn_id: TurnId,
    pub priority: QuestionPriority,        // High | Medium | Low
    pub tentative_answer: Option<String>,
    pub related_decisions: Vec<DecisionId>,
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub kind: ArtifactKind,                // PullRequest | Commit | File | Config | Diagram | Test | Document | Script | Migration | Other
    pub identifier: String,                // "PR #247", "src/cache/redis.rs", "config/redis.toml"
    pub description: String,
    pub location: Option<String>,          // GitHub URL, file path
    pub created_turn: TurnId,
    pub status: ArtifactStatus,            // Created | Modified | Merged | Deployed | Archived | Deleted
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalProgress {
    pub goal_id: GoalId,
    pub description: String,
    pub status: GoalStatus,                // Active | OnHold | Completed | Abandoned
    pub progress: f32,
    pub subgoals: Vec<Subgoal>,
    pub related_commitments: Vec<CommitmentId>,
    pub last_updated: Timestamp,
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferredPreference {
    pub dimension: PreferenceDimension,    // verbosity | initiative | technical_depth | formality | pace
    pub value: f32,                        // -1.0 to 1.0 (relative to baseline)
    pub confidence: f32,                   // 0.0–1.0
    pub evidence_turns: Vec<TurnId>,
}
```

### II.3 Tier 3: Project Memory (Aggregated)

```rust
// mm-data/src/memory/project.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMemory {
    pub project_id: ProjectId,
    pub user_id: UserId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    
    // Aggregated views (incrementally updated from segments)
    pub aggregated_decisions: Vec<DecisionRecord>,
    pub active_commitments: Vec<Commitment>,
    pub open_questions: Vec<OpenQuestion>,
    pub key_artifacts: Vec<ArtifactRef>,
    pub active_goals: Vec<GoalProgress>,
    
    // Learned project-specific preferences
    pub learned_conventions: Vec<ConventionRule>,
    pub tech_stack: TechStack,
    pub deployment_targets: Vec<DeploymentTarget>,
    
    // Semantic index
    pub segment_ids: Vec<SegmentId>,
    pub topic_index: HashMap<String, Vec<SegmentId>>,  // topic → segment IDs
    
    // Continuity
    pub last_session_id: Option<SessionId>,
    pub suggested_continuation: Option<String>,
    pub project_narrative: String,         // "Auth-service migration: fixed timeout, implementing Redis cache. Next: invalidation strategy."
    
    // Project metadata
    pub status: ProjectStatus,             // Active | OnHold | Completed | Archived
    pub members: Vec<UserId>,
    pub datasets: Vec<DatasetId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConventionRule {
    pub rule_id: RuleId,
    pub context: String,                   // "git commit messages"
    pub rule: String,                      // "Conventional commits: feat:, fix:, chore:"
    pub tool: Option<String>,              // "git", "cargo", "docker"
    pub confidence: f32,                   // 0.0–1.0
    pub learned_from_turns: Vec<TurnId>,
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechStack {
    pub languages: Vec<String>,            // ["Rust", "TypeScript", "SQL"]
    pub frameworks: Vec<String>,           // ["Axum", "React", "PostgreSQL"]
    pub infrastructure: Vec<String>,       // ["Kubernetes", "Redis", "PostgreSQL"]
    pub tools: Vec<String>,                // ["cargo", "docker", "kubectl"]
}
```

### II.4 Tier 4: User Memory (Global)

```rust
// mm-data/src/memory/user.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMemory {
    pub user_id: UserId,
    pub identity: UserIdentity,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    
    // Global preferences (learned + explicit)
    pub preferences: UserPreferenceModel,
    pub communication_style: CommunicationStyle,
    
    // Relationship & social
    pub relationship: RelationshipState,
    pub known_facts: HashMap<String, Fact>,
    
    // Cross-project learning
    pub expertise_domains: HashMap<String, ExpertiseLevel>,  // "Rust" → 85
    pub communication_patterns: CommunicationPatterns,
    pub preferred_tools: Vec<ToolPreference>,
    
    // Project registry
    pub projects: HashMap<ProjectId, ProjectSummary>,
    
    // Global continuity
    pub global_narrative: String,          // "Senior backend engineer, focuses on Rust/Go, prefers terse communication, currently working on auth-service migration"
    
    // Privacy
    pub privacy_policies: PrivacyPolicies,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferenceModel {
    // Explicit preferences (user-set)
    pub explicit: HashMap<PreferenceDimension, PreferenceValue>,
    // Learned preferences (from behavior)
    pub learned: HashMap<PreferenceDimension, LearnedPreference>,
    // Merged (what planner actually uses)
    pub merged: HashMap<PreferenceDimension, f32>,  // -1.0 to 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedPreference {
    pub mean: f32,                         // posterior mean
    pub variance: f32,                     // posterior variance
    pub n_observations: u32,
    pub last_updated: Timestamp,
    pub state: PreferenceState,            // ColdStart | Observing | Converging | Stable | DriftDetected
}
```

---

## III. Memory Lifecycle (State Machine Driven)

The **Memory Region SM** (from `conversation_engine_design.md`) drives transitions:

```
MemoryRegion SM:
Episodic → Compressing → Segmented → ProjectAggregating → UserSyncing → Episodic
     │           │            │              │                │
     │           │            │              │                └─ Cross-project learning
     │           │            │              └─ Merge segments → ProjectMemory
     │           │            └─ Store CompressedSegment (RDF + vector index)
     │           └─ Run compression pipeline (LLM extraction + narrative + embed)
     └─ Buffer turns in episodic store (Agentstream + SQLite)
```

### III.1 Compression Pipeline (Triggered at Episodic → Compressing)

```rust
// mm-data/src/memory/compressor.rs

pub struct UnifiedCompressor {
    llm: Arc<dyn LlmBackend>,
    embedder: Arc<dyn Embedder>,
    config: CompressionConfig,
    rdf_engine: Arc<RdfEngine>,
    vector_index: Arc<VectorIndex>,
}

impl UnifiedCompressor {
    /// Compress episodic turns into CompressedSegment(s)
    pub async fn compress_session(&self, session_id: SessionId) -> Result<Vec<CompressedSegment>> {
        // 1. Load episodic turns from Agentstream + SQLite
        let turns = self.load_episodic_turns(session_id).await?;
        
        // 2. Chunk into segments (~2000 tokens with overlap)
        let chunks = self.chunk_turns(&turns, self.config.segment_tokens, self.config.overlap_turns);
        
        let mut segments = Vec::new();
        for chunk in chunks {
            // Stage 1: Structured extraction (JSON)
            let extraction = self.extract_structured(&chunk).await?;
            
            // Stage 2: Narrative generation
            let narrative = self.generate_narrative(&chunk, &extraction).await?;
            
            // Stage 3: Embedding + topics
            let embedding = self.embedder.embed(&narrative).await?;
            let (topics, weights) = self.extract_topics(&narrative, &extraction);
            
            // Stage 4: Quality validation (sample)
            let quality = if self.config.validate_sample {
                Some(self.validate_quality(&chunk, &extraction).await?)
            } else { None };
            
            // Build segment
            let segment = CompressedSegment {
                segment_id: SegmentId::new(),
                session_id,
                project_id: chunk.first().and_then(|t| t.project_id),
                user_id: chunk.first().map(|t| t.user_id).unwrap(),
                turn_range: (chunk.first().unwrap().turn_id, chunk.last().unwrap().turn_id),
                token_count: chunk.iter().map(|t| t.estimated_tokens()).sum(),
                narrative,
                decisions: extraction.decisions,
                commitments: extraction.commitments,
                open_questions: extraction.open_questions,
                artifacts: extraction.artifacts,
                goals: extraction.goals,
                inferred_preferences: extraction.inferred_preferences,
                embedding,
                key_topics: topics,
                topic_weights: weights,
                compression_ratio: self.calculate_ratio(&chunk, &narrative),
                decision_recall: quality.as_ref().map(|q| q.decision_recall).unwrap_or(0.0),
                commitment_recall: quality.as_ref().map(|q| q.commitment_recall).unwrap_or(0.0),
                artifact_recall: quality.as_ref().map(|q| q.artifact_recall).unwrap_or(0.0),
                narrative_faithfulness: quality.as_ref().map(|q| q.narrative_faithfulness).unwrap_or(0.0),
                hallucination_rate: quality.as_ref().map(|q| q.hallucination_rate).unwrap_or(0.0),
                source_turn_ids: chunk.iter().map(|t| t.turn_id).collect(),
                compression_model: self.config.compression_model.clone(),
                extraction_model: self.config.extraction_model.clone(),
                embedding_model: self.config.embedding_model.clone(),
                created_at: now(),
            };
            
            // 5. Persist to RDF (CompressedSegment graph) + vector index
            self.persist_segment(&segment).await?;
            
            // 6. Update Agentstream: mark turns as compressed
            self.mark_turns_compressed(&chunk).await?;
            
            segments.push(segment);
        }
        
        Ok(segments)
    }
    
    async fn persist_segment(&self, segment: &CompressedSegment) -> Result<()> {
        // RDF: Store in cubecast as named graph
        let graph_iri = format!("https://metamind.dev/memory/segment/{}", segment.segment_id);
        let turtle = segment_to_turtle(segment);
        self.rdf_engine.execute(Command::Rdf(RdfCommand::Update {
            sparql: format!("LOAD <data:application/x-turtle,{}> INTO GRAPH <{}>", 
                urlencoding::encode(&turtle), graph_iri),
        })).await?;
        
        // Vector index: add embedding for semantic search
        self.vector_index.add(segment.segment_id, &segment.embedding, SegmentMetadata {
            user_id: segment.user_id,
            project_id: segment.project_id,
            session_id: segment.session_id,
            key_topics: segment.key_topics.clone(),
            time_range: (segment.created_at, segment.created_at),
        }).await?;
        
        Ok(())
    }
}
```

### III.2 Project Aggregation (Segmented → ProjectAggregating)

```rust
// mm-data/src/memory/aggregator.rs

pub struct ProjectAggregator {
    rdf_engine: Arc<RdfEngine>,
    vector_index: Arc<VectorIndex>,
}

impl ProjectAggregator {
    pub async fn merge_segment(&self, project_id: ProjectId, segment: &CompressedSegment) -> Result<()> {
        // Load current ProjectMemory from RDF
        let mut project_mem = self.load_project_memory(project_id).await?;
        
        // Merge decisions (dedup by topic+choice)
        for decision in &segment.decisions {
            self.upsert_decision(&mut project_mem, decision);
        }
        
        // Merge commitments
        for commitment in &segment.commitments {
            self.upsert_commitment(&mut project_mem, commitment);
        }
        
        // Merge open questions
        for question in &segment.open_questions {
            self.upsert_question(&mut project_mem, question);
        }
        
        // Merge artifacts
        for artifact in &segment.artifacts {
            self.upsert_artifact(&mut project_mem, artifact);
        }
        
        // Merge goals
        for goal in &segment.goals {
            self.upsert_goal(&mut project_mem, goal);
        }
        
        // Merge inferred preferences
        for pref in &segment.inferred_preferences {
            self.upsert_preference(&mut project_mem, pref);
        }
        
        // Update topic index
        for topic in &segment.key_topics {
            project_mem.topic_index.entry(topic.clone()).or_default().push(segment.segment_id);
        }
        
        // Add segment ID
        project_mem.segment_ids.push(segment.segment_id);
        
        // Regenerate narrative & continuation
        project_mem.project_narrative = self.regenerate_narrative(&project_mem);
        project_mem.suggested_continuation = self.suggest_continuation(&project_mem);
        project_mem.updated_at = now();
        
        // Persist to RDF (ProjectMemory graph)
        self.persist_project_memory(&project_mem).await?;
        
        Ok(())
    }
    
    fn suggest_continuation(&self, pmem: &ProjectMemory) -> Option<String> {
        // Priority order:
        // 1. Blocked commitments
        if let Some(blocked) = pmem.active_commitments.iter()
            .find(|c| matches!(c.status, CommitmentStatus::Blocked)) 
        {
            return Some(format!("Blocked: {}. Blocker: {}", blocked.description, blocked.blockers[0]));
        }
        
        // 2. Active goals with next steps
        if let Some(goal) = pmem.active_goals.iter()
            .find(|g| g.status == GoalStatus::Active && !g.next_step.is_empty()) 
        {
            return Some(format!("Last time: {}. Next: {}", goal.recent_progress, goal.next_step));
        }
        
        // 3. High-priority open questions
        if let Some(q) = pmem.open_questions.iter()
            .filter(|q| q.priority == QuestionPriority::High)
            .next() 
        {
            return Some(format!("Open question: {}", q.question));
        }
        
        // 4. Recent artifacts needing follow-up
        if let Some(artifact) = pmem.key_artifacts.iter()
            .filter(|a| a.status == ArtifactStatus::Created || a.status == ArtifactStatus::Modified)
            .next() 
        {
            return Some(format!("Follow up on {}: {}", artifact.kind, artifact.identifier));
        }
        
        None
    }
}
```

### III.3 Cross-Project Learning (UserSyncing)

```rust
// mm-data/src/memory/cross_project.rs

pub struct CrossProjectLearner {
    rdf_engine: Arc<RdfEngine>,
    vector_index: Arc<VectorIndex>,
}

impl CrossProjectLearner {
    pub async fn learn_from_project(&self, user_id: UserId, project_id: ProjectId) -> Result<()> {
        let pmem = self.load_project_memory(project_id).await?;
        let mut umem = self.load_user_memory(user_id).await?;
        
        // 1. Extract tech stack expertise
        for tech in &pmem.tech_stack.languages {
            let entry = umem.expertise_domains.entry(tech.clone()).or_insert(0);
            *entry = (*entry + 10).min(100);
        }
        for framework in &pmem.tech_stack.frameworks {
            let entry = umem.expertise_domains.entry(framework.clone()).or_insert(0);
            *entry = (*entry + 5).min(100);
        }
        
        // 2. Learn conventions (high confidence only)
        for convention in &pmem.learned_conventions {
            if convention.confidence > 0.8 {
                umem.preferred_tools.push(ToolPreference {
                    tool: convention.tool.clone(),
                    context: convention.context.clone(),
                    confidence: convention.confidence,
                    learned_from_project: project_id,
                });
            }
        }
        
        // 3. Update communication patterns
        umem.communication_patterns.update_from_project(&pmem);
        
        // 4. Update project registry
        umem.projects.insert(project_id, ProjectSummary {
            name: pmem.name.clone(),
            description: pmem.description.clone(),
            status: pmem.status,
            last_activity: pmem.updated_at,
            key_topics: pmem.topic_index.keys().cloned().collect(),
        });
        
        // 5. Regenerate global narrative
        umem.global_narrative = self.regenerate_global_narrative(&umem);
        umem.updated_at = now();
        
        // Persist
        self.persist_user_memory(&umem).await?;
        
        Ok(())
    }
    
    fn regenerate_global_narrative(&self, umem: &UserMemory) -> String {
        let top_expertise = umem.expertise_domains.iter()
            .max_by_key(|(_, &v)| v)
            .map(|(k, _)| k.clone())
            .unwrap_or("generalist".into());
        
        let style = match umem.preferences.merged.get(&PreferenceDimension::Verbosity) {
            Some(&v) if v < -0.5 => "terse",
            Some(&v) if v > 0.5 => "verbose",
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

## IV. Session Continuity (Resume with Context)

```rust
// mm-data/src/memory/continuity.rs

pub struct SessionContinuityManager {
    rdf_engine: Arc<RdfEngine>,
    vector_index: Arc<VectorIndex>,
    compressor: Arc<UnifiedCompressor>,
    aggregator: Arc<ProjectAggregator>,
    learner: Arc<CrossProjectLearner>,
}

impl SessionContinuityManager {
    pub async fn resume_or_create_session(
        &self,
        user_id: UserId,
        project_id: Option<ProjectId>,
        previous_session_id: Option<SessionId>,
    ) -> Result<SessionContext> {
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
                preferences: umem.preferences.merged.clone(),
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
        
        Ok(context)
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

---

## V. Multi-Device / Multi-UI Sync

```rust
// mm-data/src/memory/sync.rs

pub struct MultiDeviceSync {
    session_store: Arc<SessionStore>,
    presence: PresenceManager,
    rdf_engine: Arc<RdfEngine>,
}

impl MultiDeviceSync {
    pub async fn on_channel_connect(&self, session_id: SessionId, channel: UiChannel) -> Result<()> {
        let session = self.session_store.get(session_id).await?;
        
        // Send current state to new channel
        let state = SessionStateSnapshot {
            turn_count: session.turn_count,
            recent_turns: session.recent_turns(5),
            active_goal: session.active_goal.clone(),
            project_context: session.project_id,
            user_name: session.user_name,
            modality: session.current_modality,
        };
        channel.send(UiMessage::SessionState(state)).await?;
        
        // Register presence
        self.presence.register(session_id, channel).await?;
        
        Ok(())
    }
    
    pub async fn broadcast_turn(&self, session_id: SessionId, turn: Turn, origin: UiChannel) -> Result<()> {
        let channels = self.presence.get_channels(session_id).await?;
        for channel in channels {
            if channel != origin {
                channel.send(UiMessage::Turn(turn.clone())).await?;
            }
        }
        Ok(())
    }
    
    pub async fn handoff(&self, session_id: SessionId, from: UiChannel, to: UiChannel) -> Result<()> {
        let session = self.session_store.get(session_id).await?;
        
        // Preserve voice context in session
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
        }).await?;
        
        Ok(())
    }
}
```

---

## VI. RDF Storage Mapping (Cubecast Integration)

```rust
// mm-data/src/memory/rdf_mapping.rs

/// Named graph IRIs for each memory tier
pub mod graph_iris {
    pub const EPISODIC_BASE: &str = "https://metamind.dev/memory/episodic/";
    pub const SEGMENTS_BASE: &str = "https://metamind.dev/memory/segments/";
    pub const PROJECTS_BASE: &str = "https://metamind.dev/memory/projects/";
    pub const USERS_BASE: &str = "https://metamind.dev/memory/users/";
    pub const AGENTSTREAM_BASE: &str = "https://metamind.dev/agentstream/";
}

/// Ontology prefixes for memory
pub const MEMORY_PREFIXES: &str = r#"
@prefix mm: <https://metamind.dev/ontology#> .
@prefix mmd: <https://metamind.dev/data/> .
@prefix mem: <https://metamind.dev/memory#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix doap: <http://usefulinc.com/ns/doap#> .
"#;
```

### VI.1 Memory Tier → RDF Graphs

| Memory Tier | RDF Graph Pattern | Cubecast Layer |
|-------------|-------------------|----------------|
| Episodic (Agentstream) | `https://metamind.dev/agentstream/session/{id}` | Agentstream store |
| Compressed Segments | `https://metamind.dev/memory/segments/{segment_id}` | Main store + vector index |
| Project Memory | `https://metamind.dev/memory/projects/{project_id}` | Cubecast project layer |
| User Memory | `https://metamind.dev/memory/users/{user_id}` | Cubecast user layer (FOAF/SKOS) |

### VI.2 SPARQL Query Patterns

```sparql
# Find all decisions about "caching" for a user
PREFIX mem: <https://metamind.dev/memory#>
PREFIX mm: <https://metamind.dev/ontology#>

SELECT ?decision ?topic ?choice ?rationale ?confidence ?project
WHERE {
  GRAPH ?segment_graph {
    ?decision a mem:DecisionRecord ;
              mem:topic ?topic ;
              mem:choice ?choice ;
              mem:rationale ?rationale ;
              mem:confidence ?confidence ;
              mem:project ?project .
    FILTER(CONTAINS(LCASE(?topic), "caching"))
  }
  GRAPH <https://metamind.dev/memory/users/{user_id}> {
    ?project mem:hasMember <urn:metamind:user:{user_id}> .
  }
}
ORDER BY DESC(?confidence)
```

---

## VII. Configuration (Unified)

```toml
# config/memory.toml — Single source for all memory settings

[memory]
# Unified hierarchy settings
enabled = true

# Tier 1: Episodic
[memory.episodic]
retention_days = 30
compression_trigger_turns = 10
compression_trigger_tokens = 2000
compression_model = "gpt-4o-mini"
embedding_model = "sentence-transformers/all-MiniLM-L6-v2"

# Tier 2: Compressed Segments
[memory.segments]
retention_days = 365
segment_tokens = 2000
overlap_turns = 2
validate_every_n = 10
min_decision_recall = 0.90
min_commitment_recall = 0.90
min_artifact_recall = 0.85
min_narrative_faithfulness = 0.85
max_hallucination_rate = 0.02

# Tier 3: Project Memory
[memory.project]
aggregation_enabled = true
aggregation_interval_hours = 6
narrative_max_tokens = 500
continuation_suggestion_enabled = true
dedup_decisions_by_topic = true

# Tier 4: User Memory
[memory.user]
cross_project_learning_enabled = true
preference_learning_enabled = true
global_narrative_enabled = true
expertise_decay_enabled = false  # expertise doesn't decay

# Compression Pipeline
[memory.compression]
extraction_model = "gpt-4o-mini"
narrative_model = "gpt-4o-mini"
quality_validation_enabled = true
validation_sample_rate = 0.1

# Vector Index
[memory.vector_index]
dimension = 384
metric = "cosine"
ef_construction = 200
m = 16
ef_search = 100

# Sync
[memory.sync]
multi_device_enabled = true
presence_ttl_seconds = 300
state_sync_interval_ms = 100

# Privacy
[memory.privacy]
allow_cross_project_learning = true
allow_preference_learning = true
export_format = "json"
auto_delete_on_inactivity_days = 0  # 0 = never
```

---

## VIII. API (mm-data Crate)

```rust
// mm-data/src/memory/mod.rs

pub struct MemorySystem {
    pub episodic: EpisodicStore,
    pub segments: SegmentStore,
    pub projects: ProjectMemoryStore,
    pub users: UserMemoryStore,
    pub continuity: SessionContinuityManager,
    pub sync: MultiDeviceSync,
    pub compressor: UnifiedCompressor,
    pub aggregator: ProjectAggregator,
    pub learner: CrossProjectLearner,
}

impl MemorySystem {
    // Session lifecycle
    pub async fn start_session(&self, user_id: UserId, project_id: Option<ProjectId>, previous_session: Option<SessionId>) -> Result<SessionContext>;
    pub async fn end_session(&self, session_id: SessionId) -> Result<()>;
    
    // Turn logging (called by CME)
    pub async fn log_turn(&self, turn: EpisodicTurn) -> Result<()>;
    pub async fn log_agentstream_event(&self, event: AgentstreamEvent) -> Result<()>;
    
    // Compression (triggered by Memory Region SM)
    pub async fn compress_session(&self, session_id: SessionId) -> Result<Vec<CompressedSegment>>;
    pub async fn aggregate_project(&self, project_id: ProjectId) -> Result<()>;
    pub async fn sync_user(&self, user_id: UserId) -> Result<()>;
    
    // Queries
    pub async fn find_decisions(&self, topic: &str, user_id: UserId, project_id: Option<ProjectId>) -> Result<Vec<DecisionRecord>>;
    pub async fn find_commitments(&self, owner: EntityId, status: CommitmentStatus, project_id: Option<ProjectId>) -> Result<Vec<Commitment>>;
    pub async fn find_artifacts(&self, kind: ArtifactKind, query: &str, user_id: UserId) -> Result<Vec<ArtifactRef>>;
    pub async fn find_goals(&self, user_id: UserId, status: GoalStatus, project_id: Option<ProjectId>) -> Result<Vec<GoalProgress>>;
    pub async fn semantic_search(&self, query: &str, user_id: UserId, filters: SearchFilters, k: usize) -> Result<Vec<SearchResult>>;
    
    // Continuity
    pub async fn get_session_context(&self, session_id: SessionId) -> Result<SessionContext>;
    pub async fn get_project_context(&self, project_id: ProjectId) -> Result<ProjectContext>;
    pub async fn get_user_context(&self, user_id: UserId) -> Result<UserContext>;
    
    // Sync
    pub async fn register_channel(&self, session_id: SessionId, channel: UiChannel) -> Result<()>;
    pub async fn broadcast_turn(&self, session_id: SessionId, turn: Turn, origin: UiChannel) -> Result<()>;
    pub async fn handoff_modality(&self, session_id: SessionId, from: UiChannel, to: UiChannel) -> Result<()>;
    
    // Privacy
    pub async fn export_user_data(&self, user_id: UserId, format: ExportFormat) -> Result<ExportPackage>;
    pub async fn delete_user_data(&self, user_id: UserId) -> Result<DeletionReport>;
}
```

---

## IX. Metrics

| Metric | Target | Tier |
|--------|--------|------|
| Session resume latency | < 500ms | All |
| Context relevance (user rated) | > 4.0/5 | 3, 4 |
| Continuation suggestion acceptance | > 60% | 3 |
| Cross-project preference transfer accuracy | > 75% | 4 |
| Memory compression ratio | 15–25x | 1→2 |
| Decision recall | > 0.95 | 2 |
| Commitment recall | > 0.95 | 2 |
| Artifact recall | > 0.90 | 2 |
| Narrative faithfulness | > 0.90 | 2 |
| Hallucination rate | < 0.02 | 2 |
| Compression latency | < 2s/segment | 2 |
| Search latency (p99) | < 100ms | 2, 3 |
| Project aggregation latency | < 5s | 3 |
| User sync latency | < 2s | 4 |
| Multi-device sync latency | < 50ms | Sync |

---

## X. Migration from Legacy Designs

| Legacy Document | Merged Into |
|-----------------|-------------|
| `cross_session_memory.md` | Sections I–IV, VII, VIII (hierarchy, lifecycle, continuity, config) |
| `conversation_compression.md` | Sections II.2, III.1 (compressed segments, compression pipeline) |
| `conversation_engine_design.md` §V | Sections III, VIII (Memory Region SM → UnifiedCompressor, ProjectAggregator) |
| `rdf_backend_integration.md` §V | Sections II.1, VI (Episodic → Agentstream, RDF storage mapping) |

**Deprecated**: The four separate memory designs are superseded by this unified design. All implementation should reference this document.

---

*This unified memory system provides a single, coherent architecture from raw episodic events through compressed segments to aggregated project and user memory — all driven by a state machine, stored in RDF, and queryable via structured and semantic search.*