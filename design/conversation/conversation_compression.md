# Structured Conversation Compression

> Lossless semantic compression preserving queryable artifacts, decisions, commitments.

---

## 1. Problem

Current approaches lose structure:
- **Sliding window**: Drops old context entirely
- **Summarization**: Loses specific decisions, commitments, artifact references
- **No cross-session queries**: "What did we decide about caching last month?" → impossible

---

## 2. CompressedConversation Schema

```rust
struct CompressedConversation {
    // Identity
    id: ConversationId,
    project_id: Option<ProjectId>,
    user_id: UserId,
    session_ids: Vec<SessionId>,
    time_range: (Timestamp, Timestamp),
    
    // Narrative (for LLM context injection)
    narrative: String,
    // "User debugged deploy failure on auth-service (session abc123). 
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
    embedding: Vector,                    // sentence-transformers (384-dim)
    key_topics: Vec<String>,              // ["deploy", "migration", "caching", "auth-service"]
    topic_weights: HashMap<String, f32>,  // TF-IDF + LLM importance
    
    // Provenance
    source_turn_ids: Vec<TurnId>,
    compression_ratio: f32,               // original_tokens / compressed_tokens
    compressed_at: Timestamp,
    compression_model: String,            // "gpt-4o-mini", "local-extractor-v1"
    
    // Quality metrics
    decision_recall: f32,                 // vs human-labeled ground truth
    commitment_recall: f32,
    artifact_recall: f32,
}

struct DecisionRecord {
    id: DecisionId,
    topic: String,                        // "caching strategy for org lookup"
    choice: String,                       // "Redis layer, separate PR"
    rationale: String,                    // "Lower risk, incremental, team knows Redis"
    alternatives_considered: Vec<Alternative>,
    confidence: f32,                      // 0.0–1.0 (how certain the decision is)
    status: DecisionStatus,               // Decided, Tentative, Reversed, Superseded
    decided_by: EntityId,                 // user or agent
    turn_ids: Vec<TurnId>,                // Provenance
    related_artifacts: Vec<ArtifactRef>,
}

struct Alternative {
    description: String,
    reason_rejected: String,
}

enum DecisionStatus {
    Decided,
    Tentative,
    Reversed { reason: String, reversed_at: Timestamp },
    Superseded { by: DecisionId },
}

struct Commitment {
    id: CommitmentId,
    description: String,                  // "Implement Redis caching for org lookup"
    owner: EntityId,                      // user or agent
    deadline: Option<Timestamp>,
    status: CommitmentStatus,             // Open, InProgress, Done, Blocked, Cancelled
    progress: f32,                        // 0.0–1.0
    blockers: Vec<String>,
    created_turn: TurnId,
    updated_turn: Option<TurnId>,
    related_decisions: Vec<DecisionId>,
    related_artifacts: Vec<ArtifactRef>,
}

enum CommitmentStatus {
    Open,
    InProgress { started_at: Timestamp },
    Done { completed_at: Timestamp, evidence: String },
    Blocked { blocker: String, since: Timestamp },
    Cancelled { reason: String },
}

struct OpenQuestion {
    id: QuestionId,
    question: String,                     // "What cache invalidation strategy?"
    context: String,                      // "Redis layer chosen, need TTL strategy"
    asked_by: EntityId,
    asked_at: Timestamp,
    turn_id: TurnId,
    priority: QuestionPriority,           // High, Medium, Low
    tentative_answer: Option<String>,
    related_decisions: Vec<DecisionId>,
}

struct ArtifactRef {
    id: ArtifactId,
    kind: ArtifactKind,
    identifier: String,                   // "PR #247", "src/cache/redis.rs", "config/redis.toml"
    description: String,
    location: Option<String>,             // GitHub URL, file path, etc.
    created_turn: TurnId,
    status: ArtifactStatus,
}

enum ArtifactKind {
    PullRequest,
    Commit,
    File,
    Config,
    Diagram,
    Test,
    Document,
    Script,
    Migration,
    Other(String),
}

enum ArtifactStatus {
    Created,
    Modified,
    Merged,
    Deployed,
    Archived,
    Deleted,
}

struct GoalProgress {
    id: GoalId,
    description: String,
    status: GoalStatus,
    progress: f32,
    subgoals: Vec<Subgoal>,
    related_commitments: Vec<CommitmentId>,
    last_updated: Timestamp,
}

enum GoalStatus {
    Active,
    OnHold,
    Completed,
    Abandoned,
}
```

