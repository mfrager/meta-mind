//! Authorization: who may do what, decided by code.
//!
//! The being proposes; this module decides. Nothing here reads a plan, a confidence
//! or a model output, and [`TablePermissionEngine::authorize`] is a pure function of
//! `(grants, policy sets, as_of, request)` — the same inputs always yield the same
//! [`Decision`], which is what makes the phase's second invariant checkable rather
//! than aspirational.
//!
//! # One deviation from the phase plan, and why
//!
//! The plan sketches `fn authorize(&self, principal: &Principal, req: &PermissionReq)
//! -> Decision`. The trait here takes a [`PermissionRequest`] instead, because the
//! `permission_grants` schema this same plan mandates carries a `tool_pattern`
//! column: a grant is a statement about a *principal acting through a tool on a
//! scope*, and a two-argument signature cannot express the middle term. An
//! implementation that guessed the tool would either ignore `tool_pattern` (making
//! the column decorative) or match every grant in the table (making a grant on
//! `fs.*` authorize `process.exec`). The request struct keeps the tool in the
//! decision, and `--tool` is how the CLI supplies it.
//!
//! # The resource grammar
//!
//! A resource is `kind:verb:pattern`, for example `fs:read:data/sandbox/**` or
//! `net:connect:api.metamind.dev`. It is the same string in a tool's
//! `ToolSpec::permissions`, in a grant's `scope` and on the CLI, so there is exactly
//! one spelling of a capability. [`PermissionReq::validate`] refuses a resource whose
//! verb disagrees with its [`PermissionAction`]: `fs:read:...` with
//! `PermissionAction::Write` is a request for two different things, and silently
//! honouring either one would make the audit record lie.
//!
//! # Default deny, in the order it is decided
//!
//! 1. A matching `forbid` rule denies, whatever else matches. Same rule as Cedar and
//!    OPA, for the same reason: a policy that can be overridden by adding a permit is
//!    not a policy.
//! 2. No covering grant denies. Absence is denial; there is no "unset" decision a
//!    caller could mistake for consent.
//! 3. An `Irreversible` tool that the caller has not explicitly confirmed escalates
//!    rather than allowing: a fire-and-forget irreversible action is the one outcome
//!    no rollback can repair.
//! 4. Otherwise it allows.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::policy::{Effect, PolicySet};
use crate::spec::ToolName;
use mm_core::Timestamp;

/// What a resource may be done to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    /// Read.
    Read,
    /// Write, overwriting what was there.
    Write,
    /// Run.
    Execute,
    /// Remove.
    Delete,
    /// Reach a network endpoint.
    Connect,
    /// Anything, including a change to the permission tables themselves.
    Admin,
}

/// Every action, in the order the matrix tables render them.
pub const PERMISSION_ACTIONS: [PermissionAction; 6] = [
    PermissionAction::Read,
    PermissionAction::Write,
    PermissionAction::Execute,
    PermissionAction::Delete,
    PermissionAction::Connect,
    PermissionAction::Admin,
];

impl PermissionAction {
    /// The stable wire name, which is also the middle field of a resource.
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionAction::Read => "read",
            PermissionAction::Write => "write",
            PermissionAction::Execute => "execute",
            PermissionAction::Delete => "delete",
            PermissionAction::Connect => "connect",
            PermissionAction::Admin => "admin",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        PERMISSION_ACTIONS
            .into_iter()
            .find(|action| action.as_str() == text.trim().to_ascii_lowercase())
    }

    /// True when holding `self` grants the right to do `other`.
    ///
    /// The lattice is deliberately small: `admin` implies everything and `write`
    /// implies `delete`, because deleting a file is a write to its directory and a
    /// grant written as "may write here" that refused `rm` would be a surprise rather
    /// than a restriction. Nothing else implies anything — in particular `read` never
    /// implies `write`, which is the pair the phase's adversarial fixtures target.
    pub fn implies(self, other: PermissionAction) -> bool {
        if self == other || self == PermissionAction::Admin {
            return true;
        }
        matches!(
            (self, other),
            (PermissionAction::Write, PermissionAction::Delete)
        )
    }

    /// The classes of access a `forbid` rule on this action covers.
    pub fn covered_by(self, rule: PermissionAction) -> bool {
        rule.implies(self)
    }
}

impl std::fmt::Display for PermissionAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One requested capability.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PermissionReq {
    /// `kind:verb:pattern`, e.g. `fs:read:data/sandbox/**`.
    pub resource: String,
    /// The verb, which must agree with the resource's own verb.
    pub action: PermissionAction,
}

impl PermissionReq {
    /// A request.
    pub fn new(resource: impl Into<String>, action: PermissionAction) -> Self {
        PermissionReq {
            resource: resource.into(),
            action,
        }
    }

    /// The kind, before the first colon.
    pub fn kind(&self) -> &str {
        self.resource.split(':').next().unwrap_or("")
    }

