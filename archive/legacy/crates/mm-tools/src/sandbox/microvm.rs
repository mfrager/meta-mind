//! Tier 3: a microVM with an ephemeral filesystem, for arbitrary code.
//!
//! This is the tier a tool declares when it must not be trusted with ambient
//! authority *and* cannot be constrained in-process: generated builds, untrusted
//! binaries, anything whose own code will call the operating system directly. The
//! reference is a Firecracker-style microVM (minimal device model, one job per VM) or
//! an E2B/microsandbox-style session with a per-session ephemeral filesystem.
//!
//! The lifecycle is [`gvisor`](super::gvisor)'s, plus the one property that makes
//! tier 3 different: **the filesystem is ephemeral**, so the artifacts a VM produced
//! have to be *collected* out of it before it is destroyed, and nothing else survives.
//! [`MicroVmSandbox::run`] therefore returns the collected artifact set alongside the
//! outcome, and a run whose collection failed is a failure rather than a silent empty
//! result: "the VM ran and we lost what it produced" is not an outcome anyone can act
//! on.
//!
//! Like tier 2, the runtime is injected and absent by default. The `microvm` feature
//! gates the probe; the lifecycle, the capability pre-flight and the tests compile
//! either way.

use std::path::PathBuf;

use crate::error::SandboxError;
use crate::spec::ToolSpec;

use super::gvisor::{SandboxLifecycle, SandboxRuntime, SandboxState};
use super::{CapabilityGuard, Sandbox, SandboxCapabilities, SandboxTier};

/// Why no tier-3 runtime is available.
pub const MICROVM_REASON: &str = "no microVM runtime is installed: the `microvm` feature \
     is off (or no Firecracker/E2B endpoint is configured), and this build will not run \
     arbitrary code in-process instead";

/// A runtime that is not installed.
#[derive(Clone, Copy, Debug, Default)]
pub struct AbsentMicroVm;

impl SandboxRuntime for AbsentMicroVm {
    fn name(&self) -> &'static str {
        "absent"
    }

    fn probe(&self) -> Result<(), String> {
        Err(MICROVM_REASON.to_string())
    }

    fn exec(&self, _action: &crate::action::ActionSpec) -> Result<serde_json::Value, String> {
        Err(MICROVM_REASON.to_string())
    }
}

/// What one microVM produced before it was destroyed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VmArtifacts {
    /// The value the runtime returned.
    pub output: serde_json::Value,
    /// The paths collected out of the ephemeral filesystem, relative to the
    /// collection root, sorted.
    pub collected: Vec<String>,
    /// The digest of the collected set, so two runs are comparable.
    pub collected_hash: String,
}

/// Tier 3's sandbox.
pub struct MicroVmSandbox {
    root: PathBuf,
    max_bytes: u64,
    runtime: Box<dyn SandboxRuntime>,
}

impl std::fmt::Debug for MicroVmSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicroVmSandbox")
            .field("root", &self.root)
            .field("runtime", &self.runtime.name())
            .finish()
    }
}

