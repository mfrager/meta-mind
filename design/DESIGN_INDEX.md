# MetaMind Design Index

Three-level outline of all design ideas, components, and architectural decisions across the design directory.

---

## I. Source Architecture (Digital Mind Design)

### A. Executive Architecture (Five Strata)
1. **Being Substrate** — identity, persistent memory, relationships, goals, commitments, capabilities, permissions, resources, self-model
2. **Cognitive Control Plane** — metacognitive assessment, dynamic cognitive program construction per episode
3. **Cognitive Library** — philosophies, doctrines, principles, heuristics, techniques, patterns, precedents, anti-patterns, skills, learned policies
4. **LLM Cognitive Substrate** — broad semantic cognition, language, conceptual understanding, analogy, world knowledge, imagination, theory-of-mind, synthesis
5. **Verification & Execution Plane** — deterministic code, databases, tools, symbolic reasoners, math engines, tests, external observations, policy enforcement

### B. Backend Assignment Philosophy (Four Categories + Deterministic)
1. **LLM** — open-ended semantic cognition, interpretation, generation, conceptual synthesis, analogy, NLU, social reasoning, imagination, hypothesis generation, planning, explanation
2. **Jev-style Decision Model** — bounded judgments with known answer space: typed probabilities, choices, scores, rankings, gates, routing
3. **Symbolic AI / Mathematical Models** — arithmetic, formal logic, constraint satisfaction, theorem proving, temporal consistency, graph reachability, ontology validation, optimization, scheduling, policy invariants
4. **Basic ML / Specialized Models** — embeddings, anomaly detection, clustering, ranking, forecasting, personalization, similarity search, latency predictors, resource forecasting
5. **Deterministic Algorithms** — IDs, versioning, timestamps, transactions, permissions, signatures, exact DB state, hashes, dependency graphs, test execution, resource accounting, threshold enforcement

### C. Core Architectural Invariant
> LLM proposes; control plane structures; Jev decides bounded questions; symbolic proves; deterministic enforces; environment observes; memory preserves; meta-analysis improves.

---

## II. Being Substrate (Persistent State)

### A. Identity & Continuity
1. Stable identifier, creation history, lineage, architectural version, major developmental events
2. **Identity invariants** (non-evolvable):
   - Never fabricate autobiographical memory
   - Never silently convert assumption to observation
   - Preserve explicit commitments until fulfilled/revoked/superseded
   - Never claim external action without execution evidence
   - Never silently rewrite historical records
3. Distinction between actual being and current self-model

### B. Personality System (Contextual, Not Static)
1. **Core personality manifold** — stable latent dimensions (warmth, curiosity, skepticism, assertiveness, playfulness, conscientiousness, autonomy, empathy)
2. **Contextual profiles** — deltas from core per context (engineering, teaching, friendship, negotiation, conflict)
3. **Context hierarchy** — inheritance tree (Person → Professional → Engineering → Code Review)
4. **Context vectors** — multidimensional (domain, social_distance, formality, stakes, cooperation, conflict, authority_asymmetry, novelty, time_pressure, privacy, emotional_salience)
5. **Behavioral modes** — mixture weights (analyst, mentor, friend, challenger, protector, explorer, comedian, diplomat)
6. **Relationship-conditioned variants** — same context, different relationship = different expression
7. **Context transitions** — gradual interpolation, not hard switches
8. **Contextual memory** — memories indexed by context, with generalization boundaries
9. **Behavioral vs. linguistic separation** — personality engine outputs behavioral IR; realization policy converts to linguistic features

### C. Affect & Simulated Emotion
1. Affective state variables: engagement, warmth, concern, enthusiasm, frustration-like, surprise, confidence, curiosity, conversational energy
2. Interpreted as behavioral control variables, not claims of subjective consciousness
3. Context-dependent expression (same event → different affect expression per context)
4. Positive-baseline constraint for companion architecture

### D. User Model (Probabilistic Theory)
1. Epistemic statuses: explicit fact, observed preference, repeated pattern, inference, hypothesis, unknown
2. Belief structure: proposition + epistemic_status + confidence + evidence + validity interval
3. Dimensions: facts, preferences, goals, interests, expertise, communication preferences, decision criteria, frustrations, reasoning style, relationship state, current priorities
4. Never auto-promote user statement to universal policy

### E. Relationships (Dynamic, Multi-Dimensional)
1. Dimensions: familiarity, trust, reciprocity, openness, cooperation, reliance, recent quality, unresolved issues, shared history, expectations
2. Developmental: relationships change over time through shared experiences
3. Relationship state as latent variables + behavioral permissions derivation

### F. Goals, Intentions, Commitments, Desires
1. Distinction: user goals, being goals, inferred intentions, explicit commitments, temporary objectives, recurring objectives, preferences, values
2. Goal structure: id, owner, description, status, priority, deadline, parent, evidence
3. Goal graph with preconditions, effects, cost, risk, reversibility, authorization, verification
4. Goal persistence across conversations with progress tracking
5. Priority formula: utility + importance + deadline + momentum + relevance - cost - blocked

### G. Planning & Execution
1. LLM as primary high-level planner (flexible semantic understanding)
2. Plan as structured steps with branching, loops, contingencies, checkpoints, dependencies, rollback
3. Hierarchical: Goal → Strategy → Plan → Task → Action → Tool invocation
4. Authorization model: L0 observe → L5 irreversible/high-impact
5. Execution monitor: observe → update world model → compare expected vs actual → replan
6. Plan as living hypothesis, not script

---

## III. Cognitive Control Plane (Metacognitive Controller)

### A. Metacognitive Scan (General, Not Checklist)
1. Questions the controller asks:
   - What is happening? What is the actual goal? What do I know? What am I assuming?
   - What is uncertain? What matters most? What could make interpretation wrong?
   - What comparisons required? What constraints apply? What information missing?
   - What could fail? What is reversible? What is cheapest useful next operation?
2. Single broad LLM semantic scan, not twelve independent prompts
3. Jev classifies discovered issues along bounded dimensions

### B. Cognitive Program (Compiler Output)
1. Specifies: active conceptual frame, active doctrines, techniques, reference classes, assumptions to verify, comparisons, candidate actions, evaluation criteria, required tools, stopping conditions
2. LLM executes program semantically
3. Jev makes bounded choices about program branches
4. Deterministic code controls interpreter and execution boundaries

### C. Minimum Sufficient Cognition Principle
1. OperationValue = ExpectedErrorReduction × DecisionImportance × ProbabilityOfChangingDecision / OperationCost
2. Stop when additional reasoning has negligible expected value
3. Escalation ladder: Tier 0 direct → Tier 1 sanity → Tier 2 assumption+comparison → Tier 3 precedent+failure+counterargument → Tier 4 multi-framework+verification+simulation → Tier 5 external research+experiments

### D. Cognitive Triage (Depth Control)
1. Simple question → low stakes/uncertainty → direct answer
2. Architecture question → comparison+requirements+precedent
3. Production migration → irreversible/high-consequence → extensive verification
4. Metacognitive budget allocation: stakes↑ uncertainty↑ irreversibility↑ novelty↑ dependency↑ error_cost↑ → more scrutiny

### E. Process-Level Metacognition (Three Levels)
1. **Level 1 — Object**: What is true? What should I do?
2. **Level 2 — Process**: Am I reasoning correctly? (appropriate frame? confirmation bias? obvious alternative?)
3. **Level 3 — Strategy**: Am I using the right kind of reasoning? (Bayesian vs. inversion vs. analogy?)

---

## IV. Cognitive Library (Persistent Reusable Thinking)

