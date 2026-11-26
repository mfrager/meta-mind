//! Session registry, presence heartbeat, and expiry-sweep contracts.
//!
//! This is the in-process view used by the console later. Durable session RDF is
//! written by the Phase 1 `SessionStore`; the registry only tracks leases and
//! derives presence/expiry. No route is wired in Phase 0.

use std::collections::BTreeMap;
use std::sync::Arc;

use mm_data::{Presence, SessionLifecycle};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Default present window.
pub const DEFAULT_PRESENT_WINDOW_MS: i64 = 2 * 60 * 1_000;
/// Default away window.
pub const DEFAULT_AWAY_WINDOW_MS: i64 = 15 * 60 * 1_000;
/// Default idle expiry.
pub const DEFAULT_IDLE_EXPIRY_MS: i64 = 30 * 60 * 1_000;
/// Default work-stall TTL.
pub const DEFAULT_WORK_STALL_TTL_MS: i64 = 5 * 60 * 1_000;

/// Registry thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Heartbeat age at which presence becomes `away`.
    pub present_window_ms: i64,
    /// Heartbeat age at which presence becomes `absent`.
    pub away_window_ms: i64,
    /// Idle age at which an active session ends.
    pub idle_expiry_ms: i64,
    /// No-activity age at which a working item stalls.
    pub work_stall_ttl_ms: i64,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            present_window_ms: DEFAULT_PRESENT_WINDOW_MS,
            away_window_ms: DEFAULT_AWAY_WINDOW_MS,
            idle_expiry_ms: DEFAULT_IDLE_EXPIRY_MS,
            work_stall_ttl_ms: DEFAULT_WORK_STALL_TTL_MS,
        }
    }
}

impl RegistryConfig {
    /// Validate ordering.
    pub fn validate(&self) -> Result<(), RegistryError> {
        if !(self.present_window_ms > 0
            && self.away_window_ms >= self.present_window_ms
            && self.idle_expiry_ms >= self.away_window_ms
            && self.work_stall_ttl_ms > 0)
        {
            return Err(RegistryError::Invalid(
                "windows must satisfy 0 < present <= away <= idle expiry".into(),
            ));
        }
        Ok(())
    }
}

/// A live lease. The durable state is separate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLease {
    /// Session id.
    pub session_id: String,
    /// User id.
    pub user_id: String,
    /// Lifecycle.
    pub lifecycle: SessionLifecycle,
    /// Last heartbeat.
    pub last_seen_at: i64,
    /// Opening instant.
    pub opened_at: i64,
    /// Last work activity.
    pub last_work_at: i64,
    /// Whether a work item is currently active.
    #[serde(default)]
    pub work_active: bool,
    /// Explicit end time, if any.
    pub ended_at: Option<i64>,
    /// End reason.
    pub end_reason: Option<String>,
}

impl SessionLease {
    /// A fresh active lease.
    pub fn new(session_id: impl Into<String>, user_id: impl Into<String>, now_ms: i64) -> Self {
        Self {
            session_id: session_id.into(),
            user_id: user_id.into(),
            lifecycle: SessionLifecycle::Active,
            last_seen_at: now_ms,
            opened_at: now_ms,
            last_work_at: now_ms,
            work_active: false,
            ended_at: None,
            end_reason: None,
        }
    }

    /// Derived presence at `now`.
    pub fn presence(&self, now: i64, config: &RegistryConfig) -> Presence {
        if self.lifecycle != SessionLifecycle::Active {
            return Presence::Absent;
        }
        let age = now.saturating_sub(self.last_seen_at);
        if age <= config.present_window_ms {
            Presence::Present
        } else if age <= config.away_window_ms {
            Presence::Away
        } else {
            Presence::Absent
        }
    }

    /// Whether the session is idle past the registry expiry.
    pub fn is_idle_expired(&self, now: i64, config: &RegistryConfig) -> bool {
        self.lifecycle == SessionLifecycle::Active
            && now.saturating_sub(self.last_seen_at) >= config.idle_expiry_ms
    }

    /// Mark ended.
    pub fn end(&mut self, now: i64, reason: impl Into<String>) {
        self.lifecycle = SessionLifecycle::Ended;
        self.ended_at = Some(now);
        self.end_reason = Some(reason.into());
    }
}

/// One sweep result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExpiry {
    /// Session id.
    pub session_id: String,
    /// What the sweep did.
    pub action: ExpiryAction,
    /// Reason.
    pub reason: String,
}

/// Sweep actions are deliberately limited to leases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryAction {
    /// Presence crossed into away.
    Away,
    /// Presence crossed into absent.
    Absent,
    /// Active lease ended after idle expiry.
    Ended,
    /// A working item produced no activity within the stall TTL.
    WorkStall,
}

/// Thread-safe registry over leases. Persistence is a later phase.
#[derive(Debug, Clone)]
pub struct SessionRegistry {
    config: RegistryConfig,
    leases: Arc<RwLock<BTreeMap<String, SessionLease>>>,
    last_presence: Arc<RwLock<BTreeMap<String, Presence>>>,
}