---

## 3. Compression Pipeline

### 3.1 Two-Stage Extraction

```rust
struct ConversationCompressor {
    llm: Arc<dyn LlmBackend>,
    embedder: Arc<dyn Embedder>,
    config: CompressionConfig,
}

impl ConversationCompressor {
    async fn compress(&self, turns: &[Turn], project_id: Option<ProjectId>) -> CompressedConversation {
        // Stage 1: Structured extraction (JSON)
        let extraction = self.extract_structured(turns).await?;
        
        // Stage 2: Narrative summary
        let narrative = self.generate_narrative(turns, &extraction).await?;
        
        // Stage 3: Embedding + topics
        let embedding = self.embedder.embed(&narrative).await?;
        let topics = self.extract_topics(&narrative, &extraction);
        
        // Stage 4: Quality validation (optional)
        let quality = if self.config.validate {
            Some(self.validate_quality(turns, &extraction).await?)
        } else { None };
        
        CompressedConversation {
            id: ConversationId::new(),
            project_id,
            user_id: turns.first().map(|t| t.user_id).unwrap(),
            session_ids: turns.iter().map(|t| t.session_id).unique().collect(),
            time_range: (turns.first().unwrap().timestamp, turns.last().unwrap().timestamp),
            narrative,
            decisions: extraction.decisions,
            commitments: extraction.commitments,
            open_questions: extraction.open_questions,
            artifacts: extraction.artifacts,
            goals: extraction.goals,
            embedding,
            key_topics: topics.0,
            topic_weights: topics.1,
            source_turn_ids: turns.iter().map(|t| t.id).collect(),
            compression_ratio: self.calculate_ratio(turns, &narrative),
            compressed_at: now(),
            compression_model: self.config.extraction_model.clone(),
            decision_recall: quality.as_ref().map(|q| q.decision_recall).unwrap_or(0.0),
            commitment_recall: quality.as_ref().map(|q| q.commitment_recall).unwrap_or(0.0),
            artifact_recall: quality.as_ref().map(|q| q.artifact_recall).unwrap_or(0.0),
        }
    }
}
```

### 3.2 Extraction Prompt (Structured)

```markdown
<system>
You are a conversation analyst. Extract structured information from the conversation.
Output ONLY valid JSON matching the schema below. No prose.
</system>

<conversation>
{turns_json}
</conversation>

<schema>
{
  "decisions": [
    {
      "topic": "string",
      "choice": "string", 
      "rationale": "string",
      "alternatives": [{"description": "string", "reason_rejected": "string"}],
      "confidence": 0.0-1.0,
      "status": "Decided|Tentative|Reversed|Superseded",
      "turn_indices": [0, 1, 2]
    }
  ],
  "commitments": [
    {
      "description": "string",
      "owner": "user|agent",
      "deadline": "ISO8601|null",
      "status": "Open|InProgress|Done|Blocked|Cancelled",
      "progress": 0.0-1.0,
      "blockers": ["string"],
      "turn_index": 0
    }
  ],
  "open_questions": [
    {
      "question": "string",
      "context": "string",
      "asked_by": "user|agent",
      "priority": "High|Medium|Low",
      "tentative_answer": "string|null",
      "turn_index": 0
    }
  ],
  "artifacts": [
    {
      "kind": "PullRequest|Commit|File|Config|Diagram|Test|Document|Script|Migration|Other",
      "identifier": "string",
      "description": "string",
      "location": "string|null",
      "turn_index": 0
    }
  ],
  "goals": [
    {
      "description": "string",
      "status": "Active|OnHold|Completed|Abandoned",
      "progress": 0.0-1.0,
      "subgoals": [{"description": "string", "status": "Active|Done"}],
      "turn_indices": [0, 1]
    }
  ]
}
</schema>

<instructions>
- Extract ALL decisions, even tentative ones
- Commitments: only explicit commitments ("I'll do X", "We should do Y")
- Open questions: include unanswered questions from both parties
- Artifacts: any concrete deliverable mentioned (PRs, files, configs, diagrams)
- Goals: high-level objectives with observable progress
- Use turn indices (0-based) for provenance
- Confidence: how certain the choice is (1.0 = explicit agreement)
- Status: Decided (explicit), Tentative (implied), Reversed (explicitly undone)
</instructions>
```