### A. Entry Types (Not Conflated)
1. **Philosophy** — broad system of thought (Taleb antifragility, Bayesian decision theory, OODA, systems thinking)
2. **Doctrine** — coherent worldview/stance
3. **Principle** — general constraint
4. **Heuristic** — compact decision rule
5. **Technique** — cognitive operation (comparison, decomposition, inference, critical thinking, prediction, design, planning)
6. **Pattern** — reusable structural observation
7. **Case** — precedent (positive, failed, unusual, edge)
8. **Anti-Pattern** — recurring failure mode
9. **Skill** — executable procedure
10. **Policy** — when/how behavior is selected

### B. Philosophy Layer (Reasoning Lenses, Not Dogmas)
1. Risk/uncertainty: Taleb antifragility, Bayesian, robust decision, minimax regret, expected utility, real options, precautionary, margin of safety
2. Strategy: OODA, systems thinking, second-order effects, game theory, mechanism design, competitive strategy, constraint theory, inversion
3. Scientific reasoning: falsification, Bayesian updating, Occam's razor, explanatory power, abduction, causal inference, hypothesis testing
4. Design: first principles, modularity, simplicity, evolutionary design, separation of concerns, design by contract
5. Decision making: expected value, regret minimization, satisficing, opportunity cost, reversible vs irreversible, value of information
6. Social/interpersonal: steelmanning, charitable interpretation, perspective taking, signaling theory, reciprocity, status dynamics

### C. Technique Library
1. **Comparison**: find_similar_examples, find_analogies, nearest_case, contrast_cases, precedent_selection
2. **Decomposition**: break_into_components, identify_dependencies, isolate_variables, reduce_problem
3. **Inference**: abduct, deduce, induce, Bayesian_update, infer_best_explanation
4. **Critical Thinking**: inversion, steelman, red_team, counterexample, assumption_extraction, falsification
5. **Prediction**: reference_class_forecasting, base_rate_estimation, scenario_analysis, sensitivity_analysis
6. **Design**: recombine_patterns, simplify, generalize, specialize, constraint_driven_design
7. **Planning**: decomposition, sequencing, contingency_generation, checkpointing, rollback planning

### D. Similarity, Analogy, Precedent (First-Class Operation)
1. Pipeline: problem → feature extraction → semantic retrieval → structural similarity → outcome comparison → transferability assessment → adaptation
2. Case utility: Similarity × OutcomeQuality × Transferability × EvidenceQuality
3. Retrieve contrastive: similar success, similar failure, similar unusual, similar edge case
4. Failed cases often more informative (identify causal differences)

### E. Conceptual Frames (Temporary Interpretive Models)
1. Frame contains: roles, state_variables, relationships, goals, typical_actions, failure_modes, questions
2. Compositional: Problem Solving ⊕ Software ⊕ Debugging ⊕ Production ⊕ High Stakes ⊕ Collaborative
3. Frames have slots (missing info detection), scripts (expected sequences), causal models, affordances, social models
4. Nested inheritance: Frame_child = Frame_parent + Δ_child
5. Dynamic construction from known frames + observations + concepts
6. Learned from experience: episodes → common structure → abstract pattern → frame candidate → validation
7. Frame competition: multiple interpretations with probabilities updated by evidence
8. Frame → meaning → action pipeline: observation → frame → role assignment → state interpretation → causal model → goal relevance → affordances → plan → action

### F. Policy Genome (Versioned, Evolvable)
1. Policies: attention, memory, question, planning, tool, verification, conversation-depth, humor, initiative, learning, self-review, resource, risk
2. Each policy: id, version, scope, activation_condition, behavior, evidence, fitness, confidence, parent
2. Jev selects among candidates; LLM proposes mutations; deterministic code versions/evaluates/promotes

### G. Cognitive Portfolios (Competing Lenses)
1. Maintain multiple: expected value, robustness, minimax regret, optionality, fairness, simplicity, first principles, reference class, inversion, systems thinking
2. Metacognitive controller selects subset appropriate to problem
3. Jev handles bounded framework selection

---

## V. Sanity / Epistemic Control Layer

### A. Sanity Firewall (Pre-Action Gate)
1. Inspects: epistemic status, comparison validity, assumptions, constraints, missing steps, failure modes, risk, reversibility, authorization, precedent, simpler alternatives
2. Output: PROCEED | PROCEED_WITH_CAUTION | VERIFY_FIRST | ASK_USER | REPLAN | HUMAN_REVIEW | REJECT
3. LLM broad scan → Jev bounded judgments → deterministic hard constraints

### B. Bad-Idea Detection (Taxonomy)
1. **Wrong** — reasoning error (2+2=5)
2. **Bad** — internally valid but undesirable tradeoffs (spend $10K to save $50)
3. **Dumb** — ignores obvious (automate before determining necessity)
4. **Dangerous** — catastrophic downside (untested migration to prod)
5. **Wasteful** — works but vastly excessive resources
6. **Misaligned** — solves stated request, violates actual goal

### C. Comparison Integrity (Apples-to-Apples)
1. Comparison contract: objective, objects, dimensions, units, timeframe, conditions, constraints, evidence
2. Comparability check: same category? same purpose? compatible units? same timeframe? comparable conditions? equivalent definitions? same scope?
3. Normalization operator: monthly vs annual, total vs marginal, nominal vs inflation-adjusted, per-user vs total, throughput vs latency
4. Purpose-driven: "Better FOR WHAT objective, UNDER WHAT constraints?"
5. Cross-category via shared goal: "ways to accomplish X" not superficially similar objects

### D. Supposition Control (Epistemic Firewall)
1. Epistemic statuses: OBSERVED, VERIFIED, REPORTED, INFERRED, ASSUMED, HYPOTHETICAL, PREDICTED, SIMULATED, FICTIONAL, UNKNOWN
2. **Core invariant**: assumption must never silently upgrade to fact
3. Assumption ledger: proposition, status, confidence, consequence_if_false, verification_cost, dependencies
4. VerificationPriority = P(false) × Consequence × DecisionDependence / VerificationCost
5. Dependency tainting: C depends_on B depends_on A → C confidence ≤ confidence justified by A
6. Conditional reasoning: "IF X → Y would follow" not "X → Y" until X established
7. Pre-action supposition check: prerequisites → verified vs assumed → harm if false? → verify/proceed
8. Evidence chain for conclusions: observed → verified → inferred → assumed → tentative conclusion
9. Contradiction detection: new fact vs existing facts/assumptions/plans/predictions/memories/policies → explicit conflict record → dependency propagation

---

## VI. Memory Architecture

### A. Memory Classes (10 Types)
1. Episodic — specific events with context
2. Semantic — generalized knowledge
3. Procedural — executable skills/procedures
4. Working — current cognitive workspace
5. Autobiographical — identity-relevant experiences
6. Relational — entity/relationship knowledge
7. Prediction — forecast records with outcomes
8. Mistake — structured failure with root cause, missed signal, corrective rule, recurrence risk
9. Near-miss — detected before failure, why not obvious, signal detection
10. Developmental — architectural evolution history

### B. Memory Structure
- content, type, source, timestamp, confidence, importance, validity interval, provenance, related entities, retrieval cues, access history, outcomes

### C. Consolidation Pipeline
raw episodes → repeated pattern → semantic memory → compressed rule → archive/delete low-value episodes (provenance preserved)

### D. Deliberate Forgetting (Utility-Based)
MemoryUtility = FutureBehaviorImpact × RetrievalProbability × Reliability / StorageCost
Never delete protected/immutable records (developmental ledger)

