//! `mm-log` — Metamind structured logging.
//!
//! Every record is one JSON object on one line with a shared set of fields, a
//! stable event code, and a `trace_id` that ties it to the event log and to RDF
//! provenance. Records marked [`LogRecord::audited`] are additionally written to
//! the immutable audit chain *before* the caller is allowed to consider its
//! mutation successful: no committed state may exist without its audit record.
#![forbid(unsafe_code)]

pub mod codes;
pub mod record;
pub mod redact;
pub mod sinks;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mm_core::config::LogConfig;
use mm_core::{AuditRecord, AuditWriter, MmError, Timestamp, Ulid, UlidFactory};
use serde::Serialize;

pub use record::{Level, LogRecord};
pub use redact::{RedactionPolicy, REDACTED};
pub use sinks::{CollectSink, ConsoleSink, JsonlSink, Sink};

/// Counters describing what this logger has seen.
///
/// `doctor` and `logs verify` report them, so a dropped record is visible rather
/// than assumed away.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    /// Records accepted, whether or not they passed the level filter.
    pub emitted: u64,
    /// Records dropped by the level filter.
    pub filtered: u64,
    /// Sink write failures.
    pub sink_errors: u64,
    /// Values replaced by the redaction policy.
    pub redactions: u64,
    /// Accepted records per level.
    pub by_level: BTreeMap<String, u64>,
    /// Accepted records per event code.
    pub by_code: BTreeMap<String, u64>,
}

/// The kernel logger.
pub struct Logger {
    min_level: Level,
    sinks: Vec<Box<dyn Sink>>,
    audit: Option<Arc<dyn AuditWriter>>,
    redaction: RedactionPolicy,
    ids: UlidFactory,
    counts: Mutex<Counts>,
    sink_failures: Mutex<Vec<String>>,
}

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Logger")
            .field("min_level", &self.min_level)
            .field("sinks", &self.sink_names())
            .field("has_audit_writer", &self.audit.is_some())
            .finish_non_exhaustive()
    }
}

impl Logger {
    /// Build a logger from explicit parts.
    pub fn new(
        min_level: Level,
        sinks: Vec<Box<dyn Sink>>,
        audit: Option<Arc<dyn AuditWriter>>,
        redaction: RedactionPolicy,
    ) -> Self {
        Logger {
            min_level,
            sinks,
            audit,
            redaction,
            ids: UlidFactory::new(),
            counts: Mutex::new(Counts::default()),
            sink_failures: Mutex::new(Vec::new()),
        }
    }

    /// Build the kernel's sink set from configuration.
    pub fn from_config(
        cfg: &LogConfig,
        audit: Option<Arc<dyn AuditWriter>>,
    ) -> Result<Self, MmError> {
        let level = Level::parse(&cfg.level).ok_or_else(|| {
            MmError::Config(format!(
                "log.level must be one of error|warn|info|debug|trace, got {:?}",
                cfg.level
            ))
        })?;
        let mut sinks: Vec<Box<dyn Sink>> = Vec::new();
        if cfg.console {
            sinks.push(Box::new(ConsoleSink));
        }
        if cfg.jsonl {
            sinks.push(Box::new(JsonlSink::open(&cfg.dir.join("mm.jsonl"))?));
        }
        Ok(Logger::new(
            level,
            sinks,
            audit,
            RedactionPolicy::kernel_default(),
        ))
    }

    /// Replace the redaction policy.
    pub fn with_redaction(mut self, policy: RedactionPolicy) -> Self {
        self.redaction = policy;
        self
    }

    /// Add a sink.
    pub fn with_sink(mut self, sink: Box<dyn Sink>) -> Self {
        self.sinks.push(sink);
        self
    }

    /// The configured minimum level.
    pub fn min_level(&self) -> Level {
        self.min_level
    }

    /// The names of the configured sinks.
    pub fn sink_names(&self) -> Vec<String> {
        self.sinks.iter().map(|s| s.name().to_string()).collect()
    }

