//! The idempotency guard: exactly-once effects across retries.
//!
//! A retry is not an edge case in this system, it is the normal case: a call that timed
//! out may or may not have applied its effect, a process that crashed after writing a
//! file may not have recorded that it did, and a reasoning loop that lost its
//! connection will re-propose the same action. The journal is the answer, and it is the
//! same shape Temporal and Restate use: a key is claimed *before* the effect, the
//! outcome is recorded *after* it, and a repeat with the same key replays the recorded
//! outcome instead of re-applying the effect.
//!
//! # The two states, and why there are only two
//!
//! `in_flight` is claimed by [`IdempotencyStore::begin`] and `done` is set by
//! [`IdempotencyStore::finish`]. A third state ("failed") is deliberately absent: a
//! failure is an outcome like any other, and it is recorded through `finish` with an
//! [`ActionStatus::Error`](crate::action::ActionStatus::Error) result, so a retry after
//! a failure returns the recorded failure rather than re-running the tool. That is the
//! property the phase's gate checks — repeating an executed action does not re-apply
//! its effect — and it holds for failures too, because the *effect* is what must not
//! happen twice, not the attempt.
//!
//! # Claim, then work, then record
//!
//! [`IdempotencyStore::begin`] inserts with `ON CONFLICT(key) DO NOTHING` and then
//! reads, inside one transaction, so two concurrent callers cannot both be told they
//! are first: exactly one sees [`Begin::Fresh`] and the other sees
//! [`Begin::InFlight`] and is refused rather than racing the first.
//!
//! # The key is the caller's promise
//!
//! Nothing here verifies that two calls with one key are *really* the same call. That
//! is the caller's contract, and the executor gives a caller that supplies no key a
//! derived one ([`crate::action::ActionSpec::derived_idempotency_key`]) so a verbatim
//! retry is still deduplicated without anyone having to remember.

use mm_core::{Timestamp, Ulid};
use mm_store_sqlite::SqliteStore;

use crate::action::ActionResult;
use crate::error::DupError;

/// The state a key is in while its effect is being applied.
pub const STATE_IN_FLIGHT: &str = "in_flight";
/// The state a key is in once its outcome is recorded.
pub const STATE_DONE: &str = "done";

/// The key a retry repeats.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct IdempotencyKey(pub String);

impl IdempotencyKey {
    /// A key, refused when it is empty or whitespace.
    ///
    /// An empty key is refused because it is the one value that would make two
    /// unrelated actions deduplicate against each other — the failure mode is not "no
    /// deduplication" but "the wrong call's result", which is worse than either.
    pub fn new(text: impl Into<String>) -> Result<Self, DupError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(DupError::Store(
                "an idempotency key cannot be empty: it would deduplicate unrelated actions"
                    .to_string(),
            ));
        }
        Ok(IdempotencyKey(text))
    }

    /// The key as it is stored.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The answer to "may this call run?".
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Begin {
    /// Nobody has claimed the key: run the action.
    Fresh,
    /// An earlier call recorded an outcome under this key: report it, do not run.
    Replay(Box<ActionResult>),
}

impl Begin {
    /// True when the caller must report the recorded outcome instead of running.
    pub fn is_replay(&self) -> bool {
        matches!(self, Begin::Replay(_))
    }

    /// The recorded outcome, when there is one.
    pub fn recorded(&self) -> Option<&ActionResult> {
        match self {
            Begin::Replay(result) => Some(result),
            Begin::Fresh => None,
        }
    }

    /// Take the recorded outcome.
    pub fn into_recorded(self) -> Option<ActionResult> {
        match self {
            Begin::Replay(result) => Some(*result),
            Begin::Fresh => None,
        }
    }

    /// The `original_status` the `tool.idempotency.hit` record carries.
    pub fn recorded_status(&self) -> Option<&'static str> {
        self.recorded().map(|result| result.status.as_str())
    }
}

/// The journal, over `idempotency_keys`.
#[derive(Clone, Debug)]
pub struct IdempotencyStore {
    sql: SqliteStore,
}