### 3.3 Narrative Generation Prompt

```markdown
<system>
Write a 3-5 sentence narrative summary preserving the CAUSAL CHAIN.
Focus on: problem → investigation → root cause → decision → outcome.
</system>

<conversation>
{turns_json}
</conversation>

<extracted_artifacts>
{extraction_json}
</extracted_artifacts>

<instructions>
- 3-5 sentences MAX
- Include: what happened, root cause, key decision, outcome, open items
- Use specific identifiers (PR numbers, commit hashes, file names)
- Preserve attribution (who did what)
- No hedging ("it seems", "possibly") — state what occurred
</instructions>
```

---

## 4. Semantic Retrieval

### 4.1 Vector Index

```rust
struct ConversationVectorIndex {
    index: HnswIndex<f32>,           // 384-dim, cosine similarity
    metadata: DashMap<ConversationId, ConversationMetadata>,
}

struct ConversationMetadata {
    user_id: UserId,
    project_id: Option<ProjectId>,
    time_range: (Timestamp, Timestamp),
    key_topics: Vec<String>,
    turn_count: u32,
}

impl ConversationVectorIndex {
    async fn add(&self, conv: &CompressedConversation) {
        self.index.add(conv.id, &conv.embedding).await;
        self.metadata.insert(conv.id, ConversationMetadata {
            user_id: conv.user_id,
            project_id: conv.project_id,
            time_range: conv.time_range,
            key_topics: conv.key_topics.clone(),
            turn_count: conv.source_turn_ids.len() as u32,
        });
    }
    
    async fn search(&self, query: &str, user_id: UserId, filters: SearchFilters, k: usize) 
        -> Vec<SearchResult> 
    {
        let q_embedding = self.embedder.embed(query).await;
        let results = self.index.search(&q_embedding, k * 3).await; // over-fetch
        
        results.into_iter()
            .filter(|r| self.filter_match(r.id, user_id, &filters))
            .take(k)
            .map(|r| SearchResult {
                conversation_id: r.id,
                score: r.score,
                narrative: self.get_narrative(r.id).await,
                key_topics: self.metadata.get(&r.id).map(|m| m.key_topics.clone()).unwrap_or_default(),
            })
            .collect()
    }
}
```

### 4.2 Structured Queries (Beyond Semantic)

```rust
impl ConversationStore {
    // "What did we decide about caching?"
    async fn find_decisions(&self, topic: &str, user_id: UserId) -> Vec<DecisionRecord> {
        self.decision_index.search(topic, user_id).await
    }
    
    // "Show my open commitments"
    async fn find_commitments(&self, owner: EntityId, status: CommitmentStatus) -> Vec<Commitment> {
        self.commitment_index.query(owner, status).await
    }
    
    // "Find the PR for the deploy fix"
    async fn find_artifacts(&self, kind: ArtifactKind, query: &str, user_id: UserId) -> Vec<ArtifactRef> {
        self.artifact_index.search(kind, query, user_id).await
    }
    
    // "What are my active goals?"
    async fn find_goals(&self, user_id: UserId, status: GoalStatus) -> Vec<GoalProgress> {
        self.goal_index.query(user_id, status).await
    }
    
    // Cross-conversation: "All decisions about migrations"
    async fn find_decisions_across_projects(&self, topic: &str, user_id: UserId) -> Vec<DecisionRecord> {
        self.decision_index.search_global(topic, user_id).await
    }
}
```

