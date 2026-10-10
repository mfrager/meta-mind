//! The tool protocol: an MCP-shaped spec, extended with the two facts a safety
//! layer needs and MCP leaves to prose.
//!
//! MCP describes a tool with a name, a description, and a JSON Schema for input and
//! output. That is enough to *call* a tool and not enough to *authorize* one, so
//! every [`ToolSpec`] additionally declares:
//!
//! * [`Reversibility`] — whether the action can be undone, undone at a cost, or not
//!   at all. [`crate::rollback`] refuses for `Irreversible`, and the firewall
//!   requires escalation for it.
//! * [`SideEffectClass`] — how far the effect reaches. A `None` tool is pure, and
//!   purity is what makes a tool safe to retry.
//! * [`ToolAnnotations`] — the MCP hints (`readOnlyHint`, `destructiveHint`,
//!   `idempotentHint`, `openWorldHint`), kept as *hints*: they are authoritative
//!   only in the direction of caution. [`ToolAnnotations::contradicts`] reports a
//!   hint that claims a tool is safer than its declared reversibility and side
//!   effects allow, and the registry refuses the registration.
//!
//! `permissions` is the list a [`crate::sandbox::Sandbox`] turns into capabilities.
//! A tool that does not declare a capability does not get it: the guard is derived
//! from the declaration, so "the tool forgot to ask" and "the tool was denied" are
//! the same outcome, which is the only way a default-deny model stays true.

use serde::{Deserialize, Serialize};

use crate::error::RegistryError;
use crate::permissions::PermissionReq;
use crate::sandbox::SandboxTier;

/// A tool's name: `namespace.verb`, e.g. `fs.read`, `verify.run`.
///
/// The shape is enforced because the name is an API: it appears in policy rules, in
/// `tool_pattern` grants, in ledger payloads and in the CLI. A free-form name would
/// make a policy rule like `fs.*` mean whatever the last registrant decided.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolName(pub String);

impl ToolName {
    /// A name, refused unless it is `namespace.verb` in lowercase ASCII.
    pub fn new(text: impl Into<String>) -> Result<Self, RegistryError> {
        let text = text.into();
        let (namespace, verb) = text
            .split_once('.')
            .ok_or_else(|| RegistryError::BadName(text.clone()))?;
        let well_formed = |part: &str| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        };
        if !well_formed(namespace) || !well_formed(verb) || verb.contains('.') {
            return Err(RegistryError::BadName(text));
        }
        Ok(ToolName(text))
    }

    /// The name as it is spelled everywhere.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The namespace before the dot.
    pub fn namespace(&self) -> &str {
        self.0.split_once('.').map(|(n, _)| n).unwrap_or(&self.0)
    }

    /// True when `pattern` covers this name. `*` is a whole-namespace wildcard.
    ///
    /// Only the namespace may be wildcarded: a pattern that could match across
    /// namespaces would make a grant on `fs.*` accidentally cover `process.exec`,
    /// which is the confused-deputy shape the phase's fourth risk row names.
    pub fn matches(&self, pattern: &str) -> bool {
        match pattern.split_once('.') {
            Some((namespace, "*")) => namespace == self.namespace(),
            _ => pattern == self.0,
        }
    }
}

impl std::fmt::Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A reference to an ontology class or shape the tool's schema is registered under,
/// e.g. `mm:FilePath`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaRef(pub String);

/// A JSON Schema document, as MCP's `inputSchema`/`outputSchema` carry it.
///
/// Held as an opaque [`serde_json::Value`] rather than as a Rust type: the schema is
/// data a caller may write, version and diff, and a tool whose schema could not
/// round-trip through JSON would make the registry's persisted `spec_json` lossy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonSchema(pub serde_json::Value);

impl JsonSchema {
    /// A schema requiring an object with the named string properties.
    pub fn object_with_strings(properties: &[&str]) -> Self {
        let mut props = serde_json::Map::new();
        for name in properties {
            props.insert((*name).to_string(), serde_json::json!({ "type": "string" }));
        }
        JsonSchema(serde_json::json!({
            "type": "object",
            "properties": props,
            "required": properties,
        }))
    }

    /// A schema requiring the named properties without constraining their types.
    pub fn object(properties: &[&str]) -> Self {
        JsonSchema(serde_json::json!({
            "type": "object",
            "properties": properties
                .iter()
                .map(|name| ((*name).to_string(), serde_json::json!({})))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            "required": properties,
        }))
    }

