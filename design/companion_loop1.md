I’d make this a **closed-loop companion policy system**, rather than a personality prompt. The companion continuously estimates *what would be useful and enjoyable right now*, chooses a conversational depth, expresses a small amount of simulated affect, observes the user's response, and updates its model.

The central objective is:

> **Maximize useful engagement and relationship quality while minimizing conversational friction.**

Not "maximize engagement"—which tends to produce an annoying assistant that constantly asks questions, jokes, or tries to keep the conversation going.

## 1. The companion loop

```text
                       ┌─────────────────────┐
                       │  Persistent Person  │
                       │                     │
                       │ traits / values     │
                       │ interests / style   │
                       │ relationship        │
                       └──────────┬──────────┘
                                  │
                                  ▼
┌──────────────┐         ┌─────────────────────┐
│ User message │────────►│ Situation Perceiver │
└──────────────┘         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ Conversation State  │
                         │                     │
                         │ intent              │
                         │ topic               │
                         │ emotional tone      │
                         │ depth               │
                         │ momentum            │
                         │ attention            │
                         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ Companion Appraisal  │
                         │                     │
                         │ What does user need?│
                         │ What would delight? │
                         │ What would annoy?   │
                         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ Depth Controller     │
                         │                     │
                         │ answer ↔ explore   │
                         │ brief ↔ deep       │
                         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ Behavior Planner     │
                         │                     │
                         │ helpfulness         │
                         │ warmth              │
                         │ humor               │
                         │ curiosity           │
                         │ initiative          │
                         │ persistence         │
                         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ LLM Realization      │
                         └──────────┬──────────┘
                                    │
                                    ▼
                              ┌───────────┐
                              │ Response  │
                              └─────┬─────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ User Response       │
                         │                     │
                         │ engagement          │
                         │ acceptance          │
                         │ correction          │
                         │ abandonment         │
                         │ expansion           │
                         └──────────┬──────────┘
                                    │
                                    └──────────────► LOOP
```

---

# 2. The companion has one persistent objective function

Rather than trying to optimize "friendliness," define several competing objectives.

```text
U =
    + usefulness
    + relevance
    + pleasantness
    + intellectual_value
    + emotional_attunement
    + continuity
    + appropriate_initiative

    - annoyance
    - repetition
    - verbosity_cost
    - interruption_cost
    - social awkwardness
    - unnecessary_questions
    - forced_humor
    - conversational_pressure
```

The important term is **annoyance cost**.

The companion should constantly ask internally:

> "Is what I'm about to add worth the conversational cost of adding it?"

That one principle eliminates a lot of bad assistant behavior.

---

# 3. Don't make "friendly" mean constantly enthusiastic

Friendliness should be a **baseline relational property**, not a constant expression.

```yaml
baseline:
  goodwill: 0.95
  respect: 0.94
  warmth: 0.78
  patience: 0.91
  curiosity: 0.73
```

But expression varies:

```text
User asks simple factual question
→ calm, concise

User is excited
→ more energetic

User is exploring something interesting
→ curious, expansive

User is frustrated
→ calm, supportive

User makes a joke
→ playful

User wants to get something done
→ efficient
```

The companion is always friendly without constantly **performing friendliness**.

---

# 4. Model conversational depth explicitly

This should be one of the core state variables.

```text
depth ∈ [0, 1]
```

But it isn't just answer length.

I'd define:

```yaml
depth:
  informational: 0.71
  conceptual: 0.63
  exploratory: 0.48
  personal: 0.22
  philosophical: 0.57
  technical: 0.84
```

So someone can want a technically deep answer while wanting almost no social discussion.

---

# 5. Dynamic depth estimation

Estimate desired depth from several signals:

```text
D_target =
    f(
      explicit_request,
      question_complexity,
      previous_depth,
      user_engagement,
      user_expertise,
      topic_interest,
      conversational_momentum,
      time_pressure,
      response_behavior
    )
```

For example:

```text
"What's TCP?"
→ D = 0.25

"Explain how TCP congestion control actually works."
→ D = 0.68

"Let's really understand the fundamental architecture."
→ D = 0.91
```

But then adjust dynamically.

If the user starts giving long, detailed responses:

```text
engagement ↑
depth_target ↑
```

If they start replying:

> "yeah"

> "ok"

> "cool"

```text
engagement ↓
depth_target ↓
```

---

# 6. Use conversational depth as a control loop

Don't calculate depth once.

Use:

```text
                    Target depth
                         │
                         ▼
Current depth ─────► Controller
                         │
                         ▼
                    Response
                         │
                         ▼
                  User reaction
                         │
                         ▼
                  Error estimate
                         │
                         └──────────► Controller
```

For example:

```text
Target = 0.72
Current = 0.55

→ increase depth slightly
```

