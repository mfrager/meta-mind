# Deterministic Turn Planning

> CME owns conversation structure; LLM fills content slots only.

---

## 1. Motivation

Current failure modes with LLM-controlled structure:
- Rambling responses (5+ sentences per chunk)
- Missed proactive injection points
- Inconsistent pacing (sometimes 1 chunk, sometimes 6)
- LLM ignores "end with question" directives
- No testability of conversation structure

---

## 2. TurnPlan Specification

### 2.1 Core Types

```rust
struct TurnPlan {
    strategy: ResponseStrategy,
    chunks: Vec<PlannedChunk>,
    proactive_injection_points: Vec<usize>,  // indices in chunks where proactive items go
    max_duration_ms: u32,
    requires_user_input: bool,
    metadata: TurnPlanMetadata,
}

struct PlannedChunk {
    id: ChunkId,
    role: ChunkRole,
    directive: String,              // Instruction for LLM: "Acknowledge the deploy failure in 1 sentence"
    max_sentences: u8,              // Hard limit
    min_sentences: u8,              // Soft target
    pause_after_ms: Option<u32>,    // None = no pause, Some(ms) = pause before next chunk
    allow_proactive_injection: bool, // Can a proactive item be inserted here?
}

enum ChunkRole {
    Greeting,                       // "Back at it."
    Acknowledge,                    // "Deploy failed at 14:22."
    Finding,                        // "Root cause: migration timeout on add_index_users_email."
    Clarification,                  // "Faster how? Latency, throughput, or cold-start?"
    OptionPresentation,             // "Options: 1) Profile first 2) Jump to caching"
    NextStep,                       // "Want me to generate the rollback migration?"
    Proactive,                      // Injected proactive item (suggestion, correction, etc.)
    QuestionnaireQuestion,          // "What should I call you?"
    QuestionnaireAck,               // "Got it."
    Summary,                        // "PR #247 opened. Tests passing."
    Correction,                     // "That migration drops a column still used by admin."
    Joke,                           // "Fixed. Your code passes 'works on my machine' cert."
    CheckIn,                        // "Pipeline green. Anything need attention?"
    Continuation,                   // "Back at it. Last time we were on Redis caching."
    Closing,                        // "Let me know if you need anything else."
}

enum ResponseStrategy {
    Reactive { 
        clarification_needed: bool,
        expected_chunks: usize,
    },
    Proactive { 
        item: ProactiveItem,
        lead_with_proactive: bool,
    },
    Hybrid { 
        reactive_chunks: usize,
        proactive_after_chunk: usize,
    },
    Questionnaire { 
        question_index: usize,
        total_questions: usize,
    },
    Introduction { 
        persona: PersonaProfile,
    },
    ErrorRecovery { 
        error: String,
        fallback_strategy: FallbackStrategy,
    },
}

struct TurnPlanMetadata {
    created_at: Timestamp,
    trigger: TurnTrigger,
    estimated_token_count: u32,
    target_latency_ms: u32,
}

enum TurnTrigger {
    UserMessage,
    ProactiveTimer,
    ToolResult,
    Interruption,
    SessionStart,
    QuestionnaireResume,
}
```

### 2.2 TurnPlan Construction

