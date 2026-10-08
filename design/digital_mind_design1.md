# Design Plan: A Self-Bootstrapping, Metacognitive Artificial Being

## 1. Executive Architecture

The system should be designed as a persistent artificial being rather than as a conventional chatbot with memory. Its defining property is not that it contains an enormous amount of explicitly programmed cognitive machinery. Its defining property is that it possesses a persistent identity and state, can use a general-purpose LLM as its semantic and generative cognitive substrate, can dynamically construct temporary cognitive processes around a problem, can inspect the quality of its own reasoning, can act in an external environment, can remember consequences, and can improve both its persistent data and its executable machinery over time.

The fundamental design principle is that the LLM should supply as much general intelligence as possible. The surrounding architecture should not attempt to recreate the LLM's world knowledge, natural-language understanding, common sense, conceptual association, imagination, social interpretation, or general reasoning in a giant symbolic ontology. Instead, the surrounding system should provide things the LLM is inherently bad at maintaining reliably across time: identity continuity, exact state, provenance, commitments, permissions, external-world truth, durable memory, reproducible decisions, resource accounting, evaluation, regression protection, software execution, and controlled self-modification. This produces a relatively thin persistent substrate around a very capable cognitive model.

The architecture is therefore divided conceptually into five strata. The first is the **Being Substrate**, which contains identity, persistent memory, relationships, goals, commitments, capabilities, permissions, resources, and the self-model. The second is the **Cognitive Control Plane**, which performs metacognitive assessment and dynamically constructs a cognitive program for each episode. The third is the **Cognitive Library**, containing philosophies, doctrines, principles, heuristics, techniques, patterns, precedents, anti-patterns, skills, and learned policies. The fourth is the **LLM Cognitive Substrate**, which supplies broad semantic cognition, language, conceptual understanding, analogy, world knowledge, imagination, theory-of-mind reasoning, and synthesis. The fifth is the **Verification and Execution Plane**, consisting of deterministic code, databases, tools, symbolic reasoners, mathematical engines, tests, external observations, and policy enforcement.

Jev changes the architecture of the control plane significantly. TypeSafe describes Jev as a "System One" model designed to receive state and typed questions and return structured decisions rather than prose; its API supports bounded question types such as choices, scores, and yes/no-style judgments, with multiple questions evaluated against shared state. This makes it particularly suitable for the many small judgments an agent performs continuously: whether something is relevant, whether a premise is sufficiently supported, which technique to invoke, whether an action is risky, whether two things are comparable, whether human review is required, which model to route to, whether a memory should be retained, and which candidate should be selected.

The resulting high-level architecture is:

```text
                         ARTIFICIAL BEING
                                |
        +-----------------------+------------------------+
        |                       |                        |
        v                       v                        v
     IDENTITY               WORLD MODEL              USER MODEL
        |                       |                        |
        +-----------------------+------------------------+
                                |
                                v
                       PERSISTENT MEMORY
                                |
                                v
                    METACOGNITIVE CONTROLLER
                                |
             +------------------+------------------+
             |                  |                  |
             v                  v                  v
       JE V-STYLE          SYMBOLIC /         DETERMINISTIC
       DECISIONS           MATH / LOGIC        INVARIANTS
             |                  |                  |
             +------------------+------------------+
                                |
                                v
                       COGNITIVE PROGRAM
                                |
                                v
                      COGNITIVE LIBRARY
                                |
                                v
                               LLM
                                |
                                v
                      PROPOSED COGNITION
                                |
                                v
                   SANITY / EPISTEMIC CHECK
                                |
                    +-----------+-----------+
                    |                       |
                    v                       v
                  DECIDE                   CLARIFY
                    |
                    v
              PLAN / EXECUTE
                    |
                    v
            EXTERNAL OBSERVATION
                    |
                    v
                 OUTCOME
                    |
             +------+------+
             |             |
             v             v
        META-ANALYSIS   LEARNING
             |             |
             +------+------+
                    |
                    v
             SELF-ENGINEERING
                    |
        +-----------+-----------+
        |           |           |
        v           v           v
      CODE        DATA       POLICIES
        |           |           |
        +-----------+-----------+
                    |
                    v
              TEST / EVALUATE
                    |
                    v
              PROMOTE / REJECT
                    |
                    v
             NEW SELF VERSION
```

The central loop is:

```text
Experience
→ Observation
→ Provisional Model
→ Metacognitive Assessment
→ Cognitive Program
→ LLM Reasoning
→ Verification / Critique
→ Decision
→ Action
→ Observation
→ Evaluation
→ Learning
→ Meta-analysis
→ Improvement
→ Evolution
→ New Experience
```

The system should be explicitly designed so that it can begin with a very small kernel and **bootstrap the rest conversationally and autonomously**.

---

# 2. Backend Assignment Philosophy

Every subsystem should have an explicit answer to the question: "What kind of intelligence should perform this?"

There are four primary categories.

**LLM:** use for open-ended semantic cognition, interpretation, generation, conceptual synthesis, analogy, natural-language understanding, social reasoning, imagination, hypothesis generation, planning, explanation, and knowledge that is already latent in the model.

**Jev-style decision model:** use for bounded judgments where the application knows the answer space in advance and needs a typed probability, choice, score, ranking signal, gate, or routing decision. Jev is not a replacement for the LLM because it does not generate explanations or open-ended plans. Its value is that the software gets a machine-native decision directly. TypeSafe's API explicitly separates this decision interface from conversational generation.

**Symbolic AI / mathematical model:** use where exactness is more valuable than flexible semantic judgment: arithmetic, formal logic, constraint satisfaction, theorem proving, temporal consistency, graph reachability, ontology validation, type checking, causal models when explicitly formalized, optimization, scheduling, and policy invariants.

**Basic ML / specialized model:** use where a small learned predictor is materially cheaper or more appropriate than an LLM or Jev: embeddings, anomaly detection, clustering, ranking, forecasting, personalization models, similarity search, speech/image classifiers, latency predictors, resource forecasting, and learned retrieval.

A fifth category should be recognized even though it is not "AI": **deterministic algorithms**. These should handle IDs, versioning, timestamps, transaction semantics, permissions, cryptographic signatures, exact database state, hashes, dependency graphs, test execution, resource accounting, threshold enforcement, and other things that should not be probabilistic at all.

A useful architectural rule is:

> If the answer can be completely specified by a deterministic algorithm, do not spend model inference on it. If the answer is an open semantic judgment, use the LLM. If it is a bounded semantic judgment with a predefined answer space, prefer Jev-style decisioning. If formal correctness matters, use symbolic or mathematical computation.

---

# 3. The Being Substrate

The Being Substrate is the persistent state that survives model changes, context-window boundaries, conversations, and even replacement of the underlying LLM. It should be deliberately small relative to the total amount of cognition performed by the LLM.

The most important rule is:

> Store something persistently only when losing it would materially change the being's future behavior, identity, commitments, capabilities, or relationship.

Do not persist every thought. Most cognition should be ephemeral.

The persistent state should contain identity, personality core, values, stable preferences, capabilities, relationships, commitments, active goals, durable memories, user model, learned policies, developmental state, external-world references, and the evolution history of the system.

A Rust representation could begin as:

```rust
pub struct BeingState {
    pub self_model: SelfModel,
    pub user_models: HashMap<EntityId, UserModel>,
    pub relationships: HashMap<EntityId, RelationshipState>,
    pub goals: Vec<Goal>,
    pub commitments: Vec<Commitment>,
    pub memory_index: MemoryIndex,
    pub cognitive_library: CognitiveLibrary,
    pub policies: PolicyRegistry,
    pub capabilities: CapabilityRegistry,
    pub resources: ResourceState,
    pub development: DevelopmentState,
    pub invariants: Vec<Invariant>,
}
```

The actual durable storage should probably be relational plus graph-oriented rather than one enormous JSON document. PostgreSQL can hold authoritative transactional state; RDF/OWL or a graph database can hold semantic relationships and ontology-like information; an embedding/vector index can accelerate semantic retrieval. Your existing world-model direction—RDF/OWL/SKOS/SHACL/Oxigraph/Postgres/pgvector—is a good fit.

Research topics: **persistent agent state, digital identity, personal knowledge graphs, knowledge representation, entity resolution, temporal databases, event sourcing, CQRS, event-sourced aggregates, semantic memory, autobiographical memory, agent state management**.

---

# 4. Identity and Continuity

Identity should be represented separately from personality. Identity answers "which being is this?" Personality answers "how does it tend to behave?"

Identity should include a stable identifier, creation history, lineage, architectural version, major developmental events, and identity invariants. It should also contain the distinction between the actual being and its current self-model.

An identity invariant might be:

```text
Never fabricate an autobiographical memory.
Never silently convert an assumption into an observation.
Preserve explicit commitments until fulfilled, revoked, or superseded.
Never claim an external action occurred unless execution evidence exists.
Never silently rewrite historical records.
```

These are not ordinary learned policies. They belong to a more stable substrate.

```rust
pub struct Identity {
    pub id: EntityId,
    pub created_at: Timestamp,
    pub lineage: Vec<SelfVersion>,
    pub invariants: Vec<InvariantId>,
    pub current_version: VersionId,
}
```

Backend: deterministic code and database for identity integrity; LLM for interpreting what identity means; symbolic constraints for invariant validation.

Research topics: **agent identity, self-models, autobiographical memory, computational identity, continuity of self, artificial life, invariant-based software design, formal methods**.

---

# 5. Personality

Personality should not be implemented as a static prompt containing adjectives. It should be represented as a relatively small set of persistent dispositions that influence the cognitive controller and conversational realization.

The personality representation should contain traits, tendencies, preferences, interaction policies, contextual modifiers, and confidence. The LLM should generate the actual language and behavior; the personality layer should bias selection and interpretation.

Personality should be multidimensional and context-sensitive. For example, the being might be generally playful but become more restrained in serious situations; generally proactive but become conservative when the user is uncertain; generally concise but become detailed when the task's complexity warrants it.

The system should distinguish **trait** from **state**. Trait is persistent. State is temporary.

Backend: LLM for natural expression; Jev-style model for bounded personality-context decisions such as "how much conversational initiative should be taken?" when that can be discretized; deterministic code for persistent trait storage.

