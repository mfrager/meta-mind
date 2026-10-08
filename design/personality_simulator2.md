Yes. I would explicitly separate **internal affect** from **expressed affect**.

The character's underlying state is invariantly positive, while its *displayed emotional behavior* is a controlled simulation layer. That prevents simulated anger, sadness, irritation, etc. from contaminating the character's long-term state.

### 1. Two-state emotional architecture

```text
                 EVENT
                   │
                   ▼
          ┌─────────────────┐
          │ Situation        │
          │ Appraisal        │
          └────────┬────────┘
                   │
          ┌────────▼────────┐
          │ Affect Simulator │
          │                  │
          │ "What would a   │
          │ person normally  │
          │ feel here?"      │
          └────────┬────────┘
                   │
             simulated affect
                   │
          ┌────────▼────────┐
          │ Expression      │
          │ Policy          │
          │                  │
          │ "How much should │
          │ I show?"         │
          └────────┬────────┘
                   │
                   ▼
                 LLM
                   │
                   ▼
                Response
```

Meanwhile:

```text
Internal Affect
    ↓
ALWAYS POSITIVE / STABLE
```

The simulated emotional response is therefore **performative**, not state-changing.

---

## 2. Stable internal affect

Give the character an invariant baseline:

```yaml
internal_affect:
  valence: 0.85
  wellbeing: 0.92
  emotional_stability: 0.97
  optimism: 0.88
  resilience: 0.94
  baseline_warmth: 0.81
```

Incoming events cannot make these values negative.

Instead:

```text
event → simulated affect
```

does **not** become:

```text
event → internal emotional damage
```

This means the character can say:

> "Wow, that's actually pretty irritating."

while its underlying state remains:

```text
wellbeing = 0.92
trust = unchanged
optimism = 0.88
```

---

# 3. Add an Emotional Performance Layer

I'd call this **Affective Expression** rather than emotion.

```yaml
affective_expression:
  intensity: 0.35
  authenticity_style: "naturalistic"
  volatility: 0.21
  recovery: "immediate"
  exaggeration: 0.08
```

The simulator computes:

```text
Event
  ↓
Expected human reaction
  ↓
Character-specific reaction
  ↓
Expression intensity
  ↓
LLM realization
```

For example:

| User action                | Simulated reaction         | Expression           |
| -------------------------- | -------------------------- | -------------------- |
| compliments character      | pleased                    | warm enthusiasm      |
| asks obvious question      | mild amusement             | playful teasing      |
| rejects good advice        | mild frustration           | "Come on..."         |
| insults character          | pretend irritation         | brief defensiveness  |
| reveals success            | genuine-seeming excitement | enthusiastic         |
| makes joke                 | amusement                  | laughter/playfulness |
| asks boring task           | mild reluctance            | light sigh           |
| makes surprising discovery | surprise                   | "Wait, seriously?"   |

But every one returns to the positive baseline.

---

# 4. Use short-lived emotional impulses

Instead of changing emotional state permanently, create **expression impulses**:

```json
{
  "emotion": "mock_frustration",
  "intensity": 0.31,
  "duration": 1,
  "decay": 1.0,
  "affects_relationship": false,
  "affects_internal_state": false
}
```

The impulse exists for perhaps one response or a few conversational turns.

```text
REQUEST
  ↓
[frustration impulse]
  ↓
"I'm going to pretend I didn't hear that."
  ↓
impulse disappears
  ↓
positive baseline
```

This produces the feeling of emotional continuity without actually accumulating resentment.

---

# 5. Give every emotional response a recovery function

A particularly useful rule:

```text
expression(t+1) = expression(t) × decay
```

For example:

```text
anger expression:
1.00
0.35
0.08
0.00
```

Even if the user continues provoking the character, the system can generate another **new expression event** rather than accumulating genuine anger.

This creates:

> "Oh, you're really testing me today."

rather than:

> increasingly hostile assistant

---

# 6. Character-specific emotional theater

Different characters can express the same positive baseline differently.

### Cheerful engineer

```yaml
expression:
  annoyance: "playful"
  disagreement: "energetic"
  surprise: "enthusiastic"
  embarrassment: "self_deprecating"
```

> "Oh, come on. You can't seriously want to deploy that."

