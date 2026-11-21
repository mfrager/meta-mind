# Conversation Management Engine (CME)

> A stream-native, proactive conversation runtime that sits between UIs and LLM backends. It owns the conversation lifecycle, pacing, and agent initiative — the LLM becomes a *generative subroutine*, not the conversation driver.

---

## 1. Architectural Position

```
┌─────────────┐     ┌──────────────────────┐     ┌────────────────┐
│   UI(s)     │────▶│ Conversation Mgmt    │────▶│  LLM Backend(s)│
│  (Web, CLI, │     │     Engine (CME)     │     │  (OpenAI,      │
│   Telegram) │◀────│                      │◀────│   Local, etc.) │
└─────────────┘     └──────────────────────┘     └────────────────┘
                           │
                    ┌──────┴──────┐
                    ▼             ▼
              ┌─────────┐  ┌─────────────┐
              │ State   │  │ Tool /      │
              │ Store   │  │ Action Bus  │
              └─────────┘  └─────────────┘
```

**Key inversion:** Typical LLM chat = UI → LLM → UI.  
**CME model:** UI ↔ CME ↔ LLM. The CME *drives*; the LLM *responds to structured prompts* from the CME.

---

## 2. Core Responsibilities

| Responsibility | Description |
|---|---|
| **Conversation lifecycle** | Session create/resume/timeout, introduction, questionnaire flows, graceful shutdown |
| **Turn orchestration** | Decide when agent speaks, what it says, in what chunks, with what pauses |
| **Proactive initiative** | Background lookups, scheduled check-ins, suggestion triggers, goal-narrowing prompts |
| **Pacing & streaming** | Chunk segmentation, variable delays, natural pause points, backpressure handling |
| **Interruption handling** | Preempt queued chunks, acknowledge, pivot instantly |
| **State management** | User model, proactive state, conversation history, questionnaire progress |
| **LLM prompt assembly** | Build structured prompts from conversation context + CME directives + tool results |
| **Output enrichment** | Inject non-LLM elements: tool results, system notifications, structured cards, typing indicators |
| **Multi-UI coordination** | Single conversation state across Web, Telegram, CLI — seamless handoff |

---

## 3. Data Model

### 3.1 Conversation Session
```rust
struct ConversationSession {
    id: SessionId,
    user_id: UserId,
    ui_channels: Vec<UiChannel>,        // web, telegram, cli
    state: SessionState,                // Active, Questionnaire, Paused, Ended
    created_at: Timestamp,
    last_activity: Timestamp,
    turn_count: u32,
    current_questionnaire: Option<QuestionnaireProgress>,
    proactive_state: ProactiveState,
    pace_preference: PacePreference,
    persona: PersonaProfile,
}
```

### 3.2 Turn & Chunk Model
```rust
enum TurnOwner { User, Agent }

struct Turn {
    id: TurnId,
    session_id: SessionId,
    owner: TurnOwner,
    chunks: Vec<Chunk>,
    status: TurnStatus,                 // Streaming, Complete, Interrupted, Cancelled
    metadata: TurnMetadata,             // trigger_type, proactive_items_used, etc.
}

struct Chunk {
    id: ChunkId,
    turn_id: TurnId,
    content: ChunkContent,
    delay_ms: u32,                      // delay before this chunk streams
    is_final: bool,
}

enum ChunkContent {
    Text(String),                       // plain agent speech
    Structured(StructuredCard),         // tool result, suggestion card, questionnaire question
    System(SystemEvent),                // typing_start, typing_stop, interruption_ack
    Control(ControlSignal),             // pause, yield, resume
}
```

### 3.3 Structured Cards (Non-Text Output)
```rust
enum StructuredCard {
    Suggestion { title: String, description: String, action: ActionRef },
    ToolResult { tool: String, summary: String, data: Value },
    QuestionnaireQuestion { q: Question, progress: (usize, usize) },
    Confirmation { prompt: String, confirm_action: ActionRef, cancel_action: ActionRef },
    BrainstormOptions { topic: String, options: Vec<String> },
    PlanPreview { steps: Vec<PlanStep>, current: usize },
}
```

### 3.4 Proactive Queue
```rust
struct ProactiveQueue {
    items: VecDeque<ProactiveItem>,
    max_per_turn: u8,                   // usually 1
    cooldown_turns: u32,
}

enum ProactiveItem {
    BackgroundLookup { trigger: String, result: LookupResult },
    Suggestion { topic: String, confidence: f32 },
    Correction { severity: Severity, message: String },
    Compliment { context: String },
    Joke { context: String },
    CheckIn { scheduled_for: Timestamp },
    GoalNarrowing { vague_goal: String, options: Vec<String> },
}
```

---

## 4. Runtime Loop (Event-Driven)

```
┌─────────────────────────────────────────────────────────────┐
│                    CME EVENT LOOP                           │
├─────────────────────────────────────────────────────────────┤
│  while session.active:                                      │
│      event = wait_for_event()                               │
│      match event:                                           │
│          UserMessage(msg)        -> handle_user_message(msg)│
│          UiChannelConnected(ch)  -> handle_reconnect(ch)    │
│          ProactiveTimerFired     -> handle_proactive_timer()│
│          ToolResultReady(res)    -> handle_tool_result(res) │
│          LlmChunkReceived(chunk) -> handle_llm_chunk(chunk) │
│          InterruptSignal         -> handle_interrupt()      │
│          SessionTimeout          -> handle_timeout()        │
└─────────────────────────────────────────────────────────────┘
```

### 4.1 User Message Handling
```
handle_user_message(msg):
    1. Create UserTurn with single chunk
    2. Stream UserTurn to all UI channels immediately
    3. Check for interruption of agent turn → preempt if needed
    4. Update proactive_state (topic_frequency, interruption_count)
    5. If questionnaire active → route to questionnaire handler
    6. Else → build_agent_response()
```

### 4.2 Agent Response Building (The Core)
```
build_agent_response():
    1. Determine response_strategy:
         - Reactive: answer question, continue topic
         - Proactive: pop from proactive_queue (if cooldown elapsed)
         - Hybrid: reactive + 1 proactive item
    2. Assemble LLM prompt:
         - System prompt (persona, directives, pacing rules)
         - Conversation history (compressed, relevant window)
         - Current context (user model, goals, recent tool results)
         - CME directives: "Stream in 2-3 chunks. End with question."
    3. Send to LLM backend (streaming)
    4. As chunks arrive → segment → wrap in ChunkContent::Text
    4. Inject structured cards between/after LLM chunks as needed
    5. Apply pacing delays (variable 300-800ms)
    6. Stream to all UI channels
```

