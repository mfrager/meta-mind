//! The prediction ledger: a claim staked before the outcome, and its one resolution.
//!
//! The ledger is the *evidence* half of calibration. Three decisions shape it:
//!
//! * **Append-only, and the database enforces it.** `prediction_ledger` and
//!   `prediction_outcomes` carry `BEFORE UPDATE`/`BEFORE DELETE` triggers that abort,
//!   and this type exposes no update or delete path at all. A calibration computed
//!   over a ledger someone could rewrite after the fact scores nothing; the two
//!   layers are what make "the prediction preceded the outcome" a property of the
//!   schema rather than of the code that happens to be running.
//! * **A resolution happens once.** `prediction_outcomes` is keyed by the prediction,
//!   so a second `resolve` is refused both by this type ([`MetaError::AlreadyResolved`],
//!   which names what happened) and by the primary key underneath it.
//! * **Only resolved predictions are scoreable.** [`PredictionLedger::labeled`]
//!   inner-joins the outcomes: an unresolved prediction is an open claim, and a
//!   calibrator that treated it as a label would be scoring a guess against itself.
//!
//! Every prediction carries a **class**, which is the calibration subject. Class is
//! not derivable from the proposition: it is the grouping a score, a threshold and a
//! `calibration_runs.subject` are all computed for, and a class invented at read time
//! would make a stored run unreproducible.

use std::sync::Arc;
use std::time::Duration;

use mm_core::{Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::Logger;

use crate::calibration::Labeled;
use crate::error::{malformed, MetaError, Result};

/// One consequential prediction, staked at an instant and due after a horizon.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// The ledger row's identifier.
    pub id: Ulid,
    /// The calibration subject this prediction is scored under.
    pub class: String,
    /// The proposition, as a sentence a reader can judge true or false.
    pub proposition: String,
    /// The probability staked for it, in `[0,1]`.
    pub probability: f32,
    /// How far ahead it reaches. A prediction with no horizon cannot come due.
    pub horizon: Duration,
    /// The conditions it was made under.
    pub conditions: Vec<String>,
    /// The episode that produced it, when one did.
    pub episode: Option<Ulid>,
    /// When it was staked.
    pub created_at: Timestamp,
}

impl Prediction {
    /// A prediction with no conditions and no episode.
    pub fn new(
        id: Ulid,
        class: impl Into<String>,
        proposition: impl Into<String>,
        probability: f32,
        horizon: Duration,
    ) -> Self {
        Prediction {
            id,
            class: class.into(),
            proposition: proposition.into(),
            probability,
            horizon,
            conditions: Vec::new(),
            episode: None,
            created_at: Timestamp::now(),
        }
    }

    /// Record the conditions the prediction was made under.
    pub fn when(mut self, conditions: Vec<String>) -> Self {
        self.conditions = conditions;
        self
    }

    /// Attach the episode it came from.
    ///
    /// `with_episode`, not `from_episode`: the builders on `Prediction` all start
    /// from an already-staked prediction and return it, so nothing here is a
    /// constructor and the `from_*` spelling would promise one.
    pub fn with_episode(mut self, episode: Ulid) -> Self {
        self.episode = Some(episode);
        self
    }

    /// Stake it at a fixed instant, for a replay.
    pub fn at(mut self, at: Timestamp) -> Self {
        self.created_at = at;
        self
    }

    /// Refuse a prediction that cannot be scored.
    pub fn validate(&self) -> Result<()> {
        if self.proposition.trim().is_empty() {
            return Err(MetaError::validation("proposition", "must not be empty"));
        }
        if self.class.trim().is_empty() {
            return Err(MetaError::validation(
                "class",
                "must not be empty: a prediction with no subject is scored against nothing",
            ));
        }
        if !(0.0..=1.0).contains(&self.probability) || !self.probability.is_finite() {
            return Err(MetaError::validation(
                "probability",
                format!("must be in [0,1], got {}", self.probability),
            ));
        }
        if self.horizon.is_zero() {
            return Err(MetaError::validation(
                "horizon",
                "must be positive: a prediction due immediately cannot be staked before its outcome",
            ));
        }
        Ok(())
    }

    /// The horizon in whole seconds, as the column stores it.
    pub fn horizon_seconds(&self) -> i64 {
        i64::try_from(self.horizon.as_secs()).unwrap_or(i64::MAX)
    }
}

/// The resolution of a prediction.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictionOutcome {
    /// Whether the proposition came true.
    pub observed: bool,
    /// When the outcome was observed.
    pub resolved_at: Timestamp,
    /// The evidence the outcome stands on.
    pub evidence: Ulid,
}

impl PredictionOutcome {
    /// Resolve a prediction at a fixed instant.
    pub fn new(observed: bool, evidence: Ulid, resolved_at: Timestamp) -> Self {
        PredictionOutcome {
            observed,
            resolved_at,
            evidence,
        }
    }
}