    /// A schema accepting any object.
    pub fn any_object() -> Self {
        JsonSchema(serde_json::json!({ "type": "object" }))
    }

    /// The `required` names the schema declares, sorted.
    pub fn required(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .0
            .get("required")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Refuse a schema that is not an object schema, or that declares a `required`
    /// name it does not describe in `properties`.
    ///
    /// The check is deliberately small. A full JSON Schema implementation would be a
    /// second source of truth about what a tool accepts, and the tool itself already
    /// validates its arguments; what the registry owes a caller is that the schema
    /// *can* be read and is internally consistent.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let object = self
            .0
            .as_object()
            .ok_or_else(|| "the schema is not a JSON object".to_string())?;
        if let Some(kind) = object.get("type") {
            if kind != &serde_json::json!("object") {
                return Err(format!("the schema declares type {kind}, not an object"));
            }
        }
        let properties = object
            .get("properties")
            .and_then(serde_json::Value::as_object);
        for name in self.required() {
            if !properties.map(|p| p.contains_key(&name)).unwrap_or(false) {
                return Err(format!(
                    "the schema requires {name:?} but declares no property for it"
                ));
            }
        }
        Ok(())
    }
}

/// How hard an action is to undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    /// Undoable with no cost: a snapshot restores it byte for byte.
    Reversible,
    /// Undoable at a cost: a compensating action is needed, and a snapshot may not
    /// be enough.
    Compensatable,
    /// Not undoable. Requires escalation and an explicit confirmation.
    Irreversible,
}

/// Every reversibility, in the order the ledger renders them.
pub const REVERSIBILITIES: [Reversibility; 3] = [
    Reversibility::Reversible,
    Reversibility::Compensatable,
    Reversibility::Irreversible,
];

impl Reversibility {
    /// The stable wire name, which is also the `tool_registry.reversibility` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Reversibility::Reversible => "reversible",
            Reversibility::Compensatable => "compensatable",
            Reversibility::Irreversible => "irreversible",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        REVERSIBILITIES
            .into_iter()
            .find(|value| value.as_str() == text.trim().to_ascii_lowercase())
    }

    /// True when a snapshot before the action is worth taking.
    pub fn is_snapshotted(self) -> bool {
        matches!(
            self,
            Reversibility::Reversible | Reversibility::Compensatable
        )
    }
}

impl std::fmt::Display for Reversibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How far a tool's effect reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    /// Pure: no state outside the call.
    None,
    /// Touches the kernel's own local state.
    Local,
    /// Touches something outside the kernel.
    External,
    /// Touches something outside the kernel that cannot be undone.
    Irreversible,
}

/// Every side-effect class, in the order the ledger renders them.
pub const SIDE_EFFECT_CLASSES: [SideEffectClass; 4] = [
    SideEffectClass::None,
    SideEffectClass::Local,
    SideEffectClass::External,
    SideEffectClass::Irreversible,
];

impl SideEffectClass {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            SideEffectClass::None => "none",
            SideEffectClass::Local => "local",
            SideEffectClass::External => "external",
            SideEffectClass::Irreversible => "irreversible",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        SIDE_EFFECT_CLASSES
            .into_iter()
            .find(|value| value.as_str() == text.trim().to_ascii_lowercase())
    }

    /// True when calling the tool twice is a visible difference.
    pub fn is_effectful(self) -> bool {
        !matches!(self, SideEffectClass::None)
    }
}

impl std::fmt::Display for SideEffectClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The MCP hints a tool may declare about itself.
///
/// They are hints in exactly one direction. A tool that declares `destructive: true`
/// is treated as destructive whatever else it says, and a tool that declares
/// `read_only: true` while declaring an `External` side effect is refused at
/// registration rather than believed. The asymmetry is the point: a hint may make
/// the firewall *more* cautious and never less.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAnnotations {
    /// The tool does not modify anything.
    pub read_only: bool,
    /// The tool may perform a destructive update.
    pub destructive: bool,
    /// Calling it twice with the same arguments has the same effect as once.
    pub idempotent: bool,
    /// The tool interacts with an open world of external entities.
    pub open_world: bool,
}

