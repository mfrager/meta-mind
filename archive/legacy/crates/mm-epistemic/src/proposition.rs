//! The proposition: what a claim is *about*, independent of anyone's attitude to it.
//!
//! A proposition is a triple — `(subject, predicate, object)` — and nothing else.
//! Keeping it separate from the claim is what lets the same proposition be
//! reported by one source, inferred by another, and observed by a third, with the
//! status telling them apart instead of the wording.
//!
//! The object term is rendered to text by [`term_to_text`] and parsed back by
//! [`term_from_text`]. That pair is a *contract*: the SQLite row and the RDF mirror
//! both go through it, so the round-trip property test is a statement about the
//! whole store, not about a helper. Literal values are escaped, because an
//! unescaped `"` would make the encoding ambiguous.

use oxrdf::{Literal, NamedNode, Term};
use serde::{Deserialize, Serialize};

use crate::error::{EpistemicError, Result};

/// The `xsd:` namespace.
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// A subject–predicate–object statement.
///
/// `oxrdf`'s terms do not implement serde, and they should not: a term's wire form
/// is the point. The type therefore serializes as three strings and parses back
/// through [`term_from_text`], which is the same codec the SQLite row uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PropositionText", into = "PropositionText")]
pub struct Proposition {
    /// What the statement is about.
    pub subject: NamedNode,
    /// The relation asserted.
    pub predicate: NamedNode,
    /// The object: an IRI or a literal.
    pub object: Term,
}

/// The serializable form of a [`Proposition`]: three strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropositionText {
    /// The subject IRI.
    pub subject: String,
    /// The predicate IRI.
    pub predicate: String,
    /// The object, in canonical term text, or as a bare string for a literal.
    pub object: String,
}

impl From<Proposition> for PropositionText {
    fn from(proposition: Proposition) -> Self {
        PropositionText {
            subject: proposition.subject.as_str().to_string(),
            predicate: proposition.predicate.as_str().to_string(),
            object: term_to_text(&proposition.object),
        }
    }
}

impl TryFrom<PropositionText> for Proposition {
    type Error = EpistemicError;

    fn try_from(text: PropositionText) -> Result<Self> {
        // A fixture may write an object as a bare string and mean a literal; the
        // canonical forms are how the row and the mirror spell it. Accepting both
        // is what keeps the fixtures readable without a second convention.
        let object = if text.object.starts_with('<') || text.object.starts_with('"') {
            term_from_text(&text.object)?
        } else {
            Term::Literal(Literal::new_simple_literal(text.object))
        };
        Ok(Proposition {
            subject: iri_node(&text.subject)?,
            predicate: iri_node(&text.predicate)?,
            object,
        })
    }
}

impl Proposition {
    /// Build a proposition from text, validating every part.
    pub fn new(subject: &str, predicate: &str, object: &str, object_is_iri: bool) -> Result<Self> {
        let subject = iri_node(subject)?;
        let predicate = iri_node(predicate)?;
        let object = if object_is_iri {
            Term::from(iri_node(object)?)
        } else {
            Term::Literal(Literal::new_simple_literal(object))
        };
        Ok(Proposition {
            subject,
            predicate,
            object,
        })
    }

    /// A proposition whose object is a plain string literal.
    pub fn literal(subject: &str, predicate: &str, object: &str) -> Result<Self> {
        Proposition::new(subject, predicate, object, false)
    }

    /// A proposition whose object is another IRI.
    pub fn relation(subject: &str, predicate: &str, object: &str) -> Result<Self> {
        Proposition::new(subject, predicate, object, true)
    }

    /// The `(subject, predicate)` bucket key. Contradiction detection indexes by
    /// this, which is what keeps it from being pairwise over the whole store.
    pub fn key(&self) -> (String, String) {
        (
            self.subject.as_str().to_string(),
            self.predicate.as_str().to_string(),
        )
    }

    /// The object, rendered canonically.
    pub fn object_text(&self) -> String {
        term_to_text(&self.object)
    }

    /// The subject IRI.
    pub fn subject_text(&self) -> &str {
        self.subject.as_str()
    }

