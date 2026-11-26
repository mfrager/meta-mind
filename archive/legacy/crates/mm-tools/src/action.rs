//! What is asked for, and what came back.
//!
//! An [`ActionSpec`] is the *request*: a tool, its arguments, and the identity the
//! caller acts as. It carries no outcome and no permission — those are the
//! executor's, and a type that could hold both would let a caller construct an
//! action that had already succeeded.
//!
//! Canonical rendering is a contract here, not a convenience. The same action must
//! render to the same bytes across processes, because three things key on it: the
//! `args_hash` in `tool_calls`, the idempotency key a caller derives from an action,
//! and the ledger payload. `canonical_json` sorts object keys at every depth, so
//! `ActionSpec::digest` is stable against a re-parse of its own output — which is
//! the property the phase's property test asserts.

use serde::{Deserialize, Serialize};

use crate::permissions::Principal;
use crate::spec::ToolName;

/// One requested action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionSpec {
    /// The tool to call.
    pub tool: ToolName,
    /// The arguments, validated against the tool's declared input schema.
    pub args: serde_json::Value,
    /// Who is acting. Every call carries one: a tool that acted with wider rights
    /// than its caller is the confused-deputy failure the phase plan names.
    pub principal: Principal,
    /// A caller-supplied rationale, for the ledger. Never used to authorize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// The filesystem paths this action may take a snapshot of, when the tool writes.
    ///
    /// Declared by the caller rather than guessed by the executor: a snapshot is a
    /// copy of a file, and an executor that decided on its own which paths an
    /// arbitrary tool would touch would be either wrong or copying the whole tree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshot_paths: Vec<String>,
}

impl ActionSpec {
    /// A request.
    pub fn new(tool: ToolName, args: serde_json::Value, principal: Principal) -> Self {
        ActionSpec {
            tool,
            args,
            principal,
            rationale: None,
            snapshot_paths: Vec::new(),
        }
    }

    /// A request with a rationale.
    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = Some(rationale.into());
        self
    }

    /// A request that declares the paths it may write.
    pub fn with_snapshot_paths(mut self, paths: Vec<String>) -> Self {
        self.snapshot_paths = paths;
        self
    }

    /// The canonical rendering of the arguments.
    pub fn args_canonical(&self) -> String {
        crate::canonical_json(&self.args)
    }

    /// `sha256` of the canonical arguments — the `tool_calls.args_hash` value.
    ///
    /// The principal is *not* part of it: two principals making the identical call
    /// are making the identical call, and a hash that changed with the caller would
    /// make the idempotency guard useless for the case it exists for (a retry).
    pub fn args_hash(&self) -> String {
        mm_core::content_hash(self.args_canonical().as_bytes())
    }

    /// `sha256` of the whole request, including the principal.
    pub fn digest(&self) -> String {
        mm_core::content_hash(self.canonical().as_bytes())
    }

    /// A stable rendering of the whole request.
    pub fn canonical(&self) -> String {
        format!(
            "tool={}\nprincipal={}\nargs={}\nrationale={}\nsnapshot_paths={}",
            self.tool,
            self.principal,
            self.args_canonical(),
            self.rationale.as_deref().unwrap_or("-"),
            self.snapshot_paths.join(",")
        )
    }

    /// Derive the idempotency key a retry of this action would use.
    ///
    /// A caller that has not been handed a key still gets exactly-once semantics for
    /// a *verbatim* retry: the key is a function of the tool, the principal and the
    /// arguments, so re-issuing the identical call cannot double-apply it.
    pub fn derived_idempotency_key(&self) -> crate::idempotency::IdempotencyKey {
        crate::idempotency::IdempotencyKey(format!(
            "auto:{}",
            mm_core::hash_fields(&[self.tool.as_str(), &self.principal.0, &self.args_hash()])
        ))
    }
}

/// How an action ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    /// It ran and produced a result.
    Ok,
    /// It ran and failed.
    Error,
    /// It was refused before it ran.
    Denied,
    /// It ran past its deadline.
    Timeout,
    /// It did not run because an identical call already had.
    Deduplicated,
}

