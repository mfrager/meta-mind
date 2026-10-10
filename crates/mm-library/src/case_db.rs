//! Cases and analogy.
//!
//! Case-based reasoning's loop is retrieve → adapt → revise → retain; this module
//! owns the *retrieve* half and the representation it needs. A case is a problem,
//! a solution and an outcome, each a small Turtle document, plus the three
//! qualities that make it rankable — outcome quality, transferability and evidence
//! quality — so `case_utility` has real arithmetic rather than a number a caller
//! supplies.
//!
//! Analogy is **structural, not surface** (ANASIME's point): two problems match
//! when they use the same *relations*, whatever their subjects are called.
//! [`structural_similarity`] therefore compares predicate sets and maps the
//! corresponding elements, and it says so by returning the [`Correspondence`]s
//! rather than a bare number — an analogy nobody can inspect is a guess.

use std::collections::BTreeSet;

use mm_core::NamedNode;
use serde::{Deserialize, Serialize};

use crate::entry::{iri, EntryKind, LibraryEntry};
use crate::error::{LibraryError, Result};

/// The largest number of correspondences a single mapping may report.
pub const MAX_CORRESPONDENCES: usize = 16;

/// What sort of case this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseKind {
    /// It worked.
    Success,
    /// It did not.
    Failure,
    /// It worked, surprisingly.
    Unusual,
    /// It turned on a boundary condition.
    Edge,
}

/// Every case kind.
pub const CASE_KINDS: [CaseKind; 4] = [
    CaseKind::Success,
    CaseKind::Failure,
    CaseKind::Unusual,
    CaseKind::Edge,
];

impl CaseKind {
    /// The wire name, matching the `cases.kind` CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            CaseKind::Success => "success",
            CaseKind::Failure => "failure",
            CaseKind::Unusual => "unusual",
            CaseKind::Edge => "edge",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        CASE_KINDS
            .into_iter()
            .find(|kind| kind.as_str() == text.to_ascii_lowercase())
    }
}

impl std::fmt::Display for CaseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One element of a case: a node taking a role in a relation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CaseElement {
    /// The node, as rendered.
    pub iri: String,
    /// The role it plays (subject or object of the relation).
    pub role: String,
}

impl CaseElement {
    /// An element with a role.
    pub fn new(iri: impl Into<String>, role: &str) -> Self {
        CaseElement {
            iri: iri.into(),
            role: role.to_string(),
        }
    }
}

/// One structure-mapped correspondence between two cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Correspondence {
    /// The element in the query case.
    pub source: CaseElement,
    /// The element in the retrieved case.
    pub target: CaseElement,
    /// The relation they share.
    pub relation: String,
    /// How well they correspond, in `[0,1]`.
    pub score: f32,
}

/// A remembered instance: problem, solution, outcome.
///
/// Not `Serialize`: a case persists as Turtle (see [`crate::rdf`]), and
/// `oxrdf::NamedNode` is not a serde type.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    /// The versioned IRI.
    pub iri: NamedNode,
    /// What sort of case it is.
    pub kind: CaseKind,
    /// The problem, as Turtle.
    pub problem_ttl: String,
    /// The solution, as Turtle.
    pub solution_ttl: String,
    /// The outcome, as Turtle.
    pub outcome_ttl: String,
    /// How good the outcome was, in `[0,1]`.
    pub outcome_quality: f32,
    /// How far the case reaches, in `[0,1]`.
    pub transferability: f32,
    /// How well evidenced it is, in `[0,1]`.
    pub evidence_quality: f32,
    /// The version.
    pub version: u32,
}

impl Case {
    /// Build a case, refusing qualities outside `[0,1]` and empty documents.
    pub fn new(
        slug: &str,
        kind: CaseKind,
        problem_ttl: &str,
        solution_ttl: &str,
        outcome_ttl: &str,
        outcome_quality: f32,
        transferability: f32,
        evidence_quality: f32,
    ) -> Result<Self> {
        iri::check_slug(slug)?;
        let case = Case {
            iri: iri::versioned(EntryKind::Case, slug, 1),
            kind,
            problem_ttl: problem_ttl.trim().to_string(),
            solution_ttl: solution_ttl.trim().to_string(),
            outcome_ttl: outcome_ttl.trim().to_string(),
            outcome_quality,
            transferability,
            evidence_quality,
            version: 1,
        };
        case.validate()?;
        Ok(case)
    }

    /// The slug.
    pub fn slug(&self) -> String {
        iri::slug_of(self.iri.as_str()).unwrap_or_default()
    }
}

impl LibraryEntry for Case {
    fn kind(&self) -> EntryKind {
        EntryKind::Case
    }

    fn version_iri(&self) -> NamedNode {
        self.iri.clone()
    }

