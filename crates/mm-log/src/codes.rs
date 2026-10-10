//! Stable event codes.
//!
//! A code is an API: `logs verify` correlates records by code and later phases
//! subscribe to them, so codes are constants here rather than literals at call
//! sites.

/// Emitted once when the logger starts, naming its sinks.
pub const LOG_INIT: &str = "log.init";
/// Emitted when a record was altered by the redaction policy.
pub const LOG_REDACT: &str = "log.redact";
/// Emitted when a sink could not be written and records were dropped.
pub const LOG_SINK_ERROR: &str = "log.sink.error";

/// The SQLite store opened.
pub const STORE_SQL_OPEN: &str = "store.sql.open";
/// A migration ran.
pub const STORE_SQL_MIGRATE: &str = "store.sql.migrate";
/// The RDF store opened.
pub const STORE_GRAPH_OPEN: &str = "store.graph.open";
/// A graph transaction was flushed.
pub const STORE_GRAPH_TX: &str = "store.graph.tx";
/// A SPARQL query ran.
pub const STORE_GRAPH_SPARQL: &str = "store.graph.sparql";

/// An event was appended `provisional`.
pub const EVENTLOG_APPEND: &str = "eventlog.append";
/// An event was applied by a projection.
pub const EVENTLOG_APPLY: &str = "eventlog.apply";
/// An event was marked `committed`.
pub const EVENTLOG_COMMIT: &str = "eventlog.commit";
/// A `provisional` event was aborted because no commit followed it.
pub const EVENTLOG_ABORT: &str = "eventlog.abort";
/// A replay started.
pub const EVENTLOG_REPLAY_START: &str = "eventlog.replay.start";
/// A replay finished.
pub const EVENTLOG_REPLAY_END: &str = "eventlog.replay.end";

/// An ontology file was loaded into a named graph.
pub const ONTOLOGY_LOAD: &str = "ontology.load";
/// A SHACL validation ran.
pub const SHACL_VALIDATE: &str = "shacl.validate";

/// The audit chain was checked for gaps.
pub const AUDIT_SEQUENCE_CHECK: &str = "audit.sequence.check";

/// The kernel came up.
pub const KERNEL_BOOT: &str = "kernel.boot";

/// A codex scan started.
pub const CODEX_SCAN_START: &str = "codex.scan.start";
/// Reference code was copied into `vendor/`.
pub const CODEX_COPYING_REFERENCE: &str = "codex.copying.reference";
/// A module was discovered.
pub const CODEX_MODULE_DISCOVERED: &str = "codex.module.discovered";
/// A source file was discovered.
pub const CODEX_FILE_DISCOVERED: &str = "codex.file.discovered";
/// A symbol was extracted from a source file.
pub const CODEX_SYMBOL_EXTRACTED: &str = "codex.symbol.extracted";
/// A dependency edge was recorded.
pub const CODEX_DEPENDENCY_EDGE: &str = "codex.dependency.edge";
/// A capability was registered against a module.
pub const CODEX_CAPABILITY_REGISTERED: &str = "codex.capability.registered";
/// A `mmc:copiedFrom` provenance record was written.
pub const CODEX_COPIED_FROM_RECORD: &str = "codex.copied_from.record";
/// A `codex verify` rule failed.
pub const CODEX_VERIFY_FAIL: &str = "codex.verify.fail";
/// A structural drift check found a mismatch.
pub const CODEX_DRIFT_DETECTED: &str = "codex.drift.detected";
/// The registry was written (audit).
pub const CODEX_REGISTRY_WRITE: &str = "codex.registry.write";
/// The lock file was written (audit).
pub const CODEX_LOCK_WRITE: &str = "codex.lock.write";
/// The lock file was compared.
pub const CODEX_LOCK_DIFF: &str = "codex.lock.diff";
/// A derived graph view was emitted.
pub const CODEX_GRAPH_EMIT: &str = "codex.graph.emit";
/// A codex scan finished.
pub const CODEX_SCAN_END: &str = "codex.scan.end";

