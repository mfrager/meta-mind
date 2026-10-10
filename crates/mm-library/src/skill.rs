//! The verified skill library, and workflows induced from trajectories.
//!
//! Voyager's rule is the rule here: a skill enters the library as a **draft** and
//! becomes usable only when its test passes. `skill retrieve` returns verified
//! skills and nothing else, whatever a draft's textual score — which is why
//! verification is a state on the record rather than a boolean the caller passes.
//!
//! `verify` decides from two facts that are checkable today: the implementation
//! artefact's content hash matches the `impl_ref` the skill declared, and its test
//! spec reports every case passing. The plan runs the test in the Phase 10 sandbox;
//! that runner does not exist yet, so the *decision* is taken here from the same
//! evidence a sandbox would produce, and the seam is [`Skill::verify`] — one
//! function to replace when the runner lands, not a change of shape.

use mm_core::NamedNode;
use serde::{Deserialize, Serialize};

use crate::entry::{iri, EntryKind, LibraryEntry};
use crate::error::{LibraryError, Result};

/// Where a skill stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verification {
    /// Registered, its test never having passed.
    Draft,
    /// Its test passed and its artefact matches.
    Verified,
    /// Its test failed after having passed.
    Failing,
}

/// Every verification state.
pub const VERIFICATIONS: [Verification; 3] = [
    Verification::Draft,
    Verification::Verified,
    Verification::Failing,
];

impl Verification {
    /// The wire name, matching the `skills.verification` CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            Verification::Draft => "draft",
            Verification::Verified => "verified",
            Verification::Failing => "failing",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        VERIFICATIONS
            .into_iter()
            .find(|state| state.as_str() == text.to_ascii_lowercase())
    }

    /// True only for a state that may be retrieved.
    pub fn is_usable(self) -> bool {
        self == Verification::Verified
    }
}

impl std::fmt::Display for Verification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An executable, described, verified procedure.
///
/// Not `Serialize`: a skill persists as Turtle and in the `skills` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    /// The versioned IRI.
    pub iri: NamedNode,
    /// The name a caller searches by.
    pub name: String,
    /// What the skill does, in one sentence.
    pub description: String,
    /// The content-addressed artefact.
    pub impl_ref: String,
    /// The function inside the artefact.
    pub entrypoint: String,
    /// The signature the artefact exposes.
    pub signature: String,
    /// The test that decides verification.
    pub tests_ref: String,
    /// Where the skill stands.
    pub verification: Verification,
    /// When it was last verified.
    pub verified_at: Option<mm_core::Timestamp>,
    /// The embedding row, when one was built.
    pub embedding_ref: Option<String>,
    /// The version.
    pub version: u32,
}

/// The result of running a skill's test spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillTestSpec {
    /// How many cases the spec contains.
    pub cases: u32,
    /// How many passed.
    pub pass: u32,
}

impl SkillTestSpec {
    /// True when the spec reports every case passing, and has at least one case.
    pub fn all_pass(&self) -> bool {
        self.cases > 0 && self.pass == self.cases
    }
}

impl Skill {
    /// Register a skill. It always starts as a [`Verification::Draft`].
    pub fn new(
        slug: &str,
        name: &str,
        description: &str,
        impl_ref: &str,
        entrypoint: &str,
        signature: &str,
        tests_ref: &str,
    ) -> Result<Self> {
        iri::check_slug(slug)?;
        let skill = Skill {
            iri: iri::skill(slug, 1),
            name: name.trim().to_string(),
            description: description.trim().to_string(),
            impl_ref: impl_ref.trim().to_string(),
            entrypoint: entrypoint.trim().to_string(),
            signature: signature.trim().to_string(),
            tests_ref: tests_ref.trim().to_string(),
            verification: Verification::Draft,
            verified_at: None,
            embedding_ref: None,
            version: 1,
        };
        skill.validate()?;
        Ok(skill)
    }

    /// Use another version (and re-derive the IRI).
    pub fn with_version(mut self, version: u32) -> Result<Self> {
        if version == 0 {
            return Err(LibraryError::validation("version", "must be at least 1"));
        }
        let slug = iri::slug_of(self.iri.as_str()).unwrap_or_default();
        self.iri = iri::skill(&slug, version);
        self.version = version;
        Ok(self)
    }

    /// The slug.
    pub fn slug(&self) -> String {
        iri::slug_of(self.iri.as_str()).unwrap_or_default()
    }

    /// Decide verification from the artefact and the test spec.
    ///
    /// This is the whole of `skill verify`: the artefact must match what the skill
    /// declared, and every case in the spec must pass. Both are facts about files,
    /// so the decision is reproducible and needs no model.
    pub fn decide(
        &mut self,
        artefact_hash_matches: bool,
        spec: &SkillTestSpec,
        now: mm_core::Timestamp,
    ) -> Verification {
        self.verification = if artefact_hash_matches && spec.all_pass() {
            self.verified_at = Some(now);
            Verification::Verified
        } else if self.verification == Verification::Verified {
            // A regression downgrades rather than erases: the record of having been
            // verified is what makes the downgrade meaningful.
            Verification::Failing
        } else {
            Verification::Draft
        };
        self.verification
    }