    fn head_iri(&self) -> NamedNode {
        iri::library(EntryKind::Case, &self.slug())
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn title(&self) -> &str {
        &self.problem_ttl
    }

    fn referenced_iris(&self) -> Vec<NamedNode> {
        Vec::new()
    }

    fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("problem_ttl", self.problem_ttl.as_str()),
            ("solution_ttl", self.solution_ttl.as_str()),
            ("outcome_ttl", self.outcome_ttl.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(LibraryError::validation(field, "must not be empty"));
            }
        }
        for (field, value) in [
            ("outcome_quality", self.outcome_quality),
            ("transferability", self.transferability),
            ("evidence_quality", self.evidence_quality),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(LibraryError::validation(
                    field,
                    format!("must be in [0,1], got {value}"),
                ));
            }
        }
        Ok(())
    }

    fn to_turtle(&self) -> Result<String> {
        crate::rdf::case_turtle(self)
    }
}

/// The relations a Turtle document uses, with the elements that take part in each.
///
/// One pass over the parsed triples: the relation is the predicate, and the
/// subject and object are recorded with their roles. Nothing here looks at
/// *names*, which is what makes the mapping structural.
fn relations(turtle: &str) -> Result<Vec<(String, CaseElement, CaseElement)>> {
    let graph = rdf_codec::io::parse_turtle(turtle)
        .map_err(|e| LibraryError::Codec(format!("case document: {e}")))?;
    let mut out = Vec::new();
    for triple in graph.iter() {
        let relation = triple.predicate.as_str().to_string();
        out.push((
            relation,
            CaseElement::new(triple.subject.to_string(), "subject"),
            CaseElement::new(triple.object.to_string(), "object"),
        ));
    }
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    Ok(out)
}

/// The structure-mapped correspondence between two cases' problems.
///
/// Two problems correspond when they use the same relation: the *relation* is the
/// shared structure, and the elements on either side are what the mapping lines up.
/// The score of a mapping is the fraction of the union of relations that both use,
/// so a mapping that explains everything scores 1 and one that shares a single
/// relation of five scores 0.2.
pub fn structural_similarity(a: &Case, b: &Case) -> Result<Vec<Correspondence>> {
    let left = relations(&a.problem_ttl)?;
    let right = relations(&b.problem_ttl)?;
    let left_relations: BTreeSet<&str> = left.iter().map(|(r, _, _)| r.as_str()).collect();
    let right_relations: BTreeSet<&str> = right.iter().map(|(r, _, _)| r.as_str()).collect();
    let shared: Vec<&&str> = left_relations.intersection(&right_relations).collect();
    let union = left_relations.union(&right_relations).count().max(1);

    let mut out = Vec::new();
    for relation in shared {
        let source = left
            .iter()
            .find(|(r, _, _)| r == *relation)
            .map(|(_, s, _)| s.clone());
        let target = right
            .iter()
            .find(|(r, _, _)| r == *relation)
            .map(|(_, s, _)| s.clone());
        if let (Some(source), Some(target)) = (source, target) {
            out.push(Correspondence {
                source,
                target,
                relation: (*relation).to_string(),
                score: 1.0 / union as f32,
            });
        }
    }
    out.truncate(MAX_CORRESPONDENCES);
    Ok(out)
}

/// The structural score a correspondence list implies, in `[0,1]`.
pub fn correspondence_score(correspondences: &[Correspondence], union_size: usize) -> f32 {
    let union = union_size.max(1) as f32;
    correspondences
        .iter()
        .map(|c| c.score)
        .sum::<f32>()
        .min(union)
        / union
}

/// How useful a case is as a precedent.
///
/// `Similarity × OutcomeQuality × Transferability × EvidenceQuality`. A
/// multiplicative form is deliberate: a case that resembles the problem but did
/// not work is worth nothing, and one that worked but cannot transfer is worth
/// nothing either, so no single strong factor can carry a weak one.
pub fn case_utility(case: &Case, similarity: f32) -> f32 {
    let similarity = similarity.clamp(0.0, 1.0);
    (similarity * case.outcome_quality * case.transferability * case.evidence_quality)
        .clamp(0.0, 1.0)
}

/// A retrieval query: a problem to match, and how many cases to return.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseQuery {
    /// The problem, as Turtle.
    pub problem_ttl: String,
    /// How many cases to return.
    pub top: usize,
}

/// One retrieval hit: the case, its mapping, and its utility.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseHit {
    /// The case's IRI.
    pub iri: String,
    /// Its kind.
    pub kind: CaseKind,
    /// Its utility.
    pub utility: f32,
    /// The structure mapping that produced the similarity.
    pub correspondences: Vec<Correspondence>,
}

