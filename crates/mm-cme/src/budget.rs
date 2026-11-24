//! Per-turn budget classes and the reserve/settle ledger.
//!
//! The planner chooses the class; the model never does. A ledger is owned by one
//! turn or child operation, so a plan/think/frame session cannot borrow the
//! parent conversation's allowance. Exhaustion is a hard `BUDGET` outcome, never
//! a fabricated success or a truncated state block.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The initial budget classes from the design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BudgetClass {
    /// Greeting or single lookup.
    Quick,
    /// Ordinary conversation with tools.
    #[default]
    Standard,
    /// Multi-step work or delegation.
    Deep,
    /// Continuation after a pause/stall.
    Resume,
    /// A plan sub-session.
    Plan,
    /// A think sub-session.
    Think,
    /// Optional memory ranking/summary.
    Memory,
    /// One frame utterance.
    Frame,
    /// A model-using proactive/introspection action.
    Action,
    /// One agent turn summary.
    Summarize,
}

impl BudgetClass {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Deep => "deep",
            Self::Resume => "resume",
            Self::Plan => "plan",
            Self::Think => "think",
            Self::Memory => "memory",
            Self::Frame => "frame",
            Self::Action => "action",
            Self::Summarize => "summarize",
        }
    }

    /// The class limit table.
    pub fn limits(self) -> BudgetLimits {
        match self {
            Self::Quick => BudgetLimits::new(8, 24, 32_000, 120_000),
            Self::Standard => BudgetLimits::new(32, 96, 120_000, 600_000),
            Self::Deep => BudgetLimits::new(48, 144, 200_000, 900_000),
            Self::Resume => BudgetLimits::new(40, 120, 150_000, 720_000),
            Self::Plan => BudgetLimits::new(8, 16, 16_000, 90_000),
            Self::Think => BudgetLimits::new(4, 8, 8_000, 90_000),
            Self::Memory => BudgetLimits::new(2, 2, 4_000, 30_000),
            Self::Frame => BudgetLimits::new(2, 4, 8_000, 45_000),
            Self::Action => BudgetLimits::new(4, 8, 8_000, 60_000),
            Self::Summarize => BudgetLimits::new(1, 1, 8_000, 30_000),
        }
    }
}

/// One budget class's hard limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimits {
    /// Model rounds.
    pub rounds: u32,
    /// Tool steps.
    pub steps: u32,
    /// Total tokens.
    pub tokens: u64,
    /// Wall-clock milliseconds.
    pub wall_ms: u64,
}

impl BudgetLimits {
    /// No effective limits, used when turn budgeting is disabled.
    pub const UNLIMITED: Self = Self::new(u32::MAX, u32::MAX, u64::MAX, u64::MAX);

    /// Construct a limit row.
    pub const fn new(rounds: u32, steps: u32, tokens: u64, wall_ms: u64) -> Self {
        Self {
            rounds,
            steps,
            tokens,
            wall_ms,
        }
    }
}

/// What one ledger consumed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetUsage {
    /// Model rounds consumed.
    pub rounds: u32,
    /// Tool steps consumed.
    pub steps: u32,
    /// Tokens consumed (including any reservation not yet settled).
    pub tokens: u64,
}

impl BudgetUsage {
    /// Add another usage row.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self {
            rounds: self.rounds.saturating_add(other.rounds),
            steps: self.steps.saturating_add(other.steps),
            tokens: self.tokens.saturating_add(other.tokens),
        }
    }

    /// Whether every dimension fits inside `limits`.
    pub fn fits(self, limits: BudgetLimits) -> bool {
        self.rounds <= limits.rounds && self.steps <= limits.steps && self.tokens <= limits.tokens
    }
}

/// The ledger's current view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    /// Class name.
    pub class: &'static str,
    /// Rounds remaining.
    pub rounds: u32,
    /// Steps remaining.
    pub steps: u32,
    /// Tokens remaining.
    pub tokens: u64,
    /// Wall milliseconds remaining.
    pub wall_ms: u64,
    /// Whether an estimate contributed to the token value.
    pub estimated: bool,
}

