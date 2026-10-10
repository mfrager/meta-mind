//! Isolation: what a tool is allowed to reach, and the refusal when it reaches
//! further.
//!
//! Three tiers, selected per tool by its [`crate::spec::ToolSpec::sandbox_tier`]:
//!
//! | Tier | Backend | Used for |
//! |---|---|---|
//! | 1 [`SandboxTier::WasmCaps`] | capability-limited in-process | pure tools and local file work: explicit fs/net grants, no ambient authority |
//! | 2 [`SandboxTier::Gvisor`] | syscall interception | untrusted native tools where a microVM is too heavy |
//! | 3 [`SandboxTier::MicroVm`] | microVM with an ephemeral filesystem | arbitrary code: generated builds, untrusted binaries |
//!
//! # Where "run" happens, and why the trait has no `run`
//!
//! The phase plan sketches `Sandbox::run(caps, action)`. This crate splits that in
//! two, deliberately, because the split is what makes the guard *unskippable*: the
//! capability check is [`CapabilityGuard`], a value the tool is handed and cannot
//! obtain access without, and running the tool is the executor's call into
//! [`crate::registry::Tool::invoke`]. A single `run` that both decided and executed
//! would put the decision inside the tier implementation, where a tool with a
//! different tier could not reuse it and where the tier-3 case (which genuinely must
//! hand the work to another process) would have to reimplement it.
//!
//! So: **tier 1 is the guard**, enforced at the seam every filesystem and network
//! access in [`crate::tools`] goes through. Tiers 2 and 3 own a lifecycle
//! ([`gvisor`], [`microvm`]) and report [`SandboxError::TierUnavailable`] when no
//! runtime is installed, which is the honest answer on a machine without gVisor or
//! Firecracker — and is why their features are off by default.
//!
//! # The capability model, stated once
//!
//! A [`SandboxCapabilities`] is *derived from the tool's own declaration*
//! (`ToolSpec::permissions`), never supplied by the caller running the tool. That is
//! what makes undeclared access a hard error rather than a request: a tool that did
//! not declare `fs:write:<root>` has no write capability to check against, so its
//! first write is refused with [`SandboxError::PathDenied`] and there is no code path
//! that could have granted it.

pub mod gvisor;
pub mod microvm;
pub mod wasi;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::SandboxError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::spec::ToolSpec;

pub use gvisor::GvisorSandbox;
pub use microvm::MicroVmSandbox;
pub use wasi::{WasiSandbox, DEFAULT_MAX_BYTES};

/// Which isolation backend a tool runs under.
///
/// The serde spelling is the plan's SQL spelling (`WasmCaps`), because the value is
/// stored in `tool_registry.sandbox_tier` with a `CHECK` on exactly these three
/// names; a second spelling would need a second constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SandboxTier {
    /// Tier 1: capability-limited in-process.
    WasmCaps,
    /// Tier 2: syscall interception.
    Gvisor,
    /// Tier 3: microVM.
    MicroVm,
}

/// Every tier, least to most isolated.
pub const SANDBOX_TIERS: [SandboxTier; 3] = [
    SandboxTier::WasmCaps,
    SandboxTier::Gvisor,
    SandboxTier::MicroVm,
];

impl SandboxTier {
    /// The stable wire name, which is also the `tool_registry.sandbox_tier` value.
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxTier::WasmCaps => "WasmCaps",
            SandboxTier::Gvisor => "Gvisor",
            SandboxTier::MicroVm => "MicroVm",
        }
    }

    /// Parse a wire name, accepting either case.
    pub fn parse(text: &str) -> Option<Self> {
        let lower = text.trim().to_ascii_lowercase();
        SANDBOX_TIERS
            .into_iter()
            .find(|tier| tier.as_str().to_ascii_lowercase() == lower)
    }

    /// How much isolation the tier provides. Higher is more isolated.
    pub fn strength(self) -> u8 {
        self as u8
    }

    /// The tier an action of the given trust needs, so an escalation can be checked
    /// rather than asserted.
    pub fn at_least(self, required: SandboxTier) -> bool {
        self.strength() >= required.strength()
    }
}

