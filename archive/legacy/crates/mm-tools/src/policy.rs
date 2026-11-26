//! Policy as data: `forbid` wins, absence of a `permit` denies.
//!
//! The shape is Cedar's, deliberately: a rule names an effect, a principal, an action
//! and a resource, and evaluation is a pure function over the rule set. Stated in one
//! sentence, the order is **forbid > permit > default-deny**, and the middle term is
//! where most authorization bugs live: a set with neither a matching permit nor a
//! matching forbid denies, so a policy an operator forgot to update fails closed.
//!
//! # Why the rules are data and not code
//!
//! A policy rule is reviewed, versioned and audited, and it has to be reproducible
//! across a replay: [`crate::permissions::TablePermissionEngine`] holds the rules it
//! decided with, and `policy_sets` stores `rules_json` so the exact rule set that
//! permitted a call is recoverable. A rule written as a Rust closure would be none of
//! those — it could not be diffed, and the same call could decide differently after a
//! rebuild.
//!
//! # The two patterns in a rule
//!
//! `action` is `kind.verb` (`fs.write`, `process.execute`, `net.connect`) and matches
//! the request's own `kind:verb` pair; `*` and `kind.*` are whole-field wildcards.
//! `resource` is the *pattern* half of a capability (`data/sandbox/**`), and matching
//! is the same [`crate::permissions::pattern_covers`] the grant table uses, so a rule
//! and a grant can never disagree about what "inside `data/sandbox`" means.

use serde::{Deserialize, Serialize};

use crate::permissions::{pattern_covers, PermissionRequest};

/// What a matching rule does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// The request is refused, whatever else matches.
    Forbid,
    /// The request may proceed, if nothing forbids it.
    Permit,
}

/// Every effect, strongest first.
pub const EFFECTS: [Effect; 2] = [Effect::Forbid, Effect::Permit];

impl Effect {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Effect::Forbid => "forbid",
            Effect::Permit => "permit",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        EFFECTS
            .into_iter()
            .find(|effect| effect.as_str() == text.trim().to_ascii_lowercase())
    }

    /// True when the effect refuses the request.
    pub fn is_forbid(self) -> bool {
        matches!(self, Effect::Forbid)
    }
}

impl std::fmt::Display for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    /// The effect.
    pub effect: Effect,
    /// A principal, or `*` for everyone.
    pub principal: String,
    /// `kind.verb`, `kind.*` or `*`.
    pub action: String,
    /// The capability pattern, e.g. `data/sandbox/**`.
    pub resource: String,
}

impl PolicyRule {
    /// A rule.
    pub fn new(
        effect: Effect,
        principal: impl Into<String>,
        action: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        PolicyRule {
            effect,
            principal: principal.into(),
            action: action.into(),
            resource: resource.into(),
        }
    }

    /// A `permit` rule.
    pub fn permit(
        principal: impl Into<String>,
        action: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        PolicyRule::new(Effect::Permit, principal, action, resource)
    }

    /// A `forbid` rule.
    pub fn forbid(
        principal: impl Into<String>,
        action: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        PolicyRule::new(Effect::Forbid, principal, action, resource)
    }

    /// The rule's stable id: a hash of its own content.
    ///
    /// Derived rather than assigned, so the same rule in two sets has the same id and
    /// a rename cannot silently detach a rule from the audit records that cite it.
    pub fn id(&self) -> String {
        mm_core::hash_fields(&[
            self.effect.as_str(),
            &self.principal,
            &self.action,
            &self.resource,
        ])
        .chars()
        .take(16)
        .collect()
    }

    /// True when this rule matches a request.
    pub fn matches(&self, request: &PermissionRequest<'_>) -> bool {
        if self.principal != "*" && self.principal != request.principal.as_str() {
            return false;
        }
        let action = format!("{}.{}", request.req.kind(), request.req.action.as_str());
        if !action_pattern_matches(&self.action, &action) {
            return false;
        }
        pattern_covers(&self.resource, request.req.pattern())
    }