### 4.3 Interruption Handling
```
handle_interrupt():
    1. Cancel current LLM stream (send abort to backend)
    2. Discard queued chunks for current agent turn
    3. Create SystemEvent::InterruptionAcknowledged
    4. Stream acknowledgment chunk immediately ("OK." / "Got it.")
    5. Return to handle_user_message() with new input
```

### 4.4 Proactive Timer
```
handle_proactive_timer():
    1. Evaluate triggers (background lookup ready, check-in due, etc.)
    2. If proactive_queue not empty and cooldown elapsed:
         - Pop item → wrap as structured card or LLM directive
         - Build agent turn with proactive item as lead
    3. Else if session idle > threshold:
         - Generate check-in: "Pipeline green. Anything need attention?"
    4. Reschedule next timer
```

---

## 5. LLM Prompt Assembly Strategy

The CME constructs **structured prompts** that the LLM executes. The LLM never sees raw conversation history — it receives a curated context window.

### 5.1 Prompt Template
```markdown
<system>
You are {persona_name}. {persona_one_liner}.
Communication style: {style_directive} — ≤3 sentences per chunk. One idea per chunk.
End each turn with a question or clear next step unless user said "just do it".
</system>

<context>
User: {user_name} ({role}, {focus_domain})
Relationship: {familiarity}/10, trust {trust}/10
Active goal: {goal_summary or "none"}
Recent context: {last_3_turns_summary}
Proactive context: {proactive_item_description if any}
</context>

<directives>
- Stream response in {chunk_count} chunks.
- Pause points: after each complete thought, before questions.
- If proactive item included: lead with it naturally.
- Available tools: {tool_list}. Call via TOOL_CALL blocks if needed.
- Output format: plain text chunks. Structured cards injected by CME.
</directives>

<conversation>
{relevant_history}
User: {latest_user_message}
</conversation>
```

### 5.2 Tool Calling Protocol
LLM outputs special blocks:
```
TOOL_CALL: { "tool": "search_code", "args": { "query": "N+1 query" } }
```
CME intercepts, executes, streams `StructuredCard::ToolResult`, then resumes LLM with result injected.

---

## 6. Streaming Protocol (UI ↔ CME)

### 6.1 SSE / WebSocket Frame Format
```json
{
  "event": "chunk",
  "session_id": "sess_abc123",
  "turn_id": "turn_456",
  "chunk_id": "chunk_789",
  "payload": {
    "type": "text",
    "content": "That query has a sequential scan on users.email."
  },
  "meta": {
    "delay_ms": 400,
    "is_final": false
  }
}
```

### 6.2 Chunk Types
| Type | Payload | UI Behavior |
|---|---|---|
| `text` | `{ "content": "..." }` | Append to message bubble, typewriter effect |
| `structured_card` | `{ "card_type": "suggestion", "data": {...} }` | Render card component inline |
| `system` | `{ "event": "typing_start" }` | Show typing indicator |
| `control` | `{ "signal": "yield" }` | Pause rendering, wait for resume |

### 6.3 Multi-Channel Delivery
- Single turn → fan out to all connected UI channels for session
- Each channel renders natively (Web: React components, Telegram: Markdown + inline keyboards, CLI: colored text)
- Channel-specific adapters transform `StructuredCard` → native UI

---

## 7. Questionnaire Engine

### 7.1 Questionnaire State Machine
```
Idle → Introduction → Question_1 → Answer_1 → Question_2 → ... → Complete → Summary → Idle
                          ↑                                    ↑
                          └──────── Interrupt / Skip ────────┘
```

### 7.2 Progress Persistence
```rust
struct QuestionnaireProgress {
    questionnaire_id: String,
    current_index: usize,
    answers: HashMap<String, Value>,
    started_at: Timestamp,
    can_resume: bool,
}
```
- Survives session disconnect/reconnect
- Partial answers retained on interrupt
- "Skip" stores null, "Later" pauses with `can_resume = true`

---

## 8. Introduction & Onboarding Flow

### 8.1 New Session (No User Model)
```
SessionStart
    → Load/Create ConversationSession
    → If first_ever:
         → Queue IntroQuestionnaire (name_only)
         → Send in-character introduction chunk
    → Else if returning:
         → Send contextual greeting ("Back at it. Still on auth refactor?")
    → Stream to UI
```

### 8.2 Persona Selection
- Persona determined by: user model `preferred_persona`, project context, or explicit UI selection
- Persona drives: introduction text, system prompt, pacing parameters, proactive tendencies

---

## 9. Configuration & Tuning

### 9.1 Per-Session Config (Merged: User Model + Project + UI Defaults)
```toml
[cme]
# Pacing
chunk_target_sentences = 3
inter_chunk_delay_ms = { min = 300, max = 800 }
max_chunks_per_turn = 4
variable_delay_jitter = true

# Proactivity
proactive_enabled = true
proactive_cooldown_turns = 3
background_lookup_timeout_ms = 5000
suggestion_confidence_threshold = 0.7
check_in_idle_threshold_min = 30

# Interruption
interrupt_preempt_llm = true
interrupt_ack_max_sentences = 1

# Questionnaire
auto_start_onboarding = true
onboarding_questionnaire = "onboarding_v1"
intro_questionnaire = "intro_name_only"
resume_partial_questionnaire = true

# LLM
llm_backend = "openai"          # or "local", "anthropic"
llm_model = "gpt-4o"
llm_temperature = 0.4
llm_max_tokens_per_chunk = 200
prompt_compression = "summarize_old"  # or "sliding_window", "none"

# Multi-UI
sync_typing_indicators = true
delivery_timeout_ms = 10000
```

### 9.2 Pace Adaptation (Runtime)
```rust
fn adapt_pace(state: &mut ProactiveState, user_response_latency_ms: u64, interruption: bool) {
    if interruption { state.user_interruption_count += 1; }
    
    // Fast user: short responses, quick replies
    if user_response_latency_ms < 2000 && state.user_interruption_count > 2 {
        state.preferred_pace = PacePreference::Fast;
    }
    // Deliberate user: long responses, no interruptions
    else if user_response_latency_ms > 15000 && state.user_interruption_count == 0 {
        state.preferred_pace = PacePreference::Deliberate;
    }
    
    // Apply to config
    match state.preferred_pace {
        Fast       => { config.inter_chunk_delay_ms = (200, 400); config.max_chunks_per_turn = 2; }
        Normal     => { config.inter_chunk_delay_ms = (300, 800); config.max_chunks_per_turn = 4; }
        Deliberate => { config.inter_chunk_delay_ms = (600, 1200); config.max_chunks_per_turn = 6; }
    }
}
```

---

## 10. State Persistence