impl std::fmt::Display for SandboxTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a tool may do to the filesystem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum FilesystemAccess {
    /// No filesystem at all.
    None,
    /// Read anywhere inside the path.
    ReadOnly(PathBuf),
    /// Read and write inside the path.
    ReadWrite(PathBuf),
}

impl FilesystemAccess {
    /// The root, when there is one.
    pub fn root(&self) -> Option<&Path> {
        match self {
            FilesystemAccess::None => None,
            FilesystemAccess::ReadOnly(path) | FilesystemAccess::ReadWrite(path) => Some(path),
        }
    }

    /// True when the grant permits reading.
    pub fn can_read(&self) -> bool {
        !matches!(self, FilesystemAccess::None)
    }

    /// True when the grant permits writing.
    pub fn can_write(&self) -> bool {
        matches!(self, FilesystemAccess::ReadWrite(_))
    }

    /// The stable rendering the capability hash uses.
    pub fn canonical(&self) -> String {
        match self {
            FilesystemAccess::None => "fs:none".to_string(),
            FilesystemAccess::ReadOnly(path) => format!("fs:ro:{}", path.display()),
            FilesystemAccess::ReadWrite(path) => format!("fs:rw:{}", path.display()),
        }
    }
}

/// What a tool may reach over the network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NetworkAccess {
    /// Nothing.
    Denied,
    /// Only these hosts, by exact name.
    Allowlist(Vec<String>),
}

impl NetworkAccess {
    /// True when the host is reachable under this grant.
    ///
    /// The comparison is exact and case-insensitive: a suffix rule would let
    /// `evil-example.com` through an allowlist entry of `example.com`, which is the
    /// classic allowlist bypass.
    pub fn permits(&self, host: &str) -> bool {
        match self {
            NetworkAccess::Denied => false,
            NetworkAccess::Allowlist(hosts) => hosts
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(host.trim())),
        }
    }

    /// The stable rendering the capability hash uses.
    pub fn canonical(&self) -> String {
        match self {
            NetworkAccess::Denied => "net:none".to_string(),
            NetworkAccess::Allowlist(hosts) => {
                let mut hosts = hosts.clone();
                hosts.sort();
                format!("net:allow:{}", hosts.join(","))
            }
        }
    }
}

/// The capability set a tool runs under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    /// What it may do to the filesystem.
    pub filesystem: FilesystemAccess,
    /// What it may reach over the network.
    pub network: NetworkAccess,
    /// Whether it may spawn a process.
    pub subprocess: bool,
    /// Which commands it may spawn, as a pattern such as `cargo` or `cargo test`.
    ///
    /// A `bool` is not enough for this capability: "may run something" would make any
    /// granted process capability a way to run anything, which is the confused-deputy
    /// shape the phase's risk table names. The pattern is carried from the grant that
    /// authorized the call, so a caller granted `cargo build` cannot run `rm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    /// The largest payload it may read or write.
    pub max_bytes: u64,
}

impl Default for SandboxCapabilities {
    /// No ambient authority: every capability denied, including the process one.
    fn default() -> Self {
        SandboxCapabilities {
            filesystem: FilesystemAccess::None,
            network: NetworkAccess::Denied,
            subprocess: false,
            process: None,
            max_bytes: 0,
        }
    }
}

impl SandboxCapabilities {
    /// A capability set with the given filesystem root and nothing else.
    pub fn read_root(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        SandboxCapabilities {
            filesystem: FilesystemAccess::ReadOnly(root.into()),
            network: NetworkAccess::Denied,
            subprocess: false,
            process: None,
            max_bytes,
        }
    }

    /// A capability set with read/write on the given root.
    pub fn write_root(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        let root = root.into();
        SandboxCapabilities {
            filesystem: FilesystemAccess::ReadWrite(root.clone()),
            ..SandboxCapabilities::read_root(root, max_bytes)
        }
    }

