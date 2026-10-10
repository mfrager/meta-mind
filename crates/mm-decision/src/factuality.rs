//! Claim-level factuality: how much of a claim the evidence actually supports.
//!
//! A claim is not one fact. "The latency doubled because the cache was cold and
//! the replica lagged" is three assertions, and evidence that supports the first
//! supports nothing about the third. Scoring the sentence as a whole is therefore
//! the wrong measurement — it lets a supported clause carry an unsupported one —
//! so the unit of support here is the **atomic fact**, following FActScore, and the
//! headline number is [`atomic_precision`]: the fraction of atoms the evidence
//! supports.
//!
//! Three checks, and the plan's honesty requirement is what shapes them:
//!
//! * [`FactualityCheck::AtomicPrecision`] — decompose, then judge each atom
//!   against the evidence at hand. Deterministic and available now.
//! * [`FactualityCheck::SelfConsistency`] — the fraction of samples that agree with
//!   the claim. A weak signal, but a real one: a claim that comes back differently
//!   every time is not well supported by the model that made it.
//! * [`FactualityCheck::SearchAugmented`] — decompose, retrieve evidence per atom,
//!   judge. Phase 10 owns retrieval, so here the check is the **seam**:
//!   [`FactualityTool`] is the interface and [`check_factuality`] refuses with
//!   `Unavailable` rather than inventing a support number. A fabricated factuality
//!   score is the single most damaging thing this module could produce, because the
//!   firewall's `VERIFY_FIRST` is *driven* by it.
//!
//! Two judgement calls are encoded rather than documented away. First, no atoms
//! means precision `0.0`, not `1.0`: nothing was verified, and reading an
//! unverified claim as fully supported inverts the meaning of the number. Second,
//! [`LexicalSupportJudger`] is a deliberately weak, token-overlap stand-in for a
//! model-backed judge — it is reproducible and it is conservative about short
//! atoms, and the stronger judge plugs in behind the same trait when a caller has
//! one.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{DecisionError, Result};
use crate::uncertainty::normalize_text;

/// Which factuality check produced a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactualityCheck {
    /// Agreement between independent samples of the same claim.
    SelfConsistency,
    /// The fraction of atomic facts the evidence supports.
    AtomicPrecision,
    /// Decompose, retrieve per atom, judge — needs the Phase 10 tool seam.
    SearchAugmented,
}

/// Every check, in wire order.
pub const FACTUALITY_CHECKS: [FactualityCheck; 3] = [
    FactualityCheck::SelfConsistency,
    FactualityCheck::AtomicPrecision,
    FactualityCheck::SearchAugmented,
];

impl FactualityCheck {
    /// The stable wire name, which is also the `factuality_checks.check_kind`
    /// value.
    pub fn as_str(self) -> &'static str {
        match self {
            FactualityCheck::SelfConsistency => "self_consistency",
            FactualityCheck::AtomicPrecision => "atomic_precision",
            FactualityCheck::SearchAugmented => "search_augmented",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        FACTUALITY_CHECKS
            .into_iter()
            .find(|check| check.as_str() == text)
    }
}

impl std::fmt::Display for FactualityCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One assertion decomposed out of a claim, and whether the evidence supported it.
///
/// `supported` is an `Option` rather than a `bool` because "not yet judged" and
/// "judged and unsupported" are different states and only one of them is a finding.
/// Collapsing them would make a claim that was never checked indistinguishable
/// from one that was checked and failed, which is precisely the distinction an
/// operator needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AtomicFact {
    /// The assertion's text.
    pub text: String,
    /// Whether the evidence supported it, once a judge has run.
    pub supported: Option<bool>,
}

/// What a factuality check concluded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactualityReport {
    /// Which check ran.
    pub check: FactualityCheck,
    /// The support fraction, in `[0,1]`.
    pub support: f32,
    /// The atoms the claim was decomposed into.
    pub atoms: Vec<AtomicFact>,
    /// How many samples or evidence items were consulted.
    pub samples: usize,
    /// A stable rendering, so two runs compare as strings.
    pub canonical: String,
}