/// Every status, in the order the CLI reports them.
pub const ACTION_STATUSES: [ActionStatus; 5] = [
    ActionStatus::Ok,
    ActionStatus::Error,
    ActionStatus::Denied,
    ActionStatus::Timeout,
    ActionStatus::Deduplicated,
];

impl ActionStatus {
    /// The stable wire name, which is also the `tool_calls.status` value.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionStatus::Ok => "ok",
            ActionStatus::Error => "error",
            ActionStatus::Denied => "denied",
            ActionStatus::Timeout => "timeout",
            ActionStatus::Deduplicated => "deduplicated",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        ACTION_STATUSES.into_iter().find(|s| s.as_str() == text)
    }

    /// True when the action had an effect on the world.
    ///
    /// A deduplicated call had no *new* effect, which is exactly why it is not
    /// counted here: the phase's completeness check counts effects, and counting the
    /// retry would report two.
    pub fn is_effect(self) -> bool {
        matches!(self, ActionStatus::Ok | ActionStatus::Error)
    }

    /// True when the action never ran.
    pub fn is_refusal(self) -> bool {
        matches!(self, ActionStatus::Denied)
    }
}

impl std::fmt::Display for ActionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a call produced, or why it did not.
///
/// Both the value and the reason live here because the ledger entry for a failure
/// has to name the cause: an action that failed with no recorded reason is an action
/// nobody can act on, and the phase's logging table requires the full cause chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    /// The action's own ULID, which is also the idempotency and snapshot key.
    pub action_id: mm_core::Ulid,
    /// How it ended.
    pub status: ActionStatus,
    /// The tool's output, when it ran successfully.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    /// Why it failed or was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The observation recorded from this run, when an effect happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<mm_core::Ulid>,
    /// The Phase 6 evidence id the observation stands on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<mm_core::Ulid>,
    /// True when an identical call had already run under the same key.
    #[serde(default)]
    pub deduplicated: bool,
    /// How long the call took, in milliseconds.
    #[serde(default)]
    pub latency_ms: u64,
    /// The sandbox tier it ran in, when it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_tier: Option<String>,
}

impl ActionResult {
    /// A successful run.
    pub fn ok(action_id: mm_core::Ulid, output: serde_json::Value) -> Self {
        ActionResult {
            action_id,
            status: ActionStatus::Ok,
            output: Some(output),
            reason: None,
            observation_id: None,
            evidence_id: None,
            deduplicated: false,
            latency_ms: 0,
            sandbox_tier: None,
        }
    }

    /// A refusal, which never had a side effect.
    pub fn denied(action_id: mm_core::Ulid, reason: impl Into<String>) -> Self {
        ActionResult {
            status: ActionStatus::Denied,
            reason: Some(reason.into()),
            ..ActionResult::ok(action_id, serde_json::Value::Null)
        }
    }

    /// A failure after the action ran.
    pub fn error(action_id: mm_core::Ulid, reason: impl Into<String>) -> Self {
        ActionResult {
            status: ActionStatus::Error,
            reason: Some(reason.into()),
            ..ActionResult::ok(action_id, serde_json::Value::Null)
        }
    }

    /// The recorded outcome of an earlier identical call.
    pub fn deduplicated_from(action_id: mm_core::Ulid, mut original: ActionResult) -> Self {
        original.action_id = action_id;
        original.deduplicated = true;
        original.latency_ms = 0;
        original
    }

    /// The hash of what the action produced, for the `tool.invoke.end` record.
    pub fn result_hash(&self) -> String {
        let mut fields = vec![self.status.as_str(), self.reason.as_deref().unwrap_or("")];
        let output = self
            .output
            .as_ref()
            .map(crate::canonical_json)
            .unwrap_or_default();
        fields.push(&output);
        mm_core::hash_fields(&fields)
    }