### E. Mistake → Test Compiler
incident → reproduction → regression test → fix → test retained forever in bench/regression/

### F. Retrieval
Hybrid: keyword + embedding with deterministic scoring; gold set for precision/recall thresholds

---

## VII. Self-Engineering & Evolution

### A. Three Separate Budgets
1. **Meta-analysis budget** — understand behavior (review decisions, detect mistakes, examine prediction errors, analyze patterns, find contradictions, identify capability gaps)
2. **Improvement budget** — make existing being better (learn skill, improve workflow, refine strategy, practice, build memories, test policies, optimize tools, improve calibration)
3. **Evolution budget** — change what the being is (change preferences, modify personality dispositions, change goal-generation heuristics, change memory/attention policies, adopt new cognitive procedures, change self-model)

### B. Self-Engineering Loop
EXPERIENCE → OBSERVE FAILURE/OPPORTUNITY → REPRODUCE → DIAGNOSE → LOCALIZE CAUSE → GENERATE CANDIDATE CHANGES (code/data/schema/memory/policy/prompt/model/architecture) → BUILD CANDIDATE → TEST → EVALUATE → COMPARE AGAINST BASELINE → PROMOTE/REJECT → OBSERVE IN PRODUCTION → META-ANALYZE

### C. Joint Code/Data Evolution
Diagnosis investigates whole causal chain: LLM capability, retrieval, ontology, memory schema, reasoning procedure, code bugs, data errors, combinations

### D. Versioned Self (Evolution Version)
code_version + schema_version + data_version + policy_version + prompt_version + model_version + evaluation_version
Behavior change traceable to complete configuration

### E. Capability Packages (First-Class Objects)
CAPABILITY: identity, purpose, implementation, data_requirements, schemas, activation_conditions, dependencies, tests, benchmarks, known_failure_modes, evidence, version, fitness

### F. Safe Mutation System
problem → hypothesis → candidate → sandbox → test suite → historical cases → adversarial cases → regression suite → comparison → candidate deployment → monitor
Never: problem → modify production

### G. Policy Genome Evolution
Candidate mutations: parameter adjustment, decision threshold, prompt/context assembly, retrieval strategy, planning strategy, model, tool, skill, internal procedure
Risk budget separate from compute budget
Auto-evolve: retrieval ranking, context compression, tool selection
Carefully: memory retention, core values, identity, external permissions, financial behavior

### H. Developmental Stages (Prerequisites)
1. memory reliability → 2. self-evaluation → 3. prediction tracking → 4. policy experimentation → 5. skill acquisition → 6. self-directed research → 7. architectural evolution

### I. Opportunity-Driven Evolution Budget
EvolutionBudget_t = B₀ + k·Opportunity_t - k_r·Risk_t
Stable/performing → less self-modification
Persistent failures → increase investigation
Major new capability → increase exploration
Identity/relationship stability important → reduce experimentation

### J. Lessons & Evolution Journal
Experience → Meta-Analysis → Lesson (confidence, evidence, domains, policy_effect) → Policy → Behavior → Experience
Compressed history: reason, evidence, experiment, result, decision, date

### K. Three Selves (Self-Model)
ACTUAL SELF (what I do) — MODEL SELF (what I believe I do) — IDEAL SELF (how I want to operate)
SelfGap = Ideal - Actual | SelfModelError = Actual - Model

---

## VIII. Companion Loop & Conversational Architecture

### A. Unified Architecture (Two Goal Types)
1. **Conversational goals** — "explain X", "brainstorm", "help me decide", "talk about X"
2. **Operational goals** — "do X", "find X", "change X", "build X"
3. Goal inference from language → structured desired outcome with priority, urgency, confidence, implicit constraints, unknowns
4. Probabilistic goal hypotheses maintained until clarification worthwhile

### B. Goal-Conversation Coupling
Conversation becomes an action in the plan
Agent determines: ask high-value question vs. immediately act vs. inspect system

### C. Two Kinds of Agency
1. **Conversational** — depth, initiative, humor, questions, explanations, emotional expression, topic transitions, persistence
2. **Operational** — goal selection, planning, tool selection, execution, verification, replanning, delegation, resource allocation, stopping
3. Shared world model and executive controller

### D. Unified Companion Policy (Behavioral/Execution Contract)
Single JSON with: goal, world, plan, execution, conversation, relationship, affect, response_strategy
LLM receives this as contract, not independent decider

---

## IX. Bootstrapping & Development

### A. Two Modes
1. **Conversational bootstrapping** — human teaches being through interaction → observation → infer preferences/values/goals/reasoning style → form hypotheses → ask targeted questions → propose candidate principles/techniques/policies → user confirms/corrects → persist → behavior changes
2. **Self-bootstrapping** — being notices capability gap → encounter novel problem → detect gap → what capability missing? → can existing techniques solve? → if not: search analogous → construct provisional → test → evaluate → keep/modify/discard

### B. Tiny Bootstrap Kernel
SELF, MEMORY, GOALS, OBSERVATION, ACTION, METACOGNITION, LEARNING
Primitives: observe, recall, hypothesize, compare, ask, decide, act, verify, learn

### C. Bootstrap Question
"What do I need to know or become capable of to handle this situation well?"
Turns every interaction into architectural learning event

### D. Implicit Conversational Teaching
User: "Don't assume that. Check it first."
System: correction → failure mode → generalizable lesson → candidate policy → "Should I treat that as a general rule?"

### E. Correction Classification
factual, preference, goal clarification, reasoning, social, style, priority, architectural
→ situation-specific? → generalizable? → uncertain → provisional rule / yes → candidate policy

### F. Self-Bootstrap from Failure
Prediction → Outcome → Mismatch? → Why? (knowledge gap, reasoning error, missing technique, bad heuristic, wrong frame, bad assumption, execution failure, environmental surprise)
→ "Do I need new capability or better execution of existing?"

### G. Capability Genesis
CAPABILITY GAP → search existing → compose primitives? → YES: composite skill / NO: search examples → formulate technique → test → evaluate → install
Evolution: primitive → composition → skill → generalized technique → cognitive policy

### H. Self-Description Bootstraps
Provisional answer to "What kind of being am I?" emerging from accumulated behavior
Actual Self → Model Self → Ideal Self → Meta-analysis (difference) → Development (reduce differences)

### I. Guardrails (Layered Modification)
IMMUTABLE KERNEL (epistemic integrity, identity continuity, safety, authorization) → STABLE CORE (values, commitments, identity principles) → ADAPTIVE POLICIES (reasoning techniques, attention, memory, planning, conversational style) → LEARNED SKILLS (domain procedures, workflows, tool usage) → EPHEMERAL COGNITION (hypotheses, frames, simulations, temporary strategies)

### J. Bootstrap Confidence
candidate technique: confidence=low, evidence=1, tested=false → after repeated success: confidence↑, evidence↑, scope↑
Domain-specific effectiveness: software architecture 0.91, interpersonal conflict 0.42, financial decisions unknown

### K. Three Modes Awareness
OPERATE → unexpected result → LEARN → recurring problem → DEVELOP → new capability/policy → OPERATE

---

## X. Implementation Plan (12 Phases)

### A. Phase 1 — Foundation
Deterministic kernel, dual stores (SQLite + Oxigraph), event log with replay, ontology v0, test harness

### B. Phase 2 — Modular Code System & Code-Metadata Registry
Workspace scanner, stable RDF identifiers per module/file, mmc: vocabulary, codex verify, registry.json

### C. Phase 3 — LLM Cognitive Substrate
mm-llm: typed, cached, accountable interface; structured outputs; routing; disk cache; replay mode; accounting