/// Retrieve the most useful cases for a problem.
///
/// Ranked by `case_utility`, ties broken on the IRI. A case with no shared
/// relation is not returned at all: an analogy of zero structure is not an
/// analogy, and returning the whole library ranked by nothing would bury the
/// precedents that matter.
pub fn retrieve(cases: &[Case], query: &CaseQuery) -> Result<Vec<CaseHit>> {
    let probe = Case {
        iri: iri::versioned(EntryKind::Case, "query", 1),
        kind: CaseKind::Edge,
        problem_ttl: query.problem_ttl.clone(),
        solution_ttl: ".".to_string(),
        outcome_ttl: ".".to_string(),
        outcome_quality: 1.0,
        transferability: 1.0,
        evidence_quality: 1.0,
        version: 1,
    };
    let probe_relations = relations(&probe.problem_ttl)?.len();
    let mut hits: Vec<CaseHit> = Vec::new();
    for case in cases {
        let correspondences = structural_similarity(&probe, case)?;
        if correspondences.is_empty() {
            continue;
        }
        let case_relations = relations(&case.problem_ttl)?.len();
        let union = probe_relations.max(case_relations);
        let similarity = correspondence_score(&correspondences, union);
        hits.push(CaseHit {
            iri: case.iri.as_str().to_string(),
            kind: case.kind,
            utility: case_utility(case, similarity),
            correspondences,
        });
    }
    hits.sort_by(|a, b| {
        b.utility
            .partial_cmp(&a.utility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.iri.cmp(&b.iri))
    });
    hits.truncate(query.top);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(problem: &str) -> Case {
        Case::new("q", CaseKind::Edge, problem, ".", ".", 1.0, 1.0, 1.0).unwrap()
    }

    const DEPLOY: &str = r#"
@prefix ex: <https://example.dev/> .
@prefix mm: <https://metamind.dev/ontology#> .
ex:service mm:regressedAfter ex:deploy ;
           mm:exhibits ex:latency .
"#;

    #[test]
    fn a_case_needs_its_three_documents_and_qualities() {
        assert!(Case::new("c", CaseKind::Success, "", "s", "o", 1.0, 1.0, 1.0).is_err());
        assert!(Case::new("c", CaseKind::Success, "p", "s", "o", 1.2, 1.0, 1.0).is_err());
    }

    #[test]
    fn structure_maps_relations_not_names() {
        // The same shape with different names: two relations in common.
        let same_shape = probe(
            r#"
@prefix mm: <https://metamind.dev/ontology#> .
<https://other.dev/a> mm:regressedAfter <https://other.dev/b> ;
                      mm:exhibits <https://other.dev/c> .
"#,
        );
        let mut case = probe(DEPLOY);
        case.iri = iri::versioned(EntryKind::Case, "deploy", 1);
        let mapping = structural_similarity(&same_shape, &case).unwrap();
        assert_eq!(mapping.len(), 2, "both relations correspond");
        assert!(mapping
            .iter()
            .any(|c| c.relation.ends_with("regressedAfter")));

        // A different shape shares nothing.
        let different = probe(
            r#"
@prefix mm: <https://metamind.dev/ontology#> .
<https://other.dev/a> mm:unrelated <https://other.dev/b> .
"#,
        );
        assert!(structural_similarity(&different, &case).unwrap().is_empty());
    }

    #[test]
    fn utility_is_multiplicative_so_one_weak_factor_sinks_it() {
        let strong = Case::new("a", CaseKind::Success, DEPLOY, ".", ".", 1.0, 1.0, 1.0).unwrap();
        assert!((case_utility(&strong, 0.5) - 0.5).abs() < 1e-6);
        let bad_outcome =
            Case::new("b", CaseKind::Failure, DEPLOY, ".", ".", 0.0, 1.0, 1.0).unwrap();
        assert_eq!(case_utility(&bad_outcome, 1.0), 0.0);
        let untransferable =
            Case::new("c", CaseKind::Success, DEPLOY, ".", ".", 1.0, 0.0, 1.0).unwrap();
        assert_eq!(case_utility(&untransferable, 1.0), 0.0);
    }

    #[test]
    fn retrieval_orders_by_utility_and_drops_a_case_with_no_shared_relation() {
        let mut deploy = Case::new(
            "deploy",
            CaseKind::Success,
            DEPLOY,
            ".",
            ".",
            0.9,
            0.8,
            0.85,
        )
        .unwrap();
        deploy.iri = iri::versioned(EntryKind::Case, "deploy", 1);
        let unrelated = Case::new(
            "other",
            CaseKind::Failure,
            r#"@prefix mm: <https://metamind.dev/ontology#> . <https://x/y> mm:nothing <https://x/z> ."#,
            ".",
            ".",
            0.9,
            0.9,
            0.9,
        )
        .unwrap();
        let hits = retrieve(
            &[deploy, unrelated],
            &CaseQuery {
                problem_ttl: DEPLOY.to_string(),
                top: 5,
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].iri.contains("deploy"));
        assert!(!hits[0].correspondences.is_empty());
    }
}