impl FactualityReport {
    /// A report, with the support clamped into `[0,1]`.
    ///
    /// Clamping is safe in one direction only, and that is the direction taken: a
    /// non-finite or out-of-range support is read as `0.0`, which pushes the
    /// firewall *toward* verification. An inflated support would push it toward
    /// action, so it is never the reading given to a broken number.
    pub fn new(
        check: FactualityCheck,
        support: f32,
        atoms: Vec<AtomicFact>,
        samples: usize,
    ) -> Self {
        let support = if support.is_finite() {
            support.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rendered = atoms
            .iter()
            .map(|atom| {
                let state = match atom.supported {
                    Some(true) => "supported",
                    Some(false) => "unsupported",
                    None => "unjudged",
                };
                format!("{}|{state}", atom.text)
            })
            .collect::<Vec<_>>()
            .join(";");
        let canonical = format!(
            "check={},support={support:.9},samples={samples},atoms=[{rendered}]",
            check.as_str()
        );
        FactualityReport {
            check,
            support,
            atoms,
            samples,
            canonical,
        }
    }

    /// The atoms that are not supported: judged false, or never judged at all.
    ///
    /// Unjudged atoms count as unsupported because the caller asking this question
    /// is deciding whether to act, and an unverified assertion is not a reason to.
    pub fn unsupported(&self) -> Vec<&AtomicFact> {
        self.atoms
            .iter()
            .filter(|atom| atom.supported != Some(true))
            .collect()
    }
}

/// Decompose a claim into the assertions it asserts.
pub trait AtomExtractor: Send + Sync {
    /// The claim's atomic facts, with `supported` left unset.
    fn atoms(&self, claim: &str) -> Result<Vec<AtomicFact>>;
}

/// Decide whether evidence supports one assertion.
pub trait SupportJudger: Send + Sync {
    /// True when `evidence` supports `atom`.
    fn supports(&self, atom: &str, evidence: &[String]) -> Result<bool>;
}

/// The retrieval half of search-augmented factuality.
///
/// Phase 10 owns tools, so this phase defines the interface and nothing that
/// implements it in production. Defining it here rather than in the firewall keeps
/// the seam where the missing capability is, so the refusal in
/// [`check_factuality`] is a statement about retrieval rather than about
/// factuality.
pub trait FactualityTool: Send + Sync {
    /// Retrieve evidence for a query.
    fn search(&self, query: &str) -> Result<Vec<String>>;
}

/// Decompose a claim by punctuation and conjunctions: a lexical splitter.
///
/// Splitting on `and`/`but`/`plus` over-splits phrases like "terms and conditions"
/// where the conjunction joins nouns rather than clauses. That is accepted: an
/// extra atom costs one more judgement, whereas a missed atom lets a supported
/// clause carry an unsupported one — and the asymmetry of those costs is the whole
/// reason this module decomposes at all.
#[derive(Clone, Copy, Debug, Default)]
pub struct LexicalAtomExtractor;

/// Conjunctions that begin a new assertion.
const CONNECTORS: [&str; 3] = ["and", "but", "plus"];

impl AtomExtractor for LexicalAtomExtractor {
    fn atoms(&self, claim: &str) -> Result<Vec<AtomicFact>> {
        let mut facts: Vec<AtomicFact> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for fragment in claim.split(['.', ';', ',', '\n']) {
            for piece in split_conjunctions(fragment) {
                let text = piece.trim();
                if text.is_empty() {
                    continue;
                }
                let key = normalize_text(text);
                if key.is_empty() || !seen.insert(key) {
                    continue;
                }
                facts.push(AtomicFact {
                    text: text.to_string(),
                    supported: None,
                });
            }
        }
        Ok(facts)
    }
}

/// Split one fragment at its conjunctions, dropping the connectors themselves.
fn split_conjunctions(fragment: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for token in fragment.split_whitespace() {
        let stripped = token
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if CONNECTORS.contains(&stripped.as_str()) {
            if !current.is_empty() {
                pieces.push(current.join(" "));
                current.clear();
            }
            continue;
        }
        current.push(token);
    }
    if !current.is_empty() {
        pieces.push(current.join(" "));
    }
    pieces
}

/// Support by token overlap: the atom is supported when at least `min_overlap` of
/// its content tokens appear in the evidence.
///
/// The weak, deterministic stand-in described in the module doc. Its failure mode
/// is the safe one: a short atom with two tokens needs both of them, so it reports
/// "unsupported" for a claim the evidence paraphrases rather than naming — which
/// escalates to verification instead of to action.
#[derive(Clone, Copy, Debug)]
pub struct LexicalSupportJudger {
    /// The fraction of the atom's content tokens the evidence must cover.
    pub min_overlap: f32,
}

impl LexicalSupportJudger {
    /// A judger with the given threshold, clamped into `[0,1]`. A non-finite
    /// threshold becomes `1.0`: full coverage, the most demanding reading.
    pub fn new(min_overlap: f32) -> Self {
        let min_overlap = if min_overlap.is_finite() {
            min_overlap.clamp(0.0, 1.0)
        } else {
            1.0
        };
        LexicalSupportJudger { min_overlap }
    }
}

impl Default for LexicalSupportJudger {
    fn default() -> Self {
        LexicalSupportJudger::new(0.6)
    }
}

impl SupportJudger for LexicalSupportJudger {
    fn supports(&self, atom: &str, evidence: &[String]) -> Result<bool> {
        let atom_tokens = content_tokens(atom);
        if atom_tokens.is_empty() {
            // Nothing to check: an assertion made only of punctuation is not
            // supported by anything.
            return Ok(false);
        }
        let mut evidence_tokens: BTreeSet<String> = BTreeSet::new();
        for item in evidence {
            evidence_tokens.extend(content_tokens(item));
        }
        let covered = atom_tokens.intersection(&evidence_tokens).count();
        let fraction = covered as f32 / atom_tokens.len() as f32;
        Ok(fraction >= self.min_overlap)
    }
}

/// The fraction of atoms the evidence supported.
///
/// `0.0` for an empty atom list. A claim that decomposed into nothing has had
/// nothing verified, and reporting `1.0` would let an unjudged claim through the
/// firewall at maximum confidence.
pub fn atomic_precision(atoms: &[AtomicFact]) -> f32 {
    if atoms.is_empty() {
        return 0.0;
    }
    let supported = atoms
        .iter()
        .filter(|atom| atom.supported == Some(true))
        .count();
    supported as f32 / atoms.len() as f32
}

/// The fraction of samples that agree with the claim.
///
/// `0.0` when there are no samples: no independent repetitions means no measured
/// agreement, which is a weaker position than a low but measured one and must not
/// be rounded up to certainty.
pub fn self_consistency_support(claim: &str, samples: &[String]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let target = normalize_text(claim);
    let agreeing = samples
        .iter()
        .filter(|sample| normalize_text(sample) == target)
        .count();
    agreeing as f32 / samples.len() as f32
}

/// Decompose, retrieve per atom, judge — the search-augmented check.
#[derive(Clone, Debug)]
pub struct SearchAugmentedChecker<T: FactualityTool> {
    /// The retrieval seam. In production this is a Phase 10 tool.
    pub tool: T,
    /// How atoms are decomposed.
    pub extractor: LexicalAtomExtractor,
    /// How each atom is judged against what was retrieved.
    pub judger: LexicalSupportJudger,
}

impl<T: FactualityTool> SearchAugmentedChecker<T> {
    /// A checker over the given retrieval seam, with the default splitter and
    /// judge.
    pub fn new(tool: T) -> Self {
        SearchAugmentedChecker {
            tool,
            extractor: LexicalAtomExtractor,
            judger: LexicalSupportJudger::default(),
        }
    }