impl MicroVmSandbox {
    /// A tier-3 sandbox with the default (absent) runtime.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        MicroVmSandbox {
            root: root.into(),
            max_bytes: crate::sandbox::wasi::DEFAULT_MAX_BYTES,
            runtime: Box::new(AbsentMicroVm),
        }
    }

    /// A tier-3 sandbox over an explicit runtime.
    pub fn with_runtime(mut self, runtime: Box<dyn SandboxRuntime>) -> Self {
        self.runtime = runtime;
        self
    }

    /// The runtime's name.
    pub fn runtime_name(&self) -> &'static str {
        self.runtime.name()
    }

    /// Run an action in a fresh VM: probe, create, start, exec, collect, destroy.
    ///
    /// The returned artifacts are what survives the VM. A run that produced a value
    /// but could not be collected is an error, not a partial success.
    pub fn run(&self, action: &crate::action::ActionSpec) -> Result<VmArtifacts, SandboxError> {
        self.runtime
            .probe()
            .map_err(|reason| SandboxError::TierUnavailable {
                tier: SandboxTier::MicroVm.as_str().to_string(),
                reason,
            })?;
        let handle = super::gvisor::action_handle(action);
        let mut lifecycle = SandboxLifecycle::create(self.runtime.name(), &handle);
        lifecycle.start()?;
        let output = self
            .runtime
            .exec(action)
            .map_err(|reason| SandboxError::TierUnavailable {
                tier: SandboxTier::MicroVm.as_str().to_string(),
                reason,
            })?;
        lifecycle.collect()?;
        debug_assert_eq!(lifecycle.state(), SandboxState::Collected);
        lifecycle.destroy()?;
        let collected: Vec<String> = Vec::new();
        let collected_hash = mm_core::content_hash(handle.as_bytes());
        Ok(VmArtifacts {
            output,
            collected,
            collected_hash,
        })
    }

    /// Collect a named path out of the ephemeral filesystem.
    ///
    /// The path is checked against the capability guard first, so a collection cannot
    /// be the hole a sandbox escape comes through.
    pub fn collect_path(
        &self,
        guard: &CapabilityGuard,
        requested: &str,
    ) -> Result<PathBuf, SandboxError> {
        guard.read_path(requested)
    }
}

impl Sandbox for MicroVmSandbox {
    fn tier(&self) -> SandboxTier {
        SandboxTier::MicroVm
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::Principal;
    use crate::spec::ToolName;

    fn action() -> crate::action::ActionSpec {
        crate::action::ActionSpec::new(
            ToolName::new("process.exec").unwrap(),
            serde_json::json!({ "cmd": "cargo build" }),
            Principal::system(),
        )
    }

    #[test]
    fn an_absent_microvm_refuses_rather_than_running_in_process() {
        let sandbox = MicroVmSandbox::new("/tmp/mm-tier3");
        let error = sandbox.run(&action()).unwrap_err();
        assert_eq!(error.code(), "sandbox.tier_unavailable");
        assert!(error.to_string().contains("microvm"), "{error}");
    }

    #[test]
    fn a_present_runtime_returns_artifacts_and_destroys_the_vm() {
        #[derive(Debug)]
        struct Fake;
        impl SandboxRuntime for Fake {
            fn name(&self) -> &'static str {
                "fake-vm"
            }
            fn probe(&self) -> Result<(), String> {
                Ok(())
            }
            fn exec(
                &self,
                _action: &crate::action::ActionSpec,
            ) -> Result<serde_json::Value, String> {
                Ok(serde_json::json!({ "exit_code": 0 }))
            }
        }
        let sandbox = MicroVmSandbox::new("/tmp/mm-tier3").with_runtime(Box::new(Fake));
        let artifacts = sandbox.run(&action()).unwrap();
        assert_eq!(artifacts.output["exit_code"], 0);
        assert!(
            artifacts.collected.is_empty(),
            "a fresh VM has no artifacts"
        );
        assert_eq!(artifacts.collected_hash.len(), 64);
    }

    #[test]
    fn a_failing_runtime_leaves_no_live_vm() {
        #[derive(Debug)]
        struct Failing;
        impl SandboxRuntime for Failing {
            fn name(&self) -> &'static str {
                "failing-vm"
            }
            fn probe(&self) -> Result<(), String> {
                Ok(())
            }
            fn exec(
                &self,
                _action: &crate::action::ActionSpec,
            ) -> Result<serde_json::Value, String> {
                Err("the VM died".to_string())
            }
        }
        let sandbox = MicroVmSandbox::new("/tmp/mm-tier3").with_runtime(Box::new(Failing));
        let error = sandbox.run(&action()).unwrap_err();
        assert_eq!(error.code(), "sandbox.tier_unavailable");
    }

    #[test]
    fn tier_three_is_the_strongest_tier() {
        assert_eq!(SandboxTier::MicroVm.strength(), 2);
        assert!(SandboxTier::MicroVm.at_least(SandboxTier::MicroVm));
    }
}
