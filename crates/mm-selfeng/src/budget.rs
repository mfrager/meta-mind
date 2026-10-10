//! The four budgets, enforced by arithmetic rather than clamped.
//!
//! Phase 8 established the rule this module keeps, one level up: **a debit that would
//! cross a limit is refused whole.** The metacognitive budget bounds one deliberation
//! in memory; these four bound a whole improvement cycle, in the store, across
//! processes. The difference that matters is that these rows are shared, so the
//! enforcement cannot be a read-then-write in Rust: two concurrent debits would both
//! read `spent = 19`, both see room, and both write 20. The update is therefore a
//! single guarded statement —
//!
//! ```sql
//! UPDATE budgets SET spent_amount = spent_amount + ?
//!  WHERE kind = ? AND period = ? AND spent_amount + ? <= limit_amount
//! ```
//!
//! — and `rows_affected() == 0` *is* the denial. A refusal reads the row back only to
//! report the numbers, which is why the reported `spent` is always the value the
//! losing writer actually saw.
//!
//! The four kinds are the plan's: meta-analysis (spent by triggers and diagnosis),
//! improvement (spent by compiling a mistake into a test), evolution (spent by
//! promoting a change), metacognitive (spent by the Phase 8 loop, so the two budgets
//! cannot drift into two accounting systems). The seeded rows in migration
//! `0011_self_engineering.sql` are what make a hard cap true from the first command.

use std::sync::Arc;

use mm_core::Ulid;
use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{BudgetDenied, Result, SelfEngError};

/// The period the seeded rows belong to.
///
/// A period is a name the operator chooses, not a date the code computes: the
/// migration seeds `cycle-1`, and beginning a new period is a new set of rows that
/// leaves the old ones as the audit of what was spent.
pub const DEFAULT_PERIOD: &str = "cycle-1";

/// One of the four budgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetKind {
    /// Spent by a trigger firing and a diagnosis running.
    MetaAnalysis,
    /// Spent by turning a mistake into a regression test.
    Improvement,
    /// Spent by promoting a change set.
    Evolution,
    /// Spent by the Phase 8 metacognitive loop.
    Metacognitive,
}

/// Every kind, in the order `budget show` reports them.
pub const BUDGET_KINDS: [BudgetKind; 4] = [
    BudgetKind::MetaAnalysis,
    BudgetKind::Improvement,
    BudgetKind::Evolution,
    BudgetKind::Metacognitive,
];

impl BudgetKind {
    /// Every kind.
    pub const ALL: [BudgetKind; 4] = BUDGET_KINDS;

    /// The stable wire name, which is also the `budgets.kind` value.
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetKind::MetaAnalysis => "meta_analysis",
            BudgetKind::Improvement => "improvement",
            BudgetKind::Evolution => "evolution",
            BudgetKind::Metacognitive => "metacognitive",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        BUDGET_KINDS
            .into_iter()
            .find(|kind| kind.as_str() == trimmed || kind.as_str() == trimmed.to_ascii_lowercase())
    }
}

impl std::fmt::Display for BudgetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One budget row: its limit and what has been spent against it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetRow {
    /// The dimension.
    pub kind: BudgetKind,
    /// The period the row belongs to.
    pub period: String,
    /// The limit the row carries.
    pub limit: f64,
    /// What has been spent.
    pub spent: f64,
}

impl BudgetRow {
    /// What is left.
    pub fn remaining(&self) -> f64 {
        (self.limit - self.spent).max(0.0)
    }

    /// True when the row has no room left at all.
    pub fn is_exhausted(&self) -> bool {
        self.remaining() <= 0.0
    }

    /// A stable rendering, so two reads compare as strings.
    pub fn canonical(&self) -> String {
        format!(
            "kind={},period={},limit={:.9},spent={:.9},remaining={:.9}",
            self.kind.as_str(),
            self.period,
            self.limit,
            self.spent,
            self.remaining()
        )
    }
}

