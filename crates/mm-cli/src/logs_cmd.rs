//! `mm-cli logs` — the logging gate.
//!
//! `logs verify` is the check every phase must keep passing. It answers eight
//! questions, each of which is a way logging silently stops being trustworthy:
//! is the audit chain intact, does every committed event have exactly one audit
//! record, is the event sequence gapless, is every record schema-valid, did a
//! secret reach a sink, does logging change what a replay produces, does every
//! LLM ledger row have exactly one audited commit (with no replay admitting a
//! provider call), does every committed being operation have exactly one audit
//! record correlated by its ULID, and does every audited memory operation name a
//! memory row that exists.

use std::process::ExitCode;
use std::sync::Arc;

use mm_core::{MmError, UlidFactory};
use mm_eventlog::{CountingApplier, EventLog, EventStatus};
use mm_log::{codes, Level, LogRecord, Logger, RedactionPolicy};
use mm_store_sqlite::AsOf;

use crate::kernel::Kernel;

/// The secret patterns that ship with the kernel; extra ones may be added by a
/// fixture file when one is present.
const SECRET_FIXTURE: &str = "tests/fixtures/secrets/patterns.json";

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

impl Check {
    fn new(name: &'static str) -> Self {
        Check {
            name,
            ok: true,
            detail: String::new(),
        }
    }

    fn fail(&mut self, detail: impl Into<String>) {
        self.ok = false;
        let detail = detail.into();
        if self.detail.is_empty() {
            self.detail = detail;
        }
    }
}

