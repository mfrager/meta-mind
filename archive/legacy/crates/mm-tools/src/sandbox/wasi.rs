//! Tier 1: the capability-limited in-process sandbox.
//!
//! # Why this is not a Wasm engine
//!
//! The phase plan names Wasmtime/WASI as the reference for tier 1 and describes its
//! mechanism as "run in-process under an explicit capability set; undeclared access is
//! a hard error". This crate implements exactly that mechanism — a
//! [`CapabilityGuard`] derived from the tool's own declaration, which every
//! filesystem and network access in [`crate::tools`] goes through — and does **not**
//! link a Wasm runtime. Three reasons, all of them checkable:
//!
//! * `wasmtime` is not in this workspace's dependency graph or its vendored
//!   registry cache, and Phase 10's rule (parent plan §7) is that a copied-in
//!   dependency is recorded in `vendor/<origin>/COPYING.md` with its revision. Adding
//!   a WebAssembly engine is a dependency decision, not an implementation detail of
//!   this module.
//! * Every tool in this phase is a Rust function in this workspace. Running it as a
//!   Wasm module would mean compiling it to a second artifact and losing the type
//!   boundary the rest of the crate relies on, for the same capability guarantee
//!   that the guard already provides.
//! * The guard is *strictly* the property the plan asks to test: undeclared path and
//!   undeclared host both fail with a typed error before any work starts, and the
//!   adversarial fixtures for escape (path traversal, undeclared network, oversize
//!   payload) exercise it directly.
//!
//! What tier 1 does not provide, stated plainly: it does not stop a *tool's own code*
//! from calling `std::fs` directly, because in-process Rust cannot be trapped without
//! a hypervisor or a Wasm engine. The mitigation is the same one the plan gives for
//! tier 2: a tool that must not be trusted with ambient authority runs in tier 3,
//! behind a process boundary. [`crate::spec::ToolSpec::sandbox_tier`] is how a tool
//! declares that, and [`microvm`](super::microvm) is where it goes.

use std::path::PathBuf;

use crate::spec::ToolSpec;

use super::{CapabilityGuard, Sandbox, SandboxCapabilities, SandboxTier};

/// The default payload ceiling for a tier-1 tool, in bytes.
///
/// One mebibyte: large enough for a source file, a graph fixture or an HTTP body, and
/// small enough that an accidental `read` of a device or a log does not become an
/// out-of-memory. A tool that needs more declares it through its spec, and the
/// sandbox takes the smaller of the two.
pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024;

/// Tier 1's sandbox.
#[derive(Clone, Debug)]
pub struct WasiSandbox {
    root: PathBuf,
    max_bytes: u64,
}

impl WasiSandbox {
    /// A sandbox rooted at `root` with the default payload ceiling.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        WasiSandbox {
            root: root.into(),
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

    /// A sandbox with an explicit payload ceiling.
    ///
    /// A smaller ceiling than the default is a tightening and is honoured; a larger
    /// one is not, because `max_bytes` is a sandbox property rather than a
    /// per-tool preference.
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes.min(DEFAULT_MAX_BYTES);
        self
    }
}

impl Sandbox for WasiSandbox {
    fn tier(&self) -> SandboxTier {
        SandboxTier::WasmCaps
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
    use crate::permissions::{PermissionAction, PermissionReq};

    fn spec(permissions: Vec<PermissionReq>) -> ToolSpec {
        ToolSpec {
            name: crate::spec::ToolName::new("fs.read").unwrap(),
            version: "0.1.0".into(),
            description: "d".into(),
            input_schema: crate::spec::JsonSchema::any_object(),
            output_schema: crate::spec::JsonSchema::any_object(),
            permissions,
            reversibility: crate::spec::Reversibility::Reversible,
            side_effects: crate::spec::SideEffectClass::Local,
            annotations: crate::spec::ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: true,
                open_world: false,
            },
            module_uri: "https://metamind.dev/code/module/tools/fs-read".into(),
            sandbox_tier: SandboxTier::WasmCaps,
        }
    }

    #[test]
    fn a_declared_path_inside_the_root_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.txt"), "hello").unwrap();
        let sandbox = WasiSandbox::new(&root);
        let guard = sandbox.guard(&spec(vec![PermissionReq::new(
            "fs:read:data/sandbox/**",
            PermissionAction::Read,
        )]));
        let resolved = guard
            .read_path(root.join("note.txt").to_str().unwrap())
            .unwrap();
        assert_eq!(resolved, root.join("note.txt").canonicalize().unwrap());
    }

    #[test]
    fn a_path_outside_the_root_is_refused_even_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();
        let sandbox = WasiSandbox::new(&root);
        let guard = sandbox.guard(&spec(vec![PermissionReq::new(
            "fs:read:data/sandbox/**",
            PermissionAction::Read,
        )]));
        let error = guard.read_path(outside.to_str().unwrap()).unwrap_err();
        assert!(
            matches!(
                error,
                crate::error::SandboxError::Escape { .. }
                    | crate::error::SandboxError::PathDenied { .. }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_write_through_a_read_grant_is_read_only_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let sandbox = WasiSandbox::new(&root);
        let guard = sandbox.guard(&spec(vec![PermissionReq::new(
            "fs:read:data/sandbox/**",
            PermissionAction::Read,
        )]));
        let error = guard
            .write_path(root.join("new.txt").to_str().unwrap())
            .unwrap_err();
        assert_eq!(error.code(), "sandbox.read_only");
    }

    #[test]
    fn the_payload_ceiling_is_a_tightening_only() {
        let sandbox = WasiSandbox::new("/tmp/x").with_max_bytes(u64::MAX);
        assert_eq!(sandbox.max_bytes(), DEFAULT_MAX_BYTES);
        let smaller = WasiSandbox::new("/tmp/x").with_max_bytes(16);
        assert_eq!(smaller.max_bytes(), 16);
    }
}