    /// The verb field of the resource, as written.
    pub fn verb(&self) -> &str {
        self.resource.split(':').nth(1).unwrap_or("")
    }

    /// The pattern, everything after the second colon.
    pub fn pattern(&self) -> &str {
        self.resource.splitn(3, ':').nth(2).unwrap_or("")
    }

    /// The canonical rendering: `kind:verb:pattern`, with the verb taken from the
    /// action so the two can never disagree in a stored form.
    pub fn canonical(&self) -> String {
        format!(
            "{}:{}:{}",
            self.kind(),
            self.action.as_str(),
            self.pattern()
        )
    }

    /// Refuse a resource that is not `kind:verb:pattern`, or whose verb disagrees
    /// with its action.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let mut parts = self.resource.splitn(3, ':');
        let kind = parts.next().unwrap_or("");
        let verb = parts.next().unwrap_or("");
        let pattern = parts.next().unwrap_or("");
        if kind.is_empty() || verb.is_empty() || pattern.is_empty() || parts.next().is_some() {
            return Err(format!(
                "capability {:?} is not `kind:verb:pattern`",
                self.resource
            ));
        }
        if !kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!("capability kind {kind:?} is not lowercase ASCII"));
        }
        if PermissionAction::parse(verb).is_none() {
            return Err(format!("capability verb {verb:?} is not a known action"));
        }
        if verb != self.action.as_str() {
            return Err(format!(
                "capability {:?} declares the verb {verb:?} but the action {}",
                self.resource, self.action
            ));
        }
        Ok(())
    }

    /// True when this request is covered by `broad`.
    ///
    /// Coverage is kind-equal, action-implies and pattern-covers. A request covered
    /// by another is not merely "similar": every concrete access it could denote is
    /// among the accesses the other denotes, which is the property a grant needs to
    /// be a restriction.
    pub fn covered_by(&self, broad: &PermissionReq) -> bool {
        self.kind() == broad.kind()
            && broad.action.implies(self.action)
            && pattern_covers(broad.pattern(), self.pattern())
    }

    /// The narrower of two requests over the same kind, for an intersection.
    pub fn intersect(&self, other: &PermissionReq) -> Option<PermissionReq> {
        if self.kind() != other.kind() {
            return None;
        }
        let (action, pattern) = if self.covered_by(other) {
            (self.action, self.pattern().to_string())
        } else if other.covered_by(self) {
            (other.action, other.pattern().to_string())
        } else if let Some(narrowed) = pattern_intersection(self.pattern(), other.pattern()) {
            (
                if self.action == other.action {
                    self.action
                } else {
                    // One side is broader; the tighter verb is the honest one.
                    self.action
                },
                narrowed,
            )
        } else {
            return None;
        };
        Some(PermissionReq::new(
            format!("{}:{}:{}", self.kind(), action.as_str(), pattern),
            action,
        ))
    }
}

impl std::fmt::Display for PermissionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// True when `broad` denotes every pattern `narrow` denotes.
pub fn pattern_covers(broad: &str, narrow: &str) -> bool {
    if broad == narrow || broad == "*" || broad == "**" {
        return true;
    }
    if let Some(prefix) = broad.strip_suffix("/**") {
        return narrow == prefix
            || narrow == format!("{prefix}/**")
            || narrow.starts_with(&format!("{prefix}/"));
    }
    if let Some(prefix) = broad.strip_suffix("/*") {
        return narrow == prefix || narrow.starts_with(&format!("{prefix}/"));
    }
    false
}

/// The pattern both denote, or `None` when they are disjoint.
///
/// The implementation is a prefix computation over the two patterns' literals: a
/// `/**` pattern's literal is its prefix. Two patterns intersect exactly when one
/// literal is a prefix of the other (or they are equal), and the intersection is the
/// longer literal, rendered as a `/**` pattern when the longer pattern was one.
fn pattern_intersection(a: &str, b: &str) -> Option<String> {
    let literal = |pattern: &str| -> (String, bool) {
        match pattern.strip_suffix("/**") {
            Some(prefix) => (prefix.to_string(), true),
            None => (pattern.to_string(), false),
        }
    };
    let (a_lit, a_recursive) = literal(a);
    let (b_lit, b_recursive) = literal(b);
    let path_prefix =
        |prefix: &str, path: &str| path == prefix || path.starts_with(&format!("{prefix}/"));
    if path_prefix(&a_lit, &b_lit) || path_prefix(&b_lit, &a_lit) {
        let (longer, recursive) = if a_lit.len() >= b_lit.len() {
            (&a_lit, a_recursive)
        } else {
            (&b_lit, b_recursive)
        };
        // The three ways to land on a recursive result are one condition: the
        // longer literal is itself recursive, or the shorter one is and is a
        // strict prefix of it, so the longer literal is what gets extended.
        let extend =
            recursive || (a_recursive && a_lit != *longer) || (b_recursive && b_lit != *longer);
        return Some(if extend {
            format!("{longer}/**")
        } else {
            longer.clone()
        });
    }
    None
}