    /// A stable rendering.
    pub fn canonical(&self) -> String {
        format!(
            "action={}\nstatus={}\noutput={}\nreason={}\nobservation={}\nevidence={}\ndeduplicated={}",
            mm_core::ulid_string(&self.action_id),
            self.status.as_str(),
            self.output
                .as_ref()
                .map(crate::canonical_json)
                .unwrap_or_else(|| "-".to_string()),
            self.reason.as_deref().unwrap_or("-"),
            self.observation_id
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_else(|| "-".to_string()),
            self.evidence_id
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_else(|| "-".to_string()),
            self.deduplicated
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action() -> ActionSpec {
        ActionSpec::new(
            ToolName::new("fs.read").unwrap(),
            serde_json::json!({ "path": "data/sandbox/README.md", "encoding": "utf8" }),
            Principal::being("01h00000000000000000000b01".to_string()),
        )
    }

    #[test]
    fn the_canonical_rendering_is_stable_across_key_order() {
        let a = ActionSpec::new(
            ToolName::new("fs.read").unwrap(),
            serde_json::json!({ "path": "x", "encoding": "utf8" }),
            Principal::system(),
        );
        let b = ActionSpec::new(
            ToolName::new("fs.read").unwrap(),
            serde_json::json!({ "encoding": "utf8", "path": "x" }),
            Principal::system(),
        );
        assert_eq!(a.args_canonical(), b.args_canonical());
        assert_eq!(a.args_hash(), b.args_hash());
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn the_args_hash_ignores_the_principal_and_the_digest_does_not() {
        let one = action();
        let mut other = one.clone();
        other.principal = Principal::system();
        assert_eq!(one.args_hash(), other.args_hash());
        assert_ne!(one.digest(), other.digest());
    }

    #[test]
    fn the_same_call_derives_the_same_idempotency_key() {
        let a = action();
        let b = action();
        assert_eq!(a.derived_idempotency_key(), b.derived_idempotency_key());

        let mut changed = a.clone();
        changed.args = serde_json::json!({ "path": "other", "encoding": "utf8" });
        assert_ne!(
            a.derived_idempotency_key(),
            changed.derived_idempotency_key(),
            "a different argument must not be deduplicated against the first"
        );
    }

    #[test]
    fn a_canonical_rendering_round_trips_through_json() {
        let original = action();
        let text = serde_json::to_string(&original).unwrap();
        let back: ActionSpec = serde_json::from_str(&text).unwrap();
        assert_eq!(original.canonical(), back.canonical());
        assert_eq!(original.digest(), back.digest());
    }

    #[test]
    fn statuses_round_trip_and_classify() {
        for status in ACTION_STATUSES {
            assert_eq!(ActionStatus::parse(status.as_str()), Some(status));
        }
        assert!(ActionStatus::Ok.is_effect());
        assert!(ActionStatus::Error.is_effect());
        assert!(!ActionStatus::Deduplicated.is_effect());
        assert!(ActionStatus::Denied.is_refusal());
        assert!(!ActionStatus::Denied.is_effect(), "a refusal has no effect");
    }

    #[test]
    fn a_deduplicated_result_keeps_the_original_outcome() {
        let id = mm_core::Ulid::from_parts(1_700_000_000_000, 7);
        let original = ActionResult::ok(
            mm_core::Ulid::from_parts(1_700_000_000_000, 1),
            serde_json::json!({ "bytes_written": 5 }),
        );
        let deduped = ActionResult::deduplicated_from(id, original);
        assert!(deduped.deduplicated);
        assert_eq!(deduped.status, ActionStatus::Ok);
        assert_eq!(deduped.action_id, id);
        assert_eq!(deduped.output.unwrap()["bytes_written"], 5);
    }

    #[test]
    fn a_denied_action_carries_no_output() {
        let id = mm_core::Ulid::from_parts(1_700_000_000_000, 2);
        let denied = ActionResult::denied(id, "no grant covers this action");
        assert_eq!(denied.status, ActionStatus::Denied);
        assert_eq!(denied.output, Some(serde_json::Value::Null));
        assert!(denied.reason.as_deref().unwrap().contains("no grant"));
    }
}
