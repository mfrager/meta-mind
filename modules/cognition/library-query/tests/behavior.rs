//! Module behavior test: the module loads, its manifest agrees with its binary,
//! and every surface states the contract it must respect.
//!
//! These assertions are what make `mm:TechniqueSelection` a *tested* capability
//! rather than a declared one: the code graph records them as `mmc:hasTest` for the
//! capability, and `codex verify` refuses a capability that has none.

#[test]
fn the_module_loads_and_answers_every_tbox_function() {
    let manifest = library_query::manifest().expect("the embedded manifest must parse");
    assert_eq!(manifest.metadata.owned_by_phase, 7);
    assert_eq!(manifest.metadata.capability, "mm:TechniqueSelection");
    assert_eq!(
        manifest.tbox.functions.len(),
        library_query::SURFACE_FUNCTIONS.len()
    );
    for function in library_query::SURFACE_FUNCTIONS {
        assert!(
            manifest.tbox.functions.contains_key(function),
            "{function} must be declared in plugin.toml"
        );
    }
    assert_eq!(
        library_query::surfaces().len(),
        library_query::SURFACE_FUNCTIONS.len()
    );
}

#[test]
fn skill_retrieval_never_returns_an_unverified_skill() {
    // Voyager's rule, made a contract: a skill that has not passed its test is a
    // draft, and a draft is not retrieved however well it scores.
    let skill = library_query::skill_retrieve();
    assert_eq!(skill.invariant, Some("no_unverified_skill"));
    assert!(
        skill.contract.contains("Only a Verified skill"),
        "{}",
        skill.contract
    );
    assert!(skill.contract.contains("Draft") && skill.contract.contains("Failing"));
    // Retrieval is a read.
    assert!(skill.writes.is_empty());
    assert_eq!(skill.reads, &["skills"]);
}

#[test]
fn selection_is_ranked_and_never_a_default() {
    let applicable = library_query::library_applicable();
    assert_eq!(applicable.invariant, Some("no_silent_default_technique"));
    assert!(
        applicable.contract.contains("never a default"),
        "{}",
        applicable.contract
    );
    // The only table it may write is the record of the ranking it returned.
    assert_eq!(applicable.writes, &["applicability_runs"]);
    assert!(applicable.reads.contains(&"library_entries"));
}

#[test]
fn case_retrieval_is_structural_and_every_surface_states_its_contract() {
    let by_function: std::collections::BTreeMap<&str, library_query::Surface> =
        library_query::surfaces()
            .into_iter()
            .map(|s| (s.function, s))
            .collect();

    let cases = &by_function[library_query::CASE_RETRIEVE];
    assert!(
        cases.contract.contains("structure-mapped"),
        "{}",
        cases.contract
    );
    assert!(
        cases.contract.contains("case_utility"),
        "{}",
        cases.contract
    );
    // Retrieval explains itself from stored correspondences and writes nothing.
    assert_eq!(cases.reads, &["cases", "case_map"]);
    assert!(cases.writes.is_empty());
    // A retrieval asserts no invariant beyond reading what it ranked.
    assert_eq!(cases.invariant, None);

    for function in library_query::SURFACE_FUNCTIONS {
        let surface = library_query::surfaces()
            .into_iter()
            .find(|s| s.function == function)
            .expect("every function has a surface");
        assert!(!surface.reads.is_empty(), "{function} declares no reads");
        assert!(
            !surface.contract.is_empty(),
            "{function} declares no contract"
        );
    }
}