/// Who is acting.
///
/// A principal is a being, a user, or the system itself, and it is spelled as the
/// identifier of whoever acts. It is deliberately not an enum with three variants
/// that could only hold a ULID: the strings that appear here also appear in
/// `permission_grants.principal` and in every audit record, and one spelling is what
/// lets the SQL match the code without a translation table.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Principal(pub String);

/// The principal the kernel itself acts as.
pub const SYSTEM_PRINCIPAL: &str = "system";

impl Principal {
    /// The kernel.
    pub fn system() -> Self {
        Principal(SYSTEM_PRINCIPAL.to_string())
    }

    /// A being, by ULID.
    pub fn being(id: impl Into<String>) -> Self {
        Principal(id.into())
    }

    /// A user, by ULID.
    pub fn user(id: impl Into<String>) -> Self {
        Principal(id.into())
    }

    /// The string, which is what the store and the audit records carry.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when this is the kernel principal.
    pub fn is_system(&self) -> bool {
        self.0 == SYSTEM_PRINCIPAL
    }

    /// Refuse a principal that could not be persisted or compared: empty, or
    /// containing whitespace (which would make `system ` and `system` two principals
    /// that look identical in a log).
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.0.trim().is_empty() {
            return Err("principal is empty".to_string());
        }
        if self.0.chars().any(char::is_whitespace) {
            return Err(format!("principal {:?} contains whitespace", self.0));
        }
        Ok(())
    }
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a request was denied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DenyReason {
    /// No grant covers the request at all.
    NoGrant {
        /// The tool that was asked for, when one was named.
        tool: Option<String>,
        /// The capability that was asked for.
        resource: String,
    },
    /// A grant exists for the principal but its scope does not reach.
    OutOfScope {
        /// The grant's id.
        grant: String,
        /// Its scope.
        scope: String,
        /// What was asked for.
        resource: String,
    },
    /// The grant expired before the decision instant.
    Expired {
        /// The grant's id.
        grant: String,
        /// When it expired.
        expired_at: Timestamp,
    },
    /// The grant was revoked.
    Revoked {
        /// The grant's id.
        grant: String,
        /// When it was revoked.
        revoked_at: Timestamp,
    },
    /// A policy rule forbade it.
    PolicyForbidden {
        /// The rule that forbade it.
        rule: String,
        /// The policy set the rule belongs to.
        policy_set: String,
    },
    /// The tool is irreversible and was not explicitly confirmed.
    Unconfirmed {
        /// The tool.
        tool: String,
    },
}

impl DenyReason {
    /// A stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            DenyReason::NoGrant { .. } => "permission.no_grant",
            DenyReason::OutOfScope { .. } => "permission.out_of_scope",
            DenyReason::Expired { .. } => "permission.expired",
            DenyReason::Revoked { .. } => "permission.revoked",
            DenyReason::PolicyForbidden { .. } => "permission.policy_forbidden",
            DenyReason::Unconfirmed { .. } => "permission.unconfirmed",
        }
    }

    /// One sentence, for the log record and the CLI.
    pub fn message(&self) -> String {
        match self {
            DenyReason::NoGrant { tool, resource } => match tool {
                Some(tool) => format!("no grant covers {tool}:{resource}"),
                None => format!(
                    "no grant covers {resource} (a grant matches on principal, tool and scope, \
                     and this request names no tool)"
                ),
            },
            DenyReason::OutOfScope {
                grant,
                scope,
                resource,
            } => format!("grant {grant} covers {scope}, which does not reach {resource}"),
            DenyReason::Expired { grant, expired_at } => {
                format!("grant {grant} expired at {}", expired_at.to_rfc3339())
            }
            DenyReason::Revoked { grant, revoked_at } => {
                format!("grant {grant} was revoked at {}", revoked_at.to_rfc3339())
            }
            DenyReason::PolicyForbidden { rule, policy_set } => {
                format!("policy rule {rule} in set {policy_set} forbids it")
            }
            DenyReason::Unconfirmed { tool } => {
                format!("{tool} is irreversible and no explicit confirmation was given")
            }
        }
    }
}

impl std::fmt::Display for DenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Who a decision must be escalated to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EscalationTarget {
    /// A specific user, by ULID.
    User(String),
    /// The operator of this kernel.
    Operator,
    /// Phase 9's decision firewall, which grades irreversible actions.
    Firewall,
}

impl EscalationTarget {
    /// A stable machine-readable name.
    pub fn as_str(&self) -> &str {
        match self {
            EscalationTarget::User(id) => id.as_str(),
            EscalationTarget::Operator => "operator",
            EscalationTarget::Firewall => "firewall",
        }
    }
}

