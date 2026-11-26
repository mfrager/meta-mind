//! Technique, pattern and evaluation — the three kinds the selector reads.
//!
//! A **technique** is the unit Phase 8 selects: it states when it applies, what
//! backs it, a case it was tried on and a confidence, and it carries the two things
//! [`crate::applicability`] needs to rank it — a stored activation blend and a
//! per-domain fitness. A **pattern** is a shape of solution rather than a
//! procedure, so it names what backs it and when it applies. An **evaluation** is a
//! way of judging a result, so what matters is what backs *it*.
//!
//! Nothing here computes a score. The arithmetic lives in
//! [`crate::applicability`], which is pure and golden-tested; these are the
//! records it ranks.

use crate::entry::{Declarative, EntryKind};
use crate::error::Result;

/// A technique: a procedure for a kind of problem.
pub fn technique(slug: &str, title: &str, purpose: &str) -> Result<Declarative> {
    Declarative::new(EntryKind::Technique, slug, title, purpose)
}

/// A pattern: a recurring shape of solution.
pub fn pattern(slug: &str, title: &str, purpose: &str) -> Result<Declarative> {
    Declarative::new(EntryKind::Pattern, slug, title, purpose)
}

/// An evaluation: a way of judging a result.
pub fn evaluation(slug: &str, title: &str, purpose: &str) -> Result<Declarative> {
    Declarative::new(EntryKind::Evaluation, slug, title, purpose)
}

/// The domains a technique has been used in, from its fitness map.
pub fn domains(entry: &Declarative) -> Vec<String> {
    let mut domains: Vec<String> = entry.domain_fitness.keys().cloned().collect();
    domains.sort();
    domains
}

/// The fitness the entry records for one domain, or `None` when it has never been
/// used there.
///
/// `None` is not the same as zero: an untried technique is unknown, not bad, and
/// [`crate::applicability`] treats it as the neutral baseline rather than as
/// evidence against it.
pub fn fitness_in(entry: &Declarative, domain: &str) -> Option<f32> {
    entry.domain_fitness.get(domain).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{iri, LibraryEntry};

    #[test]
    fn a_technique_needs_its_case_and_confidence() {
        let bare = technique("decompose", "Decompose", "Split it").unwrap();
        assert!(bare.validate().is_err(), "no conditions, no evidence");
        let complete = bare
            .when(crate::doctrine::when("parts are independent").unwrap())
            .backed_by(iri::library(EntryKind::Case, "c"))
            .illustrated_by(iri::library(EntryKind::Case, "c"));
        assert!(
            complete.validate().is_err(),
            "a technique carries a confidence"
        );
        complete.with_confidence(0.85).unwrap().validate().unwrap();
    }

    #[test]
    fn a_pattern_needs_conditions_and_evidence_but_not_a_case() {
        let entry = pattern("p", "P", "P")
            .unwrap()
            .when(crate::doctrine::when("a failure is reported").unwrap())
            .backed_by(iri::library(EntryKind::Case, "c"));
        entry.validate().unwrap();
    }

    #[test]
    fn an_evaluation_needs_what_backs_it() {
        let entry = evaluation("e", "E", "P").unwrap();
        assert!(entry.validate().is_err());
        entry
            .backed_by(iri::library(EntryKind::Case, "c"))
            .validate()
            .unwrap();
    }

    #[test]
    fn an_untried_domain_is_none_not_zero() {
        let entry = technique("t", "T", "P")
            .unwrap()
            .when(crate::doctrine::when("x").unwrap())
            .backed_by(iri::library(EntryKind::Case, "c"))
            .illustrated_by(iri::library(EntryKind::Case, "c"))
            .with_confidence(0.5)
            .unwrap()
            .with_domain_fitness("software", 0.9)
            .unwrap();
        assert_eq!(fitness_in(&entry, "software"), Some(0.9));
        assert_eq!(fitness_in(&entry, "diagnosis"), None);
        assert_eq!(domains(&entry), vec!["software".to_string()]);
    }

    #[test]
    fn a_domain_fitness_outside_the_unit_interval_is_refused() {
        let entry = technique("t", "T", "P").unwrap();
        assert!(entry.with_domain_fitness("software", 1.5).is_err());
    }
}
