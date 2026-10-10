//! Module behavior test: the module loads, its manifest agrees with its binary,
//! and every surface states the contract it must respect.
//!
//! These assertions are what make `mm:FrameActivation` a *tested* capability
//! rather than a declared one: the code graph records them as `mmc:hasTest` for
//! the capability, and `codex verify` refuses a capability that has none.

#[test]
fn the_module_loads_and_answers_every_tbox_function() {
    let manifest = frame_activate::manifest().expect("the embedded manifest must parse");
    assert_eq!(manifest.metadata.owned_by_phase, 7);
    assert_eq!(manifest.metadata.capability, "mm:FrameActivation");
    assert_eq!(
        manifest.tbox.functions.len(),
        frame_activate::SURFACE_FUNCTIONS.len()
    );
    for function in frame_activate::SURFACE_FUNCTIONS {
        assert!(
            manifest.tbox.functions.contains_key(function),
            "{function} must be declared in plugin.toml"
        );
    }
    assert_eq!(
        frame_activate::surfaces().len(),
        frame_activate::SURFACE_FUNCTIONS.len()
    );
}

#[test]
fn composition_names_its_parents_and_refuses_an_undeclared_frame() {
    let compose = frame_activate::frame_compose();
    // A composed instance is built *from* bases, so it must say which. A frame that
    // is not already in the library cannot be one of them.
    assert_eq!(compose.invariant, Some("no_undeclared_parent"));
    assert_eq!(compose.writes, &["frame_instances"]);
    assert!(
        compose.reads.contains(&"library_entries"),
        "a frame cannot be activated from nowhere; compose must read the library"
    );
    assert!(
        compose.contract.contains("refuses a frame"),
        "{}",
        compose.contract
    );
}

#[test]
fn a_gap_is_reported_and_never_filled() {
    let missing = frame_activate::frame_missing();
    // Reading a gap must not write: reporting a hole is not the same as repairing it.
    assert!(
        missing.writes.is_empty(),
        "frame_missing must write nothing"
    );
    assert_eq!(missing.invariant, None);
    assert!(
        missing.contract.contains("never fabricated"),
        "{}",
        missing.contract
    );
}

#[test]
fn an_activation_change_is_always_explicable() {
    let switch = frame_activate::frame_switch();
    assert_eq!(switch.invariant, Some("no_frame_switch_without_reason"));
    assert!(
        switch.contract.contains("the reason"),
        "{}",
        switch.contract
    );
    assert!(switch.reads.contains(&"frame_instances"));
    assert!(switch.writes.contains(&"frame_instances"));
}

#[test]
fn every_surface_declares_a_read_and_a_contract() {
    for function in frame_activate::SURFACE_FUNCTIONS {
        let surface = frame_activate::surfaces()
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
