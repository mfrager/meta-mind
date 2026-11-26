//! SCIP-style symbol descriptors.
//!
//! A descriptor is the stable part of a symbol's identity: it is derived from the
//! *semantic* path (enclosing scope + item name + kind), never from a byte offset
//! or a line number. That is what makes a symbol IRI survive reformatting, item
//! reordering, and moving a definition within its file, while still changing when
//! the symbol is renamed or moved to a different scope.
//!
//! Descriptors follow SCIP's punctuation:
//!
//! | Kind | Suffix | Example |
//! |---|---|---|
//! | type / trait / module | `#` | `Codex#`, `VerifyFailure#` |
//! | fn | `().` | `scan().`, `Codex#scan().` |
//! | const / static / macro | `.` | `MAX_MODULES.` |
//!
//! The `/` seen in full SCIP strings separates package and namespace components;
//! Metamind carries the module path in the IRI's path component instead, so the
//! fragment holds only the local descriptor. `mmc:descriptor` and the IRI
//! fragment are therefore the same string.

use crate::model::SymbolKind;

/// The descriptor suffix for a kind.
pub fn kind_suffix(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module | SymbolKind::Trait | SymbolKind::Type => "#",
        SymbolKind::Fn | SymbolKind::Interface => "().",
        SymbolKind::Const => ".",
    }
}

/// Build the descriptor of a definition.
///
/// `scope` is the enclosing type or module name (empty for a free item); `name` is
/// the item's own name.
pub fn descriptor(scope: &str, name: &str, kind: SymbolKind) -> String {
    let mut out = String::with_capacity(scope.len() + name.len() + 3);
    if !scope.is_empty() {
        out.push_str(scope);
        out.push('#');
    }
    out.push_str(name);
    out.push_str(kind_suffix(kind));
    out
}

/// The item's own name, recovered from its descriptor.
///
/// Used to build the name index that resolves references, so it must agree with
/// [`descriptor`] exactly. `Codex#scan().` and `GraphStore/Graph#sparql().` both
/// reduce to the final segment.
pub fn local_name(descriptor: &str) -> &str {
    let body = descriptor
        .strip_suffix("().")
        .or_else(|| descriptor.strip_suffix('#'))
        .or_else(|| descriptor.strip_suffix('.'))
        .unwrap_or(descriptor);
    body.rsplit(['#', '/']).next().unwrap_or(body)
}

/// True when `descriptor` is a shape a symbol IRI can carry.
///
/// `#` is allowed — it is the scope separator, and [`mm_core::codex::symbol_iri`]
/// escapes it for the fragment. Whitespace and non-ASCII are rejected: they never
/// occur in a Rust item name, and letting them through would put an unescaped,
/// unqueryable character into every IRI that mentions the symbol. `<`, `>`, `"`,
/// and the other delimiters are rejected too: they are what terminates a Turtle
/// `<...>` IRI term, so a descriptor carrying one would emit a document that no
/// longer parses.
pub fn is_valid_descriptor(descriptor: &str) -> bool {
    !descriptor.is_empty()
        && descriptor.is_ascii()
        && !descriptor.chars().any(|c| {
            c.is_whitespace() || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::codex::symbol_iri;

    #[test]
    fn descriptors_follow_scip_punctuation() {
        assert_eq!(
            descriptor("", "case_match", SymbolKind::Fn),
            "case_match()."
        );
        assert_eq!(descriptor("Codex", "scan", SymbolKind::Fn), "Codex#scan().");
        assert_eq!(
            descriptor("", "VerifyFailure", SymbolKind::Type),
            "VerifyFailure#"
        );
        assert_eq!(descriptor("", "mm_codex", SymbolKind::Module), "mm_codex#");
        assert_eq!(
            descriptor("", "MAX_MODULES", SymbolKind::Const),
            "MAX_MODULES."
        );
        assert_eq!(
            descriptor("", "system.boot_plan", SymbolKind::Interface),
            "system.boot_plan()."
        );
    }

    #[test]
    fn a_descriptor_carries_no_offset_so_a_move_cannot_change_it() {
        // The same definition before and after being moved down a file and
        // reformatted produces byte-identical descriptors: nothing positional is
        // in the input, so nothing positional can be in the output.
        let before = descriptor("Codex", "scan", SymbolKind::Fn);
        let after = descriptor("Codex", "scan", SymbolKind::Fn);
        assert_eq!(before, after);
        assert!(
            before.chars().all(|c| !c.is_ascii_digit()),
            "a descriptor must not encode a line or byte offset: {before}"
        );
    }

    #[test]
    fn renaming_or_rescoping_changes_the_symbol_iri() {
        let path = "crates/mm-codex/src/symbol.rs";
        let original = symbol_iri(path, &descriptor("Codex", "scan", SymbolKind::Fn));
        let renamed = symbol_iri(path, &descriptor("Codex", "scan_all", SymbolKind::Fn));
        let rescoped = symbol_iri(path, &descriptor("CodexBuilder", "scan", SymbolKind::Fn));
        assert_ne!(original, renamed);
        assert_ne!(original, rescoped);
        assert_eq!(
            original.as_str(),
            "https://metamind.dev/code/symbol/crates/mm-codex/src/symbol.rs#Codex%23scan()."
        );
    }

    #[test]
    fn local_names_are_recovered_from_descriptors() {
        assert_eq!(local_name("scan()."), "scan");
        assert_eq!(local_name("Codex#scan()."), "scan");
        assert_eq!(local_name("GraphStore/Graph#sparql()."), "sparql");
        assert_eq!(local_name("VerifyFailure#"), "VerifyFailure");
        assert_eq!(local_name("MAX."), "MAX");
        assert_eq!(local_name("tests#"), "tests");
        // Round-trips against the builder for every kind.
        for kind in [
            SymbolKind::Fn,
            SymbolKind::Type,
            SymbolKind::Trait,
            SymbolKind::Module,
            SymbolKind::Const,
            SymbolKind::Interface,
        ] {
            assert_eq!(local_name(&descriptor("Scope", "thing", kind)), "thing");
            assert_eq!(local_name(&descriptor("", "thing", kind)), "thing");
        }
    }

    #[test]
    fn unsafe_descriptors_are_rejected() {
        assert!(is_valid_descriptor("scan()."));
        assert!(
            is_valid_descriptor("Codex#scan()."),
            "# is the scope separator and is escaped for the IRI fragment"
        );
        assert!(!is_valid_descriptor(""));
        assert!(!is_valid_descriptor("has space"));
        assert!(!is_valid_descriptor("tab\there"));
        assert!(!is_valid_descriptor("na\u{ef}ve"));
        assert!(
            !is_valid_descriptor("From<io::Error>#from()."),
            "a `<` would break the IRI"
        );
        assert!(!is_valid_descriptor("a\"b()."));
    }
}