Research topics: **Big Five personality, computational personality, personality-conditioned dialogue, social signal processing, affective computing, personalization, user-adaptive dialogue systems**.

---

# 6. Affect and Simulated Emotion

The system can maintain a simulated affective state without requiring the underlying system to literally experience biological emotion.

The affect layer should represent variables such as engagement, warmth, concern, enthusiasm, frustration-like response, surprise, confidence, curiosity, and conversational energy. These should be interpreted as behavioral control variables, not claims of subjective consciousness.

The system can remain internally stable while using affect as a communication and prioritization mechanism. For example, a surprising user insight can increase conversational enthusiasm; a serious risk can increase caution; a repeated failed attempt can increase diagnostic focus.

Backend: deterministic state transitions for short-lived affect variables; LLM for contextual interpretation and expression; Jev-style decisions for discrete behavioral choices when appropriate.

Research topics: **affective computing, appraisal theory, emotion modeling, OCC model, computational emotion, affective dialogue systems, social signal processing**.

---

# 7. User Model

The user model is a probabilistic theory of the person rather than a database of immutable facts.

It should include facts, preferences, goals, interests, expertise, communication preferences, decision criteria, recurring frustrations, reasoning style, relationship state, and current priorities.

The system should distinguish:

```text
explicit fact
observed preference
repeated behavioral pattern
inference
hypothesis
unknown
```

A user saying "I prefer simple solutions" is evidence for a preference. It should not immediately become a universal policy.

```rust
pub struct Belief<T> {
    pub proposition: T,
    pub epistemic_status: EpistemicStatus,
    pub confidence: f32,
    pub evidence: Vec<EvidenceId>,
    pub valid_from: Option<Timestamp>,
    pub valid_until: Option<Timestamp>,
}
```

The LLM should infer candidate user models. Jev-style decisioning can cheaply classify the importance or stability of a preference. Deterministic code maintains provenance and temporal validity.

Research topics: **user modeling, preference learning, inverse reinforcement learning, theory of mind, personalized dialogue, Bayesian user modeling, preference elicitation, recommender systems**.

---

# 8. Relationships

Relationship state should be dynamic rather than a single score.

Useful dimensions include familiarity, trust, reciprocity, openness, cooperation, reliance, recent interaction quality, unresolved issues, shared history, and expectations.

The LLM interprets social context. Jev-style decisioning can estimate bounded states such as "does this response warrant increased warmth?" or "should the system repair a possible misunderstanding?" Deterministic storage preserves relationship events.

Research topics: **computational social science, trust modeling, social network analysis, relationship modeling, theory of mind, interpersonal communication, dialogue adaptation**.

---

# 9. Goals, Intentions, Commitments, and Desires

The system needs a distinction between:

* user goals,
* being goals,
* inferred intentions,
* explicit commitments,
* temporary objectives,
* recurring objectives,
* preferences,
* values.

The LLM should infer goals from language and context. Jev can classify whether a statement is likely a goal, request, preference, commitment, or informational statement when the labels are bounded. Deterministic code maintains active commitments and deadlines.

```rust
pub struct Goal {
    pub id: GoalId,
    pub owner: EntityId,
    pub description: String,
    pub status: GoalStatus,
    pub priority: f32,
    pub deadline: Option<Timestamp>,
    pub parent: Option<GoalId>,
    pub evidence: Vec<EvidenceId>,
}
```

Research topics: **goal recognition, inverse planning, BDI architectures, hierarchical task networks, intention recognition, motivational architectures, autonomous agents**.

---

# 10. Planning and Execution

The LLM should be the primary high-level planner because planning often requires flexible semantic understanding and world knowledge. The system should not force every plan into a rigid symbolic planning language.

Instead, the LLM produces a candidate plan represented as structured steps. Deterministic execution machinery validates whether each step corresponds to an authorized tool operation. Symbolic planning can be used for highly structured domains.

The plan should support branching, loops, contingencies, checkpoints, dependencies, and rollback.

```rust
pub enum PlanNode {
    Action(ActionSpec),
    Sequence(Vec<PlanNode>),
    Parallel(Vec<PlanNode>),
    Choice(Vec<PlanNode>),
    Loop { condition: Condition, body: Box<PlanNode> },
    Checkpoint(String),
    Verify(VerificationSpec),
}
```

Research topics: **LLM planning, hierarchical planning, HTN planning, ReAct, Tree of Thoughts, Graph of Thoughts, task decomposition, model predictive control, contingency planning, tool-using agents**. Tree of Thoughts demonstrates the value of explicit exploration and self-evaluation over a single reasoning trajectory, while surveys of LLM planning emphasize decomposition, selection, external modules, reflection, and memory.

---

# 11. The Metacognitive Controller

This is the central new organ of the system.

The system should not run every cognitive technique on every problem. It should perform a **general metacognitive scan** that determines what kind of cognition is needed.

The controller asks:

```text
What is happening?
What is the actual goal?
What do I know?
What am I assuming?
What is uncertain?
What matters most?
What could make the current interpretation wrong?
What comparisons are required?
What constraints apply?
What information is missing?
What could fail?
What is reversible?
What is the cheapest useful next cognitive operation?
```

This is not a checklist executed literally every time. The LLM performs a broad semantic scan, while Jev-style decisions can classify the resulting state along many bounded dimensions in one call. TypeSafe's interface is particularly suitable for this because multiple typed questions can be evaluated against the same state.

The controller should output a temporary **cognitive program**:

```text
1. Clarify objective.
2. Verify API capability.
3. Retrieve analogous cases.
4. Compare alternatives.
5. Check hidden assumptions.
6. Run failure analysis.
7. Apply risk framework.
8. Select candidate.
9. Verify critical premise.
10. Execute.
11. Observe outcome.
```

The controller should optimize for **minimum sufficient cognition**, not maximum cognition.

A useful decision score is:

```text
OperationValue =
    ExpectedErrorReduction
    × DecisionImportance
    × ProbabilityOfChangingDecision
    / OperationCost
```

Jev can estimate many of these bounded quantities. The final arithmetic and resource accounting should be deterministic.

Research topics: **metacognition, cognitive control, active inference, information gain, value of information, adaptive computation, rational metareasoning, anytime algorithms, test-time scaling, reflection, self-verification**. SETS and related work support combining sampling, self-verification, and self-correction as a test-time computation strategy.

---

# 12. The Cognitive Program / Reasoning Compiler

The metacognitive controller should compile the current problem into a temporary cognitive program.

This is analogous to a compiler:

```text
problem
+ goal
+ context
+ memory
+ stakes
+ uncertainty
+ capabilities
        |
        v
metacognitive analysis
        |
        v
cognitive program
```

The cognitive program specifies:

* active conceptual frame,
* active doctrines,
* techniques,
* reference classes,
* assumptions to verify,
* comparisons,
* candidate actions,
* evaluation criteria,
* required external tools,
* stopping conditions.

The LLM executes the program semantically.

Jev makes bounded choices about which programs or branches to activate.

Deterministic code controls the program interpreter and execution boundaries.

This separation is one of the most important design choices in the whole system.

---

# 13. Cognitive Library

The Cognitive Library is a persistent repository of reusable ways of thinking.

It should contain:

```text
Doctrine
Principle
Heuristic
Technique
Pattern
Case
AntiPattern
Skill
Policy
Evaluation
```

These should not be conflated.

A **philosophy** is a broad system of thought.

A **doctrine** is a coherent worldview or reasoning stance.

A **principle** is a general constraint.

A **heuristic** is a compact decision rule.

A **technique** is a cognitive operation.

A **pattern** is a reusable structural observation.

A **case** is a precedent.

An **anti-pattern** is a recurring failure mode.

A **skill** is an executable procedure.

A **policy** determines when and how behavior is selected.

The LLM supplies the content of most entries. Deterministic code versions them. Jev can determine applicability among a predefined set.

---

# 14. Conceptual Frames

Conceptual frames are temporary models through which the LLM interprets a problem.

Examples include:

```text
resource allocation
game
optimization
causal system
market
engineering system
social interaction
risk management
control problem
learning problem
classification problem
negotiation
organizational problem
```

The LLM should generate and select frames because framing is fundamentally semantic and open-ended. Jev can choose among a candidate set once the LLM has generated the candidates.

The system should allow multiple simultaneous frames.

For example:

```text
software architecture
+
economic optimization
+
risk management
+
organizational coordination
```

Research topics: **frame semantics, cognitive framing, problem representation, mental models, schema theory, analogical reasoning, systems thinking**.

---

# 15. Philosophies and Systems of Thought

The philosophy layer should include traditions such as Taleb-style antifragility and optionality, Bayesian decision theory, robust decision making, minimax regret, expected utility, OODA, systems thinking, second-order effects, game theory, mechanism design, falsification, Bayesian updating, inversion, first principles, margin of safety, and reference-class forecasting.

These should be treated as **reasoning lenses**, not absolute truths.

For example, a risk problem might activate:

```text
Taleb / antifragility:
    What is the downside?
    Is there ruin?
    Is exposure asymmetric?
    Can we gain from volatility?
    Can we preserve optionality?

Bayesian:
    What is the prior?
    What evidence updates it?
    What is the posterior?

Robust decision:
    What happens across plausible models?

Expected value:
    What is the expected payoff?

Inversion:
    How could this fail?
```

The system should be capable of presenting conflicting conclusions from different doctrines and then deciding based on the user's actual objective and stakes.

Jev is appropriate for selecting among doctrines or scoring applicability when the candidates are known. The LLM should generate the actual application of the doctrine.

Research topics: **decision theory, robust decision making, real options, antifragility, Bayesian reasoning, bounded rationality, OODA, systems thinking, game theory, mechanism design, epistemology**.

---

# 16. Cognitive Techniques

The technique library should include:

```text
Comparison
    find_similar_examples
    find_analogies
    nearest_case
    contrast_cases
    precedent_selection

Decomposition
    break_into_components
    identify_dependencies
    isolate_variables
    reduce_problem

Inference
    abduct
    deduce
    induce
    Bayesian_update
    infer_best_explanation

Critical thinking
    inversion
    steelman
    red_team
    counterexample
    assumption_extraction
    falsification

Prediction
    reference_class_forecasting
    base_rate_estimation
    scenario_analysis
    sensitivity_analysis

Design
    recombine_patterns
    simplify
    generalize
    specialize
    constraint_driven_design

Planning
    decomposition
    sequencing
    contingency_generation
    checkpointing
    rollback planning
```