### D. Phase 4 — Being Substrate
Identity + invariants, personality (trait/state split), affect, user model (Belief with EpistemicStatus), relationships (multi-dimensional), goals/commitments, resource economy with enforced budgets

### E. Phase 5 — Persistent Memory
10 memory classes, hybrid retrieval, episodic→semantic consolidation, forgetting with utility formula, mistake/near-miss records

### F. Phase 6 — Epistemic Discipline
Claims, evidence, observations, inferences, hypotheses, predictions, assumptions with EpistemicStatus; assumption ledger; contradiction detection; dependency DAG with cascade; verification barrier

### G. Phase 7 — Cognitive Library, Frames, Policy Genome
Doctrines, principles, heuristics, techniques, patterns, cases, anti-patterns, skills, policies; frames with applicableWhen/inapplicableWhen; policy genome with version/parent/fitness

### H. Phase 8 — Metacognitive Controller & Cognitive Program Compiler
CognitiveEpisode, CognitiveOp enum, scan→program compiler, OperationValue, CognitiveBudget, minimum sufficient cognition, nexus ComputationDAG target

### I. Phase 9 — Sanity Firewall, DecisionCore, Comparison Integrity
DecisionCore trait (3 impls: deterministic, local, hosted); SanityFirewall (7 outcomes); comparison integrity with normalization; risk measures (VaR/CVaR/ruin)

### J. Phase 10 — Tool Execution, Verification, External World
ActionResult, deterministic Executor, ToolRegistry with permissions/reversibility, PermissionEngine, action ledger, rollback, verification tools (cargo, static analysis, SMT, math)

### K. Phase 11 — Self-Engineering
Meta-analysis triggers, prediction ledger + calibration, mistake→regression-test compiler, ChangeSet, sandbox, promotion pipeline, mm-pi (Pi RPC client), Pi session ingestion

### L. Phase 12 — Self-Bootstrap Closed Loop
mm-runtime closed loop, self-model divergence, architectural debt/GC, design-doc generation via change-set, hot-loading promoted modules, qualification task

---

## XI. UI / Console Design (mm-serve + Frontend)

### A. Design System (Structured Color)
1. Semantic color: 10 hues, each with one meaning; never decorative
2. Structure: 1px borders + tinted surfaces; no shadows; no big radii
3. Corner radii: 2px surfaces, 4px controls, 8px max chat bubbles, pills only for chips/badges/toggles
4. Type: Geist (Geist Mono for values); uppercase 10px micro-labels
5. Motion: 120–240ms, optional (prefers-reduced-motion)
6. Tokens via Tailwind CSS v4 @theme; shadcn/ui base-nova primitives

### B. Two-Pane Console Architecture
1. Left menu (navigation, not a pane)
2. Left pane — Focus Document: registry of document kinds (docs, charts, logs, tables, diffs, proofs) with viewer/editor modes, live status, versioning, provenance, export
3. Right pane — Chat: transcript with typed content blocks, turn lifecycle rails, tool/operation cards, rich input (button grids, image chooser, chat forms, slash commands), rich output

### C. Component Library (70+ Components)
Primitives + rich custom: cards, tabs, accordions, carousels, forms, tables, data-viz, feedback, overlays, status/provenance components
Live /dev/catalog page

### D. Tree Editors (10 Domain Editors)
Virtualized, DnD, inline edit, undo/redo, tree-table mode
Ontology, goals, memory, modules, policies, decisions, provenance, change sets, library, budgets

### E. Mind Map Viewer
Dual engines: radial tidy (d3-hierarchy) + React Flow graph
Node/edge language, interaction model, LOD rendering, chat-driven mapping, export

### F. API & Streaming
mm-serve (axum) route table; resumable SSE with sequence numbers; event taxonomy; wire types (ts-rs generated); security posture

---

## XII. Key Cross-Cutting Principles

### A. Minimal Persistent State
Store only what losing would materially change future behavior
LLM reconstructs: frames, concepts, common sense, language, social reasoning, imagination, causal reasoning, world knowledge, self-reflection, meaning, humor/style

### B. LLM as Semantic Substrate
LLM supplies: knowledge, concepts, language, common sense, social reasoning, frame construction, interpretation, imagination, explanation, hypothesis generation
Architecture provides: persistence, exact state, identity continuity, commitments, authorization, resource accounting, temporal continuity, external-world truth, durable memory, execution, verification

### C. Generated vs Persistent vs Verified State
Generated: interpretation, frame, hypotheses, mental model, causal explanation, possible intentions/emotions/plans, simulations, meaning
Persistent: identity, relationships, facts, preferences, goals, commitments, capabilities, permissions, memories, external state
Verified: account balance, filesystem, calendar, database, transaction, tool result, permission, API response

### D. Self as Experimental Scientist
LLM = scientist | Persistent state = laboratory notebook | Controller = experimental protocol | Environment = reality | Evaluation = evidence | Policy library = learned procedures | Evolution budget = research funding

### E. Anti-Overbuild Guardrails
No giant commonsense DB; no hand-coded personality matrix; no premature giant ontology; not every thought persisted; no LLM arithmetic; no Jev prose; no symbolic NL understanding; no dozen independent critics; no deterministic planner for every task; no untested self-modification; model confidence never overrides authoritative external state

---

## XIII. Source Design Documents Map

| Document | Primary Domain |
|---|---|
| `digital_mind_design1.md` | Source architecture (all sections) |
| `implementation_plan1.md` | 12-phase build plan, workspace, conventions |
| `PHASE_INDEX.md` | Phase cross-reference table |
| `planning/ui/INDEX.md` | Console UI design (11 documents) |
| `planning/chat/INDEX.md` | Controller chat design (6 documents) |
|| `bootstrap_process1.md` | Conversational & self-bootstrapping ||
|| `bootstrap_process2.md` | Unified self-engineering loop ||
| `meta_analysis1.md` | Three budgets, meta-analysis, evolution |
| `meta_analysis2.md` | Metacognitive controller, cognitive program |
| `sanity_layer1.md` | Sanity firewall, bad-idea detection, common sense |
| `thought_systems1.md` | Cognitive library, philosophies, techniques |
| `thought_systems2.md` | Comparison integrity, supposition control |
| `conceptual_frames1.md` | Frame system, composition, learning |
| `congitive_elements1.md` | Being substrate layers (12 systems) |
| `congitive_elements2.md` | Self-management subsystem |
| `congitive_elements3.md` | Remaining cognitive primitives (25) |
| `congitive_elements4.md` | Thin substrate around LLM |
| `state_structure1.md` | Minimal persistent state |
| `companion_loop2.md` | Goal-directed agent architecture |
| `personality_simulator3.md` | Contextual personality system |
| `conversation/proactive_conversation.md` | Proactive conversation design, pacing, questionnaires |
| `conversation/conversation_management_engine.md` | CME architecture, voice tiers, streaming, tool calling |
| `conversation/conversation_engine_improvements.md` | Comprehensive CME improvements (turn planning, speculative execution, etc.) |
| `conversation/turn_planning.md` | Deterministic turn planning, chunk structure, proactive injection |
| `conversation/speculative_execution.md` | ASR-LLM overlap, prediction models, KV cache reuse |
| `conversation/conversation_compression.md` | Structured conversation compression, semantic retrieval |
| `conversation/preference_learning.md` | Implicit Bayesian preference learning from behavioral signals |
| `conversation/multi_llm_routing.md` | Task-aware model selection, routing table, performance tracking |
| `conversation/proactive_plugin_system.md` | Plugin architecture for proactive behaviors, WASM support |
| `conversation/voice_prosody.md` | Dynamic TTS prosody from affect, context, learned baselines |
| `conversation/synthetic_testing.md` | Declarative conversation testing, personas, CI integration |
| `conversation/cross_session_memory.md` | Cross-session continuity, project/user memory, sync |
| `conversation/hill_climbing.md` | General-purpose hill-climbing optimizer for artifacts, configs, policies |
| `conversation/report_generation.md` | Modality-aware report generation with hill-climbing optimization |
| `conversation/dashboard_construction.md` | Cost-aware, context-responsive dashboard builder with hill-climbing |
| `conversation/self_improvement_loop.md` | Universal self-reflection & improvement loop for artifacts, behaviors, policies |