    /// Refuse an unusable skill for a retrieval path.
    pub fn require_verified(&self) -> Result<()> {
        if self.verification.is_usable() {
            Ok(())
        } else {
            Err(LibraryError::SkillNotVerified {
                iri: self.iri.as_str().to_string(),
                state: self.verification.as_str(),
            })
        }
    }
}

impl LibraryEntry for Skill {
    fn kind(&self) -> EntryKind {
        EntryKind::Skill
    }

    fn version_iri(&self) -> NamedNode {
        self.iri.clone()
    }

    fn head_iri(&self) -> NamedNode {
        iri::library(EntryKind::Skill, &self.slug())
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn title(&self) -> &str {
        &self.name
    }

    fn referenced_iris(&self) -> Vec<NamedNode> {
        match NamedNode::new(self.tests_ref.as_str()) {
            Ok(node) => vec![node],
            Err(_) => Vec::new(),
        }
    }

    fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("name", self.name.as_str()),
            ("description", self.description.as_str()),
            ("impl_ref", self.impl_ref.as_str()),
            ("entrypoint", self.entrypoint.as_str()),
            ("signature", self.signature.as_str()),
            ("tests_ref", self.tests_ref.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(LibraryError::validation(field, "must not be empty"));
            }
        }
        if !self.impl_ref.starts_with("data/library/skills/") {
            return Err(LibraryError::validation(
                "impl_ref",
                "a skill artefact lives under data/library/skills/",
            ));
        }
        if self.version == 0 {
            return Err(LibraryError::validation("version", "must be at least 1"));
        }
        Ok(())
    }

    fn to_turtle(&self) -> Result<String> {
        crate::rdf::skill_turtle(self)
    }
}

/// A procedure induced from a trajectory (Agent Workflow Memory).
///
/// Not `Serialize`: a workflow persists as Turtle and in `workflows`.
#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    /// The versioned IRI.
    pub iri: NamedNode,
    /// The name.
    pub name: String,
    /// The steps, in order.
    pub steps: Vec<String>,
    /// The trajectory it was induced from.
    pub induced_from: String,
    /// The version.
    pub version: u32,
}

impl Workflow {
    /// Induce a workflow from a trajectory's observed steps.
    pub fn induce(slug: &str, name: &str, steps: &[String], induced_from: &str) -> Result<Self> {
        iri::check_slug(slug)?;
        let workflow = Workflow {
            iri: iri::versioned(EntryKind::Workflow, slug, 1),
            name: name.trim().to_string(),
            steps: steps.iter().map(|s| s.trim().to_string()).collect(),
            induced_from: induced_from.trim().to_string(),
            version: 1,
        };
        workflow.validate()?;
        Ok(workflow)
    }

    /// The slug.
    pub fn slug(&self) -> String {
        iri::slug_of(self.iri.as_str()).unwrap_or_default()
    }
}

impl LibraryEntry for Workflow {
    fn kind(&self) -> EntryKind {
        EntryKind::Workflow
    }

    fn version_iri(&self) -> NamedNode {
        self.iri.clone()
    }

    fn head_iri(&self) -> NamedNode {
        iri::library(EntryKind::Workflow, &self.slug())
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn title(&self) -> &str {
        &self.name
    }

    fn referenced_iris(&self) -> Vec<NamedNode> {
        Vec::new()
    }

    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(LibraryError::validation("name", "must not be empty"));
        }
        if self.steps.is_empty() {
            return Err(LibraryError::validation(
                "steps",
                "a workflow has at least one step",
            ));
        }
        if self.steps.iter().any(|step| step.trim().is_empty()) {
            return Err(LibraryError::validation("steps", "no step may be blank"));
        }
        if self.induced_from.trim().is_empty() {
            return Err(LibraryError::validation(
                "induced_from",
                "a workflow names the trajectory it came from",
            ));
        }
        Ok(())
    }

    fn to_turtle(&self) -> Result<String> {
        crate::rdf::workflow_turtle(self)
    }
}

/// One retrieval hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillHit {
    /// The skill's IRI.
    pub iri: String,
    /// Its name.
    pub name: String,
    /// Its verification state (always `Verified` for a hit).
    pub verification: Verification,
    /// The lexical score, in `[0,1]`.
    pub score: f32,
}

/// The lexical score of a skill against a query: the fraction of the query's
/// tokens that appear in the skill's name or description.
///
/// Deliberately lexical and deterministic — the plan's "embedding retrieval" is a
/// Phase 10 concern (no embedder exists yet), and a hint that ranks rather than
/// decides is exactly what the library needs here.
pub fn lexical_score(skill: &Skill, query: &str) -> f32 {
    let haystack = format!("{} {}", skill.name, skill.description).to_ascii_lowercase();
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let hits = tokens
        .iter()
        .filter(|token| haystack.contains(token.as_str()))
        .count();
    hits as f32 / tokens.len() as f32
}