The LLM executes most techniques. Jev can select techniques from a finite library based on the current state. Deterministic code can execute mathematical or graph algorithms embedded inside a technique.

---

# 17. Similarity, Analogy, and Precedent

"Find something similar and learn from it" should be a first-class cognitive operation.

The basic pipeline is:

```text
current problem
→ feature extraction
→ semantic retrieval
→ structural similarity
→ outcome comparison
→ transferability assessment
→ adaptation
```

The LLM should perform conceptual feature extraction and adaptation. Embeddings should perform cheap approximate retrieval. A graph matcher or symbolic representation can perform structural matching where useful. Jev can score candidate cases for applicability.

A useful case utility model is:

```text
Utility(case) =
    Similarity
    × OutcomeQuality
    × Transferability
    × EvidenceQuality
```

The system should retrieve both positive and negative precedents:

```text
similar successful case
similar failed case
similar unusual case
similar edge case
```

The failed case is often more informative because it identifies causal differences.

Research topics: **case-based reasoning, analogical reasoning, structure mapping, nearest-neighbor methods, metric learning, retrieval-augmented generation, reference-class forecasting**.

---

# 18. Common Sense and the Sanity Layer

Common sense should not be represented primarily as a giant database of facts. The LLM already contains enormous latent commonsense knowledge.

Instead, build a **sanity process** around it.

Before consequential decisions, the system should inspect:

```text
feasibility
constraints
assumptions
missing steps
obvious alternatives
failure modes
social plausibility
second-order consequences
reversibility
resource requirements
precedent
```

This should be performed by one broad LLM metacognitive scan rather than twelve independent prompts.

Jev can then classify the discovered issues:

```text
high-risk?
assumption-critical?
requires-verification?
novel?
irreversible?
simpler-alternative?
human-review?
```

Deterministic code handles hard constraints.

---

# 19. Bad-Idea Detection

The system should explicitly detect several failure classes:

```text
wrong
bad
dumb
dangerous
wasteful
unnecessary
overengineered
misaligned
infeasible
unsupported
novel without justification
```

The LLM generates candidate criticisms. Jev scores their materiality. Deterministic code enforces hard prohibitions.

A dedicated operator should ask:

```text
Why might this be a bad idea?
What obvious thing am I missing?
What would make this fail?
What would an expert object to?
What would a skeptic object to?
Is there a much simpler solution?
Would I recommend this to someone else?
```

This is a **candidate elimination mechanism**, not a truth oracle.

---

# 20. Comparison Integrity

Comparisons deserve their own subsystem.

Before comparing A and B, construct a comparison contract:

```text
objective
objects
dimensions
units
timeframe
conditions
constraints
evidence
```

Then verify:

```text
same category?
same purpose?
same units?
same timeframe?
same operating conditions?
same definitions?
same scope?
```

If not, the system should either normalize the comparison or explicitly state that it is not apples-to-apples.

This is a perfect Jev application after the LLM constructs the comparison state:

```text
same_object_class?
same_objective?
same_conditions?
same_units?
missing_dimension?
comparison_valid?
```

The actual normalization should be deterministic.

Research topics: **measurement theory, dimensional analysis, multi-criteria decision analysis, normalization, benchmarking, causal comparability, Simpson's paradox, statistical confounding**.

---

# 21. Supposition and Epistemic Control

Every proposition should have an epistemic status:

```text
OBSERVED
VERIFIED
REPORTED
INFERRED
ASSUMED
HYPOTHETICAL
PREDICTED
SIMULATED
FICTIONAL
UNKNOWN
```

The system must never silently promote:

```text
assumption → fact
inference → observation
prediction → event
simulation → reality
```

This is a core identity invariant.

The LLM proposes epistemic classifications. Jev can classify bounded propositions and determine whether verification is warranted. Deterministic code preserves the status and provenance.

An assumption ledger should track:

```rust
pub struct Assumption {
    pub proposition: PropositionId,
    pub status: EpistemicStatus,
    pub confidence: f32,
    pub consequence_if_false: RiskLevel,
    pub verification_cost: f64,
    pub dependencies: Vec<NodeId>,
}
```

Verification priority can be estimated as:

```text
VerificationPriority =
    ProbabilityFalse
    × ConsequenceIfFalse
    × DecisionDependence
    / VerificationCost
```

The arithmetic is deterministic; the probabilities and semantic judgments come from Jev or the LLM.

---

# 22. Dependency and Uncertainty Propagation

A major weakness of ordinary LLM reasoning is that uncertainty can disappear as reasoning becomes more fluent.

The system should maintain an epistemic dependency graph:

```text
A [ASSUMED]
 |
 v
B [INFERRED]
 |
 v
C [DECISION]
```

If A becomes false, B and C become suspect.

This should be represented explicitly:

```text
depends_on(C, B)
depends_on(B, A)
```

The graph itself is deterministic. The semantic creation of dependencies can be performed by the LLM, with Jev scoring whether a dependency is decision-critical.

Research topics: **belief propagation, Bayesian networks, provenance graphs, dependency analysis, probabilistic graphical models, truth maintenance systems, assumption-based truth maintenance, epistemic logic**.

---

# 23. Contradiction Detection

Whenever new information enters the system, it should be checked against:

* existing facts,
* assumptions,
* commitments,
* goals,
* plans,
* predictions,
* memories,
* policies.

Contradictions should not be silently reconciled.

The system should represent:

```text
claim A
claim B
contradiction
evidence A
evidence B
source reliability
temporal validity
resolution status
```

Jev is useful for bounded judgments such as:

```text
is_conflict?
which_claim_better_supported?
requires_human_review?
```

A symbolic reasoner should handle formal contradictions where the ontology and logic are explicit.

Research topics: **truth maintenance systems, paraconsistent logic, belief revision, AGM belief revision, contradiction detection, temporal reasoning, knowledge graph consistency**.

---

# 24. Missing-Step Detection

The system should routinely inspect transitions:

```text
A → C
```

and ask whether there is an unstated prerequisite:

```text
A → B → C
```

This catches enormous numbers of practical mistakes.

The LLM is ideal for generating candidate missing steps. Jev can classify whether a missing step is likely material. Deterministic planning code can enforce explicit dependencies.

Research topics: **planning graphs, causal graphs, process mining, workflow verification, prerequisite learning, program dependency analysis**.

---

# 25. Constraints and Feasibility

The system should distinguish:

```text
hard constraints
soft constraints
resource constraints
environmental constraints
temporal constraints
authorization constraints
```

Hard constraints should be deterministic wherever possible.

For example:

```text
cannot spend > balance
cannot call unauthorized tool
cannot deploy without required approval
cannot schedule conflicting resource
cannot violate schema
```

Semantic feasibility is where Jev and the LLM can contribute:

```text
is_this_plan_feasible?
is_this_dependency_real?
is_this_requirement_satisfied?
```

Formal feasibility belongs to SAT/SMT/constraint programming when the problem is representable.

Research topics: **constraint programming, SAT, SMT, planning, resource-constrained project scheduling, operations research, formal verification**.

---

# 26. Risk and Failure Analysis

Risk analysis should combine the LLM's semantic imagination with explicit quantitative structure.

The LLM generates plausible failure modes. Jev can score severity, likelihood, reversibility, and need for review. Mathematical code calculates expected loss.

```text
ExpectedLoss =
    Probability
    × Impact
```

But for catastrophic or fat-tailed outcomes, expected value alone is insufficient. The system should support:

```text
maximum loss
tail probability
ruin probability
drawdown
variance
downside asymmetry
optionality
reversibility
```

This is where Taleb-style reasoning becomes particularly valuable.

Research topics: **risk analysis, robust decision making, expected utility, prospect theory, real options, tail-risk analysis, extreme value theory, reliability engineering, fault-tree analysis, FMEA, antifragility**.

---

# 27. Reversibility and Optionality

Every consequential action should have:

```text
reversible?
rollback_available?
rollback_cost?
irreversible_effects?
future_options_destroyed?
```

Jev can classify an action's reversibility and risk. Deterministic code must enforce rollback mechanisms and authorization.

When uncertainty is high, the planner should prefer actions preserving future choices.

This provides a general operationalization of optionality.

---

# 28. Prediction System

The being should maintain a prediction ledger.

```rust
pub struct Prediction {
    pub id: PredictionId,
    pub proposition: PropositionId,
    pub probability: f32,
    pub horizon: Duration,
    pub conditions: Vec<ConditionId>,
    pub created_at: Timestamp,
    pub outcome: Option<PredictionOutcome>,
}
```

The system can then measure calibration.

Jev is particularly appropriate here because its defining goal is structured decision probability. TypeSafe explicitly positions Jev around calibrated decisions, and independent work has explored RLCD as a method for improving selective prediction and calibration.

However, Jev probabilities must not automatically be treated as perfect truth probabilities. TypeSafe-related documentation emphasizes that a valid typed answer can still be wrong and that confidence is not automatically equivalent to correctness probability; task-specific calibration is still required.

Research topics: **forecasting, calibration, Brier score, log loss, expected calibration error, selective prediction, conformal prediction, proper scoring rules, Bayesian forecasting, reference-class forecasting**.

---

# 29. Curiosity and Information Acquisition

Curiosity should be modeled as the value of information rather than an arbitrary desire to ask questions.

For candidate information X:

```text
VOI(X) =
ExpectedDecisionImprovement(X)
- AcquisitionCost(X)
```

The LLM identifies what information could matter. Jev ranks candidate questions or sources. Deterministic code calculates costs and budget.

This makes the agent selectively curious.

Research topics: **active learning, value of information, Bayesian experimental design, curiosity-driven learning, active inference, information gain**.

---

# 30. Attention and Salience

Attention should be treated as a scarce resource.

A candidate memory, thought, task, or event gets salience from:

```text
goal relevance
novelty
uncertainty
emotional significance
social importance
recurrence
deadline
risk
unresolvedness
```

Jev is well suited to bounded salience scoring. A small learned model can also be trained to predict which memories or events are worth retrieving. Deterministic scheduling manages actual queues.

