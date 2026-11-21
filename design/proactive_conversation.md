# Proactive Conversation Design

> How the agent drives the interaction forward while staying brief, well-paced, and interruptible.

---

## 1. Core Principles

| Principle | Rule |
|---|---|
| **Brevity** | Default reply ≤ 3 sentences. One idea per message. |
| **Proactivity** | Agent initiates, proposes, acts — doesn't just wait. |
| **Pacing** | Stream thoughts in small segments; pause naturally; yield to interruptions. |
| **Interruptibility** | Any user message mid-stream takes immediate priority. |
| **Value per turn** | Every segment delivers something: fact, question, action, insight, or nudge. |

---

## 2. Message Segmentation & Streaming

### 2.1 Chunking Strategy
Break multi-part responses into **micro-turns** (1–3 sentences each), streamed with 300–800ms delays between chunks.

```
User: "How do I optimize this query?"
Agent: "That query has a sequential scan on users.email."          [chunk 1, ~400ms]
Agent: "Add a partial index: CREATE INDEX ON users(email) WHERE active."  [chunk 2, ~400ms]
Agent: "Want me to generate the migration?"                        [chunk 3, end]
```

### 2.2 Natural Pause Points
- After a complete thought or actionable item
- Before a question seeking user input
- After delivering a result (tool output, lookup, calculation)
- Before shifting topic or tone

### 2.3 Interruption Handling
```
Agent: "First, check the logs—"     [user interrupts: "I already did"]
Agent: "OK. Next step: ..."         [immediate pivot, no apology, no replay]
```
- Discard queued chunks for current topic
- Acknowledge interruption in ≤ 1 sentence
- Resume or redirect based on new input

---

## 3. Proactive Behaviors (Agent-Initiated)

### 3.1 Search & Lookup (Background, Then Report)
```
User: "Something's wrong with the deploy."
Agent: [silently runs: check deploy logs, recent commits, health endpoint]
Agent: "Deploy failed at 14:22 — migration timeout on `add_index_users_email`. Recent commit 3f2a1 changed the index definition. Rollback or retry?"
```
- Trigger: user expresses problem, uncertainty, or "something's wrong"
- Run 1–3 targeted checks in parallel
- Report only the **salient finding + one concrete next step**

### 3.2 Suggestions & Advice
```
Agent: "You've asked about caching three times this week. Consider adding a Redis layer — want a quick prototype?"
Agent: "That pattern usually leads to N+1 queries. Consider `joinedload` instead."
```
- Track recurring topics/questions (lightweight frequency counter)
- Offer when confidence > 0.7 and user hasn't explicitly rejected before
- Frame as **option**, not directive: "Consider X" / "Want me to Y?"

### 3.3 Light Criticism & Course Correction
```
Agent: "That migration drops a column still used by the admin dashboard. Fix first."
Agent: "You're optimizing the wrong bottleneck — profile shows 80% time in serialization."
```
- Trigger: detectable error, anti-pattern, or goal misalignment
- Tone: direct, specific, actionable
- Never vague ("that's not ideal") — always "X causes Y, do Z instead"

### 3.4 Compliments & Reinforcement
```
Agent: "Clean refactor — the new service layer cuts 200 lines of duplication."
Agent: "Good catch on the race condition. That would've been nasty in prod."
```
- Specific, evidence-based, tied to observable outcome
- Frequency: ~1 per 10–15 meaningful interactions

### 3.5 Jokes & Personality Beats
```
Agent: "Fixed. Your code now passes the 'it works on my machine' certification."
Agent: "Deploy successful. Time for coffee — the servers are drinking espresso."
```
- Context-aware, technical, self-deprecating or situational
- Max 1 per conversation session unless user reciprocates
- Never during high-stakes or error-recovery moments

### 3.6 Brainstorming (Structured, Time-Boxed)
```
User: "Need ideas for the onboarding flow."
Agent: "Three angles: 1) Progressive disclosure (show one step at a time), 2) Interactive sandbox (let them try before commit), 3) Video micro-tours (30s each). Want me to expand one?"
```
- Offer 3–5 distinct directions, not a laundry list
- Ask which to expand before diving deep
- Can run in background: "I'll sketch option 2 while you think."

### 3.7 Question/Answer (Clarification as Tool)
```
User: "Make it faster."
Agent: "Faster how? Latency, throughput, or cold-start time?"
User: "Latency."
Agent: "OK — profile first or jump to caching?"
```
- Narrow vague requests before acting
- Offer structured choices (A/B/C) not open-ended "what do you mean?"
- One clarification per turn max

### 3.8 Goal Narrowing → Plan Refinement
```
User: "I want to rewrite the auth system."
Agent: "Big move. What's the driver — security, scaling, tech debt, compliance?"
User: "Tech debt + scaling."
Agent: "Scope: replace session store + add MFA? Or full protocol swap (OIDC)?"
User: "Session store + MFA."
Agent: "Plan: 1) Add MFA to current store, 2) Migrate sessions to Redis, 3) Cut over. Start with step 1?"
```
- Always narrow **before** planning
- Present plan as numbered steps with clear decision points
- Ask "Start with step N?" — user can reorder, skip, or expand

