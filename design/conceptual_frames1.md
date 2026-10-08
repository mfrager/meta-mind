Yes. **Conceptual frames are a missing cognitive primitive.** They are what let the being organize raw observations, goals, memories, emotions, and plans into *meaningful situations*.

A frame should be more than a prompt template or ontology class. It should be a **temporary structured model of “what kind of situation this is, what entities matter, what relationships are relevant, what normally happens, and what actions make sense.”**

## 1. Add a Frame System

```text
                    WORLD MODEL
                         │
                    observations
                         ▼
                  FRAME ACTIVATION
                         │
              ┌──────────┴──────────┐
              ▼                     ▼
        Existing frame         Construct frame
              │                     │
              └──────────┬──────────┘
                         ▼
                 CONCEPTUAL FRAME
                         │
       ┌─────────────────┼─────────────────┐
       ▼                 ▼                 ▼
   Interpretation     Prediction        Action
       │                 │                 │
       └─────────────────┼─────────────────┘
                         ▼
                    EXPERIENCE
                         │
                         ▼
                 FRAME LEARNING
```

The frame becomes the **cognitive lens** through which the being interprets a situation.

---

# 2. What a frame contains

A useful frame might be:

```json
{
  "id": "software_debugging",
  "type": "problem_solving",

  "roles": {
    "actor": "developer",
    "system": "software_system",
    "failure": "unexpected_behavior",
    "evidence": "observations"
  },

  "state_variables": [
    "expected_behavior",
    "observed_behavior",
    "hypothesis",
    "confidence",
    "reproduction_status"
  ],

  "relationships": [
    "cause",
    "dependency",
    "constraint",
    "effect"
  ],

  "goals": [
    "explain_failure",
    "restore_expected_behavior"
  ],

  "typical_actions": [
    "observe",
    "reproduce",
    "hypothesize",
    "test",
    "modify",
    "verify"
  ],

  "failure_modes": [
    "premature_conclusion",
    "symptom_fix",
    "confirmation_bias"
  ],

  "questions": [
    "What changed?",
    "Can it be reproduced?",
    "What evidence distinguishes hypotheses?"
  ]
}
```

The frame tells the cognitive system **what matters**.

---

# 3. Frames should be compositional

This is crucial.

Don't create one gigantic frame for every possible situation.

Instead:

```text
Problem Solving
    +
Software
    +
Debugging
    +
Production
    +
High Stakes
    +
Collaborative
```

produces:

```text
Current Frame
=
Problem Solving
⊕ Software
⊕ Debugging
⊕ Production
⊕ High Stakes
⊕ Collaboration
```

This fits your existing contextual-personality architecture extremely well.

The same underlying being can therefore enter:

```text
engineering frame
teaching frame
negotiation frame
friendship frame
research frame
planning frame
conflict frame
creative frame
decision frame
```

without switching personalities.

---

# 4. Frames contain expectations

A frame is fundamentally predictive.

If the being enters:

```text
"restaurant reservation"
```

it expects:

```text
restaurant
date
time
party size
location
availability
reservation
confirmation
```

If the user says:

> "Friday night somewhere nice downtown."

the frame fills in the conceptual structure:

```text
Goal:
    obtain restaurant reservation

Known:
    date = Friday
    time = evening
    quality = nice
    location = downtown

Unknown:
    party size
    exact time
    cuisine
```

This is much more powerful than simple intent classification.

---

# 5. Frames should have slots

Classic frame theory is useful here.

```text
FRAME: PURCHASE

buyer
seller
product
price
currency
quantity
payment_method
delivery
deadline
constraints
```

When something is missing, the frame knows **which information is missing**.

That gives the agent a principled way to decide whether to ask a question.

Instead of:

> "What should I ask?"

it computes:

$$
MissingSlots(Frame)
$$

then:

$$
QuestionValue(slot)
$$

and asks only if the missing slot materially affects the goal.

---

# 6. Frames should have scripts

Frames describe **states**.

Scripts describe **expected sequences**.

For example:

```text
PURCHASE SCRIPT

discover
→ evaluate
→ select
→ purchase
→ confirmation
→ fulfillment
→ completion
```

Or:

```text
DEBUGGING SCRIPT

observe
→ reproduce
→ isolate
→ hypothesize
→ test
→ fix
→ verify
```

The agent can detect:

> "We're currently between hypothesis and test."

This gives it temporal structure.

---

# 7. Frames should have causal models

A sophisticated frame should contain causal relationships.

```text
NETWORK FAILURE

high latency
    ← congestion
    ← interference
    ← routing
    ← overloaded endpoint

packet loss
    ← interference
    ← buffer overflow
    ← link failure
```

Then the frame supports:

```text
observation
→ candidate causes
→ interventions
→ predicted outcomes
```

So frames become miniature **domain world models**.

---

# 8. Frames should encode affordances

One of the most important additions:

> Given this situation, what can I do?

For example:

```text
FRAME: ERROR

affordances:
    investigate
    reproduce
    explain
    ignore
    escalate
    repair
    prevent
```

A frame therefore transforms:

$$
Situation \rightarrow Possible\ Actions
$$

This connects frames directly to your planner.

---

# 9. Frames should contain social models

Consider:

> "My coworker hasn't responded to my message."

Possible frames:

```text
communication delay
work coordination
social rejection
conflict
overload
negotiation
```

Each frame changes interpretation.

The system shouldn't immediately conclude:

> "They are ignoring you."

Instead:

```text
Frame hypotheses:

normal_delay       P=.42
busy                P=.31
missed_message     P=.17
avoidance           P=.07
conflict            P=.03
```

