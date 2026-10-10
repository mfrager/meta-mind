//! Entry kinds, identifiers, and the declarative entry.
//!
//! The plan names a struct per declarative kind (`Doctrine`, `Principle`,
//! `Heuristic`, `Technique`, `Pattern`, `AntiPattern`, `Evaluation`). They differ
//! in exactly one field — their [`EntryKind`] — so the shape, the validation and
//! the codec live **once**, in [`Declarative`], and each kind is a constructor
//! ([`crate::doctrine`], [`crate::technique`]). Seven near-identical structs would
//! be seven places for the shape to drift; the kind is what distinguishes them and
//! it is already a field.
//!
//! Identifiers are path-style and human-diffable, because a library entry is a
//! *named, versioned thing* rather than a runtime record:
//!
//! * head       `https://metamind.dev/library/{kind}/{slug}`
//! * versioned  `…/library/{kind}/{slug}@{version}`
//! * policy     `https://metamind.dev/policy/{name}@{version}`
//! * frame      `…/library/frame/{slug}`
//! * skill      `…/library/skill/{slug}@{version}`
//!
//! Runtime records (frame instances, insights, runs) are ULID IRIs under
//! `https://metamind.dev/data/`, so a reader can tell a *named* entry from an
//! *occurrence* by looking at the IRI.

use std::collections::BTreeMap;

use mm_core::{ActivationCondition, NamedNode, Ulid};
use serde::{Deserialize, Serialize};

use crate::error::{LibraryError, Result};

/// The library's IRI base.
pub const LIBRARY_BASE: &str = "https://metamind.dev/library/";
/// The policy IRI base.
pub const POLICY_BASE: &str = "https://metamind.dev/policy/";
/// The longest slug the library accepts.
pub const MAX_SLUG_CHARS: usize = 64;

/// What sort of way of thinking an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntryKind {
    /// A general commitment about how to think.
    Doctrine,
    /// A rule that constrains method.
    Principle,
    /// A cheap rule of thumb that usually helps.
    Heuristic,
    /// A procedure for a kind of problem.
    Technique,
    /// A recurring shape of solution.
    Pattern,
    /// A remembered instance: problem, solution, outcome.
    Case,
    /// A recurring shape of failure, with its corrective rule.
    AntiPattern,
    /// An executable procedure, verified by a test.
    Skill,
    /// A behavioural rule with a scope, activation and fitness.
    Policy,
    /// A conceptual frame: roles, slots, affordances, scripts.
    Frame,
    /// A frame filled in for one episode.
    FrameInstance,
    /// A way of judging a result.
    Evaluation,
    /// A lesson distilled from trajectories, with votes.
    Insight,
    /// A procedure induced from a trajectory.
    Workflow,
}

/// Every entry kind, in the order the ontology declares them.
pub const ENTRY_KINDS: [EntryKind; 14] = [
    EntryKind::Doctrine,
    EntryKind::Principle,
    EntryKind::Heuristic,
    EntryKind::Technique,
    EntryKind::Pattern,
    EntryKind::Case,
    EntryKind::AntiPattern,
    EntryKind::Skill,
    EntryKind::Policy,
    EntryKind::Frame,
    EntryKind::FrameInstance,
    EntryKind::Evaluation,
    EntryKind::Insight,
    EntryKind::Workflow,
];