    /// Derive the capabilities a spec's own declarations justify.
    ///
    /// The mapping is the whole of the authorization-to-capability translation, and
    /// it is in one place so that "what did this tool ask for" has exactly one
    /// answer. A tool that declares no capability gets [`SandboxCapabilities::default`],
    /// which denies everything.
    pub fn from_permissions(permissions: &[PermissionReq], root: &Path, max_bytes: u64) -> Self {
        let mut capabilities = SandboxCapabilities {
            filesystem: FilesystemAccess::None,
            network: NetworkAccess::Denied,
            subprocess: false,
            process: None,
            max_bytes,
        };
        let mut hosts: Vec<String> = Vec::new();
        for permission in permissions {
            match permission.kind() {
                "fs" => {
                    let wants_write = matches!(
                        permission.action,
                        PermissionAction::Write
                            | PermissionAction::Execute
                            | PermissionAction::Delete
                    );
                    capabilities.filesystem = if wants_write {
                        FilesystemAccess::ReadWrite(root.to_path_buf())
                    } else if !capabilities.filesystem.can_write() {
                        FilesystemAccess::ReadOnly(root.to_path_buf())
                    } else {
                        capabilities.filesystem.clone()
                    };
                }
                "net" => {
                    for host in permission.pattern().split(',') {
                        let host = host.trim();
                        if !host.is_empty() && !hosts.iter().any(|h| h == host) {
                            hosts.push(host.to_string());
                        }
                    }
                    if hosts.is_empty() {
                        capabilities.network = NetworkAccess::Denied;
                    } else {
                        hosts.sort();
                        capabilities.network = NetworkAccess::Allowlist(hosts.clone());
                    }
                }
                "process" => {
                    capabilities.subprocess = permission.action == PermissionAction::Execute;
                    if capabilities.subprocess {
                        capabilities.process = Some(permission.pattern().to_string());
                    }
                }
                _ => {}
            }
        }
        capabilities
    }

    /// The `tool.sandbox.start` record's `caps_hash`.
    ///
    /// A digest of the *grant*, so two runs with the same grant are comparable and a
    /// run with a widened grant is visibly a different run.
    pub fn hash(&self) -> String {
        mm_core::hash_fields(&[
            &self.filesystem.canonical(),
            &self.network.canonical(),
            if self.subprocess {
                "subprocess"
            } else {
                "no-subprocess"
            },
            self.process.as_deref().unwrap_or("no-command"),
            &self.max_bytes.to_string(),
        ])
    }

    /// The command pattern, when process execution is granted.
    pub fn process_pattern(&self) -> Option<&str> {
        self.process.as_deref()
    }
}

/// The thing every capability check goes through.
///
/// Tools in [`crate::tools`] hold one of these and cannot reach a path or a host any
/// other way; the guard is the sandbox's enforcement point for tier 1 and the
/// pre-flight check for tiers 2 and 3.
#[derive(Clone, Debug)]
pub struct CapabilityGuard {
    capabilities: SandboxCapabilities,
    tier: SandboxTier,
    sandbox_root: PathBuf,
}

impl CapabilityGuard {
    /// A guard over a capability set, restricted to `sandbox_root`.
    pub fn new(
        capabilities: SandboxCapabilities,
        tier: SandboxTier,
        sandbox_root: impl Into<PathBuf>,
    ) -> Self {
        CapabilityGuard {
            capabilities,
            tier,
            sandbox_root: sandbox_root.into(),
        }
    }

    /// The capabilities it enforces.
    pub fn capabilities(&self) -> &SandboxCapabilities {
        &self.capabilities
    }

    /// The tier it belongs to.
    pub fn tier(&self) -> SandboxTier {
        self.tier
    }

    /// Resolve a path for reading, or refuse.
    pub fn read_path(&self, requested: &str) -> Result<PathBuf, SandboxError> {
        self.resolve(requested, "fs.read", false)
    }

    /// Resolve a path for writing, or refuse.
    pub fn write_path(&self, requested: &str) -> Result<PathBuf, SandboxError> {
        self.resolve(requested, "fs.write", true)
    }

    /// Check a size against `max_bytes`, or refuse.
    pub fn check_size(&self, requested: u64) -> Result<(), SandboxError> {
        if requested > self.capabilities.max_bytes {
            return Err(SandboxError::TooLarge {
                requested,
                allowed: self.capabilities.max_bytes,
            });
        }
        Ok(())
    }

    /// Check a host against the network grant, or refuse.
    pub fn check_host(&self, host: &str) -> Result<(), SandboxError> {
        if self.capabilities.network.permits(host) {
            Ok(())
        } else {
            Err(SandboxError::HostDenied {
                host: host.to_string(),
            })
        }
    }