    /// Run the check: each atom is searched for and judged against what came back.
    ///
    /// A retrieval refusal propagates rather than degrading to "unsupported": a
    /// search that failed is a missing input, not a finding about the claim, and
    /// the two must not be reported as the same thing.
    pub fn check(&self, claim: &str) -> Result<FactualityReport> {
        let mut atoms = self.extractor.atoms(claim)?;
        for atom in &mut atoms {
            let evidence = self.tool.search(&atom.text)?;
            atom.supported = Some(self.judger.supports(&atom.text, &evidence)?);
        }
        let support = atomic_precision(&atoms);
        let samples = atoms.len();
        Ok(FactualityReport::new(
            FactualityCheck::SearchAugmented,
            support,
            atoms,
            samples,
        ))
    }
}

/// Run one factuality check over evidence the caller already has.
///
/// [`FactualityCheck::SearchAugmented`] refuses here: retrieval belongs to Phase 10
/// and there is no tool in this signature. Returning a number anyway — even a
/// conservative one — would make the missing capability invisible in exactly the
/// place the firewall consults, so the refusal is the honest answer.
pub fn check_factuality(
    claim: &str,
    evidence: &[String],
    check: FactualityCheck,
) -> Result<FactualityReport> {
    let extractor = LexicalAtomExtractor;
    let judger = LexicalSupportJudger::default();
    match check {
        FactualityCheck::AtomicPrecision => {
            let mut atoms = extractor.atoms(claim)?;
            for atom in &mut atoms {
                atom.supported = Some(judger.supports(&atom.text, evidence)?);
            }
            let support = atomic_precision(&atoms);
            Ok(FactualityReport::new(check, support, atoms, evidence.len()))
        }
        FactualityCheck::SelfConsistency => {
            let atoms = extractor.atoms(claim)?;
            let support = self_consistency_support(claim, evidence);
            Ok(FactualityReport::new(check, support, atoms, evidence.len()))
        }
        FactualityCheck::SearchAugmented => Err(DecisionError::Unavailable(
            "search-augmented factuality needs the Phase 10 tool seam".to_string(),
        )),
    }
}

/// The content tokens of a text: alphanumeric runs, lowercased.
fn content_tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn checks_round_trip_and_reject_unknown_names() {
        for check in FACTUALITY_CHECKS {
            assert_eq!(FactualityCheck::parse(check.as_str()), Some(check));
            assert_eq!(
                FactualityCheck::parse(&check.as_str().to_uppercase()),
                Some(check)
            );
        }
        assert_eq!(FactualityCheck::parse("vibes"), None);
    }