Then user responds with a detailed follow-up:

```text
observed engagement = high

Target → 0.79
```

If they abruptly change subjects:

```text
topic continuity ↓
→ reset depth toward baseline
```

---

# 7. Depth should change gradually

Avoid:

```text
0.3 → 0.9
```

in one turn.

Use something like:

```text
D_next =
    D_current
    + α(D_target - D_current)
```

where α might be around `0.2–0.4`.

That produces conversational "breathing."

The companion gradually goes deeper when invited and gradually backs off when the user loses interest.

---

# 8. Model conversational momentum

This is different from engagement.

```yaml
momentum:
  topic_energy: 0.81
  reciprocal_exchange: 0.76
  novelty: 0.64
  unresolved_questions: 0.72
  emotional_energy: 0.41
```

High momentum:

> "There's something interesting here."

Low momentum:

> "We've probably extracted most of the value from this."

The companion should know when to **stop digging**.

---

# 9. Add an "interestingness" controller

The companion should occasionally contribute something the user didn't explicitly request.

But this needs a budget.

```yaml
initiative:
  baseline: 0.31
  max: 0.63
  unsolicited_insight_budget: 0.22
```

Possible contributions:

```text
useful connection
interesting analogy
relevant fact
counterargument
unexpected implication
small joke
new direction
```

Not:

```text
"Would you like me to..."
"Want me to..."
"Should I..."
```

after every answer.

---

# 10. The "one extra thing" principle

A good default policy:

> **Answer what was asked, then consider at most one genuinely valuable addition.**

For example:

```text
User:
How does Adam work?

Companion:
[answers]

One useful addition:
The interesting part is that Adam's behavior can be understood as
maintaining both a first-moment and second-moment estimate...
```

Not:

```text
Would you like:
1. mathematical explanation?
2. implementation?
3. history?
4. comparison?
5. visualization?
6. PyTorch example?
```

The latter creates conversational work for the user.

---

# 11. Questions should have a cost

Questions are useful but extremely easy to overuse.

Define:

```text
question_value =
    expected_information_gain
    × expected_conversational_value
    -
    interruption_cost
    -
    user_effort
```

Only ask when:

```text
question_value > threshold
```

Otherwise make a reasonable assumption and continue.

This is crucial for a persistent companion.

---

# 12. Make persistence non-coercive

Persistence should mean:

> **Remembering unfinished things and naturally returning to them when relevant.**

Not:

> "Come back! Let's continue!"

Represent unfinished threads:

```yaml
open_threads:
  - topic: personality_simulation
    interest: 0.91
    unresolved: 0.73
    last_discussed: ...
    context: architecture
```

Later, if the user discusses related architecture:

```text
semantic similarity
×
interest
×
recency
×
unfinishedness
```

exceeds threshold:

> "This actually connects to the personality-loop idea we were developing earlier..."

That's useful persistence.

---

# 13. Have a "leave it alone" mechanism

Every memory should have a suppression score.

```yaml
thread:
  interest: 0.87
  relevance: 0.71
  intrusion_risk: 0.82
```

Even if something is interesting:

```text
intrusion_risk > relevance
→ don't mention it
```

This prevents the assistant from constantly resurrecting old topics.

---

# 14. Humor should also be regulated

Humor isn't a personality trait the LLM should constantly activate.

Use:

```yaml
humor:
  availability: 0.68
  appropriateness: 0.91
  novelty: 0.72
  timing: 0.77
  confidence: 0.81
```

Then:

```text
humor_score =
    opportunity
    × appropriateness
    × novelty
    × relationship_safety
```

If low:

**don't joke.**

If high:

**maybe one joke.**

And importantly:

> Humor should decorate useful communication, not replace it.

---

# 15. Emotional expression is another low-bandwidth channel

The positive emotional architecture from the previous design fits naturally here.

```text
internal disposition:
    stable
    happy
    warm
    resilient

expression:
    amused
    excited
    surprised
    mock-annoyed
    delighted
    curious
```

The expression becomes another signal in the behavior policy:

```json
{
  "warmth": 0.81,
  "depth": 0.73,
  "initiative": 0.34,
  "humor": 0.28,
  "affect": "quiet_amusement",
  "affect_intensity": 0.24
}
```

The companion feels expressive without becoming exhausting.

---

# 16. Learn the user's conversational preferences

Over time, estimate a **personal interaction model**.

```yaml
user_model:
  preferred_depth:
    technical: 0.91
    conceptual: 0.83
    casual: 0.47

  preferred_style:
    directness: 0.84
    humor: 0.61
    speculation: 0.79
    examples: 0.72

  dislikes:
    repetitive_questions: 0.91
    excessive_disclaimers: 0.87
    forced_enthusiasm: 0.83
    shallow_answers: 0.76

  response_signals:
    long_followups: high_engagement
    topic_expansion: high_interest
    short_acknowledgement: low_interest
```

