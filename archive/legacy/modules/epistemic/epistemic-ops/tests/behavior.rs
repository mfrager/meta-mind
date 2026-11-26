//! Module behavior test: the module states the epistemic contract it must respect.
//!
//! These assertions are what make `mm:EpistemicOps` a *tested* capability rather
//! than a declared one: the code graph records them as `mmc:hasTest` for the
//! capability, and `codex verify` refuses a capability that has none.
//!
//! Unlike a crate that embeds its manifest with `include_str!`, this module's
//! `src/lib.rs` deliberately does not parse `plugin.toml` — that is the scanner's
//! job. So the checks here exercise the surfaces themselves and the one property
//! the module must keep in step with its directory: a path-derived URI.

use std::collections::BTreeMap;

#[test]
fn the_module_declares_exactly_the_three_epistemic_surfaces() {
    assert_eq!(
        epistemic_ops::functions(),
        [
            epistemic_ops::CLAIM,
            epistemic_ops::ASSUME,
            epistemic_ops::PREDICT
        ]
    );
    assert_eq!(
        epistemic_ops::surfaces().len(),
        epistemic_ops::functions().len()
    );
    for function in epistemic_ops::functions() {
        assert!(
            epistemic_ops::surfaces()
                .iter()
                .any(|surface| surface.function == function),
            "{function} must have a surface"
        );
    }
}

#[test]
fn the_uri_is_the_path_derived_form_for_this_directory() {
    // The manifest declares this value and the scanner checks it against the
    // directory it was found in; asserting it here keeps the two ends honest.
    assert_eq!(
        epistemic_ops::MODULE_URI,
        mm_core::iri::module("epistemic/epistemic-ops").as_str()
    );
}

#[test]
fn every_surface_names_one_argument_per_arity_and_a_handle_class() {
    for surface in epistemic_ops::surfaces() {
        assert!(surface.arity > 0, "{}", surface.function);
        assert_eq!(
            surface.arguments.len(),
            surface.arity,
            "{} must name one argument per arity",
            surface.function
        );
        assert!(!surface.effect.is_empty(), "{}", surface.function);
        assert!(epistemic_ops::handle_class(&surface).starts_with("mm:"));
    }
}

#[test]
fn the_claim_surface_refuses_a_silent_promotion() {
    // The whole point of the phase: a claim may not be recorded at a status its
    // kind did not earn. The surface must say so, because that is the guarantee a
    // caller wires a handler against.
    let by_function: BTreeMap<&str, epistemic_ops::Surface> = epistemic_ops::surfaces()
        .into_iter()
        .map(|surface| (surface.function, surface))
        .collect();

    let claim = &by_function[epistemic_ops::CLAIM];
    assert!(
        claim.effect.contains("silent promotion"),
        "{}",
        claim.effect
    );
    assert_eq!(epistemic_ops::handle_class(claim), "mm:Claim");

    // An assumption must state what being wrong costs and what checking costs.
    let assume = &by_function[epistemic_ops::ASSUME];
    assert_eq!(assume.arguments[3], "consequence_if_false");
    assert_eq!(assume.arguments[4], "verification_cost");
    assert_eq!(epistemic_ops::handle_class(assume), "mm:Assumption");

    // A prediction's probability is the caller's, never computed here.
    let predict = &by_function[epistemic_ops::PREDICT];
    assert_eq!(predict.arguments[3], "probability");
    assert_eq!(epistemic_ops::handle_class(predict), "mm:Prediction");
}