impl IdempotencyStore {
    /// A store over an open SQLite handle.
    pub fn new(sql: SqliteStore) -> Self {
        IdempotencyStore { sql }
    }

    /// The underlying handle.
    pub fn sql(&self) -> &SqliteStore {
        &self.sql
    }

    /// Claim the key, or report what an earlier call recorded.
    ///
    /// One transaction and one writer at a time: the insert-or-do-nothing and the read
    /// that follows it are inside the same critical section, so the answer cannot
    /// change between the two.
    pub async fn begin(&self, key: &IdempotencyKey, action_id: Ulid) -> Result<Begin, DupError> {
        if key.as_str().trim().is_empty() {
            return Err(DupError::Store(
                "an idempotency key cannot be empty".to_string(),
            ));
        }
        let now = Timestamp::now().to_rfc3339();
        let _guard = self.sql.audit_lock().lock().await;
        let mut tx = self.sql.pool().begin().await.map_err(store_err)?;

        let inserted = sqlx::query(
            "INSERT INTO idempotency_keys (key, action_id, state, result_json, created_at, \
             updated_at) VALUES (?, ?, ?, NULL, ?, ?) ON CONFLICT(key) DO NOTHING",
        )
        .bind(key.as_str())
        .bind(mm_core::ulid_string(&action_id))
        .bind(STATE_IN_FLIGHT)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(store_err)?;

        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT state, result_json FROM idempotency_keys WHERE key = ?")
                .bind(key.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_err)?;
        // Commit before deciding: the claim has to be durable for the *other* caller to
        // see it, and a claim that rolled back on the way out of `begin` would let two
        // callers run the same action.
        tx.commit().await.map_err(store_err)?;

        if inserted.rows_affected() == 1 {
            return Ok(Begin::Fresh);
        }
        match row {
            Some((state, Some(json))) if state == STATE_DONE => {
                let result: ActionResult = serde_json::from_str(&json).map_err(|e| {
                    DupError::Store(format!(
                        "the recorded result for key {} is not an ActionResult: {e}",
                        key.as_str()
                    ))
                })?;
                Ok(Begin::Replay(Box::new(result)))
            }
            Some((state, _)) if state == STATE_IN_FLIGHT => Err(DupError::InFlight(key.0.clone())),
            Some((state, _)) => Err(DupError::Store(format!(
                "key {} is in the unknown state {state:?}",
                key.as_str()
            ))),
            None => Err(DupError::Store(format!(
                "key {} was claimed and then could not be read back",
                key.as_str()
            ))),
        }
    }

    /// Record the outcome.
    ///
    /// Rewriting an already-`done` key with the same outcome is allowed, because the
    /// case this journal exists for is a crash *between* the effect and the record: a
    /// caller that retries `finish` after such a crash is doing the right thing, and
    /// refusing it would leave the action recorded as permanently in flight.
    pub async fn finish(
        &self,
        key: &IdempotencyKey,
        result: &ActionResult,
    ) -> Result<(), DupError> {
        let json = serde_json::to_string(result)
            .map_err(|e| DupError::Store(format!("could not encode the result: {e}")))?;
        let now = Timestamp::now().to_rfc3339();
        let affected = sqlx::query(
            "UPDATE idempotency_keys SET state = ?, result_json = ?, updated_at = ? WHERE key = ?",
        )
        .bind(STATE_DONE)
        .bind(&json)
        .bind(&now)
        .bind(key.as_str())
        .execute(self.sql.pool())
        .await
        .map_err(store_err)?
        .rows_affected();
        if affected == 0 {
            return Err(DupError::Store(format!(
                "no call is recorded under key {}, so there is nothing to finish",
                key.as_str()
            )));
        }
        Ok(())
    }

    /// The state the key is in, when it is recorded at all.
    pub async fn state(&self, key: &IdempotencyKey) -> Result<Option<String>, DupError> {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM idempotency_keys WHERE key = ?")
                .bind(key.as_str())
                .fetch_optional(self.sql.pool())
                .await
                .map_err(store_err)?;
        Ok(state)
    }