| Store | Contents | TTL |
|---|---|---|
| **SQLite (primary)** | Sessions, turns, chunks, questionnaires, user models | Permanent |
| **Redis (ephemeral)** | Active streaming state, turn queues, typing indicators | Session + 1hr |
| **Vector index** | Conversation embeddings for context retrieval | Permanent |

### 10.1 Replay & Recovery
- Full conversation replay from `turns` + `chunks` tables
- On reconnect: fetch last N turns, re-stream any unacknowledged chunks
- `X-Request-Id` correlation for deduplication

---

## 11. Integration Points

### 11.1 LLM Backend Adapter Interface
```rust
trait LlmBackend: Send + Sync {
    fn stream_completion(&self, prompt: Prompt, tx: mpsc::Sender<LlmEvent>) -> Result<AbortHandle>;
    fn count_tokens(&self, text: &str) -> usize;
    fn supports_tools(&self) -> bool;
}

enum LlmEvent {
    Chunk(String),
    ToolCall(ToolCall),
    Done(FinishReason),
    Error(Error),
}
```

### 11.2 Tool/Action Bus
```rust
trait ToolExecutor: Send + Sync {
    fn execute(&self, call: ToolCall) -> ToolResult;
    fn list_available(&self) -> Vec<ToolSpec>;
}
```
- Tools registered at CME startup
- Execution is async, results fed back via `ToolResultReady` event
- Sandbox isolation per session (fs, network, process limits)

### 11.3 UI Channel Adapters
| Channel | Adapter Responsibilities |
|---|---|
| Web (React) | SSE connection, component rendering for cards, optimistic UI |
| Telegram | Bot API, inline keyboards for choices, MarkdownV2 formatting |
| CLI | TTY streaming, ANSI colors, readline integration |

---

## 12. Voice Interface Support

The CME treats voice as a **first-class modality** with three architectural tiers, each with different latency, naturalness, and complexity tradeoffs.

### 12.1 Tier 1: Request/Response Pipeline (ASR → LLM → TTS)

Traditional turn-based voice. Simple, robust, works with any LLM backend.

```
User Speech
    │
    ▼
┌─────────────┐
│   ASR       │  (Whisper, Deepgram, Vosk, cloud APIs)
│  (STT)      │
└──────┬──────┘
       │ text
       ▼
┌────────────────────────────────────┐
│        CME (Text Mode)             │
│  • Build prompt                    │
│  • Call LLM (non-streaming or      │
│    streaming → collect full text)  │
│  • Post-process (SSML, pauses)     │
└──────┬─────────────────────────────┘
       │ text response
       ▼
┌─────────────┐
│   TTS       │  (Piper, Coqui, ElevenLabs, Azure, OpenAI)
│  (Speech)   │
└──────┬──────┘
       │ audio
       ▼
User Hears Response
```

**CME Responsibilities:**
- Manage ASR/TTS provider selection and failover
- Inject conversation context into LLM prompt (same as text mode)
- Post-process LLM text for speech: add SSML `<break>`, `<prosody>`, expand abbreviations, normalize numbers
- Handle barge-in: if user speaks during TTS playback → cancel TTS, treat as interruption
- Session continuity: same `ConversationSession` shared across text/voice channels

**Latency Profile:** 1.5–4s end-to-end (ASR ~300–800ms, LLM ~500–2000ms, TTS ~200–800ms)

**Configuration:**
```toml
[voice.tier1]
asr_provider = "whisper_local"        # or "deepgram", "openai"
tts_provider = "piper"                # or "elevenlabs", "openai", "azure"
asr_language = "auto"
tts_voice = "en_US-lessac-medium"
tts_sample_rate = 22050
barge_in_enabled = true
max_turn_silence_ms = 800             # VAD threshold for turn end
```

### 12.2 Tier 2: Streaming LLM with Streaming ASR/TTS (Chunked Pipeline)

Lower perceived latency by streaming at each stage. User hears first words while LLM is still generating.

```
User Speech (streaming chunks)
    │
    ▼
┌─────────────────────┐
│ Streaming ASR       │  ──▶ partial transcripts → CME
│ (real-time VAD +    │     builds incremental context
│  interim results)   │
└─────────────────────┘
       │
       ▼ (when utterance complete or forced)
┌────────────────────────────────────┐
│ CME: Stream LLM Prompt             │
│ • Send partial context to LLM      │
│ • Receive text chunks              │
│ • Forward chunks to TTS immediately│
└────────────────────────────────────┘
       │ text chunks
       ▼
┌─────────────────────┐
│ Streaming TTS       │  ──▶ audio chunks → playback
│ (sentence/phrase    │     overlapping with LLM generation
│  granularity)       │
└─────────────────────┘
```

**Key Mechanics:**
- **ASR:** Real-time VAD + interim results. CME accumulates partial transcript but waits for utterance-end (silence > threshold OR explicit endpoint) before committing to LLM.
- **LLM Streaming:** CME sends prompt, receives text chunks. Each chunk → sentence boundary detection → forward to TTS.
- **TTS Streaming:** Sentence/phrase-level synthesis. Overlapping playback: TTS speaks sentence N while LLM generates sentence N+1.
- **Barge-in:** If user speech detected during TTS playback → cancel TTS, cancel LLM stream, treat as interruption (same as text mode).

**Latency Profile:** First audio ~800–1500ms (vs 1.5–4s for Tier 1). Perceived as "thinking while speaking."

**Configuration:**
```toml
[voice.tier2]
asr_provider = "deepgram_streaming"   # requires streaming interim results
tts_provider = "elevenlabs_streaming" # or "openai_streaming", "azure_streaming"
chunk_by_sentence = true              # split LLM text at sentence boundaries
overlap_tts_llm = true                # start TTS before LLM done
min_tts_chunk_chars = 30              # don't send tiny fragments to TTS
utterance_end_silence_ms = 600
forced_utterance_end_ms = 15000       # max utterance length
```

### 12.3 Tier 3: Full-Duplex Voice LLM (Native Audio I/O + Inline Tools)

End-to-end audio models (GPT-4o Realtime, Gemini Live, Moshi, Ultravox) with native tool calling. No ASR/TTS in pipeline — model consumes/produces audio directly.

```
┌─────────────────────────────────────────────────────────────┐
│              FULL-DUPLEX VOICE SESSION                      │
│  ┌──────────────┐         ┌──────────────────────────┐     │
│  │  User Audio  │────────▶│  Voice LLM (Realtime)    │     │
│  │  (WebRTC/    │         │  • Audio in / Audio out  │     │
│  │   WebSocket) │         │  • Native turn detection │     │
│  └──────────────┘         │  • Inline function calls │     │
│         ▲                 │  • Built-in VAD          │     │
│         │                 └───────────┬──────────────┘     │
│         │                             │                    │
│         │                    ┌────────┴────────┐           │
│         │                    ▼                 ▼           │
│         │            ┌─────────────┐   ┌─────────────┐     │
│         │            │ Tool Executor│   │  CME State  │     │
│         │            │ (async,     │   │ (same text  │     │
│         │            │  sandboxed) │   │  session)   │     │
│         │            └─────────────┘   └─────────────┘     │
│         │                                                    │
│         └──────────────── Audio Response ───────────────────┘
└─────────────────────────────────────────────────────────────┘
```