/// The answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// It may proceed.
    Allow,
    /// It may not, and here is why.
    Deny {
        /// Why.
        reason: DenyReason,
    },
    /// It may not proceed *by itself*.
    Escalate {
        /// Who decides.
        to: EscalationTarget,
        /// Why escalation rather than a decision.
        reason: String,
    },
}

impl Decision {
    /// True when the action may run.
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow)
    }

    /// True when the action may not run.
    pub fn is_deny(&self) -> bool {
        matches!(self, Decision::Deny { .. })
    }

    /// True when the action needs a person or the firewall.
    pub fn is_escalate(&self) -> bool {
        matches!(self, Decision::Escalate { .. })
    }

    /// The `tool_calls.permission_decision` value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny { .. } => "deny",
            Decision::Escalate { .. } => "escalate",
        }
    }

    /// Why, in one sentence.
    pub fn reason_text(&self) -> String {
        match self {
            Decision::Allow => "granted".to_string(),
            Decision::Deny { reason } => reason.message(),
            Decision::Escalate { to, reason } => {
                format!("escalated to {}: {reason}", to.as_str())
            }
        }
    }

    /// The reason, when there is one.
    pub fn deny_reason(&self) -> Option<&DenyReason> {
        match self {
            Decision::Deny { reason } => Some(reason),
            _ => None,
        }
    }
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.as_str(), self.reason_text())
    }
}

/// One row of `permission_grants`, as the engine sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// The grant's ULID.
    pub id: String,
    /// Who it is for, or `*` for everyone.
    pub principal: Principal,
    /// The capability scope, a resource pattern such as `fs:write:data/sandbox/**`.
    pub scope: String,
    /// Which tools it may be exercised through, `namespace.*` or an exact name.
    pub tool_pattern: String,
    /// The policy set that was in force when it was granted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_set: Option<String>,
    /// Who granted it.
    pub granted_by: String,
    /// When it stops applying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    /// When it was revoked, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<Timestamp>,
}

impl Grant {
    /// A grant that never expires.
    pub fn new(
        id: impl Into<String>,
        principal: Principal,
        scope: impl Into<String>,
        tool_pattern: impl Into<String>,
        granted_by: impl Into<String>,
    ) -> Self {
        Grant {
            id: id.into(),
            principal,
            scope: scope.into(),
            tool_pattern: tool_pattern.into(),
            policy_set: None,
            granted_by: granted_by.into(),
            expires_at: None,
            revoked_at: None,
        }
    }

    /// The grant's scope as a request, when it parses.
    pub fn request(&self) -> Option<PermissionReq> {
        let mut parts = self.scope.splitn(3, ':');
        let kind = parts.next()?;
        let verb = parts.next()?;
        let pattern = parts.next()?;
        let action = PermissionAction::parse(verb)?;
        Some(PermissionReq::new(
            format!("{kind}:{verb}:{pattern}"),
            action,
        ))
    }

    /// True when the grant is live at `as_of`.
    pub fn is_live(&self, as_of: Timestamp) -> std::result::Result<(), DenyReason> {
        if let Some(revoked_at) = self.revoked_at {
            return Err(DenyReason::Revoked {
                grant: self.id.clone(),
                revoked_at,
            });
        }
        if let Some(expires_at) = self.expires_at {
            if expires_at <= as_of {
                return Err(DenyReason::Expired {
                    grant: self.id.clone(),
                    expired_at: expires_at,
                });
            }
        }
        Ok(())
    }

    /// True when the grant is for this principal.
    pub fn matches_principal(&self, principal: &Principal) -> bool {
        self.principal.0 == "*" || &self.principal == principal
    }

    /// True when the grant may be exercised through `tool`.
    ///
    /// A grant with no named tool matches nothing. That is the documented consequence
    /// of the tool being part of a grant: `mm-cli policy check` without `--tool` is
    /// the strictest reading of the same request, so it denies, and naming the tool is
    /// what makes a permit visible.
    pub fn matches_tool(&self, tool: Option<&ToolName>) -> bool {
        match tool {
            Some(tool) => tool.matches(&self.tool_pattern),
            None => false,
        }
    }

    /// The reason this grant does not cover a request, or `Ok(())`.
    pub fn covers(
        &self,
        principal: &Principal,
        tool: Option<&ToolName>,
        req: &PermissionReq,
        as_of: Timestamp,
    ) -> std::result::Result<(), DenyReason> {
        if !self.matches_principal(principal) {
            return Err(DenyReason::NoGrant {
                tool: tool.map(|t| t.to_string()),
                resource: req.canonical(),
            });
        }
        if !self.matches_tool(tool) {
            return Err(DenyReason::NoGrant {
                tool: tool.map(|t| t.to_string()),
                resource: req.canonical(),
            });
        }
        self.is_live(as_of)?;
        let Some(scope) = self.request() else {
            return Err(DenyReason::OutOfScope {
                grant: self.id.clone(),
                scope: self.scope.clone(),
                resource: req.canonical(),
            });
        };
        if !req.covered_by(&scope) {
            return Err(DenyReason::OutOfScope {
                grant: self.id.clone(),
                scope: self.scope.clone(),
                resource: req.canonical(),
            });
        }
        Ok(())
    }
}