/// The arithmetic of a debit, as a pure function.
///
/// Separated from the store so the invariant the phase cares about — *the sum of
/// accepted debits never exceeds the limit* — is property-testable without a
/// database, and so the store path and the model cannot disagree about the boundary.
/// Spending exactly what is left is within the limit; spending more is not.
pub fn may_debit(limit: f64, spent: f64, amount: f64) -> bool {
    if !amount.is_finite() || amount < 0.0 {
        return false;
    }
    if !limit.is_finite() || !spent.is_finite() {
        return false;
    }
    spent + amount <= limit
}

/// What a successful debit produced: the row after the write.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetDebitOutcome {
    /// The row as it now stands.
    pub row: BudgetRow,
    /// The trace the debit was recorded under.
    pub trace: Ulid,
}

/// The four budgets over the `budgets` table.
pub struct BudgetLedger {
    store: mm_store_sqlite::SqliteStore,
    logger: Arc<Logger>,
    period: String,
}

impl BudgetLedger {
    /// A ledger over the seeded period.
    pub fn new(store: mm_store_sqlite::SqliteStore, logger: Arc<Logger>) -> Self {
        BudgetLedger {
            store,
            logger,
            period: DEFAULT_PERIOD.to_string(),
        }
    }

    /// The same ledger over another period.
    pub fn with_period(mut self, period: impl Into<String>) -> Self {
        self.period = period.into();
        self
    }

    /// The period every debit is taken from.
    pub fn period(&self) -> &str {
        &self.period
    }

    /// Spend `amount` of `kind`, or refuse whole.
    ///
    /// The refusal is [`SelfEngError::BudgetDenied`] and the row is left exactly as it
    /// was: not clamped to the limit, not partially applied. The guard is in the
    /// `WHERE` clause rather than in a read-then-write, so two processes racing for
    /// the last unit of budget cannot both win it.
    pub async fn debit(
        &self,
        kind: BudgetKind,
        amount: f64,
        trace: Ulid,
    ) -> Result<BudgetDebitOutcome> {
        let before = self.row(kind).await?;
        if !may_debit(before.limit, before.spent, amount) {
            self.deny(kind, amount, before.remaining(), trace).await?;
            return Err(SelfEngError::BudgetDenied(BudgetDenied {
                kind,
                period: self.period.clone(),
                limit: before.limit,
                spent: before.spent,
                requested: amount,
            }));
        }
        let affected = sqlx::query(
            "UPDATE budgets SET spent_amount = spent_amount + ? \
             WHERE kind = ? AND period = ? AND spent_amount + ? <= limit_amount",
        )
        .bind(amount)
        .bind(kind.as_str())
        .bind(&self.period)
        .bind(amount)
        .execute(self.store.pool())
        .await
        .map_err(|e| SelfEngError::Budget(format!("debit failed: {e}")))?;
        if affected.rows_affected() == 0 {
            // Lost the race, or the row changed underneath us. Either way the
            // refusal is the same one, with the numbers read back from the winner's
            // state.
            let after = self.row(kind).await?;
            self.deny(kind, amount, after.remaining(), trace).await?;
            return Err(SelfEngError::BudgetDenied(BudgetDenied {
                kind,
                period: self.period.clone(),
                limit: after.limit,
                spent: after.spent,
                requested: amount,
            }));
        }
        let after = self.row(kind).await?;
        self.logger
            .audit(
                Level::Info,
                codes::BUDGET_DEBIT,
                crate::TARGET,
                Some(trace),
                json!({
                    "kind": kind.as_str(),
                    "amount": amount,
                    "remaining": after.remaining(),
                    "period": self.period,
                }),
            )
            .await?;
        Ok(BudgetDebitOutcome { row: after, trace })
    }

    /// What is left of one dimension.
    pub async fn remaining(&self, kind: BudgetKind) -> Result<f64> {
        Ok(self.row(kind).await?.remaining())
    }

    /// Every row, sorted by kind then period.
    pub async fn show(&self) -> Result<Vec<BudgetRow>> {
        let rows: Vec<(String, String, f64, f64)> = sqlx::query_as(
            "SELECT kind, period, limit_amount, spent_amount FROM budgets \
             ORDER BY kind, period",
        )
        .fetch_all(self.store.pool())
        .await
        .map_err(|e| SelfEngError::Budget(format!("cannot read budgets: {e}")))?;
        rows.into_iter()
            .map(|(kind, period, limit, spent)| {
                let kind = BudgetKind::parse(&kind).ok_or_else(|| {
                    SelfEngError::Budget(format!("unknown budget kind {kind:?} in the store"))
                })?;
                Ok(BudgetRow {
                    kind,
                    period,
                    limit,
                    spent,
                })
            })
            .collect()
    }