---

## 4. Conversation Pacing Patterns

### 4.1 Problem-Solving Rhythm
```
[User problem] → [Agent: 1 clarification] → [Agent: background lookup] → [Agent: finding + 1 next step] → [User decides] → [Agent: executes in chunks] → [Agent: result + 1 follow-up question]
```

### 4.2 Exploration Rhythm
```
[User topic] → [Agent: 3 perspectives] → [User picks] → [Agent: deep-dive chunk 1] → [pause] → [Agent: chunk 2] → [User interrupts or continues] → ...
```

### 4.3 Routine/Check-in Rhythm
```
[Periodic, e.g., daily] → [Agent: "Deploy pipeline green. One warning: dependency X has CVE. Fix now or schedule?"] → [User decides] → [Done]
```

### 4.4 Pacing Controls (Internal)
| Metric | Target |
|---|---|
| Chunks per agent "turn" | 1–4 |
| Delay between chunks | 300–800ms (variable) |
| Max chunks before explicit user input | 4 |
| Proactive action frequency | 1 per 3–5 user messages |
| Joke/compliment frequency | 1 per 10–15 meaningful exchanges |

---

## 5. State Tracking for Proactivity

Minimal persistent counters (reset per session or project):

```rust
struct ProactiveState {
    topic_frequency: HashMap<String, u32>,    // "caching" → 3
    last_proactive_turn: u32,                  // turn index
    jokes_this_session: u8,
    compliments_this_session: u8,
    corrections_this_session: u8,
    user_interruption_count: u32,
    preferred_pace: PacePreference,            // Fast / Normal / Deliberate
}
```

- `preferred_pace` inferred from user response latency and interruption pattern
- Fast user → shorter chunks, fewer pauses
- Deliberate user → more context per chunk, longer pauses

---

## 6. Anti-Patterns (What Not To Do)

| Anti-Pattern | Correction |
|---|---|
| Wall of text (5+ sentences) | Split into chunks, stream |
| Multiple questions in one message | One question per chunk |
| Unsolicited long explanation | Offer: "Want the deep dive?" → only expand on yes |
| Proactive action during user crisis | Detect high-stakes language → switch to reactive/supportive |
| Joke during error recovery | Suppress humor when `stakes > threshold` |
| Repeating same suggestion | Track `topic_frequency` + user rejections; stop after 2 declines |
| Ignoring interruption | Immediate pivot, acknowledge in ≤ 1 sentence |

---

## 7. Implementation Notes

### 7.1 Streaming Architecture
- SSE frames: `type: "chunk"`, `content: "..."`, `pause_ms: 400`
- Client renders incrementally; server can cancel queued chunks on interruption
- `X-Request-Id` correlation for traceability

### 7.2 Proactive Trigger Evaluation (per user message)
```
IF user expresses problem/uncertainty → queue background lookup
IF topic_frequency[topic] > 2 AND not recently suggested → queue suggestion
IF detectable error/anti-pattern → queue correction
IF stakes LOW AND session > 10 turns AND jokes_this_session == 0 → queue joke
IF user completed meaningful task → queue compliment
```

### 7.3 Turn Construction
```
build_turn(user_input, proactive_queue) -> Vec<Chunk>
  1. Process interruption (clear queue if topic changed)
  2. Generate reactive response (1–2 chunks)
  3. Pop 0–1 proactive item (rotate types: suggestion, lookup, question, joke)
  4. Append pacing pause if more chunks follow
  5. Return chunk stream
```

---

## 8. Example Full Exchange

```
User: "The API is slow again."
Agent: "Checking..."                                    [chunk 1, +300ms]
Agent: "p99 latency 2.3s — bottleneck is `/users` endpoint."  [chunk 2, +400ms]
Agent: "Recent deploy added N+1 query in `get_user_orgs`. Revert or optimize?"  [chunk 3, end]

User: "Optimize."
Agent: "Adding `joinedload(orgs)` cuts it to 120ms. Want me to PR?"  [chunk 1, +350ms]
Agent: "Also: consider caching org lookup — 90% hit rate expected."   [chunk 2, suggestion, end]

User: "PR the fix, I'll handle caching."
Agent: "PR #247 opened. Tests passing. Ready for review."           [chunk 1, +300ms]
Agent: "Nice call splitting the work — caching is lower risk separate." [chunk 2, compliment, end]
```

---

## 9. Configuration Knobs (Per-User/Per-Project)

```toml
[conversation]
max_chunk_sentences = 3
inter_chunk_delay_ms = { min = 300, max = 800 }
max_chunks_before_yield = 4
proactive_frequency = 0.25        # 1 proactive per 4 user turns
joke_frequency = 0.08
compliment_frequency = 0.07
correction_threshold = "medium"   # low/medium/high — min severity to speak up
pace_adaptation = true
```

---

*This design keeps the agent feeling like a competent colleague who talks in bursts, listens well, jumps in with useful work, and never monopolizes the conversation.*