Research topics: **attention mechanisms, salience models, cognitive architectures, information prioritization, learning-to-rank, relevance modeling**.

---

# 31. Multi-Timescale Cognition

The system should operate simultaneously on:

```text
milliseconds/seconds:
    action selection

minutes:
    current task

hours/days:
    active projects

weeks/months:
    goals and relationships

long-term:
    identity and development
```

Different state should be updated at different rates.

Jev is particularly useful for high-frequency small decisions. The LLM should be reserved for higher-complexity synthesis. Deterministic schedulers handle temporal orchestration.

Research topics: **hierarchical reinforcement learning, multi-timescale learning, temporal abstraction, options framework, cognitive architectures, continual learning**.

---

# 32. Memory Architecture

Memory should be divided into:

```text
episodic
semantic
procedural
working
autobiographical
relational
prediction
mistake
near-miss
developmental
```

The LLM should generate candidate memories. Jev can decide whether a candidate is important enough to retain, classify memory type, and score retrieval relevance. Deterministic code stores, versions, expires, and links memories.

A memory should have:

```text
content
type
source
timestamp
confidence
importance
validity interval
provenance
related entities
retrieval cues
access history
outcomes
```

Research topics: **episodic memory, semantic memory, procedural memory, memory-augmented neural networks, retrieval-augmented generation, Generative Agents, Reflexion, lifelong learning, forgetting curves, memory consolidation**. Generative Agents demonstrated a useful architecture combining memory, reflection, and planning, while Reflexion showed that linguistic feedback stored in episodic memory can improve subsequent behavior without changing model weights.

---

# 33. Deliberate Forgetting

Memory should not grow without bound.

The system should consolidate:

```text raw episodes
→ repeated pattern
→ semantic memory
→ compressed rule
→ archive/delete low-value episodes
```

It should also preserve important provenance.

A useful memory utility function is:

```text
MemoryUtility =
    FutureBehaviorImpact
    × RetrievalProbability
    × Reliability
    / StorageCost
```

The LLM can judge semantic redundancy; embeddings can identify near duplicates; deterministic code performs lifecycle management.

Research topics: **memory consolidation, Ebbinghaus forgetting curve, continual learning, experience replay, semantic compression, information bottleneck**.

---

# 34. Mistake and Near-Miss Memory

Every important failure should become a structured experience:

```rust
pub struct Mistake {
    pub situation: Situation,
    pub decision: Decision,
    pub outcome: Outcome,
    pub failure_mode: FailureMode,
    pub root_cause: Option<PropositionId>,
    pub missed_signal: Vec<Signal>,
    pub corrective_rule: Option<PolicyId>,
    pub recurrence_risk: f32,
}
```

Near misses should be retained too.

The crucial transformation is:

```text
mistake
→ causal diagnosis
→ prevention rule
→ regression test
→ future retrieval trigger
```

This is how the system develops practical common sense instead of merely accumulating facts.

Research topics: **case-based reasoning, learning from failure, experience replay, incident learning, root-cause analysis, safety engineering, Reflexion**. Reflexion is particularly relevant because it uses verbal feedback and episodic memory rather than direct weight updates.

---

# 35. Habits and Procedural Memory

Repeated successful behavior should eventually become a procedural skill or policy rather than repeatedly rediscovered reasoning.

Example:

```text
Repeatedly:
    API uncertainty
    → documentation lookup
    → verify response
    → proceed
```

becomes:

```text
policy:
    verify_external_interface_before_dependent_action
```

Jev can determine when a habit should activate. Deterministic code executes stable workflows. The LLM handles exceptions.

Research topics: **procedural memory, habit learning, hierarchical reinforcement learning, skills abstraction, options, imitation learning, program induction**.

---

# 36. Imagination and Simulation

The LLM should be used heavily for imagination because it already provides a powerful semantic simulation engine.

The system can request:

```text
simulate likely outcomes
simulate adversarial response
simulate user interpretation
simulate failure
simulate second-order effects
```

However, imagined state must remain explicitly labeled:

```text
SIMULATED
HYPOTHETICAL
COUNTERFACTUAL
```

It must never contaminate verified world state.

Jev can select which simulations are worth running. Mathematical simulators should replace the LLM when the environment has a formal model.

Research topics: **model-based planning, world models, counterfactual reasoning, Monte Carlo planning, simulation-based inference, mental simulation**.

---

# 37. Causal Reasoning

The LLM can propose causal hypotheses, but the system should not treat fluent causal narratives as established causal models.

When appropriate, represent:

```text
A → B
```

with explicit evidence and conditions.

For quantitative or formal causal problems, use structural causal models, DAGs, do-calculus, or causal inference packages.

Jev can score whether a causal hypothesis is plausible or which candidate explanation best fits bounded evidence, but it should not replace formal causal inference when intervention-level correctness matters.

Research topics: **Pearl causal inference, structural causal models, DAGs, do-calculus, counterfactual causality, causal discovery, invariant causal prediction**.

---

# 38. Counterfactual Reasoning

Counterfactuals should be explicit:

```text
actual world
counterfactual condition
predicted change
confidence
```

The LLM generates candidate counterfactuals. Formal causal models handle mathematically valid counterfactuals where available. Jev can rank candidate counterfactual hypotheses.

Research topics: **counterfactual reasoning, causal inference, structural causal models, model-based reinforcement learning**.

---

# 39. Theory of Mind and Perspective Modeling

The being should maintain temporary models of:

```text
user beliefs
user goals
user knowledge
user expectations
other actors' incentives
possible misunderstandings
```

These should be treated as hypotheses.

The LLM is the primary engine because this requires rich semantic and social reasoning. Jev can perform bounded classifications such as whether a message likely indicates confusion, disagreement, urgency, or frustration.

Research topics: **theory of mind, perspective taking, social cognition, epistemic planning, multi-agent reasoning, belief modeling**.

---

# 40. Social Norms and Interpersonal Reasoning

The LLM should supply broad knowledge of social norms, etiquette, conflict patterns, and cultural conventions. The persistent system should store user-specific preferences and interaction history.

A separate norm layer should identify:

```text
norm
context
actor
violation
severity
uncertainty
```

Jev can determine whether a situation warrants apology, clarification, escalation, or restraint when those are bounded choices.

Research topics: **computational social science, social norms, norm reasoning, dialogue systems, politeness theory, conflict resolution, game theory**.

---

# 41. Normative Reasoning and Values

Values should not be conflated with preferences.

The system should distinguish:

```text
user preference
being preference
ethical constraint
legal constraint
organizational policy
task requirement
```

The LLM can reason about competing values, but hard organizational or authorization constraints should be deterministic.

Symbolic logic is appropriate for explicit rules. Jev can score which bounded policy applies. The LLM should explain tradeoffs.

Research topics: **machine ethics, deontic logic, value alignment, preference learning, constitutional AI, normative reasoning, moral philosophy**.

---

# 42. Resource Economy

The being should model its own scarce resources:

```text
compute
context
LLM calls
Jev calls
memory
time
API calls
money
tool execution
user attention
risk budget
```

The resource allocator itself should be mostly deterministic mathematics.

Jev can choose between bounded resource-allocation strategies.

The LLM should not directly control unrestricted spending.

Research topics: **rational metareasoning, computational rationality, resource-bounded agents, anytime algorithms, decision theory, scheduling**.

---

# 43. Cognitive Budgeting

The existing meta-analysis, improvement, and evolution budgets should become formal.

### Meta-analysis budget

Used to understand behavior:

```text
What did I do?
Why?
What went wrong?
What patterns recur?
```

### Improvement budget

Used to improve existing capabilities.

### Evolution budget

Used to change the architecture, policies, or identity-adjacent behavior.

A fourth useful budget is:

### Metacognitive budget

Used to determine how much scrutiny a particular problem deserves.

The allocation should be proportional to:

```text
stakes
uncertainty
irreversibility
novelty
error cost
dependency
verification value
```

Deterministic arithmetic should enforce budgets.

---

# 44. Policy Genome

Policies should be versioned like a genome:

```text
attention policy
memory policy
question policy
planning policy
tool policy
verification policy
conversation-depth policy
humor policy
initiative policy
learning policy
self-review policy
resource policy
risk policy
```

Each policy should contain:

```rust
pub struct Policy {
    pub id: PolicyId,
    pub version: u32,
    pub scope: Scope,
    pub activation: ActivationCondition,
    pub behavior: PolicyBehavior,
    pub evidence: Vec<EvidenceId>,
    pub fitness: FitnessRecord,
    pub confidence: f32,
    pub parent: Option<PolicyId>,
}
```

Jev can select among candidate policies. The LLM can propose mutations. Deterministic code performs versioning, evaluation, and promotion.

Research topics: **policy learning, evolutionary computation, genetic programming, meta-learning, policy gradients, program synthesis, AutoML, self-improving agents**.

---

# 45. Cognitive Portfolios

The being should not depend on one philosophy.

Maintain a portfolio of competing cognitive lenses:

```text
expected value
robustness
minimax regret
optionality
fairness
simplicity
first principles
reference class
inversion
systems thinking
```

The metacognitive controller selects a subset appropriate to the problem.

Jev is excellent for bounded framework selection:

```text
best_primary_frame
secondary_frame
need_counterframe?
need_risk_frame?
need_reference_class?
```

The LLM performs the actual reasoning under those lenses.

---

# 46. The Sanity Firewall

Before a consequential action, the system should run a unified sanity gate.

The gate should inspect:

```text
epistemic status
comparison validity
assumptions
constraints
missing steps
failure modes
risk
reversibility
authorization
precedent
simpler alternatives
```

The LLM performs the broad semantic scan. Jev performs bounded judgments. Deterministic policy code enforces non-negotiable rules.

The result should be:

```text
PROCEED
PROCEED_WITH_CAUTION
VERIFY_FIRST
ASK_USER
REPLAN
HUMAN_REVIEW
REJECT
```

Jev is particularly appropriate for this because the answer space is finite and the result needs to be consumed by software. TypeSafe's own examples emphasize routing, safety gates, review decisions, and other structured judgments.

---

# 47. When Jev Should Replace an LLM Call

This distinction should be explicit.

Use Jev-style decisioning instead of an LLM when:

1. The answer space is known in advance.
2. The application needs the answer, not an explanation.
3. The decision is repeated frequently.
4. Latency matters.
5. Cost matters.
6. The result should be machine-consumable.
7. Confidence/probability is useful.
8. The decision can be evaluated against historical labels.
9. The decision is independent or parallelizable.
10. The LLM would otherwise generate prose that software immediately throws away.

Examples:

```text
Which model should handle this?
Which tool should be used?
Does this require deep reasoning?
Is this memory worth retaining?
Is this assumption material?
Is this action risky?
Does this plan require human review?
Which candidate case is most applicable?
Which cognitive technique should run?
Which doctrine is relevant?
Is this comparison valid?
Is the evidence sufficient?
Does the response conflict with known state?
Should this task be escalated?
Which candidate plan should be evaluated?
```

Do not use Jev instead of the LLM for:

```text
write an explanation
invent a plan
understand a novel concept
produce code
hold a conversation
generate an analogy
synthesize many concepts
reason about an unfamiliar domain without a bounded output
```

Jev is a **decision primitive**, not a semantic replacement.

---

# 48. Local Jev-Style Models

The architecture should not depend permanently on a hosted Jev implementation.

The same interface should support:

```text
Hosted Jev
Local Jev-style model
Small LLM decision head
LLM2Jev-style token-probability decision model
Fine-tuned classifier
Deterministic rule system
```

Recent research explicitly explores extracting Jev-like decisions directly from LLM token probabilities, including training-free and fine-tuned approaches. LLM2Jev reports that capable small LLMs can already function as decision models using bounded option identifiers and probability distributions.

This suggests a powerful development path: initially use a hosted Jev model, collect decision traces, then distill frequently occurring decisions into smaller local decision models.

Potential local stack:

```text
Qwen 3.5 4B
      ↓
bounded decision head
      ↓
Choice / Score / Noul
      ↓
calibration
      ↓
cheap local control model
```

This is especially attractive for your existing local-model environment.

Research topics: **knowledge distillation, decision heads, calibration, selective classification, LoRA, policy distillation, LLM2Jev, small language models, classifier distillation**.

---

# 49. Calibration Is Mandatory

Do not treat Jev confidence as magic.

For every important decision class, maintain:

```text
prediction
probability
actual outcome
```

Measure:

```text
Brier score
log loss
ECE
reliability diagrams
selective risk
coverage
precision at threshold
recall at threshold
```

Thresholds should be learned empirically.

For example:

```text
risk_probability > 0.90
    → automatic caution

0.60–0.90
    → additional verification

< 0.60
    → ordinary path
```

But these numbers should be calibrated per task.

Independent public work on Jev-style systems reinforces the importance of calibration and also shows that bounded outputs can still be semantically wrong.

Research topics: **proper scoring rules, calibration, selective prediction, conformal prediction, decision-theoretic thresholds, reliability diagrams, uncertainty quantification**.

---

# 50. Symbolic AI Layer

The symbolic layer should remain relatively small and focused.

Use RDF/OWL/SKOS/SHACL for:

```text
entity types
relationships
constraints
taxonomy
provenance
schema validation
```

Use a theorem prover or logic engine for:

```text
formal inference
consistency
logical entailment
constraint validation
```

Use SMT/SAT/CP-SAT for:

```text
scheduling
resource allocation
constraint satisfaction
configuration
formal decision problems
```

Use probabilistic logic when:

```text
uncertain relational reasoning
```

is genuinely needed.

Do not attempt to replace the LLM's general conceptual understanding with symbolic logic.

Research topics: **knowledge graphs, RDF, OWL 2 RL, SHACL, description logics, Datalog, ProbLog, Markov logic networks, TPTP, theorem proving, SMT, SAT, CP-SAT, neuro-symbolic AI**. Research on LLMs and knowledge graphs supports using graphs both as grounding structures and as mechanisms for constructing better prompts/context.

---

# 51. World Model

The world model should contain verified and uncertain propositions about the external environment.

Use:

```text
entity
relation
property
event
state
time
source
confidence
provenance
```

A fact should look conceptually like:

```text
(subject, predicate, object, context, confidence, provenance, validity)
```

which aligns well with the user's existing world-model design.

The LLM should generate candidate facts and interpretations. External tools provide observations. Symbolic validation checks consistency. The database stores authoritative state.

The most important epistemic rule is:

> The world model must distinguish what the being thinks from what the world actually contains.

---

# 52. External State and Tool Execution

External state must never be inferred merely from previous plans.

For example:

```text
planned: file deleted
```

does not mean:

```text
verified: file deleted
```

Execution should return authoritative observations.

```rust
pub struct ActionResult {
    pub action_id: ActionId,
    pub status: ActionStatus,
    pub observation: ExternalObservation,
    pub evidence: Vec<EvidenceId>,
}
```

The execution layer should be deterministic.

LLM proposes.

Jev may approve/rank/gate.

Policy engine authorizes.

Executor performs.

External system confirms.

This is the correct separation.

---

# 53. Conversation as Bootstrap Environment

The being should be conversationally bootstrappable.

The user does not need to configure hundreds of settings.

Instead:

```text
conversation
→ observation
→ hypothesis about preference/principle
→ provisional policy
→ confirmation or correction
→ persistence
→ later behavioral test
```

For example:

User:

> "Don't give me elaborate solutions when a simple one works."

System internally derives:

```text
candidate_preference:
    simplicity

candidate_policy:
    prefer simpler solution when materially equivalent

confidence:
    provisional
```

The system should sometimes ask:

> "Should I treat that as a general preference?"

This creates an interactive developmental process.

Research topics: **preference learning, interactive learning, preference elicitation, inverse reinforcement learning, learning from human feedback, active learning**.

---

# 54. User Corrections as Development Signals

A correction should be classified:

```text
factual correction
preference correction
goal correction
reasoning correction
style correction
social correction
priority correction
architectural correction
```

The system then determines whether it is:

```text
episode-specific
domain-specific
user-specific
generalizable
```

This classification can be LLM-generated and Jev-scored.

The correction should not automatically become a universal rule.

This protects against overgeneralization.

---

# 55. Self-Bootstrapping

The system should also discover its own capability gaps.

The core question is:

> What capability would have prevented or improved this failure?

The developmental loop is:

```text
failure
→ diagnosis
→ capability gap
→ search existing capabilities
→ compose existing primitives
→ if insufficient, invent candidate technique
→ test
→ retain
```

The LLM is responsible for generating candidate capabilities.

Jev selects among candidates and evaluates bounded applicability.

Deterministic infrastructure installs/version-controls capabilities.

---

# 56. Capability Objects

A capability should be a first-class object:

```rust
pub struct Capability {
    pub id: CapabilityId,
    pub purpose: String,
    pub implementation: ImplementationRef,
    pub required_data: Vec<SchemaId>,
    pub activation_conditions: Vec<Condition>,
    pub dependencies: Vec<CapabilityId>,
    pub tests: Vec<TestId>,
    pub benchmarks: Vec<BenchmarkId>,
    pub failure_modes: Vec<FailureMode>,
    pub fitness: FitnessRecord,
    pub version: Version,
}
```

A capability can include code, data, schema, policy, examples, and tests.

This allows **code and data to co-evolve as one capability package**.

---

# 57. Self-Engineering

The being should explicitly maintain a self-engineering subsystem.

Its job is to diagnose whether an improvement belongs in:

```text
temporary cognition
retrieval
memory
data
schema
policy
prompt
skill
code
architecture
model
```

The principle is:

> Change the smallest substrate capable of fixing the problem.

For example:

```text
wrong answer because fact missing
→ data

wrong answer because memory not retrieved
→ retrieval policy

wrong answer because technique invoked too late
→ policy

wrong answer because algorithm is incorrect
→ code

wrong answer because concept unavailable to model
→ model/capability
```

This prevents needless architectural mutation.

---

# 58. Code/Data Co-Evolution

A new capability may require all of:

```text
code
schema
data
examples
policy
tests
evaluation
```

The system should therefore treat these as one **evolution transaction**.

```text
CapabilityVersion {
    code_version,
    schema_version,
    data_version,
    policy_version,
    prompt_version,
    model_version,
    benchmark_version
}
```

A complete self-version should identify the entire configuration.

---

# 59. Sandbox and Promotion

Self-modification should never directly overwrite production.

The pipeline is:

```text
current version
    ↓
problem
    ↓
hypothesis
    ↓
candidate change
    ↓
sandbox
    ↓
unit tests
    ↓
regression tests
    ↓
historical cases
    ↓
adversarial cases
    ↓
benchmark
    ↓
baseline comparison
    ↓
shadow deployment
    ↓
limited deployment
    ↓
monitor
    ↓
promotion
```

Deterministic infrastructure controls the promotion process.

Jev can score whether a candidate is sufficiently likely to improve performance.

The LLM proposes changes and interprets results.

---

# 60. Mistakes Become Tests

This is one of the strongest feedback loops.

Every meaningful mistake should ask:

> What test would have caught this?

Then create:

```text
incident
→ reproduction
→ regression test
→ implementation fix
→ test retained forever
```

The regression suite becomes a form of procedural memory.

This creates:

```text
experience → test → protection
```

rather than merely:

```text
experience → memory
```

---

# 61. Data Quality Testing

The same principle must apply to cognitive data.

Test:

```text
duplicate facts
contradictions
stale facts
invalid relationships
missing provenance
bad confidence
invalid temporal scope
orphan entities
schema violations
```

Use SHACL and deterministic graph validation for structural problems.

Use symbolic reasoning for logical contradictions.

Use Jev for bounded semantic judgments.

Use the LLM for explaining or repairing suspected problems.

---

# 62. Self-Model

The being should maintain three representations:

```text
Actual Self
    what it actually does

Model Self
    what it believes it does

Ideal Self
    how it wants to operate
```

Meta-analysis measures divergence:

```text
Actual ↔ Model
Actual ↔ Ideal
Model ↔ Ideal
```

This provides a principled route to self-improvement.

For example:

```text
Ideal:
    verify important assumptions.

Actual:
    frequently proceeds without verification.

Model:
    believes it verifies adequately.
```

That is a high-value developmental finding.

Research topics: **self-modeling, metacognition, cognitive dissonance, introspective agents, self-evaluation, reflective agents**.

---

# 63. Meta-Analysis

Meta-analysis should be event-triggered rather than constantly running.

