//! One router decides whether a background result becomes speech.
//!
//! Policy resolution is deterministic and most-specific-first: instance,
//! user preference, kind, service, global. The router enforces working state,
//! presence, frame, quiet hours, min-gap, hourly rate, and expiry before it
//! returns `Deliver`. Phase 0 provides the contract and unit tests only.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use mm_data::FrameKind;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default minimum gap.
pub const DEFAULT_MIN_GAP_MS: i64 = 60_000;
/// Default hourly cap.
pub const DEFAULT_MAX_PER_HOUR: u16 = 6;

/// Delivery decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Append to the transcript/SSE now.
    Deliver,
    /// Keep for a safe boundary.
    Queue,
    /// Show a badge only.
    Badge,
    /// Drop now; may be re-evaluated later.
    Suppress,
}

/// Presence requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceRequirement {
    /// User must be present.
    Present,
    /// Present or away.
    PresentOrAway,
    /// Any presence, including absent.
    Any,
}

impl PresenceRequirement {
    fn allows(self, presence: mm_data::Presence) -> bool {
        match self {
            Self::Present => presence == mm_data::Presence::Present,
            Self::PresentOrAway => presence != mm_data::Presence::Absent,
            Self::Any => true,
        }
    }
}

/// Quiet-hours window in UTC hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    /// Inclusive start hour, 0–23.
    pub start_hour: u8,
    /// Exclusive end hour, 0–24.
    pub end_hour: u8,
}

impl QuietHours {
    /// Whether `at_ms` falls in the window (UTC).
    pub fn contains(self, at_ms: i64) -> bool {
        let hour = ((at_ms.div_euclid(3_600_000)) % 24) as u8;
        if self.start_hour <= self.end_hour {
            hour >= self.start_hour && hour < self.end_hour
        } else {
            hour >= self.start_hour || hour < self.end_hour
        }
    }

    /// Validate hours.
    pub fn validate(self) -> Result<(), InterjectionError> {
        if self.start_hour > 23 || self.end_hour > 24 || self.start_hour == self.end_hour {
            return Err(InterjectionError::Invalid(
                "quiet hours must be a non-empty 0–24 window".into(),
            ));
        }
        Ok(())
    }
}

/// A reviewed policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterjectionPolicy {
    /// Policy id, e.g. `internal.default`.
    pub id: String,
    /// Master switch.
    pub enabled: bool,
    /// Delivery while the session is working.
    pub when_working: Delivery,
    /// Delivery while the session is not working.
    pub when_idle: Delivery,
    /// Presence requirement.
    pub presence: PresenceRequirement,
    /// Allowed frame kinds; empty means any.
    pub allowed_frames: Vec<FrameKind>,
    /// Minimum gap between deliveries.
    pub min_gap_ms: i64,
    /// Hourly cap.
    pub max_per_hour: u16,
    /// Optional quiet hours.
    pub quiet_hours: Option<QuietHours>,
    /// Higher wins when draining a queue.
    pub priority: u8,
    /// Queued item expiry.
    pub expires_after_ms: i64,
}

impl Default for InterjectionPolicy {
    fn default() -> Self {
        Self {
            id: "global.default".into(),
            enabled: true,
            when_working: Delivery::Queue,
            when_idle: Delivery::Deliver,
            presence: PresenceRequirement::PresentOrAway,
            allowed_frames: Vec::new(),
            min_gap_ms: DEFAULT_MIN_GAP_MS,
            max_per_hour: DEFAULT_MAX_PER_HOUR,
            quiet_hours: None,
            priority: 5,
            expires_after_ms: 15 * 60 * 1_000,
        }
    }
}

impl InterjectionPolicy {
    /// Validate bounds.
    pub fn validate(&self) -> Result<(), InterjectionError> {
        if self.id.trim().is_empty() {
            return Err(InterjectionError::Invalid("policy id is empty".into()));
        }
        if self.min_gap_ms < 0 || self.expires_after_ms <= 0 {
            return Err(InterjectionError::Invalid(
                "policy timing bounds are invalid".into(),
            ));
        }
        if self.max_per_hour == 0 {
            return Err(InterjectionError::Invalid(
                "max_per_hour must be positive".into(),
            ));
        }
        if let Some(quiet) = self.quiet_hours {
            quiet.validate()?;
        }
        Ok(())
    }
}

