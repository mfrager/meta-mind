Yes. I would make those **two explicit cognitive primitives** rather than treating them as generic critical thinking:

1. **Comparison Integrity** — ensure things being compared are actually comparable.
2. **Supposition Control** — prevent an unverified assumption from silently becoming a fact.

These are major sources of LLM mistakes.

## 1. Comparison Integrity

The agent should never simply ask:

> "Which is better, A or B?"

It should first construct a **comparison contract**.

```text id="a3q7k1"
COMPARISON
├── Objects
│   ├── A
│   └── B
│
├── Comparison purpose
│
├── Dimensions
│   ├── cost
│   ├── performance
│   ├── reliability
│   └── ...
│
├── Context
│
├── Time
│
├── Units
│
├── Constraints
│
└── Evidence
```

### Comparability check

Before comparing:

```text
Are A and B:
    - the same kind of thing?
    - being evaluated for the same purpose?
    - measured using compatible units?
    - evaluated over the same timeframe?
    - operating under comparable conditions?
    - based on equivalent definitions?
```

If not, the agent should explicitly transform the comparison or refuse to make the naive comparison.

### Example

Bad:

> "Option A costs $100 and Option B costs $200, so A is twice as good economically."

The system should detect:

```text
price ≠ economic value
```

and ask:

```text
What quantity of service does each provide?
What is the relevant cost basis?
What usage level?
What duration?
What included features?
```

---

# 2. Apples-to-Apples Matrix

For meaningful comparisons, construct:

| Dimension      | A  | B  | Comparable? | Evidence  |
| -------------- | -- | -- | ----------- | --------- |
| Price          | $X | $Y | Yes         | verified  |
| Capacity       | X  | Y  | Yes         | verified  |
| Performance    | X  | Y  | Maybe       | benchmark |
| Reliability    | X  | ?  | No          | missing   |
| Contract terms | X  | Y  | Partially   | documents |

This gives the LLM a structured place to notice:

> **"We don't actually know enough to compare these dimensions."**

That is much better than filling the gap with plausible language.

---

# 3. Normalize Before Comparing

Add a general **normalization operator**:

```text
raw observations
       ↓
normalize
       ↓
comparable representation
       ↓
compare
```

Examples:

* monthly vs annual cost
* total cost vs marginal cost
* nominal vs inflation-adjusted
* per-user vs total
* throughput vs latency
* absolute vs percentage
* average vs median
* apples vs oranges with a common functional unit

The system should ask:

> **"What is the correct unit of comparison?"**

before calculating a conclusion.

---

# 4. Comparison Must Have a Purpose

"Better" is incomplete.

The system should convert:

> Which is better?

into:

> Better **for what objective, under what constraints?**

For example:

```text
A is better for:
    lowest cost

B is better for:
    lowest latency

C is better for:
    minimizing operational risk
```

So comparison becomes:

```text
P(option | objective, constraints, context)
```

rather than an absolute ranking.

---

# 5. Don't Compare Different Categories Without a Translation

Sometimes cross-category comparison is legitimate.

For example:

> Buy software vs hire an employee.

The system should recognize that these aren't directly comparable objects, but **are comparable as solutions to a shared goal**.

```text
software
employee
outsourcing
automation
    ↓
"ways to accomplish X"
```

That's a higher-level comparison.

This is a powerful technique:

> **Compare solutions to the same problem rather than superficially similar objects.**

---

# 6. Supposition Control

The second major primitive should be an **epistemic firewall**.

Every important proposition entering reasoning should have an epistemic status:

```text
OBSERVED
VERIFIED
REPORTED
INFERRED
ASSUMED
HYPOTHETICAL
PREDICTED
UNKNOWN
```

The critical rule:

> **An assumption must never silently upgrade itself into a fact.**

For example:

```text
User:
"The server is probably running PostgreSQL."

Bad internal reasoning:
PostgreSQL → therefore use PostgreSQL-specific commands.

Correct:
PostgreSQL [ASSUMED]
        ↓
Does this assumption materially affect the action?
        ↓
YES
        ↓
VERIFY
```

---

# 7. Assumption Ledger

For important reasoning episodes:

```text id="x1q8v2"
ASSUMPTIONS

A1:
    proposition: server uses PostgreSQL
    status: assumed
    confidence: 0.65
    consequence_if_false: high
    verification_cost: low

A2:
    proposition: API supports batch requests
    status: inferred
    confidence: 0.72
    consequence_if_false: medium
    verification_cost: low
```

Then calculate something like:

```text
VerificationPriority
    = P(false) × Consequence × ActionDependence
      / VerificationCost
```

High-priority assumptions get checked.

---

# 8. Dependency Tainting