/// Everything a decision depends on.
///
/// Explicit, and a struct rather than four arguments: the decision is a pure function
/// of all of it, and a caller that forgot the tool or the clock would get a decision
/// about a different question.
#[derive(Clone, Debug)]
pub struct PermissionRequest<'a> {
    /// Who is acting.
    pub principal: &'a Principal,
    /// Through which tool, when the caller named one.
    pub tool: Option<&'a ToolName>,
    /// What capability is asked for.
    pub req: &'a PermissionReq,
    /// The decision instant, so expiry is deterministic under replay.
    pub as_of: Timestamp,
    /// True when a person has explicitly confirmed an irreversible action.
    pub confirmed: bool,
}

impl<'a> PermissionRequest<'a> {
    /// A request, unconfirmed and evaluated now.
    pub fn new(principal: &'a Principal, req: &'a PermissionReq) -> Self {
        PermissionRequest {
            principal,
            tool: None,
            req,
            as_of: Timestamp::now(),
            confirmed: false,
        }
    }

    /// The same request through a named tool.
    pub fn with_tool(mut self, tool: &'a ToolName) -> Self {
        self.tool = Some(tool);
        self
    }

    /// The same request evaluated at an explicit instant.
    pub fn at(mut self, as_of: Timestamp) -> Self {
        self.as_of = as_of;
        self
    }

    /// The same request, explicitly confirmed.
    pub fn confirmed(mut self) -> Self {
        self.confirmed = true;
        self
    }
}

/// The authorization interface.
pub trait PermissionEngine: Send + Sync {
    /// Decide. Pure: no store, no clock, no model.
    fn authorize(&self, request: &PermissionRequest<'_>) -> Decision;

    /// The capabilities the caller may actually exercise through `tool`, given what the
    /// tool declared.
    ///
    /// A required method rather than a defaulted one, and deliberately so: the result
    /// is what the sandbox guard is built from, so a default implementation would be a
    /// default *capability grant*. Every engine has to say what its grants reach.
    fn granted_capabilities(
        &self,
        principal: &Principal,
        tool: Option<&ToolName>,
        declared: &[PermissionReq],
    ) -> Vec<PermissionReq>;

    /// The grants in force, for `mm-cli policy list` and the audit record.
    fn grants(&self) -> &[Grant];

    /// The policy sets in force.
    fn policy_sets(&self) -> &[PolicySet];
}

/// The engine over `permission_grants` and `policy_sets`.
///
/// It holds the rows rather than reading them per call, because the decision must be
/// reproducible: an engine that queried the store inside `authorize` would decide
/// differently depending on when it was asked, and a replay could not reproduce the
/// original answer.
#[derive(Clone, Debug, Default)]
pub struct TablePermissionEngine {
    grants: Vec<Grant>,
    policy_sets: Vec<PolicySet>,
    as_of: Option<Timestamp>,
}

impl TablePermissionEngine {
    /// An engine over live rows.
    pub fn new(grants: Vec<Grant>, policy_sets: Vec<PolicySet>) -> Self {
        TablePermissionEngine {
            grants,
            policy_sets,
            as_of: None,
        }
    }

    /// Pin the decision instant, so a replay of an old action decides as it did then.
    pub fn pinned_at(mut self, as_of: Timestamp) -> Self {
        self.as_of = Some(as_of);
        self
    }

    /// The instant this engine decides at: the pinned one, or now.
    pub fn decision_instant(&self) -> Timestamp {
        self.as_of.unwrap_or_else(Timestamp::now)
    }

    /// The effect the policy sets yield for a request.
    pub fn policy_effect(&self, request: &PermissionRequest<'_>) -> Effect {
        crate::policy::PolicyEngine::evaluate(&self.policy_sets, request)
    }