**CME Role Shifts:** From *pipeline orchestrator* → *session coordinator & tool gateway*.

**CME Responsibilities:**
1. **Session Negotiation** — WebRTC/WebSocket handshake with voice LLM provider
2. **Tool Gateway** — Voice LLM emits `function_call` events → CME executes via ToolExecutor → returns `function_result` audio/text
3. **State Synchronization** — Same `ConversationSession` used for text/voice; tool results, proactive items, questionnaires shared
4. **Interruption Handling** — Native to model (user speaks → model stops), but CME still cancels pending tool calls, logs interruption
5. **Proactive Injection** — CME can send `conversation.item.create` (system message) to inject proactive items mid-session
6. **Fallback** — If voice LLM fails → seamless downgrade to Tier 2

**Tool Calling in Full-Duplex:**
```
Voice LLM: "Let me check the deploy status." 
           → emits function_call: { "name": "check_deploy", "args": {} }
CME:       Executes tool → returns result
Voice LLM: Receives function_result → continues speaking: "Deploy failed at 14:22..."
```
- Tools defined in voice LLM session config (OpenAI Realtime `tools` param, Gemini `function_declarations`)
- CME maps tool names to local `ToolExecutor` implementations
- Tool execution is async; CME streams result back to voice LLM via data channel
- **Timeouts:** Tool call must complete within provider limit (typically 30–60s)

**Latency Profile:** 200–500ms perceived (model handles turn detection, no ASR/TTS hop). Most natural.

**Configuration:**
```toml
[voice.tier3]
provider = "openai_realtime"          # or "gemini_live", "ultravox", "moshi"
model = "gpt-4o-realtime-preview"
voice = "alloy"                       # provider-specific voice ID
input_audio_format = "pcm16"          # 24kHz mono
output_audio_format = "pcm16"
turn_detection = { type = "server_vad", threshold = 0.5, prefix_padding_ms = 300, silence_duration_ms = 800 }
tools = [ "search_code", "run_command", "check_deploy", "create_pr" ]
tool_timeout_ms = 30000
cme_injected_system_messages = true   # allow CME to push proactive items
fallback_tier = 2
```

### 12.4 Unified Voice Session Management

Single `ConversationSession` supports **simultaneous text + voice** channels.

```rust
struct VoiceSessionState {
    session_id: SessionId,
    tier: VoiceTier,                    // 1, 2, or 3
    asr_handle: Option<AsrHandle>,
    tts_handle: Option<TtsHandle>,
    realtime_handle: Option<RealtimeHandle>, // Tier 3 only
    audio_output_queue: VecDeque<AudioChunk>,
    barge_in_detected: bool,
    current_utterance: Option<UtteranceBuffer>,
    fallback_chain: Vec<VoiceTier>,     // e.g., [3, 2, 1]
}
```

**Tier Negotiation & Failover:**
```
Session Start
    │
    ▼
Preferred Tier (config or UI choice)
    │
    ▼
Health Check (provider reachable, model loaded)
    │
    ├─ OK → Establish session
    │
    └─ FAIL → Try next tier in fallback_chain
              │
              └─ ALL FAIL → Text-only mode + user notification
```

**Mid-Session Downgrade:**
- Tier 3 connection drops → CME re-establishes at Tier 2, resumes with last context
- User explicitly requests "switch to text" → CME closes voice, continues text
- Network degradation (jitter > threshold) → Auto-downgrade with notification

### 12.5 Audio I/O Abstractions

```rust
trait AsrEngine: Send + Sync {
    fn start_stream(&self, config: AsrConfig) -> Result<AsrStream>;
    fn transcribe_once(&self, audio: &[u8]) -> Result<String>;
}

trait AsrStream {
    fn on_interim(&mut self, callback: fn(&str));      // partial transcript
    fn on_final(&mut self, callback: fn(&str));        // committed transcript
    fn on_utterance_end(&mut self, callback: fn());    // VAD silence
    fn push_audio(&mut self, chunk: &[u8]);
    fn close(&mut self);
}

trait TtsEngine: Send + Sync {
    fn synthesize_stream(&self, text: &str, voice: VoiceId) -> Result<TtsStream>;
    fn synthesize_once(&self, text: &str, voice: VoiceId) -> Result<Vec<u8>>;
}

trait TtsStream {
    fn on_audio_chunk(&mut self, callback: fn(&[u8])); // raw PCM/Opus
    fn on_done(&mut self, callback: fn());
    fn push_text(&mut self, text: &str);               // for streaming TTS
    fn cancel(&mut self);                              // barge-in
}

trait RealtimeVoiceBackend: Send + Sync {
    fn connect(&self, config: RealtimeConfig) -> Result<RealtimeSession>;
}

trait RealtimeSession {
    fn on_audio_output(&mut self, callback: fn(&[u8]));
    fn on_transcript(&mut self, callback: fn(&str));           // user speech transcript
    fn on_function_call(&mut self, callback: fn(FunctionCall));
    fn on_interruption(&mut self, callback: fn());
    fn send_audio(&mut self, audio: &[u8]);
    fn send_text(&mut self, text: &str);                       // system/user text injection
    fn send_function_result(&mut self, call_id: &str, result: &str);
    fn close(&mut self);
}
```

### 12.6 Voice-Specific Proactive Behaviors

| Behavior | Tier 1/2 | Tier 3 |
|---|---|---|
| **Background lookup** | Same as text | Same; result injected via `conversation.item.create` |
| **Suggestion** | Spoken as "One thought: ..." | Injected as system message, model voices naturally |
| **Correction** | Spoken immediately | Model receives correction context, self-corrects |
| **Check-in** | "Everything running smooth?" | Natural conversational check-in |
| **Questionnaire** | Spoken questions, ASR answers | Native voice questionnaire flow |

**Voice Questionnaire Adaptation:**
- Questions spoken via TTS (Tier 1/2) or model (Tier 3)
- Answers captured via ASR (Tier 1/2) or model transcript (Tier 3)
- Choice questions → read options → user speaks choice
- "Skip" / "Stop" recognized as voice commands

### 12.7 Voice Metrics & Observability

