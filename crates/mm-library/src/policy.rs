//! Policies: scoped behavioural rules with immutable versions and fitness.
//!
//! A policy is a *rule about how to act*, and three things keep it from becoming
//! folklore: a **scope** that says where it applies, an **activation condition**
//! that says when it is relevant, and a **behaviour** that is a typed fragment
//! rather than a paragraph of prose. The behaviour is where the honesty lives —
//! [`PolicyBehavior`] refuses anything that does not parse as `key:value` pairs,
//! so a policy cannot be stored as "be sensible".
//!
//! Versions are immutable: [`Policy`] is built, never mutated in place, and a new
//! version carries its parent. That is what makes `policy history` a real answer to
//! "how did this rule get here".

use mm_core::{ActivationCondition, NamedNode};
use serde::{Deserialize, Serialize};

use crate::entry::{iri, EntryKind, LibraryEntry};
use crate::error::{LibraryError, Result};

/// Where a policy applies.
///
/// Not `Serialize`: two of its variants carry a `NamedNode`, and the stored form
/// is the string [`Scope::as_str`] produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Everywhere.
    Global,
    /// One domain.
    Domain(String),
    /// One frame.
    Frame(NamedNode),
    /// One relationship.
    Relationship(NamedNode),
}

impl Scope {
    /// The wire form stored in `policy_versions.scope_ttl`.
    pub fn as_str(&self) -> String {
        match self {
            Scope::Global => "global".to_string(),
            Scope::Domain(domain) => format!("domain:{domain}"),
            Scope::Frame(frame) => format!("frame:{}", frame.as_str()),
            Scope::Relationship(rel) => format!("relationship:{}", rel.as_str()),
        }
    }

    /// Parse the stored form.
    pub fn parse(text: &str) -> Option<Self> {
        match text.split_once(':') {
            None if text == "global" => Some(Scope::Global),
            Some(("domain", domain)) if !domain.is_empty() => {
                Some(Scope::Domain(domain.to_string()))
            }
            Some(("frame", iri)) => Some(Scope::Frame(NamedNode::new_unchecked(iri.to_string()))),
            Some(("relationship", iri)) => Some(Scope::Relationship(NamedNode::new_unchecked(
                iri.to_string(),
            ))),
            _ => None,
        }
    }
}

/// A typed DSL fragment, not free prose.
///
/// The grammar is deliberately tiny: a comma-separated list of `key:value` pairs,
/// with one to three numeric values inside a value. It is enough to express a
/// weighting (which is what the genome evolves) and little enough that a model
/// cannot smuggle a paragraph past it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PolicyBehavior(pub String);

impl PolicyBehavior {
    /// Build a behaviour, refusing anything that is not `key:value` pairs.
    pub fn new(text: &str) -> Result<Self> {
        let behavior = PolicyBehavior(text.trim().to_string());
        behavior.validate()?;
        Ok(behavior)
    }

    /// Build one from weights, the form the genome evolves.
    pub fn from_weights(weights: [f64; 3]) -> Self {
        // `+` joins the values of one key; `,` separates keys. A comma inside a
        // value would make the fragment ambiguous, and a behaviour a reader cannot
        // parse is prose again.
        PolicyBehavior(format!(
            "weights:{}+{}+{}",
            weights[0], weights[1], weights[2]
        ))
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The parsed `key → values` pairs.
    pub fn parse_pairs(&self) -> Result<Vec<(String, Vec<f64>)>> {
        let mut out = Vec::new();
        for pair in self.0.split(',') {
            let pair = pair.trim();
            let (key, value) = pair.split_once(':').ok_or_else(|| {
                LibraryError::validation("behavior", format!("{pair:?} is not key:value"))
            })?;
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(LibraryError::validation(
                    "behavior",
                    format!("{key:?} is not a legal key"),
                ));
            }
            let values: Vec<f64> = value
                .split('+')
                .map(|v| {
                    v.trim().parse::<f64>().map_err(|_| {
                        LibraryError::validation("behavior", format!("{v:?} is not a number"))
                    })
                })
                .collect::<Result<_>>()?;
            out.push((key.to_string(), values));
        }
        Ok(out)
    }

    /// The weights, when the behaviour is a weight vector.
    pub fn weights(&self) -> Option<[f64; 3]> {
        let pairs = self.parse_pairs().ok()?;
        let (_, values) = pairs.iter().find(|(key, _)| key == "weights")?;
        if values.len() != 3 {
            return None;
        }
        Some([values[0], values[1], values[2]])
    }

    /// Refuse prose.
    pub fn validate(&self) -> Result<()> {
        if self.0.is_empty() {
            return Err(LibraryError::validation("behavior", "must not be empty"));
        }
        if self.0.chars().count() > 200 {
            return Err(LibraryError::validation(
                "behavior",
                "a behaviour is a fragment, not a document",
            ));
        }
        self.parse_pairs().map(|_| ())
    }
}

