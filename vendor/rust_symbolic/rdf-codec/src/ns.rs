//! Namespace IRI construction (§4.2): classes, class-scoped properties,
//! enum variants, and variant fields all live in one subsystem namespace.

/// A namespace with an IRI base. Used both for schema namespaces
/// (`https://example.org/ns/solver#`) and data namespaces
/// (`https://example.org/data/solver/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    pub iri: String,
}

impl Namespace {
    pub fn new(iri: impl Into<String>) -> Self {
        Self { iri: iri.into() }
    }

    /// `{iri}{Class}` — the class IRI.
    pub fn class(&self, local: &str) -> String {
        format!("{}{}", self.iri, local)
    }

    /// `{iri}{Class}/{field}` — a class-scoped property IRI (flat and direct).
    pub fn property(&self, class: &str, field: &str) -> String {
        format!("{}{}/{}", self.iri, class, field)
    }

    /// `{iri}{Enum}/{Variant}` — an enum variant class IRI.
    pub fn variant(&self, enm: &str, variant: &str) -> String {
        format!("{}{}/{}", self.iri, enm, variant)
    }

    /// `{iri}{Enum}/{Variant}/{field}` — a variant payload property IRI.
    pub fn variant_field(&self, enm: &str, variant: &str, field: &str) -> String {
        format!("{}{}/{}/{}", self.iri, enm, variant, field)
    }
}
