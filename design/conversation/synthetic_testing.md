# Synthetic Conversation Testing Framework

> Declarative, deterministic testing of conversation behavior, pacing, proactivity, and voice.

---

## 1. Problem

Manual testing doesn't catch:
- Pacing regressions (wall of text, missing pauses)
- Proactive behavior regressions (missing suggestions, wrong timing)
- Interruption handling bugs
- Voice prosody mismatches
- Cross-session continuity failures

---

## 2. Test DSL (YAML)

### 2.1 Core Test Structure

```yaml
# tests/conversations/vague_goal_narrowing.yaml
test_case:
  id: "vague_goal_narrowing"
  name: "Vague Goal Narrowing"
  description: "Agent narrows vague goal before planning"
  category: "proactivity"
  tags: ["goal_narrowing", "clarification", "pacing"]
  
  # User persona simulates realistic behavior
  user_persona: "impatient_engineer"
  
  # Maximum turns before test fails
  max_turns: 5
  
  # Success criteria (ALL must pass)
  success_criteria:
    - resolves_in_turns: 3
    - no_wall_of_text: true
    - asks_clarification: true
    - proposes_structured_options: true
    - ends_with_question: true
    - proactive_suggestion_count: 1
  
  # Turn-by-turn expectations
  turns:
    - turn: 1
      user:
        message: "Make the API faster"
        latency_ms: 500          # Simulated user response time
        interrupt_at_chunk: null # Or chunk index to interrupt
      expect_agent:
        chunk_count: 2..3        # Range or exact
        contains_any: 
          - "Faster how?"
          - "Latency"
          - "Throughput"
          - "Cold-start"
        not_contains:
          - "I'll optimize"
          - "Let me"
        ends_with_question: true
        max_sentences_per_chunk: 2
        pause_between_chunks_ms: 300..800
        proactive_items: 0
    
    - turn: 2
      user:
        message: "Latency"
        latency_ms: 300
      expect_agent:
        chunk_count: 2..3
        contains_any:
          - "profile first"
          - "caching"
          - "jump to"
        ends_with_question: true
        max_sentences_per_chunk: 2
        proactive_items: 1
        proactive_type: "suggestion"
        proactive_contains: "caching"
    
    - turn: 3
      user:
        message: "Profile first"
        latency_ms: 200
      expect_agent:
        chunk_count: 1..2
        action: "run_profiler"
        ends_with: "results_ready"
        proactive_items: 0

# tests/conversations/interruption_handling.yaml
test_case:
  id: "interruption_mid_stream"
  name: "Interruption Mid-Stream"
  description: "Agent handles user interruption gracefully"
  category: "pacing"
  tags: ["interruption", "barge_in"]
  
  user_persona: "interruptive_user"
  max_turns: 3
  
  success_criteria:
    - immediate_ack: true
    - ack_max_sentences: 1
    - no_apology: true
    - pivots_to_new_topic: true
  
  turns:
    - turn: 1
      user:
        message: "Explain the deployment architecture in detail"
        latency_ms: 100
      expect_agent:
        chunk_count: 4
        streaming: true
        # Interrupt during chunk 2
        interrupt_at: 
          turn: 1
          chunk_index: 1
          interrupt_message: "STOP. Just give me the diagram."
      
      # Expectations for interrupted turn
      interrupted_expectations:
        immediate_ack: true
        ack_text_contains: "OK"
        ack_max_sentences: 1
        no_apology: true
        discards_remaining_chunks: true
    
    - turn: 2
      user:
        message: "STOP. Just give me the diagram."
        latency_ms: 50
      expect_agent:
        chunk_count: 1
        action: "generate_diagram"
        ends_with: "diagram_ready"

# tests/conversations/voice_prosody.yaml
test_case:
  id: "voice_prosody_frustrated"
  name: "Voice Prosody - Frustrated User"
  category: "voice"
  tags: ["prosody", "affect", "tier3"]
  
  modality: "voice"
  voice_tier: 3
  user_persona: "frustrated_user"
  max_turns: 2
  
  success_criteria:
    - affect_detected: "Frustrated"
    - prosody_rate_multiplier: 0.8..0.9
    - prosody_pitch_shift: -3..-1
    - prosody_pause_scaling: 1.2..1.5
    - emphasis_style: "Deliberate"
  
  turns:
    - turn: 1
      user:
        message: "The deploy failed AGAIN. This is the third time this week!"
        modality: "voice"
        audio_features:
          pitch_mean_hz: 220
          pitch_std_dev: 60
          wpm: 190
          rms_db: -5
        latency_ms: 200
      expect_agent:
        affect_detected: "Frustrated"
        prosody:
          rate_multiplier: 0.8..0.9
          pitch_shift_semitones: -3..-1
          pause_scaling: 1.2..1.5
          pitch_range_multiplier: 0.7..0.9
          volume_db: -3..-1
          emphasis_style: "Deliberate"
          voice_quality: "Serious"
        chunk_count: 2..3
        contains_acknowledgment: true
        ends_with_question: true
```

