//! Module behavior test: the module loads, its manifest agrees with its binary,
//! and every surface states the contract it must respect.
//!
//! These assertions are what make `mm:MemoryStore` a *tested* capability rather
//! than a declared one: the code graph records them as `mmc:hasTest` for the
//! capability, and `codex verify` refuses a capability that has none.

#[test]
fn the_module_loads_and_answers_every_tbox_function() {
    let manifest = memory_recall::manifest().expect("the embedded manifest must parse");
    assert_eq!(manifest.metadata.owned_by_phase, 5);
    assert_eq!(manifest.metadata.capability, "mm:MemoryStore");
    assert_eq!(
        manifest.tbox.functions.len(),
        memory_recall::SURFACE_FUNCTIONS.len()
    );
    for function in memory_recall::SURFACE_FUNCTIONS {
        assert!(
            manifest.tbox.functions.contains_key(function),
            "{function} must be declared in plugin.toml"
        );
    }
    assert_eq!(
        memory_recall::surfaces().len(),
        memory_recall::SURFACE_FUNCTIONS.len()
    );
}

#[test]
fn forgetting_is_declared_as_archive_never_delete() {
    let forget = memory_recall::forget();
    // The whole point of the phase: forgetting moves a record out of retrieval and
    // never removes it. The surface must say so, and must only ever write the
    // memories projection.
    assert!(
        forget.contract.contains("archives and never deletes"),
        "{}",
        forget.contract
    );
    assert!(forget.contract.contains("refused"), "{}", forget.contract);
    assert_eq!(forget.writes, &["memories"]);
    assert!(forget.reads.contains(&"commitments"));
}

#[test]
fn the_provenance_bearing_surfaces_name_their_invariant() {
    let by_function: std::collections::BTreeMap<&str, memory_recall::Surface> =
        memory_recall::surfaces()
            .into_iter()
            .map(|s| (s.function, s))
            .collect();

    assert_eq!(
        by_function[memory_recall::REMEMBER].invariant,
        Some("no_memory_without_provenance")
    );
    assert_eq!(
        by_function[memory_recall::FORGET].invariant,
        Some("no_fabricated_autobiography")
    );
    assert_eq!(
        by_function[memory_recall::MISTAKES].invariant,
        Some("no_action_without_evidence")
    );
    assert_eq!(
        by_function[memory_recall::CONSOLIDATE].invariant,
        Some("no_history_rewrite")
    );
    // Recall asserts nothing; it only reads and records accesses.
    assert_eq!(by_function[memory_recall::RECALL].invariant, None);
    assert_eq!(
        by_function[memory_recall::RECALL].writes,
        &["memory_access"]
    );
}

#[test]
fn every_memory_kind_has_a_surface_that_can_reach_it() {
    // The ten classes of design §32 are reachable through the module's surfaces:
    // episodes and the retrieval machinery through recall/remember, mistakes and
    // near misses through mistakes, procedural records through procedures,
    // summaries and communities through summaries.
    for function in memory_recall::SURFACE_FUNCTIONS {
        let surface = memory_recall::surfaces()
            .into_iter()
            .find(|s| s.function == function)
            .expect("every function has a surface");
        assert!(!surface.reads.is_empty(), "{function} declares no reads");
        assert!(
            !surface.contract.is_empty(),
            "{function} declares no contract"
        );
    }
    // A procedure becomes a habit only through repetition *and* proficiency, which
    // is the rule that keeps a single success from being remembered as a skill.
    let procedures = memory_recall::procedures();
    assert!(
        procedures.contract.contains("three successes") && procedures.contract.contains("0.8"),
        "{}",
        procedures.contract
    );
}
