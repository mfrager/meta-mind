//! The parse layer: tree-sitter concrete syntax trees.
//!
//! Tree-sitter is used for the three things a concrete syntax tree is good at and
//! a semantic pass is not:
//!
//! * **byte ranges** for every definition, so `mmc:byteStart`/`byteEnd` are exact
//!   without any offset arithmetic on top of a line/column API;
//! * **error tolerance** — a file that does not parse is reported as such rather
//!   than silently contributing nothing;
//! * **incrementality** — the scanner parses only files whose content hash
//!   changed, which is the granularity that actually matters here.
//!
//! Descriptors deliberately do not come from this pass' offsets: see
//! [`crate::symbol`]. Offsets are recorded for navigation only.

use std::collections::HashSet;

use mm_core::MmError;
use tree_sitter::{Node, Parser};

use crate::model::SymbolKind;
use crate::symbol;

/// One definition found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedItem {
    /// The definition's name.
    pub name: String,
    /// The SCIP-style descriptor, scoped by its enclosing items.
    pub descriptor: String,
    /// What kind of definition it is.
    pub kind: SymbolKind,
    /// Whether it carries a `pub` visibility modifier.
    pub is_public: bool,
    /// Whether it is a test.
    pub is_test: bool,
    /// First byte of the definition.
    pub byte_start: usize,
    /// One past the last byte of the definition.
    pub byte_end: usize,
}

/// Everything the parse layer reports about one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    /// `rust`, `toml`, ...
    pub language: String,
    /// True when tree-sitter recovered from a syntax error.
    pub has_errors: bool,
    /// Every definition, in byte order.
    pub items: Vec<ParsedItem>,
    /// Every identifier occurrence that is **not** a definition's own name, as
    /// `(name, byte_start)`. Resolving these to definitions is the scanner's job.
    pub occurrences: Vec<(String, usize)>,
}

/// The language label for a file, or `None` when the scanner does not index it.
pub fn language_of(rel_path: &str) -> Option<&'static str> {
    let ext = rel_path.rsplit('.').next()?;
    if ext == rel_path {
        return None;
    }
    Some(match ext {
        "rs" => "rust",
        "toml" => "toml",
        "sql" => "sql",
        "ttl" => "turtle",
        "json" => "json",
        "jsonl" => "jsonl",
        "md" => "markdown",
        _ => return None,
    })
}

/// True when a file with this name is part of the code index.
pub fn is_indexed(rel_path: &str) -> bool {
    language_of(rel_path).is_some()
}

/// The module path a source file contributes inside its crate.
///
/// Rust files are modules, so two files in one crate may each define a top-level
/// `verify` and neither is a duplicate. The descriptor scope therefore begins with
/// the file's own module path: `src/logs_cmd.rs` is `logs_cmd`, `src/lib.rs` and
/// `src/main.rs` are the crate root, and `src/a/b.rs` is `a#b`.
pub fn file_module_scope(file_rel: &str, module_dir: &str) -> String {
    let within = file_rel
        .strip_prefix(&format!("{}/", module_dir.trim_end_matches('/')))
        .unwrap_or(file_rel);
    let within = within.strip_prefix("src/").unwrap_or(within);
    let stem = within.strip_suffix(".rs").unwrap_or(within);
    if stem == "lib" || stem == "main" {
        return String::new();
    }
    stem.split('/')
        .filter(|part| !part.is_empty() && *part != "mod")
        .collect::<Vec<_>>()
        .join("#")
}

/// Parse a Rust source file whose top-level items live at the crate root.
pub fn parse_rust(source: &str) -> Result<ParsedFile, MmError> {
    parse_rust_in(source, "")
}

/// Parse a Rust source file, scoping its top-level items under `module_scope`.
pub fn parse_rust_in(source: &str, module_scope: &str) -> Result<ParsedFile, MmError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| MmError::Codec(format!("cannot load the rust grammar: {e}")))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| MmError::Codec("tree-sitter refused to parse the source".into()))?;
    let root = tree.root_node();

    let mut items = Vec::new();
    let mut def_names: HashSet<(usize, usize)> = HashSet::new();
    collect_items(
        root,
        source,
        module_scope,
        false,
        &mut items,
        &mut def_names,
    );
    items.sort_by_key(|i| (i.byte_start, i.byte_end));

    let mut occurrences = Vec::new();
    collect_occurrences(root, source, &def_names, &mut occurrences);
    occurrences.sort();

    Ok(ParsedFile {
        language: "rust".to_string(),
        has_errors: root.has_error(),
        items,
        occurrences,
    })
}

/// The byte range of a node's `name` field, when it has one.
fn name_node<'a>(node: Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("name")
}

fn text(node: Node<'_>, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or_default()
        .to_string()
}