    #[test]
    fn the_extractor_splits_clauses_and_drops_duplicates() {
        let extractor = LexicalAtomExtractor;
        let atoms = extractor
            .atoms("The latency doubled, and the cache was cold. The cache was cold.")
            .unwrap();
        let texts: Vec<&str> = atoms.iter().map(|atom| atom.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["The latency doubled", "the cache was cold"],
            "{atoms:?}"
        );
        assert!(atoms.iter().all(|atom| atom.supported.is_none()));

        // A claim of nothing decomposes into nothing rather than into one atom.
        assert!(extractor.atoms("   ").unwrap().is_empty());
        assert!(extractor.atoms("...").unwrap().is_empty());
    }

    #[test]
    fn atomic_precision_counts_supported_atoms_only() {
        let atoms = vec![
            AtomicFact {
                text: "a".into(),
                supported: Some(true),
            },
            AtomicFact {
                text: "b".into(),
                supported: Some(false),
            },
            AtomicFact {
                text: "c".into(),
                supported: None,
            },
        ];
        let precision = atomic_precision(&atoms);
        assert!((precision - 1.0 / 3.0).abs() < 1e-6, "{precision}");
        assert_eq!(atomic_precision(&[]), 0.0);
    }

    #[test]
    fn a_report_reads_a_broken_support_number_as_no_support() {
        let report = FactualityReport::new(FactualityCheck::AtomicPrecision, f32::NAN, vec![], 0);
        assert_eq!(report.support, 0.0);
        let report = FactualityReport::new(FactualityCheck::AtomicPrecision, 5.0, vec![], 0);
        assert_eq!(report.support, 1.0);
        let report = FactualityReport::new(FactualityCheck::AtomicPrecision, -2.0, vec![], 0);
        assert_eq!(report.support, 0.0);
    }

    #[test]
    fn unjudged_atoms_count_as_unsupported() {
        let report = FactualityReport::new(
            FactualityCheck::AtomicPrecision,
            0.5,
            vec![
                AtomicFact {
                    text: "supported".into(),
                    supported: Some(true),
                },
                AtomicFact {
                    text: "unjudged".into(),
                    supported: None,
                },
            ],
            0,
        );
        let unsupported: Vec<&str> = report
            .unsupported()
            .iter()
            .map(|atom| atom.text.as_str())
            .collect();
        assert_eq!(unsupported, vec!["unjudged"]);
    }