Then determine what evidence would distinguish them.

This is where frames and Bayesian inference become extremely powerful.

---

# 10. Frames should be nested

```text
SOCIAL INTERACTION
    └── COLLABORATION
         └── PROJECT
              └── DEADLINE
                   └── BLOCKER
                        └── CONFLICT
```

The active frame can therefore inherit properties from its parents.

$$
Frame_{child}
=
Frame_{parent}
+
\Delta_{child}
$$

This prevents the system from needing millions of independent frames.

---

# 11. Frames should be dynamically constructed

The system shouldn't only retrieve predefined frames.

It should be able to create:

```text
known frames
+
new observations
+
existing concepts
+
relationships
=
temporary frame
```

For example, it encounters a novel business situation:

```text
No exact frame exists.
        ↓
Retrieve:
    negotiation
    marketplace
    subscription
    enterprise procurement
        ↓
compose
        ↓
novel conceptual frame
```

That temporary frame can later become a reusable frame.

This is a natural mechanism for **concept formation**.

---

# 12. Frames should be learned from experience

After repeated encounters:

```text
episodes
   ↓
common structure
   ↓
abstract pattern
   ↓
frame candidate
   ↓
validation
   ↓
new frame
```

For example:

```text
Episode 1:
    customer asks for discount

Episode 2:
    customer asks for discount

Episode 3:
    customer threatens to leave

Episode 4:
    customer asks about competitor pricing
```

Eventually:

```text
FRAME:
    price negotiation / retention risk
```

The system has learned a **conceptual category**, rather than memorizing individual conversations.

---

# 13. Frames should interact with personality

This is where the architecture becomes particularly interesting.

Same frame:

```text
CONFLICT
```

Different personality/context:

```text
assertive character
→ confront directly

diplomatic character
→ preserve relationship

analytical character
→ examine disagreement

humorous character
→ defuse tension

protective character
→ defend user
```

So:

$$
Behavior =
f(
Frame,
Personality,
Relationship,
Goals,
State
)
$$

rather than personality simply generating responses.

---

# 14. Frames should interact with goals

A goal activates relevant frames.

```text
Goal:
    "Help me launch my company."

        ↓

Frames:
    product development
    project management
    marketing
    legal
    finance
    hiring
    deployment
    customer acquisition
```

The agent can move between frames as the project evolves.

This gives the being a **conceptual workspace** for a long-running project.

---

# 15. Frames should interact with self-management

This is perhaps the most important integration.

The being should be able to recognize:

> "I'm applying the wrong conceptual frame."

For example:

```text
Problem persists
      ↓
SELF.DEBUG
      ↓
current frame = "technical bug"
      ↓
evidence doesn't fit
      ↓
switch frame
      ↓
"configuration problem"
      ↓
new explanation
```

So frame switching becomes a form of **cognitive flexibility**.

---

# 16. Add frame operations to the internal API

I'd add:

```text
FRAME.activate()
FRAME.deactivate()
FRAME.inspect()
FRAME.compose()
FRAME.inherit()
FRAME.switch()
FRAME.compare()
FRAME.fill_slot()
FRAME.find_missing()
FRAME.predict()
FRAME.generate_questions()
FRAME.generate_affordances()
FRAME.simulate()
FRAME.learn()
FRAME.generalize()
```

And especially:

```text
FRAME.challenge()
```

which asks:

> "What if my current frame is wrong?"

---

# 17. Frame competition

The being should maintain multiple interpretations when uncertainty is meaningful.

```text
OBSERVATION
     │
     ├── Frame A: debugging        .51
     ├── Frame B: configuration    .29
     ├── Frame C: infrastructure   .14
     └── Frame D: user error       .06
```

Then evidence updates them:

$$
P(F_i|E)
\propto
P(E|F_i)P(F_i)
$$

This gives the system **perspective flexibility**.

---

# 18. Frame → meaning → action

I'd make this one of the fundamental pipelines:

```text
OBSERVATION
     ↓
FRAME
     ↓
ROLE ASSIGNMENT
     ↓
STATE INTERPRETATION
     ↓
CAUSAL MODEL
     ↓
GOAL RELEVANCE
     ↓
AFFORDANCES
     ↓
PLAN
     ↓
ACTION
```

And afterward:

```text
ACTION
 ↓
OUTCOME
 ↓
FRAME VALIDATION
 ↓
FRAME UPDATE
```

That makes conceptual frames **active cognitive models**, rather than static semantic labels.

---

# 19. Ultimately, the being needs a Frame Graph

I would make this a major component of your architecture:

```text
                    FRAME GRAPH
                         │
        ┌────────────────┼────────────────┐
        │                │                │
     Concepts         Situations       Scripts
        │                │                │
        └────────────────┼────────────────┘
                         │
                    Causal Models
                         │
                    Affordances
                         │
                     Goals
                         │
                    Predictions
```

Each frame is a reusable **cognitive lens** connecting:

**concepts → entities → roles → relationships → states → causes → expectations → goals → actions.**

And the artificial being continually learns which frames are useful, when they fail, when to combine them, and when to invent new ones.

That gives you a much deeper architecture:

> **Memory tells it what happened.
> The world model tells it what exists.
> Conceptual frames tell it what a situation means.
> Goals tell it what matters.
> Values tell it why it matters.
> Planning tells it how to change it.
> Self-management tells it how to manage itself while doing so.
> Personality tells it how it tends to behave.
> The LLM turns the resulting cognitive state into language.**

At that point, **concept formation and frame formation become part of the being's development**, rather than something we manually program into it.

