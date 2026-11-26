//! The code-metadata URI funnel.
//!
//! Every identifier in the `/code` graph is spelled here and nowhere else, so the
//! scanner, the emitter, the verifier, and the query layer cannot disagree about
//! what a module or a symbol is called. Three of the six are *not* path-stable,
//! and the difference is the point:
//!
//! | Entity | Stability |
//! |---|---|
//! | module | path-stable — moving the file moves the IRI |
//! | module version | immutable per release |
//! | file | content-addressed — editing the file yields a new IRI |
//! | symbol | SCIP-style descriptor — survives reformatting and line moves |
//! | capability | name-stable |
//! | phase | fixed |
//!
//! A file IRI carrying the sha256 is what makes "the same bytes" checkable; a
//! symbol IRI carrying a semantic descriptor is what makes a symbol trackable
//! across edits.

use oxrdf::NamedNode;

use crate::iri;

/// The stable IRI of a module: `.../code/module/{identity-path}`.
///
/// The `modules/` directory is a container, not part of a module's identity:
/// `modules/system/kernel-bootstrap` is the module `system/kernel-bootstrap`, the
/// URI its `plugin.toml` has declared since Phase 1. Every other path is used as
/// written, so a crate keeps its `crates/` segment.
pub fn module_iri(rel_path: &str) -> NamedNode {
    iri::module(module_identity_path(rel_path))
}

/// A module's identity path: its repository path with the `modules/` container
/// prefix removed.
pub fn module_identity_path(rel_path: &str) -> &str {
    let trimmed = rel_path.trim_matches('/');
    trimmed.strip_prefix("modules/").unwrap_or(trimmed)
}

/// The IRI of one released module version:
/// `.../code/module/{rel-path}@{semver}`.
pub fn module_version_iri(rel_path: &str, semver: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{}@{semver}", module_iri(rel_path).as_str()))
}

/// The content-addressed IRI of a source file:
/// `.../code/file/{rel-path}@{sha256}`.
pub fn file_iri(rel_path: &str, sha256: &str) -> NamedNode {
    let path = rel_path.trim_matches('/');
    NamedNode::new_unchecked(format!("{}file/{path}@{sha256}", iri::CODE))
}

/// The SCIP-style IRI of a symbol:
/// `.../code/symbol/{rel-path}#{descriptor}`.
///
/// The descriptor is percent-encoded for the fragment: RFC 3986 does not allow a
/// literal `#` inside a fragment, and a descriptor uses `#` as its scope
/// separator (`Codex#scan().`). Without the encoding the IRI would carry two `#`
/// characters and re-parsing it would silently truncate the descriptor.
pub fn symbol_iri(rel_path: &str, descriptor: &str) -> NamedNode {
    let path = rel_path.trim_matches('/');
    NamedNode::new_unchecked(format!(
        "{}symbol/{path}#{}",
        iri::CODE,
        encode_fragment(descriptor)
    ))
}

/// Percent-encode the characters an IRI fragment may not carry literally.
///
/// `%` is escaped alongside `#` so the mapping is injective and a descriptor can
/// always be recovered from its IRI.
fn encode_fragment(descriptor: &str) -> String {
    let mut out = String::with_capacity(descriptor.len());
    for ch in descriptor.chars() {
        match ch {
            '#' => out.push_str("%23"),
            '%' => out.push_str("%25"),
            ' ' => out.push_str("%20"),
            _ => out.push(ch),
        }
    }
    out
}

/// The name-stable IRI of a capability: `.../code/capability/{name}`.
///
/// Accepts a manifest's prefixed form (`mm:CaseRetrieval`) or a bare name.
pub fn capability_iri(name: &str) -> NamedNode {
    let local = name.strip_prefix("mm:").unwrap_or(name);
    NamedNode::new_unchecked(format!("{}capability/{local}", iri::CODE))
}

/// The IRI of a build phase: `https://metamind.dev/phase/{n}`.
pub fn phase_iri(n: u8) -> NamedNode {
    NamedNode::new_unchecked(format!("{}{n}", iri::PHASE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_version_iris_are_path_derived() {
        assert_eq!(
            module_iri("crates/mm-codex").as_str(),
            "https://metamind.dev/code/module/crates/mm-codex"
        );
        // The module funnel is the same one Phase 1's modules already use.
        assert_eq!(
            module_iri("system/kernel-bootstrap"),
            iri::module("system/kernel-bootstrap")
        );
        // `modules/` is a container: dropping it makes the path-derived URI agree
        // with what the `plugin.toml` in that directory declares.
        assert_eq!(
            module_iri("modules/system/kernel-bootstrap").as_str(),
            "https://metamind.dev/code/module/system/kernel-bootstrap"
        );
        assert_eq!(module_identity_path("modules/a"), "a");
        assert_eq!(module_identity_path("crates/a"), "crates/a");
        assert_eq!(
            module_version_iri("crates/mm-codex", "0.1.0").as_str(),
            "https://metamind.dev/code/module/crates/mm-codex@0.1.0"
        );
    }

    #[test]
    fn file_iris_keep_the_full_path() {
        // Only the module IRI drops `modules/`; a file is still identified by its
        // whole repository path.
        let f = file_iri("modules/system/kernel-bootstrap/src/lib.rs", "aa");
        assert_eq!(
            f.as_str(),
            "https://metamind.dev/code/file/modules/system/kernel-bootstrap/src/lib.rs@aa"
        );
    }

    #[test]
    fn file_iri_is_content_addressed() {
        let a = file_iri("crates/mm-codex/src/lib.rs", "ab12");
        assert_eq!(
            a.as_str(),
            "https://metamind.dev/code/file/crates/mm-codex/src/lib.rs@ab12"
        );
        // Same path, different bytes => different identity.
        let b = file_iri("crates/mm-codex/src/lib.rs", "cd34");
        assert_ne!(a, b);
        // Same bytes at the same path => the same identity, however spelled.
        assert_eq!(a, file_iri("/crates/mm-codex/src/lib.rs/", "ab12"));
    }

    #[test]
    fn symbol_iri_survives_line_moves() {
        let s = symbol_iri("crates/mm-codex", "Codex#scan().");
        // The scope separator is escaped, so the IRI holds exactly one `#`.
        assert_eq!(
            s.as_str(),
            "https://metamind.dev/code/symbol/crates/mm-codex#Codex%23scan()."
        );
        assert_eq!(s.as_str().matches('#').count(), 1);
        // An unscoped descriptor needs no encoding, so it reads as written.
        assert_eq!(
            symbol_iri("crates/mm-codex", "scan().").as_str(),
            "https://metamind.dev/code/symbol/crates/mm-codex#scan()."
        );
        // The IRI carries no offset, so a move within the file cannot change it.
        assert_eq!(s, symbol_iri("crates/mm-codex", "Codex#scan()."));
        // Renaming does change it.
        assert_ne!(s, symbol_iri("crates/mm-codex", "Codex#scan_all()."));
    }

    #[test]
    fn capability_iri_normalizes_the_prefixed_and_bare_forms() {
        assert_eq!(
            capability_iri("mm:CaseRetrieval").as_str(),
            "https://metamind.dev/code/capability/CaseRetrieval"
        );
        assert_eq!(
            capability_iri("mm:CaseRetrieval"),
            capability_iri("CaseRetrieval")
        );
    }

    #[test]
    fn phase_iri_is_not_under_the_code_namespace() {
        assert_eq!(phase_iri(7).as_str(), "https://metamind.dev/phase/7");
        assert_eq!(phase_iri(1).as_str(), iri::PHASE.to_string() + "1");
    }
}