    /// The predicate IRI.
    pub fn predicate_text(&self) -> &str {
        self.predicate.as_str()
    }

    /// True when two propositions say something about the same subject and
    /// relation, whatever their objects. This is the comparability test.
    pub fn same_subject_and_predicate(&self, other: &Proposition) -> bool {
        self.key() == other.key()
    }
}

/// Build a named node from text, refusing something that is not an IRI.
///
/// `NamedNode::new`'s error type is not part of this crate's contract, and an IRI
/// that the store accepted but the parser would not is worse than a refusal, so
/// the check is explicit and the constructor is the unchecked one.
pub fn iri_node(text: &str) -> Result<NamedNode> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.contains(':') || trimmed.contains(char::is_whitespace) {
        return Err(EpistemicError::validation(
            "iri",
            format!("{text:?} is not an IRI"),
        ));
    }
    Ok(NamedNode::new_unchecked(trimmed))
}

/// Render a term canonically.
pub fn term_to_text(term: &Term) -> String {
    match term {
        Term::NamedNode(node) => format!("<{}>", node.as_str()),
        Term::BlankNode(node) => format!("_:{}", node.as_str()),
        Term::Literal(literal) => {
            let mut out = format!("\"{}\"", escape(literal.value()));
            if let Some(language) = literal.language() {
                out.push('@');
                out.push_str(language);
            } else {
                let datatype = literal.datatype().as_str();
                if datatype != format!("{XSD}string") {
                    out.push_str(&format!("^^{datatype}"));
                }
            }
            out
        }
    }
}

/// Parse a term back out of its canonical rendering.
pub fn term_from_text(text: &str) -> Result<Term> {
    let text = text.trim();
    if let Some(inner) = text.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
        return Ok(Term::from(iri_node(inner)?));
    }
    if text.starts_with("_:") {
        return Err(EpistemicError::Codec(
            "propositions do not use blank nodes".to_string(),
        ));
    }
    let Some(rest) = text.strip_prefix('"') else {
        return Err(EpistemicError::Codec(format!("{text:?} is not a term")));
    };
    // Walk to the closing quote, honouring escapes, so a value containing `\"`
    // parses back to the same string it was rendered from.
    let mut value = String::new();
    let mut chars = rest.char_indices();
    let mut end = None;
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => match chars.next().map(|(_, c)| c) {
                Some('n') => value.push('\n'),
                Some('r') => value.push('\r'),
                Some('t') => value.push('\t'),
                Some('"') => value.push('"'),
                Some('\\') => value.push('\\'),
                Some(other) => {
                    return Err(EpistemicError::Codec(format!(
                        "unknown escape \\{other} in {text:?}"
                    )))
                }
                None => {
                    return Err(EpistemicError::Codec(format!(
                        "trailing escape in {text:?}"
                    )))
                }
            },
            '"' => {
                end = Some(index);
                break;
            }
            other => value.push(other),
        }
    }
    let Some(end) = end else {
        return Err(EpistemicError::Codec(format!(
            "unterminated literal in {text:?}"
        )));
    };
    let suffix = &rest[end + 1..];
    if suffix.is_empty() {
        return Ok(Term::Literal(Literal::new_simple_literal(value)));
    }
    if let Some(language) = suffix.strip_prefix('@') {
        let literal = Literal::new_language_tagged_literal(value, language)
            .map_err(|e| EpistemicError::Codec(format!("bad language tag {language:?}: {e}")))?;
        return Ok(Term::Literal(literal));
    }
    let Some(datatype) = suffix.strip_prefix("^^") else {
        return Err(EpistemicError::Codec(format!(
            "unparsable literal suffix {suffix:?} in {text:?}"
        )));
    };
    Ok(Term::Literal(Literal::new_typed_literal(
        value,
        iri_node(datatype)?,
    )))
}

/// Escape a literal value so the rendering can be parsed back.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// A proposition with a literal object and a `xsd:double` value, for the shapes
/// that need a typed number (thresholds, costs).
pub fn double_literal(value: f64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        value.to_string(),
        NamedNode::new_unchecked(format!("{XSD}double")),
    ))
}

