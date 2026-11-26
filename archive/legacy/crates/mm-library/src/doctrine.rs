//! Doctrine, principle, heuristic and anti-pattern.
//!
//! Four kinds that differ in *how strong* a claim they make about method, not in
//! what fields they carry, so each is a constructor over [`Declarative`] with the
//! validation that kind needs:
//!
//! * a **doctrine** says what to stand for, and must say when it applies and what
//!   backs it;
//! * a **principle** constrains method and must name what backs it;
//! * a **heuristic** is a cheap rule that usually helps, so it must say when it
//!   applies and what it was noticed from;
//! * an **anti-pattern** names a way of failing and, without a corrective rule, is
//!   a complaint rather than a lesson.

use mm_core::ActivationCondition;

use crate::entry::{Declarative, EntryKind};
use crate::error::Result;

/// A doctrine: a general commitment about how to think.
pub fn doctrine(slug: &str, title: &str, purpose: &str) -> Result<Declarative> {
    Declarative::new(EntryKind::Doctrine, slug, title, purpose)
}

/// A principle: a rule that constrains method.
pub fn principle(slug: &str, title: &str, purpose: &str) -> Result<Declarative> {
    Declarative::new(EntryKind::Principle, slug, title, purpose)
}

/// A heuristic: a cheap rule of thumb that usually helps.
pub fn heuristic(slug: &str, title: &str, purpose: &str) -> Result<Declarative> {
    Declarative::new(EntryKind::Heuristic, slug, title, purpose)
}

/// An anti-pattern: a recurring shape of failure with its corrective rule.
///
/// The corrective rule is required by construction — [`declarative_validate`]
/// refuses an anti-pattern without one — so this constructor cannot produce a
/// complaint dressed as a lesson.
pub fn anti_pattern(
    slug: &str,
    title: &str,
    purpose: &str,
    corrective_rule: &str,
) -> Result<Declarative> {
    Ok(
        Declarative::new(EntryKind::AntiPattern, slug, title, purpose)?
            .corrective_rule(corrective_rule),
    )
}

/// Re-run the kind-specific validation of a declarative entry.
pub fn declarative_validate(entry: &Declarative) -> Result<()> {
    crate::entry::LibraryEntry::validate(entry)
}

/// When a doctrine applies: a condition the caller states plainly.
pub fn when(condition: &str) -> Result<ActivationCondition> {
    ActivationCondition::new(condition).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{iri, LibraryEntry};

    #[test]
    fn the_constructors_produce_their_own_kind() {
        assert_eq!(doctrine("d", "D", "P").unwrap().kind, EntryKind::Doctrine);
        assert_eq!(principle("p", "P", "P").unwrap().kind, EntryKind::Principle);
        assert_eq!(heuristic("h", "H", "P").unwrap().kind, EntryKind::Heuristic);
        assert_eq!(
            anti_pattern("a", "A", "P", "do the other thing")
                .unwrap()
                .kind,
            EntryKind::AntiPattern
        );
    }

    #[test]
    fn an_anti_pattern_always_arrives_with_its_corrective_rule() {
        let mut entry = anti_pattern(
            "premature-optimization",
            "Premature optimization",
            "Tuning what nobody measured.",
            "measure first",
        )
        .unwrap();
        assert_eq!(entry.corrective_rule.as_deref(), Some("measure first"));
        // The constructor cannot produce one without it; the rest of the commit
        // rule still applies, so evidence is added before the entry is valid.
        assert!(entry.validate().is_err());
        entry.evidence.push(iri::library(EntryKind::Case, "c"));
        entry.validate().unwrap();
    }

    #[test]
    fn a_doctrine_without_evidence_is_refused_at_validation() {
        let entry = doctrine("d", "D", "P")
            .unwrap()
            .when(when("state changes that others depend on").unwrap());
        assert!(entry.validate().is_err(), "a doctrine names what backs it");
        let entry = entry.backed_by(iri::library(EntryKind::Case, "c"));
        entry.validate().unwrap();
    }

    #[test]
    fn a_blank_condition_is_a_validation_error_not_a_panic() {
        assert!(when("   ").is_err());
    }
}