    /// Check that process execution is granted, or refuse.
    pub fn check_subprocess(&self) -> Result<(), SandboxError> {
        if self.capabilities.subprocess {
            Ok(())
        } else {
            Err(SandboxError::SubprocessDenied)
        }
    }

    /// The command pattern this guard permits, when it permits any.
    pub fn process_pattern(&self) -> Option<&str> {
        self.capabilities.process_pattern()
    }

    /// Check a whole command against the granted pattern, or refuse.
    ///
    /// Checked *in addition to* [`CapabilityGuard::check_subprocess`], because the two
    /// answer different questions: whether this caller may spawn a process at all, and
    /// whether this particular command is the one it was granted.
    pub fn check_command(&self, command: &str) -> Result<(), SandboxError> {
        match self.capabilities.process_pattern() {
            Some(pattern) if command_pattern_covers(pattern, command) => Ok(()),
            _ => Err(SandboxError::SubprocessDenied),
        }
    }

    /// The path resolution every filesystem access goes through.
    ///
    /// The order matters, and it is: read-only first (a write through a read grant is
    /// refused before any path question is asked, so the error names the actual
    /// problem), then sandbox root, then the granted root. Both checks canonicalize,
    /// which is what makes `data/sandbox/../../etc/shadow` and a symlink out of the
    /// tree fail the same way: the comparison is between resolved paths, not between
    /// the strings that were typed.
    fn resolve(
        &self,
        requested: &str,
        capability: &str,
        write: bool,
    ) -> Result<PathBuf, SandboxError> {
        if !self.capabilities.filesystem.can_read() {
            return Err(SandboxError::PathDenied {
                capability: capability.to_string(),
                requested: requested.to_string(),
                allowed: "none".to_string(),
            });
        }
        if write && !self.capabilities.filesystem.can_write() {
            return Err(SandboxError::ReadOnly {
                capability: capability.to_string(),
            });
        }
        let granted = self
            .capabilities
            .filesystem
            .root()
            .ok_or_else(|| SandboxError::PathDenied {
                capability: capability.to_string(),
                requested: requested.to_string(),
                allowed: "none".to_string(),
            })?
            .to_path_buf();
        let candidate = Path::new(requested);
        // A path may be relative to the repository root (which is the process's
        // working directory) or absolute; `resolve` makes one of each without
        // touching the filesystem, then the *existing* ancestor is canonicalized so a
        // path that does not exist yet is still checked against its real parent.
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| SandboxError::Resolution {
                    requested: requested.to_string(),
                    reason: format!("cannot read the working directory: {e}"),
                })?
                .join(candidate)
        };
        let resolved = canonicalize_with_missing_tail(&absolute).map_err(|reason| {
            SandboxError::Resolution {
                requested: requested.to_string(),
                reason,
            }
        })?;
        let root = canonicalize_existing(&self.sandbox_root).unwrap_or(self.sandbox_root.clone());
        if !resolved.starts_with(&root) {
            return Err(SandboxError::Escape {
                requested: requested.to_string(),
            });
        }
        let granted = canonicalize_existing(&granted).unwrap_or(granted);
        if !resolved.starts_with(&granted) {
            return Err(SandboxError::PathDenied {
                capability: capability.to_string(),
                requested: requested.to_string(),
                allowed: granted.display().to_string(),
            });
        }
        Ok(resolved)
    }
}

/// Canonicalize a path, and the longest existing ancestor of one that does not exist
/// yet, appending the missing tail unchanged.
fn canonicalize_with_missing_tail(path: &Path) -> std::result::Result<PathBuf, String> {
    if let Ok(resolved) = path.canonicalize() {
        return Ok(resolved);
    }
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match cursor.parent() {
            Some(parent) => {
                let name = cursor
                    .file_name()
                    .ok_or_else(|| format!("{} has no file name", path.display()))?;
                tail.push(name.to_os_string());
                if let Ok(resolved) = parent.canonicalize() {
                    let mut out = resolved;
                    for part in tail.iter().rev() {
                        out.push(part);
                    }
                    return Ok(out);
                }
                cursor = parent.to_path_buf();
            }
            None => {
                return Err(format!(
                    "no existing ancestor of {} could be resolved",
                    path.display()
                ))
            }
        }
    }
}

