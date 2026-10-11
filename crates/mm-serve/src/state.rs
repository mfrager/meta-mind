//! Server state: the kernel boot, the console write lock, and the health
//! report (plan `01` §5).
//!
//! The boot is deliberately the same ordered open `mm-cli`'s kernel performs
//! (`crates/mm-cli/src/kernel.rs`): configuration, then the tabular store and
//! its migrations, then the logger, then the event log — so a failure leaves
//! nothing half-built and the console never holds a store the CLI cannot
//! recognize. The graph store is opened on demand by later milestones (M3);
//! until then health reports it `unavailable` *with a reason*.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_core::{Config, MmError, UlidFactory};
use mm_eventlog::EventLog;
use mm_log::Logger;
use mm_store_sqlite::SqliteStore;

use crate::dto::{
    EventHead, HealthEnvelope, KernelStatus, ServiceState, ServiceStatus, WIRE_VERSION,
};
use crate::error::AppError;

/// Everything the request handlers share.
pub struct AppState {
    /// When the process started (epoch ms), for uptime in the status bar.
    pub started_at: i64,
    /// The resolved configuration (same resolution rules as the CLI).
    pub cfg: Config,
    /// The tabular store.
    pub sqlite: SqliteStore,
    /// The append-only event log.
    pub events: EventLog,
    /// The audit-chained logger.
    pub logger: Arc<Logger>,
    /// The monotonic identifier factory, shared with the event log.
    pub ids: Arc<UlidFactory>,
    /// Held for its `Drop`: releasing `data/console.lock` on shutdown.
    pub _lock: ConsoleLock,
}

impl AppState {
    /// Resolve configuration exactly like `mm-cli` (`--data-dir` wins over
    /// `--config`, which wins over the repository default).
    pub fn resolve_config(
        config: Option<PathBuf>,
        data_dir: Option<PathBuf>,
    ) -> Result<Config, MmError> {
        if let Some(dir) = data_dir {
            return Ok(Config::for_data_dir(dir));
        }
        match config {
            Some(path) => Config::load(&path),
            None => Config::load_default(),
        }
    }

    /// Boot the kernel: the ordered open above, plus a doctor-style preflight
    /// that proves the stores are writable before any route is served
    /// (plan `01` §5.1).
    pub async fn boot(cfg: Config, lock: ConsoleLock) -> Result<Self, MmError> {
        std::fs::create_dir_all(&cfg.store.data_dir)?;

        let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await?;
        sqlite.migrate().await?;

        let logger = Arc::new(Logger::from_config(
            &cfg.log,
            Some(Arc::new(sqlite.clone())),
        )?);

        let ids = Arc::new(UlidFactory::open(&cfg.ulid_watermark_path())?);
        let events = EventLog::new(sqlite.clone(), Arc::clone(&logger), Arc::clone(&ids));

        logger
            .emit(
                mm_log::LogRecord::new(mm_log::Level::Info, mm_log::codes::LOG_INIT, "mm.serve")
                    .with_field("sinks", logger.sink_names())
                    .with_field("level", logger.min_level().as_str())
                    .with_field("log_dir", cfg.log.dir.display().to_string()),
            )
            .await?;

        Ok(Self {
            started_at: now_ms(),
            cfg,
            sqlite,
            events,
            logger,
            ids,
            _lock: lock,
        })
    }