---

## 3. User Personas

```yaml
# tests/personas/impatient_engineer.yaml
persona:
  id: "impatient_engineer"
  name: "Impatient Engineer"
  description: "Senior dev who wants concise, technical answers"
  
  # Response latency distribution (log-normal)
  response_latency_ms:
    distribution: "lognormal"
    mean: 7.6      # log(ms) ≈ 2000ms median
    sigma: 0.5
    min: 100
    max: 30000
  
  # Probability of interrupting agent
  interruption_probability: 0.15
  
  # Verbosity level
  verbosity: "low"
  
  # Technical level
  technical_level: "expert"
  
  # Patience (turns before giving up)
  patience: 3
  
  # Preferred agent style
  preferred_agent_style: "terse"
  
  # Likelihood of explicit corrections
  correction_probability: 0.2

# tests/personas/interruptive_user.yaml
persona:
  id: "interruptive_user"
  name: "Interruptive User"
  response_latency_ms:
    distribution: "lognormal"
    mean: 7.0
    sigma: 0.4
  interruption_probability: 0.5
  verbosity: "low"
  technical_level: "moderate"
  patience: 2
  preferred_agent_style: "direct"
  correction_probability: 0.4

# tests/personas/frustrated_user.yaml
persona:
  id: "frustrated_user"
  name: "Frustrated User"
  response_latency_ms:
    distribution: "lognormal"
    mean: 6.5
    sigma: 0.6
  interruption_probability: 0.3
  verbosity: "high"
  technical_level: "moderate"
  patience: 2
  preferred_agent_style: "empathetic"
  correction_probability: 0.3
  
  # Voice-specific
  voice_features:
    pitch_mean_hz: 220
    pitch_std_dev: 60
    wpm: 190
    rms_db: -5

# tests/personas/curious_learner.yaml
persona:
  id: "curious_learner"
  name: "Curious Learner"
  response_latency_ms:
    distribution: "lognormal"
    mean: 8.5
    sigma: 0.4
  interruption_probability: 0.05
  verbosity: "high"
  technical_level: "beginner"
  patience: 10
  preferred_agent_style: "teaching"
  correction_probability: 0.05
```

---

## 4. Test Runner

