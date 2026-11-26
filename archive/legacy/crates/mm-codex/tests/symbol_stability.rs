//! Symbol identity is stable under edits that do not rename the symbol.
//!
//! This is the property Phase 2 §7 names explicitly, and it is the whole reason a
//! symbol's identifier is a *semantic* descriptor (scope + item name + kind) rather
//! than a byte offset or a line number:
//!
//! * **Moving** a definition within its file must not change its identity, or a
//!   later phase's map of the code would rot every time a line was inserted;
//! * **Reordering** items and **reformatting** are non-events for a reader, so they
//!   must be non-events for the graph;
//! * **Renaming** and **rescoping** are real changes and must produce a new
//!   identity, or two different definitions would share one symbol;
//! * A **file's module path** is part of the scope, because Rust files are modules
//!   and two files in one crate may each define a top-level `verify`.
//!
//! Byte ranges are still recorded (`mmc:byteStart`/`byteEnd`) but they are for
//! navigation only: they move freely, and nothing that identifies a symbol is
//! derived from them.

use mm_codex::model::SymbolKind;
use mm_codex::parse::{file_module_scope, parse_rust_in};
use mm_codex::symbol::{descriptor, local_name};
use mm_core::codex::symbol_iri;
use proptest::prelude::*;

const PATH: &str = "crates/demo/src/lib.rs";

/// The descriptors a source produces, sorted.
fn descriptors(source: &str) -> Vec<String> {
    let mut out: Vec<String> = parse_rust_in(source, "")
        .expect("the fixture source must parse")
        .items
        .into_iter()
        .map(|item| item.descriptor)
        .collect();
    out.sort();
    out
}