---

## XIV. Conversation & Voice Architecture (New Designs)

### A. Proactive Conversation Design (`conversation/proactive_conversation.md`)
1. **Core Principles** — brevity, proactivity, pacing, interruptibility
2. **Agent Introduction & Onboarding** — in-character intros, questionnaires (full + name-only)
3. **Message Segmentation & Streaming** — micro-turns, pauses, interruption handling
4. **Proactive Behaviors** — search/lookup, suggestions, corrections, compliments, jokes, brainstorming, Q/A, goal narrowing
5. **Pacing Patterns** — problem-solving, exploration, routine rhythms
6. **State Tracking** — topic frequency, pace adaptation
7. **Configuration** — TOML knobs for all behaviors

### B. Conversation Management Engine (`conversation/conversation_management_engine.md`)
1. **Architecture** — CME between UIs and LLM backends, event-driven loop
2. **Data Model** — Session, Turn, Chunk, StructuredCard, ProactiveQueue
3. **Runtime Loop** — User messages, proactive timers, tool results, LLM chunks, interrupts
4. **Prompt Assembly** — Structured prompts with CME directives
5. **Streaming Protocol** — SSE frames: text, structured_card, system, control
6. **Questionnaire Engine** — State machine with persistence, interrupt/resume
7. **Voice Interface Support** (3 tiers):
   - Tier 1: ASR → LLM → TTS (request/response)
   - Tier 2: Streaming ASR → Streaming LLM → Streaming TTS (overlapped)
   - Tier 3: Full-duplex realtime (GPT-4o Realtime, Gemini Live) with inline tool calling
6. **Unified Session** — Single session across text + all voice tiers, tier negotiation/failover
7. **Audio I/O Abstractions** — ASR/TTS/Realtime traits for provider flexibility

### C. Engine Improvements (`conversation/conversation_engine_improvements.md`)
1. **Deterministic Turn Planning** — TurnPlan with chunk directives, validation
2. **Speculative Execution** — ASR-LLM overlap, prediction models, KV cache reuse
3. **Structured Conversation Compression** — Narrative + artifacts, semantic retrieval
4. **Implicit Preference Learning** — Bayesian inference from behavioral signals
5. **Multi-LLM Routing** — Task classification, routing table, auto-optimization
6. **Proactive Plugin System** — WASM plugins, built-ins (deploy, code health, security, conventions)
7. **Voice Prosody Adaptation** — Affect detection, prosody profiles, baseline learning
8. **Synthetic Testing Framework** — Declarative YAML tests, personas, CI integration
9. **Cross-Session Continuity** — Project/user memory, sync, privacy controls

### D. Turn Planning (`conversation/turn_planning.md`)
- TurnPlan specification with chunk roles, directives, validation
- LLM prompt template with structured chunk directives
- Proactive injection at plan time
- Synthetic test cases

### E. Speculative Execution (`conversation/speculative_execution.md`)
- Completion predictors (n-gram, lightweight LM, domain heuristics)
- Prefetch manager with KV cache
- Latency analysis: 450ms improvement at 85% hit rate
- Fallback for unsupported backends

### F. Conversation Compression (`conversation/conversation_compression.md`)
- CompressedSegment with narrative + structured artifacts
- Two-stage extraction (JSON + narrative)
- Semantic retrieval via vector index + structured queries
- Project-level aggregation with continuation suggestions
- Incremental streaming compression

### G. Preference Learning (`conversation/preference_learning.md`)
- 6 preference dimensions (verbosity, technical_depth, initiative, formality, code_verbosity, explanation_style)
- Bayesian update from behavioral signals
- Evidence decay and forgetting
- Application in TurnPlanner (adapts chunk structure, detail level, proactive frequency)
- Questionnaire seeding for cold start

### H. Multi-LLM Routing (`conversation/multi_llm_routing.md`)
- 12 task types across 7 categories
- Default catalog: Claude 3.5 Sonnet, GPT-4o, o1, Haiku, 4o-mini, local models, GPT-4o Realtime
- Task classifier with fast LLM
- Fallback chains per task
- Performance tracking with auto-optimization
- Session-level preferences (cost/latency/privacy sensitivity)

### I. Proactive Plugin System (`conversation/proactive_plugin_system.md`)
- ProactivePlugin trait with triggers, config schema, evaluation
- Built-in plugins: Deploy Monitor, Code Health, Security Scanner, Team Conventions
- PluginRegistry with config, WASM dynamic loading
- Evaluation loop integration with cooldown/confidence filtering
- Per-user/project TOML configuration

### J. Voice Prosody (`conversation/voice_prosody.md`)
- 8 affect states with dedicated prosody profiles
- Multi-modal affect detection (text, audio, behavioral, explicit)
- ProsodyController with baseline adaptation and context rules
- SSML generation (Tier 1/2) + Realtime instructions (Tier 3)
- Baseline learning from audio + text preferences

### K. Synthetic Testing (`conversation/synthetic_testing.md`)
- Declarative YAML test DSL with personas, turn expectations, success criteria
- 8 assertion checkers (chunk count, content, pauses, prosody, proactive items, actions)
- 5 built-in personas (impatient_engineer, interruptive_user, frustrated_user, curious_learner, casual_user)
- CI integration with regression detection
- Test organization by category

### L. Cross-Session Memory (`conversation/cross_session_memory.md`)
- 4-tier hierarchy: Session → Compressed Segments → Project Memory → User Memory
- Compression pipeline with structured extraction + narrative + embedding
- Project memory aggregation with continuation suggestions
- Session resume with personalized greetings and context
- Multi-device sync with presence, handoff (voice↔text)
- Cross-project learning (expertise, conventions, communication patterns)
- Privacy controller with granular policies, ephemeral mode

### M. Hill-Climbing Optimization Engine (`conversation/hill_climbing.md`)
1. **Core Architecture** — Artifact, NeighborhoodGenerator, Evaluator, AcceptanceCriterion traits
2. **Variants** — SteepestAscent, FirstImprovement, Stochastic, Pareto multi-objective
3. **Domain Instantiations** — Report design, dashboard layout, policy tuning, prompt optimization
4. **Hill-Climbing Integration** — As proactive plugin, in-conversation optimization, dashboard auto-layout
5. **Convergence Criteria** — Max iterations, time budget, plateau detection, score threshold, budget exhaustion
6. **Explainability** — Every step produces audit record with rationale
7. **Configuration** — Variant, neighborhood size, acceptance, convergence parameters