/// Run every logging check.
pub async fn verify(cfg: mm_core::Config) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, false).await?;
    let mut checks = Vec::new();

    // 1. The audit chain itself.
    let chain = kernel.sqlite.audit_chain_report().await?;
    let mut c = Check::new("audit chain");
    if !chain.is_ok() {
        c.fail(chain.summary());
    }
    checks.push((
        c,
        format!(
            "{} record(s), {} re-hashed",
            chain.rows, chain.hashes_verified
        ),
    ));

    // 2. Exactly one audit record per committed (or aborted) event.
    let events = kernel.events.all_records().await?;
    let audit_rows = kernel.sqlite.audit_rows().await?;
    let settled = events
        .iter()
        .filter(|e| matches!(e.status, EventStatus::Committed | EventStatus::Aborted))
        .count();
    let event_audits = audit_rows
        .iter()
        .filter(|r| r.event_code == codes::EVENTLOG_COMMIT || r.event_code == codes::EVENTLOG_ABORT)
        .count();
    let mut c = Check::new("audit completeness");
    if settled != event_audits {
        c.fail(format!(
            "{settled} settled event(s) but {event_audits} audit record(s)"
        ));
    }
    checks.push((c, format!("{settled} settled event(s)")));

    // 3. Gapless event sequence over every row, whatever its status.
    let mut c = Check::new("event sequence");
    for (expected, event) in (1i64..).zip(&events) {
        if event.seq != expected {
            c.fail(format!("expected seq {expected}, found {}", event.seq));
            break;
        }
    }
    checks.push((c, format!("{} event(s)", events.len())));

    // 4. Every emitted record is schema-valid.
    let lines = read_lines(&kernel.cfg.jsonl_path())?;
    let mut c = Check::new("record schema");
    for (i, line) in lines.iter().enumerate() {
        if let Err(e) = validate_record(line) {
            c.fail(format!("line {}: {e}", i + 1));
            break;
        }
    }
    checks.push((c, format!("{} record(s)", lines.len())));

    // 5. No secret reached a sink, including on error paths.
    let mut policy = RedactionPolicy::kernel_default();
    let fixture = kernel.cfg.root.join(SECRET_FIXTURE);
    if fixture.exists() {
        policy = policy.extend_from_file(&fixture)?;
    }
    let mut c = Check::new("redaction");
    let mut leaks = 0usize;
    for line in &lines {
        let mut value = serde_json::Value::String(line.clone());
        if policy.apply_to_value(&mut value) > 0 {
            leaks += 1;
        }
    }
    for row in &audit_rows {
        if policy.apply_to_str(&row.payload).1 > 0 {
            leaks += 1;
        }
    }
    if leaks > 0 {
        c.fail(format!(
            "{leaks} value(s) matched a secret pattern in a sink"
        ));
    }
    checks.push((
        c,
        format!("{} sink(s) scanned", lines.len() + audit_rows.len()),
    ));

    // 6. Replay produces the same state with logging on and with it off.
    let quiet = Arc::new(Logger::new(
        Level::Error,
        Vec::new(),
        Some(Arc::new(kernel.sqlite.clone())),
        RedactionPolicy::kernel_default(),
    ));
    let quiet_log = EventLog::new(kernel.sqlite.clone(), quiet, Arc::new(UlidFactory::new()));
    let mut loud_applier = CountingApplier::new();
    let loud = kernel.events.replay(AsOf::now(), &mut loud_applier).await?;
    let mut quiet_applier = CountingApplier::new();
    let silent = quiet_log.replay(AsOf::now(), &mut quiet_applier).await?;
    let mut c = Check::new("replay determinism");
    if loud.log_hash != silent.log_hash {
        c.fail(format!(
            "state_hash differs with logging on ({}) and off ({})",
            loud.log_hash, silent.log_hash
        ));
    }
    if loud.applier_hash != silent.applier_hash {
        c.fail("applier hash differs with logging on and off");
    }
    checks.push((c, format!("state_hash {}", loud.log_hash)));

    // 7. LLM accounting: every ledger row has exactly one audited commit record,
    //    and no replay admitted a provider call. The ledger, the audit chain, and
    //    the replay claim are written by different code, so a disagreement here is
    //    how a Phase 3 guarantee silently stops holding.
    let mut c = Check::new("llm accounting");
    let ledger: Vec<String> = {
        let index: &dyn mm_core::Tabular = &kernel.sqlite;
        index
            .query_json("SELECT id FROM llm_calls", mm_core::Params::new())
            .await?
            .iter()
            .filter_map(|row| row["id"].as_str().map(str::to_string))
            .collect()
    };
    let mut commits: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for row in &audit_rows {
        if row.event_code != codes::LLM_ACCOUNTING_COMMIT {
            continue;
        }
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&row.payload) {
            if let Some(id) = payload["call_id"].as_str() {
                *commits.entry(id.to_string()).or_default() += 1;
            }
        }
    }
    for id in &ledger {
        match commits.get(id).copied().unwrap_or(0) {
            1 => {}
            0 => c.fail(format!("ledger row {id} has no audited commit record")),
            n => c.fail(format!("ledger row {id} has {n} audited commit records")),
        }
    }
    for id in commits.keys() {
        if !ledger.contains(id) {
            c.fail(format!("audited commit {id} has no ledger row"));
        }
    }
    let mut replays = 0usize;
    for line in &lines {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            if value["event_code"] == codes::LLM_REPLAY_END {
                replays += 1;
                if value["fields"]["provider_calls"].as_u64() != Some(0) {
                    c.fail(format!(
                        "a replay reported {} provider call(s)",
                        value["fields"]["provider_calls"]
                    ));
                }
            }
        }
    }
    checks.push((
        c,
        format!("{} ledger row(s), {replays} replay(s)", ledger.len()),
    ));

    // 8. Being: every committed being operation has exactly one audit record,
    //    correlated by the operation's ULID. A goal transition, a debit, and a
    //    belief update each write a table row keyed by that ULID and an audit
    //    record carrying it as the trace id, so the two can be checked against
    //    each other without a second bookkeeping table. The identity is created
    //    once, so its init record must match its row one-for-one.
    let mut c = Check::new("being correlation");
    let index: &dyn mm_core::Tabular = &kernel.sqlite;
    let mut correlated = 0usize;
    for (code, table) in [
        (codes::BEING_GOAL_TRANSITION, "goal_transitions"),
        (codes::BEING_COMMITMENT_TRANSITION, "commitment_transitions"),
        (codes::BEING_BUDGET_DEBIT, "resource_ledger"),
        (codes::BEING_BELIEF_UPDATE, "user_beliefs"),
    ] {
        let rows = index
            .query_json(&format!("SELECT id FROM {table}"), mm_core::Params::new())
            .await?;
        let ids: std::collections::BTreeSet<&str> =
            rows.iter().filter_map(|row| row["id"].as_str()).collect();
        for row in &audit_rows {
            if row.event_code != code {
                continue;
            }
            correlated += 1;
            match row.trace_id.as_deref() {
                Some(trace) if ids.contains(trace) => {}
                Some(trace) => c.fail(format!("{code} {trace} has no {table} row")),
                None => c.fail(format!("{code} audit record has no trace id")),
            }
        }
    }
    let identities = index
        .query_json("SELECT id FROM identity", mm_core::Params::new())
        .await?;
    let init_traces: Vec<&str> = audit_rows
        .iter()
        .filter(|row| row.event_code == codes::BEING_IDENTITY_INIT)
        .filter_map(|row| row.trace_id.as_deref())
        .collect();
    if identities.len() != init_traces.len() {
        c.fail(format!(
            "{} identity row(s) but {} init audit record(s)",
            identities.len(),
            init_traces.len()
        ));
    }
    for row in &identities {
        if let Some(id) = row["id"].as_str() {
            match init_traces.iter().filter(|trace| **trace == id).count() {
                1 => {}
                0 => c.fail(format!("identity {id} has no init audit record")),
                n => c.fail(format!("identity {id} has {n} init audit records")),
            }
        }
    }
    checks.push((
        c,
        format!(
            "{correlated} being op(s), {} identity(ies)",
            identities.len()
        ),
    ));

    // 9. Memory: every audited memory operation names a memory row that exists.
    //    An add, an archival, a refusal, and a mistake each write a `memories` row
    //    and an audit record carrying its ULID as the trace id, so the two are
    //    checkable against each other without a second bookkeeping table. A memory
    //    that was forgotten is *archived*, never deleted, so its audit record must
    //    keep resolving — which is what makes this check able to prove the
    //    no-deletion invariant from the log alone.
    let mut c = Check::new("memory correlation");
    let memory_rows = index
        .query_json("SELECT id FROM memories", mm_core::Params::new())
        .await?;
    let memory_ids: std::collections::BTreeSet<&str> = memory_rows
        .iter()
        .filter_map(|row| row["id"].as_str())
        .collect();
    let mut memory_ops = 0usize;
    for code in [
        codes::MEMORY_ADD,
        codes::MEMORY_FORGET,
        codes::MEMORY_PROTECTED_REFUSAL,
        codes::MEMORY_MISTAKE_CREATE,
        codes::MEMORY_NEAR_MISS_CREATE,
    ] {
        for row in &audit_rows {
            if row.event_code != code {
                continue;
            }
            memory_ops += 1;
            match row.trace_id.as_deref() {
                Some(trace) if memory_ids.contains(trace) => {}
                Some(trace) => c.fail(format!("{code} {trace} has no memories row")),
                None => c.fail(format!("{code} audit record has no trace id")),
            }
        }
    }
    // A `memory.add` record must exist for every memory row, because a row that
    // appeared without an audit record appeared without the log knowing.
    let adds: std::collections::BTreeSet<&str> = audit_rows
        .iter()
        .filter(|row| row.event_code == codes::MEMORY_ADD)
        .filter_map(|row| row.trace_id.as_deref())
        .collect();
    for id in &memory_ids {
        if !adds.contains(id) {
            c.fail(format!("memory {id} has no audited add record"));
        }
    }
    checks.push((
        c,
        format!("{memory_ops} memory op(s), {} row(s)", memory_ids.len()),
    ));

    // 10. Epistemic: every audited epistemic operation names a row that exists,
    //     and every claim was ingested under an audited record. A claim that
    //     appeared without an `epistemic.claim.ingest` record appeared without the
    //     log knowing, which is exactly what "no silent promotion" forbids one
    //     layer up.
    let mut c = Check::new("epistemic correlation");
    let claim_ids: std::collections::BTreeSet<String> = index
        .query_json("SELECT id FROM claims", mm_core::Params::new())
        .await?
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_string))
        .collect();
    let assumption_ids: std::collections::BTreeSet<String> = index
        .query_json("SELECT id FROM assumptions", mm_core::Params::new())
        .await?
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_string))
        .collect();
    let contradiction_ids: std::collections::BTreeSet<String> = index
        .query_json("SELECT id FROM contradictions", mm_core::Params::new())
        .await?
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_string))
        .collect();
    let transition_ids: std::collections::BTreeSet<String> = index
        .query_json(
            "SELECT id FROM epistemic_transitions",
            mm_core::Params::new(),
        )
        .await?
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_string))
        .collect();
    let mut epistemic_ops = 0usize;
    let mut correlated_epistemic =
        |code: &str, ids: &std::collections::BTreeSet<String>, table: &str, check: &mut Check| {
            for row in &audit_rows {
                if row.event_code != code {
                    continue;
                }
                epistemic_ops += 1;
                match row.trace_id.as_deref() {
                    Some(trace) if ids.contains(trace) => {}
                    Some(trace) => check.fail(format!("{code} {trace} has no {table} row")),
                    None => check.fail(format!("{code} audit record has no trace id")),
                }
            }
        };
    correlated_epistemic(codes::EPISTEMIC_CLAIM_INGEST, &claim_ids, "claims", &mut c);
    correlated_epistemic(
        codes::EPISTEMIC_EVIDENCE_ATTACH,
        &claim_ids,
        "claims",
        &mut c,
    );
    correlated_epistemic(codes::EPISTEMIC_WORLD_WRITE, &claim_ids, "claims", &mut c);
    correlated_epistemic(codes::EPISTEMIC_CASCADE, &claim_ids, "claims", &mut c);
    correlated_epistemic(
        codes::EPISTEMIC_STATUS_TRANSITION,
        &transition_ids,
        "epistemic_transitions",
        &mut c,
    );
    correlated_epistemic(
        codes::EPISTEMIC_ASSUMPTION_CREATE,
        &assumption_ids,
        "assumptions",
        &mut c,
    );
    correlated_epistemic(
        codes::EPISTEMIC_CONTRADICTION_DETECT,
        &contradiction_ids,
        "contradictions",
        &mut c,
    );
    let ingests: std::collections::BTreeSet<&str> = audit_rows
        .iter()
        .filter(|row| row.event_code == codes::EPISTEMIC_CLAIM_INGEST)
        .filter_map(|row| row.trace_id.as_deref())
        .collect();
    for id in &claim_ids {
        if !ingests.contains(id.as_str()) {
            c.fail(format!("claim {id} has no audited ingest record"));
        }
    }
    checks.push((
        c,
        format!(
            "{epistemic_ops} epistemic op(s), {} claim(s)",
            claim_ids.len()
        ),
    ));

    // --- report ------------------------------------------------------------
    let mut failed = 0;
    println!("logs verify");
    for (check, detail) in &checks {
        println!(
            "  [{}] {:<20} {}",
            if check.ok { "ok" } else { "FAIL" },
            check.name,
            if check.detail.is_empty() {
                detail.clone()
            } else {
                format!("{detail} — {}", check.detail)
            }
        );
        if !check.ok {
            failed += 1;
        }
    }
    let counts = kernel.logger.counts();
    println!(
        "  logger: {} emitted, {} filtered, {} sink error(s), {} redaction(s)",
        counts.emitted, counts.filtered, counts.sink_errors, counts.redactions
    );
    if counts.sink_errors > 0 {
        failed += 1;
        println!(
            "  [FAIL] logger reported sink errors: {:?}",
            kernel.logger.sink_failures()
        );
    }
    if failed > 0 {
        println!("logs verify FAILED ({failed} check(s))");
        return Ok(ExitCode::from(1));
    }
    println!("logs verify passed");
    Ok(ExitCode::SUCCESS)
}