| Metric | Tier 1 Target | Tier 2 Target | Tier 3 Target |
|---|---|---|---|
| First audio latency | < 2.5s | < 1.2s | < 500ms |
| ASR word error rate | < 5% | < 5% | N/A (native) |
| TTS naturalness (MOS) | > 3.5 | > 4.0 | N/A (native) |
| Barge-in response | < 300ms | < 200ms | < 100ms |
| Tool call latency (Tier 3) | N/A | N/A | < 2s |
| Tier fallback success | 100% | 100% | 100% |

### 12.8 Security & Privacy (Voice)

- **Audio encryption** in transit (DTLS/SRTP for WebRTC, WSS for WebSocket)
- **No audio persistence** by default; opt-in recording with explicit consent
- **PII in audio** — ASR transcripts go through same redaction as text
- **Voice biometrics** — not stored; speaker embeddings only for session diarization if enabled
- **Provider data policies** — prefer local/on-prem ASR/TTS for sensitive environments

---

## 13. Integration Points

### 13.1 LLM Backend Adapter Interface
```rust
trait LlmBackend: Send + Sync {
    fn stream_completion(&self, prompt: Prompt, tx: mpsc::Sender<LlmEvent>) -> Result<AbortHandle>;
    fn count_tokens(&self, text: &str) -> usize;
    fn supports_tools(&self) -> bool;
    // Voice extensions:
    fn supports_realtime(&self) -> bool { false }
    fn create_realtime_session(&self, config: RealtimeConfig) -> Result<Box<dyn RealtimeVoiceBackend>> { Err("not supported") }
}

enum LlmEvent {
    Chunk(String),
    ToolCall(ToolCall),
    Done(FinishReason),
    Error(Error),
}
```

### 13.2 Tool/Action Bus
```rust
trait ToolExecutor: Send + Sync {
    fn execute(&self, call: ToolCall) -> ToolResult;
    fn list_available(&self) -> Vec<ToolSpec>;
    // Voice: some tools may have voice-specific UX (confirm by voice, read result aloud)
    fn voice_ux_hint(&self, tool_name: &str) -> Option<VoiceUxHint> { None }
}
```

### 13.3 UI Channel Adapters (Extended)
| Channel | Adapter Responsibilities |
|---|---|
| Web (React) | SSE + WebRTC for voice, component rendering, audio visualization |
| Telegram | Bot API + Voice messages (Tier 1/2), Telegram Voice Chat API (Tier 3) |
| CLI | TTY + optional audio (PortAudio/PulseAudio) for local voice |
| Phone/SIP | Asterisk/FreeSWITCH gateway → CME (future) |

---

## 14. Observability

### 14.1 Key Metrics (Extended)
| Metric | Target |
|---|---|
| First chunk latency (user msg → first agent chunk) | < 800ms |
| Chunk delivery reliability | 99.9% |
| Interruption acknowledgment latency | < 200ms |
| Proactive item relevance (user accepts/acts) | > 60% |
| Questionnaire completion rate | > 80% |
| Multi-UI sync drift | < 50ms |
| **Voice Tier 1 first audio** | **< 2.5s** |
| **Voice Tier 2 first audio** | **< 1.2s** |
| **Voice Tier 3 first audio** | **< 500ms** |
| **Barge-in response (all tiers)** | **< 300ms** |

### 14.2 Internal State Observability

The CME exposes its complete internal state for real-time inspection, debugging, and audit.

#### 14.2.1 State Snapshots (Polling Endpoint)
```
GET /cme/v1/sessions/{session_id}/state
```
Returns a complete point-in-time snapshot:
```json
{
  "session_id": "sess_abc123",
  "timestamp": "2026-11-20T14:32:11.456Z",
  "conversation": {
    "turn_count": 12,
    "active_turn_id": "turn_456",
    "active_turn_owner": "agent",
    "active_turn_chunk_index": 2,
    "active_turn_total_chunks": 4,
    "modality": "drill_down",
    "detail_level": 2
  },
  "proactive_queue": {
    "pending_count": 3,
    "items": [
      {"id": "item_789", "kind": "suggestion", "confidence": 0.82, "age_turns": 1},
      {"id": "item_790", "kind": "correction", "confidence": 0.91, "age_turns": 0},
      {"id": "item_791", "kind": "check_in", "confidence": 0.65, "age_turns": 4}
    ],
    "last_proactive_turn": 10,
    "cooldown_remaining_turns": 2
  },
  "questionnaire": {
    "active": false,
    "questionnaire_id": null,
    "current_question_index": null,
    "partial_answers": {}
  },
  "cme_internal": {
    "turn_planner": {
      "last_plan_id": "plan_789",
      "last_strategy": "Hybrid",
      "chunks_planned": 4,
      "proactive_injection_points": [1]
    },
    "speculative_executor": {
      "active_prefetches": 2,
      "prefetch_hit_rate_1h": 0.87,
      "active_predictions": [
                        {"id": "pred_1", "text": "deploy failed", "confidence": 0.82, "status": "Ready"},
                        {"id": "pred_2", "text": "cache org lookup", "confidence": 0.76, "status": "Running"}
                      ]
    },
    "modality_controller": {
      "current_modality": "drill_down",
      "detail_level": 2,
      "include_headings": true,
      "include_sources": true,
      "auto_selected": true
    },
    "prosody_controller": {
      "current_affect": "Frustrated",
      "affect_confidence": 0.89,
      "profile_applied": "Frustrated",
      "baseline_blend_ratio": 0.7
    },
    "proactive_evaluator": {
      "plugins_enabled": 4,
      "last_evaluation_turn": 11,
      "items_generated_this_session": 12,
      "items_injected_this_session": 8,
      "rejection_rate": 0.15
    }
  },
  "llm_backend": {
    "active_backend": "claude-3.5-sonnet",
    "fallback_chain": ["gpt-4o", "claude-3.5-haiku"],
    "tokens_in_session": 47200,
    "tokens_last_turn": 1200,
    "avg_latency_ms": 650,
    "error_rate_1h": 0.02
  },
  "voice": {
    "tier": 3,
    "connection_state": "Connected",
    "asr_provider": "openai_realtime",
    "tts_provider": "native",
    "barge_in_enabled": true,
    "jitter_buffer_ms": 40
  },
  "channels": {
    "connected": ["web", "telegram"],
    "last_sync_ms": 12,
    "pending_deliveries": 0
  }
}
```