---

## 5. Cross-Session Continuity

### 5.1 Project-Level Aggregation

```rust
struct ProjectConversationMemory {
    project_id: ProjectId,
    user_id: UserId,
    compressed_conversations: Vec<CompressedConversation>,
    
    // Aggregated views (updated incrementally)
    aggregated_decisions: Vec<DecisionRecord>,
    active_commitments: Vec<Commitment>,
    open_questions: Vec<OpenQuestion>,
    key_artifacts: Vec<ArtifactRef>,
    active_goals: Vec<GoalProgress>,
    learned_preferences: UserPreferenceModel,
    
    updated_at: Timestamp,
}

impl ProjectConversationMemory {
    fn merge_new_conversation(&mut self, new: CompressedConversation) {
        // Merge decisions (dedup by topic)
        for decision in new.decisions {
            self.upsert_decision(decision);
        }
        
        // Merge commitments
        for commitment in new.commitments {
            self.upsert_commitment(commitment);
        }
        
        // Update preferences
        self.learned_preferences.merge(new.inferred_preferences);
        
        // Rebuild aggregated views
        self.rebuild_views();
    }
    
    fn get_context_for_new_session(&self) -> NewSessionContext {
        NewSessionContext {
            project_summary: self.generate_project_narrative(),
            active_goals: self.active_goals.clone(),
            open_commitments: self.active_commitments.iter()
                .filter(|c| c.status == CommitmentStatus::Open || c.status == CommitmentStatus::InProgress)
                .cloned()
                .collect(),
            recent_artifacts: self.key_artifacts.iter().take(10).cloned().collect(),
            learned_preferences: self.learned_preferences.clone(),
            suggested_continuation: self.suggest_continuation(),
        }
    }
    
    fn suggest_continuation(&self) -> Option<String> {
        // Priority: blocked commitments → active goals with next steps → open questions
        if let Some(blocked) = self.active_commitments.iter()
            .find(|c| c.status == CommitmentStatus::Blocked) 
        {
            return Some(format!("Blocked: {}. Blocker: {}", blocked.description, blocked.blockers[0]));
        }
        
        if let Some(goal) = self.active_goals.first() {
            return Some(format!("Last time: {}. Next: {}", goal.recent_progress, goal.next_step));
        }
        
        if let Some(q) = self.open_questions.first() {
            return Some(format!("Open question: {}", q.question));
        }
        
        None
    }
}
```

---

## 6. Incremental Compression

### 6.1 Streaming Compression

```rust
struct IncrementalCompressor {
    buffer: Vec<Turn>,
    buffer_token_count: usize,
    flush_threshold: usize,           // e.g., 2000 tokens
    llm: Arc<dyn LlmBackend>,
    embedder: Arc<dyn Embedder>,
}

impl IncrementalCompressor {
    fn add_turn(&mut self, turn: Turn) -> Option<CompressedSegment> {
        self.buffer.push(turn);
        self.buffer_token_count += turn.estimated_tokens();
        
        if self.buffer_token_count >= self.flush_threshold {
            let segment = self.flush_buffer()?;
            return Some(segment);
        }
        None
    }
    
    fn flush_buffer(&mut self) -> Result<CompressedSegment> {
        let turns = std::mem::take(&mut self.buffer);
        self.buffer_token_count = 0;
        
        let extraction = self.extract_structured(&turns)?;
        let narrative = self.generate_narrative(&turns, &extraction)?;
        let embedding = self.embedder.embed(&narrative)?;
        
        Ok(CompressedSegment {
            id: SegmentId::new(),
            turns: turns.clone(),
            narrative,
            extraction,
            embedding,
            token_count: turns.iter().map(|t| t.estimated_tokens()).sum(),
        })
    }
}
```

### 6.2 Segment Merging