```rust
struct TurnPlanner {
    config: TurnPlannerConfig,
    persona: PersonaProfile,
}

impl TurnPlanner {
    fn build_plan(&self, ctx: &PlanContext) -> TurnPlan {
        let strategy = self.determine_strategy(ctx);
        let chunks = self.build_chunks(&strategy, ctx);
        let proactive_points = self.calculate_proactive_points(&chunks, ctx);
        
        TurnPlan {
            strategy,
            chunks,
            proactive_injection_points: proactive_points,
            max_duration_ms: self.config.max_turn_duration_ms,
            requires_user_input: self.requires_user_input(&strategy),
            metadata: TurnPlanMetadata {
                created_at: now(),
                trigger: ctx.trigger.clone(),
                estimated_token_count: self.estimate_tokens(&chunks),
                target_latency_ms: self.config.target_first_chunk_ms,
            },
        }
    }
    
    fn determine_strategy(&self, ctx: &PlanContext) -> ResponseStrategy {
        match &ctx.trigger {
            TurnTrigger::UserMessage => {
                if ctx.proactive_queue.has_high_priority() {
                    ResponseStrategy::Hybrid {
                        reactive_chunks: 2,
                        proactive_after_chunk: 1,
                    }
                } else if ctx.needs_clarification() {
                    ResponseStrategy::Reactive { 
                        clarification_needed: true, 
                        expected_chunks: 3 
                    }
                } else {
                    ResponseStrategy::Reactive { 
                        clarification_needed: false, 
                        expected_chunks: 2 
                    }
                }
            }
            TurnTrigger::ProactiveTimer => {
                ResponseStrategy::Proactive { 
                    item: ctx.proactive_queue.pop().unwrap(),
                    lead_with_proactive: true,
                }
            }
            TurnTrigger::QuestionnaireResume => {
                ResponseStrategy::Questionnaire {
                    question_index: ctx.questionnaire_progress.current_index,
                    total_questions: ctx.questionnaire_progress.total,
                }
            }
            TurnTrigger::SessionStart => {
                ResponseStrategy::Introduction {
                    persona: ctx.persona.clone(),
                }
            }
            TurnTrigger::Interruption => {
                ResponseStrategy::Reactive {
                    clarification_needed: false,
                    expected_chunks: 1, // Just acknowledgment
                }
            }
            _ => ResponseStrategy::Reactive { clarification_needed: false, expected_chunks: 2 }
        }
    }
}
```

---

## 3. LLM Prompt Template

### 3.1 Structured Prompt Format

```markdown
<system>
You are {persona_name}. {persona_one_liner}.
Communication style: {style_directive} — follow the turn plan exactly.
</system>

<turn_plan>
{turn_plan_json}
</turn_plan>

<context>
User: {user_name} ({role})
Active goal: {goal_summary}
Recent context: {last_3_turns_summary}
Proactive item: {proactive_item_description_if_any}
</context>

<instructions>
Generate content for EACH chunk in the turn plan. 
Output format (exactly):
CHUNK_1: [content for chunk 1]
CHUNK_2: [content for chunk 2]
CHUNK_3: [content for chunk 3]
...

Rules:
- Each chunk content MUST match its role and max_sentences
- Do NOT add extra chunks
- Do NOT combine chunks
- End chunks with questions ONLY where role=Clarification or role=NextStep
- Proactive chunks (role=Proactive) will be replaced by system — generate placeholder
</instructions>
```

### 3.2 Example Turn Plan JSON

```json
{
  "strategy": "Hybrid",
  "chunks": [
    {
      "id": "chunk_1",
      "role": "Acknowledge",
      "directive": "Acknowledge the deploy failure in 1 sentence. Mention the service name.",
      "max_sentences": 1,
      "min_sentences": 1,
      "pause_after_ms": 400,
      "allow_proactive_injection": false
    },
    {
      "id": "chunk_2", 
      "role": "Finding",
      "directive": "State the root cause: migration timeout on add_index_users_email. Include commit hash.",
      "max_sentences": 2,
      "min_sentences": 1,
      "pause_after_ms": 400,
      "allow_proactive_injection": true
    },
    {
      "id": "chunk_3",
      "role": "Proactive",
      "directive": "[PROACTIVE: suggestion about rollback migration]",
      "max_sentences": 2,
      "min_sentences": 1,
      "pause_after_ms": 400,
      "allow_proactive_injection": false
    },
    {
      "id": "chunk_4",
      "role": "NextStep",
      "directive": "Ask: 'Rollback or retry?'",
      "max_sentences": 1,
      "min_sentences": 1,
      "pause_after_ms": null,
      "allow_proactive_injection": false
    }
  ],
  "proactive_injection_points": [1],
  "requires_user_input": true
}
```

### 3.3 LLM Response Parsing

```rust
fn parse_llm_response(response: &str, plan: &TurnPlan) -> Result<Vec<ChunkContent>> {
    let mut chunks = Vec::new();
    
    for (i, planned) in plan.chunks.iter().enumerate() {
        let pattern = format!("CHUNK_{}:", i + 1);
        let content = extract_chunk(response, &pattern)
            .ok_or(ParseError::MissingChunk(i))?;
        
        // Validate sentence count
        let sentence_count = count_sentences(&content);
        if sentence_count > planned.max_sentences as usize {
            // Truncate to max sentences
            let truncated = truncate_to_sentences(&content, planned.max_sentences);
            chunks.push(ChunkContent::Text(truncated));
        } else if sentence_count < planned.min_sentences as usize && planned.min_sentences > 0 {
            // Pad if needed (rare)
            chunks.push(ChunkContent::Text(content));
        } else {
            chunks.push(ChunkContent::Text(content));
        }
    }
    
    Ok(chunks)
}
```

