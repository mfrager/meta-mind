//! The Rust semantic analyzer.
//!
//! This pass covers the three facts a concrete syntax tree cannot give cleanly:
//! an item's **attributes** (is this a T-Box function?), its **visibility**
//! among a body's items, and the **roots of its `use` paths** (which are what
//! turn into cross-crate dependency edges).
//!
//! `syn` is a real Rust parser, so an unparseable file is a hard error here. That
//! is deliberate and is the mitigation the phase plan calls for: a scanner that
//! silently skips a file it cannot read reports a codebase that does not exist.

use std::collections::BTreeSet;

use mm_core::MmError;
use syn::{Attribute, Item, UseTree};

/// The attribute that marks a function as a T-Box function.
pub const TBOX_ATTR: &str = "tbox_fn";

/// What the analyzer found in one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    /// Names of functions declared `#[tbox_fn]`.
    pub tbox_functions: BTreeSet<String>,
    /// Names of `pub` traits.
    pub public_traits: BTreeSet<String>,
    /// The roots of every `use` path in the file: crate names, plus `crate`,
    /// `self`, and `super`.
    pub use_roots: BTreeSet<String>,
}

impl Analysis {
    /// True when `name` is a T-Box function declared in this file.
    pub fn is_tbox_function(&self, name: &str) -> bool {
        self.tbox_functions.contains(name)
    }
}

/// True when the file extension is Rust.
pub fn is_rust_file(rel_path: &str) -> bool {
    rel_path.ends_with(".rs")
}

/// Analyze one Rust source file.
pub fn analyze(source: &str, rel_path: &str) -> Result<Analysis, MmError> {
    let file = syn::parse_file(source)
        .map_err(|e| MmError::Codec(format!("{rel_path}: cannot parse Rust source: {e}")))?;
    let mut analysis = Analysis::default();
    for item in &file.items {
        walk(item, &mut analysis);
    }
    Ok(analysis)
}

fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

fn walk(item: &Item, out: &mut Analysis) {
    match item {
        Item::Fn(f) => {
            if has_attr(&f.attrs, TBOX_ATTR) {
                out.tbox_functions.insert(f.sig.ident.to_string());
            }
        }
        Item::Trait(t) => {
            if matches!(t.vis, syn::Visibility::Public(_)) {
                out.public_traits.insert(t.ident.to_string());
            }
        }
        Item::Use(u) => collect_roots(&u.tree, &mut out.use_roots),
        Item::Mod(m) => {
            if let Some((_, items)) = &m.content {
                for inner in items {
                    walk(inner, out);
                }
            }
        }
        Item::Impl(i) => {
            for inner in &i.items {
                walk_impl_item(inner, out);
            }
        }
        _ => {}
    }
}

/// An impl block's members are a different type from top-level items, so an impl
/// needs its own walk rather than reusing [`walk`].
fn walk_impl_item(item: &syn::ImplItem, out: &mut Analysis) {
    if let syn::ImplItem::Fn(f) = item {
        if has_attr(&f.attrs, TBOX_ATTR) {
            out.tbox_functions.insert(f.sig.ident.to_string());
        }
    }
}

fn collect_roots(tree: &UseTree, out: &mut BTreeSet<String>) {
    match tree {
        UseTree::Path(p) => {
            out.insert(p.ident.to_string());
        }
        UseTree::Group(g) => {
            for inner in &g.items {
                collect_roots(inner, out);
            }
        }
        // `use ::foo` and `use foo` are the same root; a bare `*`/rename has none.
        UseTree::Name(_) | UseTree::Rename(_) | UseTree::Glob(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tbox_functions_are_found_by_attribute_however_the_fn_is_written() {
        let src = r#"
/// A handler the monad can address.
#[tbox_fn]
pub fn boot_plan(stores_ready: bool) -> Plan { todo!() }

pub fn helper() {}

impl Kernel {
    #[tbox_fn]
    fn internal(&self) {}
}
"#;
        let a = analyze(src, "modules/x/src/lib.rs").unwrap();
        assert!(a.is_tbox_function("boot_plan"));
        assert!(a.is_tbox_function("internal"));
        assert!(!a.is_tbox_function("helper"));
    }

    #[test]
    fn only_public_traits_are_interfaces() {
        let src = "pub trait Verifiable {}\ntrait Private {}\npub struct NotATrait;\n";
        let a = analyze(src, "x.rs").unwrap();
        assert!(a.public_traits.contains("Verifiable"));
        assert!(!a.public_traits.contains("Private"));
        assert!(!a.public_traits.contains("NotATrait"));
    }

    #[test]
    fn use_roots_include_crates_and_relative_roots() {
        let src = r#"
use std::collections::BTreeMap;
use mm_core::{iri, NamedNode};
use crate::model::SymbolKind;
use super::hash;
use self::inner;
use serde::{Deserialize as De, Serialize};
use std::io::*;
"#;
        let a = analyze(src, "crates/mm-codex/src/lib.rs").unwrap();
        for root in ["std", "mm_core", "crate", "super", "self", "serde"] {
            assert!(
                a.use_roots.contains(root),
                "missing {root} in {:?}",
                a.use_roots
            );
        }
    }

    #[test]
    fn a_nested_module_is_analyzed_too() {
        let src = "mod inner {\n    #[tbox_fn]\n    pub fn deep() {}\n    pub trait Deep {}\n}\n";
        let a = analyze(src, "x.rs").unwrap();
        assert!(a.is_tbox_function("deep"));
        assert!(a.public_traits.contains("Deep"));
    }

    #[test]
    fn an_unparseable_file_is_a_hard_error() {
        let err = analyze("fn broken( {", "crates/x/src/bad.rs").unwrap_err();
        assert!(matches!(err, MmError::Codec(_)), "got {err:?}");
        assert!(err.to_string().contains("crates/x/src/bad.rs"), "{err}");
    }

    #[test]
    fn rust_files_are_recognized_by_extension() {
        assert!(is_rust_file("src/lib.rs"));
        assert!(!is_rust_file("src/lib.rs.orig"));
        assert!(!is_rust_file("Cargo.toml"));
    }
}