impl SessionRegistry {
    /// Build with validated thresholds.
    pub fn new(config: RegistryConfig) -> Result<Self, RegistryError> {
        config.validate()?;
        Ok(Self {
            config,
            leases: Arc::new(RwLock::new(BTreeMap::new())),
            last_presence: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    /// Build with defaults.
    pub fn default_config() -> Self {
        Self::new(RegistryConfig::default()).expect("default registry config is valid")
    }

    /// Thresholds.
    pub fn config(&self) -> RegistryConfig {
        self.config
    }

    /// Insert a lease, replacing the same id.
    pub async fn insert(&self, lease: SessionLease) {
        let id = lease.session_id.clone();
        self.last_presence
            .write()
            .await
            .insert(id.clone(), lease.presence(lease.last_seen_at, &self.config));
        self.leases.write().await.insert(id, lease);
    }

    /// Look up a clone.
    pub async fn get(&self, session_id: &str) -> Option<SessionLease> {
        self.leases.read().await.get(session_id).cloned()
    }

    /// Number of live leases.
    pub async fn len(&self) -> usize {
        self.leases.read().await.len()
    }

    /// Whether no leases are tracked.
    pub async fn is_empty(&self) -> bool {
        self.leases.read().await.is_empty()
    }

    /// Record a heartbeat and return the new presence.
    pub async fn heartbeat(&self, session_id: &str, now_ms: i64) -> Option<Presence> {
        let mut leases = self.leases.write().await;
        let lease = leases.get_mut(session_id)?;
        if lease.lifecycle != SessionLifecycle::Active {
            return Some(Presence::Absent);
        }
        lease.last_seen_at = now_ms;
        let presence = lease.presence(now_ms, &self.config);
        drop(leases);
        self.last_presence
            .write()
            .await
            .insert(session_id.to_owned(), presence);
        Some(presence)
    }

    /// Mark work activity without changing presence.
    pub async fn touch_work(&self, session_id: &str, now_ms: i64) -> bool {
        let mut leases = self.leases.write().await;
        let Some(lease) = leases.get_mut(session_id) else {
            return false;
        };
        if lease.lifecycle != SessionLifecycle::Active {
            return false;
        }
        lease.last_work_at = now_ms;
        lease.work_active = true;
        true
    }

    /// Explicitly end a lease. Returns false when absent or already ended.
    pub async fn end(&self, session_id: &str, now_ms: i64, reason: &str) -> bool {
        let mut leases = self.leases.write().await;
        let Some(lease) = leases.get_mut(session_id) else {
            return false;
        };
        if lease.lifecycle == SessionLifecycle::Ended {
            return false;
        }
        lease.end(now_ms, reason);
        true
    }

    /// Sweep presence transitions and idle expiry.
    pub async fn sweep(&self, now_ms: i64) -> Vec<SessionExpiry> {
        let mut out = Vec::new();
        let mut leases = self.leases.write().await;
        let mut last = self.last_presence.write().await;
        for lease in leases.values_mut() {
            let previous = last
                .get(&lease.session_id)
                .copied()
                .unwrap_or(Presence::Absent);
            let current = lease.presence(now_ms, &self.config);
            if current != previous {
                let action = match current {
                    Presence::Away => Some(ExpiryAction::Away),
                    Presence::Absent => Some(ExpiryAction::Absent),
                    Presence::Present => None,
                };
                if let Some(action) = action {
                    out.push(SessionExpiry {
                        session_id: lease.session_id.clone(),
                        action,
                        reason: "heartbeat window crossed".into(),
                    });
                }
                last.insert(lease.session_id.clone(), current);
            }
            if lease.lifecycle == SessionLifecycle::Active
                && lease.work_active
                && now_ms.saturating_sub(lease.last_work_at) >= self.config.work_stall_ttl_ms
            {
                lease.work_active = false;
                out.push(SessionExpiry {
                    session_id: lease.session_id.clone(),
                    action: ExpiryAction::WorkStall,
                    reason: "work stall TTL".into(),
                });
            }
            if lease.is_idle_expired(now_ms, &self.config) {
                lease.end(now_ms, "idle expiry");
                last.insert(lease.session_id.clone(), Presence::Absent);
                out.push(SessionExpiry {
                    session_id: lease.session_id.clone(),
                    action: ExpiryAction::Ended,
                    reason: "idle expiry".into(),
                });
            }
        }
        out
    }

    /// Remove ended leases older than `retain_ms`.
    pub async fn prune_ended(&self, now_ms: i64, retain_ms: i64) -> usize {
        let mut leases = self.leases.write().await;
        let before = leases.len();
        leases.retain(|_, lease| {
            lease.lifecycle != SessionLifecycle::Ended
                || lease
                    .ended_at
                    .is_none_or(|ended| now_ms.saturating_sub(ended) < retain_ms)
        });
        before.saturating_sub(leases.len())
    }
}

/// Registry failures.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Invalid thresholds.
    #[error("invalid registry config: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn heartbeat_drives_presence_and_idle_sweep_ends() {
        let registry = SessionRegistry::default_config();
        registry.insert(SessionLease::new("s", "u", 1_000)).await;
        assert_eq!(
            registry.heartbeat("s", 1_000).await,
            Some(Presence::Present)
        );
        assert_eq!(
            registry
                .sweep(1_000 + DEFAULT_AWAY_WINDOW_MS + 1)
                .await
                .len(),
            1
        );
        let sweep = registry.sweep(1_000 + DEFAULT_IDLE_EXPIRY_MS).await;
        assert!(sweep.iter().any(|item| item.action == ExpiryAction::Ended));
        assert_eq!(
            registry.get("s").await.unwrap().lifecycle,
            SessionLifecycle::Ended
        );
    }

    #[test]
    fn invalid_windows_are_rejected() {
        let config = RegistryConfig {
            present_window_ms: 10,
            away_window_ms: 5,
            idle_expiry_ms: 20,
            work_stall_ttl_ms: 5,
        };
        assert!(config.validate().is_err());
    }
}