/// The append-only ledger, over the kernel's tabular store.
pub struct PredictionLedger {
    store: mm_store_sqlite::SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
}

impl std::fmt::Debug for PredictionLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PredictionLedger")
            .field("path", &self.store.path())
            .finish_non_exhaustive()
    }
}

impl PredictionLedger {
    /// Build a ledger over an open store.
    pub fn new(
        store: mm_store_sqlite::SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
    ) -> Self {
        PredictionLedger { store, logger, ids }
    }

    /// The underlying store.
    pub fn store(&self) -> &mm_store_sqlite::SqliteStore {
        &self.store
    }

    /// The logger.
    pub fn logger(&self) -> &Arc<Logger> {
        &self.logger
    }

    /// Mint a fresh identifier.
    pub fn next_id(&self) -> Ulid {
        self.ids.next()
    }

    async fn query(&self, sql: &str, args: Params) -> Result<Vec<serde_json::Value>> {
        Tabular::query_json(&self.store, sql, args)
            .await
            .map_err(|e| MetaError::Store(e.to_string()))
    }

    async fn exec(&self, sql: &str, args: Params) -> Result<u64> {
        Tabular::execute(&self.store, sql, args)
            .await
            .map_err(|e| MetaError::Store(e.to_string()))
    }

    /// Stake a prediction, returning its identifier.
    ///
    /// The identifier is taken from the prediction when the caller set one and minted
    /// otherwise, so a replay can reproduce the row exactly.
    pub async fn record(&self, prediction: &Prediction) -> Result<Ulid> {
        prediction.validate()?;
        let id = if prediction.id.is_nil() {
            self.ids.next()
        } else {
            prediction.id
        };
        let conditions = serde_json::to_string(&prediction.conditions)?;
        self.exec(
            "INSERT INTO prediction_ledger (id, class, proposition, probability, horizon_seconds, \
             conditions_json, episode_ulid, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&id)),
                Param::Text(prediction.class.clone()),
                Param::Text(prediction.proposition.clone()),
                Param::Real(f64::from(prediction.probability)),
                Param::Int(prediction.horizon_seconds()),
                Param::Text(conditions),
                Param::opt_text(prediction.episode.map(|e| mm_core::ulid_string(&e))),
                Param::Text(prediction.created_at.to_rfc3339()),
            ],
        )
        .await?;
        Ok(id)
    }

    /// Resolve a prediction, once.
    pub async fn resolve(&self, id: Ulid, outcome: &PredictionOutcome) -> Result<()> {
        let existing = self
            .query(
                "SELECT prediction_ulid FROM prediction_outcomes WHERE prediction_ulid = ?",
                vec![Param::Text(mm_core::ulid_string(&id))],
            )
            .await?;
        if !existing.is_empty() {
            return Err(MetaError::AlreadyResolved(mm_core::ulid_string(&id)));
        }
        if self.get(id).await?.is_none() {
            return Err(MetaError::UnknownPrediction(mm_core::ulid_string(&id)));
        }
        self.exec(
            "INSERT INTO prediction_outcomes (prediction_ulid, observed, resolved_at, evidence_ulid) \
             VALUES (?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&id)),
                Param::Int(i64::from(outcome.observed)),
                Param::Text(outcome.resolved_at.to_rfc3339()),
                Param::Text(mm_core::ulid_string(&outcome.evidence)),
            ],
        )
        .await?;
        Ok(())
    }

    /// One prediction, if it is in the ledger.
    pub async fn get(&self, id: Ulid) -> Result<Option<Prediction>> {
        let rows = self
            .query(
                "SELECT id, class, proposition, probability, horizon_seconds, conditions_json, \
                 episode_ulid, created_at FROM prediction_ledger WHERE id = ?",
                vec![Param::Text(mm_core::ulid_string(&id))],
            )
            .await?;
        match rows.first() {
            None => Ok(None),
            Some(row) => Ok(Some(parse_prediction(row)?)),
        }
    }

    /// Every prediction that has not been resolved, oldest first.
    pub async fn unresolved(&self) -> Result<Vec<Prediction>> {
        let rows = self
            .query(
                "SELECT p.id, p.class, p.proposition, p.probability, p.horizon_seconds, \
                 p.conditions_json, p.episode_ulid, p.created_at FROM prediction_ledger p \
                 LEFT JOIN prediction_outcomes o ON o.prediction_ulid = p.id \
                 WHERE o.prediction_ulid IS NULL ORDER BY p.created_at, p.id",
                Vec::new(),
            )
            .await?;
        rows.iter().map(parse_prediction).collect()
    }

    /// Every resolved prediction, as a labeled set. The only way to get labels.
    pub async fn labeled(&self, class: Option<&str>) -> Result<Vec<Labeled>> {
        let (sql, args) = match class {
            Some(class) => (
                "SELECT p.class, p.probability, o.observed FROM prediction_ledger p \
                 JOIN prediction_outcomes o ON o.prediction_ulid = p.id \
                 WHERE p.class = ? ORDER BY p.created_at, p.id",
                vec![Param::Text(class.to_string())],
            ),
            None => (
                "SELECT p.class, p.probability, o.observed FROM prediction_ledger p \
                 JOIN prediction_outcomes o ON o.prediction_ulid = p.id \
                 ORDER BY p.created_at, p.id",
                Vec::new(),
            ),
        };
        let rows = self.query(sql, args).await?;
        rows.iter()
            .map(|row| {
                let class = text(row, "class")?;
                let predicted = number(row, "probability")? as f32;
                let observed = integer(row, "observed")?;
                Ok(Labeled {
                    class,
                    predicted,
                    label: observed != 0,
                })
            })
            .collect()
    }
}

