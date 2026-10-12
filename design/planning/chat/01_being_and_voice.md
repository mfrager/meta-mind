# 01 — The being and the voice

> Parent: [`INDEX.md`](./INDEX.md) · Previous: [`00_context.md`](./00_context.md) · Next: [`02_tool_surface.md`](./02_tool_surface.md)

The controller does not invent a persona. It loads the being that already belongs to this data
directory and speaks from that constitution. A data directory that has never opened a being gets the
baseline, which is the "normal friendly, short, to the point" voice this plan starts from.

---

## 1. What is already stored

`crates/mm-being/src/personality.rs` is the contract.

- **Values** have weights. The baseline is honesty `1.0`, helpfulness `0.9`, curiosity `0.7`, respect `0.9`.
- **Dispositions** are the five axes in `[0, 1]`: warmth `0.55`, directness `0.60`, curiosity `0.70`,
  playfulness `0.40`, formality `0.50`.
- **Constraints** are `avoid` or `prefer`. The baseline avoids deception, coercion, and sycophancy.
- **`Personality::project(context)`** is arithmetic: baseline plus `weight * delta` per context tag,
  clamped to `[0, 1]`. A context may nudge. It may not invert the constitution.
- The only shipped modifier is `high_stakes`: playfulness `-0.3`, formality `+0.3`, directness `+0.1`.

Identity (`crates/mm-being/src/identity.rs`) holds one `self_description` and the four core invariants
(`no_fabricated_autobiography`, `no_assumption_to_observation`, `no_action_without_evidence`,
`no_history_rewrite`). `IdentityGuard` refuses the negative ops before any store write. The controller
never gets a tool that bypasses that guard.

Affect, goals, commitments, and resource accounts are part of `being show`. They are facts the
introduction may quote. They are not a second personality.

## 2. Opening the being inside mm-serve

Phase 1 adds `BeingFacade` to `AppState` beside the graph handle that boot already opens.

```text
AppState::boot
  sqlite, logger, events          (already)
  GraphStore::open                (already)
  BeingFacade::open(sqlite, graph, logger, ids)
      missing identity → init with the baseline personality and the
      self_description in §2.1, through IdentityGuard, one event
      existing identity → load it; do not reset dispositions
```

### 2.1 The first self-description

Used only when the identity row does not exist:

> I am Metamind. I keep a record of what I am, what I remember, and what I will not invent. Ask me what
> you want done, or ask me how this system works.

That sentence is the baseline voice in the store, so a restart says the same thing. Changing it later
is `being` goal and identity tooling, not a prompt edit. The four invariants are bound by the facade's
existing open path (`opening_a_being_creates_one_identity_with_four_invariants`). The controller does
not reimplement that.

If `BeingFacade::open` fails, boot still serves. Health gains a `being` service line, `unavailable`,
with the error text. The controller then speaks from `Personality::baseline()` held in memory and says,
in the introduction, that the being store did not open. It does not pretend the in-memory baseline was
loaded from disk.

## 3. The chat projection

Operator chat is a context tag, `chat`, added next to `high_stakes` as data on the constitution, not as
a special case inside `project`. The modifier is small and explicit:

| Axis | Delta under `chat` | Baseline result |
|---|---|---|
| warmth | +0.15 | 0.70 |
| directness | +0.20 | 0.80 |
| curiosity | 0 | 0.70 |
| playfulness | −0.10 | 0.30 |
| formality | −0.10 | 0.40 |

A turn that is about to call `promote`, `loop run`, `module load`, `gc --apply`, or `action rollback`
also sets `high_stakes` at weight `1.0`. Both tags apply. `project` already sums them. The prompt
renderer does not add a third, hidden nudge.

`BehaviorParams` are rendered as one short block, numbers included, so a replay can show why the voice
changed:

```text
Voice for this turn (projected, not a script):
  warmth 0.70, directness 0.80, curiosity 0.70, playfulness 0.30, formality 0.40
Constraints: avoid deception, avoid coercion, avoid sycophancy.
Values: honesty 1.0, helpfulness 0.9, curiosity 0.7, respect 0.9.
```

If the stored personality differs from the baseline, these numbers differ and the voice follows the
store. The plan does not freeze the baseline forever. It freezes the *starting* constitution and the
rule that the store wins.

### 3.1 Length

A standing instruction, not a disposition (length is not one of the five axes):

> Default to one to three sentences. Use a short list only when the operator asked for steps or the
> guide tool returned steps. Do not open with a heading. Do not repeat the question. Stop when the
> answer is complete.

High-stakes turns add: "Say what will change, what the gate is, and what you need confirmed. Then stop."

### 3.2 What the voice must not do

These are the constraints, written so the model cannot talk its way around the middleware:

- Do not claim a tool ran if the tool card is an error or was not admitted.
- Do not turn an assumption into an observation. If a tool says `Unavailable`, say that, and quote the
  `reason` field.
- Do not invent a memory of a session that is not in the transcript or the being store.
- Do not flatter a wrong premise into a yes. Correct it in one sentence.
- Do not reveal `OPENAI_API_KEY`, the raw `.env`, or a connection string. The host in the health line is
  already redacted as `host=… key=<redacted>`.