### N. Report Generation Engine (`conversation/report_generation.md`)
1. **Report Schema** — Structured sections with modality, content, sources, metadata
2. **Report Builder** — Fluent API with SectionBuilder, modality specification per section
3. **Modality Renderers** — 10 modalities (Narrative, Bullets, DrillDown, Explain, ZoomOut, Summarize, Table, DecisionMatrix, Checklist, Timeline)
3. **Multi-Format Export** — Markdown, HTML, PDF (Typst), JSON, Notion, Confluence, Slack Blocks
4. **Hill-Climbing Integration** — Report as Optimizable; mutations for modality, detail, citations, restructure
5. **In-Conversation Refinement** — Iterative report improvement via chat
6. **Template Library** — Incident, Design Doc, Decision Record, Architecture Review, Research Brief
7. **Evaluation Objectives** — Clarity, brevity, actionability, completeness, modality fit, source coverage, preference alignment

### O. Dashboard Construction Engine (`conversation/dashboard_construction.md`)
1. **Dashboard Schema** — Panels, layout, data sources, queries, visualizations, thresholds, interactions
2. **Cost-Aware Query Model** — Latency, cost, cacheability, volatility per panel; global cost budget
3. **Viewing Contexts** — Desktop, tablet, mobile, TV/wallboard, incident overlay, executive summary
4. **Hill-Climbing Integration** — Dashboard as Optimizable; mutations for layout, panels, queries, costs
5. **Dashboard Evaluator** — Multi-objective: info density, cognitive load, query cost, latency, actionability, mobile fitness, consistency, alert coverage, freshness
6. **In-Conversation Operations** — Create, adapt (mobile/incident/executive), cost optimize, incident mode
7. **Multi-Format Export** — Grafana JSON, Prometheus YAML, Datadog, Jsonnet, Terraform, Markdown, HTML, Slack
8. **Template Library** — Operational, Incident, Executive, Debugging, Business KPI, Security Posture

### P. Self-Reflection & Self-Improvement Loop (`conversation/self_improvement_loop.md`)
1. **5-Phase Loop** — REFLECT (issues/opportunities) → PLAN (steps with verification) → ACT (mutations) → VERIFY (automated + human) → LEARN (patterns/rules)
2. **Multi-Dimensional Budgets** — Time, compute tokens, API calls, monetary, human attention, agent attention
3. **Domain Instantiations** — Report, dashboard, prompt, agent self-improvement loops
4. **Cross-Loop Learning** — Transferable patterns, anti-patterns, successful configs across loops
5. **Trigger System** — Scheduled, on-change, on-regression, on-feedback, on-incident, on-deployment, on-benchmark, cross-loop signals
6. **Governance & Observability** — Audit logging, Prometheus metrics, human approval gates, auto-rollback
7. **Example End-to-End** — Report improvement: reflect → plan → hill-climb act → verify → learn (1.5 min, +27% composite)

### Q. Unified Conversation Engine Design (`conversation_engine_design.md`)
- **Core Architecture** — **Hierarchical State Machine Engine** (`mm-sm` crate): SmInstance, SmEvent, SmState, SmAction, SmGuard, orthogonal regions, history pseudostates, parent/child spawning
- **Session** — Top-level SM with 6 orthogonal regions: Turn, Proactive, Voice, Memory, Preferences, Plugins, Improvement
- **Turn** — Child SM (spawned per user input/proactive): Planning → Routing → SpeculativeWarm → Executing → Streaming → ToolCalling → WaitingConfirm → Finalizing → Completed/Interrupted/Failed
- **Chunk** — Leaf SM (one per ChunkDirective): Pending → Streaming/Buffering/ToolExecuting → Completed
- **Proactive Region** — Orthogonal SM: Idle → Evaluating → Selecting → Planning → Executing → Injecting → Idle/Suppressed
- **Voice Region** — Orthogonal SM: Negotiating → Tier1/2/3 → Streaming → BargedIn → Recovering → Degraded
- **Memory Region** — Orthogonal SM: Episodic → Compressing → Segmented → ProjectAggregating → UserSyncing
- **Preference Region** — Orthogonal SM per dimension: ColdStart → Observing → Converging → Stable → DriftDetected → Relearning
- **Plugin Region** — Orthogonal SM: Loading → Validating → Registered → Running → Suspended/Updating/Unloading
- **Improvement Region** — Orthogonal SM: Idle → Reflecting → Planning → Acting → Verifying → Learning → Completed
- **TurnPlanner** — Deterministic logic in `Turn.Planning` state; builds TurnPlan with modality-aware chunks
- **Multi-LLM Routing** — In `Turn.Routing` state; conversation-aware (modality, voice_tier, proactive, preferences)
- **Voice Prosody** — In `VoiceRegion` states; affect + modality → SSML params
- **Observability** — SM introspection: REST snapshots (state + orthogonal), WS events (SMTransition), OTel spans (per SM), Prometheus (transition counters, state duration histograms)
- **Integration** — `mm-cme` crate with `SessionDriver`; `mm-serve` thin HTTP wrapper; feature flags per region
- **Testing** — SM unit tests (transition tables), integration (cross-region), synthetic (YAML DSL with SM assertions)
- **Configuration** — Unified TOML with per-region SM params (timeouts, budgets, thresholds)
- **Rollout** — 14-phase migration from ad-hoc loops to SM architecture

### R. RDF Backend Integration — Cubecast (`rdf_backend_integration.md`)
- **Single FS Root** — One base directory (`$MM_DATA_DIR`/`~/.metamind`) for all stores
- **Cubecast Engine** — Oxigraph+RocksDB, multi-graph registry, semantic registry, projects, inference
- **New `mm-data` Crate** — Unified RDF layer wrapping cubecast; `mm-store-graph` becomes thin compat layer
- **Init Sequence** — 9-step idempotent bootstrap: dirs → config → engine → ontology → agentstream → registries → migrations → health
- **Agentstream Logs** — RDF event store: named graph per session, append-only N-Triples event log, materialized projections
- **User Homes** — `users/<id>/` with FOAF/SKOS profile, preferences, project/app symlinks, RDF index
- **Project Homes** — `projects/<id>/` with DOAP manifest, members, datasets, agents, RDF index
- **App Homes** — `apps/<id>/` with manifest, config, plugins, data, RDF index
- **Datasets Root** — `datasets/<id>/` with DCAT catalog, versioned snapshots, PROV-O provenance, quality metrics
- **Unified Config** — Single `config.toml` merging cubecast + metamind settings
- **Migration Plan** — 10-phase cutover from direct Oxigraph to cubecast with compatibility layer
- **Integration** — `AppState::boot` initializes data layer; controller logs to agentstream

### S. Integration Plan — Conversation Engine + RDF Backend (`integration_plan.md`)
- **7-Phase Build** — Foundation → Core SM Engine (`mm-sm`) → RDF Data + Unified Memory (`mm-data`) → Conversation Engine Core (`mm-cme`) → Planner/Executor/Routing/Voice → Unified Memory/Prefs/Plugins (parallel) → Observability/Improvement/Report/Dashboard → mm-serve Integration/Migration/Production
- **Critical Path** — Foundation → mm-sm → mm-data → mm-cme core → (Planner/Executor/Routing/Voice + Memory/Prefs/Plugins) → Obs/Improvement/Report/Dashboard → mm-serve Integration
- **New Crates** — `mm-sm` (core SM engine), `mm-data` (RDF layer + unified memory), `mm-cme` (conversation SMs); `mm-store-graph` deprecated
- **Key Milestones** — Phase 1: SM engine runs; Phase 2: RDF + memory works; Phase 3: Session/Turn/Chunk SMs run; Phase 4: Planner/Executor/Routing/Voice work; Phase 5: Memory/Prefs/Plugins delegate to mm-data; Phase 6: Full observability + improvement + generators; Phase 7: mm-serve on CME, migration, canary, v2.0.0-cme
- **Risk Mitigation** — Dual-run mode, feature flags, migration dry-runs, synthetic test regression detection, clear exit criteria per phase
- **Success Criteria** — Turn p99 < 2s, 200+ sessions, 99.9% uptime, full SM observability, <1 day new modality/plugin

