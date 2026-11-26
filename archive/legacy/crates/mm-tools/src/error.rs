//! Every way a tool call can fail.
//!
//! One error enum per boundary, and the boundaries are named so a caller can tell
//! *where* a refusal came from: [`ToolError`] is a tool refusing its own work,
//! [`SandboxError`] is the capability guard refusing before any work starts,
//! [`RegistryError`] is the registry refusing a name, [`LedgerError`] is the chain
//! refusing an append, [`RollbackError`] is a restore refusing to claim success it
//! cannot prove, and [`DupError`] is the idempotency guard.
//!
//! The distinction is load-bearing for the phase's second invariant: a *denial* is
//! never a `ToolError`, because a tool that refuses its own work has already been
//! authorized and sandboxed. Denials come from [`crate::permissions::Decision`].

use thiserror::Error;

use crate::spec::ToolName;

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, ToolError>;

/// A tool refusing to do its work.
#[derive(Debug, Error)]
pub enum ToolError {
    /// The arguments are not the shape the tool's schema declares.
    #[error("tool {tool} refused its arguments: {reason}")]
    BadArguments {
        /// Which tool.
        tool: ToolName,
        /// What was wrong with them.
        reason: String,
    },
    /// The tool ran and failed.
    #[error("tool {tool} failed: {reason}")]
    Failed {
        /// Which tool.
        tool: ToolName,
        /// Why.
        reason: String,
    },
    /// The tool is registered but not implemented in this build.
    #[error("tool {tool} is not implemented: {reason}")]
    Unsupported {
        /// Which tool.
        tool: ToolName,
        /// Why it is not implemented.
        reason: String,
    },
    /// A store or graph operation failed.
    #[error("tool {tool} could not reach its backing store: {reason}")]
    Backing {
        /// Which tool.
        tool: ToolName,
        /// Why.
        reason: String,
    },
    /// The sandbox refused a capability the tool asked for.
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    /// The executor could not proceed.
    #[error("execution error: {0}")]
    Execution(String),
    /// A kernel error.
    #[error(transparent)]
    Core(#[from] mm_core::MmError),
}

impl ToolError {
    /// A stable machine-readable code, used in log records and ledger payloads.
    pub fn code(&self) -> &'static str {
        match self {
            ToolError::BadArguments { .. } => "tool.bad_arguments",
            ToolError::Failed { .. } => "tool.failed",
            ToolError::Unsupported { .. } => "tool.unsupported",
            ToolError::Backing { .. } => "tool.backing",
            ToolError::Sandbox(_) => "tool.sandbox",
            ToolError::Execution(_) => "tool.execution",
            ToolError::Core(_) => "tool.core",
        }
    }

    /// True when the failure is the capability guard's, not the tool's.
    pub fn is_sandbox_denial(&self) -> bool {
        matches!(self, ToolError::Sandbox(_))
    }

    /// True when retrying the identical call could plausibly succeed.
    ///
    /// Only a failure that is *not* a refusal is retryable: a bad argument, an
    /// unsupported tool, or a capability denial is a property of the request, and
    /// retrying it would produce the same refusal and (for the capability case) a
    /// second ledger entry for an effect that never happened.
    pub fn is_retryable(&self) -> bool {
        matches!(self, ToolError::Failed { .. } | ToolError::Backing { .. })
    }
}

/// The capability guard refusing.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SandboxError {
    /// The requested path is outside every granted filesystem capability.
    #[error("capability {capability} denied: {requested} is outside {allowed}")]
    PathDenied {
        /// The capability that was asked for, e.g. `fs.write`.
        capability: String,
        /// What was requested.
        requested: String,
        /// What was granted.
        allowed: String,
    },
    /// The requested host is not on the network allowlist.
    #[error("capability network denied: {host} is not allowlisted")]
    HostDenied {
        /// The host that was requested.
        host: String,
    },
    /// A write was requested through a read-only capability.
    #[error("capability {capability} denied: the grant is read-only")]
    ReadOnly {
        /// The capability that was asked for.
        capability: String,
    },
    /// The payload is larger than the grant allows.
    #[error("capability max_bytes denied: {requested} exceeds {allowed}")]
    TooLarge {
        /// How large the payload is.
        requested: u64,
        /// The ceiling.
        allowed: u64,
    },
    /// A subprocess was requested with no subprocess capability.
    #[error("capability subprocess denied: the sandbox grants no process execution")]
    SubprocessDenied,
    /// The tier cannot run here.
    #[error("sandbox tier {tier} is unavailable: {reason}")]
    TierUnavailable {
        /// The tier that was requested.
        tier: String,
        /// Why it cannot run.
        reason: String,
    },
    /// The request escaped the sandbox root.
    #[error("sandbox escape refused: {requested} does not resolve inside the sandbox root")]
    Escape {
        /// What was requested.
        requested: String,
    },
    /// The filesystem could not be consulted.
    #[error("sandbox could not resolve {requested}: {reason}")]
    Resolution {
        /// What was requested.
        requested: String,
        /// Why it could not be resolved.
        reason: String,
    },
}

impl SandboxError {
    /// A stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            SandboxError::PathDenied { .. } => "sandbox.path_denied",
            SandboxError::HostDenied { .. } => "sandbox.host_denied",
            SandboxError::ReadOnly { .. } => "sandbox.read_only",
            SandboxError::TooLarge { .. } => "sandbox.too_large",
            SandboxError::SubprocessDenied => "sandbox.subprocess_denied",
            SandboxError::TierUnavailable { .. } => "sandbox.tier_unavailable",
            SandboxError::Escape { .. } => "sandbox.escape",
            SandboxError::Resolution { .. } => "sandbox.resolution",
        }
    }

    /// The capability the refusal is about, for the `tool.sandbox.deny` record.
    pub fn capability(&self) -> &'static str {
        match self {
            SandboxError::PathDenied { .. } => "filesystem",
            SandboxError::HostDenied { .. } => "network",
            SandboxError::ReadOnly { .. } => "filesystem.read_only",
            SandboxError::TooLarge { .. } => "max_bytes",
            SandboxError::SubprocessDenied => "subprocess",
            SandboxError::TierUnavailable { .. } => "tier",
            SandboxError::Escape { .. } => "filesystem.root",
            SandboxError::Resolution { .. } => "filesystem",
        }
    }
}