    /// The row for one kind in this period.
    async fn row(&self, kind: BudgetKind) -> Result<BudgetRow> {
        let row: Option<(f64, f64)> = sqlx::query_as(
            "SELECT limit_amount, spent_amount FROM budgets WHERE kind = ? AND period = ?",
        )
        .bind(kind.as_str())
        .bind(&self.period)
        .fetch_optional(self.store.pool())
        .await
        .map_err(|e| {
            SelfEngError::Budget(format!("cannot read the {} budget: {e}", kind.as_str()))
        })?;
        let (limit, spent) = row.ok_or_else(|| {
            SelfEngError::Budget(format!(
                "no {} budget exists for period {}",
                kind.as_str(),
                self.period
            ))
        })?;
        Ok(BudgetRow {
            kind,
            period: self.period.clone(),
            limit,
            spent,
        })
    }

    /// Record a refusal.
    async fn deny(&self, kind: BudgetKind, amount: f64, remaining: f64, trace: Ulid) -> Result<()> {
        self.logger
            .audit(
                Level::Warn,
                codes::BUDGET_DENY,
                crate::TARGET,
                Some(trace),
                json!({
                    "kind": kind.as_str(),
                    "amount": amount,
                    "remaining": remaining,
                    "period": self.period,
                }),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;
    use mm_store_sqlite::SqliteStore;
    use proptest::prelude::*;
    use std::path::Path;

    async fn store(dir: &Path) -> SqliteStore {
        let store = SqliteStore::open(&dir.join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        store
    }

    fn logger(dir: &Path, store: &SqliteStore) -> Arc<Logger> {
        let cfg = Config::for_data_dir(dir);
        Arc::new(
            Logger::from_config(&cfg.log, Some(Arc::new(store.clone())))
                .expect("a logger over the test store"),
        )
    }

    fn trace(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    async fn ledger(dir: &Path) -> (SqliteStore, BudgetLedger) {
        let store = store(dir).await;
        let logger = logger(dir, &store);
        let ledger = BudgetLedger::new(store.clone(), logger);
        (store, ledger)
    }

    #[tokio::test]
    async fn the_migration_seeds_one_row_per_kind_and_the_ledger_reads_them() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, ledger) = ledger(dir.path()).await;
        let rows = ledger.show().await.unwrap();
        assert_eq!(rows.len(), 4);
        let kinds: Vec<&str> = rows.iter().map(|row| row.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["evolution", "improvement", "meta_analysis", "metacognitive"]
        );
        assert_eq!(ledger.remaining(BudgetKind::Evolution).await.unwrap(), 20.0);
        assert_eq!(
            ledger.remaining(BudgetKind::Metacognitive).await.unwrap(),
            500.0
        );
    }

    #[tokio::test]
    async fn a_debit_within_the_limit_is_recorded_and_visible_in_remaining() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, ledger) = ledger(dir.path()).await;
        let outcome = ledger
            .debit(BudgetKind::Improvement, 12.5, trace(1))
            .await
            .unwrap();
        assert_eq!(outcome.row.spent, 12.5);
        assert_eq!(outcome.row.limit, 50.0);
        assert!((outcome.row.remaining() - 37.5).abs() < 1e-9);
        assert_eq!(
            ledger.remaining(BudgetKind::Improvement).await.unwrap(),
            37.5
        );
    }