```rust
fn merge_segments(segments: &[CompressedSegment]) -> CompressedConversation {
    // Combine narratives with transitions
    let narrative = segments.iter()
        .map(|s| s.narrative.clone())
        .collect::<Vec<_>>()
        .join(" → ");
    
    // Merge extractions (dedup by ID)
    let mut all_decisions = Vec::new();
    let mut seen_decisions = HashSet::new();
    for seg in segments {
        for d in &seg.extraction.decisions {
            if seen_decisions.insert(d.topic.clone()) {
                all_decisions.push(d.clone());
            }
        }
    }
    // ... same for commitments, questions, artifacts, goals
    
    // Re-embed combined narrative
    let embedding = embedder.embed(&narrative)?;
    
    CompressedConversation { ... }
}
```

---

## 7. Quality Validation

### 7.1 Ground Truth Evaluation

```rust
struct CompressionQuality {
    decision_recall: f32,
    decision_precision: f32,
    commitment_recall: f32,
    commitment_precision: f32,
    artifact_recall: f32,
    narrative_faithfulness: f32,     // LLM judge: does narrative match source?
    hallucination_rate: f32,         // artifacts/decisions not in source
}

async fn evaluate_compression(
    original_turns: &[Turn], 
    compressed: &CompressedConversation,
    ground_truth: &GroundTruthAnnotation
) -> CompressionQuality {
    // Decision recall: how many ground truth decisions recovered?
    let decision_recall = compute_recall(
        &ground_truth.decisions,
        &compressed.decisions,
        |a, b| a.topic == b.topic && a.choice == b.choice
    );
    
    // Narrative faithfulness: LLM judge
    let judge_prompt = format!(
        "Rate 1-5: Does this narrative accurately reflect the conversation?\n\nConversation: {}\n\nNarrative: {}",
        turns_to_string(original_turns),
        compressed.narrative
    );
    let score: f32 = llm_judge.complete(judge_prompt).await.parse().unwrap();
    let narrative_faithfulness = score / 5.0;
    
    CompressionQuality { ... }
}
```

### 7.2 Regression Tests

```yaml
# tests/compression/regression.yaml
test_cases:
  - name: "deploy_debugging_session"
    input: "tests/fixtures/deploy_debug.yaml"
    ground_truth:
      decisions:
        - topic: "rollback vs retry"
          choice: "retry with migration fix"
      commitments:
        - description: "Generate rollback migration"
          owner: "agent"
          status: "Done"
      artifacts:
        - kind: PullRequest
          identifier: "PR #247"
    expected_quality:
      decision_recall: 1.0
      commitment_recall: 1.0
      artifact_recall: 1.0
      narrative_faithfulness: 0.9
```

---

## 8. Configuration

```toml
[conversation_compression]
# Trigger
flush_threshold_tokens = 2000
max_segment_age_hours = 24

# Models
extraction_model = "gpt-4o-mini"       # Cheap, good at JSON
narrative_model = "gpt-4o-mini"
embedding_model = "sentence-transformers/all-MiniLM-L6-v2"  # 384-dim

# Quality
validate_every_n = 10                  # Run quality check periodically
min_decision_recall = 0.9
min_commitment_recall = 0.9
min_narrative_faithfulness = 0.85

# Storage
retention_days = 365
vector_index_ef_construction = 200
vector_index_m = 16

# Incremental
segment_overlap_turns = 2              # Overlap for context continuity
```

---

## 9. Privacy & Security

- **PII redaction** before extraction (names, emails, keys, secrets)
- **Per-user encryption** of compressed conversations
- **Project isolation** — user A cannot search user B's conversations
- **Opt-out** — users can disable compression for specific sessions
- **Retention policy** — auto-delete after configurable period

---

## 10. Metrics

| Metric | Target |
|---|---|
| Compression ratio | 15–25x |
| Decision recall | > 0.95 |
| Commitment recall | > 0.95 |
| Artifact recall | > 0.90 |
| Narrative faithfulness | > 0.90 |
| Hallucination rate | < 0.02 |
| Compression latency | < 2s per segment |
| Search latency (p99) | < 100ms |

---

*Structured compression turns ephemeral chat into **queryable organizational memory** — decisions, commitments, and artifacts survive context windows and session boundaries.*