---

## 4. Proactive Injection

### 4.1 Injection at Plan Time

```rust
fn inject_proactive_items(plan: &mut TurnPlan, proactive_queue: &mut ProactiveQueue) {
    for &injection_point in &plan.proactive_injection_points {
        if let Some(item) = proactive_queue.pop_if(|i| i.confidence() > 0.7) {
            // Replace the placeholder chunk with actual proactive content
            if injection_point < plan.chunks.len() {
                plan.chunks[injection_point] = PlannedChunk {
                    id: ChunkId::new(),
                    role: ChunkRole::Proactive,
                    directive: format_proactive_directive(&item),
                    max_sentences: 2,
                    min_sentences: 1,
                    pause_after_ms: Some(500),
                    allow_proactive_injection: false,
                };
            }
        }
    }
}

fn format_proactive_directive(item: &ProactiveItem) -> String {
    match item {
        ProactiveItem::Suggestion { topic, .. } => 
            format!("Suggest: 'Consider {} — want me to prototype?'", topic),
        ProactiveItem::Correction { message, .. } => 
            format!("Correct: '{}'", message),
        ProactiveItem::Compliment { context } => 
            format!("Compliment: '{}'", context),
        ProactiveItem::Joke { context } => 
            format!("Joke: '{}'", context),
        ProactiveItem::CheckIn { message } => 
            format!("Check in: '{}'", message),
        ProactiveItem::GoalNarrowing { options, .. } => 
            format!("Narrow goal: 'Options: {}'", options.join(", ")),
    }
}
```

---

## 5. Validation & Testing

### 5.1 TurnPlan Validation

```rust
fn validate_turn_plan(plan: &TurnPlan) -> Result<(), ValidationError> {
    // Chunk count limits
    if plan.chunks.len() > 6 {
        return Err(ValidationError::TooManyChunks(plan.chunks.len()));
    }
    
    // Sentence limits per chunk
    for (i, chunk) in plan.chunks.iter().enumerate() {
        if chunk.max_sentences > 3 {
            return Err(ValidationError::MaxSentencesExceeded { chunk: i, max: chunk.max_sentences });
        }
        if chunk.min_sentences > chunk.max_sentences {
            return Err(ValidationError::MinExceedsMax { chunk: i });
        }
    }
    
    // Proactive injection points valid
    for &point in &plan.proactive_injection_points {
        if point >= plan.chunks.len() {
            return Err(ValidationError::InvalidInjectionPoint(point));
        }
    }
    
    // Total estimated tokens
    let total_tokens: u32 = plan.chunks.iter()
        .map(|c| c.max_sentences as u32 * 25) // ~25 tokens/sentence
        .sum();
    if total_tokens > 500 {
        return Err(ValidationError::TokenBudgetExceeded(total_tokens));
    }
    
    Ok(())
}
```

### 5.2 Synthetic Tests

```yaml
# tests/turn_plans/hybrid_proactive.yaml
test_case:
  name: "hybrid_proactive_injection"
  context:
    trigger: UserMessage
    proactive_queue:
      - type: Suggestion
        topic: "Redis caching"
        confidence: 0.85
  expected_plan:
    strategy: Hybrid
    chunk_count: 4
    chunk_roles: [Acknowledge, Finding, Proactive, NextStep]
    proactive_injection_points: [1]  # after chunk 1 (0-indexed)
    requires_user_input: true
```

---

## 6. Configuration

```toml
[turn_planner]
max_chunks_per_turn = 4
max_sentences_per_chunk = 3
default_pause_ms = 400
pause_range_ms = { min = 300, max = 800 }
max_turn_duration_ms = 8000
target_first_chunk_ms = 600
proactive_injection_enabled = true
proactive_min_confidence = 0.7
validation_strict = true
```

---

## 7. Migration from Free-Form

| Before | After |
|---|---|
| LLM decides chunking | TurnPlanner decides |
| "End with question" directive | ChunkRole::NextStep with directive |
| Proactive items appended randomly | Injected at `proactive_injection_points` |
| Variable 1–6 chunks | Fixed 1–4 chunks per strategy |
| Untestable structure | TurnPlan validated, parseable |

---

*Turn planning makes conversation structure a **verified contract** between CME and LLM, not a hope.*