impl std::fmt::Display for PolicyBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a policy has performed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FitnessRecord {
    /// How many times it was tried.
    pub trials: u32,
    /// How many times it succeeded.
    pub successes: u32,
    /// The mean utility of its outcomes.
    pub mean_utility: f32,
}

impl Default for FitnessRecord {
    fn default() -> Self {
        FitnessRecord {
            trials: 0,
            successes: 0,
            mean_utility: 0.0,
        }
    }
}

impl FitnessRecord {
    /// The success rate, or `None` before the first trial.
    ///
    /// `None` rather than zero: an untried policy is not a failing one, and
    /// [`crate::applicability`] treats it as the neutral prior.
    pub fn success_rate(&self) -> Option<f32> {
        (self.trials > 0).then(|| self.successes as f32 / self.trials as f32)
    }

    /// Record one outcome.
    pub fn record(&mut self, success: bool, utility: f32) {
        let total = self.mean_utility * self.trials as f32 + utility;
        self.trials += 1;
        if success {
            self.successes += 1;
        }
        self.mean_utility = total / self.trials as f32;
    }
}

/// One immutable policy version.
///
/// Not `Serialize`: a policy persists as Turtle and in `policy_versions`.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// The versioned IRI.
    pub iri: NamedNode,
    /// The version.
    pub version: u32,
    /// Where it applies.
    pub scope: Scope,
    /// When it becomes relevant.
    pub activation: ActivationCondition,
    /// What it does, as a typed fragment.
    pub behavior: PolicyBehavior,
    /// What backs it.
    pub evidence: Vec<NamedNode>,
    /// How it has performed.
    pub fitness: FitnessRecord,
    /// The library's confidence, in `[0,1]`.
    pub confidence: f32,
    /// The version it descends from; `None` only for version 1.
    pub parent: Option<NamedNode>,
    /// The generation it was evolved in; 0 when it was authored.
    pub generation: u32,
    /// The title.
    pub title: String,
    /// What the policy is for.
    pub purpose: String,
}

/// The change a new version introduces.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyDelta {
    /// The new behaviour.
    pub behavior: PolicyBehavior,
    /// The new scope.
    pub scope: Scope,
    /// The new activation condition.
    pub activation: ActivationCondition,
    /// The new confidence.
    pub confidence: f32,
    /// Why the version was created.
    pub reason: String,
}

impl Policy {
    /// Author version 1 of a policy.
    pub fn new(
        name: &str,
        scope: Scope,
        activation: ActivationCondition,
        behavior: PolicyBehavior,
        confidence: f32,
        title: &str,
        purpose: &str,
    ) -> Result<Self> {
        iri::check_slug(name)?;
        let policy = Policy {
            iri: iri::policy(name, 1),
            version: 1,
            scope,
            activation,
            behavior,
            evidence: Vec::new(),
            fitness: FitnessRecord::default(),
            confidence,
            parent: None,
            generation: 0,
            title: title.trim().to_string(),
            purpose: purpose.trim().to_string(),
        };
        policy.validate()?;
        Ok(policy)
    }

    /// The policy's name (the part before `@`).
    pub fn name(&self) -> String {
        iri::slug_of(self.iri.as_str()).unwrap_or_default()
    }

    /// Derive the next version from this one.
    ///
    /// The parent is *this* version and the version number is one higher, so a
    /// chain of deltas is also a chain of provenance. Nothing about `self` is
    /// mutated: an old version stays byte-identical.
    pub fn next(&self, delta: PolicyDelta, generation: u32) -> Result<Self> {
        let next = Policy {
            iri: iri::policy(&self.name(), self.version + 1),
            version: self.version + 1,
            scope: delta.scope,
            activation: delta.activation,
            behavior: delta.behavior,
            evidence: self.evidence.clone(),
            fitness: FitnessRecord::default(),
            confidence: delta.confidence,
            parent: Some(self.iri.clone()),
            generation,
            title: self.title.clone(),
            purpose: self.purpose.clone(),
        };
        next.validate()?;
        Ok(next)
    }

    /// Refuse a version whose parent is not the previous version.
    pub fn check_parent_chain(&self) -> Result<()> {
        match (self.version, &self.parent) {
            (1, None) => Ok(()),
            (1, Some(_)) => Err(LibraryError::validation(
                "parent",
                "version 1 has no parent",
            )),
            (_, None) => Err(LibraryError::validation(
                "parent",
                "every version after the first names its parent",
            )),
            (_, Some(parent)) => {
                let expected = iri::policy(&self.name(), self.version - 1);
                if parent.as_str() != expected.as_str() {
                    return Err(LibraryError::validation(
                        "parent",
                        format!(
                            "version {} must descend from {}",
                            self.version,
                            expected.as_str()
                        ),
                    ));
                }
                Ok(())
            }
        }
    }
}