fn parse_prediction(row: &serde_json::Value) -> Result<Prediction> {
    let id = parse_ulid(&text(row, "id")?)?;
    let probability = number(row, "probability")?;
    let horizon_seconds = integer(row, "horizon_seconds")?;
    Ok(Prediction {
        id,
        class: text(row, "class")?,
        proposition: text(row, "proposition")?,
        probability: probability as f32,
        horizon: Duration::from_secs(u64::try_from(horizon_seconds).unwrap_or(0)),
        conditions: serde_json::from_str(&text(row, "conditions_json")?)
            .map_err(|e| malformed("prediction_ledger", format!("conditions_json: {e}")))?,
        episode: match row.get("episode_ulid").and_then(serde_json::Value::as_str) {
            Some(text) => Some(parse_ulid(text)?),
            None => None,
        },
        created_at: parse_timestamp(&text(row, "created_at")?)?,
    })
}

fn text(row: &serde_json::Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| malformed("prediction_ledger", format!("column {key} is missing")))
}

fn number(row: &serde_json::Value, key: &str) -> Result<f64> {
    row.get(key)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| malformed("prediction_ledger", format!("column {key} is missing")))
}

fn integer(row: &serde_json::Value, key: &str) -> Result<i64> {
    row.get(key)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| malformed("prediction_ledger", format!("column {key} is missing")))
}

fn parse_ulid(text: &str) -> Result<Ulid> {
    mm_core::id::parse_ulid(text).map_err(|e| malformed("prediction_ledger", e.to_string()))
}

fn parse_timestamp(text: &str) -> Result<Timestamp> {
    Timestamp::from_rfc3339(text)
        .map_err(|e| malformed("prediction_ledger", format!("timestamp {text:?}: {e}")))
}