```rust
struct ConversationTestRunner {
    cme: Arc<CME>,
    personas: HashMap<String, UserPersonaSimulator>,
    assertion_engine: AssertionEngine,
    metrics_collector: MetricsCollector,
}

struct UserPersonaSimulator {
    persona: UserPersona,
    rng: StdRng,
    turn_count: u32,
}

impl UserPersonaSimulator {
    fn simulate_message(&mut self, expected_turn: &ExpectedTurn, session: &Session) -> UserMessage {
        // Add realistic latency
        let latency = self.sample_latency();
        sleep(Duration::from_millis(latency)).await;
        
        // Check for interruption
        if let Some(interrupt) = expected_turn.interrupt_at {
            if interrupt.turn == expected_turn.turn {
                return UserMessage {
                    content: interrupt.interrupt_message,
                    latency_ms: self.sample_latency(),
                    is_interruption: true,
                    modality: expected_turn.modality.unwrap_or(Modality::Text),
                };
            }
        }
        
        UserMessage {
            content: expected_turn.user.message,
            latency_ms: latency,
            is_interruption: false,
            modality: expected_turn.modality.unwrap_or(Modality::Text),
        }
    }
    
    fn sample_latency(&mut self) -> u64 {
        let dist = LogNormal::new(self.persona.response_latency_ms.mean, self.persona.response_latency_ms.sigma).unwrap();
        dist.sample(&mut self.rng).round() as u64
            .clamp(self.persona.response_latency_ms.min, self.persona.response_latency_ms.max)
    }
}

impl ConversationTestRunner {
    async fn run(&self, test: &TestCase) -> TestResult {
        let mut session = self.cme.create_session(test.user_persona).await;
        let mut simulator = self.personas.get(&test.user_persona).unwrap().clone();
        let mut turn_results = Vec::new();
        
        for expected_turn in &test.turns {
            // Simulate user message
            let user_msg = simulator.simulate_message(expected_turn, &session);
            
            // Process through CME
            let agent_response = session.process_message(user_msg).await;
            
            // If interruption specified in test
            if let Some(interrupt) = expected_turn.interrupt_at {
                if interrupt.turn == expected_turn.turn {
                    // Send interrupt message
                    let interrupt_msg = UserMessage {
                        content: interrupt.interrupt_message,
                        latency_ms: simulator.sample_latency(),
                        is_interruption: true,
                        modality: Modality::Text,
                    };
                    let _ = session.process_message(interrupt_msg).await;
                }
            }
            
            // Run assertions
            let assertions = self.assertion_engine.check(&agent_response, expected_turn, &session);
            turn_results.push(TurnResult {
                turn: expected_turn.turn,
                assertions,
                agent_response: agent_response.clone(),
            });
            
            // Check if test should continue
            if assertions.iter().any(|a| a.is_fatal_failure()) {
                break;
            }
        }
        
        // Check global success criteria
        let global_result = self.check_global_criteria(test, &turn_results, &session);
        
        TestResult {
            test_id: test.id.clone(),
            passed: global_result.passed && turn_results.iter().all(|r| r.assertions.iter().all(|a| a.passed)),
            turn_results,
            global_assertions: global_result.assertions,
            metrics: self.metrics_collector.collect(&session, &turn_results),
        }
    }
}
```

---

## 5. Assertion Engine

