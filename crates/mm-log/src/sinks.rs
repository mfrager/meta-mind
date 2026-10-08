//! Sinks.
//!
//! A sink receives an already-serialized line and must not modify it: what one
//! sink writes is what every sink writes, which is what makes a JSONL file and
//! the console comparable.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mm_core::MmError;

/// A destination for serialized records.
pub trait Sink: Send + Sync {
    /// A stable name used in `log.init`.
    fn name(&self) -> &str;
    /// Write one line (without a trailing newline).
    fn write_line(&self, line: &str) -> Result<(), MmError>;
}

/// Writes to stdout. Failures (for example a closed pipe) are reported to the
/// caller but never panic.
pub struct ConsoleSink;

impl Sink for ConsoleSink {
    fn name(&self) -> &str {
        "console"
    }

    fn write_line(&self, line: &str) -> Result<(), MmError> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").map_err(|e| MmError::Internal(format!("console sink: {e}")))
    }
}

/// Appends newline-delimited JSON to a file, opened once and held open.
pub struct JsonlSink {
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

impl JsonlSink {
    /// Open (creating if needed) an append-only JSONL file.
    pub fn open(path: &Path) -> Result<Self, MmError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(JsonlSink {
            path: path.to_path_buf(),
            file: Mutex::new(file),
        })
    }

    /// The file this sink writes.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Sink for JsonlSink {
    fn name(&self) -> &str {
        "jsonl"
    }

    fn write_line(&self, line: &str) -> Result<(), MmError> {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(file, "{line}").map_err(|e| MmError::Store(format!("jsonl sink: {e}")))
    }
}

/// Keeps records in memory. Used by tests and by `logs verify`, which must be
/// able to inspect exactly what was emitted.
#[derive(Default)]
pub struct CollectSink {
    lines: Mutex<Vec<String>>,
}

impl CollectSink {
    /// A new, empty collector.
    pub fn new() -> Self {
        CollectSink::default()
    }

    /// Everything written so far, in order.
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// How many records have been collected.
    pub fn len(&self) -> usize {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether nothing has been collected.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Sink for CollectSink {
    fn name(&self) -> &str {
        "collect"
    }

    fn write_line(&self, line: &str) -> Result<(), MmError> {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_sink_appends_without_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/mm.jsonl");
        {
            let sink = JsonlSink::open(&path).unwrap();
            sink.write_line(r#"{"a":1}"#).unwrap();
        }
        {
            let sink = JsonlSink::open(&path).unwrap();
            sink.write_line(r#"{"a":2}"#).unwrap();
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw, "{\"a\":1}\n{\"a\":2}\n");
        assert_eq!(JsonlSink::open(&path).unwrap().name(), "jsonl");
    }

    #[test]
    fn collect_sink_records_in_order() {
        let sink = CollectSink::new();
        assert!(sink.is_empty());
        sink.write_line("one").unwrap();
        sink.write_line("two").unwrap();
        assert_eq!(sink.lines(), vec!["one", "two"]);
        assert_eq!(sink.len(), 2);
    }
}