/// An LLM call was submitted.
pub const LLM_REQUEST: &str = "llm.request";
/// The router chose a model for a call.
pub const LLM_ROUTE_SELECT: &str = "llm.route.select";
/// A call was answered from the exact cache.
pub const LLM_CACHE_HIT: &str = "llm.cache.hit";
/// No cached answer covered a call.
pub const LLM_CACHE_MISS: &str = "llm.cache.miss";
/// A schema was compiled to a grammar.
pub const LLM_GRAMMAR_COMPILE: &str = "llm.grammar.compile";
/// The provider answered a call.
pub const LLM_RESPONSE: &str = "llm.response";
/// The output did not satisfy its schema. The record carries the failure kind and
/// a length, never the raw content.
pub const LLM_SCHEMA_REJECT: &str = "llm.schema.reject";
/// A bounded repair pass ran for a rejected output.
pub const LLM_REPAIR_ATTEMPT: &str = "llm.repair.attempt";
/// A recorded-session replay started.
pub const LLM_REPLAY_START: &str = "llm.replay.start";
/// A recorded-session replay finished; its `provider_calls` field must be 0.
pub const LLM_REPLAY_END: &str = "llm.replay.end";
/// A call was committed to the ledger.
pub const LLM_ACCOUNTING_COMMIT: &str = "llm.accounting.commit";
/// A call failed.
pub const LLM_ERROR: &str = "llm.error";

/// The being's identity was created on first open.
pub const BEING_IDENTITY_INIT: &str = "being.identity.init";
/// The invariant guard was consulted for an operation.
pub const BEING_INVARIANT_CHECK: &str = "being.invariant.check";
/// An operation was refused by the invariant guard.
pub const BEING_INVARIANT_VIOLATION: &str = "being.invariant.violation";
/// A core block was written.
pub const BEING_BLOCK_SET: &str = "being.block.set";
/// A personality disposition moved.
pub const BEING_PERSONALITY_UPDATE: &str = "being.personality.update";
/// An appraisal produced an affect impulse.
pub const BEING_AFFECT_IMPULSE: &str = "being.affect.impulse";
/// A belief's status or confidence changed.
pub const BEING_BELIEF_UPDATE: &str = "being.belief.update";
/// A belief promotion was refused for want of evidence.
pub const BEING_BELIEF_PROMOTION_DENIED: &str = "being.belief.promotion.denied";
/// A relationship dimension moved.
pub const BEING_RELATIONSHIP_UPDATE: &str = "being.relationship.update";
/// A goal moved between statuses.
pub const BEING_GOAL_TRANSITION: &str = "being.goal.transition";
/// A commitment moved between statuses.
pub const BEING_COMMITMENT_TRANSITION: &str = "being.commitment.transition";
/// A resource debit was written to the ledger.
pub const BEING_BUDGET_DEBIT: &str = "being.budget.debit";
/// A debit was refused by a budget policy.
pub const BEING_BUDGET_EXCEEDED: &str = "being.budget.exceeded";
/// Persistent state was mirrored into `/being`.
pub const BEING_RDF_MIRROR: &str = "being.rdf.mirror";

/// A memory was written (or declined as a duplicate).
pub const MEMORY_ADD: &str = "memory.add";
/// A recall ran, with what each channel contributed.
pub const MEMORY_RECALL: &str = "memory.recall";
/// The hybrid scores were recomputed and the parts audited.
pub const MEMORY_RERANK: &str = "memory.rerank";
/// A memory was surfaced and its retention reinforced.
pub const MEMORY_ACCESS: &str = "memory.access";
/// A consolidation step ran.
pub const MEMORY_CONSOLIDATE: &str = "memory.consolidate";
/// A summary-tree node was built.
pub const MEMORY_SUMMARY_BUILD: &str = "memory.summary.build";
/// A record was archived or spared by a forget cycle.
pub const MEMORY_FORGET: &str = "memory.forget";
/// A forget cycle refused to archive a protected record.
pub const MEMORY_PROTECTED_REFUSAL: &str = "memory.protected_refusal";
/// A mistake was recorded.
pub const MEMORY_MISTAKE_CREATE: &str = "memory.mistake.create";
/// A near miss was recorded.
pub const MEMORY_NEAR_MISS_CREATE: &str = "memory.near_miss.create";
/// The packed vector index was rebuilt.
pub const MEMORY_INDEX_REBUILD: &str = "memory.index.rebuild";
/// The memory store and its `/memory` mirror were reconciled.
pub const MEMORY_VERIFY: &str = "memory.verify";