impl EntryKind {
    /// The wire name, matching the `library_entries.kind` CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            EntryKind::Doctrine => "Doctrine",
            EntryKind::Principle => "Principle",
            EntryKind::Heuristic => "Heuristic",
            EntryKind::Technique => "Technique",
            EntryKind::Pattern => "Pattern",
            EntryKind::Case => "Case",
            EntryKind::AntiPattern => "AntiPattern",
            EntryKind::Skill => "Skill",
            EntryKind::Policy => "Policy",
            EntryKind::Frame => "Frame",
            EntryKind::FrameInstance => "FrameInstance",
            EntryKind::Evaluation => "Evaluation",
            EntryKind::Insight => "Insight",
            EntryKind::Workflow => "Workflow",
        }
    }

    /// The `mm:` class the entry is typed as.
    pub fn rdf_class(self) -> &'static str {
        self.as_str()
    }

    /// The path segment of the entry's IRI.
    pub fn dir(self) -> &'static str {
        match self {
            EntryKind::Doctrine => "doctrine",
            EntryKind::Principle => "principle",
            EntryKind::Heuristic => "heuristic",
            EntryKind::Technique => "technique",
            EntryKind::Pattern => "pattern",
            EntryKind::Case => "case",
            EntryKind::AntiPattern => "antipattern",
            EntryKind::Skill => "skill",
            EntryKind::Policy => "policy",
            EntryKind::Frame => "frame",
            EntryKind::FrameInstance => "frame_instance",
            EntryKind::Evaluation => "evaluation",
            EntryKind::Insight => "insight",
            EntryKind::Workflow => "workflow",
        }
    }

    /// True for the kinds carried by [`Declarative`].
    pub fn is_declarative(self) -> bool {
        matches!(
            self,
            EntryKind::Doctrine
                | EntryKind::Principle
                | EntryKind::Heuristic
                | EntryKind::Technique
                | EntryKind::Pattern
                | EntryKind::AntiPattern
                | EntryKind::Evaluation
        )
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        ENTRY_KINDS
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(text))
    }
}

impl std::fmt::Display for EntryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The identifier grammar, in one place.
pub mod iri {
    use mm_core::{NamedNode, Ulid};

    use super::{EntryKind, LIBRARY_BASE, MAX_SLUG_CHARS, POLICY_BASE};
    use crate::error::{LibraryError, Result};

    /// True when `slug` is a legal path segment: lowercase, no spaces, bounded.
    pub fn is_slug(slug: &str) -> bool {
        !slug.is_empty()
            && slug.chars().count() <= MAX_SLUG_CHARS
            && slug
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            && !slug.starts_with('-')
            && !slug.ends_with('-')
    }

    /// Refuse a slug that would make an ambiguous or unreadable IRI.
    pub fn check_slug(slug: &str) -> Result<()> {
        if is_slug(slug) {
            Ok(())
        } else {
            Err(LibraryError::validation(
                "slug",
                format!(
                    "{slug:?} must be lowercase alphanumeric with `-`/`_`, \
                     at most {MAX_SLUG_CHARS} characters"
                ),
            ))
        }
    }

    /// The head IRI of an entry: `…/library/{kind}/{slug}`.
    pub fn library(kind: EntryKind, slug: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("{LIBRARY_BASE}{}/{slug}", kind.dir()))
    }

    /// The versioned IRI: `…/library/{kind}/{slug}@{version}`.
    pub fn versioned(kind: EntryKind, slug: &str, version: u32) -> NamedNode {
        NamedNode::new_unchecked(format!("{LIBRARY_BASE}{}/{slug}@{version}", kind.dir()))
    }

    /// A frame's IRI: `…/library/frame/{slug}`.
    pub fn frame(slug: &str) -> NamedNode {
        library(EntryKind::Frame, slug)
    }

    /// A skill's IRI: `…/library/skill/{slug}@{version}`.
    pub fn skill(slug: &str, version: u32) -> NamedNode {
        versioned(EntryKind::Skill, slug, version)
    }

    /// A policy's IRI: `…/policy/{name}@{version}`.
    pub fn policy(name: &str, version: u32) -> NamedNode {
        NamedNode::new_unchecked(format!("{POLICY_BASE}{name}@{version}"))
    }

    /// A runtime record's IRI: `…/data/{ulid}`.
    pub fn instance(id: &Ulid) -> NamedNode {
        mm_core::iri::data(id)
    }

    /// The head IRI of a versioned IRI, if it is one.
    pub fn head_of(versioned: &str) -> Option<String> {
        versioned.rsplit_once('@').map(|(head, _)| head.to_string())
    }

    /// The version of a versioned IRI, if it is one.
    pub fn version_of(versioned: &str) -> Option<u32> {
        versioned
            .rsplit_once('@')
            .and_then(|(_, version)| version.parse().ok())
    }

    /// The slug of a library IRI, if it is one.
    pub fn slug_of(iri: &str) -> Option<String> {
        let head = head_of(iri).unwrap_or_else(|| iri.to_string());
        head.rsplit('/').next().map(str::to_string)
    }

    /// True when `iri` is a policy IRI rather than a library entry IRI.
    pub fn is_policy(iri: &str) -> bool {
        iri.starts_with(POLICY_BASE)
    }
}

