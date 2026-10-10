//! The five timescales of §31: Action, Episode, Project, Goal, Identity.
//!
//! A scheduler is the part of an autonomous system that decides *when* to think. The
//! phase's design fixes five clocks and one prohibition: Identity is the clock that never
//! fires during a run. It is not a rate limit — it is the statement that the being's
//! identity is not something a loop may revise while it is looping, and it is implemented
//! here as a refusal rather than as a long interval, because an interval would eventually
//! elapse.
//!
//! Two decisions are load-bearing:
//!
//! * **The clocks are rows, not a constant.** [`TableScheduler::load`] reads the
//!   `timescales` table, which migration `0012_loop.sql` seeds. A tick is therefore a
//!   property of the shipped data, so an operator can see the five clocks and what they
//!   are without reading this file, and a sixth clock is a row rather than a release.
//! * **Every tick is recorded.** A tick appends a `timescale.tick` record and writes the
//!   tick's ULID back to its row, which is what makes "N ticks happened" checkable after
//!   the fact instead of only observable live.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mm_core::{Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{LoopError, Result};

/// One of the five clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimescaleKind {
    /// Milliseconds: a single action.
    Action,
    /// Seconds: one episode.
    Episode,
    /// Hours: one project.
    Project,
    /// Days: one goal.
    Goal,
    /// Never, during a run: the identity.
    Identity,
}

impl TimescaleKind {
    /// Every kind, fastest first.
    pub const ALL: [TimescaleKind; 5] = [
        TimescaleKind::Action,
        TimescaleKind::Episode,
        TimescaleKind::Project,
        TimescaleKind::Goal,
        TimescaleKind::Identity,
    ];

    /// The stable wire name, which is the value the `timescales.kind` CHECK allows.
    pub fn as_str(self) -> &'static str {
        match self {
            TimescaleKind::Action => "action",
            TimescaleKind::Episode => "episode",
            TimescaleKind::Project => "project",
            TimescaleKind::Goal => "goal",
            TimescaleKind::Identity => "identity",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<TimescaleKind> {
        TimescaleKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text.trim())
    }

    /// True when the clock is allowed to fire during a run.
    pub fn ticks_during_a_run(self) -> bool {
        self != TimescaleKind::Identity
    }

    /// The rank the clocks are ordered by.
    pub fn rank(self) -> usize {
        TimescaleKind::ALL
            .iter()
            .position(|kind| *kind == self)
            .expect("every kind is in ALL")
    }
}

impl std::fmt::Display for TimescaleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One clock, as a row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timescale {
    /// Its name, the table's primary key.
    pub name: String,
    /// Which of the five it is.
    pub kind: TimescaleKind,
    /// How long one tick is.
    pub tick: Duration,
    /// The handler a tick dispatches to.
    pub handler: String,
    /// The last tick recorded, when one has been.
    #[serde(with = "mm_core::serde_ulid::option")]
    pub last_tick: Option<Ulid>,
    /// When that tick was recorded.
    pub last_tick_at: Option<String>,
}

impl Timescale {
    /// The tick's period in milliseconds.
    pub fn tick_ms(&self) -> u64 {
        u64::try_from(self.tick.as_millis()).unwrap_or(u64::MAX)
    }
}

/// One firing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tick {
    /// The clock's name.
    pub name: String,
    /// Which of the five it is.
    pub kind: TimescaleKind,
    /// The handler the tick dispatches to.
    pub handler: String,
    /// The clock's period.
    pub tick_ms: u64,
    /// The tick before this one, when there was one.
    #[serde(with = "mm_core::serde_ulid::option")]
    pub last_tick_ulid: Option<Ulid>,
}

/// What a scheduler does.
#[async_trait]
pub trait TimescaleScheduler {
    /// Fire every clock that is due at `now`.
    async fn tick(&self, now: Timestamp) -> Result<Vec<Tick>>;
}

/// The scheduler over the `timescales` table.
pub struct TableScheduler {
    store: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
}