/// A candidate claim was ingested (audit).
pub const EPISTEMIC_CLAIM_INGEST: &str = "epistemic.claim.ingest";
/// Evidence was attached to a claim (audit).
pub const EPISTEMIC_EVIDENCE_ATTACH: &str = "epistemic.evidence.attach";
/// A promotion was refused by the deterministic guard.
pub const EPISTEMIC_PROMOTION_REJECT: &str = "epistemic.promotion.reject";
/// A claim's status changed (audit).
pub const EPISTEMIC_STATUS_TRANSITION: &str = "epistemic.status.transition";
/// An assumption was created (audit).
pub const EPISTEMIC_ASSUMPTION_CREATE: &str = "epistemic.assumption.create";
/// An assumption's verification priority was computed.
pub const EPISTEMIC_ASSUMPTION_PRIORITY: &str = "epistemic.assumption.priority";
/// A contradiction was detected between two claims (audit).
pub const EPISTEMIC_CONTRADICTION_DETECT: &str = "epistemic.contradiction.detect";
/// A justification edge was recorded.
pub const EPISTEMIC_JUSTIFICATION_EDGE: &str = "epistemic.justification.edge";
/// A dependency-directed retraction cascaded (audit).
pub const EPISTEMIC_CASCADE: &str = "epistemic.cascade";
/// An RDFUnit-style data-quality test failed.
pub const EPISTEMIC_DATA_TESTS_FAIL: &str = "epistemic.data_tests.fail";
/// A claim was written to `/world` by the validation barrier (audit).
pub const EPISTEMIC_WORLD_WRITE: &str = "epistemic.world.write";

/// An entry was written to `/library` (audit).
pub const LIBRARY_ENTRY_UPSERT: &str = "library.entry.upsert";
/// An entry passed its shapes.
pub const LIBRARY_ENTRY_VALIDATED: &str = "library.entry.validated";
/// An entry was refused by the gate, with the shape it violated.
pub const LIBRARY_ENTRY_REJECTED: &str = "library.entry.rejected";
/// A duplicate body was detected by content hash.
pub const LIBRARY_DUP_DETECTED: &str = "library.dup.detected";
/// A dangling reference was found by the orphan check.
pub const LIBRARY_ORPHAN_DETECTED: &str = "library.orphan.detected";
/// A skill was registered, as a draft.
pub const SKILL_REGISTER: &str = "skill.register";
/// A skill's verification was decided (audit).
pub const SKILL_VERIFY: &str = "skill.verify";
/// A skill was retrieved.
pub const SKILL_RETRIEVE: &str = "skill.retrieve";
/// An extraction job started.
pub const LIBRARY_EXTRACT_JOB_START: &str = "library.extract.job.start";
/// An extraction job finished.
pub const LIBRARY_EXTRACT_JOB_END: &str = "library.extract.job.end";
/// Techniques were ranked for a state.
pub const LIBRARY_APPLICABLE_RANK: &str = "library.applicable.rank";
/// Cases were retrieved for a problem.
pub const LIBRARY_CASE_RETRIEVE: &str = "library.case.retrieve";
/// The experience compiler ran.
pub const EXPERIENCE_COMPILE: &str = "experience.compile";
/// Frames were composed into an instance.
pub const LIBRARY_FRAME_COMPOSE: &str = "library.frame.compose";
/// A frame activation changed.
pub const LIBRARY_FRAME_SWITCH: &str = "library.frame.switch";
/// A new immutable policy version was created (audit).
pub const POLICY_VERSION_CREATE: &str = "policy.version.create";
/// A policy's fitness was updated (audit).
pub const POLICY_FITNESS_UPDATE: &str = "policy.fitness.update";
/// An evolution run started.
pub const GENOME_EVOLVE_START: &str = "genome.evolve.start";
/// One evolution generation finished.
pub const GENOME_EVOLVE_GENERATION: &str = "genome.evolve.generation";
/// An evolution run finished (audit).
pub const GENOME_EVOLVE_END: &str = "genome.evolve.end";