/// A ULID content-addressed by a string.
///
/// Used for identifiers that must be the same on every machine — a lesson's row, a
/// reference node — so a replay produces the same identifier rather than a fresh one.
/// The first 16 bytes of the SHA-256 of the text, with the version bits cleared the
/// way ULID requires.
pub fn content_ulid(text: &str) -> Ulid {
    let digest = mm_core::content_hash(text.as_bytes());
    let bytes = digest.as_bytes();
    let mut parts = [0u8; 16];
    for (index, slot) in parts.iter_mut().enumerate() {
        let high = hex_nibble(bytes[index * 2]) << 4;
        let low = hex_nibble(bytes[index * 2 + 1]);
        *slot = high | low;
    }
    parts[0] &= 0b0000_0111;
    Ulid::from_bytes(parts)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> Timestamp {
        Timestamp::from_epoch_seconds(seconds)
    }

    async fn ledger() -> (
        tempfile::TempDir,
        PredictionLedger,
        mm_store_sqlite::SqliteStore,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let ledger = PredictionLedger::new(
            store.clone(),
            crate::test_logger(&store),
            Arc::new(UlidFactory::new()),
        );
        (dir, ledger, store)
    }

    fn prediction(class: &str, probability: f32) -> Prediction {
        Prediction::new(
            Ulid::nil(),
            class,
            "the migration applies cleanly",
            probability,
            Duration::from_secs(3600),
        )
        .at(at(1_700_000_000))
        .when(vec!["the store was empty".into()])
    }

    #[tokio::test]
    async fn a_recorded_prediction_reads_back_unchanged() {
        let (_dir, ledger, _store) = ledger().await;
        let id = ledger.record(&prediction("safety", 0.8)).await.unwrap();
        let loaded = ledger.get(id).await.unwrap().expect("the row was written");
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.class, "safety");
        assert!((loaded.probability - 0.8).abs() < 1e-6);
        assert_eq!(loaded.horizon_seconds(), 3600);
        assert_eq!(loaded.conditions, vec!["the store was empty".to_string()]);
        assert_eq!(loaded.created_at, at(1_700_000_000));
        assert!(loaded.episode.is_none());
    }

    #[tokio::test]
    async fn a_prediction_is_refused_when_it_cannot_be_scored() {
        let (_dir, ledger, _store) = ledger().await;
        let mut bad = prediction("safety", 1.5);
        assert!(ledger.record(&bad).await.is_err());
        bad = prediction("", 0.5);
        assert!(ledger.record(&bad).await.is_err());
        let mut no_horizon = prediction("safety", 0.5);
        no_horizon.horizon = Duration::ZERO;
        assert!(ledger.record(&no_horizon).await.is_err());
        let mut empty = prediction("safety", 0.5);
        empty.proposition = "   ".into();
        assert!(ledger.record(&empty).await.is_err());
    }

    #[tokio::test]
    async fn a_resolution_is_written_once() {
        let (_dir, ledger, store) = ledger().await;
        let id = ledger.record(&prediction("safety", 0.8)).await.unwrap();
        let outcome = PredictionOutcome::new(true, Ulid::from_parts(1, 9), at(1_700_000_100));
        ledger.resolve(id, &outcome).await.unwrap();

        let again = ledger
            .resolve(
                id,
                &PredictionOutcome::new(false, Ulid::from_parts(1, 10), at(1_700_000_200)),
            )
            .await
            .unwrap_err();
        assert_eq!(again.code(), "already_resolved");
        assert!(again.to_string().contains(&mm_core::ulid_string(&id)));

        let labeled = ledger.labeled(None).await.unwrap();
        assert_eq!(labeled.len(), 1);
        assert!(labeled[0].label, "the first resolution stands");

        // The unknown prediction is a different refusal from the resolved one.
        let unknown = ledger
            .resolve(
                Ulid::from_parts(2, 2),
                &PredictionOutcome::new(true, Ulid::from_parts(1, 11), at(1_700_000_300)),
            )
            .await
            .unwrap_err();
        assert_eq!(unknown.code(), "unknown_prediction");
        assert_eq!(store.row_count("prediction_outcomes").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn the_database_refuses_to_rewrite_or_erase_a_prediction() {
        let (_dir, ledger, store) = ledger().await;
        ledger.record(&prediction("safety", 0.8)).await.unwrap();
        let update = Tabular::execute(
            &store,
            "UPDATE prediction_ledger SET probability = 0.1",
            Vec::new(),
        )
        .await;
        assert!(update.is_err(), "the ledger must be append-only");
        let delete = Tabular::execute(&store, "DELETE FROM prediction_ledger", Vec::new()).await;
        assert!(delete.is_err(), "the ledger must not be erasable");
        let loaded = ledger.get(Ulid::nil()).await.unwrap();
        assert!(loaded.is_none());
        assert_eq!(store.row_count("prediction_ledger").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_resolution_cannot_be_rewritten_either() {
        let (_dir, ledger, store) = ledger().await;
        let id = ledger.record(&prediction("safety", 0.8)).await.unwrap();
        ledger
            .resolve(
                id,
                &PredictionOutcome::new(true, Ulid::from_parts(1, 9), at(1_700_000_100)),
            )
            .await
            .unwrap();
        let update = Tabular::execute(
            &store,
            "UPDATE prediction_outcomes SET observed = 0",
            Vec::new(),
        )
        .await;
        assert!(update.is_err());
        assert_eq!(store.row_count("prediction_outcomes").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn only_resolved_predictions_are_labeled() {
        let (_dir, ledger, _store) = ledger().await;
        let resolved = ledger.record(&prediction("safety", 0.9)).await.unwrap();
        ledger.record(&prediction("safety", 0.4)).await.unwrap();
        ledger.record(&prediction("retrieval", 0.7)).await.unwrap();
        ledger
            .resolve(
                resolved,
                &PredictionOutcome::new(true, Ulid::from_parts(1, 9), at(1_700_000_100)),
            )
            .await
            .unwrap();

        assert_eq!(ledger.unresolved().await.unwrap().len(), 2);
        let all = ledger.labeled(None).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].class, "safety");
        assert_eq!(ledger.labeled(Some("safety")).await.unwrap().len(), 1);
        assert!(ledger.labeled(Some("retrieval")).await.unwrap().is_empty());
    }

    #[test]
    fn a_content_ulid_is_stable_and_content_sensitive() {
        let a = content_ulid("lesson:retrieval:01h");
        let b = content_ulid("lesson:retrieval:01h");
        let c = content_ulid("lesson:retrieval:01j");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(mm_core::ulid_string(&a).len(), mm_core::ULID_LEN);
    }
}