/// Print the last `n` record lines.
pub async fn tail(cfg: mm_core::Config, n: usize) -> Result<ExitCode, MmError> {
    let lines = read_lines(&cfg.jsonl_path())?;
    let start = lines.len().saturating_sub(n);
    for line in &lines[start..] {
        println!("{line}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Print every record correlated with `trace_id`, plus matching events and audit
/// records.
pub async fn trace(cfg: mm_core::Config, id: &str) -> Result<ExitCode, MmError> {
    let wanted = mm_core::id::parse_ulid(id)?;
    let wanted = mm_core::ulid_string(&wanted);
    let kernel = Kernel::open(cfg, false).await?;

    let mut found = 0usize;
    for line in read_lines(&kernel.cfg.jsonl_path())? {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value.get("trace_id").and_then(|v| v.as_str()) == Some(wanted.as_str()) {
                println!("log      {line}");
                found += 1;
            }
        }
    }
    for event in kernel.events.all_records().await? {
        if event
            .correlation
            .map(|c| mm_core::ulid_string(&c))
            .as_deref()
            == Some(wanted.as_str())
        {
            println!(
                "event    seq {} {} {}",
                event.seq,
                event.kind.as_str(),
                event.status.as_str()
            );
            found += 1;
        }
    }
    for row in kernel.sqlite.audit_rows().await? {
        if row.trace_id.as_deref() == Some(wanted.as_str()) {
            println!("audit    seq {} {}", row.seq, row.event_code);
            found += 1;
        }
    }
    println!("{found} record(s) for trace {wanted}");
    Ok(if found == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn read_lines(path: &std::path::Path) -> Result<Vec<String>, MmError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(MmError::Store(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
    }
}

/// A record is schema-valid when every required field is present and well formed.
fn validate_record(line: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    let record: LogRecord =
        serde_json::from_value(value).map_err(|e| format!("not a LogRecord: {e}"))?;
    if record.record_id.len() != mm_core::ULID_LEN {
        return Err(format!("record_id must be {} chars", mm_core::ULID_LEN));
    }
    if record.record_id.chars().any(|c| c.is_ascii_uppercase()) {
        return Err("record_id must be lowercase Crockford".into());
    }
    if record.event_code.is_empty() {
        return Err("event_code must not be empty".into());
    }
    if record.target.is_empty() {
        return Err("target must not be empty".into());
    }
    if !record.fields.is_object() {
        return Err("fields must be a JSON object".into());
    }
    if record.at.seconds == 0 && record.at.nanos == 0 {
        return Err("at must be a real instant".into());
    }
    if let Some(trace) = &record.trace_id {
        if trace.len() != mm_core::ULID_LEN {
            return Err("trace_id must be a ULID".into());
        }
    }
    Ok(())
}