/// The registry refusing a registration or a resolution.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RegistryError {
    /// The name is already registered.
    #[error("tool {0} is already registered")]
    Duplicate(String),
    /// No tool has that name.
    #[error("no tool named {0} is registered")]
    Unknown(String),
    /// The name is not a `namespace.name` pair.
    #[error("tool name {0:?} is not a `namespace.name` pair")]
    BadName(String),
    /// The spec is not internally consistent.
    #[error("tool {tool} declares an invalid spec: {reason}")]
    InvalidSpec {
        /// Which tool.
        tool: String,
        /// What is wrong with it.
        reason: String,
    },
}

impl RegistryError {
    /// A stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            RegistryError::Duplicate(_) => "registry.duplicate",
            RegistryError::Unknown(_) => "registry.unknown",
            RegistryError::BadName(_) => "registry.bad_name",
            RegistryError::InvalidSpec { .. } => "registry.invalid_spec",
        }
    }
}

/// The action ledger refusing an append or failing verification.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum LedgerError {
    /// The entry does not name the current chain head.
    #[error("ledger chain broken at seq {seq}: entry names prev {named}, head is {head}")]
    ChainBroken {
        /// The sequence the entry claimed.
        seq: i64,
        /// The `prev_hash` the entry carried.
        named: String,
        /// The actual head.
        head: String,
    },
    /// The stored entry hash does not match its content.
    #[error("ledger entry {seq} has hash {stored} but its content hashes to {computed}")]
    HashMismatch {
        /// The sequence number.
        seq: i64,
        /// What the row says.
        stored: String,
        /// What the content hashes to.
        computed: String,
    },
    /// A sequence number is missing.
    #[error("ledger sequence is not gapless: expected {expected}, found {found}")]
    Gap {
        /// The sequence that should have been there.
        expected: i64,
        /// The sequence that was.
        found: i64,
    },
    /// The chain does not start at the genesis hash.
    #[error("ledger does not start at the genesis hash, it starts at {found}")]
    BadGenesis {
        /// The first `prev_hash` found.
        found: String,
    },
    /// A store failure.
    #[error("ledger store failure: {0}")]
    Store(String),
}

impl LedgerError {
    /// A stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            LedgerError::ChainBroken { .. } => "ledger.chain_broken",
            LedgerError::HashMismatch { .. } => "ledger.hash_mismatch",
            LedgerError::Gap { .. } => "ledger.gap",
            LedgerError::BadGenesis { .. } => "ledger.bad_genesis",
            LedgerError::Store(_) => "ledger.store",
        }
    }
}

/// A rollback refusing to restore.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RollbackError {
    /// The snapshot is not in the store.
    #[error("no snapshot {0} is recorded")]
    Unknown(String),
    /// The tool is irreversible, so no snapshot may be taken or restored.
    #[error("action {action} is irreversible and cannot be rolled back")]
    Irreversible {
        /// Which action.
        action: String,
    },
    /// The restored content does not hash to the recorded value.
    #[error("restore of {path} produced {found} but the snapshot recorded {expected}")]
    NotByteIdentical {
        /// What was restored.
        path: String,
        /// What the snapshot recorded.
        expected: String,
        /// What the file now hashes to.
        found: String,
    },
    /// The snapshot's kind has no restore implementation.
    #[error("snapshot kind {0} has no restore path in this build")]
    UnsupportedKind(String),
    /// The snapshot could not be taken.
    #[error("could not snapshot {path}: {reason}")]
    Snapshot {
        /// What was being snapshotted.
        path: String,
        /// Why it failed.
        reason: String,
    },
    /// A store failure.
    #[error("rollback store failure: {0}")]
    Store(String),
}

impl RollbackError {
    /// A stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            RollbackError::Unknown(_) => "rollback.unknown",
            RollbackError::Irreversible { .. } => "rollback.irreversible",
            RollbackError::NotByteIdentical { .. } => "rollback.not_byte_identical",
            RollbackError::UnsupportedKind(_) => "rollback.unsupported_kind",
            RollbackError::Snapshot { .. } => "rollback.snapshot",
            RollbackError::Store(_) => "rollback.store",
        }
    }
}

/// The idempotency guard refusing a duplicate.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DupError {
    /// A call with this key is in flight.
    #[error("an action with idempotency key {0} is already in flight")]
    InFlight(String),
    /// A store failure.
    #[error("idempotency store failure: {0}")]
    Store(String),
}

impl DupError {
    /// A stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            DupError::InFlight(_) => "idempotency.in_flight",
            DupError::Store(_) => "idempotency.store",
        }
    }
}

/// The MCP bridge refusing.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum McpError {
    /// The request is not a JSON-RPC 2.0 request.
    #[error("not a JSON-RPC 2.0 request: {0}")]
    BadRequest(String),
    /// The method is not one the bridge serves.
    #[error("unknown MCP method {0:?}")]
    UnknownMethod(String),
    /// The named tool is not registered.
    #[error("no tool named {0} is registered")]
    UnknownTool(String),
    /// The call was refused before it reached the tool.
    #[error("MCP call refused: {0}")]
    Refused(String),
    /// The transport failed.
    #[error("MCP transport failure: {0}")]
    Transport(String),
}