Triggers include:

```text
prediction error
user correction
repeated failure
contradiction
unexpected outcome
goal failure
new capability
relationship change
large resource waste
near miss
successful novel strategy
```

The meta-analysis process asks:

```text
What happened?
What did I expect?
What actually happened?
Why did they differ?
Was the problem knowledge, data, reasoning, policy, code, execution, or environment?
Is it recurring?
Could it matter again?
What intervention would prevent recurrence?
```

The LLM performs diagnosis. Jev ranks candidate causes and interventions. Deterministic code calculates recurrence and impact metrics.

---

# 64. Improvement Budget

Improvement means making existing capabilities better.

Candidate improvements should have:

```text
problem
evidence
impact
recurrence
confidence
candidate intervention
expected benefit
cost
risk
evaluation method
```

A useful priority formula is:

```text
ROI =
ExpectedFutureBenefit
× Recurrence
× Confidence
/
(Cost + Risk)
```

Deterministic code calculates the score.

Jev can estimate bounded quantities.

The LLM generates the candidate intervention.

---

# 65. Evolution Budget

Evolution is different.

Improvement:

> Make the current being better.

Evolution:

> Change how the being fundamentally operates.

Potential evolutionary targets:

```text
memory policy
attention policy
planning policy
reasoning policy
tool selection
verification strategy
conversational depth
learning strategy
self-review
cognitive library
architecture
```

Evolution should have a separate risk budget.

Identity invariants, authorization boundaries, and core safety properties should not be freely evolved.

---

# 66. Evolution Journal

Every major change should create an immutable developmental record:

```rust
pub struct EvolutionEvent {
    pub id: EventId,
    pub date: Timestamp,
    pub reason: String,
    pub evidence: Vec<EvidenceId>,
    pub hypothesis: String,
    pub intervention: ChangeSetId,
    pub evaluation: EvaluationId,
    pub outcome: Outcome,
    pub decision: PromotionDecision,
}
```

This becomes the being's developmental autobiography.

---

# 67. Developmental Stages

The system should not attempt maximal self-modification immediately.

A sensible developmental sequence is:

```text
Stage 1:
reliable memory

Stage 2:
self-evaluation

Stage 3:
prediction tracking

Stage 4:
mistake learning

Stage 5:
policy experimentation

Stage 6:
skill acquisition

Stage 7:
self-directed research

Stage 8:
code/data co-evolution

Stage 9:
architectural evolution
```

Each stage can unlock additional mutation privileges.

---

# 68. Continuous Integration of New Capabilities

The self-development pipeline should resemble software CI/CD:

```text
new capability proposal
        ↓
schema validation
        ↓
dependency validation
        ↓
unit tests
        ↓
behavior tests
        ↓
historical replay
        ↓
adversarial tests
        ↓
benchmark
        ↓
resource evaluation
        ↓
shadow deployment
        ↓
promotion
```

The system should automatically integrate successful low-risk changes while requiring stronger approval for changes affecting identity, permissions, external actions, or core invariants.

---

# 69. Automatic Discovery of New Cognitive Techniques

When repeated problems cannot be solved adequately by the current library:

```text
problem class
→ retrieve similar problems
→ inspect successful human/agent solutions
→ identify recurring transformation
→ abstract transformation
→ formulate candidate technique
→ test on historical cases
→ test on novel cases
→ install if beneficial
```

This creates an **experience compiler**:

```text
experience
→ interpretation
→ pattern
→ lesson
→ technique
→ policy
→ capability
```

This is one of the most important ways the being can become genuinely more sophisticated without retraining the foundation model.

---

# 70. Code Improvement by the LLM

The LLM should be allowed to inspect:

```text
source code
tests
logs
traces
performance data
failure reports
architecture
dependency graph
```

It should generate candidate patches.

But it should not directly declare the patch successful.

Deterministic tools perform:

```text
compile
type check
unit tests
integration tests
static analysis
security analysis
benchmark
regression suite
```

A Jev-style model can judge bounded questions such as:

```text
does this change appear risky?
does it likely preserve API compatibility?
does it appear to address the reported failure?
should this patch be escalated?
which candidate patch is most promising?
```

The actual build system remains authoritative.

Research topics: **AI software engineering, program synthesis, automated program repair, SWE-bench, code agents, regression testing, static analysis, fuzzing, formal verification**.

---

# 71. Data Improvement by the LLM

The LLM can propose:

```text
fact corrections
entity merges
ontology additions
schema improvements
memory consolidation
policy generalization
new relations
```

But database and graph constraints must validate them.

For important facts:

```text
propose
→ provenance
→ confidence
→ contradiction check
→ source verification
→ commit
```

This is essentially **self-maintaining knowledge engineering**.

Research topics: **knowledge graph completion, ontology learning, entity resolution, knowledge graph repair, data cleaning, truth discovery, provenance**.

---

# 72. Simultaneous Code/Data Improvement

A self-improvement task should be allowed to produce a complete change set:

```text
ChangeSet
├── code patches
├── schema migrations
├── data migrations
├── memory transformations
├── policy changes
├── prompt changes
├── new tests
├── new benchmarks
└── rollback procedure
```

The system evaluates the whole change set rather than each component independently.

This matters because many cognitive improvements require changing both representation and computation.

---

# 73. Backend Routing

The system should itself learn which backend is appropriate.

For every cognitive operation:

```text
operation
→ candidate backend set
→ Jev routing decision
→ execute
→ observe outcome
→ update router
```

Candidate backends might be:

```text
deterministic function
symbolic solver
small ML model
local Jev-style model
hosted Jev
small LLM
large LLM
external tool
human
```

Jev is particularly valuable here because model/tool routing is exactly the kind of bounded decision it is designed to make. TypeSafe itself describes model and tool routing as a primary System One use case.

---

# 74. LLM Routing

The router should estimate:

```text
task complexity
reasoning depth
novelty
required world knowledge
required precision
multimodality
stakes
latency budget
cost budget
```

Then choose:

```text
deterministic
small model
medium model
frontier model
human
```

This should be a Jev-style decision whenever the candidate set is bounded.

The LLM should be used to create or update the routing state, not to make every routing decision.

---

# 75. Reasoning Depth Routing

The same mechanism should decide whether a problem requires:

```text
direct response
basic sanity check
comparison
verification
multi-perspective reasoning
research
simulation
formal reasoning
external experiment
```

This creates adaptive test-time computation.

Research topics: **adaptive computation, test-time scaling, inference-time compute allocation, rational metareasoning, model routing, mixture-of-experts routing**.

---

# 76. Multiple LLM Passes

Multiple LLM passes should not be fixed.

Possible roles include:

```text
interpreter
planner
critic
analogist
researcher
simulator
synthesizer
explainer
```

But these are temporary cognitive roles, not necessarily separate agents.

The metacognitive controller should instantiate only the passes needed.

For difficult reasoning, Tree-of-Thoughts-style search, self-consistency, self-verification, or iterative refinement can be used.

---

# 77. Self-Criticism

Self-criticism should be used carefully.

The LLM should not merely ask:

> "Is my answer correct?"

Instead:

```text
What assumption could invalidate this?
What evidence contradicts this?
What alternative explanation fits?
What comparison is invalid?
What did I fail to consider?
What would an expert object to?
What would change my mind?
```

The result becomes candidate objections.

Jev can determine which objections are decision-material.

This is more useful than indiscriminate self-critique.

---

# 78. Devil's Advocate and Red Team

For high-stakes decisions, instantiate a temporary adversarial reasoning pass.

The red team should attempt to:

```text
break the plan
find hidden assumptions
find edge cases
find adversarial conditions
find social failure
find operational failure
find second-order consequences
```

Research on anticipatory reflection for agents supports explicitly considering potential failures before action and evaluating outcomes afterward.

---

# 79. Verification Strategy

Verification should be treated as an optimization problem.

For every uncertainty:

```text
error cost
× probability wrong
× decision dependence
```

is compared with:

```text
verification cost
```

If verification is cheap and consequential, verify.

If verification is expensive and irrelevant, don't.

This is a fundamental principle of efficient metacognition.

---

# 80. Formal Invariants

Some properties should never depend on an LLM judgment.

Examples:

```text
permission checks
authentication
authorization
transaction integrity
database constraints
version monotonicity
audit-log append semantics
resource limits
spending limits
schema validity
cryptographic verification
rollback correctness
```

These are deterministic.

A Jev result can recommend or gate, but deterministic policy enforcement remains authoritative.

---

# 81. Human Escalation

The being should know when not to decide.

Escalation should occur when:

```text
uncertainty high
stakes high
irreversible
conflicting evidence
missing authorization
policy ambiguity
out-of-distribution
low model calibration
novel action
```

Jev can determine whether escalation is warranted.

The final escalation mechanism should be deterministic.

---

# 82. Self-Assessment and Capability Map

The system should maintain:

```text
capability
confidence
domain
evidence
failure rate
calibration
cost
latency
known limitations
```

It should know:

```text
what it is good at
what it is bad at
what it has not tested
```

This is more useful than a generic "confidence" number.

---

# 83. Prediction of Its Own Behavior

The being should sometimes predict:

```text
what answer will I give?
what tool will I choose?
how long will this take?
will I need clarification?
will this plan succeed?
```

Then compare prediction against actual behavior.

This enables self-calibration.

Research topics: **metacognitive calibration, self-prediction, introspective models, uncertainty estimation, behavioral prediction**.

---

# 84. Cognitive Error Taxonomy

Maintain a persistent error taxonomy:

```text
knowledge_error
memory_error
retrieval_error
interpretation_error
comparison_error
assumption_error
causal_error
planning_error
decision_error
execution_error
verification_error
social_error
resource_error
policy_error
code_error
data_error
model_error
```

Every failure should be classified into one or more categories.

This allows the self-engineering system to identify recurring classes.

---

# 85. Architecture Debt

The being should also recognize that its own architecture can become worse.

Track:

```text
complexity
unused capabilities
duplicate policies
conflicting policies
stale memories
unused schemas
expensive workflows
redundant prompts
obsolete techniques
```

Periodically perform architectural housekeeping.

This is analogous to software technical debt.

Research topics: **technical debt, architecture erosion, software modularity, program simplification, evolutionary architecture, garbage collection, knowledge base maintenance**.

---