    /// A snapshot of the counters.
    pub fn counts(&self) -> Counts {
        self.counts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Sinks that have failed at least once, as `name: error`.
    pub fn sink_failures(&self) -> Vec<String> {
        self.sink_failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Emit a record.
    ///
    /// A failing sink is counted and reported but never fatal: losing a console
    /// line must not abort a kernel operation. A failing *audit* write is fatal,
    /// because the mutation the record attests to must then not be treated as
    /// committed.
    pub async fn emit(&self, record: LogRecord) -> Result<(), MmError> {
        self.bump_counters(&record.level, &record.event_code);
        // A record below the configured level is counted and dropped. `Level`
        // orders by severity, so "below" is the less severe direction.
        if record.level < self.min_level {
            self.counts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .filtered += 1;
            return Ok(());
        }

        let mut record = record;
        let id = self.ids.next();
        record.record_id = mm_core::ulid_string(&id);
        let redactions = self.redaction.apply_to_value(&mut record.fields);
        record.redactions = redactions;
        if redactions > 0 {
            self.counts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .redactions += u64::from(redactions);
        }
        if !record.fields.is_object() {
            record.fields = serde_json::Value::Object(serde_json::Map::new());
        }
        if record.at == Timestamp::EPOCH {
            record.at = Timestamp::now();
        }

        let line = record.to_json();
        let mut failures = Vec::new();
        for sink in &self.sinks {
            if let Err(e) = sink.write_line(&line) {
                failures.push(format!("{}: {e}", sink.name()));
            }
        }
        if !failures.is_empty() {
            self.counts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sink_errors += failures.len() as u64;
            self.sink_failures
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(failures.iter().cloned());
            for f in &failures {
                eprintln!("mm-log: sink failure ({f})");
            }
        }

        if record.audit {
            let writer = self.audit.as_ref().ok_or_else(|| {
                MmError::Internal(format!(
                    "record {} is audited but no audit writer is configured",
                    record.event_code
                ))
            })?;
            let audit = AuditRecord {
                record_id: id,
                event_code: record.event_code.clone(),
                level: record.level.as_str().to_string(),
                target: record.target.clone(),
                trace_id: record
                    .trace_id
                    .as_deref()
                    .and_then(|t| mm_core::id::parse_ulid(t).ok()),
                payload: record.fields.clone(),
                at: record.at,
            };
            writer.write_audit(&audit).await?;
        }
        Ok(())
    }

    /// Emit a record with no body.
    pub async fn log(&self, level: Level, event_code: &str, target: &str) -> Result<(), MmError> {
        self.emit(LogRecord::new(level, event_code, target)).await
    }

    /// Emit an audited record: this only returns once the audit chain has
    /// accepted it.
    pub async fn audit(
        &self,
        level: Level,
        event_code: &str,
        target: &str,
        trace_id: Option<Ulid>,
        fields: serde_json::Value,
    ) -> Result<(), MmError> {
        let mut record = LogRecord::new(level, event_code, target).audited();
        if let Some(t) = trace_id {
            record = record.with_trace(t);
        }
        record.fields = fields;
        self.emit(record).await
    }

    fn bump_counters(&self, level: &Level, code: &str) {
        let mut c = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        c.emitted += 1;
        *c.by_level.entry(level.as_str().to_string()).or_insert(0) += 1;
        *c.by_code.entry(code.to_string()).or_insert(0) += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Default)]
    struct CountingAudit {
        writes: AtomicU64,
    }

    #[async_trait::async_trait]
    impl AuditWriter for CountingAudit {
        async fn write_audit(&self, _record: &AuditRecord) -> Result<(), MmError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A `Sink` that writes into a shared [`CollectSink`] so the test can read
    /// back exactly what was emitted.
    struct SharedSink(Arc<CollectSink>);

    impl Sink for SharedSink {
        fn name(&self) -> &str {
            "collect"
        }
        fn write_line(&self, line: &str) -> Result<(), MmError> {
            self.0.write_line(line)
        }
    }

    fn logger_with(sink: Arc<CollectSink>, audit: Option<Arc<dyn AuditWriter>>) -> Logger {
        Logger::new(
            Level::Info,
            vec![Box::new(SharedSink(sink))],
            audit,
            RedactionPolicy::kernel_default(),
        )
    }

    #[tokio::test]
    async fn counters_show_filtering_and_redaction() {
        let sink = Arc::new(CollectSink::new());
        let logger = logger_with(sink.clone(), None);

        logger
            .log(Level::Debug, codes::STORE_GRAPH_SPARQL, "mm.graph")
            .await
            .unwrap();
        logger
            .emit(
                LogRecord::new(Level::Info, codes::KERNEL_BOOT, "mm.cli")
                    .with_field("api_key", "sk-secret"),
            )
            .await
            .unwrap();

        let counts = logger.counts();
        assert_eq!(counts.filtered, 1);
        assert_eq!(counts.emitted, 2);
        assert_eq!(counts.redactions, 1);
        assert_eq!(counts.by_level.get("info"), Some(&1));
        assert_eq!(counts.by_code.get(codes::KERNEL_BOOT), Some(&1));

        let lines = sink.lines();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains("sk-secret"));
        assert!(lines[0].contains(REDACTED));
    }

    #[tokio::test]
    async fn audited_records_require_and_reach_the_audit_writer() {
        let sink = Arc::new(CollectSink::new());
        let audit = Arc::new(CountingAudit::default());

        let without = logger_with(sink.clone(), None);
        let err = without
            .audit(
                Level::Info,
                codes::KERNEL_BOOT,
                "mm.cli",
                None,
                serde_json::json!({}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, MmError::Internal(_)), "got {err:?}");

        let with = logger_with(sink, Some(audit.clone()));
        with.audit(
            Level::Info,
            codes::KERNEL_BOOT,
            "mm.cli",
            None,
            serde_json::json!({"stores_ready": true}),
        )
        .await
        .unwrap();
        assert_eq!(audit.writes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn every_record_gets_a_lowercase_ulid_and_a_timestamp() {
        let sink = Arc::new(CollectSink::new());
        let logger = logger_with(sink.clone(), None);
        logger
            .log(Level::Info, codes::LOG_INIT, "mm.log")
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&sink.lines()[0]).unwrap();
        let id = v["record_id"].as_str().unwrap();
        assert_eq!(id.len(), mm_core::ULID_LEN);
        assert!(!id.chars().any(|c| c.is_ascii_uppercase()));
        assert!(v["at"].as_str().unwrap().ends_with('Z'));
    }

    #[tokio::test]
    async fn trace_ids_correlate_records() {
        let sink = Arc::new(CollectSink::new());
        let logger = logger_with(sink.clone(), None);
        let trace = mm_core::UlidFactory::new().next();
        logger
            .emit(
                LogRecord::new(Level::Info, codes::EVENTLOG_COMMIT, "mm.eventlog")
                    .with_trace(trace),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&sink.lines()[0]).unwrap();
        assert_eq!(
            v["trace_id"].as_str().unwrap(),
            mm_core::ulid_string(&trace)
        );
    }
}