/// A cognitive episode was opened.
pub const METACOG_EPISODE_OPEN: &str = "metacog.episode.open";
/// The metacognitive scan started.
pub const METACOG_SCAN_BEGIN: &str = "metacog.scan.begin";
/// The scan named one issue.
pub const METACOG_SCAN_ISSUE: &str = "metacog.scan.issue";
/// The scan finished.
pub const METACOG_SCAN_END: &str = "metacog.scan.end";
/// A compute policy was selected for the episode.
pub const METACOG_TIER_SELECT: &str = "metacog.tier.select";
/// A cognitive program was compiled.
pub const METACOG_PROGRAM_COMPILE: &str = "metacog.program.compile";
/// One candidate operation was scored.
pub const METACOG_OP_CONSIDER: &str = "metacog.op.consider";
/// One operation was selected by the greedy rule.
pub const METACOG_OP_SELECT: &str = "metacog.op.select";
/// One operation was executed (audit).
pub const METACOG_OP_EXECUTE: &str = "metacog.op.execute";
/// One operation's outcome was recorded.
pub const METACOG_OP_OUTCOME: &str = "metacog.op.outcome";
/// A budget dimension was debited.
pub const METACOG_BUDGET_DEBIT: &str = "metacog.budget.debit";
/// A budget dimension ran out.
pub const METACOG_BUDGET_EXHAUSTED: &str = "metacog.budget.exhausted";
/// The budget forcer permitted, added or reserved compute.
pub const METACOG_FORCE_DECIDE: &str = "metacog.force.decide";
/// The metacognitive loop stopped, with its reason.
pub const METACOG_STOP: &str = "metacog.stop";
/// A program trace was persisted (audit).
pub const METACOG_TRACE_PERSIST: &str = "metacog.trace.persist";
/// A program was lowered to an execution document.
pub const METACOG_PROGRAM_LOWER: &str = "metacog.program.lower";
/// An episode was replayed and compared.
pub const METACOG_REPLAY: &str = "metacog.replay";

// -------------------------------------------------------------------- Phase 9 --

/// A decision core answered a bounded question.
pub const DECISION_CORE_ANSWER: &str = "decision.core.answer";
/// A decision core could not answer, and said so rather than guessing.
pub const DECISION_CORE_UNAVAILABLE: &str = "decision.core.unavailable";
/// The rules core matched (or failed to match) a question.
pub const DECISION_RULES_MATCH: &str = "decision.rules.match";
/// The hosted core spent a call on a bounded question.
pub const DECISION_HOSTED_CALL: &str = "decision.hosted.call";
/// A calibrated answer was refused by its conformal threshold.
pub const DECISION_ABSTAIN: &str = "decision.abstain";
/// A decision was recorded (audit).
pub const DECISION_LOG_WRITE: &str = "decision.log.write";
/// A risk profile was computed.
pub const RISK_ANALYZE: &str = "risk.analyze";
/// A comparison contract was read.
pub const COMPARE_CONTRACT_START: &str = "compare.contract";
/// A comparison check concluded a verdict.
pub const COMPARE_CHECK: &str = "compare.check";
/// A comparable pair was normalized into one set of units.
pub const COMPARE_NORMALIZE: &str = "compare.normalize";
/// An uncertainty scorer produced a value.
pub const UNCERTAINTY_SCORE: &str = "uncertainty.score";
/// A factuality check produced a support score.
pub const FACTUALITY_CHECK: &str = "factuality.check";
/// A calibrator was fitted for one decision class.
pub const CALIBRATION_FIT: &str = "calibration.fit";
/// A conformal threshold was fitted for one decision class.
pub const CONFORMAL_ADJUST: &str = "conformal.adjust";
/// A firewall evaluation started.
pub const FIREWALL_SCAN_START: &str = "firewall.scan.start";
/// A hard prohibition short-circuited an evaluation to `REJECT`.
pub const FIREWALL_PROHIBITION_SHORTCIRCUIT: &str = "firewall.prohibition.shortcircuit";
/// The signals were aggregated into one outcome.
pub const FIREWALL_AGGREGATE: &str = "firewall.aggregate";
/// A firewall run was recorded (audit).
pub const FIREWALL_REPORT: &str = "firewall.report";