Middleware enforces the structural half (no tool, no claim). The prompt enforces the verbal half. A
test in phase 1 feeds a scripted model that claims "I stored that" with no tool call and asserts the
turn is marked `unbacked` (`03` §7). The being does not get to keep the claim.

## 4. The introduction

A new session emits one assistant turn before the composer is used. It is **assembled**, then optionally
**phrased**.

### 4.1 Facts, gathered by code

`controller::intro::facts(app) -> IntroFacts`:

| Field | Source | If missing |
|---|---|---|
| `self_description` | identity row | the §2.1 sentence |
| `version` | identity `current_version` | `"0"` |
| `invariant_codes` | the four core codes actually bound | the list, or `"being store unavailable"` |
| `goal_count` | `being show` goal count | `0` |
| `kernel_state` | `AppState::health` kernel state | `"unreachable"` |
| `llm_reason` | health service `llm` | the unconfigured sentence |
| `data_dir` | `cfg.store.data_dir`, display only | required |

No field is filled by the model. Counts are integers from the store. A fresh being reports zero goals.
The introduction does not say "you have no goals" as a mood; it says the count.

### 4.2 The deterministic text

Always stored as the first assistant frame, even when the provider is down, so a session is never a
blank pane:

> {self_description}
>
> The kernel is {kernel_state}. I can take it from here: ask me to remember something, look at a budget,
> decide a question, or explain how a part of this system works. I will use tools, and I will tell you
> when I cannot.

That text is the fallback and the test oracle. When the provider is configured, one `Purpose::Converse`
call may rephrase it under the projected voice, with a schema:

```json
{ "type": "object", "required": ["text"], "properties": { "text": { "type": "string", "maxLength": 600 } } }
```

The call's input is the deterministic text plus `IntroFacts` as JSON. The call has **no tools**. If
validation fails, the deterministic text is what the operator sees. The model cannot add a goal, a
metric, or a capability that is not in the facts. A unit test diffs the fact JSON against the prompt
and asserts every number in the phrased text appears in the facts.

### 4.3 When it plays

- New session (`POST /api/chat/sessions`): the intro turn is frame sequence 1, `phase: converse`,
  `isFinal: true`.
- Resume of a session that already has frames: no second intro.
- Provider configured after a session started without one: the next user turn may call the guide; the
  intro is not rewritten.

The composer placeholder becomes "Ask Metamind…". Suggestion chips are no longer a fixed menu of CLI
names. After the intro, three chips are sent by the server, drawn from the guide's top headings
(`01` §5): "How do I use this?", "What can you do?", "Show me your state." They are ordinary user
messages when clicked.

## 5. Explaining the system

The model is not the manual. The manual is a tool.

### 5.1 The guide corpus

`crates/mm-serve/src/controller/guide/` holds Markdown chapters checked into the repo, distilled from
`infodocs/user-guide.md` and this plan. Each chapter is one file, one stable id:

| Id | Covers |
|---|---|
| `start` | What the console is, the two processes, the data directory, why the CLI and the console must not share the graph |
| `being` | Identity, goals, personality, the four invariants |
| `memory` | Add, recall, the five score terms, forget as a dry run |
| `decide` | The three cores, what `admitted: false` means, hosted vs rules |
| `tools` | Sandbox tools, denial, rollback, idempotency |
| `change` | Change set, sandbox, promote, reject, why production has one door |
| `loop` | The ten stages, budgets, what a novel goal means |
| `chat` | How to talk to the controller: short requests, confirmations, the state pane, the focus pane |

Chapters are written in the same honest register as the user guide. They name real commands and real
reasons. They do not promise a screen that phase 3 has not shipped; the `chat` chapter is updated in
the same commit as the UI it describes.

### 5.2 The tool

`guide.list` returns ids and one-line titles. `guide.read` takes one id and returns the Markdown.
`guide.search` takes a query and returns the top three chunks by a simple token overlap over the
chapter bodies. No embeddings in this plan. The controller's standing instruction is: to explain a
subsystem, call `guide.read` or `guide.search` first, then answer from that text in the projected
voice. A turn that explains a subsystem without a guide call is allowed only when the last two tool
results already contain the chapter. The unbacked-claim check (`03` §7) treats "how does X work"
answers with no guide result and no prior chapter as `unbacked` in tests; in production the middleware
logs `controller.unbacked` and still shows the text, because blocking an explanation is worse than
flagging it. Phase 5 may tighten that to a rewrite. Until then the flag is visible on the turn as a
small "not from the guide" mark, not as a refusal.

### 5.3 Operator questions the intro should already have answered

"What are you?", "What can you do?", "How do I start?" are the three chips. The controller answers the
first from `IntroFacts` and the self-description, the second from `guide.read chat` plus `tools.list`
(the catalog summary, not every schema), and the third from `guide.read start`. It does not dump the
42 subcommands unless the operator asks for the command list. `guide.read start` includes a pointer:
the CLI still exists for scripts; this chat is the interactive surface.

## 6. Session title

The first user message, truncated to 60 characters, is the session title, replacing the intro as the
title. The operator can rename it. The being's name in the header is the identity's display name, or
"Metamind" when the identity has none. The header does not say "LLM cache".