/// Reserve/settle state for one operation.
#[derive(Debug, Clone)]
pub struct BudgetLedger {
    class: BudgetClass,
    limits: BudgetLimits,
    budgeted: bool,
    rounds: u32,
    steps: u32,
    tokens: u64,
    reserved_tokens: u64,
    started: Instant,
    estimated: bool,
}

impl BudgetLedger {
    /// Start a ledger at its class limits.
    pub fn new(class: BudgetClass) -> Self {
        Self::with_limits(class, class.limits(), true)
    }

    /// Start a ledger that never refuses a round, step, token, or wall limit.
    pub fn unlimited(class: BudgetClass) -> Self {
        Self::with_limits(class, BudgetLimits::UNLIMITED, false)
    }

    fn with_limits(class: BudgetClass, limits: BudgetLimits, budgeted: bool) -> Self {
        Self {
            class,
            limits,
            budgeted,
            rounds: limits.rounds,
            steps: limits.steps,
            tokens: limits.tokens,
            reserved_tokens: 0,
            started: Instant::now(),
            estimated: false,
        }
    }

    /// Whether this ledger enforces limits.
    pub fn budgeted(&self) -> bool {
        self.budgeted
    }

    /// The class.
    pub fn class(&self) -> BudgetClass {
        self.class
    }

    /// The class limits.
    pub fn limits(&self) -> BudgetLimits {
        self.limits
    }

    /// Remaining allowances.
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            class: self.class.as_str(),
            rounds: self.rounds,
            steps: self.steps,
            tokens: self.tokens,
            wall_ms: self.wall_remaining().as_millis().min(u128::from(u64::MAX)) as u64,
            estimated: self.estimated,
        }
    }

    /// Reserve one model round.
    pub fn reserve_round(&mut self) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        self.check_wall()?;
        if self.rounds == 0 {
            return Err(BudgetError::Exhausted {
                scope: "round",
                reason: "no model rounds remain".into(),
            });
        }
        self.rounds -= 1;
        Ok(())
    }

    /// Reserve tool steps.
    pub fn reserve_steps(&mut self, count: u32) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        self.check_wall()?;
        if self.steps < count {
            return Err(BudgetError::Exhausted {
                scope: "step",
                reason: format!("requested {count} tool steps with {} remaining", self.steps),
            });
        }
        self.steps -= count;
        Ok(())
    }

    /// Reserve a conservative token estimate before a model call.
    pub fn reserve_tokens(&mut self, estimate: u64) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        self.check_wall()?;
        if self.tokens < estimate {
            return Err(BudgetError::Exhausted {
                scope: "token",
                reason: format!("requested {estimate} tokens with {} remaining", self.tokens),
            });
        }
        self.tokens -= estimate;
        self.reserved_tokens = self.reserved_tokens.saturating_add(estimate);
        Ok(())
    }

    /// Settle a reservation with actual provider usage, or the estimate.
    pub fn settle_tokens(&mut self, actual: Option<u64>) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        match actual {
            Some(actual) => {
                if actual > self.reserved_tokens {
                    let overrun = actual - self.reserved_tokens;
                    if self.tokens < overrun {
                        return Err(BudgetError::Exhausted {
                            scope: "token",
                            reason: format!("{overrun} tokens over the reservation"),
                        });
                    }
                    self.tokens -= overrun;
                } else {
                    self.tokens = self
                        .tokens
                        .saturating_add(self.reserved_tokens.saturating_sub(actual));
                }
                self.reserved_tokens = 0;
                Ok(())
            }
            None => {
                self.estimated = true;
                self.reserved_tokens = 0;
                Ok(())
            }
        }
    }

    /// Fail if the wall-clock cap has elapsed.
    pub fn check_wall(&self) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        if self.started.elapsed() >= Duration::from_millis(self.limits.wall_ms) {
            return Err(BudgetError::Exhausted {
                scope: "wall",
                reason: format!("{} ms elapsed", self.limits.wall_ms),
            });
        }
        Ok(())
    }

    /// Wall time remaining.
    pub fn wall_remaining(&self) -> Duration {
        if !self.budgeted {
            return Duration::MAX;
        }
        Duration::from_millis(self.limits.wall_ms).saturating_sub(self.started.elapsed())
    }

    /// What this ledger has consumed so far.
    pub fn usage(&self) -> BudgetUsage {
        BudgetUsage {
            rounds: self.limits.rounds.saturating_sub(self.rounds),
            steps: self.limits.steps.saturating_sub(self.steps),
            tokens: self.limits.tokens.saturating_sub(self.tokens),
        }
    }
}