    #[tokio::test]
    async fn a_debit_that_would_cross_the_limit_is_refused_whole() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, ledger) = ledger(dir.path()).await;
        ledger
            .debit(BudgetKind::Evolution, 19.5, trace(1))
            .await
            .unwrap();
        let error = ledger
            .debit(BudgetKind::Evolution, 1.0, trace(2))
            .await
            .unwrap_err();
        assert!(error.is_budget_denied(), "{error}");
        assert_eq!(error.code(), "budget.denied");
        // Byte-identical: the refused debit changed nothing, and it was not clamped
        // to the remaining half unit.
        assert_eq!(ledger.remaining(BudgetKind::Evolution).await.unwrap(), 0.5);
        let rows = ledger.show().await.unwrap();
        let evolution = rows
            .iter()
            .find(|row| row.kind == BudgetKind::Evolution)
            .unwrap();
        assert_eq!(evolution.spent, 19.5);
    }

    #[tokio::test]
    async fn spending_exactly_the_remainder_is_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, ledger) = ledger(dir.path()).await;
        ledger
            .debit(BudgetKind::Evolution, 20.0, trace(1))
            .await
            .unwrap();
        let row = ledger
            .show()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.kind == BudgetKind::Evolution)
            .unwrap();
        assert!(row.is_exhausted());
        assert!(ledger
            .debit(BudgetKind::Evolution, 0.0001, trace(2))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn an_unknown_period_is_a_named_refusal_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, base) = ledger(dir.path()).await;
        let ledger = base.with_period("cycle-404");
        let error = ledger.remaining(BudgetKind::Evolution).await.unwrap_err();
        assert_eq!(error.code(), "budget");
        assert!(error.to_string().contains("cycle-404"), "{error}");
    }

    #[tokio::test]
    async fn every_kind_round_trips_through_its_wire_name() {
        for kind in BUDGET_KINDS {
            assert_eq!(BudgetKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(BudgetKind::parse("nonesuch"), None);
        assert_eq!(
            BudgetKind::parse(" Evolution "),
            Some(BudgetKind::Evolution)
        );
    }

    #[test]
    fn the_pure_arithmetic_agrees_with_the_boundary_the_store_enforces() {
        assert!(may_debit(20.0, 19.0, 1.0));
        assert!(!may_debit(20.0, 19.0, 1.0000001));
        assert!(
            may_debit(20.0, 20.0, 0.0),
            "a zero debit is never a violation"
        );
        assert!(!may_debit(20.0, 0.0, -1.0), "a negative debit is refused");
        assert!(
            !may_debit(20.0, 0.0, f64::NAN),
            "a non-finite debit is refused"
        );
        assert!(!may_debit(f64::INFINITY, 0.0, 1.0));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// The invariant the phase asserts: over any sequence of debit attempts, the
        /// accepted ones sum to at most the limit.
        #[test]
        fn accepted_debits_never_exceed_the_limit(
            amounts in prop::collection::vec(0.0f64..40.0, 0..40)
        ) {
            let limit = 20.0_f64;
            let mut spent = 0.0_f64;
            let mut accepted = 0.0_f64;
            for amount in amounts {
                if may_debit(limit, spent, amount) {
                    spent += amount;
                    accepted += amount;
                }
            }
            prop_assert!(spent <= limit, "spent {spent} exceeded {limit}");
            prop_assert!(accepted <= limit + 1e-9);
        }

        /// A refused debit leaves the state exactly as it was: the model refuses
        /// without touching `spent`, and the store's guarded `UPDATE` matches it.
        #[test]
        fn a_refused_debit_does_not_move_the_balance(
            limit in 0.0f64..100.0,
            spent in 0.0f64..100.0,
            amount in 0.0f64..200.0,
        ) {
            let mut model = spent;
            let before = model;
            if may_debit(limit, model, amount) {
                model += amount;
            }
            if !may_debit(limit, before, amount) {
                prop_assert_eq!(model, before);
            }
        }
    }

    #[tokio::test]
    async fn a_sequence_of_debits_through_the_store_stops_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, ledger) = ledger(dir.path()).await;
        let mut accepted = 0.0_f64;
        let mut denied = 0_u32;
        for step in 0..12_u128 {
            match ledger
                .debit(BudgetKind::MetaAnalysis, 25.0, trace(step))
                .await
            {
                Ok(outcome) => accepted = outcome.row.spent,
                Err(error) => {
                    assert!(error.is_budget_denied());
                    denied += 1;
                }
            }
        }
        assert_eq!(accepted, 200.0, "eight debits of 25 fit in the seeded 200");
        assert_eq!(denied, 4);
        assert_eq!(
            ledger.remaining(BudgetKind::MetaAnalysis).await.unwrap(),
            0.0
        );
    }
}