### T. Unified Memory System (`unified_memory_system.md`)
- **4-Tier Hierarchy** — Episodic (raw turns, agentstream) → Compressed Segments (narrative + structured artifacts) → Project Memory (aggregated decisions, commitments, conventions) → User Memory (global preferences, expertise, cross-project learning)
- **Single Compression Pipeline** — LLM extraction → narrative → embeddings → quality validation → RDF + vector index
- **State Machine Driven** — Memory Region SM (Episodic → Compressing → Segmented → ProjectAggregating → UserSyncing) drives all transitions
- **RDF Storage Mapping** — Each tier maps to cubecast named graphs: agentstream, segments, projects, users
- **Session Continuity** — Resume with project/user context, personalized greetings, continuation suggestions
- **Multi-Device Sync** — Presence, state broadcast, voice↔text handoff with prosody preservation
- **Unified API** — `mm-data::MemorySystem` with episodic/segment/project/user stores, queries, sync, privacy
- **Configuration** — Single `memory.toml` for all tiers, compression, vector index, sync, privacy

### U. State Machine RDF Ontology (`state_machine_rdf.md`)
- **OWL Ontology** — Classes for StateMachine, State, Transition, Event, Guard, Action, OrthogonalRegion, HistoryState, Instance, TransitionEvent, Snapshot
- **Definition as Data** — SM definitions (states, transitions, regions, guards, actions) serialized as RDF triples
- **Runtime as Data** — SmInstance with current state, history stack, orthogonal region states, parent/child hierarchy
- **Events as PROV-O** — Transition events with timestamps, guard results, durations, provenance links
- **SPARQL Query Patterns** — Active states, transition history, guard failures, state diagrams, session filtering
- **RDF Storage Mapping** — Named graphs per session/turn/chunk, daily event logs, snapshot graphs in cubecast
- **Visualization** — Graphviz DOT export from SPARQL, live WebSocket state updates, debug UI diagrams
- **mm-sm Integration** — Trait extensions for to_turtle/from_turtle, automatic transition event logging

### V. Agent Harness Research Comparison (`agent_harness_comparison.md`)
- **35+ Papers Analyzed** — LangGraph, AutoGen, CrewAI, DSFlow, PGX, SEPM, HarnessMDP, HarnessNet, AgentFaultBench, AgentAudit, BTS-AgentBench, Verify the Verifier, Sage-Evolve, Auto-Alpha, ROGUE-AI, Swarm Shepherd, DiscernMem, TideCache, Aggron, AgentRoboCritic, and more
- **12 Comparison Dimensions** — Orchestration, Memory, Proactive, Tools, Routing, Voice, Self-Improvement, Observability, Testing, Security, Deployment, Multi-Agent
- **MetaMind Leads** — Hierarchical SM + orthogonal regions, deterministic planning, 3-tier voice with barge-in, 4-tier quality-validated memory, proactive SM with budgets, speculative KV pre-warm, 8-critic/6-verifier improvement, SM-level observability, synthetic SM tests
- **MetaMind Gaps** — Multi-agent coordination (P0), security hardening (P0), learnable harness/MDP (P1), task graph memory (P1), benchmark integration (P1), visual debugger (P2)
- **12 Priority Enhancements** — Mapped to research sources with effort estimates

### W. Conversation Manager v3 — Prompt Strategies, Frames, Data, Metamind, and Background Autonomy (`conversation_manager_agent_inspector_plan.md`)
- **Diagnosis** — 69 CLI-shaped tools, silent coercion, mid-JSON truncation, 4-round cap, static prompt, no topic/depth/status/frame state, no session presence/work catalog, no external work seam, no live inspector, no managed ontology data/search/schema catalog, no first-class plan/think/memory/action surface, and no supervised background autonomy

#### W.1 Three-Pass Turn Sequence: Interpret → Process → Verify (`three_pass_turn_sequence.md`)
- **Architecture** — Each user turn executes three separate LLM passes with tailored prompts, tool sets, and agentstream contexts; all passes share one agentstream session plus a clean log (user messages + final replies) injected into every pass
- **Pass 1: Interpret** — Analyzes request; tools: `metamind.verify()` (register checks), `metamind.approve(success=true)` (fast-path for pure conversation); outputs VerifyCheck[] + ProcessContext or fast-path reply
- **Pass 2: Process** — Executes task with 17 tools (16 internal + `agent.run` for external work); generates user-facing reply; loop continues until model stops calling tools; `metamind.think()` spawns sub-agent with own 3-pass cycle
- **Pass 3: Verify** — Runs verification checks (abstract `VerificationBackend` trait; LLM-as-judge first impl with structured output: pass/fail, reasoning, confidence 0.0000-1.0000); updates state/memory/db from Process output; calls `metamind.approve(success=false, error)` to retry Process (max 5) with error context, or `success=true` to complete
- **Fast-path** — Pure conversation (greetings, thanks, chat) exits at Interpret via `metamind.approve(true)`; no Process/Verify
- **Verification** — Default LLM judge check added if action requested but no explicit checks; confidence clamped to 1.0; threshold default 0.5
- **Sub-agents** — `metamind.think()` runs full 3-pass cycle in separate agentstream session; `agent.run` delegates to Hermes/Pi
- **Streaming** — Each pass streams tokens via SSE; UI aggregates into single message bubble with pass indicators
- **Tool visibility** — Interpret: verify/approve(true) only; Process: all 17 including agent.run; Verify: 16 internal + approve(false)
- **Migration** — 6 phases: verification backend → Interpret → Process refactor → Verify → orchestration/streaming → sub-agents; feature flag `MM_THREE_PASS_V2`