/// Canonicalize a path that may not exist, falling back to the path itself.
fn canonicalize_existing(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

/// A requested path, re-spelled the way capability patterns spell it.
///
/// A declaration names a subtree as the sandbox root is written —
/// `fs:write:data/sandbox/**` — and a policy rule that forbids part of that subtree is
/// written in the same words (`data/sandbox/forbidden/**`). A run's argument, though, may
/// be any path at all, usually absolute or relative to the working directory: this is the
/// translation between the two, and "the same words" is [`crate::SANDBOX_ROOT`] as the
/// prefix, which is the one name every declaration in [`crate::tools`] already uses.
///
/// The path is resolved exactly as [`CapabilityGuard::resolve`] resolves it — relative
/// against the working directory, with the longest existing ancestor canonicalized — so a
/// spelling exists for precisely the paths that resolution would let through. `None` when
/// it does not resolve or lies outside `root`: refusing those is the guard's job, and a
/// spelling invented here would be a claim in a decision record the run cannot back.
pub fn spell_path(root: &Path, requested: &str) -> Option<PathBuf> {
    let candidate = Path::new(requested);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(candidate)
    };
    let resolved = canonicalize_with_missing_tail(&absolute).ok()?;
    let canonical_root = canonicalize_existing(root).unwrap_or_else(|| root.to_path_buf());
    let tail = resolved.strip_prefix(&canonical_root).ok()?;
    Some(Path::new(crate::SANDBOX_ROOT).join(tail))
}

/// A sandbox: a tier, the capabilities it derives for a spec, and the guard.
pub trait Sandbox: Send + Sync {
    /// Which tier this is.
    fn tier(&self) -> SandboxTier;

    /// The capabilities the spec's own declarations justify under this sandbox.
    fn capabilities(&self, spec: &ToolSpec) -> SandboxCapabilities;

    /// A guard over those capabilities.
    fn guard(&self, spec: &ToolSpec) -> CapabilityGuard {
        CapabilityGuard::new(self.capabilities(spec), self.tier(), self.root())
    }

    /// The sandbox root every filesystem capability is a restriction of.
    fn root(&self) -> PathBuf;

    /// The largest payload this sandbox will move, when the tier caps it below what
    /// a tool declared.
    fn max_bytes(&self) -> u64;
}

/// True when a granted command pattern covers a command.
///
/// A pattern is a program name, optionally with leading subcommand words (`cargo`,
/// `cargo test`), and it covers a command when the command *starts with it as a whole
/// word*. A substring rule would let a grant on `cargo` cover `cargo-evil`, and a
/// prefix rule without the word boundary would let it cover `cargo-evil` too — both are
/// the escape the capability exists to stop.
pub fn command_pattern_covers(pattern: &str, command: &str) -> bool {
    let pattern = pattern.trim();
    let command = command.trim();
    if pattern == "*" || pattern == "**" || pattern == command {
        return true;
    }
    command.starts_with(&format!("{pattern} "))
}

/// Whether a tier can run here at all.
///
/// Tier 1 is the capability guard and is always available. Tiers 2 and 3 need a
/// runtime this build does not link by default, and this is where that refusal is
/// produced — before any work, and with the reason, rather than as a runtime failure
/// somewhere inside a tool.
pub fn tier_available(tier: SandboxTier) -> Result<(), SandboxError> {
    match tier {
        SandboxTier::WasmCaps => Ok(()),
        SandboxTier::Gvisor => Err(SandboxError::TierUnavailable {
            tier: SandboxTier::Gvisor.as_str().to_string(),
            reason: gvisor::TRUSTED_RUNTIME_REASON.to_string(),
        }),
        SandboxTier::MicroVm => Err(SandboxError::TierUnavailable {
            tier: SandboxTier::MicroVm.as_str().to_string(),
            reason: microvm::MICROVM_REASON.to_string(),
        }),
    }
}