/// An entry a caller can write, version and select.
///
/// Deliberately small: an entry can name itself, say what backs it, and produce
/// its own Turtle. Where the Turtle comes from is [`crate::rdf`]'s business, not
/// each type's — one codec, one place for the canonical form to be defined.
pub trait LibraryEntry: Send + Sync {
    /// The kind.
    fn kind(&self) -> EntryKind;
    /// The versioned IRI the entry is stored under.
    ///
    /// Owned rather than borrowed because a frame instance's IRI is derived from
    /// its ULID, so it is not a field of the value; requiring a reference would
    /// force every such type to keep one around.
    fn version_iri(&self) -> NamedNode;
    /// The head IRI: the versioned IRI without its `@version`.
    fn head_iri(&self) -> NamedNode;
    /// The version.
    fn version(&self) -> u32;
    /// The title.
    fn title(&self) -> &str;
    /// Every IRI this entry references, for the orphan check.
    fn referenced_iris(&self) -> Vec<NamedNode>;
    /// Refuse an entry that is missing what its kind requires.
    fn validate(&self) -> Result<()>;
    /// The canonical Turtle the entry is stored as.
    fn to_turtle(&self) -> Result<String>;
}

/// A declarative entry: everything except the case, skill, policy, frame,
/// insight and workflow kinds, which carry fields of their own.
///
/// Not `Serialize`: `oxrdf::NamedNode` is not a serde type, and the persistence
/// form of an entry is its Turtle ([`crate::rdf`]), not a JSON mirror of the
/// struct. A second, lossier encoding would be a second place for the shape to
/// drift.
#[derive(Debug, Clone, PartialEq)]
pub struct Declarative {
    /// Which of the declarative kinds this is.
    pub kind: EntryKind,
    /// The versioned IRI.
    pub iri: NamedNode,
    /// The version.
    pub version: u32,
    /// The title.
    pub title: String,
    /// What the entry is for.
    pub purpose: String,
    /// The domains the entry has been used in.
    pub domain: Vec<String>,
    /// When it applies.
    pub applicable_when: Vec<ActivationCondition>,
    /// When it does not.
    pub inapplicable_when: Vec<ActivationCondition>,
    /// When applying it would be harmful.
    pub contraindications: Vec<ActivationCondition>,
    /// The entries or cases that back it.
    pub evidence: Vec<NamedNode>,
    /// Cases that illustrate it.
    pub examples: Vec<NamedNode>,
    /// Cases that tell against it.
    pub counterexamples: Vec<NamedNode>,
    /// The tests that decide it.
    pub tested_by: Vec<NamedNode>,
    /// Entries this one improves on.
    pub improves: Vec<NamedNode>,
    /// Entries this one replaces.
    pub replaces: Vec<NamedNode>,
    /// For an anti-pattern: what to do instead.
    pub corrective_rule: Option<String>,
    /// The failure this entry explains, when it explains one.
    pub failure_mode: Option<String>,
    /// How confident the library is, in `[0,1]`.
    pub confidence: Option<f32>,
    /// The stored ACT-R-style activation blend, in `[0,1]`.
    pub activation_score: f32,
    /// How well the entry has worked, per domain.
    pub domain_fitness: BTreeMap<String, f32>,
    /// Trajectories it was distilled from.
    pub derived_from_trajectory: Vec<String>,
}

impl Declarative {
    /// A skeleton entry: kind, slug, title and purpose; everything else is added
    /// by the builder methods.
    pub fn new(kind: EntryKind, slug: &str, title: &str, purpose: &str) -> Result<Self> {
        if !kind.is_declarative() {
            return Err(LibraryError::validation(
                "kind",
                format!("{kind} is not a declarative entry kind"),
            ));
        }
        iri::check_slug(slug)?;
        let entry = Declarative {
            kind,
            iri: iri::versioned(kind, slug, 1),
            version: 1,
            title: title.trim().to_string(),
            purpose: purpose.trim().to_string(),
            domain: Vec::new(),
            applicable_when: Vec::new(),
            inapplicable_when: Vec::new(),
            contraindications: Vec::new(),
            evidence: Vec::new(),
            examples: Vec::new(),
            counterexamples: Vec::new(),
            tested_by: Vec::new(),
            improves: Vec::new(),
            replaces: Vec::new(),
            corrective_rule: None,
            failure_mode: None,
            confidence: None,
            activation_score: 0.5,
            domain_fitness: BTreeMap::new(),
            derived_from_trajectory: Vec::new(),
        };
        entry.check_scaffold()?;
        Ok(entry)
    }

