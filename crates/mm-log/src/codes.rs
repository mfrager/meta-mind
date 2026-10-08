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

/// Every code defined by the kernel, so `logs verify` can reject unknown codes.
pub const KERNEL_CODES: [&str; 70] = [
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