    /// The `GET /api/health` envelope (plan `09` §1).
    ///
    /// `ok` reports whether the server is serving; kernel capability problems
    /// are reported inside `kernel`, never by making `ok` lie (plan `00` §4.3).
    pub async fn health(&self) -> HealthEnvelope {
        let (event_seq, event_err) = match self.events.head_seq().await {
            Ok(seq) => (seq, None),
            Err(e) => (0, Some(e.to_string())),
        };
        let (audit_seq, audit_hash, audit_err) = match self.sqlite.audit_head().await {
            Ok(Some((seq, hash))) => (Some(seq), Some(hash), None),
            Ok(None) => (None, None, None),
            Err(e) => (None, None, Some(e.to_string())),
        };

        let mut services = vec![ServiceStatus {
            name: "sqlite".to_string(),
            state: ServiceState::Ok,
            reason: None,
        }];
        services.push(match event_err {
            None => ServiceStatus {
                name: "eventlog".to_string(),
                state: ServiceState::Ok,
                reason: None,
            },
            Some(reason) => ServiceStatus {
                name: "eventlog".to_string(),
                state: ServiceState::Degraded,
                reason: Some(reason),
            },
        });
        services.push(ServiceStatus {
            name: "graph".to_string(),
            state: ServiceState::Unavailable,
            reason: Some(
                "the RDF store is opened on demand and no route needs it yet; \
                 graph-backed documents arrive with the document pane (M3)"
                    .to_string(),
            ),
        });
        services.push(ServiceStatus {
            name: "llm".to_string(),
            state: ServiceState::Unavailable,
            reason: Some("the chat orchestrator arrives with the chat pane (M5)".to_string()),
        });

        if let Some(reason) = audit_err {
            services.push(ServiceStatus {
                name: "audit".to_string(),
                state: ServiceState::Degraded,
                reason: Some(reason),
            });
        }

        let all_ok = services
            .iter()
            .all(|s| s.state == ServiceState::Ok || s.name == "graph" || s.name == "llm");

        HealthEnvelope {
            ok: true,
            wire_version: WIRE_VERSION,
            kernel: KernelStatus {
                state: if all_ok {
                    ServiceState::Ok
                } else {
                    ServiceState::Degraded
                },
                reason: if all_ok {
                    None
                } else {
                    Some("see kernel.services for the failing component".to_string())
                },
                services,
            },
            data_dir: self.cfg.store.data_dir.display().to_string(),
            event_head: EventHead {
                seq: event_seq,
                audit_seq,
                audit_hash,
            },
        }
    }
}

/// `data/console.lock` — the single-writer guard (plan `01` §5.1).
///
/// While any `mm-serve` holds this file the data directory is owned by the
/// console; a second console (or a store-locking CLI command) gets a `locked`
/// error naming the holder. A stale lock (dead pid) is reclaimed on boot.
pub struct ConsoleLock {
    path: PathBuf,
}

impl ConsoleLock {
    /// Acquire the lock for `data_dir`, or explain who holds it.
    pub fn acquire(data_dir: &Path) -> Result<Self, AppError> {
        let path = data_dir.join("console.lock");
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write as _;
                    let _ = writeln!(f, "{} {}", std::process::id(), now_ms());
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let holder = std::fs::read_to_string(&path).unwrap_or_default();
                    if lock_holder_alive(&holder) {
                        return Err(AppError::Locked(format!(
                            "{} is held by {}; stop that console (or the CLI command) first",
                            path.display(),
                            holder.trim()
                        )));
                    }
                    // Stale lock from a dead process: reclaim and retry once.
                    let _ = std::fs::remove_file(&path);
                }
                Err(e) => {
                    return Err(AppError::Internal(format!(
                        "cannot create {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        Err(AppError::Locked(format!(
            "could not acquire {}",
            path.display()
        )))
    }
}

impl Drop for ConsoleLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether the pid recorded in a lock file is still running (Linux `/proc`).
fn lock_holder_alive(holder: &str) -> bool {
    let pid = holder
        .split_whitespace()
        .next()
        .and_then(|p| p.parse::<u32>().ok());
    match pid {
        Some(pid) if pid == std::process::id() => true,
        Some(pid) => Path::new(&format!("/proc/{pid}")).exists(),
        None => false,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_holder_blocks_and_a_dead_one_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("console.lock");

        // A lock naming a live process (this one) is honored.
        std::fs::write(&lock_path, format!("{} 0", std::process::id())).unwrap();
        let err = ConsoleLock::acquire(dir.path()).unwrap_err();
        assert!(matches!(err, AppError::Locked(_)), "{err:?}");

        // A lock naming a dead pid is reclaimed.
        std::fs::write(&lock_path, "999999999 0").unwrap();
        let _lock = ConsoleLock::acquire(dir.path()).unwrap();
        assert!(!lock_path.exists() == false, "lock file exists while held");
    }

    #[test]
    fn dropping_the_lock_releases_it() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("console.lock");
        {
            let _lock = ConsoleLock::acquire(dir.path()).unwrap();
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists(), "Drop must remove console.lock");
    }
}