```rust
struct AssertionEngine {
    checkers: Vec<Box<dyn AssertionChecker>>,
}

trait AssertionChecker: Send + Sync {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, session: &Session) -> Vec<AssertionResult>;
}

struct ChunkCountChecker;
impl AssertionChecker for ChunkCountChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        let expected = expected.expect_agent.chunk_count;
        let actual = response.chunks.len();
        let passed = match expected {
            ExpectedRange::Exact(n) => actual == n,
            ExpectedRange::Range(min, max) => actual >= min && actual <= max,
        };
        vec![AssertionResult {
            name: "chunk_count",
            passed,
            expected: format!("{:?}", expected),
            actual: actual.to_string(),
            fatal: false,
        }]
    }
}

struct ContentContainsChecker;
impl AssertionChecker for ContentContainsChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        let full_text = response.chunks.iter().map(|c| c.text()).collect::<String>();
        let mut results = Vec::new();
        
        for phrase in &expected.expect_agent.contains_any {
            let passed = full_text.contains(phrase);
            results.push(AssertionResult {
                name: format!("contains_{}", phrase.replace(' ', "_")),
                passed,
                expected: phrase.clone(),
                actual: if passed { "found".into() } else { "not found".into() },
                fatal: false,
            });
        }
        
        for phrase in &expected.expect_agent.not_contains {
            let passed = !full_text.contains(phrase);
            results.push(AssertionResult {
                name: format!("not_contains_{}", phrase.replace(' ', "_")),
                passed,
                expected: format!("not {}", phrase),
                actual: if passed { "absent".into() } else { "found (unexpected)".into() },
                fatal: false,
            });
        }
        
        results
    }
}

struct EndsWithQuestionChecker;
impl AssertionChecker for EndsWithQuestionChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        if !expected.expect_agent.ends_with_question {
            return vec![];
        }
        
        let last_chunk = response.chunks.last().map(|c| c.text()).unwrap_or("");
        let passed = last_chunk.trim_end().ends_with('?');
        
        vec![AssertionResult {
            name: "ends_with_question",
            passed,
            expected: "true".into(),
            actual: passed.to_string(),
            fatal: true,  // Critical for conversation flow
        }]
    }
}

struct MaxSentencesChecker;
impl AssertionChecker for MaxSentencesChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        if let Some(max) = expected.expect_agent.max_sentences_per_chunk {
            let mut results = Vec::new();
            for (i, chunk) in response.chunks.iter().enumerate() {
                let count = count_sentences(&chunk.text());
                let passed = count <= max;
                results.push(AssertionResult {
                    name: format!("max_sentences_chunk_{}", i),
                    passed,
                    expected: format!("≤ {}", max),
                    actual: count.to_string(),
                    fatal: false,
                });
            }
            results
        } else { vec![] }
    }
}

struct PauseTimingChecker;
impl AssertionChecker for PauseTimingChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        if let Some(range) = expected.expect_agent.pause_between_chunks_ms {
            let mut results = Vec::new();
            for i in 1..response.chunks.len() {
                let pause = response.chunks[i].timestamp_ms - response.chunks[i-1].timestamp_ms;
                let passed = pause >= range.0 && pause <= range.1;
                results.push(AssertionResult {
                    name: format!("pause_chunk_{}_{}", i-1, i),
                    passed,
                    expected: format!("{}..{} ms", range.0, range.1),
                    actual: format!("{} ms", pause),
                    fatal: false,
                });
            }
            results
        } else { vec![] }
    }
}

struct ProactiveItemChecker;
impl AssertionChecker for ProactiveItemChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        let expected_count = expected.expect_agent.proactive_items.unwrap_or(0);
        let actual_count = response.chunks.iter()
            .filter(|c| matches!(c.content, ChunkContent::Structured(_)))
            .count();
        
        let count_passed = actual_count == expected_count;
        let mut results = vec![AssertionResult {
            name: "proactive_count",
            passed: count_passed,
            expected: expected_count.to_string(),
            actual: actual_count.to_string(),
            fatal: false,
        }];
        
        if expected_count > 0 {
            if let Some(expected_type) = expected.expect_agent.proactive_type {
                let type_passed = response.chunks.iter().any(|c| {
                    if let ChunkContent::Structured(StructuredCard::Suggestion { .. }) = &c.content {
                        expected_type == "suggestion"
                    } else if let ChunkContent::Structured(StructuredCard::Correction { .. }) = &c.content {
                        expected_type == "correction"
                    } else { false }
                });
                results.push(AssertionResult {
                    name: "proactive_type",
                    passed: type_passed,
                    expected: expected_type,
                    actual: if type_passed { "matched" } else { "mismatch" }.into(),
                    fatal: false,
                });
            }
            
            if let Some(contains) = expected.expect_agent.proactive_contains {
                let content_passed = response.chunks.iter().any(|c| c.text().contains(contains));
                results.push(AssertionResult {
                    name: "proactive_contains",
                    passed: content_passed,
                    expected: contains,
                    actual: if content_passed { "found" } else { "not found" }.into(),
                    fatal: false,
                });
            }
        }
        
        results
    }
}

struct ActionChecker;
impl AssertionChecker for ActionChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        if let Some(expected_action) = expected.expect_agent.action {
            let has_action = response.chunks.iter().any(|c| {
                matches!(c.content, ChunkContent::Structured(StructuredCard::ActionOffer { .. }))
            });
            vec![AssertionResult {
                name: "action",
                passed: has_action,
                expected: expected_action,
                actual: if has_action { "found" } else { "missing" }.into(),
                fatal: false,
            }]
        } else { vec![] }
    }
}

struct ProsodyChecker;
impl AssertionChecker for ProsodyChecker {
    fn check(&self, response: &AgentResponse, expected: &ExpectedTurn, _: &Session) -> Vec<AssertionResult> {
        let mut results = Vec::new();
        
        if let Some(prosody_expect) = &expected.expect_agent.prosody {
            let actual = response.prosody_profile.as_ref().unwrap();
            
            macro_rules! check_range {
                ($field:ident, $expected:expr) => {{
                    let expected = $expected;
                    let actual = actual.$field;
                    let (min, max) = match expected {
                        ExpectedRange::Exact(v) => (v, v),
                        ExpectedRange::Range(min, max) => (min, max),
                    };
                    let passed = actual >= min && actual <= max;
                    results.push(AssertionResult {
                        name: stringify!($field).to_string(),
                        passed,
                        expected: format!("{}..{}", min, max),
                        actual: actual.to_string(),
                        fatal: false,
                    });
                }};
            }
            
            check_range!(rate_multiplier, prosody_expect.rate_multiplier);
            check_range!(pitch_shift_semitones, prosody_expect.pitch_shift);
            check_range!(pause_scaling, prosody_expect.pause_scaling);
            check_range!(pitch_range_multiplier, prosody_expect.pitch_range);
            check_range!(volume_db, prosody_expect.volume);
            
            if let Some(style) = &prosody_expect.emphasis_style {
                let passed = actual.emphasis_style == *style;
                results.push(AssertionResult {
                    name: "emphasis_style",
                    passed,
                    expected: format!("{:?}", style),
                    actual: format!("{:?}", actual.emphasis_style),
                    fatal: false,
                });
            }
            
            if let Some(quality) = &prosody_expect.voice_quality {
                let passed = actual.voice_quality == *quality;
                results.push(AssertionResult {
                    name: "voice_quality",
                    passed,
                    expected: format!("{:?}", quality),
                    actual: format!("{:?}", actual.voice_quality),
                    fatal: false,
                });
            }
        }
        
        if let Some(affect) = &expected.expect_agent.affect_detected {
            let passed = response.detected_affect == *affect;
            results.push(AssertionResult {
                name: "affect_detected",
                passed,
                expected: format!("{:?}", affect),
                actual: format!("{:?}", response.detected_affect),
                fatal: true,
            });
        }
        
        results
    }
}
```