/// The total allowance for one turn across Interpret, Process, Verify, and retries.
///
/// Each pass still gets its own class ledger, but the envelope is the hard ceiling:
/// a pass (or a retry, or a child operation) cannot spend what the turn does not have.
#[derive(Debug, Clone)]
pub struct TurnEnvelope {
    limits: BudgetLimits,
    budgeted: bool,
    used: BudgetUsage,
    reserved_tokens: u64,
    retries: u32,
    max_retries: u32,
    started: Instant,
}

impl TurnEnvelope {
    /// Build an envelope from explicit limits.
    pub fn new(limits: BudgetLimits, max_retries: u32) -> Self {
        Self {
            limits,
            budgeted: true,
            used: BudgetUsage::default(),
            reserved_tokens: 0,
            retries: 0,
            max_retries,
            started: Instant::now(),
        }
    }

    /// The default three-pass allowance: quick + standard + quick.
    pub fn standard(max_retries: u32) -> Self {
        let quick = BudgetClass::Quick.limits();
        let standard = BudgetClass::Standard.limits();
        Self::new(
            BudgetLimits::new(
                quick.rounds + standard.rounds + quick.rounds,
                quick.steps + standard.steps + quick.steps,
                quick.tokens + standard.tokens + quick.tokens,
                quick.wall_ms + standard.wall_ms + quick.wall_ms,
            ),
            max_retries,
        )
    }

    /// An envelope that only caps retries.
    pub fn unlimited(max_retries: u32) -> Self {
        Self {
            limits: BudgetLimits::UNLIMITED,
            budgeted: false,
            used: BudgetUsage::default(),
            reserved_tokens: 0,
            retries: 0,
            max_retries,
            started: Instant::now(),
        }
    }

    /// The envelope limits.
    pub fn limits(&self) -> BudgetLimits {
        self.limits
    }

    /// What has been consumed.
    pub fn used(&self) -> BudgetUsage {
        self.used
    }

    /// Retries consumed so far.
    pub fn retries(&self) -> u32 {
        self.retries
    }

    /// Remaining retries.
    pub fn retries_remaining(&self) -> u32 {
        self.max_retries.saturating_sub(self.retries)
    }

    /// Tokens still available after used and reserved amounts.
    pub fn remaining_tokens(&self) -> u64 {
        if !self.budgeted {
            return u64::MAX;
        }
        self.limits
            .tokens
            .saturating_sub(self.used.tokens)
            .saturating_sub(self.reserved_tokens)
    }

    /// Start one pass at its class limits. The envelope does not pre-deduct the
    /// class ceiling; [`absorb`](Self::absorb) accounts what the pass actually used.
    pub fn ledger(&mut self, class: BudgetClass) -> Result<BudgetLedger, BudgetError> {
        if !self.budgeted {
            return Ok(BudgetLedger::unlimited(class));
        }
        self.check_wall()?;
        if self.remaining_tokens() == 0 {
            return Err(BudgetError::Exhausted {
                scope: "envelope",
                reason: "no tokens remain for another pass".into(),
            });
        }
        Ok(BudgetLedger::new(class))
    }