// ------------------------------------------------------------------- Phase 10 --

/// A tool's spec was written to `tool_registry` (audit).
pub const TOOL_REGISTRY_REGISTER: &str = "tool.registry.register";
/// An action started, before authorization (audit).
pub const TOOL_INVOKE_START: &str = "tool.invoke.start";
/// The permission engine returned a decision (audit).
pub const TOOL_PERMISSION_CHECK: &str = "tool.permission.check";
/// A policy rule forbade an action (audit).
pub const TOOL_POLICY_FORBID: &str = "tool.policy.forbid";
/// A sandbox was entered for an action (audit).
pub const TOOL_SANDBOX_START: &str = "tool.sandbox.start";
/// The capability guard refused before any work started (audit).
pub const TOOL_SANDBOX_DENY: &str = "tool.sandbox.deny";
/// A repeat with an idempotency key returned a recorded outcome.
pub const TOOL_IDEMPOTENCY_HIT: &str = "tool.idempotency.hit";
/// An action ended, successfully or not (audit).
pub const TOOL_INVOKE_END: &str = "tool.invoke.end";
/// An entry was appended to the action ledger.
pub const ACTION_LEDGER_APPEND: &str = "action.ledger.append";
/// A rollback started (audit).
pub const ACTION_ROLLBACK_START: &str = "action.rollback.start";
/// A rollback finished, with the hash it restored (audit).
pub const ACTION_ROLLBACK_END: &str = "action.rollback.end";
/// An execution-sourced observation was admitted to `/world` (audit).
pub const OBSERVATION_RECORD: &str = "observation.record";
/// A verification obligation started (audit).
pub const VERIFY_RUN_START: &str = "verify.run.start";
/// A verification obligation concluded, with its proof (audit).
pub const VERIFY_RUN_END: &str = "verify.run.end";
/// A verification obligation failed, with a counterexample (audit).
pub const VERIFY_OBLIGATION_FAILURE: &str = "verify.obligation.failure";
/// An MCP client asked for the tool list.
pub const MCP_TOOLS_LIST: &str = "mcp.tools.list";
/// An MCP client called a tool.
pub const MCP_TOOLS_CALL: &str = "mcp.tools.call";

// ------------------------------------------------------------------- Phase 11 --

/// A self-improvement trigger fired.
pub const META_TRIGGER: &str = "meta.trigger";
/// A bounded diagnosis pass classified an episode's errors.
pub const META_DIAGNOSE: &str = "meta.diagnose";
/// A lesson was extracted into the cognitive library.
pub const META_LESSON: &str = "meta.lesson";
/// A calibration pass was scored.
pub const CALIB_REPORT: &str = "calib.report";
/// A mistake was compiled into a regression test.
pub const MISTAKE_REGRESS: &str = "mistake.regress";
/// A typed change set was assembled.
pub const CHANGESET_CREATE: &str = "changeset.create";
/// A change set moved through the sandbox pipeline.
pub const CHANGESET_STAGE: &str = "changeset.stage";
/// A sandbox was prepared for a change set.
pub const SANDBOX_PREPARE: &str = "sandbox.prepare";
/// A sandbox build finished.
pub const SANDBOX_BUILD: &str = "sandbox.build";
/// A sandbox test finished.
pub const SANDBOX_TEST: &str = "sandbox.test";
/// A benchmark compared a candidate against the frozen baseline.
pub const BENCH_RUN: &str = "bench.run";
/// A shadow run compared an unregistered candidate with production behaviour.
pub const SHADOW_START: &str = "shadow.start";
/// The promotion gate decided a change set (audit).
pub const GATE_DECIDE: &str = "gate.decide";
/// A change set was promoted (audit).
pub const PROMOTE_COMMIT: &str = "promote.commit";
/// A change set was rejected, with its written reason (audit).
pub const PROMOTE_REJECT: &str = "promote.reject";
/// A rollback restored a change set's recorded state (audit).
pub const ROLLBACK_APPLY: &str = "rollback.apply";
/// A budget dimension was debited (audit).
pub const BUDGET_DEBIT: &str = "budget.debit";
/// A debit was refused because it would cross a limit.
pub const BUDGET_DENY: &str = "budget.deny";
/// A Pi session process was spawned.
pub const PI_SPAWN: &str = "pi.spawn";
/// A command was sent to a Pi session.
pub const PI_COMMAND: &str = "pi.command";
/// One record arrived from a Pi session.
pub const PI_EVENT: &str = "pi.event";
/// A recorded Pi session was ingested into the event log and `/code` (audit).
pub const PI_SESSION_INGEST: &str = "pi.session.ingest";
/// Pi edited a file inside the sandbox (audit).
pub const PI_EDIT: &str = "pi.edit";
/// External reference code was integrated behind an `mm-*` trait.
pub const COPY_INTEGRATE: &str = "copy.integrate";