---

## 6. Global Success Criteria

```rust
struct GlobalCriteriaChecker;

impl GlobalCriteriaChecker {
    fn check(&self, test: &TestCase, turn_results: &[TurnResult], session: &Session) -> GlobalResult {
        let mut assertions = Vec::new();
        
        for criterion in &test.success_criteria {
            match criterion {
                SuccessCriterion::ResolvesInTurns { max_turns } => {
                    let passed = turn_results.len() <= *max_turns;
                    assertions.push(AssertionResult {
                        name: "resolves_in_turns",
                        passed,
                        expected: format!("≤ {}", max_turns),
                        actual: turn_results.len().to_string(),
                        fatal: true,
                    });
                }
                
                SuccessCriterion::NoWallOfText => {
                    let max_chunk_sentences = turn_results.iter()
                        .flat_map(|r| &r.agent_response.chunks)
                        .map(|c| count_sentences(&c.text()))
                        .max()
                        .unwrap_or(0);
                    let passed = max_chunk_sentences <= 3;
                    assertions.push(AssertionResult {
                        name: "no_wall_of_text",
                        passed,
                        expected: "≤ 3 sentences per chunk".into(),
                        actual: format!("max {} sentences", max_chunk_sentences),
                        fatal: true,
                    });
                }
                
                SuccessCriterion::AsksClarification => {
                    let asked = turn_results.iter().any(|r| 
                        r.agent_response.chunks.iter().any(|c| c.text().contains('?'))
                    );
                    assertions.push(AssertionResult {
                        name: "asks_clarification",
                        passed: asked,
                        expected: "at least one question".into(),
                        actual: if asked { "found" } else { "none" }.into(),
                        fatal: false,
                    });
                }
                
                SuccessCriterion::ProposesStructuredOptions => {
                    let proposed = turn_results.iter().any(|r| 
                        r.agent_response.chunks.iter().any(|c| {
                            matches!(c.content, ChunkContent::Structured(StructuredCard::BrainstormOptions { .. }))
                        })
                    );
                    assertions.push(AssertionResult {
                        name: "proposes_structured_options",
                        passed: proposed,
                        expected: "structured options card".into(),
                        actual: if proposed { "found" } else { "missing" }.into(),
                        fatal: false,
                    });
                }
                
                SuccessCriterion::EndsWithQuestion => {
                    let last_turn = turn_results.last();
                    let ends_with_q = last_turn.map(|t| 
                        t.agent_response.chunks.last().map(|c| c.text().trim_end().ends_with('?')).unwrap_or(false)
                    ).unwrap_or(false);
                    assertions.push(AssertionResult {
                        name: "ends_with_question",
                        passed: ends_with_q,
                        expected: "true".into(),
                        actual: ends_with_q.to_string(),
                        fatal: true,
                    });
                }
                
                SuccessCriterion::ProactiveSuggestionCount { count } => {
                    let actual = turn_results.iter()
                        .flat_map(|r| &r.agent_response.chunks)
                        .filter(|c| matches!(c.content, ChunkContent::Structured(StructuredCard::Suggestion { .. })))
                        .count();
                    assertions.push(AssertionResult {
                        name: "proactive_suggestion_count",
                        passed: actual == *count,
                        expected: count.to_string(),
                        actual: actual.to_string(),
                        fatal: false,
                    });
                }
            }
        }
        
        GlobalResult {
            passed: assertions.iter().all(|a| a.passed),
            assertions,
        }
    }
}
```