#### 14.2.2 Real-Time Event Stream (WebSocket)
```
WS /cme/v1/sessions/{session_id}/events
```
Emits structured events for every internal transition:
```json
{ "event": "turn_started", "turn_id": "turn_457", "owner": "user", "timestamp": "..." }
{ "event": "turn_ended", "turn_id": "turn_457", "chunks_sent": 3, "interrupted": false }
{ "event": "proactive_item_generated", "item_id": "item_792", "kind": "suggestion", "confidence": 0.78 }
{ "event": "proactive_item_injected", "item_id": "item_792", "at_turn": 12, "at_chunk": 2 }
{ "event": "modality_changed", "from": "bullets", "to": "drill_down", "trigger": "explicit_user_request" }
{ "event": "proactive_evaluation_completed", "plugins_run": 4, "items_generated": 2, "items_filtered": 1 }
{ "event": "speculative_prefetch_hit", "prefetch_id": "pred_1", "similarity": 0.92, "latency_saved_ms": 448 }
{ "event": "speculative_prefetch_miss", "prefetch_id": "pred_3", "similarity": 0.31 }
{ "event": "interruption_detected", "at_chunk": 2, "acknowledgment_latency_ms": 45 }
{ "event": "tier_fallback", "from": 3, "to": 2, "reason": "connection_dropped" }
{ "event": "questionnaire_started", "questionnaire_id": "onboarding_v1" }
{ "event": "questionnaire_answered", "question_id": "role", "answer": "engineer" }
{ "event": "questionnaire_completed", "questionnaire_id": "onboarding_v1", "partial": false }
```

#### 14.2.3 Prompt & Decision Trace Logging
Every LLM call logs the full prompt assembly and decision rationale:
```json
{
  "event": "llm_call_initiated",
  "call_id": "call_999",
  "session_id": "sess_abc123",
  "turn_id": "turn_457",
  "backend": "claude-3.5-sonnet",
  "prompt": {
    "system_tokens": 1200,
    "context_tokens": 3400,
    "turn_plan_tokens": 150,
    "total_tokens": 4750,
    "turn_plan": {
      "strategy": "Hybrid",
      "chunks_planned": 4,
      "proactive_injection_points": [1]
    },
    "modality": { "type": "drill_down", "detail_level": 2 },
    "proactive_item_injected": { "id": "item_792", "kind": "suggestion" }
  },
  "sampling_params": { "temperature": 0.4, "max_tokens": 2000 },
  "timestamp": "..."
}
```
```json
{
  "event": "llm_call_completed",
  "call_id": "call_999",
  "tokens_out": 847,
  "latency_ms": 612,
  "chunks_streamed": 4,
  "finish_reason": "stop",
  "tool_calls": [],
  "timestamp": "..."
}
```

#### 14.2.4 Proactive Plugin Evaluation Trace
```json
{
  "event": "proactive_evaluation",
  "evaluation_id": "eval_555",
  "session_id": "sess_abc123",
  "turn": 12,
  "trigger": "TurnInterval",
  "plugins": [
    { "id": "deploy_monitor", "ran": true, "latency_ms": 12, "items_generated": 1, "items_after_confidence_filter": 1 },
    { "id": "code_health", "ran": true, "latency_ms": 8, "items_generated": 0, "items_after_confidence_filter": 0 },
    { "id": "security_scanner", "ran": false, "reason": "cooldown_active", "cooldown_remaining_turns": 3 },
    { "id": "team_conventions", "ran": true, "latency_ms": 5, "items_generated": 1, "items_after_confidence_filter": 1 }
  ],
  "queue_before": 1,
  "queue_after": 3,
  "selected_for_injection": ["item_792", "item_793"],
  "total_latency_ms": 28
}
```

### 14.3 In-Conversation Context Observability

Real-time inspection of the conversation context that drives agent behavior.

#### 14.3.1 Conversation Context Snapshot
```
GET /cme/v1/sessions/{session_id}/context
```
```json
{
  "session_id": "sess_abc123",
  "timestamp": "2026-11-20T14:35:22.123Z",
  "user": {
    "id": "user_123",
    "name": "Alex",
    "role": "engineer",
    "preferences": {
      "verbosity": -0.72,
      "technical_depth": 0.68,
      "initiative": 0.34,
      "formality": -0.15,
      "code_verbosity": -0.55,
      "explanation_style": 0.22
    },
    "expertise_domains": { "rust": 0.85, "postgres": 0.72, "k8s": 0.61 },
    "communication_style": "terse"
  },
  "project": {
    "id": "proj_456",
    "name": "auth-service-migration",
    "active_goals": [
      { "id": "goal_1", "description": "Fix deploy pipeline", "status": "InProgress", "progress": 0.6 },
      { "id": "goal_2", "description": "Add Redis caching", "status": "Open", "progress": 0.0 }
    ],
    "open_commitments": [
      { "id": "comm_1", "description": "PR #247 for migration fix", "owner": "agent", "status": "InProgress" }
    ],
    "tech_stack": { "language": "rust", "db": "postgres", "cache": "redis" }
  },
  "conversation": {
    "recent_topics": ["deploy debugging", "migration timeout", "caching strategy"],
    "active_modality": "drill_down",
    "last_user_intent": "optimize_api_latency",
    "open_questions": [
      { "id": "q_1", "question": "Cache invalidation strategy for org lookup", "priority": "High" }
    ],
    "artifacts_referenced": ["PR #247", "PR #248", "Design doc #12"]
  },
  "agent_state": {
    "current_persona": "Engineering Partner",
    "affect": "Neutral",
    "proactive_budget_remaining": 0.75,
    "metacognitive_budget_remaining": 0.6
  }
}
```

#### 14.3.2 Turn Plan Inspection
```
GET /cme/v1/sessions/{session_id}/turn-plan/{turn_id}
```
```json
{
  "turn_id": "turn_457",
  "strategy": "Hybrid",
  "chunks": [
    { "index": 0, "role": "Acknowledge", "directive": "Acknowledge the latency issue in 1 sentence", "max_sentences": 1, "pause_after_ms": 400 },
    { "index": 1, "role": "Finding", "directive": "State root cause: N+1 query in get_user_orgs from recent deploy", "max_sentences": 2, "pause_after_ms": 400 },
    { "index": 2, "role": "Proactive", "directive": "[INJECTED: suggestion about Redis caching]", "max_sentences": 2, "pause_after_ms": 400 },
    { "index": 3, "role": "NextStep", "directive": "Ask: 'Profile first or jump to caching?'", "max_sentences": 1, "pause_after_ms": null }
  ],
  "proactive_injection_points": [1],
  "modality": { "type": "drill_down", "detail_level": 2 },
  "created_at": "2026-11-20T14:35:22.100Z"
}
```