impl ToolAnnotations {
    /// The annotations a pure, retryable, local tool declares.
    pub fn pure() -> Self {
        ToolAnnotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
        }
    }

    /// The hint that a tool's own declaration contradicts, if any.
    ///
    /// `read_only` with a side effect, `idempotent` on an irreversible tool, and
    /// `destructive: false` on an irreversible tool are all contradictions, and each
    /// is a claim that the tool is safer than its own reversibility says.
    pub fn contradicts(
        self,
        reversibility: Reversibility,
        side_effects: SideEffectClass,
    ) -> Option<String> {
        if self.read_only && side_effects.is_effectful() {
            return Some(format!(
                "it declares read_only but its side effects are {}",
                side_effects
            ));
        }
        if self.idempotent && reversibility == Reversibility::Irreversible {
            return Some(
                "it declares idempotent but it is irreversible, so a retry cannot be proven safe"
                    .to_string(),
            );
        }
        if !self.destructive && side_effects == SideEffectClass::Irreversible {
            return Some(
                "it declares destructive=false but its side effects are irreversible".to_string(),
            );
        }
        None
    }
}

/// Everything a caller can learn about a tool without calling it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// The tool's name.
    pub name: ToolName,
    /// The tool's version. A change to the schema or the capabilities needs one.
    pub version: String,
    /// One sentence a model can select on.
    pub description: String,
    /// MCP `inputSchema`.
    pub input_schema: JsonSchema,
    /// MCP `outputSchema`.
    pub output_schema: JsonSchema,
    /// The capabilities the tool needs. It gets exactly these and no others.
    pub permissions: Vec<PermissionReq>,
    /// How hard the action is to undo.
    pub reversibility: Reversibility,
    /// How far the effect reaches.
    pub side_effects: SideEffectClass,
    /// The tool's own hints about itself.
    pub annotations: ToolAnnotations,
    /// The module IRI this tool is implemented in.
    pub module_uri: String,
    /// The isolation tier the tool runs in.
    pub sandbox_tier: SandboxTier,
}