    /// The checks that hold for an entry *under construction*: everything except
    /// the kind-specific completeness.
    ///
    /// Completeness is a commit-time rule, not a construction-time one — an entry
    /// is written by filling it in, and requiring its evidence before it can be
    /// named would make the builder unusable. [`LibraryEntry::validate`] is the
    /// commit-time rule and it is what [`crate::rdf`] and the store call.
    pub fn check_scaffold(&self) -> Result<()> {
        if !self.kind.is_declarative() {
            return Err(LibraryError::validation(
                "kind",
                format!("{} is not a declarative entry kind", self.kind),
            ));
        }
        if self.title.trim().is_empty() {
            return Err(LibraryError::validation("title", "must not be empty"));
        }
        if self.purpose.trim().is_empty() {
            return Err(LibraryError::validation("purpose", "must not be empty"));
        }
        if self.version == 0 {
            return Err(LibraryError::validation("version", "must be at least 1"));
        }
        Ok(())
    }

    /// Use this version (and re-derive the IRI).
    pub fn with_version(mut self, version: u32) -> Result<Self> {
        if version == 0 {
            return Err(LibraryError::validation("version", "must be at least 1"));
        }
        let slug = iri::slug_of(self.iri.as_str()).unwrap_or_default();
        self.iri = iri::versioned(self.kind, &slug, version);
        self.version = version;
        Ok(self)
    }

    /// Add a domain.
    pub fn with_domain(mut self, domain: &str) -> Self {
        self.domain.push(domain.to_string());
        self
    }

    /// Record when the entry applies.
    pub fn when(mut self, condition: ActivationCondition) -> Self {
        self.applicable_when.push(condition);
        self
    }

    /// Record when the entry does not apply.
    pub fn unless(mut self, condition: ActivationCondition) -> Self {
        self.inapplicable_when.push(condition);
        self
    }

    /// Record a contraindication.
    pub fn contraindicated_when(mut self, condition: ActivationCondition) -> Self {
        self.contraindications.push(condition);
        self
    }

    /// Name something that backs the entry.
    pub fn backed_by(mut self, iri: NamedNode) -> Self {
        self.evidence.push(iri);
        self
    }

    /// Name a case that illustrates the entry.
    pub fn illustrated_by(mut self, iri: NamedNode) -> Self {
        self.examples.push(iri);
        self
    }

    /// Name a case that tells against the entry.
    pub fn countered_by(mut self, iri: NamedNode) -> Self {
        self.counterexamples.push(iri);
        self
    }

    /// Name the test that decides the entry.
    pub fn tested_by(mut self, iri: NamedNode) -> Self {
        self.tested_by.push(iri);
        self
    }

    /// State what to do instead of the failure this entry names.
    pub fn corrective_rule(mut self, rule: &str) -> Self {
        self.corrective_rule = Some(rule.trim().to_string());
        self
    }

    /// Set the library's confidence.
    pub fn with_confidence(mut self, confidence: f32) -> Result<Self> {
        if !(0.0..=1.0).contains(&confidence) {
            return Err(LibraryError::validation(
                "confidence",
                format!("must be in [0,1], got {confidence}"),
            ));
        }
        self.confidence = Some(confidence);
        Ok(self)
    }

    /// Set the stored activation blend.
    pub fn with_activation(mut self, score: f32) -> Result<Self> {
        if !(0.0..=1.0).contains(&score) {
            return Err(LibraryError::validation(
                "activation_score",
                format!("must be in [0,1], got {score}"),
            ));
        }
        self.activation_score = score;
        Ok(self)
    }

    /// Record how well the entry has worked in a domain.
    pub fn with_domain_fitness(mut self, domain: &str, score: f32) -> Result<Self> {
        if !(0.0..=1.0).contains(&score) {
            return Err(LibraryError::validation(
                "domain_fitness",
                format!("must be in [0,1], got {score}"),
            ));
        }
        self.domain_fitness.insert(domain.to_string(), score);
        Ok(self)
    }