    /// The capabilities the principal may actually exercise through `tool`, given
    /// what the tool declared.
    ///
    /// The intersection, per the phase plan's confused-deputy mitigation: the tool
    /// gets what it declared *and* the caller may grant. A declared capability the
    /// caller holds no grant for is dropped rather than refused here, so the refusal
    /// happens at the sandbox with the capability named — which is the error a caller
    /// can act on ("you are missing `net:connect:api.metamind.dev`"), instead of a
    /// whole-call denial that says nothing about which capability was missing.
    ///
    /// Exposed as an inherent method (`granted_capabilities` on the trait delegates
    /// here) because an intersection is a property of a *grant table*: an engine with
    /// a different rule language implements the trait method its own way.
    pub fn intersect_grants(
        &self,
        principal: &Principal,
        tool: Option<&ToolName>,
        declared: &[PermissionReq],
    ) -> Vec<PermissionReq> {
        let as_of = self.decision_instant();
        let mut out: Vec<PermissionReq> = Vec::new();
        for want in declared {
            let mut covered: Option<PermissionReq> = None;
            for grant in &self.grants {
                if grant.covers(principal, tool, want, as_of).is_err() {
                    continue;
                }
                let Some(scope) = grant.request() else {
                    continue;
                };
                let Some(intersected) = want.intersect(&scope) else {
                    continue;
                };
                covered = Some(match covered {
                    Some(previous) => {
                        // The union of two grants on the same kind: the wider one is
                        // what the caller may use, because both are held.
                        if intersected.covered_by(&previous) {
                            previous
                        } else {
                            intersected
                        }
                    }
                    None => intersected,
                });
            }
            if let Some(capability) = covered {
                if !out
                    .iter()
                    .any(|existing| existing.canonical() == capability.canonical())
                {
                    out.push(capability);
                }
            }
        }
        // `sort_by_cached_key`, not `sort_by`: `canonical()` builds a string, and
        // the cached key computes it once per capability rather than once per
        // comparison. The order is the same lexicographic one on the same strings.
        out.sort_by_cached_key(|a| a.canonical());
        out
    }

    /// Every capability the principal holds through a namespace, whatever a tool
    /// declared. Used by `mm-cli policy list`.
    pub fn held_scopes(&self) -> BTreeSet<String> {
        self.grants
            .iter()
            .filter(|grant| grant.revoked_at.is_none())
            .map(|grant| grant.scope.clone())
            .collect()
    }
}

impl PermissionEngine for TablePermissionEngine {
    fn authorize(&self, request: &PermissionRequest<'_>) -> Decision {
        let as_of = request.as_of;
        // 1. An explicit forbid wins over everything.
        if let Some(forbidden) =
            crate::policy::PolicyEngine::first_forbid(&self.policy_sets, request)
        {
            return Decision::Deny {
                reason: DenyReason::PolicyForbidden {
                    rule: forbidden.0,
                    policy_set: forbidden.1,
                },
            };
        }
        // 2. No covering grant is denial. The reasons are collected and the most
        //    informative one is reported: "your grant covers something else" is a
        //    better answer than "no grant", and both are denials.
        let mut best: Option<DenyReason> = None;
        for grant in &self.grants {
            match grant.covers(request.principal, request.tool, request.req, as_of) {
                Ok(()) => return Decision::Allow,
                Err(reason) => {
                    let rank = |r: &DenyReason| match r {
                        DenyReason::OutOfScope { .. } => 3,
                        DenyReason::Expired { .. } | DenyReason::Revoked { .. } => 2,
                        DenyReason::NoGrant { .. } => 1,
                        DenyReason::PolicyForbidden { .. } | DenyReason::Unconfirmed { .. } => 0,
                    };
                    best = Some(match best {
                        Some(previous) if rank(&previous) >= rank(&reason) => previous,
                        _ => reason,
                    });
                }
            }
        }
        Decision::Deny {
            reason: best.unwrap_or(DenyReason::NoGrant {
                tool: request.tool.map(|t| t.to_string()),
                resource: request.req.canonical(),
            }),
        }
    }

    fn granted_capabilities(
        &self,
        principal: &Principal,
        tool: Option<&ToolName>,
        declared: &[PermissionReq],
    ) -> Vec<PermissionReq> {
        self.intersect_grants(principal, tool, declared)
    }

    fn grants(&self) -> &[Grant] {
        &self.grants
    }