impl LibraryEntry for Policy {
    fn kind(&self) -> EntryKind {
        EntryKind::Policy
    }

    fn version_iri(&self) -> NamedNode {
        self.iri.clone()
    }

    fn head_iri(&self) -> NamedNode {
        NamedNode::new_unchecked(format!("{}{}", crate::entry::POLICY_BASE, self.name()))
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn referenced_iris(&self) -> Vec<NamedNode> {
        let mut out = self.evidence.clone();
        if let Some(parent) = &self.parent {
            out.push(parent.clone());
        }
        out
    }

    fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(LibraryError::validation("title", "must not be empty"));
        }
        if self.purpose.trim().is_empty() {
            return Err(LibraryError::validation("purpose", "must not be empty"));
        }
        if self.version == 0 {
            return Err(LibraryError::validation("version", "must be at least 1"));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(LibraryError::validation(
                "confidence",
                format!("must be in [0,1], got {}", self.confidence),
            ));
        }
        if let Scope::Domain(domain) = &self.scope {
            if domain.trim().is_empty() {
                return Err(LibraryError::validation(
                    "scope",
                    "a domain scope needs a domain",
                ));
            }
        }
        self.behavior.validate()?;
        self.check_parent_chain()
    }

    fn to_turtle(&self) -> Result<String> {
        crate::rdf::policy_turtle(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activation() -> ActivationCondition {
        ActivationCondition::new("two candidate solutions are viable").unwrap()
    }

    fn policy() -> Policy {
        Policy::new(
            "prefer_simpler_solution",
            Scope::Global,
            activation(),
            PolicyBehavior::from_weights([1.0, 0.2, 0.2]),
            0.6,
            "Prefer the simpler solution",
            "Take the simpler of two viable options.",
        )
        .unwrap()
    }

    #[test]
    fn behaviour_must_be_a_typed_fragment_not_prose() {
        assert!(PolicyBehavior::new("be sensible about things").is_err());
        assert!(PolicyBehavior::new("weights:1.0+0.2+0.2").is_ok());
        // A comma inside a value is ambiguous and refused.
        assert!(PolicyBehavior::new("weights:1.0,0.2").is_err());
        assert_eq!(
            PolicyBehavior::from_weights([1.0, 0.5, 0.0]).weights(),
            Some([1.0, 0.5, 0.0])
        );
        assert!(PolicyBehavior::new("weights:notanumber").is_err());
        assert!(PolicyBehavior::new("").is_err());
    }

    #[test]
    fn a_scope_round_trips_through_its_stored_form() {
        for scope in [
            Scope::Global,
            Scope::Domain("software".into()),
            Scope::Frame(iri::frame("debugging")),
            Scope::Relationship(NamedNode::new_unchecked("https://metamind.dev/data/abc")),
        ] {
            assert_eq!(Scope::parse(&scope.as_str()), Some(scope));
        }
        assert_eq!(Scope::parse("nonsense"), None);
    }

    #[test]
    fn versions_are_immutable_and_parented() {
        let first = policy();
        assert!(first.parent.is_none());
        assert!(first.check_parent_chain().is_ok());

        let second = first
            .next(
                PolicyDelta {
                    behavior: PolicyBehavior::from_weights([1.0, 0.4, 0.1]),
                    scope: Scope::Global,
                    activation: activation(),
                    confidence: 0.65,
                    reason: "fitness gain".into(),
                },
                1,
            )
            .unwrap();
        assert_eq!(second.version, 2);
        assert_eq!(
            second.parent.as_ref().map(|node| node.as_str()),
            Some("https://metamind.dev/policy/prefer_simpler_solution@1")
        );
        assert_eq!(
            second.iri.as_str(),
            "https://metamind.dev/policy/prefer_simpler_solution@2"
        );
        // The first is untouched.
        assert_eq!(first.behavior.as_str(), "weights:1+0.2+0.2");
        second.check_parent_chain().unwrap();

        // A wrong parent is refused.
        let mut bad = second.clone();
        bad.parent = Some(iri::policy("prefer_simpler_solution", 5));
        assert!(bad.check_parent_chain().is_err());
    }

    #[test]
    fn fitness_is_none_before_the_first_trial_and_updates_in_place() {
        let mut record = FitnessRecord::default();
        assert_eq!(record.success_rate(), None);
        record.record(true, 1.0);
        record.record(false, 0.0);
        assert_eq!(record.trials, 2);
        assert_eq!(record.successes, 1);
        assert_eq!(record.success_rate(), Some(0.5));
        assert!((record.mean_utility - 0.5).abs() < 1e-6);
    }
}