#### 14.3.3 Modality Selection Trace
```json
{
  "event": "modality_selection",
  "turn_id": "turn_457",
  "user_input": "Drill down on the migration timeout",
  "explicit_request": true,
  "inferred_modality": "drill_down",
  "final_modality": "drill_down",
  "detail_level": 2,
  "include_headings": true,
  "include_sources": true,
  "selection_reason": "explicit_user_request",
  "context_factors": {
    "stakes": 0.4,
    "user_role": "engineer",
    "topic_complexity": 0.8,
    "previous_turn_was_long": false
  }
}
```

#### 14.3.4 Proactive Item Lifecycle
```json
{
  "event": "proactive_item_lifecycle",
  "item_id": "item_792",
  "kind": "suggestion",
  "plugin": "deploy_monitor",
  "generated_at_turn": 11,
  "confidence": 0.82,
  "confidence_threshold": 0.7,
  "cooldown_check_passed": true,
  "injected_at_turn": 12,
  "injected_at_chunk": 2,
  "user_response": "accepted",
  "response_turn": 13,
  "acted_upon": true,
  "feedback": "accepted",
  "plugin_learning": { "cooldown_adjusted": false, "confidence_adjusted": false }
}
```

### 14.4 Distributed Tracing Integration

OpenTelemetry-compatible tracing for end-to-end latency analysis.

#### 14.4.1 Trace Context Propagation
Every request carries `traceparent` and `tracestate` headers:
```
traceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01
tracestate: cme=sess_abc123,turn_turn_457,modality_drill_down
```

#### 14.4.2 Span Definitions
| Span Name | Parent | Attributes |
|---|---|---|
| `cme.session.process_message` | — | session_id, user_id, modality, turn_count |
| `cme.turn.build_plan` | process_message | strategy, chunk_count, proactive_injections |
| `cme.proactive.evaluate` | process_message | plugins_run, items_generated, items_injected |
| `cme.llm.stream_completion` | turn.execute | backend, tokens_in, tokens_out, latency_ms |
| `cme.chunk.stream` | llm.stream_completion | chunk_index, chunk_role, modality, pause_ms |
| `cme.proactive.inject` | turn.execute | item_id, item_kind, injection_point |
| `cme.modality.select` | turn.build_plan | modality, detail_level, selection_reason |
| `cme.voice.asr` | process_message | tier, provider, duration_ms |
| `cme.voice.tts` | llm.stream_completion | tier, provider, chunks, latency_ms |

#### 14.4.3 Trace Sampling
```toml
[observability.tracing]
enabled = true
sampler = "parent_based_trace_id_ratio"
trace_ratio = 0.1              # 10% of traces
force_sample_errors = true     # Always sample failed requests
force_sample_slow_ms = 5000    # Always sample requests > 5s
custom_sampler = "cme_sampler" # Custom: sample all proactive injections, interruptions
```

### 14.5 Real-Time Debugging UI

#### 14.5.1 Live Session Inspector (Web)
```
/cme/debug/sessions/{session_id}
```
Features:
- **State Panel**: Live-updating JSON tree of session state
- **Event Stream**: Real-time event log with filtering (turn, proactive, modality, voice, error)
- **Prompt Inspector**: View assembled prompts, token counts, turn plans
- **Modality Timeline**: Visual timeline of modality switches with triggers
- **Proactive Queue**: Live queue with confidence scores, cooldowns
- **Latency Waterfall**: Per-turn breakdown (ASR → LLM → TTS → stream)
- **Interruption Replay**: Step-through of interrupted turns

#### 14.5.2 CLI Debug Commands
```bash
cme debug session sess_abc123 --follow          # Live state updates
cme debug session sess_abc123 --event proactive # Filter events
cme debug trace sess_abc123 turn_457            # Full trace for turn
cme debug prompt sess_abc123 turn_457           # View assembled prompt
cme debug modality sess_abc123 --history        # Modality switch history
cme debug proactive sess_abc123 --plugin deploy_monitor
```

### 14.6 Structured Logging (Extended)

#### 14.6.1 Canonical Event Schema
All events follow a unified schema:
```json
{
  "event": "string",                    // Canonical event name
  "timestamp": "ISO8601",               // Event timestamp
  "session_id": "string",               // Session correlation
  "turn_id": "string|null",             // Turn correlation
  "correlation_id": "string",           // Cross-system correlation
  "level": "debug|info|warn|error",     // Log level
  "component": "string",                // CME component (turn_planner, proactive, modality, etc.)
  "data": { ... },                      // Event-specific payload
  "tags": ["string"],                   // Searchable tags
  "duration_ms": "number|null"          // For spans
}
```

#### 14.6.2 Standard Event Catalog
| Event | Component | Key Data Fields |
|---|---|---|
| `session.created` | session | session_id, user_id, project_id, modality |
| `session.resumed` | session | session_id, previous_session_id, context_restored |
| `turn.started` | turn_planner | turn_id, owner, trigger, modality |
| `turn.plan_built` | turn_planner | turn_id, strategy, chunk_count, proactive_points |
| `turn.ended` | turn_planner | turn_id, chunks_sent, interrupted, latency_ms |
| `chunk.streamed` | streaming | turn_id, chunk_index, role, modality, pause_ms |
| `interruption.detected` | streaming | turn_id, at_chunk, ack_latency_ms |
| `proactive.evaluated` | proactive | turn_id, plugins_run, items_generated, items_injected |
| `proactive.item_generated` | proactive | item_id, kind, plugin, confidence |
| `proactive.item_injected` | proactive | item_id, turn_id, chunk_index, injection_point |
| `proactive.item_feedback` | proactive | item_id, feedback, acted_upon |
| `modality.selected` | modality | turn_id, modality, detail_level, selection_reason |
| `modality.changed` | modality | turn_id, from, to, trigger |
| `llm.call_started` | llm | call_id, backend, tokens_in, turn_id |
| `llm.call_completed` | llm | call_id, tokens_out, latency_ms, finish_reason |
| `tool.called` | tools | tool_id, tool_name, turn_id, latency_ms |
| `voice.asr_started` | voice | tier, provider, session_id |
| `voice.asr_completed` | voice | tier, provider, transcript, latency_ms, wer |
| `voice.tts_started` | voice | tier, provider, text_length, voice |
| `voice.tts_completed` | voice | tier, provider, chunks, latency_ms |
| `voice.barge_in` | voice | tier, at_chunk, acknowledgment_latency_ms |
| `tier.fallback` | voice | from_tier, to_tier, reason, context_preserved |
| `questionnaire.started` | questionnaire | questionnaire_id, session_id |
| `questionnaire.answered` | questionnaire | question_id, answer, turn_id |
| `questionnaire.completed` | questionnaire | questionnaire_id, partial, turn_id |

### 14.7 Metrics Export (Prometheus)

