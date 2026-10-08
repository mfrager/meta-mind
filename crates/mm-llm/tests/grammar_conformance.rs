//! Grammar conformance: a compiled grammar accepts what the schema accepts, and
//! nothing a permissive JSON grammar would wave through.
//!
//! Every fixture pairs a valid value with a *mutated* one — an extra field, an
//! out-of-range number, an empty array — that only the schema can reject. If the
//! grammar were a comment rather than a constraint, the mutated value would be
//! accepted and this test would fail.

mod common;

use std::path::PathBuf;

use common::fixture;
use mm_llm::client::{DecoderKind, SchemaId};
use mm_llm::grammar::{self, GrammarSpec};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct GrammarFixture {
    name: String,
    schema: serde_json::Value,
    #[serde(default)]
    valid: serde_json::Value,
    #[serde(default)]
    mutated: serde_json::Value,
    #[serde(default)]
    expect_compile_error: bool,
}

fn load_fixtures(dir: &str) -> Vec<GrammarFixture> {
    let root = fixture(dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            let raw = std::fs::read_to_string(path).unwrap();
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

fn compile_all(
    fixture: &GrammarFixture,
) -> Vec<(DecoderKind, Result<GrammarSpec, mm_llm::LlmError>)> {
    let id = SchemaId::new(fixture.name.clone());
    grammar::backends()
        .into_iter()
        .map(|backend| (backend.kind(), backend.compile(&id, &fixture.schema)))
        .collect()
}

#[test]
fn every_fixture_schema_compiles_and_separates_valid_from_mutated() {
    let fixtures = load_fixtures("bench/llm/grammar_fixtures");
    assert!(fixtures.len() >= 4, "the fixture set must not shrink");

    for f in &fixtures {
        let compiled = compile_all(f);
        assert_eq!(compiled.len(), 2, "both backends are exercised");

        if f.expect_compile_error {
            for (kind, outcome) in &compiled {
                let err = outcome
                    .as_ref()
                    .expect_err(&format!("{}: {kind:?} must refuse this schema", f.name));
                // The refusal names the construct, so the fix is obvious.
                assert!(err.to_string().contains("oneOf"), "{}: {err}", f.name);
            }
            continue;
        }

        for (kind, outcome) in &compiled {
            let spec = outcome
                .as_ref()
                .unwrap_or_else(|e| panic!("{}: {kind:?} cannot compile: {e}", f.name));
            assert_eq!(spec.decoder, *kind);
            assert_eq!(spec.schema_sha.len(), 64);
            assert_eq!(
                spec.fingerprint,
                mm_llm::schema::grammar_fingerprint(&spec.schema_sha, kind.as_str()),
                "the cache key is the schema hash and the decoder"
            );
            assert!(
                spec.accepts(&f.valid),
                "{}: {kind:?} rejects a valid sample",
                f.name
            );
            assert!(
                !spec.accepts(&f.mutated),
                "{}: {kind:?} accepts a mutated sample ({})",
                f.name,
                f.mutated
            );
            // A grammar that accepted arbitrary JSON would pass the line above but
            // fail here: a string is not this schema's object.
            assert!(!spec.accepts(&serde_json::json!("anything at all")));
        }
    }
}

#[test]
fn compilation_is_deterministic_and_the_dialects_differ() {
    let fixtures = load_fixtures("bench/llm/grammar_fixtures");
    for f in fixtures.iter().filter(|f| !f.expect_compile_error) {
        let id = SchemaId::new(f.name.clone());
        let a = grammar::compile(grammar::GrammarDialect::Llguidance, &id, &f.schema).unwrap();
        let b = grammar::compile(grammar::GrammarDialect::Llguidance, &id, &f.schema).unwrap();
        assert_eq!(
            a.grammar, b.grammar,
            "{}: compilation is deterministic",
            f.name
        );

        let x = grammar::compile(grammar::GrammarDialect::XGrammar, &id, &f.schema).unwrap();
        assert_ne!(
            a.grammar, x.grammar,
            "{}: dialects render differently",
            f.name
        );
        assert_ne!(a.fingerprint, x.fingerprint);
        assert_eq!(
            a.schema_sha, x.schema_sha,
            "the schema hash is dialect-free"
        );
    }
}

#[test]
fn a_grammar_is_invalidated_by_a_schema_change() {
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["score"],
        "properties": { "score": { "type": "integer", "minimum": 0, "maximum": 10 } }
    });
    let id = SchemaId::new("score.v1");
    let before = grammar::compile(grammar::GrammarDialect::Llguidance, &id, &schema).unwrap();

    let mut changed = schema.clone();
    changed["properties"]["score"]["maximum"] = serde_json::json!(99);
    let after = grammar::compile(grammar::GrammarDialect::Llguidance, &id, &changed).unwrap();

    assert_ne!(before.schema_sha, after.schema_sha);
    assert_ne!(
        before.grammar, after.grammar,
        "the bound is part of the grammar"
    );
    assert!(before.accepts(&serde_json::json!({"score": 10})));
    // Widening the bound changed the grammar: the value the first grammar refused
    // is the one the second admits.
    assert!(!before.accepts(&serde_json::json!({"score": 42})));
    assert!(after.accepts(&serde_json::json!({"score": 42})));
}

#[test]
fn a_compiled_grammar_is_golden() {
    let fixtures = load_fixtures("bench/llm/grammar_fixtures");
    let extract = fixtures
        .iter()
        .find(|f| f.name == "extract_v1")
        .expect("the extract fixture is present");
    let id = SchemaId::new("extract.v1");
    let spec = grammar::compile(grammar::GrammarDialect::Llguidance, &id, &extract.schema).unwrap();
    insta::assert_snapshot!("extract_v1_llguidance_grammar", spec.grammar);
}