/// The sandbox for a tier.
///
/// A tier whose runtime is absent is still returned: constructing it and asking it
/// to work is what produces [`SandboxError::TierUnavailable`], and a factory that
/// failed instead would turn "this machine has no microVM" into a load-time surprise
/// rather than a per-call refusal.
pub fn sandbox_for(tier: SandboxTier, root: impl Into<PathBuf>) -> Box<dyn Sandbox> {
    let root = root.into();
    match tier {
        SandboxTier::WasmCaps => Box::new(WasiSandbox::new(root)),
        SandboxTier::Gvisor => Box::new(GvisorSandbox::new(root)),
        SandboxTier::MicroVm => Box::new(MicroVmSandbox::new(root)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::PermissionReq;

    fn spec_with(permissions: Vec<PermissionReq>) -> ToolSpec {
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
    fn capabilities_default_to_no_ambient_authority() {
        let caps = SandboxCapabilities::default();
        assert_eq!(caps.filesystem, FilesystemAccess::None);
        assert_eq!(caps.network, NetworkAccess::Denied);
        assert!(!caps.subprocess);
        assert_eq!(caps.max_bytes, 0);
    }

    #[test]
    fn a_read_permission_grants_read_only_and_a_write_permission_grants_write() {
        let root = Path::new("/tmp/sandbox");
        let read = SandboxCapabilities::from_permissions(
            &[PermissionReq::new(
                "fs:read:data/sandbox/**",
                PermissionAction::Read,
            )],
            root,
            1024,
        );
        assert!(read.filesystem.can_read());
        assert!(!read.filesystem.can_write());

        let write = SandboxCapabilities::from_permissions(
            &[PermissionReq::new(
                "fs:write:data/sandbox/**",
                PermissionAction::Write,
            )],
            root,
            1024,
        );
        assert!(write.filesystem.can_write());
        assert_ne!(read.hash(), write.hash());
    }

    #[test]
    fn a_network_allowlist_is_exact() {
        let caps = NetworkAccess::Allowlist(vec!["example.com".into()]);
        assert!(caps.permits("example.com"));
        assert!(caps.permits("EXAMPLE.COM"));
        assert!(!caps.permits("evil-example.com"));
        assert!(!caps.permits("example.com.evil.net"));
        assert!(!NetworkAccess::Denied.permits("example.com"));
    }

    #[test]
    fn subprocess_is_granted_only_by_an_execute_permission() {
        let root = std::env::temp_dir().join("mm-sandbox-caps");
        let none = SandboxCapabilities::from_permissions(&[], &root, 16);
        assert!(!none.subprocess);
        let exec = SandboxCapabilities::from_permissions(
            &[PermissionReq::new(
                "process:execute:cargo test",
                PermissionAction::Execute,
            )],
            &root,
            16,
        );
        assert!(exec.subprocess);
    }

    #[test]
    fn a_tool_that_declares_nothing_is_denied_a_read() {
        let root = std::env::temp_dir().join("mm-sandbox-empty");
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&[], &root, 16),
            SandboxTier::WasmCaps,
            root.clone(),
        );
        let error = guard.read_path("anything").unwrap_err();
        assert_eq!(error.code(), "sandbox.path_denied");
        assert!(guard.check_subprocess().is_err());
        assert!(guard.check_host("example.com").is_err());
        assert_eq!(
            spec_with(vec![]).permissions.len(),
            0,
            "the fixture declares nothing, which is the case under test"
        );
    }

    #[test]
    fn tier_strength_is_ordered() {
        assert!(SandboxTier::MicroVm.at_least(SandboxTier::Gvisor));
        assert!(SandboxTier::Gvisor.at_least(SandboxTier::WasmCaps));
        assert!(!SandboxTier::WasmCaps.at_least(SandboxTier::Gvisor));
        for tier in SANDBOX_TIERS {
            assert_eq!(SandboxTier::parse(tier.as_str()), Some(tier));
            assert_eq!(
                SandboxTier::parse(&tier.as_str().to_lowercase()),
                Some(tier)
            );
        }
        assert_eq!(SandboxTier::parse("nonesuch"), None);
    }

    #[test]
    fn the_factory_returns_a_sandbox_for_every_tier() {
        let root = std::env::temp_dir().join("mm-sandbox-factory");
        for tier in SANDBOX_TIERS {
            let sandbox = sandbox_for(tier, root.clone());
            assert_eq!(sandbox.tier(), tier);
        }
    }
}
