//! Per-pass and per-turn budgets.
//!
//! Rounds are always capped so a pass terminates. Token and wall-clock ceilings
//! apply only when `budget.enabled` is set in `config/three_pass.toml`.

use serde::{Deserialize, Serialize};

/// The size class of one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BudgetClass {
    /// Interpret and Verify: small, fast calls.
    Quick,
    /// Process: the working pass.
    #[default]
    Standard,
    /// A deep pass, should one ever be configured.
    Deep,
}

impl BudgetClass {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetClass::Quick => "quick",
            BudgetClass::Standard => "standard",
            BudgetClass::Deep => "deep",
        }
    }

    /// The limits for this class.
    pub fn limits(self) -> BudgetLimits {
        match self {
            BudgetClass::Quick => BudgetLimits {
                rounds: 6,
                tokens: 32_000,
                wall_ms: 120_000,
            },
            BudgetClass::Standard => BudgetLimits {
                rounds: 12,
                tokens: 96_000,
                wall_ms: 300_000,
            },
            BudgetClass::Deep => BudgetLimits {
                rounds: 16,
                tokens: 184_000,
                wall_ms: 600_000,
            },
        }
    }
}

/// A pass ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetLimits {
    /// Model rounds.
    pub rounds: u32,
    /// Prompt plus completion tokens.
    pub tokens: u64,
    /// Wall-clock milliseconds.
    pub wall_ms: u64,
}

/// A prompt-facing budget snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSnapshot {
    /// Class name.
    pub class: String,
    /// Rounds remaining.
    pub rounds: u32,
    /// Tokens remaining.
    pub tokens: u64,
    /// Wall-clock milliseconds remaining.
    pub wall_ms: u64,
    /// Whether token and wall ceilings are enforced.
    pub enforced: bool,
}

impl BudgetSnapshot {
    /// A snapshot for one class.
    pub fn for_class(class: BudgetClass, enforced: bool) -> Self {
        let limits = class.limits();
        Self {
            class: class.as_str().into(),
            rounds: limits.rounds,
            tokens: limits.tokens,
            wall_ms: limits.wall_ms,
            enforced,
        }
    }
}

/// A live pass budget.
#[derive(Debug, Clone)]
pub struct PassBudget {
    limits: BudgetLimits,
    enforced: bool,
    rounds_used: u32,
    tokens_used: u64,
    started: std::time::Instant,
}

impl PassBudget {
    /// Start a pass.
    pub fn new(class: BudgetClass, enforced: bool) -> Self {
        Self {
            limits: class.limits(),
            enforced,
            rounds_used: 0,
            tokens_used: 0,
            started: std::time::Instant::now(),
        }
    }

    /// Reserve one model round.
    pub fn reserve_round(&mut self) -> bool {
        if self.rounds_used >= self.limits.rounds {
            return false;
        }
        self.rounds_used += 1;
        true
    }

    /// Record token usage.
    pub fn add_tokens(&mut self, tokens: u64) {
        self.tokens_used = self.tokens_used.saturating_add(tokens);
    }

    /// The remaining budget.
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            class: "pass".into(),
            rounds: self.limits.rounds.saturating_sub(self.rounds_used),
            tokens: self.limits.tokens.saturating_sub(self.tokens_used),
            wall_ms: self
                .limits
                .wall_ms
                .saturating_sub(self.started.elapsed().as_millis() as u64),
            enforced: self.enforced,
        }
    }

    /// Whether token or wall limits are exhausted.
    pub fn exhausted(&self) -> bool {
        self.enforced
            && (self.tokens_used >= self.limits.tokens
                || self.started.elapsed().as_millis() as u64 >= self.limits.wall_ms)
    }
}

/// The whole turn's budget.
#[derive(Debug, Clone)]
pub struct TurnEnvelope {
    /// Maximum verify attempts including the first.
    pub max_attempts: u32,
    /// Tokens spent in the turn.
    pub tokens_used: u64,
    /// Token ceiling when enforcement is on.
    pub tokens: u64,
    /// Whether token limits are enforced.
    pub enforced: bool,
}

impl TurnEnvelope {
    /// Start a turn.
    pub fn new(max_attempts: u32, enforced: bool, tokens: u64) -> Self {
        Self {
            max_attempts,
            tokens_used: 0,
            tokens,
            enforced,
        }
    }

    /// Record a call's usage.
    pub fn add_tokens(&mut self, tokens: u64) {
        self.tokens_used = self.tokens_used.saturating_add(tokens);
    }

    /// Whether the turn has spent its token allowance.
    pub fn exhausted(&self) -> bool {
        self.enforced && self.tokens_used >= self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_are_capped() {
        let mut budget = PassBudget::new(BudgetClass::Quick, false);
        for _ in 0..6 {
            assert!(budget.reserve_round());
        }
        assert!(!budget.reserve_round());
    }

    #[test]
    fn tokens_are_only_enforced_when_configured() {
        let mut budget = PassBudget::new(BudgetClass::Quick, false);
        budget.add_tokens(1_000_000);
        assert!(!budget.exhausted());
        let mut budget = PassBudget::new(BudgetClass::Quick, true);
        budget.add_tokens(32_000);
        assert!(budget.exhausted());
    }
}