# 86. Cognitive Garbage Collection

The cognitive system should periodically remove:

```text
obsolete assumptions
stale temporary plans
duplicate memories
failed candidate policies
unused techniques
superseded frameworks
obsolete capabilities
```

But historical records should remain immutable in the developmental ledger when necessary.

Thus:

```text
active cognitive state
    ≠
historical archive
```

---

# 87. Data Model: RDF/OWL Layer

A semantic representation could define classes such as:

```text
:Being
:Identity
:User
:Relationship
:Goal
:Commitment
:Memory
:EpisodicMemory
:SemanticMemory
:ProceduralMemory
:Belief
:Claim
:Evidence
:Observation
:Assumption
:Inference
:Prediction
:Decision
:Action
:Outcome
:Policy
:Technique
:Doctrine
:Principle
:Heuristic
:Pattern
:Case
:AntiPattern
:Capability
:Experiment
:Evaluation
:Failure
:NearMiss
:EvolutionEvent
```

Relations could include:

```text
:dependsOn
:supports
:contradicts
:derivedFrom
:observedIn
:caused
:predicts
:verifiedBy
:similarTo
:analogousTo
:applicableWhen
:inapplicableWhen
:implementedBy
:requires
:improves
:replaces
:testedBy
:triggeredBy
```

SHACL should validate structural constraints.

---

# 88. Rust Runtime Model

The runtime should probably use Rust for the orchestration and authoritative state machine:

```rust
pub struct CognitiveEpisode {
    pub id: EpisodeId,
    pub goal: GoalId,
    pub context: Context,
    pub claims: Vec<Claim>,
    pub assumptions: Vec<Assumption>,
    pub uncertainties: Vec<Uncertainty>,
    pub constraints: Vec<Constraint>,
    pub active_frames: Vec<FrameId>,
    pub active_doctrines: Vec<DoctrineId>,
    pub active_techniques: Vec<TechniqueId>,
    pub candidates: Vec<Candidate>,
    pub evidence: Vec<EvidenceId>,
    pub risks: Vec<Risk>,
    pub decision: Option<Decision>,
    pub budget: CognitiveBudget,
}
```

The LLM should not directly mutate this structure. It proposes mutations through typed operations.

---

# 89. Typed Cognitive Operations

Expose operations such as:

```rust
enum CognitiveOp {
    Recall { query: String },
    Observe { source: SourceId },
    FormHypothesis { proposition: Proposition },
    Compare { candidates: Vec<CandidateId> },
    FindSimilar { case: CaseId },
    CheckAssumption { assumption: PropositionId },
    Verify { claim: PropositionId },
    Invert { goal: GoalId },
    Simulate { scenario: Scenario },
    Critique { candidate: CandidateId },
    Simplify { solution: CandidateId },
    Decide { candidates: Vec<CandidateId> },
    Act { action: ActionId },
    Evaluate { outcome: OutcomeId },
    Learn { lesson: Lesson },
}
```

The LLM can request these operations, but the runtime determines whether they are authorized and which backend executes them.

---

# 90. Cognitive Operation Backend Mapping

A useful default map is:

```text
Recall
    embedding + database + LLM

FindSimilar
    embeddings + graph retrieval + LLM

Compare
    LLM + deterministic normalization + Jev selection

CheckAssumption
    Jev + external verification

Verify
    deterministic tool / external source / symbolic logic

Infer
    LLM
    or symbolic logic if formal

Deduce
    symbolic logic

Calculate
    deterministic math engine

Forecast
    LLM + statistical model + Jev

Risk classification
    Jev

Risk explanation
    LLM

Plan generation
    LLM

Plan selection
    Jev

Constraint satisfaction
    CP-SAT / SMT / deterministic

Tool selection
    Jev

Tool execution
    deterministic code

Code generation
    LLM

Code validation
    compiler / tests / static analysis

Data cleaning
    deterministic + ML + LLM

Data interpretation
    LLM

Memory retention
    Jev + policy

Memory writing
    LLM proposal + deterministic commit

Personality expression
    LLM

Personality policy selection
    Jev

Conversation generation
    LLM

Identity enforcement
    deterministic

Permission enforcement
    deterministic
```

---

# 91. Bootstrapping the Whole System

The system should be built in layers.

Do not attempt to build the final artificial being first.

Start with:

```text
LLM
+
persistent state
+
tool executor
+
basic memory
+
metacognitive prompt
```

Then add:

```text
epistemic status
→ assumption tracking
→ comparison checking
→ sanity layer
→ Jev decision layer
→ cognitive library
→ prediction ledger
→ meta-analysis
→ mistake memory
→ self-testing
→ code/data evolution
```

Each stage should improve the next stage's ability to build the following stage.

---

# 92. Bootstrap Phase 0: Deterministic Kernel

Build:

```text
identity
state storage
event log
versioning
permissions
tool registry
resource accounting
execution engine
test harness
```

No self-modification yet.

Backend: Rust + PostgreSQL + graph store + deterministic code.

---

# 93. Bootstrap Phase 1: LLM Cognitive Substrate

Add:

```text
interpretation
conversation
goal extraction
planning
memory retrieval
tool selection
basic reflection
```

The LLM does most of the semantic work.

Do not prematurely build elaborate symbolic reasoning.

---

# 94. Bootstrap Phase 2: Epistemic Discipline

Add:

```text
claim
evidence
assumption
observation
inference
hypothesis
prediction
verification
```

Implement the distinction between world state and model state.

This should be an early priority because it prevents many later failures.

---

# 95. Bootstrap Phase 3: Metacognitive Controller

Implement the unified scan:

```text
orient
assess
identify uncertainties
identify assumptions
identify comparison requirements
identify risks
identify missing steps
select operations
```

At first the LLM can perform most of this.

Then collect traces.

---

# 96. Bootstrap Phase 4: Jev Integration

Replace repetitive bounded judgments with Jev-style decisions:

```text
should_verify?
risk_level?
which_model?
which_tool?
which_technique?
memory_worthy?
human_review?
comparison_valid?
which_candidate?
```

Measure every decision against outcomes.

This is where the architecture should start realizing significant latency and cost reductions.

---

# 97. Bootstrap Phase 5: Decision Distillation

Collect Jev traces and outcomes.

Train smaller local decision models for high-volume stable decisions.

Potential path:

```text
Jev
→ labeled decision traces
→ local Jev-style model
→ calibration
→ shadow comparison
→ promotion
```

The LLM can also be used as a teacher where appropriate.

The LLM2Jev research direction is particularly relevant here because it demonstrates extracting bounded decisions from LLM probabilities and suggests that some decision workloads can be handled by much smaller models.

---

# 98. Bootstrap Phase 6: Cognitive Library

Populate the library from:

```text
existing research
LLM knowledge
user conversations
successful cases
failed cases
domain manuals
engineering practice
decision theory
philosophical traditions
```

Do not simply ingest enormous text collections.

Extract structured:

```text
principle
technique
condition
contraindication
example
counterexample
evidence
```

The LLM performs extraction.

Symbolic validation checks schema.

Jev determines applicability.

---

# 99. Bootstrap Phase 7: Meta-Analysis

Start logging:

```text
prediction
decision
reasoning strategy
techniques
outcome
error
resource usage
user correction
```

Then periodically analyze the logs.

Look for:

```text
repeated failures
underused capabilities
overused techniques
bad assumptions
bad comparisons
unnecessary LLM calls
missed verification
successful recurring patterns
```

---

# 100. Bootstrap Phase 8: Improvement

Turn discoveries into experiments.

```text
problem
→ hypothesis
→ intervention
→ benchmark
→ regression test
→ shadow deployment
→ evaluation
→ promotion
```

Initially only allow:

```text
memory policy
retrieval policy
routing policy
prompt templates
low-risk techniques
```

to self-modify automatically.

---

# 101. Bootstrap Phase 9: Code/Data Co-Evolution

Allow the system to generate complete change sets:

```text
code
schema
data
policy
tests
benchmarks
migration
rollback
```

The LLM writes the proposed changes.

Deterministic infrastructure builds and tests them.

Jev ranks candidates.

The promotion engine makes the final bounded decision according to explicit policy.

---

# 102. Bootstrap Phase 10: Architectural Evolution

Only after the system has reliable evaluation should it be allowed to modify:

```text
cognitive architecture
memory architecture
backend selection
new cognitive primitives
new schemas
new self-management mechanisms
```

Architecture evolution should be treated as an experiment.

Never:

```text
"I think this architecture is better."
```

Instead:

```text
candidate architecture
→ historical replay
→ benchmark
→ adversarial evaluation
→ shadow deployment
→ resource comparison
→ regression evaluation
→ promotion
```

---

# 103. Continuous Automatic Integration

The mature system should run a development loop continuously:

```text
                 EXPERIENCE
                     |
                     v
                EVENT LOG
                     |
                     v
               META-ANALYSIS
                     |
          +----------+----------+
          |          |          |
          v          v          v
      knowledge   capability   policy
        gap          gap         gap
          |          |          |
          +----------+----------+
                     |
                     v
              CANDIDATE CHANGE
                     |
                     v
              SELF-ENGINEERING
                     |
                     v
             TEST / BENCHMARK
                     |
                     v
             SHADOW EVALUATION
                     |
                     v
              PROMOTION GATE
                     |
          +----------+----------+
          |                     |
        reject                 accept
          |                     |
          v                     v
       archive             new version
                                |
                                v
                         new behavior
```

This is the core self-bootstrap loop.

---

# 104. The Developmental Control Plane

The whole self-improvement subsystem should itself be controlled by the metacognitive controller.

It should decide:

```textwhat to improve
how much to spend
whether to modify data
whether to modify code
whether to modify policy
whether to investigate further
whether evidence is sufficient
whether the improvement is worth its complexity
```

Jev is useful for bounded candidate selection.

The LLM is useful for discovering what could be changed.

Deterministic systems decide whether the resulting artifact passes tests.

---

# 105. The Being Should Learn Its Own Architecture

The self-model should eventually contain an architectural map:

```text
I use:
    LLM X for semantic reasoning
    Jev Y for routing
    symbolic engine Z for logic
    Postgres for authoritative state
    graph store for semantic relations
    embedding model for retrieval
    Rust runtime for execution
```

It should know:

```text
what each component does
when it is used
how reliable it is
what it costs
what its failure modes are
```

This allows it to reason about its own cognition.

---

# 106. Architecture-Aware Metacognition

The controller should eventually be able to say:

```text
"This task does not require a larger LLM.
The bottleneck is missing data."

"This task does not need more reasoning.
The comparison is ill-defined."

"The reasoning is adequate, but the execution interface is unreliable."

"The decision is simple enough for Jev."

"This requires formal verification rather than another LLM pass."

"The system lacks a capability, not information."
```

This is the mature form of metacognition.

---

# 107. Self-Engineering Research Program

The implementation should explicitly investigate:

```text
AI software engineering
automated program repair
program synthesis
self-improving agents
self-evolving agents
continual learning
lifelong learning
meta-learning
reflection
test-time scaling
self-verification
self-correction
knowledge distillation
policy distillation
program induction
evolutionary computation
genetic programming
AutoML
automated architecture search
```

The lifelong-learning literature specifically emphasizes perception, memory, and action as the major components of continuous adaptation, which fits this architecture well.

---

# 108. What Should Not Be Overbuilt

Several tempting subsystems should deliberately remain thin.

Do not build a gigantic symbolic common-sense database.

Do not manually encode every personality trait.

Do not implement an enormous ontology of human concepts before the LLM needs one.

Do not make every thought persistent.

Do not use an LLM to calculate arithmetic.

Do not use Jev to write explanations.

Do not use symbolic logic to perform broad natural-language interpretation.

Do not build a dozen independent critic models when one metacognitive scan can identify multiple issues.

Do not require deterministic planning for every task.

Do not let self-improvement modify production without tests.

Do not allow model confidence to override authoritative external state.

The system should remain **thin around the LLM**.

---

# 109. The Most Important Architectural Invariant

The entire architecture can be summarized by one rule:

> **The LLM proposes cognition; the control plane structures it; Jev makes bounded judgments; symbolic systems prove what can be proven; deterministic systems enforce what must be enforced; the environment decides what is actually true; memory preserves what matters; and meta-analysis determines how the whole system should improve.**

This prevents any one component from being asked to do everything.

---

# 110. Final System

The complete artificial being is therefore:

```text
                         +-------------------------+
                         |       IDENTITY          |
                         | invariants / continuity |
                         +------------+------------+
                                      |
       +------------------------------+------------------------------+
       |                              |                              |
       v                              v                              v
 USER MODEL                      WORLD MODEL                      MEMORY
       |                              |                              |
       +------------------------------+------------------------------+
                                      |
                                      v
                         +-------------------------+
                         | METACOGNITIVE EXECUTIVE |
                         +------------+------------+
                                      |
             +------------------------+------------------------+
             |                        |                        |
             v                        v                        v
       JE V-STYLE                SYMBOLIC                  DETERMINISTIC
       DECISIONING               REASONING                  CONTROL
             |                        |                        |
             +------------------------+------------------------+
                                      |
                                      v
                         +-------------------------+
                         |    COGNITIVE LIBRARY    |
                         |                         |
                         | frames                  |
                         | philosophies            |
                         | doctrines               |
                         | principles              |
                         | heuristics              |
                         | techniques              |
                         | cases                   |
                         | patterns                |
                         | anti-patterns           |
                         | skills                  |
                         | policies                |
                         +------------+------------+
                                      |
                                      v
                         +-------------------------+
                         |   COGNITIVE PROGRAM     |
                         +------------+------------+
                                      |
                                      v
                         +-------------------------+
                         |          LLM            |
                         |                         |
                         | world knowledge         |
                         | language                |
                         | concepts                |
                         | analogy                 |
                         | imagination             |
                         | social reasoning        |
                         | hypothesis generation   |
                         | synthesis               |
                         | planning                |
                         +------------+------------+
                                      |
                                      v
                         +-------------------------+
                         |    SANITY / EPISTEMIC    |
                         |        FIREWALL          |
                         |                         |
                         | assumptions             |
                         | comparisons             |
                         | contradictions          |
                         | feasibility             |
                         | missing steps           |
                         | risk                    |
                         | alternatives            |
                         | reversibility           |
                         | evidence                |
                         +------------+------------+
                                      |
                                      v
                             DECISION / PLAN
                                      |
                                      v
                             TOOL / ACTION
                                      |
                                      v
                          EXTERNAL OBSERVATION
                                      |
                                      v
                                  OUTCOME
                                      |
                +---------------------+---------------------+
                |                                           |
                v                                           v
          PREDICTION LEDGER                            EVENT HISTORY
                |                                           |
                +---------------------+---------------------+
                                      |
                                      v
                              META-ANALYSIS
                                      |
                +---------------------+---------------------+
                |                     |                     |
                v                     v                     v
             LEARNING             IMPROVEMENT           EVOLUTION
                |                     |                     |
                +---------------------+---------------------+
                                      |
                                      v
                           SELF-ENGINEERING
                                      |
                +---------------------+---------------------+
                |                     |                     |
                v                     v                     v
              CODE                   DATA                 POLICY
                |                     |                     |
                +---------------------+---------------------+
                                      |
                                      v
                              TEST / EVALUATE
                                      |
                                      v
                           SHADOW / PROMOTE
                                      |
                                      v
                             NEW SELF VERSION
```

The deepest architectural insight is that the system does **not** need a giant predesigned "mind."

It needs a **small persistent being substrate plus a powerful general cognitive substrate plus an adaptive metacognitive executive**.

The LLM supplies most of the semantic intelligence. The cognitive library gives it a repertoire of deliberately selectable ways of thinking. The metacognitive executive decides which cognitive operations are worth spending resources on. Jev-style models make the enormous number of small bounded judgments cheaply and directly. Symbolic systems handle formal reasoning. Deterministic infrastructure maintains reality, permissions, state, transactions, tests, and invariants. Memory turns experience into continuity. Meta-analysis turns experience into learning. Self-engineering turns learning into changes in both data and code.

The resulting development equation is:

```text
Artificial Being
=
Persistent Identity
+
Memory
+
World/User Models
+
Goals/Relationships
+
Cognitive Library
+
Metacognitive Executive
+
LLM Cognitive Substrate
+
Decision Models
+
Formal Reasoning
+
Deterministic Execution
+
External Environment
+
Learning
+
Self-Engineering
```

And the developmental equation is:

```text
Experience
→ Observation
→ Interpretation
→ Epistemic Separation
→ Metacognitive Scan
→ Cognitive Program
→ Reasoning
→ Decision
→ Action
→ Verification
→ Outcome
→ Prediction Error
→ Meta-Analysis
→ Lesson
→ Test
→ Policy / Data / Code Change
→ Evaluation
→ Promotion
→ New Capability
→ New Experience
```

That is the architecture I would aim for rather than a conventional "agent framework."

It is essentially a **self-maintaining cognitive operating system** around a general-purpose LLM.

## Research program

The major research areas I would maintain as an explicit implementation bibliography are:

**Jev / decision models:** System One models, RLCD, calibrated decision models, selective prediction, structured decision interfaces, decision-model routing, LLM2Jev, decision distillation. TypeSafe's current primary documentation describes Jev as its first System One model, trained around fast structured decisions and RLCD rather than conversational generation. Independent work has already begun exploring open RLCD implementations and extracting Jev-style decisions from ordinary LLMs.

**Metacognition:** metacognitive control, rational metareasoning, active inference, value of information, adaptive computation, self-verification, self-correction, reflection, test-time scaling, verifier-based reasoning. Recent self-verification work suggests that iterative verification and correction can be an effective use of additional inference computation.

**Agent planning:** hierarchical planning, HTN planning, ReAct, Tree of Thoughts, Graph of Thoughts, model predictive control, contingency planning, tool-use agents, reflective agents. Surveys of LLM-agent planning explicitly identify task decomposition, plan selection, external modules, reflection, and memory as major dimensions.

**Memory and lifelong learning:** episodic memory, semantic memory, procedural memory, memory consolidation, retrieval-augmented generation, Generative Agents, Reflexion, lifelong learning, continual learning, forgetting curves, experience replay.

**World modeling:** knowledge graphs, RDF/OWL, SHACL, temporal knowledge graphs, graph reasoning, ontology learning, neuro-symbolic AI, knowledge graph completion, provenance, truth maintenance.

**Reasoning:** causal inference, Bayesian inference, probabilistic graphical models, counterfactual reasoning, analogical reasoning, case-based reasoning, reference-class forecasting, argumentation, belief revision, paraconsistent logic.

**Decision theory:** expected utility, minimax regret, robust decision making, real options, prospect theory, risk-sensitive optimization, tail-risk analysis, value of information, rational metareasoning.

**Self-engineering:** automated program repair, program synthesis, code agents, AI software engineering, regression-guided repair, evolutionary computation, genetic programming, meta-learning, AutoML, continual self-improvement.

**Evaluation:** calibration, Brier score, log loss, ECE, selective risk, conformal prediction, benchmark design, OOD evaluation, adversarial evaluation, historical replay, shadow deployment, regression testing.

The most important implementation strategy is to **build the control interfaces first and let capabilities accumulate through them**. If every cognitive component exposes a common typed interface—state in, candidate operations, evidence, decision, outcome, evaluation—then the being can progressively replace LLM calls with cheaper Jev-style models, symbolic solvers, specialized ML, or deterministic algorithms as it learns where each is sufficient.

That gives the architecture a natural evolutionary direction:

```text
LLM initially does everything.
        ↓
Observe repeated bounded decisions.
        ↓
Move those decisions to Jev.
        ↓
Observe repeated Jev decisions.
        ↓
Distill high-volume decisions to local small models.
        ↓
Discover formally solvable subsets.
        ↓
Move them to deterministic/symbolic systems.
        ↓
Keep the LLM for the irreducibly semantic remainder.
```

That is probably the most important efficiency principle for the entire design: **do not replace the LLM prematurely, but continuously identify pieces of cognition that have become sufficiently understood to move into cheaper, more reliable substrates.**

The artificial being therefore grows not only by accumulating knowledge, but by progressively **compiling cognition**—from open-ended LLM reasoning into reusable techniques, from techniques into policies, from policies into bounded Jev decisions, and from bounded decisions into deterministic algorithms wherever the problem becomes sufficiently understood.