/// Retrieve the best verified skills for a query.
///
/// Only [`Verification::Verified`] skills are considered: a draft is invisible to
/// retrieval, which is what makes the verification state load-bearing rather than
/// decorative. Ties break on the IRI, so the same query returns the same order.
pub fn retrieve(skills: &[Skill], query: &str, top: usize) -> Vec<SkillHit> {
    let mut hits: Vec<SkillHit> = skills
        .iter()
        .filter(|skill| skill.verification.is_usable())
        .map(|skill| SkillHit {
            iri: skill.iri.as_str().to_string(),
            name: skill.name.clone(),
            verification: skill.verification,
            score: lexical_score(skill, query),
        })
        .filter(|hit| hit.score > 0.0)
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.iri.cmp(&b.iri))
    });
    hits.truncate(top);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, description: &str) -> Skill {
        Skill::new(
            "api-capability-check",
            name,
            description,
            "data/library/skills/api-capability-check.wasm",
            "check",
            "fn(check: Capability) -> Verdict",
            "https://metamind.dev/library/evaluation/regression-suite",
        )
        .unwrap()
    }

    #[test]
    fn a_new_skill_is_a_draft_and_is_not_retrievable() {
        let skill = skill("API capability check", "verify an external api before use");
        assert_eq!(skill.verification, Verification::Draft);
        assert!(retrieve(std::slice::from_ref(&skill), "verify external api", 3).is_empty());
    }

    #[test]
    fn verification_needs_a_matching_artefact_and_a_passing_spec() {
        let now = mm_core::Timestamp::now();
        let mut draft = skill("API capability check", "verify an external api before use");
        draft.decide(true, &SkillTestSpec { cases: 4, pass: 3 }, now);
        assert_eq!(
            draft.verification,
            Verification::Draft,
            "3 of 4 is not a pass"
        );

        draft.decide(true, &SkillTestSpec { cases: 4, pass: 4 }, now);
        assert_eq!(draft.verification, Verification::Verified);
        assert!(draft.verified_at.is_some());
        assert_eq!(
            retrieve(std::slice::from_ref(&draft), "verify external api", 3).len(),
            1
        );

        // A regression after verification downgrades to failing, not to draft.
        draft.decide(true, &SkillTestSpec { cases: 4, pass: 1 }, now);
        assert_eq!(draft.verification, Verification::Failing);
        assert!(draft.require_verified().is_err());
    }

    #[test]
    fn a_mismatched_artefact_never_verifies() {
        let mut skill = skill("x", "y");
        assert_eq!(
            skill.decide(
                false,
                &SkillTestSpec { cases: 1, pass: 1 },
                mm_core::Timestamp::now()
            ),
            Verification::Draft
        );
    }

    #[test]
    fn retrieval_ranks_by_lexical_overlap_and_breaks_ties_by_iri() {
        let mut a = Skill::new(
            "api-capability-check",
            "API capability check",
            "confirm an external api behaves as documented",
            "data/library/skills/api-capability-check.wasm",
            "check",
            "fn() -> Verdict",
            "https://metamind.dev/library/evaluation/regression-suite",
        )
        .unwrap();
        let mut b = Skill::new(
            "retry-budget-check",
            "Retry budget check",
            "cap the number of retries before use",
            "data/library/skills/retry-budget-check.wasm",
            "check",
            "fn() -> Verdict",
            "https://metamind.dev/library/evaluation/regression-suite",
        )
        .unwrap();
        let now = mm_core::Timestamp::now();
        a.decide(true, &SkillTestSpec { cases: 1, pass: 1 }, now);
        b.decide(true, &SkillTestSpec { cases: 1, pass: 1 }, now);
        let hits = retrieve(&[a, b], "verify external api before use", 3);
        assert_eq!(hits.len(), 2);
        // "api" appears in the first only, so it scores higher.
        assert!(hits[0].iri.contains("api-capability-check"));
        assert!(hits
            .iter()
            .all(|hit| hit.verification == Verification::Verified));
    }

    #[test]
    fn a_skill_artefact_must_live_under_the_skills_directory() {
        assert!(Skill::new(
            "x",
            "X",
            "y",
            "/tmp/evil.wasm",
            "check",
            "fn()",
            "https://metamind.dev/library/evaluation/regression-suite",
        )
        .is_err());
    }

    #[test]
    fn a_workflow_records_its_steps_and_trajectory() {
        let steps = ["reproduce", "isolate", "fix", "verify"].map(String::from);
        let workflow = Workflow::induce("w", "Reproduce then fix", &steps, "trajectory-1").unwrap();
        assert_eq!(workflow.steps.len(), 4);
        assert!(Workflow::induce("w", "W", &[], "t").is_err());
    }
}