    /// The action a key was claimed for.
    pub async fn action_for(&self, key: &IdempotencyKey) -> Result<Option<Ulid>, DupError> {
        let action: Option<String> =
            sqlx::query_scalar("SELECT action_id FROM idempotency_keys WHERE key = ?")
                .bind(key.as_str())
                .fetch_optional(self.sql.pool())
                .await
                .map_err(store_err)?;
        match action {
            Some(text) => mm_core::id::parse_ulid(&text).map(Some).map_err(|e| {
                DupError::Store(format!("key {} names a bad action id: {e}", key.as_str()))
            }),
            None => Ok(None),
        }
    }
}

fn store_err(e: sqlx::Error) -> DupError {
    DupError::Store(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionStatus;

    /// An open, migrated store in a temporary directory; leaked so the pool outlives it.
    async fn store() -> IdempotencyStore {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let sql = SqliteStore::open(&dir.path().join("keys.db"))
            .await
            .unwrap();
        sql.migrate().await.unwrap();
        IdempotencyStore::new(sql)
    }

    fn action(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn key(text: &str) -> IdempotencyKey {
        IdempotencyKey::new(text).unwrap()
    }

    #[tokio::test]
    async fn the_first_begin_is_fresh_and_the_second_replays_the_recorded_outcome() {
        let store = store().await;
        let k = key("k1");
        assert_eq!(store.begin(&k, action(1)).await.unwrap(), Begin::Fresh);

        let mut result = ActionResult::ok(action(1), serde_json::json!({ "bytes_written": 5 }));
        result.observation_id = Some(action(2));
        result.latency_ms = 7;
        store.finish(&k, &result).await.unwrap();
        assert_eq!(store.state(&k).await.unwrap().as_deref(), Some(STATE_DONE));

        let replay = store.begin(&k, action(9)).await.unwrap();
        assert!(replay.is_replay());
        assert_eq!(replay.recorded(), Some(&result));
        assert_eq!(replay.recorded_status(), Some("ok"));
        assert_eq!(
            replay.into_recorded().unwrap().output.unwrap()["bytes_written"],
            5
        );
    }

    #[tokio::test]
    async fn a_second_begin_while_the_first_is_in_flight_is_refused() {
        let store = store().await;
        let k = key("k2");
        assert_eq!(store.begin(&k, action(1)).await.unwrap(), Begin::Fresh);
        let error = store.begin(&k, action(1)).await.unwrap_err();
        assert_eq!(error.code(), "idempotency.in_flight");
        assert!(error.to_string().contains("k2"));
        assert_eq!(
            store.state(&k).await.unwrap().as_deref(),
            Some(STATE_IN_FLIGHT)
        );
    }

    #[tokio::test]
    async fn two_concurrent_begins_cannot_both_be_fresh() {
        let store = store().await;
        let one = store.clone();
        let two = store.clone();
        let (a, b) = tokio::join!(
            async move { one.begin(&key("k3"), action(1)).await },
            async move { two.begin(&key("k3"), action(1)).await },
        );
        let fresh = [&a, &b]
            .iter()
            .filter(|r| matches!(r, Ok(Begin::Fresh)))
            .count();
        let in_flight = [&a, &b]
            .iter()
            .filter(|r| matches!(r, Err(DupError::InFlight(_))))
            .count();
        assert_eq!(fresh, 1, "exactly one caller claims the key: {a:?} {b:?}");
        assert_eq!(
            in_flight, 1,
            "the other is refused rather than racing: {a:?} {b:?}"
        );
    }

    #[tokio::test]
    async fn a_failure_is_recorded_and_replayed_like_any_other_outcome() {
        let store = store().await;
        let k = key("k4");
        store.begin(&k, action(1)).await.unwrap();
        let failed = ActionResult::error(action(1), "the tool could not reach the store");
        store.finish(&k, &failed).await.unwrap();
        let replay = store.begin(&k, action(1)).await.unwrap();
        assert_eq!(replay.recorded().unwrap().status, ActionStatus::Error);
        assert!(replay
            .recorded()
            .unwrap()
            .reason
            .as_deref()
            .unwrap()
            .contains("could not reach"));
        // The retry does not re-run: it reports the recorded failure.
        assert!(replay.is_replay());
    }

    #[tokio::test]
    async fn a_denial_is_recorded_too() {
        let store = store().await;
        let k = key("k5");
        store.begin(&k, action(1)).await.unwrap();
        store
            .finish(&k, &ActionResult::denied(action(1), "no grant covers it"))
            .await
            .unwrap();
        assert_eq!(
            store
                .begin(&k, action(1))
                .await
                .unwrap()
                .recorded()
                .unwrap()
                .status,
            ActionStatus::Denied
        );
    }

    #[tokio::test]
    async fn finishing_a_key_that_was_never_claimed_is_refused() {
        let store = store().await;
        let error = store
            .finish(
                &key("unknown"),
                &ActionResult::ok(action(1), serde_json::json!({})),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "idempotency.store");
        assert!(error.to_string().contains("nothing to finish"));
    }

    #[tokio::test]
    async fn finishing_twice_is_allowed_and_keeps_the_same_outcome() {
        let store = store().await;
        let k = key("k6");
        store.begin(&k, action(1)).await.unwrap();
        let result = ActionResult::ok(action(1), serde_json::json!({ "n": 1 }));
        store.finish(&k, &result).await.unwrap();
        store.finish(&k, &result).await.unwrap();
        assert_eq!(
            store.begin(&k, action(1)).await.unwrap().recorded(),
            Some(&result)
        );
    }

    #[tokio::test]
    async fn an_empty_key_is_refused() {
        let error = IdempotencyKey::new("   ").unwrap_err();
        assert_eq!(error.code(), "idempotency.store");
        assert!(error.to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn distinct_keys_are_independent() {
        let store = store().await;
        let one = key("k7");
        let two = key("k8");
        store.begin(&one, action(1)).await.unwrap();
        store.begin(&two, action(2)).await.unwrap();
        store
            .finish(
                &one,
                &ActionResult::ok(action(1), serde_json::json!({ "which": 1 })),
            )
            .await
            .unwrap();
        assert_eq!(
            store.state(&one).await.unwrap().as_deref(),
            Some(STATE_DONE)
        );
        assert_eq!(
            store.state(&two).await.unwrap().as_deref(),
            Some(STATE_IN_FLIGHT)
        );
        assert_eq!(store.action_for(&one).await.unwrap(), Some(action(1)));
        assert_eq!(store.action_for(&two).await.unwrap(), Some(action(2)));
        assert_eq!(store.action_for(&key("nothing")).await.unwrap(), None);
        assert_eq!(store.state(&key("nothing")).await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_automatic_key_a_verbatim_retry_derives_is_stable() {
        use crate::permissions::Principal;
        use crate::spec::ToolName;
        let make = || {
            crate::action::ActionSpec::new(
                ToolName::new("fs.write").unwrap(),
                serde_json::json!({ "path": "data/sandbox/target.txt", "text": "hello" }),
                Principal::system(),
            )
            .derived_idempotency_key()
        };
        assert_eq!(make(), make());
        let store = store().await;
        let derived = make();
        store.begin(&derived, action(1)).await.unwrap();
        store
            .finish(
                &derived,
                &ActionResult::ok(action(1), serde_json::json!({})),
            )
            .await
            .unwrap();
        assert!(store.begin(&make(), action(2)).await.unwrap().is_replay());
    }

    #[tokio::test]
    async fn begin_round_trips_through_json() {
        let begin = Begin::Replay(Box::new(ActionResult::ok(
            action(1),
            serde_json::json!({ "n": 1 }),
        )));
        let text = serde_json::to_string(&begin).unwrap();
        let back: Begin = serde_json::from_str(&text).unwrap();
        assert_eq!(begin, back);
        assert!(Begin::Fresh.recorded().is_none());
        assert_eq!(Begin::Fresh.recorded_status(), None);
    }
}
