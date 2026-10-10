//! The executor: one ordered pass from a proposal to a recorded outcome.
//!
//! ```text
//! resolve → authorize → tier → snapshot → claim → invoke → observe → ledger
//! ```
//!
//! Six properties hold because of *where* each step sits, not because of a check
//! somewhere else:
//!
//! 1. **A denial short-circuits before any side effect.** Authorization runs before the
//!    idempotency claim, the snapshot and the invocation, in that order, so a refused
//!    action has nothing to undo and nothing to observe. The refusal is still *recorded*
//!    — a `tool_calls` row, a ledger entry and an [`ActionResult`] with status `denied` —
//!    because "nothing happened" is a fact the ledger has to be able to attest to.
//! 2. **The observation is built from the run, never supplied.** The only constructor
//!    call site is [`Executor::invoke_tool`]'s success arm, and it is handed the tool's
//!    own [`ToolOutcome`](crate::registry::ToolOutcome), a fresh evidence id and a fresh
//!    claim id. A plan, a model or a caller has no path to it.
//! 3. **The claim is checked by the barrier before it is recorded.** The executor calls
//!    [`mm_epistemic::ValidationBarrier::admit`] on the claim the observation produces,
//!    which is Phase 6's own door, and it never writes an observation whose claim the
//!    barrier would refuse.
//! 4. **Effects are exactly-once.** The idempotency key is claimed before the tool runs
//!    and finished after it, so a verbatim retry replays the recorded outcome.
//! 5. **A reversible action is snapshotted before it runs**, and its `touched_paths`
//!    reach the caller, so `action rollback` has something to restore.
//! 6. **Every step is on the ledger.** The events are the ledger's own closed set
//!    ([`crate::ledger::LEDGER_EVENTS`]), so a caller cannot invent one.
//!
//! # The observer, and what happens without one
//!
//! `ingest`ing the claim is how an observation reaches `/world`. It needs an
//! [`EpistemicEngine`](mm_epistemic::EpistemicEngine), which needs a logger and an
//! identifier factory — the same wiring `mm-cli` already does for its epistemic
//! commands. [`Executor::with_observer`] takes it. **Without an observer the executor
//! still builds the claim, still asks the barrier, and still records the observation and
//! its evidence; it does not mirror `/world`.** That is stated rather than silently
//! assumed: a run with no observer produces every record except the `/world` quad.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::action::{ActionResult, ActionSpec, ActionStatus};
use crate::error::{DupError, LedgerError, RegistryError, RollbackError, SandboxError, ToolError};
use crate::idempotency::{Begin, IdempotencyKey, IdempotencyStore};
use crate::ledger;
use crate::observation::{Observation, ObservationPayload, SourceRef};
use crate::permissions::{
    Decision, DenyReason, EscalationTarget, Grant, PermissionEngine, PermissionReq,
    PermissionRequest,
};
use crate::policy::PolicySet;
use crate::registry::{ExecCtx, ToolArgs, ToolOutcome, ToolRegistry};
use crate::rollback::Snapshot;
use crate::sandbox::{self, CapabilityGuard, SandboxCapabilities};
use crate::spec::{SideEffectClass, ToolSpec};
use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_store_graph::GraphHandle;
use mm_store_sqlite::SqliteStore;

/// Why the executor could not produce a recorded outcome.
///
/// A *decision* is not here: a denial, an escalation and a tool failure are all
/// [`ActionResult`]s, because each of them is something the ledger attests to. These are
/// the failures where no honest record could be produced.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// The tool is not registered.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// The ledger refused an entry, so the run is not recorded and must not proceed.
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    /// The idempotency guard refused.
    #[error(transparent)]
    Dup(#[from] DupError),
    /// A snapshot could not be taken.
    #[error(transparent)]
    Rollback(#[from] RollbackError),
    /// The sandbox tier cannot run here.
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    /// An irreversible action needs a person or the firewall.
    #[error("the action was escalated to {}: {reason}", to.as_str())]
    Escalate {
        /// Who must decide.
        to: EscalationTarget,
        /// Why.
        reason: String,
    },
    /// The principal is not a usable identifier.
    #[error("the principal is not valid: {0}")]
    Principal(String),
    /// The observation could not be built, which means the run cannot be recorded
    /// truthfully.
    #[error("the observation could not be recorded: {0}")]
    Observation(String),
    /// A store failure.
    #[error("store failure: {0}")]
    Store(String),
}

/// The pipeline.
pub struct Executor {
    registry: ToolRegistry,
    permissions: Arc<dyn PermissionEngine>,
    sql: SqliteStore,
    graph: GraphHandle,
    ids: Arc<UlidFactory>,
    sandbox_root: PathBuf,
    max_bytes: u64,
    observer: Option<Arc<mm_epistemic::EpistemicEngine>>,
}

impl std::fmt::Debug for Executor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Executor")
            .field("tools", &self.registry.len())
            .field("sandbox_root", &self.sandbox_root)
            .field("observer", &self.observer.is_some())
            .finish_non_exhaustive()
    }
}