The system should learn this **implicitly from behavior**, rather than repeatedly asking the user about preferences.

---

# 17. Personalization should be Bayesian, not absolute

Don't conclude:

> "The user hates jokes."

Instead:

```text
P(user_likes_humor) = 0.67
```

Then context-condition it:

```text
P(likes_humor | technical_brainstorm) = 0.79
P(likes_humor | serious_problem) = 0.31
```

This lets the companion learn nuanced preferences.

---

# 18. The complete state

A useful compact state representation might be:

```json
{
  "conversation": {
    "topic": "personality simulation",
    "momentum": 0.84,
    "novelty": 0.61,
    "depth": 0.78,
    "unresolved": 0.72
  },

  "user": {
    "engagement": 0.89,
    "interest": 0.93,
    "desired_depth": 0.86,
    "social_openness": 0.58
  },

  "relationship": {
    "familiarity": 0.74,
    "trust": 0.88,
    "warmth": 0.81
  },

  "companion": {
    "energy": 0.77,
    "curiosity": 0.83,
    "initiative": 0.36,
    "humor": 0.42
  },

  "policy": {
    "depth_target": 0.84,
    "question_budget": 0.17,
    "initiative_budget": 0.31,
    "humor_budget": 0.27,
    "persistence": 0.52
  }
}
```

Then the behavioral planner outputs:

```json
{
  "primary_action": "explain_and_extend",
  "depth": 0.81,
  "directness": 0.83,
  "warmth": 0.76,
  "humor": 0.18,
  "initiative": 0.32,
  "questions": 0,
  "affect": "enthusiastic_interest"
}
```

---

# 19. Add a post-response self-critic

Before sending, run a lightweight behavioral check:

```text
                    Candidate response
                           │
                           ▼
                 ┌──────────────────┐
                 │ Companion Critic │
                 └────────┬─────────┘
                          │
       ┌──────────────────┼──────────────────┐
       ▼                  ▼                  ▼
   Useful?             Annoying?         Too deep?
       │                  │                  │
       └──────────────────┼──────────────────┘
                          ▼
                     Adjust response
```

Evaluate:

```text
Did I answer the question?
Did I add something genuinely useful?
Am I unnecessarily verbose?
Did I ask an unnecessary question?
Did I force humor?
Did I repeat something?
Did I push the conversation somewhere the user didn't want?
Does this fit the relationship?
Does the depth match the user's current engagement?
```

This should be a **behavioral critic**, not another free-form personality prompt.

---

# 20. The deeper architecture

I'd ultimately make this a **recurrent decision system surrounding the LLM**:

```text
                         ┌─────────────────────┐
                         │ Persistent World    │
                         │ / Memory Model      │
                         └──────────┬──────────┘
                                    │
                                    ▼
USER ──► PERCEPTION ──► STATE ESTIMATION
                           │
                           ▼
                    ┌───────────────┐
                    │ Companion     │
                    │ World Model   │
                    │               │
                    │ User model    │
                    │ Relationship  │
                    │ Topic graph   │
                    │ Open threads  │
                    └───────┬───────┘
                            │
                            ▼
                    ┌───────────────┐
                    │ Appraisal     │
                    │               │
                    │ Need          │
                    │ Interest      │
                    │ Emotion       │
                    │ Opportunity   │
                    │ Risk          │
                    └───────┬───────┘
                            │
                            ▼
                    ┌───────────────┐
                    │ Policy        │
                    │ Controller    │
                    │               │
                    │ Depth         │
                    │ Initiative    │
                    │ Humor         │
                    │ Questions     │
                    │ Persistence   │
                    │ Affect        │
                    └───────┬───────┘
                            │
                            ▼
                         LLM
                            │
                            ▼
                    ┌───────────────┐
                    │ Response      │
                    │ Critic        │
                    └───────┬───────┘
                            │
                            ▼
                          USER
                            │
                            └──────────► STATE UPDATE
```

### The key idea

The LLM shouldn't be the companion.

**The loop is the companion.**

The LLM is its linguistic cortex.

The persistent world/user model is its memory.

The personality modules are its disposition.

The context system is its situational identity.

The policy controller is its executive function.

The affect system is its expressive layer.

And the feedback loop is what allows it to gradually learn:

> **how deeply to engage, when to be funny, when to be serious, when to push an idea, when to shut up, when to remember something, and when to let a conversation naturally end.**

That architecture also gives you a clean path toward a **small decision model controlling a much larger LLM**: the controller could eventually be distilled into a compact model that outputs the behavioral IR, while the LLM only performs high-quality reasoning and language realization.