/// The `rdf:type` IRI.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// How bad it is if an assumption is wrong.
///
/// The weights are exact binary fractions (`1/16`, `1/4`, `1/2`, `3/4`, `1`) so a
/// priority computed from them is reproducible bit-for-bit in a golden file. They
/// order the levels and nothing else: they are not probabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Being wrong costs nothing measurable.
    Negligible,
    /// Being wrong is annoying.
    Low,
    /// Being wrong costs time or trust.
    Medium,
    /// Being wrong damages the being's work or its relationships.
    High,
    /// Being wrong is unrecoverable.
    Catastrophic,
}

/// Every risk level, least to most severe.
pub const RISK_LEVELS: [RiskLevel; 5] = [
    RiskLevel::Negligible,
    RiskLevel::Low,
    RiskLevel::Medium,
    RiskLevel::High,
    RiskLevel::Catastrophic,
];

impl RiskLevel {
    /// The stable wire name, matching the `assumptions.consequence_if_false` CHECK.
    pub fn as_str(self) -> &'static str {
        match self {
            RiskLevel::Negligible => "negligible",
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Catastrophic => "catastrophic",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        RISK_LEVELS.into_iter().find(|level| level.as_str() == text)
    }

    /// The level's weight in the priority arithmetic, in `(0,1]`.
    pub fn weight(self) -> f64 {
        match self {
            RiskLevel::Negligible => 0.0625,
            RiskLevel::Low => 0.25,
            RiskLevel::Medium => 0.5,
            RiskLevel::High => 0.75,
            RiskLevel::Catastrophic => 1.0,
        }
    }
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proposition_validates_every_part() {
        let p = Proposition::literal(
            "https://metamind.dev/data/a",
            "https://metamind.dev/ontology#status",
            "hello",
        )
        .unwrap();
        assert_eq!(p.object_text(), "\"hello\"");
        assert!(Proposition::literal("not-an-iri", "https://x/y", "v").is_err());
        assert!(Proposition::literal("https://x/y", "has spaces", "v").is_err());
    }

    #[test]
    fn terms_round_trip_through_their_text_form() {
        let cases = vec![
            Term::from(NamedNode::new_unchecked("https://metamind.dev/data/x")),
            Term::Literal(Literal::new_simple_literal("plain")),
            Term::Literal(Literal::new_simple_literal("quote \" and \\ slash")),
            Term::Literal(Literal::new_simple_literal("line\nbreak")),
            Term::Literal(Literal::new_typed_literal(
                "1.5",
                NamedNode::new_unchecked(format!("{XSD}double")),
            )),
            Term::Literal(Literal::new_language_tagged_literal("hola", "es").unwrap()),
        ];
        for term in cases {
            let text = term_to_text(&term);
            let back = term_from_text(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(term, back, "{text}");
        }
    }

    #[test]
    fn an_unparsable_term_is_an_error_not_a_panic() {
        for bad in ["nonsense", "\"unterminated", "_:blank", "\"v\"^^not-an-iri"] {
            assert!(term_from_text(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn risk_levels_are_ordered_and_weighed_exactly() {
        for level in RISK_LEVELS {
            assert_eq!(RiskLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(RiskLevel::parse("nope"), None);
        // Exact binary fractions, so a golden file can compare bit-for-bit.
        for level in RISK_LEVELS {
            let weight = level.weight();
            assert!(weight > 0.0 && weight <= 1.0, "{level} = {weight}");
            assert_eq!(weight * 16.0, (weight * 16.0).round());
        }
        assert!(RiskLevel::Negligible < RiskLevel::Low);
        assert!(RiskLevel::Low < RiskLevel::Catastrophic);
        assert!(RiskLevel::Catastrophic.weight() > RiskLevel::High.weight());
    }

    #[test]
    fn comparability_is_subject_and_predicate_only() {
        let a = Proposition::literal("https://x/s", "https://x/p", "one").unwrap();
        let b = Proposition::literal("https://x/s", "https://x/p", "two").unwrap();
        let c = Proposition::literal("https://x/s", "https://x/q", "one").unwrap();
        assert!(a.same_subject_and_predicate(&b));
        assert!(!a.same_subject_and_predicate(&c));
    }
}