impl ToolSpec {
    /// Refuse a spec that is not internally consistent.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.version.trim().is_empty() {
            return Err("it declares no version".to_string());
        }
        if self.description.trim().is_empty() {
            return Err("it declares no description".to_string());
        }
        self.input_schema.validate()?;
        self.output_schema.validate()?;
        if !self
            .module_uri
            .starts_with("https://metamind.dev/code/module/")
        {
            return Err(format!(
                "its module_uri {:?} is not a Metamind module IRI",
                self.module_uri
            ));
        }
        if let Some(contradiction) = self
            .annotations
            .contradicts(self.reversibility, self.side_effects)
        {
            return Err(contradiction);
        }
        let mut seen = std::collections::BTreeSet::new();
        for permission in &self.permissions {
            if !seen.insert(permission.canonical()) {
                return Err(format!(
                    "it declares the capability {} twice",
                    permission.canonical()
                ));
            }
            permission.validate()?;
        }
        Ok(())
    }

    /// Refuse a registration whose spec is invalid.
    pub fn checked(self) -> std::result::Result<Self, RegistryError> {
        self.validate()
            .map_err(|reason| RegistryError::InvalidSpec {
                tool: self.name.0.clone(),
                reason,
            })?;
        Ok(self)
    }

    /// A digest of everything that makes this spec a different contract.
    ///
    /// The digest covers the schema, the capabilities and the safety declarations,
    /// because those are what a policy rule was written against. It deliberately does
    /// not cover the description: rewording a description must not invalidate a
    /// grant, or every doc fix would become a permission review.
    pub fn digest(&self) -> String {
        let mut permissions: Vec<String> = self
            .permissions
            .iter()
            .map(PermissionReq::canonical)
            .collect();
        permissions.sort();
        mm_core::hash_fields(&[
            self.name.as_str(),
            &self.version,
            &crate::canonical_json(&self.input_schema.0),
            &crate::canonical_json(&self.output_schema.0),
            &permissions.join(","),
            self.reversibility.as_str(),
            self.side_effects.as_str(),
            self.sandbox_tier.as_str(),
        ])
    }

    /// True when the tool changes nothing outside its own call.
    pub fn is_pure(&self) -> bool {
        self.side_effects == SideEffectClass::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::PermissionAction;

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: ToolName::new(name).unwrap(),
            version: "0.1.0".into(),
            description: "a tool".into(),
            input_schema: JsonSchema::object_with_strings(&["path"]),
            output_schema: JsonSchema::any_object(),
            permissions: vec![PermissionReq::new(
                "fs:read:data/sandbox/**",
                PermissionAction::Read,
            )],
            reversibility: Reversibility::Reversible,
            side_effects: SideEffectClass::Local,
            annotations: ToolAnnotations {
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
    fn a_tool_name_is_a_namespace_and_a_verb() {
        assert_eq!(ToolName::new("fs.read").unwrap().namespace(), "fs");
        assert!(ToolName::new("fs").is_err());
        assert!(ToolName::new("fs.").is_err());
        assert!(ToolName::new("FS.read").is_err());
        assert!(ToolName::new("fs.read.deeper").is_err());
        assert_eq!(ToolName::new("verify.run").unwrap().as_str(), "verify.run");
    }

    #[test]
    fn a_wildcard_covers_one_namespace_only() {
        let name = ToolName::new("fs.read").unwrap();
        assert!(name.matches("fs.*"));
        assert!(name.matches("fs.read"));
        assert!(!name.matches("process.*"));
        assert!(!name.matches("*"));
    }

    #[test]
    fn a_read_only_hint_on_an_effectful_tool_is_refused() {
        let mut bad = spec("fs.write");
        bad.annotations = ToolAnnotations::pure();
        assert!(bad.validate().unwrap_err().contains("read_only"));

        // The permissive direction is allowed: declaring destructive on a
        // reversible tool is extra caution, not a contradiction.
        let mut cautious = spec("fs.write");
        cautious.annotations.destructive = true;
        assert!(cautious.validate().is_ok());
    }

    #[test]
    fn an_idempotent_irreversible_tool_is_refused() {
        let mut bad = spec("http.post");
        bad.reversibility = Reversibility::Irreversible;
        bad.side_effects = SideEffectClass::Irreversible;
        bad.annotations = ToolAnnotations {
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: true,
        };
        assert!(bad.validate().unwrap_err().contains("idempotent"));
    }

    #[test]
    fn a_schema_that_requires_an_undeclared_property_is_refused() {
        let schema = JsonSchema(serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path", "text"]
        }));
        assert!(schema.validate().unwrap_err().contains("text"));
        assert_eq!(
            JsonSchema::object_with_strings(&["a", "b"]).required(),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn a_non_metamind_module_uri_is_refused() {
        let mut bad = spec("fs.read");
        bad.module_uri = "https://example.com/tools".into();
        assert!(bad.validate().unwrap_err().contains("module IRI"));
    }

    #[test]
    fn a_duplicate_capability_is_refused() {
        let mut bad = spec("fs.read");
        bad.permissions.push(bad.permissions[0].clone());
        assert!(bad.validate().unwrap_err().contains("twice"));
    }

    #[test]
    fn the_digest_ignores_the_description_and_tracks_the_contract() {
        let base = spec("fs.read");
        let mut reworded = base.clone();
        reworded.description = "a different sentence".into();
        assert_eq!(base.digest(), reworded.digest());

        let mut new_capability = base.clone();
        new_capability.permissions.push(PermissionReq::new(
            "fs:write:data/sandbox/**",
            PermissionAction::Write,
        ));
        assert_ne!(base.digest(), new_capability.digest());

        let mut new_tier = base.clone();
        new_tier.sandbox_tier = SandboxTier::MicroVm;
        assert_ne!(base.digest(), new_tier.digest());
    }

    #[test]
    fn a_pure_tool_says_so() {
        let mut pure = spec("math.check");
        pure.side_effects = SideEffectClass::None;
        pure.annotations = ToolAnnotations::pure();
        assert!(pure.is_pure());
        assert!(pure.validate().is_ok());
        assert!(!spec("fs.read").is_pure());
    }

    #[test]
    fn reversibility_and_side_effects_round_trip() {
        for value in REVERSIBILITIES {
            assert_eq!(Reversibility::parse(value.as_str()), Some(value));
        }
        for value in SIDE_EFFECT_CLASSES {
            assert_eq!(SideEffectClass::parse(value.as_str()), Some(value));
        }
        assert_eq!(Reversibility::parse("nonsense"), None);
        assert!(Reversibility::Reversible.is_snapshotted());
        assert!(!Reversibility::Irreversible.is_snapshotted());
    }
}