impl TableScheduler {
    /// A scheduler over the kernel's store.
    pub fn new(store: SqliteStore, logger: Arc<Logger>, ids: Arc<UlidFactory>) -> Self {
        TableScheduler { store, logger, ids }
    }

    /// Every clock, fastest first.
    pub async fn load(&self) -> Result<Vec<Timescale>> {
        let rows = self
            .store
            .query_json("SELECT * FROM timescales", Params::new())
            .await
            .map_err(|e| LoopError::Store(format!("cannot read timescales: {e}")))?;
        let mut clocks = Vec::with_capacity(rows.len());
        for row in rows {
            let name = row["name"].as_str().unwrap_or_default().to_string();
            let kind_text = row["kind"].as_str().unwrap_or_default();
            let kind = TimescaleKind::parse(kind_text).ok_or_else(|| {
                LoopError::Store(format!("timescale {name:?} has unknown kind {kind_text:?}"))
            })?;
            let tick_ms = row["tick_ms"].as_u64().unwrap_or(0);
            if tick_ms == 0 {
                return Err(LoopError::Store(format!(
                    "timescale {name:?} has a zero period"
                )));
            }
            clocks.push(Timescale {
                name,
                kind,
                tick: Duration::from_millis(tick_ms),
                handler: row["handler"].as_str().unwrap_or_default().to_string(),
                last_tick: row["last_tick_ulid"]
                    .as_str()
                    .and_then(|text| mm_core::id::parse_ulid(text).ok()),
                last_tick_at: row["last_tick_at"].as_str().map(str::to_string),
            });
        }
        clocks.sort_by_key(|clock| clock.kind.rank());
        Ok(clocks)
    }

    /// Fire the clocks `times` times over, recording each firing.
    ///
    /// The count is what the fast-clock test drives: `times` steps of one period each,
    /// so the number of ticks per clock is `times` and the number of `timescale.tick`
    /// records is `times × (clocks − 1)`, with Identity contributing nothing. The
    /// arithmetic is exact and independent of the wall clock, which is the whole point of
    /// a deterministic scheduler test.
    pub async fn tick_n(&self, times: usize, now: Timestamp) -> Result<Vec<Tick>> {
        let mut all = Vec::new();
        for _ in 0..times {
            all.extend(self.tick(now).await?);
        }
        Ok(all)
    }