    /// Account a finished ledger against the envelope.
    pub fn absorb(&mut self, ledger: &BudgetLedger) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        let next = self.used.plus(ledger.usage());
        if !next.fits(self.limits) {
            return Err(BudgetError::Exhausted {
                scope: "envelope",
                reason: format!(
                    "turn used {} rounds, {} steps, {} tokens",
                    next.rounds, next.steps, next.tokens
                ),
            });
        }
        if next.tokens.saturating_add(self.reserved_tokens) > self.limits.tokens {
            return Err(BudgetError::Exhausted {
                scope: "envelope",
                reason: "turn usage plus retry reservations exceed the token ceiling".into(),
            });
        }
        self.used = next;
        Ok(())
    }

    /// Reserve a token ceiling for a child operation (think, agent.run, a step)
    /// before it starts. The caller releases it once the child is accounted.
    pub fn reserve_tokens(&mut self, tokens: u64) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        if self.remaining_tokens() < tokens {
            return Err(BudgetError::Exhausted {
                scope: "token",
                reason: format!("{tokens} tokens requested for a child operation"),
            });
        }
        self.reserved_tokens = self.reserved_tokens.saturating_add(tokens);
        Ok(())
    }

    /// Reserve the token ceiling for one retry before it starts.
    pub fn reserve_retry(&mut self, tokens: u64) -> Result<(), BudgetError> {
        if self.retries >= self.max_retries {
            return Err(BudgetError::Exhausted {
                scope: "retry",
                reason: format!("{} retries already used", self.max_retries),
            });
        }
        if self.budgeted {
            self.reserve_tokens(tokens)?;
        }
        self.retries += 1;
        Ok(())
    }

    /// Release a retry reservation once the retry's ledgers have been absorbed.
    pub fn release_reservation(&mut self, tokens: u64) {
        self.reserved_tokens = self.reserved_tokens.saturating_sub(tokens);
    }

    /// Fail if the turn's wall-clock cap has elapsed.
    pub fn check_wall(&self) -> Result<(), BudgetError> {
        if !self.budgeted {
            return Ok(());
        }
        if self.started.elapsed() >= Duration::from_millis(self.limits.wall_ms) {
            return Err(BudgetError::Exhausted {
                scope: "wall",
                reason: format!("{} ms elapsed", self.limits.wall_ms),
            });
        }
        Ok(())
    }
}

/// Budget failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BudgetError {
    /// A scope has no allowance left.
    #[error("BUDGET: {scope} exhausted: {reason}")]
    Exhausted {
        /// `round`, `step`, `token`, or `wall`.
        scope: &'static str,
        /// Operator-facing reason.
        reason: String,
    },
}