/// Whether a node carries a visibility modifier (`pub`, `pub(crate)`, ...).
fn is_public(node: Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    let mut public = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" && text(child, source).starts_with("pub") {
            public = true;
            break;
        }
    }
    public
}

/// The identifier a type expression names, with generics and paths stripped.
///
/// `crate::scan::Codex<'a>` becomes `Codex`, so an impl block scopes its members
/// by the type's *name* and nothing positional.
fn type_name(node: Node<'_>, source: &str) -> String {
    let raw = text(node, source);
    let head = raw.split('<').next().unwrap_or(&raw);
    let tail = head.rsplit("::").next().unwrap_or(head);
    tail.trim().trim_start_matches('&').trim().to_string()
}

/// A trait reference reduced to a descriptor-safe name: the trait plus a compact
/// form of its arguments.
///
/// A crate may write several `impl From<_> for MmError` blocks, and every one of
/// them defines a method named `from`. Scoping them all as `MmError/From` would
/// collide, so the arguments are folded in: `From<io::Error>` becomes
/// `From_io_Error`. Every character that could not appear in an IRI fragment is
/// collapsed to `_`, so the descriptor stays a legal IRI fragment and a legal
/// Turtle `<...>` term.
fn trait_descriptor(node: Node<'_>, source: &str) -> String {
    let mut out = String::new();
    let mut pending_colon = false;
    for ch in text(node, source).chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => {
                out.push(ch);
                pending_colon = false;
            }
            ':' => {
                if pending_colon {
                    out.push('_');
                }
                pending_colon = true;
            }
            _ => {
                if !out.ends_with('_') {
                    out.push('_');
                }
                pending_colon = false;
            }
        }
    }
    out.trim_matches('_').to_string()
}

fn scope_join(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{scope}#{name}")
    }
}

fn collect_items(
    node: Node<'_>,
    source: &str,
    scope: &str,
    in_test_mod: bool,
    items: &mut Vec<ParsedItem>,
    def_names: &mut HashSet<(usize, usize)>,
) {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    let mut pending_attrs: Vec<String> = Vec::new();

    for child in children {
        if child.kind() == "attribute_item" || child.kind() == "inner_attribute_item" {
            pending_attrs.push(text(child, source));
            continue;
        }
        let attrs = std::mem::take(&mut pending_attrs);
        let attr_text = attrs.join("\n");
        let has_test_attr = attr_text.contains("#[test]") || attr_text.contains("::test]");

        let kind = match child.kind() {
            "function_item" | "function_signature_item" => Some(SymbolKind::Fn),
            "struct_item" | "enum_item" | "union_item" | "type_item" => Some(SymbolKind::Type),
            "trait_item" => Some(SymbolKind::Trait),
            "mod_item" => Some(SymbolKind::Module),
            "const_item" | "static_item" | "macro_definition" => Some(SymbolKind::Const),
            _ => None,
        };

        if child.kind() == "impl_item" {
            // `impl Graph for GraphStore` and `impl GraphStore` can both define a
            // `sparql`, so a trait impl is scoped by `Type/Trait`. Without that the
            // two definitions would share one descriptor and the symbol table's
            // uniqueness constraint would be unsatisfiable.
            let inner_scope = match child.child_by_field_name("type") {
                Some(t) => {
                    let ty = type_name(t, source);
                    match child.child_by_field_name("trait") {
                        Some(tr) => {
                            scope_join(scope, &format!("{ty}/{}", trait_descriptor(tr, source)))
                        }
                        None => scope_join(scope, &ty),
                    }
                }
                None => scope.to_string(),
            };
            // An impl's members sit inside a `declaration_list` body, not among
            // the impl's own children, so the body is what gets walked.
            if let Some(body) = child.child_by_field_name("body") {
                collect_items(body, source, &inner_scope, in_test_mod, items, def_names);
            }
            continue;
        }

        if let Some(kind) = kind {
            let name_text = name_node(child).map(|n| text(n, source));
            if let Some(named) = name_text.clone() {
                let descriptor = symbol::descriptor(scope, &named, kind);
                if let Some(name) = name_node(child) {
                    def_names.insert((name.start_byte(), name.end_byte()));
                }
                items.push(ParsedItem {
                    name: named,
                    descriptor,
                    kind,
                    is_public: is_public(child, source),
                    is_test: in_test_mod || has_test_attr,
                    byte_start: child.start_byte(),
                    byte_end: child.end_byte(),
                });
            }
            // Some bodies open a new scope: a module nests its items, and a trait
            // nests its method signatures. A *function* body does not, so it is
            // deliberately not recursed into — a `fn` inside a `fn` is not part
            // of the module's interface.
            if let Some(body) = child.child_by_field_name("body") {
                let name = name_text.unwrap_or_default();
                match child.kind() {
                    "mod_item" => {
                        let inner_test =
                            in_test_mod || name == "tests" || attr_text.contains("cfg(test)");
                        collect_items(
                            body,
                            source,
                            &scope_join(scope, &name),
                            inner_test,
                            items,
                            def_names,
                        );
                    }
                    "trait_item" => {
                        collect_items(
                            body,
                            source,
                            &scope_join(scope, &name),
                            in_test_mod,
                            items,
                            def_names,
                        );
                    }
                    _ => {}
                }
            }
            continue;
        }
    }
}