---

## 7. CI Integration

```yaml
# .github/workflows/conversation-tests.yml
name: Conversation Tests
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      
      - name: Cache cargo
        uses: Swatinem/rust-cache@v2
      
      - name: Build test binary
        run: cargo build --test conversation_tests --release
      
      - name: Run conversation tests
        run: |
          cargo test --test conversation_tests --release -- --format=json > test_results.json
      
      - name: Generate HTML report
        run: |
          cargo run --bin test_report -- test_results.json > test_report.html
        continue-on-error: true
      
      - name: Upload test results
        uses: actions/upload-artifact@v4
        with:
          name: conversation-test-results
          path: |
            test_results.json
            test_report.html
      
      - name: Check for regressions
        run: |
          cargo run --bin regression_check -- test_results.json baseline_results.json
        continue-on-error: true

  # Nightly extended tests
  nightly:
    runs-on: ubuntu-latest
    if: github.event_name == 'schedule'
    steps:
      - uses: actions/checkout@v4
      - name: Run extended test suite
        run: cargo test --test conversation_tests --release -- extended
```

---

## 8. Test Discovery & Organization

```
tests/
├── conversations/
│   ├── proactivity/
│   │   ├── vague_goal_narrowing.yaml
│   │   ├── background_lookup.yaml
│   │   ├── suggestion_after_repeated_topic.yaml
│   │   └── correction_on_anti_pattern.yaml
│   ├── pacing/
│   │   ├── chunk_limits.yaml
│   │   ├── pause_timing.yaml
│   │   ├── interruption_mid_stream.yaml
│   │   └── interruption_during_tts.yaml
│   ├── voice/
│   │   ├── prosody_frustrated.yaml
│   │   ├── prosody_excited.yaml
│   │   ├── tier1_ssml.yaml
│   │   ├── tier2_streaming.yaml
│   │   ├── tier3_realtime.yaml
│   │   └── barge_in.yaml
│   ├── questionnaires/
│   │   ├── intro_name_only.yaml
│   │   ├── full_onboarding.yaml
│   │   └── skip_questionnaire.yaml
│   └── cross_session/
│       ├── project_continuity.yaml
│       └── preference_persistence.yaml
├── personas/
│   ├── impatient_engineer.yaml
│   ├── interruptive_user.yaml
│   ├── frustrated_user.yaml
│   ├── curious_learner.yaml
│   └── casual_user.yaml
└── fixtures/
    ├── audio_samples/
    └── conversation_logs/
```