    #[test]
    fn self_consistency_support_is_a_fraction_of_agreeing_samples() {
        assert_eq!(
            self_consistency_support(
                "the server is up",
                &evidence(&["the server is up", "The server is up!"])
            ),
            1.0
        );
        assert_eq!(
            self_consistency_support("the server is up", &evidence(&["the server is down"])),
            0.0
        );
        let mixed = self_consistency_support(
            "the server is up",
            &evidence(&[
                "the server is up",
                "the server is down",
                "the server is down",
            ]),
        );
        assert!((mixed - 1.0 / 3.0).abs() < 1e-6, "{mixed}");
        assert_eq!(self_consistency_support("the server is up", &[]), 0.0);
    }

    #[test]
    fn the_lexical_judger_needs_the_atoms_tokens_in_the_evidence() {
        let judger = LexicalSupportJudger::default();
        assert!(judger
            .supports(
                "the cache was cold",
                &evidence(&["the cache was cold, so the latency doubled"])
            )
            .unwrap());
        assert!(!judger
            .supports(
                "the cache was cold",
                &evidence(&["the network was saturated"])
            )
            .unwrap());
        // An atom with no content tokens is not supported by anything.
        assert!(!judger.supports("...", &evidence(&["anything"])).unwrap());
        // The threshold is clamped rather than trusted.
        assert_eq!(LexicalSupportJudger::new(7.0).min_overlap, 1.0);
        assert_eq!(LexicalSupportJudger::new(-1.0).min_overlap, 0.0);
        assert_eq!(LexicalSupportJudger::new(f32::NAN).min_overlap, 1.0);
        assert_eq!(LexicalSupportJudger::default().min_overlap, 0.6);
    }

    #[test]
    fn atomic_precision_scores_a_claim_against_its_evidence() {
        let report = check_factuality(
            "The latency doubled, and the cache was cold.",
            &evidence(&["the latency doubled after the deploy"]),
            FactualityCheck::AtomicPrecision,
        )
        .unwrap();
        assert_eq!(report.check, FactualityCheck::AtomicPrecision);
        assert_eq!(report.atoms.len(), 2);
        // "The latency doubled" is covered; "the cache was cold" is not.
        assert!((report.support - 0.5).abs() < 1e-6, "{report:?}");
        assert_eq!(report.samples, 1);
        assert_eq!(report.unsupported().len(), 1);
    }

    #[test]
    fn self_consistency_is_reachable_through_the_shared_entry_point() {
        let report = check_factuality(
            "the cache was cold",
            &evidence(&[
                "the cache was cold",
                "the cache was cold",
                "the network saturated",
            ]),
            FactualityCheck::SelfConsistency,
        )
        .unwrap();
        assert_eq!(report.check, FactualityCheck::SelfConsistency);
        assert!((report.support - 2.0 / 3.0).abs() < 1e-6, "{report:?}");
    }

    #[test]
    fn search_augmented_refuses_rather_than_inventing_a_number() {
        let err = check_factuality(
            "the cache was cold",
            &evidence(&["the cache was cold"]),
            FactualityCheck::SearchAugmented,
        )
        .unwrap_err();
        assert_eq!(err.code(), "unavailable");
        assert!(err.is_unavailable());
        assert!(err.to_string().contains("Phase 10"), "{err}");
    }

    #[test]
    fn a_search_augmented_checker_judges_each_atom_against_what_it_retrieved() {
        /// A tool that answers from a fixed table, so the test is deterministic.
        struct FixedTool;

        impl FactualityTool for FixedTool {
            fn search(&self, query: &str) -> Result<Vec<String>> {
                if query.contains("latency") {
                    return Ok(evidence(&["the latency doubled after the deploy"]));
                }
                if query.contains("replica") {
                    return Err(DecisionError::Unavailable("no index".to_string()));
                }
                Ok(Vec::new())
            }
        }

        let checker = SearchAugmentedChecker::new(FixedTool);
        let report = checker
            .check("The latency doubled, and the cache was cold.")
            .unwrap();
        assert_eq!(report.check, FactualityCheck::SearchAugmented);
        assert_eq!(report.atoms.len(), 2);
        assert!((report.support - 0.5).abs() < 1e-6, "{report:?}");

        // A retrieval refusal propagates: a failed search is not a finding.
        let refused = checker.check("the replica lagged").unwrap_err();
        assert!(refused.is_unavailable());

        // And the report renders stably.
        let again = checker
            .check("The latency doubled, and the cache was cold.")
            .unwrap();
        assert_eq!(report.canonical, again.canonical);
    }
}