/// descriptor -> symbol IRI, for comparing identities across two renderings.
fn identities(source: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = parse_rust_in(source, "")
        .expect("the fixture source must parse")
        .items
        .into_iter()
        .map(|item| {
            let iri = symbol_iri(PATH, &item.descriptor).as_str().to_string();
            (item.descriptor, iri)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn reordering_items_and_reformatting_keep_every_identity() {
    let spread_out = "
impl T {
    fn a(&self) {}
    fn b(&self) {}
}
";
    let squeezed = "impl T{fn b(&self){}
fn a(&self){}}\n";

    assert_eq!(
        descriptors(spread_out),
        descriptors(squeezed),
        "whitespace and item order must not change the descriptor set"
    );
    assert_eq!(descriptors(spread_out), vec!["T#a().", "T#b()."]);
    assert_eq!(
        identities(spread_out),
        identities(squeezed),
        "the symbol IRIs must be equal too, not merely the descriptors"
    );
}

#[test]
fn moving_a_definition_within_its_file_keeps_its_identity() {
    let at_the_top = "fn solo() {}\n";
    let moved_down = "// The same definition, now preceded by a doc comment and\n// several blank lines, so every byte offset below it shifts.\n\n\n\nfn solo() {}\n";

    let first = parse_rust_in(at_the_top, "").expect("source parses");
    let second = parse_rust_in(moved_down, "").expect("source parses");
    let a = &first.items[0];
    let b = &second.items[0];

    assert_eq!(a.descriptor, "solo().");
    assert_eq!(
        a.descriptor, b.descriptor,
        "a move must not change the descriptor"
    );
    assert_ne!(
        a.byte_start, b.byte_start,
        "the fixture must actually move the definition"
    );
    assert_eq!(
        symbol_iri(PATH, &a.descriptor),
        symbol_iri(PATH, &b.descriptor),
        "a move must not change the symbol IRI"
    );
}

#[test]
fn renaming_changes_the_descriptor_and_the_iri() {
    let before = descriptor("Codex", "scan", SymbolKind::Fn);
    let renamed = descriptor("Codex", "scan_all", SymbolKind::Fn);
    assert_ne!(before, renamed);
    assert_ne!(
        symbol_iri(PATH, &before),
        symbol_iri(PATH, &renamed),
        "a renamed definition is a different symbol"
    );
}

#[test]
fn rescoping_changes_the_descriptor_and_the_iri() {
    let here = descriptor("Codex", "scan", SymbolKind::Fn);
    let elsewhere = descriptor("CodexBuilder", "scan", SymbolKind::Fn);
    assert_ne!(here, elsewhere);
    assert_ne!(
        symbol_iri(PATH, &here),
        symbol_iri(PATH, &elsewhere),
        "the same method on a different type is a different symbol"
    );
}

#[test]
fn a_descriptor_survives_the_iri_as_exactly_one_fragment_separator() {
    let scoped = descriptor("T", "a", SymbolKind::Fn);
    assert_eq!(scoped, "T#a().");
    let iri = symbol_iri(PATH, &scoped);
    assert_eq!(
        iri.as_str().matches('#').count(),
        1,
        "the IRI must carry exactly one `#`: the fragment separator. Got {iri}"
    );
    assert!(
        iri.as_str().contains("T%23a()."),
        "a scope separator inside the descriptor must be percent-encoded: {iri}"
    );

    let unscoped = descriptor("", "solo", SymbolKind::Fn);
    let iri = symbol_iri(PATH, &unscoped);
    assert!(
        !iri.as_str().contains("%23"),
        "an unscoped descriptor needs no encoding: {iri}"
    );
    assert_eq!(iri.as_str().matches('#').count(), 1, "{iri}");
}

#[test]
fn two_files_of_one_crate_do_not_share_a_top_level_descriptor() {
    assert_eq!(
        file_module_scope("crates/x/src/logs_cmd.rs", "crates/x"),
        "logs_cmd"
    );
    assert_eq!(
        file_module_scope("crates/x/src/doctor.rs", "crates/x"),
        "doctor"
    );
    assert_eq!(file_module_scope("crates/x/src/lib.rs", "crates/x"), "");
    assert_eq!(file_module_scope("crates/x/src/main.rs", "crates/x"), "");
    assert_eq!(file_module_scope("crates/x/src/a/b.rs", "crates/x"), "a#b");
    assert_eq!(file_module_scope("crates/x/src/a/mod.rs", "crates/x"), "a");
    assert_eq!(
        file_module_scope("crates/x/tests/smoke.rs", "crates/x"),
        "tests#smoke"
    );

    let source = "pub fn verify() {}\n";
    let logs = parse_rust_in(
        source,
        &file_module_scope("crates/x/src/logs_cmd.rs", "crates/x"),
    )
    .expect("source parses");
    let doctor = parse_rust_in(
        source,
        &file_module_scope("crates/x/src/doctor.rs", "crates/x"),
    )
    .expect("source parses");
    assert_eq!(logs.items[0].descriptor, "logs_cmd#verify().");
    assert_eq!(doctor.items[0].descriptor, "doctor#verify().");
    assert_ne!(logs.items[0].descriptor, doctor.items[0].descriptor);

    // The crate root stays unscoped, so `src/lib.rs` reads the way the plan's
    // example does (`case_match().`, not `lib#case_match().`).
    let root = parse_rust_in(
        source,
        &file_module_scope("crates/x/src/lib.rs", "crates/x"),
    )
    .expect("source parses");
    assert_eq!(root.items[0].descriptor, "verify().");
}

fn identifier_strategy() -> impl Strategy<Value = String> {
    // A legal Rust identifier: a leading ASCII letter, then letters, digits, or
    // underscores. Kept inside ASCII so the descriptor stays an IRI fragment.
    prop::collection::vec(
        prop::sample::select(vec!['a', 'b', 'z', 'A', 'Z', '_', '0', '9', 'x', 'y']),
        1..16,
    )
    .prop_map(|tail| format!("n{}", tail.into_iter().collect::<String>()))
}

proptest! {
    #[test]
    fn a_descriptor_round_trips_back_to_its_name(name in identifier_strategy()) {
        for kind in [
            SymbolKind::Module,
            SymbolKind::Trait,
            SymbolKind::Fn,
            SymbolKind::Type,
            SymbolKind::Const,
            SymbolKind::Interface,
        ] {
            let unscoped = descriptor("", &name, kind);
            prop_assert_eq!(local_name(&unscoped), name.as_str());

            let scoped = descriptor("Scope", &name, kind);
            prop_assert_eq!(local_name(&scoped), name.as_str());

            // The IRI is derived from the descriptor and carries it intact.
            let iri = symbol_iri(PATH, &scoped);
            prop_assert_eq!(iri.as_str().matches('#').count(), 1);
            prop_assert!(iri.as_str().starts_with("https://metamind.dev/code/symbol/"));
        }
    }

    #[test]
    fn distinct_names_yield_distinct_descriptors(
        a in identifier_strategy(),
        b in identifier_strategy(),
    ) {
        prop_assume!(a != b);
        prop_assert_ne!(descriptor("", &a, SymbolKind::Fn), descriptor("", &b, SymbolKind::Fn));
        prop_assert_ne!(
            symbol_iri(PATH, &descriptor("", &a, SymbolKind::Fn)),
            symbol_iri(PATH, &descriptor("", &b, SymbolKind::Fn))
        );
    }
}