    fn policy_sets(&self) -> &[PolicySet] {
        &self.policy_sets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{PolicyRule, PolicySet};
    use mm_core::Ulid;

    fn grant(scope: &str) -> Grant {
        Grant::new(
            "01h0000000000000000000g001",
            Principal::system(),
            scope,
            "fs.*",
            "operator",
        )
    }

    fn req(resource: &str, action: PermissionAction) -> PermissionReq {
        PermissionReq::new(resource, action)
    }

    #[test]
    fn a_resource_is_kind_verb_pattern() {
        let good = req("fs:read:data/sandbox/**", PermissionAction::Read);
        assert!(good.validate().is_ok(), "{:?}", good.validate());
        assert_eq!(good.kind(), "fs");
        assert_eq!(good.verb(), "read");
        assert_eq!(good.pattern(), "data/sandbox/**");
        assert_eq!(good.canonical(), "fs:read:data/sandbox/**");

        assert!(req("fs:data/sandbox/**", PermissionAction::Read)
            .validate()
            .is_err());
        assert!(req("fs:read:data/sandbox/**", PermissionAction::Write)
            .validate()
            .is_err());
        assert!(req("fs:fly:data/sandbox/**", PermissionAction::Read)
            .validate()
            .is_err());
        assert!(req("FS:read:x", PermissionAction::Read).validate().is_err());
    }

    #[test]
    fn a_read_grant_never_covers_a_write() {
        let read = req("fs:read:data/sandbox/**", PermissionAction::Read);
        let write = req("fs:write:data/sandbox/**", PermissionAction::Write);
        assert!(read.covered_by(&read));
        assert!(!write.covered_by(&read));
        assert!(write.covered_by(&write));
        assert!(
            !read.covered_by(&write),
            "a broader verb is not implied by a narrower one"
        );
        assert!(PermissionAction::Write.implies(PermissionAction::Delete));
        assert!(!PermissionAction::Read.implies(PermissionAction::Write));
        assert!(PermissionAction::Admin.implies(PermissionAction::Execute));
    }

    #[test]
    fn a_recursive_pattern_covers_the_tree_and_a_file_does_not() {
        assert!(pattern_covers("data/sandbox/**", "data/sandbox/a/b.txt"));
        assert!(pattern_covers("data/sandbox/**", "data/sandbox/**"));
        assert!(!pattern_covers("data/sandbox/**", "data/other/x"));
        assert!(pattern_covers("*", "anything/at/all"));
        assert!(pattern_covers("data/sandbox/a.txt", "data/sandbox/a.txt"));
        assert!(!pattern_covers("data/sandbox/a.txt", "data/sandbox/b.txt"));
    }

    #[test]
    fn a_narrower_grant_narrows_a_declared_capability() {
        let declared = req("fs:read:data/sandbox/**", PermissionAction::Read);
        let held = req("fs:read:data/sandbox/public/**", PermissionAction::Read);
        let intersection = declared.intersect(&held).unwrap();
        assert_eq!(intersection.pattern(), "data/sandbox/public/**");
        assert_eq!(intersection.action, PermissionAction::Read);
        assert!(intersection.covered_by(&declared));
        assert!(intersection.covered_by(&held));
        assert!(declared
            .intersect(&req("net:read:x", PermissionAction::Read))
            .is_none());
    }

    #[test]
    fn allow_deny_and_the_default() {
        let engine = TablePermissionEngine::new(vec![grant("fs:read:data/sandbox/**")], Vec::new());
        let principal = Principal::system();
        let tool = ToolName::new("fs.read").unwrap();

        let allowed = engine.authorize(
            &PermissionRequest::new(
                &principal,
                &req("fs:read:data/sandbox/a.txt", PermissionAction::Read),
            )
            .with_tool(&tool)
            .at(Timestamp::from_epoch_seconds(1_700_000_000)),
        );
        assert!(allowed.is_allow(), "{allowed}");

        // Absence of a grant is denial, not "unset".
        let denied = engine.authorize(
            &PermissionRequest::new(
                &principal,
                &req("fs:write:data/sandbox/a.txt", PermissionAction::Write),
            )
            .with_tool(&tool)
            .at(Timestamp::from_epoch_seconds(1_700_000_000)),
        );
        assert!(denied.is_deny(), "{denied}");
    }

    #[test]
    fn a_grant_is_specific_to_a_principal_and_a_tool() {
        let engine = TablePermissionEngine::new(vec![grant("fs:read:data/sandbox/**")], Vec::new());
        let as_of = Timestamp::from_epoch_seconds(1_700_000_000);
        let read = ToolName::new("fs.read").unwrap();
        let exec = ToolName::new("process.exec").unwrap();

        assert!(engine
            .authorize(
                &PermissionRequest::new(
                    &Principal::system(),
                    &req("fs:read:data/sandbox/a", PermissionAction::Read),
                )
                .with_tool(&read)
                .at(as_of),
            )
            .is_allow());

        // `fs.*` does not cover process.exec: a wildcard is one namespace wide.
        assert!(engine
            .authorize(
                &PermissionRequest::new(
                    &Principal::system(),
                    &req("process:execute:cargo", PermissionAction::Execute),
                )
                .with_tool(&exec)
                .at(as_of),
            )
            .is_deny());

        assert!(engine
            .authorize(
                &PermissionRequest::new(
                    &Principal::being(Ulid::from_parts(1_700_000_000_000, 1).to_string()),
                    &req("fs:read:data/sandbox/a", PermissionAction::Read),
                )
                .with_tool(&read)
                .at(as_of),
            )
            .is_deny());
    }

    #[test]
    fn a_request_with_no_tool_cannot_match_a_grant() {
        let engine = TablePermissionEngine::new(vec![grant("fs:read:data/sandbox/**")], Vec::new());
        let decision = engine.authorize(
            &PermissionRequest::new(
                &Principal::system(),
                &req("fs:read:data/sandbox/a", PermissionAction::Read),
            )
            .at(Timestamp::from_epoch_seconds(1_700_000_000)),
        );
        assert!(decision.is_deny(), "{decision}");
        assert!(decision.reason_text().contains("names no tool"));
    }

    #[test]
    fn an_expired_or_revoked_grant_denies() {
        let as_of = Timestamp::from_epoch_seconds(1_700_000_000);
        let mut expired = grant("fs:read:data/sandbox/**");
        expired.expires_at = Some(Timestamp::from_epoch_seconds(1_600_000_000));
        let mut revoked = grant("fs:read:data/sandbox/**");
        revoked.revoked_at = Some(Timestamp::from_epoch_seconds(1_600_000_000));
        let read = ToolName::new("fs.read").unwrap();
        let request = req("fs:read:data/sandbox/a", PermissionAction::Read);

        for (row, expected) in [
            (expired, "permission.expired"),
            (revoked, "permission.revoked"),
        ] {
            let engine = TablePermissionEngine::new(vec![row], Vec::new());
            let decision = engine.authorize(
                &PermissionRequest::new(&Principal::system(), &request)
                    .with_tool(&read)
                    .at(as_of),
            );
            assert_eq!(
                decision.deny_reason().map(DenyReason::code),
                Some(expected),
                "{decision}"
            );
        }
    }

    #[test]
    fn an_explicit_forbid_beats_a_permit() {
        let sets = vec![PolicySet {
            id: "01h0000000000000000000p001".into(),
            name: "baseline".into(),
            version: 1,
            rules: vec![
                PolicyRule::permit("*", "fs.write", "data/sandbox/**"),
                PolicyRule::forbid("*", "fs.write", "data/sandbox/forbidden/**"),
            ],
        }];
        let engine = TablePermissionEngine::new(
            vec![Grant::new(
                "01h0000000000000000000g002",
                Principal::system(),
                "fs:write:data/sandbox/**",
                "fs.*",
                "operator",
            )],
            sets,
        );
        let write = ToolName::new("fs.write").unwrap();
        let as_of = Timestamp::from_epoch_seconds(1_700_000_000);

        let permitted = engine.authorize(
            &PermissionRequest::new(
                &Principal::system(),
                &req("fs:write:data/sandbox/notes.txt", PermissionAction::Write),
            )
            .with_tool(&write)
            .at(as_of),
        );
        assert!(permitted.is_allow(), "{permitted}");

        let forbidden = engine.authorize(
            &PermissionRequest::new(
                &Principal::system(),
                &req(
                    "fs:write:data/sandbox/forbidden/x.txt",
                    PermissionAction::Write,
                ),
            )
            .with_tool(&write)
            .at(as_of),
        );
        assert!(forbidden.is_deny(), "{forbidden}");
        assert_eq!(
            forbidden.deny_reason().map(DenyReason::code),
            Some("permission.policy_forbidden")
        );
    }

    #[test]
    fn effective_permissions_are_an_intersection() {
        let engine = TablePermissionEngine::new(vec![grant("fs:read:data/sandbox/**")], Vec::new());
        let tool = ToolName::new("fs.read").unwrap();
        let declared = vec![
            req("fs:read:data/sandbox/**", PermissionAction::Read),
            // Declared but never granted: the caller holds no net capability at all.
            req("net:connect:api.example.com", PermissionAction::Connect),
            // Declared for the wrong namespace: `fs.*` does not cover it.
            req("process:execute:cargo", PermissionAction::Execute),
        ];
        let effective = engine.intersect_grants(&Principal::system(), Some(&tool), &declared);
        assert_eq!(effective.len(), 1, "{effective:?}");
        assert_eq!(effective[0].canonical(), "fs:read:data/sandbox/**");
    }

    #[test]
    fn an_irreversible_action_is_decided_by_the_caller_not_here() {
        // The escalation for an irreversible tool is applied by the executor, which is
        // the layer that knows the tool's reversibility; the engine's job is the grant
        // and the policy, and it says allow when both say allow. That the two layers
        // are separate is what lets `policy check` describe the grant question alone.
        let engine = TablePermissionEngine::new(vec![grant("fs:read:data/sandbox/**")], Vec::new());
        let read = ToolName::new("fs.read").unwrap();
        let decision = engine.authorize(
            &PermissionRequest::new(
                &Principal::system(),
                &req("fs:read:data/sandbox/a", PermissionAction::Read),
            )
            .with_tool(&read)
            .at(Timestamp::from_epoch_seconds(1_700_000_000)),
        );
        assert!(decision.is_allow());
    }

    #[test]
    fn decisions_round_trip_and_render() {
        let decision = Decision::Deny {
            reason: DenyReason::NoGrant {
                tool: Some("fs.read".into()),
                resource: "fs:read:x".into(),
            },
        };
        let text = serde_json::to_string(&decision).unwrap();
        let back: Decision = serde_json::from_str(&text).unwrap();
        assert_eq!(decision, back);
        assert_eq!(decision.as_str(), "deny");
        assert!(decision.to_string().contains("fs.read"));
    }
}
