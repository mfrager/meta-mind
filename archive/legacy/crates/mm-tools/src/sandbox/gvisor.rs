//! Tier 2: syscall interception, behind a runtime that this build does not link by
//! default.
//!
//! gVisor's shape is a user-space application kernel that intercepts syscalls, so a
//! tool sees a Linux interface while the host sees a filtered one. Nothing about that
//! can be done inside the process being filtered, which is why this tier is a
//! *lifecycle* rather than a guard: it creates a sandbox, starts it, collects the
//! artifacts, and destroys it, and the work between those steps happens in the
//! runtime.
//!
//! What this module owns is the part that must be identical whatever the runtime is:
//!
//! * **The state machine.** `created → running → collected → destroyed`, with every
//!   illegal transition refused rather than ignored. That is what makes "no state
//!   leaks between actions" a property of the code and not of the operator's
//!   discipline: a handle that was not destroyed cannot be reused, and there is no
//!   transition that returns to `running` from `destroyed`.
//! * **The pre-flight capability check.** The tier-2 sandbox still derives the
//!   capabilities from the tool's declaration and still refuses an undeclared path or
//!   host *before* the runtime is asked to do anything, so a denial costs nothing and
//!   is produced by the same code as tier 1.
//! * **The refusal when the runtime is absent.** [`AbsentRuntime`] reports
//!   [`SandboxError::TierUnavailable`] with the reason, instead of silently running
//!   the tool in-process — which would be the one failure mode that turns a sandbox
//!   into a decoration.
//!
//! The `gvisor` feature gates the *runtime probe* only. With it off (the default) the
//! probe is unconditionally absent with a stated reason; with it on, the probe looks
//! for the configured runsc endpoint and still reports absent when there is none.
//! Either way the lifecycle and its tests compile and run.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::SandboxError;
use crate::spec::ToolSpec;

use super::{CapabilityGuard, Sandbox, SandboxCapabilities, SandboxTier};

/// Where a tier-2 handle is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxState {
    /// The handle exists and nothing has run.
    Created,
    /// An action is running in it.
    Running,
    /// The artifacts were collected; the sandbox is still alive but idle.
    Collected,
    /// The sandbox is gone and the handle cannot be reused.
    Destroyed,
}

/// Every state, in lifecycle order.
pub const SANDBOX_STATES: [SandboxState; 4] = [
    SandboxState::Created,
    SandboxState::Running,
    SandboxState::Collected,
    SandboxState::Destroyed,
];

impl SandboxState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxState::Created => "created",
            SandboxState::Running => "running",
            SandboxState::Collected => "collected",
            SandboxState::Destroyed => "destroyed",
        }
    }
}

/// Something that can actually run an action in an isolated environment.
///
/// The trait exists so the lifecycle can be tested without gVisor, Firecracker or a
/// network: [`AbsentRuntime`] is the production default on a machine with neither, and
/// a test installs a fake to prove the transitions and the executable-identity check.
pub trait SandboxRuntime: Send + Sync {
    /// The runtime's name, for the `tool.sandbox.start` record.
    fn name(&self) -> &'static str;

    /// Whether it can run, and why not when it cannot.
    fn probe(&self) -> Result<(), String>;

    /// Run one action, returning the payload the runtime produced.
    fn exec(&self, action: &crate::action::ActionSpec) -> Result<serde_json::Value, String>;
}

/// A runtime that is not installed.
#[derive(Clone, Copy, Debug, Default)]
pub struct AbsentRuntime;

impl SandboxRuntime for AbsentRuntime {
    fn name(&self) -> &'static str {
        "absent"
    }

    fn probe(&self) -> Result<(), String> {
        Err(TRUSTED_RUNTIME_REASON.to_string())
    }

    fn exec(&self, _action: &crate::action::ActionSpec) -> Result<serde_json::Value, String> {
        Err(TRUSTED_RUNTIME_REASON.to_string())
    }
}

/// Why no tier-2 runtime is available, in one place so every refusal says the same
/// thing.
pub const TRUSTED_RUNTIME_REASON: &str = "no syscall-interception runtime is installed: \
     the `gvisor` feature is off (or no runsc endpoint is configured), and this build \
     will not run an untrusted tool in-process instead";

/// One sandbox's life.
#[derive(Clone, Debug)]
pub struct SandboxLifecycle {
    state: SandboxState,
    runtime: String,
    action: String,
    exec_calls: u32,
}

impl SandboxLifecycle {
    /// Create a handle in `created`.
    pub fn create(runtime: &str, action_id: &str) -> Self {
        SandboxLifecycle {
            state: SandboxState::Created,
            runtime: runtime.to_string(),
            action: action_id.to_string(),
            exec_calls: 0,
        }
    }

    /// The current state.
    pub fn state(&self) -> SandboxState {
        self.state
    }