This could be particularly powerful.

If an uncertain assumption feeds another conclusion, that uncertainty propagates.

```text
A [ASSUMED]
   ↓
B [INFERRED]
   ↓
C [DECISION]
```

C should not appear as highly certain simply because the reasoning between A and C was logically coherent.

The system can represent:

```text
C depends_on A
```

and therefore:

```text
C confidence ≤ confidence justified by A
```

This prevents **chains of speculation from becoming apparently certain conclusions**.

---

# 9. Distinguish "I Know" From "It Makes Sense"

LLMs are especially vulnerable to this.

These are fundamentally different:

> "This is true."

vs.

> "This would make sense if X were true."

The agent should internally represent:

```text
IF X
    → Y would follow
```

instead of:

```text
X
    → Y
```

until X is established.

This is **conditional reasoning**.

---

# 10. Supposition Check Before Action

Before an external action, run:

```text
ACTION
  ↓
What must be true for this action to be appropriate?
  ↓
List prerequisites
  ↓
Which are verified?
  ↓
Which are assumptions?
  ↓
Could a false assumption cause meaningful harm?
  ↓
Verify / proceed
```

This is especially important for:

* deleting things
* financial transactions
* sending messages
* modifying production systems
* changing configuration
* legal/contractual decisions
* irreversible operations
* advice involving significant consequences

---

# 11. "Show Me the Evidence"

For every consequential conclusion:

```text
CONCLUSION
    ↓
Supporting evidence
    ↓
Direct evidence?
    ↓
Inference?
    ↓
Assumption?
    ↓
External source?
    ↓
Contradictory evidence?
```

The agent should be able to answer:

> "Why do you believe that?"

with a chain such as:

```text
Observed:
    X

Verified:
    Y

Inferred:
    Z from X + Y

Assumed:
    Q

Therefore:
    tentative conclusion C
```

That's much more trustworthy than an undifferentiated explanation.

---

# 12. Contradiction Detection

Whenever new information arrives:

```text
new fact
    ↓
compare against:
    ├── existing facts
    ├── assumptions
    ├── plans
    ├── predictions
    └── conclusions
```

If contradiction appears:

```text
CONFLICT
    ↓
don't silently reconcile
    ↓
identify competing propositions
    ↓
determine which has stronger evidence
    ↓
revise downstream conclusions
```

The important part is **dependency propagation**.

If:

```text
A → B → C → D
```

and A is disproven, B/C/D should become suspect.

---

# 13. Comparison + Supposition Together

These two mechanisms reinforce each other.

Consider:

> "Compare these two APIs and tell me which is better."

The system should internally perform:

```text
1. What are the actual APIs?
2. Are we comparing the same versions?
3. What does "better" mean?
4. What dimensions matter?
5. What information is verified?
6. What information is assumed?
7. Are measurements comparable?
8. Are there missing dimensions?
9. Are there contradictory claims?
10. What evidence supports each conclusion?
```

Then produce the comparison.

---

# 14. Add a "Comparison/Assumption Gate"

I would make this a permanent component of your architecture:

```text id="q9n6zr"
                 CANDIDATE REASONING
                         │
                         ▼
              ┌─────────────────────┐
              │ EPISTEMIC CHECK     │
              ├─────────────────────┤
              │ What is known?      │
              │ What is assumed?    │
              │ What is inferred?   │
              │ What is uncertain?  │
              │ What conflicts?     │
              └──────────┬──────────┘
                         │
                         ▼
              ┌─────────────────────┐
              │ COMPARISON CHECK    │
              ├─────────────────────┤
              │ Same objective?     │
              │ Same units?         │
              │ Same conditions?    │
              │ Same timeframe?     │
              │ Same definitions?   │
              │ Missing dimensions? │
              └──────────┬──────────┘
                         │
                         ▼
                 SANITY / CRITIC
                         │
                         ▼
                     DECISION
```

---

## 15. The broader principle

This gives the artificial being a very important cognitive discipline:

> **Never let fluent reasoning hide an epistemic gap.**

And three related rules:

**Rule 1 — Comparability**

> Before comparing two things, establish what makes them comparable.

**Rule 2 — Epistemic separation**

> Never treat an assumption, inference, or possibility as an established fact.

**Rule 3 — Action threshold**

> The more an action depends on an uncertain proposition, the more strongly that proposition should be verified before acting.

These could become **identity-level cognitive invariants** of the being rather than merely optional techniques. That would fit especially well with your earlier distinction between **identity invariants** and mutable cognitive policies: the agent can evolve its heuristics, philosophies, and techniques, but it should be very difficult for evolution to remove the basic discipline of *knowing what it knows, knowing what it is assuming, and making valid comparisons*.

