//! Module smoke test: the module loads, its manifest agrees with its binary, and
//! its T-Box function answers.

#[test]
fn module_loads_and_answers_its_tbox_function() {
    let manifest = kernel_bootstrap::manifest().expect("the embedded manifest must parse");

    assert_eq!(manifest.plugin.name, "kernel-bootstrap");
    assert_eq!(manifest.metadata.owned_by_phase, 1);
    assert!(
        manifest
            .tbox
            .functions
            .contains_key(kernel_bootstrap::BOOT_PLAN),
        "the module must declare its T-Box function"
    );

    let plan = kernel_bootstrap::boot_plan(true);
    assert!(plan.stores_ready);
    assert!(!plan.steps.is_empty());
}

#[test]
fn manifest_uri_is_stable_across_reads() {
    let first = kernel_bootstrap::manifest().unwrap().plugin.uri;
    let second = kernel_bootstrap::manifest_at(&kernel_bootstrap::manifest_path())
        .unwrap()
        .plugin
        .uri;
    assert_eq!(first, second);
    assert!(first.starts_with("https://metamind.dev/code/module/"));
}