/// A source/category for policy lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterjectionSource {
    /// Scheduled/registered internal action.
    InternalAction,
    /// Cron-backed scheduled action.
    ScheduledAction,
    /// Agent result.
    AgentResult,
    /// Frame utterance.
    FrameUtterance,
    /// Plan monitor (never user-facing).
    PlanMonitor,
    /// Explicit introspection.
    Introspection,
}

impl InterjectionSource {
    /// The service default policy id.
    pub fn service_policy(self) -> &'static str {
        match self {
            Self::InternalAction => "internal.default",
            Self::ScheduledAction => "scheduled.default",
            Self::AgentResult => "agent.result",
            Self::FrameUtterance => "frame.utterance",
            Self::PlanMonitor => "plan.monitor",
            Self::Introspection => "introspection.default",
        }
    }
}

/// A candidate delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterjectionRequest {
    /// Candidate id.
    pub id: String,
    /// Session.
    pub session_id: String,
    /// Source/category.
    pub source: InterjectionSource,
    /// Optional kind-specific policy id.
    pub kind_policy: Option<String>,
    /// Optional user-preference policy id.
    pub user_policy: Option<String>,
    /// Optional instance policy id.
    pub instance_policy: Option<String>,
    /// Current frame kind, if any.
    pub frame: Option<FrameKind>,
    /// Whether the session is working.
    pub working: bool,
    /// Current presence.
    pub presence: mm_data::Presence,
    /// Decision instant.
    pub at: i64,
    /// Candidate expiry.
    pub expires_at: i64,
}

/// The router's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterjectionDecision {
    /// Chosen delivery.
    pub decision: Delivery,
    /// Stable reason.
    pub reason: String,
    /// Policy that won resolution.
    pub policy_id: String,
    /// When a queued/badged item may be reconsidered.
    pub retry_at: Option<i64>,
}

/// Policy resolution with rate state.
#[derive(Debug, Clone)]
pub struct InterjectionRouter {
    policies: BTreeMap<String, InterjectionPolicy>,
    global_policy: String,
    rate: std::sync::Arc<Mutex<BTreeMap<String, VecDeque<i64>>>>,
    last_delivery: std::sync::Arc<Mutex<BTreeMap<String, i64>>>,
}

