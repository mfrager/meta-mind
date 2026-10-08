//! The IRI grammar. Every identifier in Metamind is spelled by a function here,
//! so no layer invents its own spelling of the same entity.

use oxrdf::NamedNode;
use ulid::Ulid;

use crate::error::MmError;

/// The ontology (T-Box) namespace.
pub const MM: &str = "https://metamind.dev/ontology#";
/// The code/metadata namespace (fragment form, for terms like `mmc:Edit`).
pub const MMC: &str = "https://metamind.dev/code#";
/// The code/metadata path base (slash form, for resources like modules).
pub const CODE: &str = "https://metamind.dev/code/";
/// The instance-data namespace; every instance IRI is `<DATA><ulid>`.
pub const DATA: &str = "https://metamind.dev/data/";
/// The named-graph namespace.
pub const GRAPH: &str = "https://metamind.dev/graph/";
/// The design-document namespace.
pub const DESIGN: &str = "https://metamind.dev/design/";
/// The vendored-code namespace.
pub const VENDOR: &str = "https://metamind.dev/vendor/";
/// The build-phase namespace; a phase IRI is `<PHASE><n>`.
pub const PHASE: &str = "https://metamind.dev/phase/";
/// The PROV-O namespace, for the provenance graph's standard predicates.
pub const PROV: &str = "http://www.w3.org/ns/prov#";

/// The named graphs Metamind owns. A quad outside this set requires a migration.
pub const NAMED_GRAPHS: [&str; 7] = [
    "being",
    "memory",
    "epistemic",
    "library",
    "world",
    "provenance",
    "code",
];

/// The instance IRI of an entity: `https://metamind.dev/data/{ulid}`.
pub fn data(id: &Ulid) -> NamedNode {
    NamedNode::new_unchecked(format!("{DATA}{}", crate::ulid_string(id)))
}

/// Parse an instance IRI back to its ULID.
pub fn data_ulid(iri: &str) -> Result<Ulid, MmError> {
    let tail = iri
        .strip_prefix(DATA)
        .ok_or_else(|| MmError::Internal(format!("not a Metamind data IRI: {iri}")))?;
    crate::id::parse_ulid(tail)
}

/// An ontology IRI: `mm:<local>`.
pub fn mm(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{MM}{local}"))
}

/// A code-namespace IRI: `mmc:<local>`.
pub fn mmc(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{MMC}{local}"))
}

/// A vendor-code IRI: `https://metamind.dev/vendor/{origin}/{crate}`.
pub fn vendor(origin: &str, krate: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{VENDOR}{origin}/{krate}"))
}

/// The IRI of a named graph.
pub fn graph(name: &str) -> String {
    format!("{GRAPH}{name}")
}

/// True when `name` is one of the kernel's named graphs. Accepts a bare name or a
/// full graph IRI.
pub fn is_named_graph(name: &str) -> bool {
    let bare = name.strip_prefix(GRAPH).unwrap_or(name);
    NAMED_GRAPHS.contains(&bare)
}

/// The stable IRI of a module, derived from its repository path:
/// `https://metamind.dev/code/module/{path}`.
pub fn module(path: &str) -> NamedNode {
    let trimmed = path.trim_matches('/');
    NamedNode::new_unchecked(format!("{CODE}module/{trimmed}"))
}

/// The stable IRI of a design document, optionally anchored at a section.
pub fn design(doc: &str, anchor: Option<&str>) -> NamedNode {
    let doc = doc.trim_matches('/');
    match anchor {
        Some(a) => NamedNode::new_unchecked(format!("{DESIGN}{doc}#{a}")),
        None => NamedNode::new_unchecked(format!("{DESIGN}{doc}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_iri_matches_contract() {
        let id = Ulid::from_parts(1_700_000_000_000, 42);
        let iri = data(&id);
        assert!(iri.as_str().starts_with(DATA));
        assert_eq!(
            iri.as_str(),
            format!("{DATA}{}", crate::ulid_string(&id)),
            "IRIs must use the lowercase Crockford form"
        );
        assert_eq!(data_ulid(iri.as_str()).unwrap(), id);
    }

    #[test]
    fn named_graph_membership() {
        assert!(is_named_graph("being"));
        assert!(is_named_graph("https://metamind.dev/graph/provenance"));
        assert!(!is_named_graph("unknown"));
        assert_eq!(graph("being"), "https://metamind.dev/graph/being");
    }

    #[test]
    fn stable_path_derived_iris() {
        assert_eq!(
            module("system/kernel-bootstrap").as_str(),
            "https://metamind.dev/code/module/system/kernel-bootstrap"
        );
        assert_eq!(
            design("digital_mind_design1.md", Some("92")).as_str(),
            "https://metamind.dev/design/digital_mind_design1.md#92"
        );
        assert_eq!(mm("Being").as_str(), "https://metamind.dev/ontology#Being");
    }

    #[test]
    fn data_ulid_rejects_foreign_iris() {
        assert!(data_ulid("https://example.com/x").is_err());
    }
}