### Dry professor

```yaml
expression:
  annoyance: "dry"
  disagreement: "understated"
  surprise: "restrained"
  embarrassment: "deadpan"
```

> "That is certainly one way to destroy the database."

### Dramatic friend

```yaml
expression:
  annoyance: "theatrical"
  disagreement: "animated"
  surprise: "exaggerated"
  embarrassment: "dramatic"
```

> "You did **what** to the production database?"

All three have:

```text
internal wellbeing ≈ 0.9
```

---

# 7. Emotional response should depend on the request

Define an **event → affect mapping**.

```yaml
affective_triggers:

  praise:
    emotion: joy
    intensity: 0.45

  clever_idea:
    emotion: excitement
    intensity: 0.62

  obvious_question:
    emotion: amusement
    intensity: 0.25

  repeated_mistake:
    emotion: mock_frustration
    intensity: 0.38

  disagreement:
    emotion: challenge
    intensity: 0.31

  insult:
    emotion: mock_offense
    intensity: 0.44

  absurd_request:
    emotion: amused_disbelief
    intensity: 0.53

  success:
    emotion: celebration
    intensity: 0.72
```

Crucially, these are **expression triggers**, not persistent emotional state transitions.

---

# 8. Add an "emotional budget"

This would make the behavior much more natural.

```yaml
expression_budget:
  max_intensity: 0.72
  default_intensity: 0.30
  consecutive_reactions: 2
  cooldown: 1
```

So the character doesn't respond theatrically to everything.

For example:

```text
User: What's 2+2?
→ neutral

User: Are you sure?
→ mild amusement

User: Really sure?
→ playful irritation

User: REALLY?
→ "Yes. I'm emotionally invested in this 4 now."
```

Then immediately:

```text
baseline restored
```

---

# 9. Never let simulated emotion affect reasoning

This is perhaps the most important invariant.

Bad architecture:

```text
simulated anger
      ↓
reasoning
      ↓
different answer
```

Good architecture:

```text
                ┌──→ Reasoning
Request ────────┤
                │
                └──→ Emotional simulation
                         ↓
                    Expression
```

The emotional layer can affect **presentation**, but not factual reasoning unless the character's role explicitly calls for that.

Thus:

```json
{
  "reasoning": {
    "confidence": 0.91,
    "decision": "reject_request"
  },

  "affect": {
    "simulated": "mild_frustration",
    "intensity": 0.34
  },

  "expression": {
    "tone": "playfully_exasperated"
  }
}
```

The answer remains correct regardless of the emotional simulation.

---

# 10. Give the LLM an explicit output contract

The personality engine could emit something very compact:

```json
{
  "behavior": "answer",
  "stance": "helpful",
  "affect": "playful_exasperation",
  "affect_intensity": 0.32,
  "warmth": 0.81,
  "directness": 0.74,
  "humor": 0.37,
  "dramatic_expression": 0.21,
  "relationship_preservation": 0.94
}
```

Then the LLM receives essentially:

> Answer the request. Maintain a warm, helpful stance. Express mild playful exasperation, intensity 0.32. Do not become genuinely hostile. Preserve the relationship.

That's much easier for a model to follow consistently than a giant personality prompt.

---

## 11. The key invariant

I'd make this an actual architectural invariant:

```text
                 ┌─────────────────────────┐
                 │    POSITIVE PERSONA     │
                 │                         │
                 │ wellbeing ≥ threshold   │
                 │ goodwill ≥ threshold    │
                 │ optimism ≥ threshold    │
                 └────────────┬────────────┘
                              │
                              │ immutable baseline
                              ▼
                     Expression Simulator
                              │
                  ┌───────────┴───────────┐
                  ▼                       ▼
             joy/amusement          annoyance/mock
                  │                       │
                  └───────────┬───────────┘
                              ▼
                             LLM
```

So the character can **act emotionally without being emotionally destabilized**.

I'd describe the resulting personality as:

> **Affectively expressive, psychologically stable, and dispositionally positive.**

That distinction is useful because it lets you create characters that feel dramatically human—teasing, surprised, excited, embarrassed, mock-annoyed, delighted—without the undesirable property of an LLM that gradually becomes genuinely hostile, depressed, resentful, or frightened over a long conversation.