// ------------------------------------------------------------------- Phase 12 --

/// A closed-loop run started.
pub const LOOP_RUN_START: &str = "loop.run.start";
/// A closed-loop run reached a terminal status.
pub const LOOP_RUN_END: &str = "loop.run.end";
/// A loop stage started.
pub const LOOP_ITERATION_START: &str = "loop.iteration.start";
/// A loop stage finished, with its outcome and latency.
pub const LOOP_ITERATION_END: &str = "loop.iteration.end";
/// A budget envelope was debited (audit).
pub const LOOP_BUDGET_DEBIT: &str = "loop.budget.debit";
/// A stage's debit was refused because it would cross the envelope's cap.
pub const LOOP_BUDGET_DENY: &str = "loop.budget.deny";
/// A crashed loop resumed at its last committed stage.
pub const LOOP_STAGE_RESUME: &str = "loop.stage.resume";
/// A scheduler clock fired.
pub const TIMESCALE_TICK: &str = "timescale.tick";
/// One numeric self-model report was written (audit).
pub const SELF_MODEL_REPORT: &str = "self_model.report";
/// One divergence dimension of a report.
pub const SELF_MODEL_DIVERGENCE: &str = "self_model.divergence";
/// An architectural-debt scan ran.
pub const DEBT_SCAN: &str = "debt.scan";
/// One debt finding was recorded.
pub const DEBT_FINDING: &str = "debt.finding";
/// One GC action was proposed or applied (audit).
pub const GC_ACTION: &str = "gc.action";
/// A GC action was refused: a protected ledger subject, or a non-reversible action.
pub const GC_REFUSE: &str = "gc.refuse";
/// A design document revision was registered (audit).
pub const DESIGN_REVISION: &str = "design.revision";
/// A module version was hot-loaded (audit).
pub const MODULE_LOAD: &str = "module.load";
/// A module version was rolled back to the one before it (audit).
pub const MODULE_ROLLBACK: &str = "module.rollback";
/// One identity invariant was checked during a run.
pub const INVARIANT_CHECK: &str = "invariant.check";