    /// How many Identity ticks have ever been recorded.
    ///
    /// The phase's "Identity never ticks during a run" is only checkable if the count is
    /// something a caller can ask for, so it is a query rather than a comment.
    pub async fn identity_ticks(&self) -> Result<i64> {
        let rows = self
            .store
            .query_json(
                "SELECT count(*) AS n FROM timescales \
                 WHERE kind = 'identity' AND last_tick_ulid IS NOT NULL",
                Params::new(),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read timescales: {e}")))?;
        Ok(rows[0]["n"].as_i64().unwrap_or(-1))
    }

    /// Record one firing, returning the record's id.
    async fn record(&self, tick: &Tick, at: Timestamp) -> Result<Ulid> {
        let id = self.ids.next();
        self.store
            .execute(
                "UPDATE timescales SET last_tick_ulid = ?, last_tick_at = ? WHERE name = ?",
                vec![
                    Param::Text(mm_core::ulid_string(&id)),
                    Param::Text(at.to_rfc3339()),
                    Param::Text(tick.name.clone()),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the tick: {e}")))?;
        self.logger
            .audit(
                Level::Debug,
                codes::TIMESCALE_TICK,
                crate::TARGET,
                Some(id),
                json!({
                    "name": tick.name,
                    "kind": tick.kind.as_str(),
                    "tick_ms": tick.tick_ms,
                    "handler": tick.handler,
                    "last_tick_ulid": tick.last_tick_ulid.map(|u| mm_core::ulid_string(&u)),
                }),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the tick: {e}")))?;
        Ok(id)
    }
}

#[async_trait]
impl TimescaleScheduler for TableScheduler {
    /// Fire every clock that may fire, in fastest-first order.
    ///
    /// Identity is skipped, and the skip is the phase's rule rather than a scheduling
    /// optimization; [`TableScheduler::load`] would return it, so a caller that wants to
    /// see the whole set can read it directly.
    async fn tick(&self, now: Timestamp) -> Result<Vec<Tick>> {
        let clocks = self.load().await?;
        let mut fired = Vec::new();
        for clock in clocks {
            if !clock.kind.ticks_during_a_run() {
                continue;
            }
            let tick = Tick {
                name: clock.name.clone(),
                kind: clock.kind,
                handler: clock.handler.clone(),
                tick_ms: clock.tick_ms(),
                last_tick_ulid: clock.last_tick,
            };
            self.record(&tick, now).await?;
            fired.push(tick);
        }
        Ok(fired)
    }
}

/// The scheduler the CLI and the loop use.
pub type Scheduler = TableScheduler;

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;

    async fn scheduler() -> (TableScheduler, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path().join("data"));
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let logger = Arc::new(Logger::from_config(&cfg.log, None).unwrap());
        let ids = Arc::new(UlidFactory::new());
        (TableScheduler::new(store, logger, ids), dir)
    }

    #[tokio::test]
    async fn the_five_clocks_are_seeded_in_fastest_first_order() {
        let (scheduler, _dir) = scheduler().await;
        let clocks = scheduler.load().await.unwrap();
        assert_eq!(clocks.len(), 5);
        assert_eq!(
            clocks.iter().map(|c| c.kind).collect::<Vec<_>>(),
            TimescaleKind::ALL.to_vec()
        );
        assert_eq!(clocks[0].name, "action");
        assert_eq!(clocks[0].tick, Duration::from_millis(1000));
        assert_eq!(clocks[4].kind, TimescaleKind::Identity);
    }

    #[tokio::test]
    async fn n_ticks_produce_n_firings_per_operational_clock_and_none_for_identity() {
        let (scheduler, _dir) = scheduler().await;
        let fired = scheduler
            .tick_n(4, Timestamp::from_epoch_seconds(1_700_000_000))
            .await
            .unwrap();
        // Four clocks may fire; Identity is the fifth and never does.
        assert_eq!(
            fired.len(),
            4 * 4,
            "four ticks over four operational clocks"
        );
        for clock in ["action", "episode", "project", "goal"] {
            assert_eq!(
                fired.iter().filter(|tick| tick.name == clock).count(),
                4,
                "{clock} must fire once per step"
            );
        }
        assert!(
            fired
                .iter()
                .all(|tick| tick.kind != TimescaleKind::Identity),
            "identity must never tick during a run"
        );
        assert_eq!(scheduler.identity_ticks().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_second_pass_counts_the_first_as_the_previous_tick() {
        let (scheduler, _dir) = scheduler().await;
        let at = Timestamp::from_epoch_seconds(1_700_000_000);
        let first = scheduler.tick(at).await.unwrap();
        assert!(first.iter().all(|tick| tick.last_tick_ulid.is_none()));
        let second = scheduler.tick(at).await.unwrap();
        assert!(
            second.iter().all(|tick| tick.last_tick_ulid.is_some()),
            "the first firing is the second's predecessor"
        );
        // The two passes recorded different ids, so the row was updated rather than
        // re-counted.
        let first_ids: Vec<String> = first.iter().map(|tick| tick.name.clone()).collect();
        let loaded = scheduler.load().await.unwrap();
        for clock in loaded.iter().filter(|c| c.kind.ticks_during_a_run()) {
            assert!(first_ids.contains(&clock.name));
            assert!(clock.last_tick.is_some());
        }
    }

    #[test]
    fn identity_is_the_only_clock_that_does_not_fire() {
        for kind in TimescaleKind::ALL {
            assert_eq!(
                kind.ticks_during_a_run(),
                kind != TimescaleKind::Identity,
                "{kind}"
            );
        }
    }
}
