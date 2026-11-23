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
        let limits = class.limits();
        Self {
            class,
            limits,
            rounds: limits.rounds,
            steps: limits.steps,
            tokens: limits.tokens,
            reserved_tokens: 0,
            started: Instant::now(),
            estimated: false,
        }
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
        Duration::from_millis(self.limits.wall_ms).saturating_sub(self.started.elapsed())
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
    fn child_ledgers_are_isolated() {
        let mut parent = BudgetLedger::new(BudgetClass::Standard);
        let mut child = BudgetLedger::new(BudgetClass::Plan);
        parent.reserve_round().unwrap();
        child.reserve_round().unwrap();
        assert_eq!(parent.snapshot().rounds, 31);
        assert_eq!(child.snapshot().rounds, 7);
    }
}