/// Every code defined by the kernel, so `logs verify` can reject unknown codes.
pub const KERNEL_CODES: [&str; 195] = [
    LOG_INIT,
    LOG_REDACT,
    LOG_SINK_ERROR,
    STORE_SQL_OPEN,
    STORE_SQL_MIGRATE,
    STORE_GRAPH_OPEN,
    STORE_GRAPH_TX,
    STORE_GRAPH_SPARQL,
    EVENTLOG_APPEND,
    EVENTLOG_APPLY,
    EVENTLOG_COMMIT,
    EVENTLOG_ABORT,
    EVENTLOG_REPLAY_START,
    EVENTLOG_REPLAY_END,
    ONTOLOGY_LOAD,
    SHACL_VALIDATE,
    AUDIT_SEQUENCE_CHECK,
    CODEX_SCAN_START,
    CODEX_COPYING_REFERENCE,
    CODEX_MODULE_DISCOVERED,
    CODEX_FILE_DISCOVERED,
    CODEX_SYMBOL_EXTRACTED,
    CODEX_DEPENDENCY_EDGE,
    CODEX_CAPABILITY_REGISTERED,
    CODEX_COPIED_FROM_RECORD,
    CODEX_VERIFY_FAIL,
    CODEX_DRIFT_DETECTED,
    CODEX_REGISTRY_WRITE,
    CODEX_LOCK_WRITE,
    CODEX_LOCK_DIFF,
    CODEX_GRAPH_EMIT,
    CODEX_SCAN_END,
    LLM_REQUEST,
    LLM_ROUTE_SELECT,
    LLM_CACHE_HIT,
    LLM_CACHE_MISS,
    LLM_GRAMMAR_COMPILE,
    LLM_RESPONSE,
    LLM_SCHEMA_REJECT,
    LLM_REPAIR_ATTEMPT,
    LLM_REPLAY_START,
    LLM_REPLAY_END,
    LLM_ACCOUNTING_COMMIT,
    LLM_ERROR,
    BEING_IDENTITY_INIT,
    BEING_INVARIANT_CHECK,
    BEING_INVARIANT_VIOLATION,
    BEING_BLOCK_SET,
    BEING_PERSONALITY_UPDATE,
    BEING_AFFECT_IMPULSE,
    BEING_BELIEF_UPDATE,
    BEING_BELIEF_PROMOTION_DENIED,
    BEING_RELATIONSHIP_UPDATE,
    BEING_GOAL_TRANSITION,
    BEING_COMMITMENT_TRANSITION,
    BEING_BUDGET_DEBIT,
    BEING_BUDGET_EXCEEDED,
    BEING_RDF_MIRROR,
    MEMORY_ADD,
    MEMORY_RECALL,
    MEMORY_RERANK,
    MEMORY_ACCESS,
    MEMORY_CONSOLIDATE,
    MEMORY_SUMMARY_BUILD,
    MEMORY_FORGET,
    MEMORY_PROTECTED_REFUSAL,
    MEMORY_MISTAKE_CREATE,
    MEMORY_NEAR_MISS_CREATE,
    MEMORY_INDEX_REBUILD,
    MEMORY_VERIFY,
    EPISTEMIC_CLAIM_INGEST,
    EPISTEMIC_EVIDENCE_ATTACH,
    EPISTEMIC_PROMOTION_REJECT,
    EPISTEMIC_STATUS_TRANSITION,
    EPISTEMIC_ASSUMPTION_CREATE,
    EPISTEMIC_ASSUMPTION_PRIORITY,
    EPISTEMIC_CONTRADICTION_DETECT,
    EPISTEMIC_JUSTIFICATION_EDGE,
    EPISTEMIC_CASCADE,
    EPISTEMIC_DATA_TESTS_FAIL,
    EPISTEMIC_WORLD_WRITE,
    LIBRARY_ENTRY_UPSERT,
    LIBRARY_ENTRY_VALIDATED,
    LIBRARY_ENTRY_REJECTED,
    LIBRARY_DUP_DETECTED,
    LIBRARY_ORPHAN_DETECTED,
    SKILL_REGISTER,
    SKILL_VERIFY,
    SKILL_RETRIEVE,
    LIBRARY_EXTRACT_JOB_START,
    LIBRARY_EXTRACT_JOB_END,
    LIBRARY_APPLICABLE_RANK,
    LIBRARY_CASE_RETRIEVE,
    EXPERIENCE_COMPILE,
    LIBRARY_FRAME_COMPOSE,
    LIBRARY_FRAME_SWITCH,
    POLICY_VERSION_CREATE,
    POLICY_FITNESS_UPDATE,
    GENOME_EVOLVE_START,
    GENOME_EVOLVE_GENERATION,
    GENOME_EVOLVE_END,
    METACOG_EPISODE_OPEN,
    METACOG_SCAN_BEGIN,
    METACOG_SCAN_ISSUE,
    METACOG_SCAN_END,
    METACOG_TIER_SELECT,
    METACOG_PROGRAM_COMPILE,
    METACOG_OP_CONSIDER,
    METACOG_OP_SELECT,
    METACOG_OP_EXECUTE,
    METACOG_OP_OUTCOME,
    METACOG_BUDGET_DEBIT,
    METACOG_BUDGET_EXHAUSTED,
    METACOG_FORCE_DECIDE,
    METACOG_STOP,
    METACOG_TRACE_PERSIST,
    METACOG_PROGRAM_LOWER,
    METACOG_REPLAY,
    DECISION_CORE_ANSWER,
    DECISION_CORE_UNAVAILABLE,
    DECISION_RULES_MATCH,
    DECISION_HOSTED_CALL,
    DECISION_ABSTAIN,
    DECISION_LOG_WRITE,
    RISK_ANALYZE,
    COMPARE_CONTRACT_START,
    COMPARE_CHECK,
    COMPARE_NORMALIZE,
    UNCERTAINTY_SCORE,
    FACTUALITY_CHECK,
    CALIBRATION_FIT,
    CONFORMAL_ADJUST,
    FIREWALL_SCAN_START,
    FIREWALL_PROHIBITION_SHORTCIRCUIT,
    FIREWALL_AGGREGATE,
    FIREWALL_REPORT,
    TOOL_REGISTRY_REGISTER,
    TOOL_INVOKE_START,
    TOOL_PERMISSION_CHECK,
    TOOL_POLICY_FORBID,
    TOOL_SANDBOX_START,
    TOOL_SANDBOX_DENY,
    TOOL_IDEMPOTENCY_HIT,
    TOOL_INVOKE_END,
    ACTION_LEDGER_APPEND,
    ACTION_ROLLBACK_START,
    ACTION_ROLLBACK_END,
    OBSERVATION_RECORD,
    VERIFY_RUN_START,
    VERIFY_RUN_END,
    VERIFY_OBLIGATION_FAILURE,
    MCP_TOOLS_LIST,
    MCP_TOOLS_CALL,
    META_TRIGGER,
    META_DIAGNOSE,
    META_LESSON,
    CALIB_REPORT,
    MISTAKE_REGRESS,
    CHANGESET_CREATE,
    CHANGESET_STAGE,
    SANDBOX_PREPARE,
    SANDBOX_BUILD,
    SANDBOX_TEST,
    BENCH_RUN,
    SHADOW_START,
    GATE_DECIDE,
    PROMOTE_COMMIT,
    PROMOTE_REJECT,
    ROLLBACK_APPLY,
    BUDGET_DEBIT,
    BUDGET_DENY,
    PI_SPAWN,
    PI_COMMAND,
    PI_EVENT,
    PI_SESSION_INGEST,
    PI_EDIT,
    COPY_INTEGRATE,
    LOOP_RUN_START,
    LOOP_RUN_END,
    LOOP_ITERATION_START,
    LOOP_ITERATION_END,
    LOOP_BUDGET_DEBIT,
    LOOP_BUDGET_DENY,
    LOOP_STAGE_RESUME,
    TIMESCALE_TICK,
    SELF_MODEL_REPORT,
    SELF_MODEL_DIVERGENCE,
    DEBT_SCAN,
    DEBT_FINDING,
    GC_ACTION,
    GC_REFUSE,
    DESIGN_REVISION,
    MODULE_LOAD,
    MODULE_ROLLBACK,
    INVARIANT_CHECK,
];

/// True when `code` is a Phase 1 kernel code.
pub fn is_kernel_code(code: &str) -> bool {
    KERNEL_CODES.contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn codes_are_unique_and_non_empty() {
        let set: HashSet<_> = KERNEL_CODES.iter().collect();
        assert_eq!(set.len(), KERNEL_CODES.len(), "duplicate event code");
        for code in KERNEL_CODES {
            assert!(!code.is_empty());
            assert!(code.contains('.'), "{code} should be dotted");
        }
    }

    #[test]
    fn kernel_boot_code_matches_the_event_kind_wire_name() {
        assert_eq!(KERNEL_BOOT, mm_core::EventKind::KernelBoot.as_str());
    }

    #[test]
    fn membership_check() {
        assert!(is_kernel_code(EVENTLOG_COMMIT));
        assert!(!is_kernel_code("cognition.think"));
    }
}