impl Executor {
    /// An executor over a registry, an authorization engine, a store and a graph.
    pub fn new(
        registry: ToolRegistry,
        permissions: Arc<dyn PermissionEngine>,
        sql: SqliteStore,
        graph: GraphHandle,
        ids: Arc<UlidFactory>,
    ) -> Self {
        Executor {
            registry,
            permissions,
            sql,
            graph,
            ids,
            sandbox_root: PathBuf::from(crate::SANDBOX_ROOT),
            max_bytes: sandbox::DEFAULT_MAX_BYTES,
            observer: None,
        }
    }

    /// Wire the observer that admits observations to `/world`.
    pub fn with_observer(mut self, observer: Arc<mm_epistemic::EpistemicEngine>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Put the sandbox somewhere else. Tests need this; a deployment does not.
    pub fn with_sandbox_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.sandbox_root = root.into();
        self
    }

    /// Change the payload ceiling. A *smaller* ceiling than the default is a tightening
    /// and is honoured; a larger one is not.
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes.min(sandbox::DEFAULT_MAX_BYTES);
        self
    }

    /// The registry.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// The authorization engine.
    pub fn permissions(&self) -> &dyn PermissionEngine {
        self.permissions.as_ref()
    }

    /// The store.
    pub fn sql(&self) -> &SqliteStore {
        &self.sql
    }

    /// Run an action, deriving the idempotency key when the caller supplied none.
    pub async fn execute(&self, action: ActionSpec) -> Result<ActionResult, ExecError> {
        self.execute_with_key(action, None).await
    }

    /// Run an action under an explicit idempotency key.
    pub async fn execute_with_key(
        &self,
        action: ActionSpec,
        key: Option<IdempotencyKey>,
    ) -> Result<ActionResult, ExecError> {
        action.principal.validate().map_err(ExecError::Principal)?;
        let action_id = self.ids.next();
        let trace_id = self.ids.next();
        let started_at = Timestamp::now();

        // Resolve first: an unknown tool has no spec, so nothing after this could be
        // decided about it.
        let tool = self.registry.resolve(&action.tool)?;
        let spec = tool.spec().clone();
        let args = ToolArgs::new(&spec, action.args.clone()).map_err(|error| {
            ExecError::Registry(RegistryError::InvalidSpec {
                tool: spec.name.0.clone(),
                reason: error.to_string(),
            })
        })?;

        ledger::append(
            &self.sql,
            action_id,
            "action.proposed",
            &serde_json::json!({
                "tool": spec.name.as_str(),
                "principal": action.principal.as_str(),
                "args_hash": action.args_hash(),
                "trace_id": mm_core::ulid_string(&trace_id),
            }),
        )
        .await?;

        // ---- authorize -------------------------------------------------------
        let decision = self.decide(&action, &spec, action_id).await?;
        if !decision.is_allow() {
            return self
                .record_refusal(
                    &action, &spec, action_id, trace_id, started_at, &decision, key,
                )
                .await;
        }

        // ---- tier ------------------------------------------------------------
        if let Err(refusal) = sandbox::tier_available(spec.sandbox_tier) {
            return self
                .record_refusal(
                    &action,
                    &spec,
                    action_id,
                    trace_id,
                    started_at,
                    &Decision::Deny {
                        reason: DenyReason::NoGrant {
                            tool: Some(spec.name.0.clone()),
                            resource: refusal.to_string(),
                        },
                    },
                    key,
                )
                .await;
        }

        // ---- claim, snapshot, invoke ----------------------------------------
        let idempotency = IdempotencyStore::new(self.sql.clone());
        let key = key.unwrap_or_else(|| action.derived_idempotency_key());
        match idempotency.begin(&key, action_id).await? {
            Begin::Fresh => {}
            Begin::Replay(recorded) => {
                let deduplicated = ActionResult::deduplicated_from(action_id, *recorded);
                ledger::append(
                    &self.sql,
                    action_id,
                    "permission.check",
                    &serde_json::json!({
                        "decision": "allow",
                        "policy_effect": "permit",
                        "reason": "an identical call was already executed under this key",
                        "principal": action.principal.as_str(),
                        "idempotency_key": key.as_str(),
                    }),
                )
                .await?;
                self.write_call_row(
                    &action,
                    &spec,
                    action_id,
                    trace_id,
                    &deduplicated,
                    key.as_str(),
                    started_at,
                )
                .await?;
                return Ok(deduplicated);
            }
        }

        // Snapshotted before the tool runs, so the captured bytes are the pre-state.
        let _snapshots = self.snapshot(&action, &spec, action_id).await?;
        let outcome = self
            .invoke_tool(&action, &spec, args, action_id, trace_id, started_at)
            .await?;
        let result = match outcome {
            Ok((result, observation)) => {
                // Only a successful action is observed; a refusal or a failure is an
                // outcome the ledger attests to, not a claim about the world.
                if let Some(observation) = observation {
                    self.record_observation(&observation, action_id).await?;
                }
                // And its call row, which is the record that this action *ran*. The
                // refusal path writes its own and the replay path writes the row of the
                // call it replayed, so this is the only place a successful call is
                // written up — which is why it is written here rather than in the
                // refusal helper's neighbour.
                self.write_call_row(
                    &action,
                    &spec,
                    action_id,
                    trace_id,
                    &result,
                    key.as_str(),
                    started_at,
                )
                .await?;
                result
            }
            Err(result) => result,
        };
        idempotency.finish(&key, &result).await?;
        Ok(result)
    }

    /// Decide, applying the escalation the *tool's* reversibility requires.
    async fn decide(
        &self,
        action: &ActionSpec,
        spec: &ToolSpec,
        action_id: Ulid,
    ) -> Result<Decision, ExecError> {
        for declared in &spec.permissions {
            // The request is made about what this call *touches*, not about the subtree
            // the tool promises to stay in when it does.
            let concrete = self.concrete_request(action, declared);
            let req = concrete.as_ref().unwrap_or(declared);
            let request = PermissionRequest {
                principal: &action.principal,
                tool: Some(&action.tool),
                req,
                as_of: Timestamp::now(),
                confirmed: action
                    .rationale
                    .as_deref()
                    .map(|r| r.contains("confirmed"))
                    .unwrap_or(false),
            };
            let answer = self.permissions.authorize(&request);
            ledger::append(
                &self.sql,
                action_id,
                "permission.check",
                &serde_json::json!({
                    "tool": spec.name.as_str(),
                    "resource": req.canonical(),
                    "decision": answer.as_str(),
                    "policy_effect": crate::policy::PolicyEngine::evaluate(
                        self.permissions.policy_sets(),
                        &request,
                    )
                    .as_str(),
                    "reason": answer.reason_text(),
                    "principal": action.principal.as_str(),
                }),
            )
            .await?;
            match answer {
                Decision::Allow => {}
                other => return Ok(other),
            }
        }
        // An irreversible tool needs an explicit confirmation, whatever the grants say.
        if spec.reversibility == crate::spec::Reversibility::Irreversible {
            return Ok(Decision::Escalate {
                to: EscalationTarget::Operator,
                reason: format!(
                    "{} is irreversible, so it needs an explicit confirmation before it runs",
                    spec.name
                ),
            });
        }
        // Every declared capability is covered and the tool is reversible: it proceeds.
        Ok(Decision::Allow)
    }

    /// The request a declared capability is exercised on, when the action names its target.
    ///
    /// A tool *declares* a subtree (`fs:write:data/sandbox/**`) while a run asks about one
    /// path, and a rule that forbids part of that subtree is written the way the
    /// declaration is (`data/sandbox/forbidden/**`). Left at the declaration, such a rule
    /// could never match anything: the request would say "somewhere under `data/sandbox`"
    /// and a `forbid` an operator wrote would be silently inert. So the path is taken from
    /// the action's `path` argument — the same convention [`snapshot_paths`] relies on —
    /// resolved and re-spelled by [`crate::sandbox::spell_path`], and the policy is asked
    /// about that.
    ///
    /// `None` when the action names no path for the capability (`graph:read` and
    /// `net:connect` requests are about the declaration itself, and a `net` pattern is a
    /// host while the argument is a URL), when the path is not one the sandbox would let
    /// through — the guard refuses that by name, which is a better answer than a policy
    /// denial about a path the tool may never touch — or when the re-spelled path falls
    /// outside what the tool declared.
    fn concrete_request(
        &self,
        action: &ActionSpec,
        declared: &PermissionReq,
    ) -> Option<PermissionReq> {
        let target = match declared.kind() {
            "fs" => {
                let requested = action.args.get("path")?.as_str()?;
                crate::sandbox::spell_path(&self.sandbox_root, requested)?
            }
            _ => return None,
        };
        let concrete = PermissionReq::new(
            format!(
                "{}:{}:{}",
                declared.kind(),
                declared.action.as_str(),
                target.display()
            ),
            declared.action,
        );
        concrete.covered_by(declared).then_some(concrete)
    }

    /// Record a refusal and return it as an outcome rather than an error.
    async fn record_refusal(
        &self,
        action: &ActionSpec,
        spec: &ToolSpec,
        action_id: Ulid,
        trace_id: Ulid,
        started_at: Timestamp,
        decision: &Decision,
        key: Option<IdempotencyKey>,
    ) -> Result<ActionResult, ExecError> {
        let result = ActionResult {
            status: ActionStatus::Denied,
            reason: Some(decision.reason_text()),
            action_id,
            output: None,
            observation_id: None,
            evidence_id: None,
            deduplicated: false,
            latency_ms: 0,
            sandbox_tier: Some(spec.sandbox_tier.as_str().to_string()),
        };
        if let Some(reason) = decision.deny_reason() {
            let payload = match reason {
                DenyReason::PolicyForbidden { rule, policy_set } => serde_json::json!({
                    "tool": spec.name.as_str(),
                    "policy_set": policy_set,
                    "rule_id": rule,
                    "principal": action.principal.as_str(),
                }),
                other => serde_json::json!({
                    "tool": spec.name.as_str(),
                    "capability": other.code(),
                    "requested": action.args_canonical(),
                    "allowed": "the capability was not granted",
                }),
            };
            ledger::append(&self.sql, action_id, "sandbox.deny", &payload).await?;
        }
        ledger::append(
            &self.sql,
            action_id,
            "invoke.end",
            &serde_json::json!({
                "tool": spec.name.as_str(),
                "status": result.status.as_str(),
                "latency_ms": 0,
                "result_hash": result.result_hash(),
                "error": result.reason,
            }),
        )
        .await?;
        // The refusal path has no key yet: a denial never claimed one, so the row records
        // that no idempotency key was consumed rather than naming the one it would have.
        let key_text = key.as_ref().map(|k| k.0.as_str()).unwrap_or("");
        self.write_call_row(
            action, spec, action_id, trace_id, &result, key_text, started_at,
        )
        .await?;
        Ok(result)
    }

    /// Snapshot the paths a reversible local action may change.
    async fn snapshot(
        &self,
        action: &ActionSpec,
        spec: &ToolSpec,
        action_id: Ulid,
    ) -> Result<Vec<Snapshot>, ExecError> {
        let mut taken = Vec::new();
        for path in snapshot_paths(spec, action) {
            if let Some(snapshot) =
                crate::rollback::snapshot(&self.sql, action_id, spec.reversibility, &path).await?
            {
                taken.push(snapshot);
            }
        }
        Ok(taken)
    }

    /// Build the context, invoke the tool, and turn the outcome into a result and an
    /// observation.
    #[allow(clippy::too_many_arguments)]
    async fn invoke_tool(
        &self,
        action: &ActionSpec,
        spec: &ToolSpec,
        args: ToolArgs,
        action_id: Ulid,
        trace_id: Ulid,
        started_at: Timestamp,
    ) -> Result<Result<(ActionResult, Option<Observation>), ActionResult>, ExecError> {
        let effective = self.permissions.granted_capabilities(
            &action.principal,
            Some(&action.tool),
            &spec.permissions,
        );
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&effective, &self.sandbox_root, self.max_bytes),
            spec.sandbox_tier,
            self.sandbox_root.clone(),
        );
        let caps_hash = guard.capabilities().hash();
        ledger::append(
            &self.sql,
            action_id,
            "sandbox.start",
            &serde_json::json!({
                "tool": spec.name.as_str(),
                "tier": spec.sandbox_tier.as_str(),
                "caps_hash": caps_hash,
            }),
        )
        .await?;

        let ctx = ExecCtx {
            action_id,
            trace_id,
            action: action.clone(),
            spec: spec.clone(),
            guard,
            sql: self.sql.clone(),
            graph: self.graph.clone(),
            workdir: self.sandbox_root.clone(),
            started_at,
            confirmed: false,
        };
        let tool = self.registry.resolve(&action.tool)?;
        let (status, output, reason, observation) = match tool.invoke(&ctx, args).await {
            Ok(outcome) => {
                let observation = self.build_observation(action, spec, &outcome, action_id)?;
                (
                    ActionStatus::Ok,
                    Some(outcome.output.clone()),
                    None,
                    Some(observation),
                )
            }
            Err(ToolError::Sandbox(refusal)) => {
                ledger::append(
                    &self.sql,
                    action_id,
                    "sandbox.deny",
                    &serde_json::json!({
                        "tool": spec.name.as_str(),
                        "capability": refusal.capability(),
                        "requested": action.args_canonical(),
                        "allowed": refusal.to_string(),
                    }),
                )
                .await?;
                // The reason carries the refusal's stable code as well as its prose. The
                // ledger's `sandbox.deny` record names the capability, and an operator
                // reading a run's output should be able to match what they see against
                // the record — a message alone is not greppable.
                (
                    ActionStatus::Denied,
                    None,
                    Some(format!("{}: {refusal}", refusal.code())),
                    None,
                )
            }
            Err(error) => (ActionStatus::Error, None, Some(error.to_string()), None),
        };

        let latency_ms = elapsed_ms(started_at);
        let result = ActionResult {
            action_id,
            status,
            output,
            reason,
            observation_id: observation.as_ref().map(|o| o.id),
            evidence_id: observation.as_ref().map(|o| o.evidence_id),
            deduplicated: false,
            latency_ms,
            sandbox_tier: Some(spec.sandbox_tier.as_str().to_string()),
        };
        ledger::append(
            &self.sql,
            action_id,
            "invoke.end",
            &serde_json::json!({
                "tool": spec.name.as_str(),
                "status": result.status.as_str(),
                "latency_ms": latency_ms,
                "result_hash": result.result_hash(),
                "error": result.reason,
            }),
        )
        .await?;

        if result.status == ActionStatus::Denied {
            return Ok(Err(result));
        }
        Ok(Ok((result, observation)))
    }

    /// The only place an [`Observation`] is constructed.
    ///
    /// Every successful action gets exactly one, a pure read included. "Pure" is a
    /// statement about *side effects* — the read changed nothing — not about whether the
    /// world was observed, and a read is the clearest case of an action observing it: the
    /// digest of what was read is knowledge, and the phase's gate asks for exactly one
    /// `observation.record` with an `evidence_id` per successful action. A claim the
    /// barrier would refuse is refused when the observation is recorded (see
    /// [`Executor::record_observation`]), not by dropping the observation here.
    fn build_observation(
        &self,
        action: &ActionSpec,
        spec: &ToolSpec,
        outcome: &ToolOutcome,
        action_id: Ulid,
    ) -> Result<Observation, ExecError> {
        let endpoint = outcome
            .touched_paths
            .first()
            .or_else(|| outcome.artifacts.first())
            .cloned()
            .or_else(|| {
                action
                    .args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| spec.name.0.clone());
        let payload = ObservationPayload::new(
            outcome.summary.clone(),
            outcome.output.clone(),
            outcome.artifacts.clone(),
        );
        let observation = Observation::from_outcome(
            action_id,
            spec.name.clone(),
            SourceRef::tool(endpoint),
            payload,
            self.ids.next(),
            self.ids.next(),
        )
        .map_err(|e| ExecError::Observation(e.to_string()))?;
        Ok(observation)
    }

    /// Ask the barrier, write the observation, and hand the claim to the observer.
    async fn record_observation(
        &self,
        observation: &Observation,
        action_id: Ulid,
    ) -> Result<(), ExecError> {
        let (claim, evidence) = observation
            .to_claim()
            .map_err(|e| ExecError::Observation(e.to_string()))?;
        // Phase 6's door. An observation whose claim the barrier would refuse is not
        // recorded at all: recording it would be exactly the "fabricated observation" the
        // phase's adversarial fixtures look for.
        mm_epistemic::ValidationBarrier::admit(&claim)
            .map_err(|e| ExecError::Observation(e.to_string()))?;

        sqlx::query(
            "INSERT INTO observed_payloads \
             (id, action_id, source, payload_json, evidence_id, observed_at, graph_iri) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(mm_core::ulid_string(&observation.id))
        .bind(mm_core::ulid_string(&action_id))
        .bind(observation.source.canonical())
        .bind(observation.payload_json())
        .bind(mm_core::ulid_string(&observation.evidence_id))
        .bind(observation.observed_at.to_rfc3339())
        .bind(&observation.node_iri)
        .execute(self.sql.pool())
        .await
        .map_err(|e| ExecError::Store(e.to_string()))?;

        let quads = crate::rdf::observation_quads(observation);
        crate::rdf::mirror(&self.graph, quads)
            .await
            .map_err(|e| ExecError::Store(e.to_string()))?;

        ledger::append(
            &self.sql,
            action_id,
            "observation.record",
            &serde_json::json!({
                "observation_id": mm_core::ulid_string(&observation.id),
                "action_id": mm_core::ulid_string(&action_id),
                "source": observation.source.canonical(),
                "evidence_id": mm_core::ulid_string(&observation.evidence_id),
                "graph_iri": observation.node_iri,
            }),
        )
        .await?;

        if let Some(observer) = &self.observer {
            observer
                .ingest(&claim, &[evidence])
                .await
                .map_err(|e| ExecError::Observation(format!("/world ingestion failed: {e}")))?;
        }
        Ok(())
    }

    /// Write the `tool_calls` row.
    async fn write_call_row(
        &self,
        action: &ActionSpec,
        spec: &ToolSpec,
        action_id: Ulid,
        trace_id: Ulid,
        result: &ActionResult,
        key: &str,
        started_at: Timestamp,
    ) -> Result<(), ExecError> {
        sqlx::query(
            "INSERT INTO tool_calls \
             (id, tool_name, args_hash, args_json, permission_decision, sandbox_tier, status, \
              idempotency_key, trace_id, started_at, ended_at, latency_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(mm_core::ulid_string(&action_id))
        .bind(spec.name.as_str())
        .bind(action.args_hash())
        .bind(action.args_canonical())
        .bind(if result.status == ActionStatus::Denied {
            "deny"
        } else {
            "allow"
        })
        .bind(spec.sandbox_tier.as_str())
        .bind(result.status.as_str())
        .bind(key)
        .bind(mm_core::ulid_string(&trace_id))
        .bind(started_at.to_rfc3339())
        .bind(Timestamp::now().to_rfc3339())
        .bind(result.latency_ms as i64)
        .execute(self.sql.pool())
        .await
        .map_err(|e| ExecError::Store(e.to_string()))?;
        Ok(())
    }
}

/// The paths a reversible local action may change.
///
/// `ActionSpec::snapshot_paths`, plus the `path` argument when the tool declares itself
/// reversible and local and its schema names a `path`. That second half is a convention,
/// and it is written down here rather than inferred later: every tool in
/// [`crate::tools`] that writes takes its target as `path`, and a snapshot taken of the
/// wrong path is worse than no snapshot at all because it reports a restorable state
/// that is not.
pub fn snapshot_paths(spec: &ToolSpec, action: &ActionSpec) -> Vec<String> {
    let mut paths = action.snapshot_paths.clone();
    if spec.reversibility.is_snapshotted() && spec.side_effects == SideEffectClass::Local {
        if let Some(serde_json::Value::String(path)) = action.args.get("path") {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }
    paths
}

fn elapsed_ms(started_at: Timestamp) -> u64 {
    let now = Timestamp::now();
    now.as_nanos()
        .saturating_sub(started_at.as_nanos())
        .checked_div(1_000_000)
        .unwrap_or(0) as u64
}

/// The engine's answer, as the ledger records it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordedDecision {
    /// The decision.
    pub decision: String,
    /// Why.
    pub reason: String,
}

/// One row of `permission_grants`, in the order the query below selects it.
///
/// Named rather than written inline: `query_as` wants the row type spelled out,
/// and the eight-column tuple is unreadable in the middle of the call.
type GrantRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);

/// Read the grants and policy sets a store holds.
///
/// The engine this returns is the one `mm-cli tool run` builds and the one `mm-cli policy
/// check` builds, so the two cannot disagree about a decision.
pub async fn load_engine(
    sql: &SqliteStore,
) -> Result<crate::permissions::TablePermissionEngine, mm_core::MmError> {
    let grant_rows: Vec<GrantRow> = sqlx::query_as(
        "SELECT id, principal, scope, tool_pattern, policy_set_id, granted_by, expires_at, \
             revoked_at FROM permission_grants ORDER BY id",
    )
    .fetch_all(sql.pool())
    .await
    .map_err(|e| mm_core::MmError::Store(e.to_string()))?;
    let grants: Vec<Grant> = grant_rows
        .into_iter()
        .map(
            |(
                id,
                principal,
                scope,
                tool_pattern,
                policy_set,
                granted_by,
                expires_at,
                revoked_at,
            )| {
                Grant {
                    id,
                    principal: crate::permissions::Principal(principal),
                    scope,
                    tool_pattern,
                    policy_set,
                    granted_by,
                    expires_at: expires_at
                        .as_deref()
                        .and_then(|t| Timestamp::from_rfc3339(t).ok()),
                    revoked_at: revoked_at
                        .as_deref()
                        .and_then(|t| Timestamp::from_rfc3339(t).ok()),
                }
            },
        )
        .collect();

    let set_rows: Vec<(String, String, i64, String)> = sqlx::query_as(
        "SELECT id, name, version, rules_json FROM policy_sets ORDER BY name, version",
    )
    .fetch_all(sql.pool())
    .await
    .map_err(|e| mm_core::MmError::Store(e.to_string()))?;
    let mut sets: Vec<PolicySet> = Vec::new();
    for (id, name, version, rules_json) in set_rows {
        let rules = PolicySet::parse_rules(&rules_json)
            .map_err(|e| mm_core::MmError::Store(format!("policy set {name} is invalid: {e}")))?;
        sets.push(PolicySet {
            id,
            name,
            version: version.max(0) as u32,
            rules,
        });
    }
    Ok(crate::permissions::TablePermissionEngine::new(grants, sets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{PermissionAction, PermissionReq, Principal, TablePermissionEngine};
    use crate::sandbox::SandboxTier;
    use crate::spec::{JsonSchema, Reversibility, ToolAnnotations};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A harness: a tool registry, an engine, a store and a temp sandbox.
    struct Harness {
        dir: tempfile::TempDir,
        executor: Executor,
        calls: Arc<AtomicUsize>,
    }

    struct CountingWriteTool {
        spec: ToolSpec,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl crate::registry::Tool for CountingWriteTool {
        fn spec(&self) -> &ToolSpec {
            &self.spec
        }

        async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let path = ctx.arg_str("path")?;
            let resolved = ctx.write_path(&path)?;
            let text = ctx.arg_str("text")?;
            std::fs::write(&resolved, text.as_bytes())
                .map_err(|e| ctx.failed(format!("cannot write: {e}")))?;
            Ok(ctx
                .outcome(
                    serde_json::json!({ "path": resolved.display().to_string(), "bytes": text.len() }),
                    format!("wrote {} bytes to {}", text.len(), resolved.display()),
                )
                .touching(vec![resolved.display().to_string()]))
        }
    }

    async fn harness(grants: Vec<Grant>) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let sql = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        sql.migrate().await.unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl");
        let graph = Box::leak(Box::new(
            mm_store_graph::GraphStore::in_memory(&shapes)
                .await
                .unwrap(),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = ToolRegistry::new();
        registry
            .register(Arc::new(CountingWriteTool {
                spec: write_spec(),
                calls: calls.clone(),
            }))
            .unwrap();
        let sets = vec![
            PolicySet::new("01h0000000000000000000p100", "baseline", 1).with_rule(
                crate::policy::PolicyRule::permit("*", "fs.write", "data/sandbox/**"),
            ),
        ];
        let engine = TablePermissionEngine::new(grants, sets);
        let executor = Executor::new(
            registry,
            Arc::new(engine),
            sql,
            graph.handle().clone(),
            Arc::new(UlidFactory::new()),
        )
        .with_sandbox_root(&root);
        Harness {
            dir,
            executor,
            calls,
        }
    }

    fn write_spec() -> ToolSpec {
        ToolSpec {
            name: crate::spec::ToolName::new("fs.write").unwrap(),
            version: "0.1.0".into(),
            description: "write a file".into(),
            input_schema: JsonSchema::object_with_strings(&["path", "text"]),
            output_schema: JsonSchema::any_object(),
            permissions: vec![
                PermissionReq::new("fs:write:data/sandbox/**", PermissionAction::Write),
                PermissionReq::new("fs:read:data/sandbox/**", PermissionAction::Read),
            ],
            reversibility: Reversibility::Reversible,
            side_effects: SideEffectClass::Local,
            annotations: ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: true,
                open_world: false,
            },
            module_uri: "https://metamind.dev/code/module/tools/fs-write".into(),
            sandbox_tier: SandboxTier::WasmCaps,
        }
    }

    /// The grants a caller that may write inside the sandbox needs.
    ///
    /// Both of them, because `fs.write` declares both: it reads the file it is about
    /// to replace, and the permission engine authorizes every capability a spec
    /// declares rather than letting a write stand in for a read. The Phase 10 seed
    /// grants `system` the same pair.
    fn write_grants() -> Vec<Grant> {
        vec![
            Grant::new(
                "01h0000000000000000000g100",
                Principal::system(),
                "fs:write:data/sandbox/**",
                "fs.*",
                "operator",
            ),
            Grant::new(
                "01h0000000000000000000g101",
                Principal::system(),
                "fs:read:data/sandbox/**",
                "fs.*",
                "operator",
            ),
        ]
    }

    fn action(target: &std::path::Path) -> ActionSpec {
        ActionSpec::new(
            crate::spec::ToolName::new("fs.write").unwrap(),
            serde_json::json!({ "path": target.to_string_lossy(), "text": "hello" }),
            Principal::system(),
        )
    }

    #[tokio::test]
    async fn an_authorized_write_runs_once_records_an_observation_and_chains_the_ledger() {
        let harness = harness(write_grants()).await;
        let target = harness.dir.path().join("sandbox").join("out.txt");
        let key = IdempotencyKey::new("k1").unwrap();

        let result = harness
            .executor
            .execute_with_key(action(&target), Some(key.clone()))
            .await
            .unwrap();
        assert_eq!(result.status, ActionStatus::Ok, "{result:?}");
        assert_eq!(harness.calls.load(Ordering::SeqCst), 1);
        assert!(target.exists());
        assert!(
            result.observation_id.is_some(),
            "an effect produces an observation"
        );
        assert!(result.evidence_id.is_some());

        // The ledger is intact and ends with the observation.
        ledger::verify_chain(&harness.executor.sql).await.unwrap();
        let entries = ledger::entries_for(&harness.executor.sql, &result.action_id)
            .await
            .unwrap();
        assert_eq!(entries.last().unwrap().event, "observation.record");

        // A retry under the same key replays the outcome and does not re-apply the effect.
        let replay = harness
            .executor
            .execute_with_key(action(&target), Some(key))
            .await
            .unwrap();
        assert!(replay.deduplicated);
        assert_eq!(harness.calls.load(Ordering::SeqCst), 1, "the tool ran once");
    }

    #[tokio::test]
    async fn a_denial_short_circuits_before_any_side_effect() {
        let harness = harness(Vec::new()).await;
        let target = harness.dir.path().join("sandbox").join("denied.txt");
        let result = harness.executor.execute(action(&target)).await.unwrap();
        assert_eq!(result.status, ActionStatus::Denied);
        assert_eq!(
            harness.calls.load(Ordering::SeqCst),
            0,
            "the tool never ran"
        );
        assert!(!target.exists());
        assert!(result.reason.unwrap().contains("no grant"));

        let entries = ledger::entries_for(&harness.executor.sql, &result.action_id)
            .await
            .unwrap();
        let events: Vec<&str> = entries.iter().map(|e| e.event.as_str()).collect();
        assert!(events.contains(&"sandbox.deny"), "{events:?}");
        assert_eq!(events.last().copied(), Some("invoke.end"));
        assert!(
            !events.contains(&"sandbox.start"),
            "a denial never enters the sandbox: {events:?}"
        );
    }

    #[tokio::test]
    async fn an_unregistered_tool_is_an_error_not_a_denial() {
        let harness = harness(write_grants()).await;
        let action = ActionSpec::new(
            crate::spec::ToolName::new("fs.read").unwrap(),
            serde_json::json!({ "path": "x" }),
            Principal::system(),
        );
        let error = harness.executor.execute(action).await.unwrap_err();
        assert!(matches!(error, ExecError::Registry(_)), "{error}");
    }

    #[tokio::test]
    async fn a_reversible_local_write_is_snapshotted_before_it_runs() {
        let harness = harness(write_grants()).await;
        let target = harness.dir.path().join("sandbox").join("snap.txt");
        std::fs::write(&target, "before").unwrap();
        let result = harness.executor.execute(action(&target)).await.unwrap();
        assert_eq!(result.status, ActionStatus::Ok);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");

        let restored = crate::rollback::rollback(&harness.executor.sql, &result.action_id)
            .await
            .unwrap();
        assert_eq!(restored.len(), 64);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "before");
    }

    #[tokio::test]
    async fn an_illegal_principal_is_refused_before_the_ledger_is_touched() {
        let harness = harness(write_grants()).await;
        let mut action = action(&harness.dir.path().join("sandbox").join("x.txt"));
        action.principal = Principal(" ".to_string());
        let error = harness.executor.execute(action).await.unwrap_err();
        assert!(matches!(error, ExecError::Principal(_)), "{error}");
        assert_eq!(
            ledger::verify_chain(&harness.executor.sql).await.unwrap(),
            0,
            "nothing was recorded"
        );
    }

    #[tokio::test]
    async fn the_snapshot_paths_convention_is_the_documented_one() {
        let spec = write_spec();
        let action = ActionSpec::new(
            spec.name.clone(),
            serde_json::json!({ "path": "data/sandbox/a.txt", "text": "x" }),
            Principal::system(),
        );
        assert_eq!(
            snapshot_paths(&spec, &action),
            vec!["data/sandbox/a.txt".to_string()]
        );

        let explicit = action.clone().with_snapshot_paths(vec!["other.txt".into()]);
        assert_eq!(
            snapshot_paths(&spec, &explicit),
            vec!["other.txt".to_string(), "data/sandbox/a.txt".to_string()]
        );

        let mut pure = write_spec();
        pure.side_effects = SideEffectClass::None;
        pure.annotations = ToolAnnotations::pure();
        assert!(
            snapshot_paths(&pure, &action).is_empty(),
            "a pure tool changes no path"
        );
    }
}