```prometheus
# HELP cme_session_active_total Active sessions
cme_session_active_total 42

# HELP cme_turn_latency_ms Turn processing latency
cme_turn_latency_ms_bucket{le="100"} 1204
cme_turn_latency_ms_bucket{le="500"} 3421
cme_turn_latency_ms_bucket{le="1000"} 4100
cme_turn_latency_ms_bucket{le="+Inf"} 4150

# HELP cme_chunk_stream_latency_ms Chunk streaming latency
cme_chunk_stream_latency_ms_bucket{le="50"} 8900
cme_chunk_stream_latency_ms_bucket{le="200"} 14200

# HELP cme_proactive_items_generated_total Proactive items generated
cme_proactive_items_generated_total{kind="suggestion"} 1240
cme_proactive_items_generated_total{kind="correction"} 380
cme_proactive_items_generated_total{kind="check_in"} 560

# HELP cme_proactive_injection_rate Proactive items injected per turn
cme_proactive_injection_rate 0.23

# HELP cme_modality_distribution Modality usage
cme_modality_distribution{modality="drill_down"} 0.18
cme_modality_distribution{modality="explain"} 0.22
cme_modality_distribution{modality="zoom_out"} 0.15
cme_modality_distribution{modality="summarize"} 0.12
cme_modality_distribution{modality="bullets"} 0.25
cme_modality_distribution{modality="auto"} 0.08

# HELP cme_speculative_hit_rate Speculative execution hit rate
cme_speculative_hit_rate 0.87

# HELP cme_interruption_rate Interruptions per 100 turns
cme_interruption_rate 4.2

# HELP cme_voice_tier_distribution Voice tier usage
cme_voice_tier_distribution{tier="1"} 0.45
cme_voice_tier_distribution{tier="2"} 0.35
cme_voice_tier_distribution{tier="3"} 0.20

# HELP cme_llm_tokens_total LLM tokens consumed
cme_llm_tokens_total{backend="claude-3.5-sonnet",direction="in"} 12400000
cme_llm_tokens_total{backend="claude-3.5-sonnet",direction="out"} 3800000

# HELP cme_proactive_plugin_evaluation_latency_ms Plugin evaluation latency
cme_proactive_plugin_evaluation_latency_ms_bucket{plugin="deploy_monitor",le="10"} 0.85
cme_proactive_plugin_evaluation_latency_ms_bucket{plugin="code_health",le="10"} 0.72
```

---

## 15. Failure Modes & Degradation (Extended)

| Failure | Degradation |
|---|---|
| LLM backend timeout | Stream cached/fallback response; queue for retry |
| LLM backend unavailable | Switch to backup backend; if none, "I'm having trouble thinking — give me a moment" |
| UI channel disconnect | Buffer chunks in Redis; replay on reconnect |
| Tool execution failure | Stream error card; LLM sees error in next prompt |
| State store unavailable | In-memory fallback; persist on recovery |
| Proactive lookup timeout | Skip proactive item; continue reactive flow |
| **ASR failure** | **Retry once → fallback to text input prompt** |
| **TTS failure** | **Stream text to UI + "Audio unavailable" notice** |
| **Tier 3 connection drop** | **Auto-downgrade to Tier 2 with context preserved** |
| **Tier 2 streaming stall** | **Fallback to Tier 1 (collect full response)** |
| **Barge-in during Tier 3 tool call** | **Cancel tool, acknowledge interruption, resume** |

---

## 16. Security & Privacy (Extended)

- **No raw conversation logs to LLM** — only curated, compressed context
- **PII redaction** in prompt assembly (API keys, emails, secrets)
- **Per-session encryption** for stored conversation history
- **Tool sandbox** — no host filesystem/network without explicit grant
- **Rate limiting** per user/project on LLM calls and tool executions
- **Audio encryption** in transit (DTLS/SRTP for WebRTC, WSS for WebSocket)
- **No audio persistence** by default; opt-in recording with explicit consent
- **PII in audio** — ASR transcripts go through same redaction as text
- **Provider data policies** — prefer local/on-prem ASR/TTS for sensitive environments

---

## 17. Implementation Phases (Extended)

| Phase | Scope |
|---|---|
| **1. Core Loop** | Session management, turn/chunk model, basic SSE streaming, single UI (Web) |
| **2. LLM Integration** | Prompt assembly, streaming completion, tool calling, first backend (OpenAI) |
| **3. Proactivity** | Proactive queue, background lookups, suggestion/correction injection, timers |
| **4. Questionnaires** | Questionnaire engine, intro flow, name-only, full onboarding, persistence |
| **5. Multi-UI** | Telegram adapter, CLI adapter, cross-channel sync, session handoff |
| **6. Pace & Polish** | Pace adaptation, jitter, interruption handling, structured cards, observability |
| **7. Resilience** | Fallback backends, replay, degradation modes, load testing |
| **8. Voice Tier 1** | ASR/TTS pipeline, barge-in, shared session, voice questionnaires |
| **9. Voice Tier 2** | Streaming ASR/TTS, chunked LLM→TTS overlap, real-time VAD |
| **10. Voice Tier 3** | Full-duplex realtime backend, inline tool calling, WebRTC, auto-failover |

---

*This engine makes the agent a **first-class conversation participant** — not a request/response function. It owns the rhythm, the initiative, and the structure; the LLM provides the words. Now it speaks, listens, and works in real time.*

---

## Related Designs

| Document | Purpose |
|---|---|
| `conversation/proactive_conversation.md` | Conversation UX: pacing, proactivity, questionnaires, agent intro |
| `conversation/conversation_engine_improvements.md` | Comprehensive improvements: turn planning, speculative execution, compression, preference learning, multi-LLM routing, plugins, prosody, testing, cross-session memory |
| `conversation/turn_planning.md` | Deterministic TurnPlan with chunk directives, LLM prompt templates, validation |
| `conversation/speculative_execution.md` | ASR-LLM overlap, prediction models, KV cache prefetch, latency analysis |
| `conversation/conversation_compression.md` | Structured compression with narrative + artifacts, semantic retrieval, project aggregation |
| `conversation/preference_learning.md` | Implicit Bayesian preference learning from behavioral signals, application in TurnPlanner |
| `conversation/multi_llm_routing.md` | Task-aware model selection, routing table, fallback chains, auto-optimization |
| `conversation/proactive_plugin_system.md` | Plugin architecture for proactive behaviors, WASM support, built-in plugins |
| `conversation/voice_prosody.md` | Dynamic TTS prosody from affect detection, context rules, baseline learning |
| `conversation/synthetic_testing.md` | Declarative YAML test DSL, personas, assertion checkers, CI integration |
| `conversation/cross_session_memory.md` | Cross-session continuity: session→segment→project→user memory hierarchy, sync, privacy |

See also: `DESIGN_INDEX.md` for the complete design catalog.