    /// Refuse a rule that could never match, or that would match everything.
    ///
    /// A rule with an empty field is almost always a bug in the data that an operator
    /// would rather hear about than debug: `principal: ""` matches nobody, and a
    /// silently inert `forbid` is the worst of these — it is the rule an operator
    /// wrote to *stop* something.
    pub fn validate(&self) -> std::result::Result<(), String> {
        for (name, value) in [
            ("principal", &self.principal),
            ("action", &self.action),
            ("resource", &self.resource),
        ] {
            if value.trim().is_empty() {
                return Err(format!("rule declares an empty {name}"));
            }
        }
        if !self.action.contains('.') && self.action != "*" {
            return Err(format!(
                "rule action {:?} is not `kind.verb`, `kind.*` or `*`",
                self.action
            ));
        }
        Ok(())
    }
}

/// True when a rule action pattern covers a concrete `kind.verb`.
fn action_pattern_matches(pattern: &str, action: &str) -> bool {
    if pattern == "*" || pattern == action {
        return true;
    }
    match pattern.split_once('.') {
        Some((namespace, "*")) => action
            .split_once('.')
            .map(|(requested, _)| requested == namespace)
            .unwrap_or(false),
        _ => false,
    }
}

/// A versioned set of rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicySet {
    /// The set's ULID.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its version. A change needs a new version, not a new set.
    pub version: u32,
    /// The rules.
    pub rules: Vec<PolicyRule>,
}

impl PolicySet {
    /// A set.
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: u32) -> Self {
        PolicySet {
            id: id.into(),
            name: name.into(),
            version,
            rules: Vec::new(),
        }
    }

    /// The same set with a rule added.
    pub fn with_rule(mut self, rule: PolicyRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Parse the `rules_json` a `policy_sets` row carries.
    pub fn parse_rules(json: &str) -> std::result::Result<Vec<PolicyRule>, String> {
        let rules: Vec<PolicyRule> = serde_json::from_str(json)
            .map_err(|e| format!("rules_json is not a rule list: {e}"))?;
        for rule in &rules {
            rule.validate()?;
        }
        Ok(rules)
    }

    /// Refuse a set with a rule that cannot match.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("policy set has no name".to_string());
        }
        if self.version == 0 {
            return Err("policy set version must be at least 1".to_string());
        }
        for rule in &self.rules {
            rule.validate()?;
        }
        Ok(())
    }
}