impl InterjectionRouter {
    /// Build from the global plus service policies. Specific policies may be
    /// added with [`InterjectionRouter::upsert_policy`].
    pub fn new(
        global: InterjectionPolicy,
        services: impl IntoIterator<Item = (String, InterjectionPolicy)>,
    ) -> Result<Self, InterjectionError> {
        global.validate()?;
        let mut policies = BTreeMap::new();
        let global_id = global.id.clone();
        policies.insert(global.id.clone(), global);
        for (id, policy) in services {
            if id != policy.id {
                return Err(InterjectionError::Invalid(format!(
                    "policy key {id} does not match id {}",
                    policy.id
                )));
            }
            policy.validate()?;
            policies.insert(id, policy);
        }
        Ok(Self {
            policies,
            global_policy: global_id,
            rate: std::sync::Arc::new(Mutex::new(BTreeMap::new())),
            last_delivery: std::sync::Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// The production defaults from `config/interjections.toml`.
    pub fn defaults() -> Result<Self, InterjectionError> {
        let global = InterjectionPolicy {
            id: "global.default".into(),
            when_working: Delivery::Queue,
            when_idle: Delivery::Deliver,
            presence: PresenceRequirement::PresentOrAway,
            min_gap_ms: 60_000,
            max_per_hour: 6,
            quiet_hours: Some(QuietHours {
                start_hour: 22,
                end_hour: 8,
            }),
            priority: 0,
            expires_after_ms: 15 * 60 * 1_000,
            ..InterjectionPolicy::default()
        };
        let mut router = Self::new(global, std::iter::empty())?;
        let services = [
            (
                "internal.default",
                InterjectionPolicy {
                    id: "internal.default".into(),
                    when_working: Delivery::Suppress,
                    when_idle: Delivery::Deliver,
                    allowed_frames: vec![FrameKind::Work, FrameKind::Resume, FrameKind::Custom],
                    priority: 3,
                    ..InterjectionPolicy::default()
                },
            ),
            (
                "scheduled.default",
                InterjectionPolicy {
                    id: "scheduled.default".into(),
                    when_working: Delivery::Badge,
                    when_idle: Delivery::Deliver,
                    priority: 2,
                    ..InterjectionPolicy::default()
                },
            ),
            (
                "agent.result",
                InterjectionPolicy {
                    id: "agent.result".into(),
                    when_working: Delivery::Queue,
                    when_idle: Delivery::Deliver,
                    priority: 6,
                    ..InterjectionPolicy::default()
                },
            ),
            (
                "frame.utterance",
                InterjectionPolicy {
                    id: "frame.utterance".into(),
                    when_working: Delivery::Suppress,
                    when_idle: Delivery::Deliver,
                    priority: 1,
                    ..InterjectionPolicy::default()
                },
            ),
            (
                "plan.monitor",
                InterjectionPolicy {
                    id: "plan.monitor".into(),
                    enabled: false,
                    when_working: Delivery::Suppress,
                    when_idle: Delivery::Suppress,
                    priority: 0,
                    ..InterjectionPolicy::default()
                },
            ),
            (
                "introspection.default",
                InterjectionPolicy {
                    id: "introspection.default".into(),
                    when_working: Delivery::Suppress,
                    when_idle: Delivery::Deliver,
                    priority: 7,
                    ..InterjectionPolicy::default()
                },
            ),
        ];
        for (_, policy) in services {
            router.upsert_policy(policy)?;
        }
        Ok(router)
    }

    /// Add or replace a policy.
    pub fn upsert_policy(&mut self, policy: InterjectionPolicy) -> Result<(), InterjectionError> {
        policy.validate()?;
        self.policies.insert(policy.id.clone(), policy);
        Ok(())
    }

    /// A policy by id.
    pub fn policy(&self, id: &str) -> Option<&InterjectionPolicy> {
        self.policies.get(id)
    }

    /// Resolve and route one candidate.
    pub fn route(
        &self,
        request: &InterjectionRequest,
    ) -> Result<InterjectionDecision, InterjectionError> {
        let candidates = [
            request.instance_policy.as_deref(),
            request.user_policy.as_deref(),
            request.kind_policy.as_deref(),
            Some(request.source.service_policy()),
            Some(self.global_policy.as_str()),
        ];
        let policy = candidates
            .iter()
            .flatten()
            .find_map(|id| self.policies.get(*id))
            .ok_or_else(|| InterjectionError::Invalid("no interjection policy resolved".into()))?;

        if !policy.enabled {
            return Ok(decision(
                Delivery::Suppress,
                "policy disabled",
                &policy.id,
                None,
            ));
        }
        if request.at >= request.expires_at {
            return Ok(decision(
                Delivery::Suppress,
                "candidate expired",
                &policy.id,
                None,
            ));
        }
        if !policy.presence.allows(request.presence) {
            return Ok(decision(
                Delivery::Suppress,
                "presence requirement not met",
                &policy.id,
                None,
            ));
        }
        if let Some(frame) = request.frame {
            if !policy.allowed_frames.is_empty() && !policy.allowed_frames.contains(&frame) {
                return Ok(decision(
                    Delivery::Suppress,
                    "frame kind not allowed",
                    &policy.id,
                    None,
                ));
            }
        }
        let quiet_hours = policy.quiet_hours.or_else(|| {
            self.policies
                .get(&self.global_policy)
                .and_then(|global| global.quiet_hours)
        });
        if let Some(quiet) = quiet_hours {
            if quiet.contains(request.at) {
                return Ok(decision(
                    Delivery::Queue,
                    "quiet hours",
                    &policy.id,
                    Some(next_hour(request.at)),
                ));
            }
        }
        let delivery = if request.working {
            policy.when_working
        } else {
            policy.when_idle
        };
        if delivery != Delivery::Deliver {
            return Ok(decision(
                delivery,
                if request.working {
                    "working state"
                } else {
                    "idle policy"
                },
                &policy.id,
                None,
            ));
        }
        if let Some(last) = self
            .last_delivery
            .lock()
            .expect("interjection mutex")
            .get(&request.session_id)
            .copied()
        {
            if request.at.saturating_sub(last) < policy.min_gap_ms {
                return Ok(decision(
                    Delivery::Queue,
                    "minimum gap",
                    &policy.id,
                    Some(last.saturating_add(policy.min_gap_ms)),
                ));
            }
        }
        let mut rate = self.rate.lock().expect("interjection mutex");
        let bucket = rate.entry(request.session_id.clone()).or_default();
        while bucket
            .front()
            .is_some_and(|at| *at <= request.at - 3_600_000)
        {
            bucket.pop_front();
        }
        if bucket.len() >= usize::from(policy.max_per_hour) {
            return Ok(decision(
                Delivery::Queue,
                "hourly rate limit",
                &policy.id,
                Some(request.at.saturating_add(3_600_000)),
            ));
        }
        bucket.push_back(request.at);
        drop(rate);
        self.last_delivery
            .lock()
            .expect("interjection mutex")
            .insert(request.session_id.clone(), request.at);
        Ok(decision(Delivery::Deliver, "delivered", &policy.id, None))
    }
}

fn decision(
    decision: Delivery,
    reason: &str,
    policy_id: &str,
    retry_at: Option<i64>,
) -> InterjectionDecision {
    InterjectionDecision {
        decision,
        reason: reason.into(),
        policy_id: policy_id.into(),
        retry_at,
    }
}

fn next_hour(at: i64) -> i64 {
    (at.div_euclid(3_600_000) + 1).saturating_mul(3_600_000)
}

/// Interjection-contract failures.
#[derive(Debug, Error)]
pub enum InterjectionError {
    /// Invalid policy/request.
    #[error("invalid interjection: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(source: InterjectionSource, working: bool) -> InterjectionRequest {
        InterjectionRequest {
            id: "i".into(),
            session_id: "s".into(),
            source,
            kind_policy: None,
            user_policy: None,
            instance_policy: None,
            frame: Some(FrameKind::Work),
            working,
            presence: mm_data::Presence::Present,
            at: 12 * 3_600_000,
            expires_at: 13 * 3_600_000,
        }
    }

    #[test]
    fn internal_action_is_suppressed_while_working() {
        let router = InterjectionRouter::defaults().unwrap();
        let decision = router
            .route(&request(InterjectionSource::InternalAction, true))
            .unwrap();
        assert_eq!(decision.decision, Delivery::Suppress);
        assert_eq!(decision.policy_id, "internal.default");
    }

    #[test]
    fn internal_action_delivers_when_idle() {
        let router = InterjectionRouter::defaults().unwrap();
        let decision = router
            .route(&request(InterjectionSource::InternalAction, false))
            .unwrap();
        assert_eq!(decision.decision, Delivery::Deliver);
    }

    #[test]
    fn plan_monitor_is_never_user_facing() {
        let router = InterjectionRouter::defaults().unwrap();
        let decision = router
            .route(&request(InterjectionSource::PlanMonitor, false))
            .unwrap();
        assert_eq!(decision.decision, Delivery::Suppress);
    }

    #[test]
    fn presence_and_quiet_hours_are_gates() {
        let router = InterjectionRouter::defaults().unwrap();
        let mut req = request(InterjectionSource::AgentResult, false);
        req.presence = mm_data::Presence::Absent;
        assert_eq!(router.route(&req).unwrap().decision, Delivery::Suppress);
        req.presence = mm_data::Presence::Present;
        req.at = 23 * 3_600_000;
        req.expires_at = req.at + 100_000;
        assert_eq!(router.route(&req).unwrap().decision, Delivery::Queue);
    }

    #[test]
    fn instance_policy_outranks_service_policy() {
        let mut router = InterjectionRouter::defaults().unwrap();
        router
            .upsert_policy(InterjectionPolicy {
                id: "instance.always".into(),
                when_working: Delivery::Badge,
                ..InterjectionPolicy::default()
            })
            .unwrap();
        let mut req = request(InterjectionSource::InternalAction, true);
        req.instance_policy = Some("instance.always".into());
        let decision = router.route(&req).unwrap();
        assert_eq!(decision.policy_id, "instance.always");
        assert_eq!(decision.decision, Delivery::Badge);
    }
}
