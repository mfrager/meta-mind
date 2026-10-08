//! Module behavior test: the module loads, its manifest agrees with its binary,
//! and every surface states the contract it must respect.
//!
//! These assertions are what make `mm:BeingState` a *tested* capability rather
//! than a declared one: the code graph records them as `mmc:hasTest` for the
//! capability, and `codex verify` refuses a capability that has none.

#[test]
fn the_module_loads_and_answers_every_tbox_function() {
    let manifest = being::manifest().expect("the embedded manifest must parse");
    assert_eq!(manifest.metadata.owned_by_phase, 4);
    assert_eq!(manifest.metadata.capability, "mm:BeingState");
    assert_eq!(
        manifest.tbox.functions.len(),
        being::SURFACE_FUNCTIONS.len()
    );

    for function in being::SURFACE_FUNCTIONS {
        assert!(
            manifest.tbox.functions.contains_key(function),
            "{function} must be declared in plugin.toml"
        );
    }
    let surfaces = being::surfaces();
    assert_eq!(surfaces.len(), being::SURFACE_FUNCTIONS.len());
}

#[test]
fn affect_writes_no_fact_bearing_surface() {
    let affect = being::affect();
    // The separation rule, stated as data: nothing about an impulse can be read
    // back as a claim, and the surfaces it touches are affect only.
    for table in affect.writes {
        assert!(
            table.starts_with("affect_"),
            "affect must not write `{table}`"
        );
    }
    assert!(
        affect.contract.contains("always false"),
        "{}",
        affect.contract
    );
}

#[test]
fn the_invariant_bearing_surfaces_name_their_invariant() {
    let by_function: std::collections::BTreeMap<&str, being::Surface> = being::surfaces()
        .into_iter()
        .map(|s| (s.function, s))
        .collect();

    assert_eq!(
        by_function[being::IDENTITY].invariant,
        Some("no_history_rewrite")
    );
    assert_eq!(
        by_function[being::BELIEFS].invariant,
        Some("no_assumption_to_observation")
    );
    assert_eq!(
        by_function[being::GOALS].invariant,
        Some("no_history_rewrite")
    );
    // Affect and relationships are control/derived state: they hold no invariant
    // because they assert nothing.
    assert_eq!(by_function[being::AFFECT].invariant, None);
    assert_eq!(by_function[being::RELATIONSHIPS].invariant, None);
}

#[test]
fn every_surface_declares_at_least_one_table_it_reads() {
    for surface in being::surfaces() {
        assert!(
            !surface.reads.is_empty(),
            "{} declares no reads",
            surface.function
        );
        assert!(
            !surface.contract.is_empty(),
            "{} declares no contract",
            surface.function
        );
    }
}