    /// Record a trajectory this entry came from.
    pub fn derived_from(mut self, trajectory: &str) -> Self {
        self.derived_from_trajectory.push(trajectory.to_string());
        self
    }

    /// The slug.
    pub fn slug(&self) -> String {
        iri::slug_of(self.iri.as_str()).unwrap_or_default()
    }
}

impl LibraryEntry for Declarative {
    fn kind(&self) -> EntryKind {
        self.kind
    }

    fn version_iri(&self) -> NamedNode {
        self.iri.clone()
    }

    fn head_iri(&self) -> NamedNode {
        let slug = iri::slug_of(self.iri.as_str()).unwrap_or_default();
        iri::library(self.kind, &slug)
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn referenced_iris(&self) -> Vec<NamedNode> {
        let mut out: Vec<NamedNode> = Vec::new();
        out.extend(self.evidence.iter().cloned());
        out.extend(self.examples.iter().cloned());
        out.extend(self.counterexamples.iter().cloned());
        out.extend(self.tested_by.iter().cloned());
        out.extend(self.improves.iter().cloned());
        out.extend(self.replaces.iter().cloned());
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out.dedup_by(|a, b| a.as_str() == b.as_str());
        out
    }

    /// The kind-specific requirements, refusing with the *field* that is missing.
    ///
    /// These mirror `ontology/shapes/library.shacl.ttl`: the shape is the hard gate
    /// at commit, and this is the same rule applied earlier so a caller gets a
    /// useful field name instead of a violation list.
    fn validate(&self) -> Result<()> {
        self.check_scaffold()?;
        if !(0.0..=1.0).contains(&self.activation_score) {
            return Err(LibraryError::validation(
                "activation_score",
                format!("must be in [0,1], got {}", self.activation_score),
            ));
        }
        for (domain, score) in &self.domain_fitness {
            if !(0.0..=1.0).contains(score) {
                return Err(LibraryError::validation(
                    "domain_fitness",
                    format!("{domain}: {score} is outside [0,1]"),
                ));
            }
        }
        match self.kind {
            EntryKind::Technique => {
                if self.applicable_when.is_empty() {
                    return Err(LibraryError::validation(
                        "applicable_when",
                        "a technique states when it applies",
                    ));
                }
                if self.evidence.is_empty() {
                    return Err(LibraryError::validation(
                        "evidence",
                        "a technique names what backs it",
                    ));
                }
                if self.examples.is_empty() {
                    return Err(LibraryError::validation(
                        "example",
                        "a technique names a case it was tried on",
                    ));
                }
                if self.confidence.is_none() {
                    return Err(LibraryError::validation(
                        "confidence",
                        "a technique carries a confidence",
                    ));
                }
            }
            EntryKind::Heuristic | EntryKind::Pattern | EntryKind::Doctrine => {
                if self.applicable_when.is_empty() {
                    return Err(LibraryError::validation(
                        "applicable_when",
                        "the entry states when it applies",
                    ));
                }
                if self.evidence.is_empty() {
                    return Err(LibraryError::validation(
                        "evidence",
                        "the entry names what backs it",
                    ));
                }
            }
            EntryKind::Principle if self.evidence.is_empty() => {
                return Err(LibraryError::validation(
                    "evidence",
                    "a principle names what backs it",
                ));
            }
            EntryKind::AntiPattern => {
                if self.evidence.is_empty() {
                    return Err(LibraryError::validation(
                        "evidence",
                        "an anti-pattern names what backs it",
                    ));
                }
                match &self.corrective_rule {
                    None => {
                        return Err(LibraryError::validation(
                            "corrective_rule",
                            "an anti-pattern without a corrective rule is a complaint",
                        ))
                    }
                    Some(rule) if rule.trim().is_empty() => {
                        return Err(LibraryError::validation(
                            "corrective_rule",
                            "must not be blank",
                        ))
                    }
                    Some(_) => {}
                }
            }
            EntryKind::Evaluation if self.evidence.is_empty() => {
                return Err(LibraryError::validation(
                    "evidence",
                    "an evaluation names what backs it",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn to_turtle(&self) -> Result<String> {
        crate::rdf::declarative_turtle(self)
    }
}

/// True when two entries are the same named thing at the same version.
pub fn same_version(a: &NamedNode, b: &NamedNode) -> bool {
    a.as_str() == b.as_str()
}

/// The ULID a runtime record's IRI carries.
pub fn instance_ulid(iri: &str) -> Option<Ulid> {
    mm_core::iri::data_ulid(iri).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_checked() {
        assert!(iri::is_slug("api-capability-check"));
        assert!(iri::is_slug("prefer_simpler_solution"));
        assert!(!iri::is_slug(""));
        assert!(!iri::is_slug("Upper"));
        assert!(!iri::is_slug("has space"));
        assert!(!iri::is_slug("-leading"));
        assert!(iri::check_slug("ok").is_ok());
        assert!(iri::check_slug("Bad").is_err());
    }

    #[test]
    fn identifiers_follow_the_plan_grammar() {
        assert_eq!(
            iri::library(EntryKind::Case, "outage-rollback").as_str(),
            "https://metamind.dev/library/case/outage-rollback"
        );
        assert_eq!(
            iri::skill("api-capability-check", 1).as_str(),
            "https://metamind.dev/library/skill/api-capability-check@1"
        );
        assert_eq!(
            iri::policy("prefer_simpler_solution", 2).as_str(),
            "https://metamind.dev/policy/prefer_simpler_solution@2"
        );
        assert_eq!(
            iri::versioned(EntryKind::Technique, "decompose", 3).as_str(),
            "https://metamind.dev/library/technique/decompose@3"
        );
    }

    #[test]
    fn a_versioned_iri_round_trips() {
        let iri = "https://metamind.dev/policy/prefer_simpler_solution@2";
        assert_eq!(
            iri::head_of(iri).as_deref(),
            Some("https://metamind.dev/policy/prefer_simpler_solution")
        );
        assert_eq!(iri::version_of(iri), Some(2));
        assert_eq!(
            iri::slug_of(iri).as_deref(),
            Some("prefer_simpler_solution")
        );
        assert!(iri::is_policy(iri));
        assert!(!iri::is_policy("https://metamind.dev/library/case/x@1"));
    }

    #[test]
    fn every_kind_round_trips_through_its_wire_name() {
        for kind in ENTRY_KINDS {
            assert_eq!(EntryKind::parse(kind.as_str()), Some(kind));
            assert!(!kind.dir().is_empty());
        }
        assert_eq!(EntryKind::parse("technique"), Some(EntryKind::Technique));
        assert_eq!(EntryKind::parse("nope"), None);
    }

    #[test]
    fn a_technique_without_its_conditions_is_refused_at_commit() {
        // Construction is allowed — an entry is written by filling it in — but the
        // commit-time rule refuses it while it is incomplete.
        let bare =
            Declarative::new(EntryKind::Technique, "decompose", "Decompose", "Split it").unwrap();
        let error = bare.validate().unwrap_err();
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("applicable_when"));

        let complete = bare
            .when(ActivationCondition::new("parts are independent").unwrap())
            .backed_by(iri::library(EntryKind::Case, "c"))
            .illustrated_by(iri::library(EntryKind::Case, "c"))
            .with_confidence(0.8)
            .unwrap();
        complete.validate().unwrap();
    }

    #[test]
    fn an_anti_pattern_needs_a_corrective_rule() {
        let entry = Declarative::new(
            EntryKind::AntiPattern,
            "tune-blind",
            "Blind tuning",
            "Tuning",
        )
        .unwrap()
        .backed_by(iri::library(EntryKind::Case, "c"));
        assert!(entry.validate().is_err());
        entry.corrective_rule("measure first").validate().unwrap();
    }

    #[test]
    fn a_non_declarative_kind_is_refused_by_the_declarative_constructor() {
        assert!(Declarative::new(EntryKind::Skill, "x", "t", "p").is_err());
        assert!(Declarative::new(EntryKind::Frame, "x", "t", "p").is_err());
    }

    #[test]
    fn a_slug_that_cannot_be_an_iri_is_refused() {
        assert!(Declarative::new(EntryKind::Technique, "Not A Slug", "t", "p").is_err());
    }
}