    /// How many times this handle has been handed an action.
    pub fn exec_calls(&self) -> u32 {
        self.exec_calls
    }

    /// Whether the handle has been destroyed.
    pub fn is_destroyed(&self) -> bool {
        self.state == SandboxState::Destroyed
    }

    /// Move to `running`, or refuse.
    pub fn start(&mut self) -> Result<(), SandboxError> {
        match self.state {
            SandboxState::Created | SandboxState::Collected => {
                self.state = SandboxState::Running;
                self.exec_calls += 1;
                Ok(())
            }
            other => Err(SandboxError::TierUnavailable {
                tier: self.runtime.clone(),
                reason: format!(
                    "a sandbox in state {} cannot start; it must be created or collected",
                    other.as_str()
                ),
            }),
        }
    }

    /// Move to `collected`, or refuse.
    pub fn collect(&mut self) -> Result<(), SandboxError> {
        if self.state != SandboxState::Running {
            return Err(SandboxError::TierUnavailable {
                tier: self.runtime.clone(),
                reason: format!(
                    "a sandbox in state {} has nothing to collect",
                    self.state.as_str()
                ),
            });
        }
        self.state = SandboxState::Collected;
        Ok(())
    }

    /// Move to `destroyed`. Idempotent: destroying a destroyed sandbox is a no-op
    /// rather than an error, because the caller's intent ("this is gone") is already
    /// true and failing would only tempt a caller to skip the call.
    pub fn destroy(&mut self) -> Result<(), SandboxError> {
        self.state = SandboxState::Destroyed;
        Ok(())
    }

    /// The action this handle was created for.
    pub fn action_id(&self) -> &str {
        &self.action
    }
}

/// Tier 2's sandbox.
pub struct GvisorSandbox {
    root: PathBuf,
    max_bytes: u64,
    runtime: Box<dyn SandboxRuntime>,
}

impl std::fmt::Debug for GvisorSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GvisorSandbox")
            .field("root", &self.root)
            .field("max_bytes", &self.max_bytes)
            .field("runtime", &self.runtime.name())
            .finish()
    }
}

impl GvisorSandbox {
    /// A tier-2 sandbox with the default (absent) runtime.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        GvisorSandbox {
            root: root.into(),
            max_bytes: crate::sandbox::wasi::DEFAULT_MAX_BYTES,
            runtime: Box::new(AbsentRuntime),
        }
    }

    /// A tier-2 sandbox over an explicit runtime.
    ///
    /// This is the seam the tests use, and it is also how a deployment with a real
    /// runsc endpoint would be wired: the runtime is injected rather than probed for
    /// in a constructor, so a missing runtime is a per-call refusal.
    pub fn with_runtime(mut self, runtime: Box<dyn SandboxRuntime>) -> Self {
        self.runtime = runtime;
        self
    }

    /// The runtime's name.
    pub fn runtime_name(&self) -> &'static str {
        self.runtime.name()
    }

    /// Run an action in the sandbox: probe, create, start, exec, collect, destroy.
    ///
    /// Every step is a refusal rather than a warning, and the handle is destroyed on
    /// the way out whatever happened, so a failed exec cannot leave a live sandbox
    /// holding state that the next action would inherit.
    pub fn run(
        &self,
        action: &crate::action::ActionSpec,
    ) -> Result<serde_json::Value, SandboxError> {
        self.runtime
            .probe()
            .map_err(|reason| SandboxError::TierUnavailable {
                tier: SandboxTier::Gvisor.as_str().to_string(),
                reason,
            })?;
        let handle = action_handle(action);
        let mut lifecycle = SandboxLifecycle::create(self.runtime.name(), &handle);
        let outcome = (|| {
            lifecycle.start()?;
            self.runtime
                .exec(action)
                .map_err(|reason| SandboxError::TierUnavailable {
                    tier: SandboxTier::Gvisor.as_str().to_string(),
                    reason,
                })
        })();
        let _ = lifecycle.collect();
        lifecycle.destroy()?;
        debug_assert!(
            lifecycle.is_destroyed(),
            "the handle is destroyed on every path"
        );
        outcome
    }
}

impl Sandbox for GvisorSandbox {
    fn tier(&self) -> SandboxTier {
        SandboxTier::Gvisor
    }

    fn capabilities(&self, spec: &ToolSpec) -> SandboxCapabilities {
        SandboxCapabilities::from_permissions(&spec.permissions, &self.root, self.max_bytes)
    }

    fn guard(&self, spec: &ToolSpec) -> CapabilityGuard {
        CapabilityGuard::new(self.capabilities(spec), self.tier(), self.root.clone())
    }

    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
}