/// Result alias.
pub type Result<T, E = BudgetError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_limits_match_the_design() {
        assert_eq!(
            BudgetClass::Standard.limits(),
            BudgetLimits::new(32, 96, 120_000, 600_000)
        );
        assert_eq!(
            BudgetClass::Think.limits(),
            BudgetLimits::new(4, 8, 8_000, 90_000)
        );
        assert_eq!(
            BudgetClass::Frame.limits(),
            BudgetLimits::new(2, 4, 8_000, 45_000)
        );
    }

    #[test]
    fn an_unlimited_ledger_never_refuses_work() {
        let mut ledger = BudgetLedger::unlimited(BudgetClass::Memory);
        assert!(!ledger.budgeted());
        for _ in 0..10 {
            ledger.reserve_round().unwrap();
            ledger.reserve_steps(10).unwrap();
            ledger.reserve_tokens(u64::MAX / 2).unwrap();
        }
        ledger.settle_tokens(Some(u64::MAX)).unwrap();
        ledger.check_wall().unwrap();
        assert_eq!(ledger.wall_remaining(), Duration::MAX);
    }

    #[test]
    fn an_unlimited_envelope_still_counts_retries() {
        let mut envelope = TurnEnvelope::unlimited(1);
        let mut ledger = envelope.ledger(BudgetClass::Quick).unwrap();
        assert!(!ledger.budgeted());
        ledger.reserve_tokens(u64::MAX).unwrap();
        envelope.absorb(&ledger).unwrap();
        envelope.reserve_retry(u64::MAX).unwrap();
        assert!(envelope.reserve_retry(1).is_err());
    }

    #[test]
    fn reservation_and_settlement_refund_the_difference() {
        let mut ledger = BudgetLedger::new(BudgetClass::Quick);
        ledger.reserve_round().unwrap();
        ledger.reserve_tokens(10_000).unwrap();
        assert_eq!(ledger.snapshot().tokens, 22_000);
        ledger.settle_tokens(Some(4_000)).unwrap();
        assert_eq!(ledger.snapshot().tokens, 28_000);
        assert_eq!(ledger.snapshot().rounds, 7);
    }

    #[test]
    fn over_budget_is_never_a_success() {
        let mut ledger = BudgetLedger::new(BudgetClass::Memory);
        ledger.reserve_tokens(4_000).unwrap();
        let error = ledger.reserve_tokens(1).unwrap_err();
        assert!(matches!(
            error,
            BudgetError::Exhausted { scope: "token", .. }
        ));
        assert!(error.to_string().starts_with("BUDGET:"));
    }

    #[test]
    fn an_envelope_totals_the_three_passes() {
        let mut envelope = TurnEnvelope::standard(5);
        assert_eq!(envelope.limits().tokens, 184_000);
        assert_eq!(envelope.limits().rounds, 48);

        let mut interpret = envelope.ledger(BudgetClass::Quick).unwrap();
        interpret.reserve_round().unwrap();
        interpret.reserve_tokens(1_000).unwrap();
        envelope.absorb(&interpret).unwrap();

        let mut process = envelope.ledger(BudgetClass::Standard).unwrap();
        process.reserve_round().unwrap();
        process.reserve_tokens(40_000).unwrap();
        envelope.absorb(&process).unwrap();

        assert_eq!(envelope.used().rounds, 2);
        assert_eq!(envelope.used().tokens, 41_000);
        assert_eq!(envelope.remaining_tokens(), 143_000);
    }

    #[test]
    fn an_envelope_refuses_an_overrun() {
        let mut envelope = TurnEnvelope::new(BudgetLimits::new(2, 2, 100, 1_000), 0);
        let mut ledger = envelope.ledger(BudgetClass::Quick).unwrap();
        ledger.reserve_tokens(100).unwrap();
        envelope.absorb(&ledger).unwrap();
        let error = envelope.ledger(BudgetClass::Quick).unwrap_err();
        assert!(matches!(
            error,
            BudgetError::Exhausted {
                scope: "envelope",
                ..
            }
        ));
    }

    #[test]
    fn retries_are_capped_and_reserved_before_they_run() {
        let mut envelope = TurnEnvelope::new(BudgetLimits::new(10, 10, 1_000, 60_000), 1);
        envelope.reserve_retry(400).unwrap();
        assert_eq!(envelope.retries(), 1);
        assert_eq!(envelope.remaining_tokens(), 600);
        let error = envelope.reserve_retry(100).unwrap_err();
        assert!(matches!(
            error,
            BudgetError::Exhausted { scope: "retry", .. }
        ));
        envelope.release_reservation(400);
        assert_eq!(envelope.remaining_tokens(), 1_000);
    }

    #[test]
    fn child_ledgers_are_isolated() {
        let mut parent = BudgetLedger::new(BudgetClass::Standard);
        let mut child = BudgetLedger::new(BudgetClass::Plan);
        parent.reserve_round().unwrap();
        child.reserve_round().unwrap();
        assert_eq!(parent.snapshot().rounds, 31);
        assert_eq!(child.snapshot().rounds, 7);
    }
}