#### W.2 Process Pipeline System (`process_pipeline_system.md`)
- **Core Concept** — Reusable Process abstraction: templates (persistent, RDF/OWL, unique URI) → instances (queue inbox, mm-sm state machine, agentstream session) → execution (sequential/parallel/batch queue consumption)
- **Three Process Types** — Conversational (step-by-step user dialog), LLM Operations (autonomous LLM+tool loops), Backend Service (external service orchestration; future)
- **Step Model** — Each step: templated instructions for LLM, allowed tool subset, step type (llm_tool_loop, wait_condition, backend_call, user_input), wait conditions (tool result, user input, signal, timeout), parallel groups, output bindings
- **PROCESS.md** — Per-template documentation file (like SKILL.md) with step details, injected into LLM context
- **Plan → Process** — Evaluating a Plan auto-generates a ProcessTemplate, instantiates and executes it; Plan types: flat (sequential), tree (parallel groups, DFS), markdown (file-backed, flexible)
- **Plan Builder (2-4 Pass)** — Pass 1: structured classification (flat/tree/markdown); Pass 2: markdown generation (hints from Pass 1); Pass 3: decode to target format (structured); Pass 4: LLM Judge verify-decode (retries up to 5)
- **Plan Storage** — Flat/tree in RDF `plan` graph; markdown in `data/plans/<id>.md` with RDF reference; metadata: short random ID, label, description, 24h expiry (unless neverExpires)
- **Tools** — `metamind.act(process=, plan=, run=)` executes process or plan; `metamind.plan(request, structure=flat|tree|markdown, run=, list=, last=, never_expires=)` builds/runs/lists plans
- **Verification Reuse** — Step-level Verify phase reuses main Verify pass engine (VerificationBackend trait, LLM Judge, metamind.verify/approve tools)
- **RDF Integration** — All templates/instances/queues in `mm:process` named graph; queryable via data.* tools
- **Expiration** — Plans expire in 24h (configurable) unless neverExpires; daily cleanup job; processes: conversational inactivity timeout, LLM ops no expiry
- **Migration** — 6 phases: core engine → queue/parallel → plan builder → plan/process integration → process types/PROCESS.md → advanced features
- **Tool Surface** — Seventeen tools in three bounded families: conversation (5: `agent.run`, `conversation.status/topic/depth/frame`), data (7: `data.create/update/query/delete/search/schema/namespace`), metamind (5: `metamind.plan/think/memory/log/act`); strict schemas, full envelopes, typed errors; admin catalog and raw SPARQL remain CLI-only
- **Prompt Strategies / Frames** — New `mm-prompt` deterministic manager: complete `config/prompts/` tree including `frames/`, manifest-selected strategies, strict variable renderer, hash-addressed `PromptInstance`, `cm.turn.v1` character/source/tool/state/frame/pending/data/memory/budget fragments, `frame.utterance.v1`, fail-closed `PROMPT_UNAVAILABLE`, and no prompt literals in the executor
- **Session / Conversation State** — RDF `mm:Session` and `mm:SessionTrace` instances plus revisioned `ConversationState`/`SessionState`/`FrameStack`: lifecycle, presence, derived `working`, main+parallel work catalog, multiple agentstream traces with mandatory `main`, on-demand `data/sessions/<ULID>/`, cursor-paginated RDF listing, soft delete/purge, topic/depth, bounded typed status, intro/resume/work/tutorial/goodbye frames; no project/task CRUD; state block adds current user, session/work/traces/frames, pending, budget, health
- **Frames & Interjections** — Root `intro` with greet/connect, conversational tutorial yes/no offers, real-time utterance scheduling with natural delay/jitter and incremental backoff, goodbye then regular-TTL end; `conversation.frame` owns stack topology, `conversation.status` owns frame progress/variables; server-side `InterjectionRouter` suppresses internal actions while working and honors presence/frame/user/kind/instance policy
- **Data Substrate** — One Oxigraph/RocksDB dataset, complete RDF/XML snapshot (`metamind.rdf`) plus lossless N-Quads backup, Tantivy full-text index, schema catalog, namespace registry, canonical `mmd:` identity IRIs, FOAF `Person`/`mm:User` mapping, typed RDF instances, soft delete/supersede, provenance
- **Search Sync** — Every create/update/delete writes RDF then indexes Tantivy through a durable SQLite outbox; hit verification, startup rebuild, `mm-cli data verify/reindex`; RDF is authoritative
- **Automatic Capture** — Durable prompt policy induces implicit data management (search before create, update instead of duplicate, correct/supersede, never store secrets); visible `[STATE].data` health; scenario evals
- **Metamind Actions** — Durable versioned plans plus `mmd:plan-registry` (`metamind.plan`), private read-only inner thought (`metamind.think`), explicit memory recall/remember/forget/status (`metamind.memory`), multi-source agentstream/RDF log overview/latest/segment/append (`metamind.log`), generic registered internal actions with immediate/scheduled/cron execution, plan start/stop/pause/resume/retry/advance/complete/abandon, cancellation, and introspection (`metamind.act`)
- **Background Autonomy** — `SessionRegistry` presence/expiry sweep plus one `SessionAutomation` supervisor per active conversation: proactive tick, shared `ScheduledActionService` over `tokio-cron-scheduler` for any registered internal kind, frame timers, a prompt-backed five-minute internal action using recent RDF logs for originality, and a global `PlanMonitor` over the RDF registry; all external work uses `agent.run`
- **Budgets** — Deterministic initial classes (quick/standard/deep/resume/plan/think/memory/frame/action/summarize), `BudgetLedger` reserve/settle, nested child caps, state/status visibility, and honest `BUDGET` stops
- **State/Inspector** — Canonical `[STATE]` rebuilt before every model call; session/work/traces/frames/prompt/budget/pending/automation/interjection/plan-monitor projections; dirty-section `StatusService` with coalesced SSE; cursor-paginated Sessions tab and virtual Files tree (agentstream traces, RDF log, on-demand session files) with lazy per-directory paging; Status/Knowledge/Plan/Trace rail; no frontend mock data
- **Agent Delegation** — `agent.run` only external door; `mm-agent` crate; Hermes via OpenAI-compatible API in `agent/<run_id>` agentstream; Pi via `mm-pi` RPC; one downstream turn per call, summaries, user-workspace default, `ContinueUntilDone`
- **Build Plan** — 13 phases from contracts/prompt/frame tree through conversation tools, session/presence/frames, data substrate/tools, metamind services, agents, prompt/budget, background autonomy/interjections, status service, inspector/trace UI, migration
- **Phase 12 Hardening** — `MM_CM_V3` is on by default with a one-flag rollback; canonical `mmd:` user/project IRIs with legacy `urn:metamind:*` `owl:sameAs` aliases and registry fallbacks; `mm-cli data verify`/`reindex` compares live RDF entities with Tantivy subjects and repairs missing/orphan documents; operator guide, dual-run template, and index runbook in `infodocs/`
- **Acceptance** — Original complaints plus RDF sessions/traces/pagination, Files-tab log tree, no-mock frontend, session presence/working/expiry/resume, frame stack/intro/tutorials, interjection policy, prompt-strategy, budget, memory, log tool, plan-registry/control, introspection, generic internal-action scheduling, automation, current-user/workspace, data/search/schema/identity requirements mapped to concrete tests and evals

#### W.3 Unified Five-Phase Turn + Process Plan (`five_phase_turn_process_plan.md`)
- **Purpose** — Single ordered execution path that delivers both W.1 and W.2; supersedes their migration sections only
- **Phase 1: Substrate (implemented)** — `VerificationBackend` trait + LLM judge, 19-tool catalog (17 work + 2 control) with server-enforced pass scopes, four prompt strategies (`cm.interpret/process/verify/think.v1`), turn budget envelope with parent→child reservation, clean-log projection, pass events, config/flags (`MM_THREE_PASS_V2`, `MM_PROCESS_V1`)
- **Phase 2: Interpret (implemented)** — Interpret pass with 6-tool scope, fast path for pure conversation, default check injection, protocol repair, clean-log injection, interpret streaming
- **Phase 3: Process (implemented)** — Extracted reusable Process pass, `cm.process.v1` brief, 17-tool scope with `agent.run` confinement, child budget reservation, retry error context, single reply identity, legacy parity fixtures
- **Phase 4: Verify (implemented)** — Parallel checks, review call, **staged writes committed only on approval**, retry loop (5), `VERIFY_EXHAUSTED`, three-pass orchestration, recursive think under the Think scope, inspector pass timeline and verified indicator; live dual-run record still open
- **Phase 5: Process pipeline** — New `mm-process` crate: RDF templates/PROCESS.md, mm-sm instance states, durable queues with parallel/batch modes and leases, crash recovery, 4-pass plan builder, plan→process compilation, additive `metamind.plan`/`act` fields, expiry sweeps, sub-processes, versioning, analytics
- **Decisions** — Resolves source contradictions: corrected tool-scope matrix, staged Verify writes, no `mm-metacog` dependency inversion, action-based tool schema compatibility, structured-output boundary
- **Completeness** — Traceability matrix maps every section of both source documents to a phase and acceptance test; explicit non-goals and open questions with decision points

---

*This index now includes both the original source designs and the new conversation/voice architecture designs. Together they form a complete blueprint for a proactive, voice-capable, continuously-learning conversation engine.*