fn collect_occurrences(
    node: Node<'_>,
    source: &str,
    def_names: &HashSet<(usize, usize)>,
    out: &mut Vec<(String, usize)>,
) {
    if node.kind() == "identifier" || node.kind() == "type_identifier" {
        let range = (node.start_byte(), node.end_byte());
        if !def_names.contains(&range) {
            let text = text(node, source);
            if !text.is_empty() {
                out.push((text, node.start_byte()));
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_occurrences(child, source, def_names, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
use std::path::Path;

pub const MAX: usize = 4;

#[derive(Debug)]
pub struct Codex {
    root: PathBuf,
}

impl Codex {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    fn scan(&self) -> usize {
        MAX
    }
}

pub trait Verifiable {
    fn rule(&self) -> &'static str;
}

mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
"#;

    fn descriptors(source: &str) -> Vec<String> {
        parse_rust(source)
            .unwrap()
            .items
            .iter()
            .map(|i| i.descriptor.clone())
            .collect()
    }

    #[test]
    fn items_are_found_with_scip_descriptors() {
        let d = descriptors(SAMPLE);
        for expected in [
            "MAX.",
            "Codex#",
            "Codex#new().",
            "Codex#scan().",
            "Verifiable#",
            "Verifiable#rule().",
            "tests#",
            "tests#it_works().",
        ] {
            assert!(
                d.contains(&expected.to_string()),
                "missing {expected} in {d:?}"
            );
        }
    }

    #[test]
    fn nothing_is_found_for_a_scoped_macro() {
        // `#[derive(Debug)]` is an attribute, not a definition.
        assert!(!descriptors(SAMPLE).contains(&"Debug#".to_string()));
    }

    #[test]
    fn visibility_and_tests_are_recorded() {
        let parsed = parse_rust(SAMPLE).unwrap();
        let by = |d: &str| parsed.items.iter().find(|i| i.descriptor == d).unwrap();
        assert!(by("Codex#").is_public);
        assert!(by("Codex#new().").is_public);
        assert!(!by("Codex#scan().").is_public, "scan has no pub modifier");
        assert!(!by("Codex#scan().").is_test);
        assert!(
            by("tests#it_works().").is_test,
            "a member of `mod tests` is a test"
        );
        assert!(!by("Codex#new().").is_test);
    }

    #[test]
    fn byte_ranges_are_exact_and_ordered() {
        let parsed = parse_rust(SAMPLE).unwrap();
        let codex = parsed
            .items
            .iter()
            .find(|i| i.descriptor == "Codex#")
            .unwrap();
        let slice = &SAMPLE[codex.byte_start..codex.byte_end];
        assert!(slice.starts_with("pub struct Codex"), "got {slice:?}");
        assert!(slice.ends_with('}'), "got {slice:?}");
        let mut starts: Vec<usize> = parsed.items.iter().map(|i| i.byte_start).collect();
        let sorted = starts.clone();
        starts.sort_unstable();
        assert_eq!(starts, sorted, "items must be reported in byte order");

        for item in &parsed.items {
            assert!(item.byte_start < item.byte_end, "{item:?}");
            assert!(item.byte_end <= SAMPLE.len(), "{item:?}");
        }
    }

    #[test]
    fn descriptors_survive_reordering_and_reformatting() {
        // The same three items, written in a different order and with different
        // whitespace, must yield the same descriptor set.
        let a = "impl T {\n    fn a(&self) {}\n    fn b(&self) {}\n}\n";
        let b = "impl T{fn b(&self){}\nfn a(&self){}}\n";
        let mut da = descriptors(a);
        let mut db = descriptors(b);
        da.sort();
        db.sort();
        assert_eq!(da, db);
        assert_eq!(da, vec!["T#a().", "T#b()."]);
    }

    #[test]
    fn a_trait_impl_is_scoped_by_its_trait_so_names_cannot_collide() {
        let src = r#"
impl GraphStore {
    pub async fn sparql(&self) {}
}
impl Graph for GraphStore {
    async fn sparql(&self) {}
}
"#;
        let d = descriptors(src);
        assert!(d.contains(&"GraphStore#sparql().".to_string()), "{d:?}");
        assert!(
            d.contains(&"GraphStore/Graph#sparql().".to_string()),
            "{d:?}"
        );
        let unique: std::collections::HashSet<_> = d.iter().collect();
        assert_eq!(unique.len(), d.len(), "descriptors must be unique: {d:?}");
    }

    #[test]
    fn several_trait_impls_of_one_trait_do_not_collide() {
        // Every `impl From<_> for MmError` defines `from`; the argument is folded
        // into the scope so the descriptors stay unique.
        let src = r#"
impl From<io::Error> for MmError {
    fn from(e: io::Error) -> Self { Self::Store(e.to_string()) }
}
impl From<sqlx::Error> for MmError {
    fn from(e: sqlx::Error) -> Self { Self::Store(e.to_string()) }
}
"#;
        let d = descriptors(src);
        assert!(
            d.contains(&"MmError/From_io_Error#from().".to_string()),
            "{d:?}"
        );
        assert!(
            d.contains(&"MmError/From_sqlx_Error#from().".to_string()),
            "{d:?}"
        );
        let unique: std::collections::HashSet<_> = d.iter().collect();
        assert_eq!(unique.len(), d.len(), "descriptors must be unique: {d:?}");
        // No descriptor may carry a character that would break a Turtle IRI term.
        // `(` and `)` are legal — they are the SCIP function suffix.
        for descriptor in &d {
            assert!(
                !descriptor.contains(['<', '>', '"', '{', '}', '|', '\\']),
                "{descriptor}"
            );
        }
    }

    #[test]
    fn a_definition_name_is_not_its_own_reference() {
        let parsed = parse_rust("fn solo() {}\n").unwrap();
        assert!(
            !parsed.occurrences.iter().any(|(n, _)| n == "solo"),
            "the definition's own name must not appear as an occurrence: {:?}",
            parsed.occurrences
        );
        let parsed = parse_rust("fn solo() { solo(); }\n").unwrap();
        assert!(
            parsed.occurrences.iter().any(|(n, _)| n == "solo"),
            "the call must appear as an occurrence"
        );
    }

    #[test]
    fn a_syntax_error_is_reported_not_swallowed() {
        let parsed = parse_rust("fn broken( {").unwrap();
        assert!(parsed.has_errors);
    }

    #[test]
    fn a_files_module_path_scopes_its_descriptors() {
        assert_eq!(
            file_module_scope("crates/mm-cli/src/logs_cmd.rs", "crates/mm-cli"),
            "logs_cmd"
        );
        assert_eq!(
            file_module_scope("crates/mm-cli/src/main.rs", "crates/mm-cli"),
            ""
        );
        assert_eq!(
            file_module_scope("crates/mm-core/src/lib.rs", "crates/mm-core"),
            ""
        );
        assert_eq!(file_module_scope("crates/x/src/a/b.rs", "crates/x"), "a#b");
        assert_eq!(file_module_scope("crates/x/src/a/mod.rs", "crates/x"), "a");
        assert_eq!(
            file_module_scope("crates/x/tests/smoke.rs", "crates/x"),
            "tests#smoke"
        );
        // Two files in one crate may each define `verify`; their scopes differ.
        let a = parse_rust_in("pub fn verify() {}\n", "logs_cmd");
        let b = parse_rust_in("pub fn verify() {}\n", "doctor");
        let da = a.unwrap().items[0].descriptor.clone();
        let db = b.unwrap().items[0].descriptor.clone();
        assert_eq!(da, "logs_cmd#verify().");
        assert_eq!(db, "doctor#verify().");
        assert_ne!(da, db);
    }

    #[test]
    fn items_in_the_crate_root_keep_unscoped_descriptors() {
        let parsed = parse_rust_in("pub fn verify() {}\n", "").unwrap();
        assert_eq!(parsed.items[0].descriptor, "verify().");
    }

    #[test]
    fn language_detection_covers_the_indexed_extensions() {
        assert_eq!(language_of("crates/mm-core/src/lib.rs"), Some("rust"));
        assert_eq!(language_of("crates/mm-core/Cargo.toml"), Some("toml"));
        assert_eq!(
            language_of("crates/mm-store-sqlite/migrations/0001_kernel.sql"),
            Some("sql")
        );
        assert_eq!(language_of("ontology/mm.ttl"), Some("turtle"));
        assert_eq!(language_of("modules/registry.json"), Some("json"));
        assert_eq!(
            language_of("codex.lock"),
            None,
            "a lock file is not indexed"
        );
        assert_eq!(language_of("target/debug/thing"), None);
        assert_eq!(language_of("Makefile"), None);
        assert!(!is_indexed("Cargo.lock"));
        assert!(is_indexed("crates/mm-core/src/lib.rs"));
    }
}
