//! `mm-tools` — authorized, observable action.
//!
//! Phase 10 is the boundary where the being can touch the world and learn whether it
//! succeeded. Four invariants carry the phase, and each one is a mechanism rather
//! than a promise:
//!
//! 1. **The environment decides what is true.** An [`observation::Observation`] is
//!    constructed by [`executor::Executor`] from a [`spec::ToolSpec`]'s own outcome
//!    and admitted to `/world` through Phase 6's one door
//!    ([`mm_epistemic::ValidationBarrier`]). Nothing else can make one: a plan, a
//!    model, or a caller may *propose* an action, never a fact about the world.
//! 2. **Authorization is deterministic.** [`permissions::PermissionEngine`] is a pure
//!    function of the grants and the policy sets; a model's confidence never
//!    overrides a denial, and [`policy::PolicyEngine`] gives explicit `forbid`
//!    precedence over any `permit`, with absence of a permit meaning deny.
//! 3. **Every action is auditable, and either reversible or explicitly
//!    irreversible.** [`ledger`] is append-only and hash-chained; [`rollback`]
//!    restores a snapshot and refuses for `Irreversible` tools.
//! 4. **Effects are exactly-once.** [`idempotency`] guards every invocation: a
//!    retry with the same key returns the recorded outcome instead of re-applying
//!    the effect.
//!
//! The pipeline is one ordered pass, and a denial short-circuits it before any side
//! effect:
//!
//! ```text
//! resolve → authorize → tier → snapshot → invoke → observe → ledger
//! ```
//!
//! # Two deviations from the phase plan, and why
//!
//! * **The tool RDF mirror lives in a `/tools` graph, not in `/provenance`.** Phase 6's
//!   epistemic mirror *clears and rewrites* `/world` and `/provenance` on every
//!   mutation, so action quads written there would be erased by the next claim
//!   promotion. `/tools` is added to `mm_core::iri::NAMED_GRAPHS` by migration
//!   `0010_tools.sql`, which is what that module's own rule requires of a new graph
//!   — the same mechanism Phase 9 used for `/decision`. Observations themselves *do*
//!   reach `/world`, as claims, through the barrier.
//! * **The observation node class is `mm:ActionObservation`, not `mm:Observation`.**
//!   `mm:Observation` is already the RDF class of every epistemic claim of kind
//!   `Observation` (`mm_epistemic::rdf::claim_quads` types the node by its
//!   `ClaimKind`), so a shape requiring `mm:producedEvidence` on `mm:Observation`
//!   would fire on Phase 6's own `/epistemic` and `/world` content. It is the same
//!   collision `ontology/shapes/decision.ttl` records for `mm:Decision`, handled
//!   the same way: a distinct class, and the reason written down rather than
//!   discovered later.
#![forbid(unsafe_code)]

pub mod action;
pub mod error;
pub mod executor;
pub mod idempotency;
pub mod ledger;
pub mod mcp;
pub mod observation;
pub mod permissions;
pub mod policy;
pub mod rdf;
pub mod registry;
pub mod rollback;
pub mod sandbox;
pub mod spec;
pub mod tools;
pub mod verify;

pub use action::{ActionResult, ActionSpec, ActionStatus};
pub use error::{
    DupError, LedgerError, McpError, RegistryError, Result, RollbackError, SandboxError, ToolError,
};
pub use executor::{ExecError, Executor};
pub use idempotency::{Begin, IdempotencyKey, IdempotencyStore};
pub use ledger::{verify_chain, LedgerEntry, LedgerRef};
pub use observation::{Observation, ObservationPayload, SourceRef};
pub use permissions::{
    Decision, DenyReason, EscalationTarget, PermissionAction, PermissionEngine, PermissionReq,
    Principal, TablePermissionEngine,
};
pub use policy::{Effect, PolicyEngine, PolicyRule, PolicySet};
pub use registry::{ExecCtx, PersistedTool, Tool, ToolArgs, ToolOutcome, ToolRegistry};
pub use rollback::{rollback, snapshot, Snapshot, SnapshotId, SnapshotKind};
pub use sandbox::{
    CapabilityGuard, FilesystemAccess, NetworkAccess, Sandbox, SandboxCapabilities, SandboxTier,
    WasiSandbox,
};
pub use spec::{
    JsonSchema, Reversibility, SchemaRef, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};
pub use verify::{obligation_for, Obligation, ObligationKind, Proof, Verdict};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.tools";

/// A canonical JSON rendering: object keys sorted at every depth, no insignificant
/// whitespace.
///
/// It is a contract, not a convenience. Four things key on it — `tool_calls.args_hash`,
/// the idempotency key a caller derives, the ledger payload, and the observation
/// payload hash — and each of them has to be stable across processes and machines.
/// Two values that differ only in key order are the same value, so they must render to
/// the same bytes; otherwise a retry of the identical call would look like a different
/// call and apply its effect twice.
///
/// Numbers are rendered by `serde_json`, which this workspace builds with
/// `float_roundtrip`, so a value read back from its own rendering is bit-identical.
pub fn canonical_json(value: &serde_json::Value) -> String {
    serde_json::to_string(&canonicalize(value)).unwrap_or_else(|_| "null".to_string())
}

/// Rebuild a value with its object keys sorted, recursively.
fn canonicalize(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted = serde_json::Map::with_capacity(map.len());
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                sorted.insert(key.clone(), canonicalize(&map[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize).collect())
        }
        other => other.clone(),
    }
}

/// The sandbox root, relative to the repository root: every filesystem capability
/// is a restriction of this directory.
///
/// It is a constant rather than configuration because the capability model's whole
/// claim is that a tool gets *no* ambient authority: a sandbox root an operator
/// could point at `/` would make the tier-1 guard decorative.
pub const SANDBOX_ROOT: &str = "data/sandbox";