/// A stable handle for an action, for the lifecycle record.
///
/// Derived from the action's digest rather than drawn at random, so two runs of the
/// same action name the same handle: the lifecycle record then answers "which action
/// was this sandbox for" unambiguously, and a test can assert the identity without
/// threading an id through every call.
///
/// The first byte is masked to three bits because a ULID's leading 48 bits are a
/// timestamp and `from_bytes` requires them to fit; the handle is an identifier, not
/// a clock reading.
pub(crate) fn action_handle(action: &crate::action::ActionSpec) -> String {
    let digest = action.digest();
    let bytes = digest.as_bytes();
    let mut parts = [0u8; 16];
    for (index, slot) in parts.iter_mut().enumerate() {
        let high = hex_nibble(bytes[index * 2]) << 4;
        let low = hex_nibble(bytes[index * 2 + 1]);
        *slot = high | low;
    }
    parts[0] &= 0b0000_0111;
    mm_core::ulid_string(&mm_core::Ulid::from_bytes(parts))
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::Principal;
    use crate::spec::ToolName;

    fn action() -> crate::action::ActionSpec {
        crate::action::ActionSpec::new(
            ToolName::new("fs.read").unwrap(),
            serde_json::json!({ "path": "data/sandbox/README.md" }),
            Principal::system(),
        )
    }

    #[test]
    fn the_lifecycle_refuses_illegal_transitions() {
        let mut lifecycle = SandboxLifecycle::create("fake", "01h00000000000000000000001");
        assert_eq!(lifecycle.state(), SandboxState::Created);
        assert!(lifecycle.collect().is_err(), "nothing has run yet");
        lifecycle.start().unwrap();
        assert_eq!(lifecycle.state(), SandboxState::Running);
        lifecycle.collect().unwrap();
        assert_eq!(lifecycle.state(), SandboxState::Collected);
        lifecycle.start().unwrap();
        assert_eq!(lifecycle.exec_calls(), 2);
        lifecycle.destroy().unwrap();
        assert!(lifecycle.is_destroyed());
        assert!(
            lifecycle.start().is_err(),
            "a destroyed handle is not reused"
        );
    }

    #[test]
    fn destroy_is_idempotent() {
        let mut lifecycle = SandboxLifecycle::create("fake", "01h00000000000000000000002");
        lifecycle.start().unwrap();
        lifecycle.destroy().unwrap();
        lifecycle.destroy().unwrap();
        assert!(lifecycle.is_destroyed());
    }

    #[test]
    fn an_absent_runtime_refuses_rather_than_running_in_process() {
        let sandbox = GvisorSandbox::new("/tmp/mm-tier2");
        let error = sandbox.run(&action()).unwrap_err();
        assert_eq!(error.code(), "sandbox.tier_unavailable");
        assert!(error.to_string().contains("gvisor"), "{error}");
        assert_eq!(sandbox.runtime_name(), "absent");
    }

    #[test]
    fn an_installed_runtime_runs_and_the_handle_is_destroyed() {
        #[derive(Debug)]
        struct Fake;
        impl SandboxRuntime for Fake {
            fn name(&self) -> &'static str {
                "fake"
            }
            fn probe(&self) -> Result<(), String> {
                Ok(())
            }
            fn exec(
                &self,
                action: &crate::action::ActionSpec,
            ) -> Result<serde_json::Value, String> {
                Ok(serde_json::json!({ "tool": action.tool.as_str(), "ran": true }))
            }
        }
        let sandbox = GvisorSandbox::new("/tmp/mm-tier2").with_runtime(Box::new(Fake));
        let output = sandbox.run(&action()).unwrap();
        assert_eq!(output["ran"], true);
        assert_eq!(output["tool"], "fs.read");
    }

    #[test]
    fn a_failing_runtime_still_destroys_the_handle() {
        #[derive(Debug)]
        struct Failing;
        impl SandboxRuntime for Failing {
            fn name(&self) -> &'static str {
                "failing"
            }
            fn probe(&self) -> Result<(), String> {
                Ok(())
            }
            fn exec(
                &self,
                _action: &crate::action::ActionSpec,
            ) -> Result<serde_json::Value, String> {
                Err("the sandbox died".to_string())
            }
        }
        let sandbox = GvisorSandbox::new("/tmp/mm-tier2").with_runtime(Box::new(Failing));
        assert!(sandbox.run(&action()).is_err());
    }

    #[test]
    fn the_derived_handle_is_stable_for_the_same_call() {
        assert_eq!(action_handle(&action()), action_handle(&action()));
        assert_eq!(action_handle(&action()).len(), mm_core::ULID_LEN);
        let mut other = action();
        other.args = serde_json::json!({ "path": "other" });
        assert_ne!(action_handle(&action()), action_handle(&other));
    }

    #[test]
    fn every_state_has_a_wire_name() {
        for state in SANDBOX_STATES {
            assert!(!state.as_str().is_empty());
        }
    }
}