/// The evaluator. A unit struct, because it holds no state: everything a decision
/// depends on arrives as an argument.
#[derive(Clone, Copy, Debug, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    /// The effect of a set of policy sets on a request.
    ///
    /// `Forbid` when any rule forbids it — including the case where nothing matches,
    /// because absence of a permit is denial. The two are distinguishable through
    /// [`PolicyEngine::first_forbid`], which is what the audit record needs in order to
    /// name a rule when there is one.
    pub fn evaluate(sets: &[PolicySet], request: &PermissionRequest<'_>) -> Effect {
        if Self::first_forbid(sets, request).is_some() {
            return Effect::Forbid;
        }
        if Self::first_permit(sets, request).is_some() {
            return Effect::Permit;
        }
        Effect::Forbid
    }

    /// The first matching `forbid` rule, as `(rule_id, set_name)`.
    ///
    /// Deterministic ordering: sets in the order given, rules in the order declared.
    /// A caller that reorders the input gets a different *rule id* but the same
    /// effect, which is the property that matters — the decision, not the citation.
    pub fn first_forbid(
        sets: &[PolicySet],
        request: &PermissionRequest<'_>,
    ) -> Option<(String, String)> {
        Self::first_with(sets, request, Effect::Forbid)
    }

    /// The first matching `permit` rule, as `(rule_id, set_name)`.
    pub fn first_permit(
        sets: &[PolicySet],
        request: &PermissionRequest<'_>,
    ) -> Option<(String, String)> {
        Self::first_with(sets, request, Effect::Permit)
    }

    fn first_with(
        sets: &[PolicySet],
        request: &PermissionRequest<'_>,
        effect: Effect,
    ) -> Option<(String, String)> {
        sets.iter().find_map(|set| {
            set.rules
                .iter()
                .find(|rule| rule.effect == effect && rule.matches(request))
                .map(|rule| (rule.id(), set.name.clone()))
        })
    }

    /// The names of every rule that matches, for `mm-cli policy check --explain`.
    pub fn matching_rules(
        sets: &[PolicySet],
        request: &PermissionRequest<'_>,
    ) -> Vec<(String, String, Effect)> {
        sets.iter()
            .flat_map(|set| {
                set.rules
                    .iter()
                    .filter(|rule| rule.matches(request))
                    .map(|rule| (rule.id(), set.name.clone(), rule.effect))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{PermissionAction, PermissionReq, Principal};
    use crate::spec::ToolName;
    use mm_core::Timestamp;

    fn request<'a>(
        principal: &'a Principal,
        tool: &'a ToolName,
        req: &'a PermissionReq,
    ) -> PermissionRequest<'a> {
        PermissionRequest::new(principal, req)
            .with_tool(tool)
            .at(Timestamp::from_epoch_seconds(1_700_000_000))
    }

    #[test]
    fn nothing_matching_denies() {
        let sets = vec![PolicySet::new("01h0000000000000000000p001", "empty", 1)];
        let principal = Principal::system();
        let tool = ToolName::new("fs.read").unwrap();
        let req = PermissionReq::new("fs:read:data/sandbox/**", PermissionAction::Read);
        assert_eq!(
            PolicyEngine::evaluate(&sets, &request(&principal, &tool, &req)),
            Effect::Forbid
        );
        assert!(PolicyEngine::first_permit(&sets, &request(&principal, &tool, &req)).is_none());
    }

    #[test]
    fn forbid_beats_permit_whatever_the_order() {
        let principal = Principal::system();
        let tool = ToolName::new("fs.write").unwrap();
        let req = PermissionReq::new("fs:write:data/sandbox/forbidden/x", PermissionAction::Write);
        let permit = PolicyRule::permit("*", "fs.write", "data/sandbox/**");
        let forbid = PolicyRule::forbid("*", "fs.write", "data/sandbox/forbidden/**");

        for rules in [
            vec![permit.clone(), forbid.clone()],
            vec![forbid.clone(), permit],
        ] {
            let sets = vec![PolicySet::new("01h0000000000000000000p002", "baseline", 1)
                .with_rule(rules[0].clone())
                .with_rule(rules[1].clone())];
            assert_eq!(
                PolicyEngine::evaluate(&sets, &request(&principal, &tool, &req)),
                Effect::Forbid
            );
            let (id, set) =
                PolicyEngine::first_forbid(&sets, &request(&principal, &tool, &req)).unwrap();
            assert_eq!(id, forbid.id());
            assert_eq!(set, "baseline");
        }
    }

    #[test]
    fn a_permit_alone_allows() {
        let sets = vec![PolicySet::new("01h0000000000000000000p003", "baseline", 1)
            .with_rule(PolicyRule::permit("*", "fs.write", "data/sandbox/**"))];
        let principal = Principal::system();
        let tool = ToolName::new("fs.write").unwrap();
        let req = PermissionReq::new("fs:write:data/sandbox/a.txt", PermissionAction::Write);
        assert_eq!(
            PolicyEngine::evaluate(&sets, &request(&principal, &tool, &req)),
            Effect::Permit
        );
    }

    #[test]
    fn a_rule_matches_one_principal_one_namespace_and_one_tree() {
        let rule = PolicyRule::permit("01h00000000000000000000b01", "fs.*", "data/sandbox/**");
        let tool = ToolName::new("fs.read").unwrap();
        let inside = PermissionReq::new("fs:read:data/sandbox/a.txt", PermissionAction::Read);
        let outside = PermissionReq::new("fs:read:data/other/a.txt", PermissionAction::Read);
        let other_ns = PermissionReq::new("process:execute:cargo", PermissionAction::Execute);

        let owner = Principal::being("01h00000000000000000000b01");
        assert!(rule.matches(&request(&owner, &tool, &inside)));
        assert!(!rule.matches(&request(&owner, &tool, &outside)));
        assert!(!rule.matches(&request(&owner, &tool, &other_ns)));
        assert!(!rule.matches(&request(&Principal::system(), &tool, &inside)));
    }

    #[test]
    fn a_wildcard_action_covers_a_namespace_only() {
        assert!(action_pattern_matches("fs.*", "fs.write"));
        assert!(action_pattern_matches("*", "process.execute"));
        assert!(!action_pattern_matches("fs.*", "process.execute"));
        assert!(action_pattern_matches("fs.write", "fs.write"));
        assert!(!action_pattern_matches("fs.write", "fs.read"));
    }

    #[test]
    fn evaluation_is_a_pure_function_of_its_inputs() {
        let sets = vec![PolicySet::new("01h0000000000000000000p004", "baseline", 1)
            .with_rule(PolicyRule::permit("*", "fs.*", "data/sandbox/**"))
            .with_rule(PolicyRule::forbid(
                "*",
                "fs.write",
                "data/sandbox/secret/**",
            ))];
        let principal = Principal::system();
        let tool = ToolName::new("fs.read").unwrap();
        let req = PermissionReq::new("fs:read:data/sandbox/a", PermissionAction::Read);
        let first = PolicyEngine::evaluate(&sets, &request(&principal, &tool, &req));
        for _ in 0..8 {
            assert_eq!(
                PolicyEngine::evaluate(&sets, &request(&principal, &tool, &req)),
                first,
                "the same inputs must always yield the same decision"
            );
        }
    }

    #[test]
    fn a_rule_with_an_empty_field_is_refused() {
        assert!(PolicyRule::forbid("", "fs.write", "x").validate().is_err());
        assert!(PolicyRule::forbid("*", "fs", "x").validate().is_err());
        assert!(PolicyRule::forbid("*", "fs.write", "  ")
            .validate()
            .is_err());
        assert!(PolicyRule::forbid("*", "fs.write", "x").validate().is_ok());
    }

    #[test]
    fn rules_parse_from_the_stored_json() {
        let json = r#"[
            {"effect":"permit","principal":"*","action":"fs.write","resource":"data/sandbox/**"},
            {"effect":"forbid","principal":"*","action":"fs.write","resource":"data/sandbox/forbidden/**"}
        ]"#;
        let rules = PolicySet::parse_rules(json).unwrap();
        assert_eq!(rules.len(), 2);
        assert!(rules[0].effect == Effect::Permit);
        assert!(PolicySet::parse_rules(
            "[{\"effect\":\"allow\",\"principal\":\"*\",\"action\":\"a.b\",\"resource\":\"c\"}]"
        )
        .is_err());
    }

    #[test]
    fn a_rule_id_is_stable_and_content_derived() {
        let rule = PolicyRule::permit("*", "fs.write", "data/sandbox/**");
        assert_eq!(
            rule.id(),
            PolicyRule::permit("*", "fs.write", "data/sandbox/**").id()
        );
        assert_ne!(
            rule.id(),
            PolicyRule::forbid("*", "fs.write", "data/sandbox/**").id()
        );
        assert_eq!(rule.id().len(), 16);
    }

    #[test]
    fn matching_rules_reports_every_citation() {
        let sets = vec![PolicySet::new("01h0000000000000000000p005", "baseline", 1)
            .with_rule(PolicyRule::permit("*", "fs.write", "data/sandbox/**"))
            .with_rule(PolicyRule::forbid(
                "*",
                "fs.write",
                "data/sandbox/secret/**",
            ))];
        let principal = Principal::system();
        let tool = ToolName::new("fs.write").unwrap();
        let req = PermissionReq::new("fs:write:data/sandbox/secret/a", PermissionAction::Write);
        let matched = PolicyEngine::matching_rules(&sets, &request(&principal, &tool, &req));
        assert_eq!(matched.len(), 2);
        assert!(matched
            .iter()
            .any(|(_, _, effect)| *effect == Effect::Forbid));
    }

    #[test]
    fn effects_round_trip() {
        for effect in EFFECTS {
            assert_eq!(Effect::parse(effect.as_str()), Some(effect));
        }
        assert_eq!(Effect::parse("ALLOW"), None);
        assert!(Effect::Forbid.is_forbid());
    }

    #[test]
    fn a_set_must_be_named_and_versioned() {
        assert!(PolicySet::new("id", "", 1).validate().is_err());
        assert!(PolicySet::new("id", "baseline", 0).validate().is_err());
        assert!(PolicySet::new("id", "baseline", 1).validate().is_ok());
    }
}
