//! Wire types shared across every `mm-serve` route (plan `09` §1, §4).
//!
//! These types are the single source of truth for the TypeScript mirror in
//! `frontend/src/lib/wire/`. Regenerate the mirror with:
//!
//! ```text
//! cargo test -p mm-serve export_types -- --ignored
//! ```
//!
//! and never edit `frontend/src/lib/wire/*.ts` by hand. Field names on the wire
//! follow the plan literally: snake_case except where the plan's frames name a
//! camelCase field (`correlationId`, `requestId`, `sessionId`, `wireVersion`,
//! `dataDir`, `eventHead`) — those carry an explicit `serde(rename)`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod chat;
pub mod focus;

/// The error codes the UI maps to chip/banner styles (plan `09` §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    /// A component cannot do this here; `reason` says why. Show it verbatim.
    Unavailable,
    /// The request was malformed or a field failed validation.
    InvalidArgument,
    /// No such object.
    NotFound,
    /// State conflicts with the request (e.g. a production write outside a
    /// change set — the promotion rule enforced by the API).
    Conflict,
    /// A hard budget is exhausted; the UI shows meters and a retry affordance.
    BudgetExceeded,
    /// The sanity firewall refused the action; `reason` lists the signals.
    FirewallBlocked,
    /// Another process (a second console or a CLI command) holds the write lock.
    Locked,
    /// Anything unexpected; `correlationId` ties it to the UI log.
    Internal,
}

/// Every non-2xx JSON body (plan `09` §1).
///
/// `reason` is the kernel's "Unavailable with a reason" string: the UI must
/// show it verbatim rather than paraphrase it (plan `00` §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    /// The kernel's own explanation, verbatim. Present whenever one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Ties the error to the UI log entry (`data/logs/ui/`).
    #[serde(rename = "correlationId")]
    pub correlation_id: String,
    /// Whether retrying the same request could plausibly succeed.
    pub retryable: bool,
}

/// Health of one kernel service: `ok | degraded | unavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    /// Working.
    Ok,
    /// Partially working; `reason` says what still works (plan `02` §3.2).
    Degraded,
    /// Not present in this build/boot; `reason` says why.
    Unavailable,
}

/// One service line in the health envelope's `kernel.services` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ServiceStatus {
    /// Stable service name: `sqlite`, `eventlog`, `graph`, `llm`, `ui`.
    pub name: String,
    pub state: ServiceState,
    /// Required unless `state` is `ok` — honest state is a UI invariant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The kernel's overall state plus the per-service breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KernelStatus {
    pub state: ServiceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub services: Vec<ServiceStatus>,
}

/// Where the event log and its audit chain currently stand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EventHead {
    /// Highest committed event sequence (`EventLog::head_seq`), 0 when empty.
    pub seq: i64,
    /// Audit-chain head sequence, when the audit log has records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_seq: Option<i64>,
    /// Audit-chain head hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_hash: Option<String>,
}

/// The `GET /api/health` body (plan `09` §1).
///
/// `budgets` joins this envelope when the self-engineering budget service is
/// wired (M6); wire changes are additive within `wireVersion: 1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HealthEnvelope {
    /// Whether the server itself is serving. Kernel problems are reported in
    /// `kernel`, not by making `ok` lie.
    pub ok: bool,
    /// The wire contract version. The SPA refuses to run against an older one.
    #[serde(rename = "wireVersion")]
    pub wire_version: u32,
    pub kernel: KernelStatus,
    #[serde(rename = "dataDir")]
    pub data_dir: String,
    #[serde(rename = "eventHead")]
    pub event_head: EventHead,
}

/// The one wire contract version this build speaks (plan `01` §7).
pub const WIRE_VERSION: u32 = 1;