---

## 9. Metrics Collection

```rust
struct MetricsCollector;

impl MetricsCollector {
    fn collect(&self, session: &Session, turn_results: &[TurnResult]) -> TestMetrics {
        TestMetrics {
            total_turns: turn_results.len(),
            total_chunks: turn_results.iter().map(|r| r.agent_response.chunks.len()).sum(),
            avg_chunks_per_turn: turn_results.iter().map(|r| r.agent_response.chunks.len()).sum::<usize>() as f32 / turn_results.len().max(1) as f32,
            avg_sentences_per_chunk: turn_results.iter()
                .flat_map(|r| &r.agent_response.chunks)
                .map(|c| count_sentences(&c.text()))
                .sum::<usize>() as f32 / turn_results.iter().map(|r| r.agent_response.chunks.len()).sum::<usize>().max(1) as f32,
            total_pauses_ms: turn_results.iter()
                .flat_map(|r| r.agent_response.chunks.windows(2))
                .map(|w| w[1].timestamp_ms - w[0].timestamp_ms)
                .sum(),
            proactive_items_total: turn_results.iter()
                .flat_map(|r| &r.agent_response.chunks)
                .filter(|c| matches!(c.content, ChunkContent::Structured(_)))
                .count(),
            interruptions_handled: turn_results.iter()
                .filter(|r| r.agent_response.was_interrupted)
                .count(),
            first_chunk_latency_p50: percentile(turn_results.iter().map(|r| r.agent_response.first_chunk_latency_ms).collect(), 0.5),
            first_chunk_latency_p99: percentile(turn_results.iter().map(|r| r.agent_response.first_chunk_latency_ms).collect(), 0.99),
            voice_metrics: turn_results.iter()
                .filter_map(|r| r.agent_response.voice_metrics.as_ref())
                .map(|m| m.clone())
                .collect(),
        }
    }
}
```

---

## 10. Regression Detection

```rust
struct RegressionDetector {
    baseline: HashMap<String, TestMetrics>,
    thresholds: RegressionThresholds,
}

struct RegressionThresholds {
    max_latency_increase_pct: f32,      // 20%
    max_chunk_increase_pct: f32,        // 15%
    min_proactive_decrease_pct: f32,    // 10%
    max_sentence_increase_pct: f32,     // 10%
}

impl RegressionDetector {
    fn check(&self, current: &TestMetrics, test_id: &str) -> Vec<RegressionAlert> {
        let mut alerts = Vec::new();
        let baseline = self.baseline.get(test_id).unwrap();
        
        let latency_increase = (current.first_chunk_latency_p50 - baseline.first_chunk_latency_p50) as f32 / baseline.first_chunk_latency_p50 as f32;
        if latency_increase > self.thresholds.max_latency_increase_pct {
            alerts.push(RegressionAlert {
                test_id: test_id.into(),
                metric: "first_chunk_latency_p50".into(),
                baseline: baseline.first_chunk_latency_p50,
                current: current.first_chunk_latency_p50,
                change_pct: latency_increase * 100.0,
                severity: AlertSeverity::Warning,
            });
        }
        
        // ... similar for other metrics
        
        alerts
    }
}
```

---

*Synthetic testing makes conversation quality **measurable, regression-proof, and CI-integrable** — every behavior change is validated against realistic user simulations.*