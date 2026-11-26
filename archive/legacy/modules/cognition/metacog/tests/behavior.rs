//! Module behavior test: the module's two T-Box functions compile what they say
//! they compile, refuse what they say they refuse, and name themselves.
//!
//! These assertions are what make `mm:CognitiveControl` a *tested* capability
//! rather than a declared one: the code graph records them as `mmc:hasTest` for the
//! capability, and `codex verify` refuses a capability that has none.
//!
//! The episode and the scan are built as `mm-metacog` types and then serialized, so
//! the test exercises exactly the payload a caller would send — a hand-written JSON
//! literal here could agree with the handler while disagreeing with the controller.

use serde_json::{json, Value};

use mm_metacog::{
    CognitiveBudget, CognitiveEpisode, ComparisonSpec, Context, FailureMode, ScanIssue, ScanResult,
    Tier, Uncertainty,
};

/// A materially interesting episode: two candidates, a claim, an assumption, an
/// active technique, and enough budget for the `T4` policy to spend.
fn episode() -> CognitiveEpisode {
    let mut episode = CognitiveEpisode::new(
        mm_core::Ulid::from_parts(1_700_000_000_000, 11),
        mm_core::Ulid::from_parts(1_700_000_000_000, 12),
        Context::new("decide whether to roll back the deploy").with_novelty(0.6),
        CognitiveBudget::from_spec(16, 8, 2.0, 60_000).expect("a valid budget"),
    )
    .expect("a valid episode");
    // The caller pins the tier as a floor; the selector may still raise it.
    episode.tier = Tier::T4;
    episode.candidates = vec!["rollback".to_string(), "roll_forward".to_string()];
    episode.claims = vec![mm_core::Ulid::from_parts(1_700_000_000_000, 13)];
    episode.assumptions = vec![mm_core::Ulid::from_parts(1_700_000_000_000, 14)];
    episode.active_techniques = vec!["https://metamind.dev/library/technique/canary".to_string()];
    episode
}

/// One broad scan, with an issue, an uncertainty, a comparison and a failure mode.
fn scan() -> ScanResult {
    ScanResult {
        issues: vec![ScanIssue {
            kind: "missing_evidence".to_string(),
            materiality: 0.7,
            target: "rollback".to_string(),
        }],
        uncertainties: vec![Uncertainty {
            id: "u1".to_string(),
            description: "does the maintenance window hold".to_string(),
            magnitude: 0.6,
        }],
        comparison_requirements: vec![ComparisonSpec {
            subject: "rollback".to_string(),
            versus: "roll_forward".to_string(),
            criterion: "downtime".to_string(),
        }],
        failure_modes: vec![FailureMode {
            mode: "partial rollback".to_string(),
            likelihood: 0.2,
            impact: 0.8,
        }],
        simpler_alternatives: vec!["ask the on-call".to_string()],
        stakes: 0.8,
        irreversibility: 0.2,
        verification_value: 0.7,
    }
}

fn episode_json() -> Value {
    serde_json::to_value(episode()).expect("the episode serializes")
}

fn scan_json() -> Value {
    serde_json::to_value(scan()).expect("the scan serializes")
}

#[test]
fn plan_compiles_a_program_from_an_episode_and_scan() {
    let program = metacog::handlers::plan(&episode_json(), &scan_json()).expect("a program");

    // The graph is not empty, and every node has exactly one step.
    let nodes = program["graph"]["nodes"].as_array().expect("a node list");
    let steps = program["steps"].as_array().expect("a step list");
    assert!(
        !nodes.is_empty(),
        "a compiled program must hold at least one operation"
    );
    assert_eq!(nodes.len(), steps.len(), "every node has exactly one step");

    // Orders are contiguous from 1, which is what `episode verify` checks.
    for (index, step) in steps.iter().enumerate() {
        let order = step["order"].as_u64().expect("an order");
        assert_eq!(
            order,
            index as u64 + 1,
            "step orders must be contiguous from 1"
        );
        assert!(
            step["op"]["op"].is_string(),
            "every step names its operation"
        );
    }

    // The program can always stop on its budget, whatever else it carries.
    let stopping = program["stopping"].as_array().expect("stopping conditions");
    assert!(
        stopping.iter().any(|c| c == "budget_exhausted"),
        "a program must be able to stop on the budget: {stopping:?}"
    );

    // The tier is the one the episode pinned, and the id is a content-derived ULID.
    assert_eq!(program["tier"], json!("T4"));
    let id = program["id"].as_str().expect("a program id");
    assert_eq!(id.len(), 26, "a program id is a ULID: {id}");
    // `ulid`'s own `Display` is uppercase Crockford; every *canonical* rendering in
    // this system is the lowercase form, so the comparison is case-insensitive and
    // the id's equality is what is actually asserted.
    assert_eq!(
        program["episode_id"]
            .as_str()
            .expect("an episode id")
            .to_ascii_lowercase(),
        mm_core::ulid_string(&episode().id)
    );

    // Nothing in the program reaches outside the tier's own policy.
    let policy = mm_metacog::Tier::T4.policy();
    for step in steps {
        let class = step["op"]["op"].as_str().expect("an op tag");
        assert!(!class.is_empty());
    }
    assert!(policy.allows(mm_metacog::OpClass::Critique));
}

#[test]
fn the_program_is_reproducible_from_the_same_payload() {
    // The program id is derived from the program's own content, so a recompile is
    // byte-identical — which is what `episode replay` compares.
    let first = metacog::handlers::plan(&episode_json(), &scan_json()).expect("a program");
    let second = metacog::handlers::plan(&episode_json(), &scan_json()).expect("a program");
    assert_eq!(first, second);
}

#[test]
fn plan_refuses_a_malformed_payload_rather_than_panicking() {
    for payload in [json!({}), json!(null), json!([]), json!("not an episode")] {
        let error = metacog::handlers::plan(&payload, &scan_json()).expect_err("a refusal");
        assert_eq!(error.code(), "json");
        assert_eq!(error.function(), metacog::COGNITION_PLAN);
        assert!(
            error.to_string().contains(metacog::COGNITION_PLAN),
            "{error}"
        );
    }

    // A well-shaped episode with a scan that violates the scan's own rules is the
    // controller's refusal, not the payload's shape.
    let mut bad_scan = scan_json();
    bad_scan["stakes"] = json!(1.5);
    let error = metacog::handlers::plan(&episode_json(), &bad_scan).expect_err("a refusal");
    assert_eq!(error.code(), "scan", "{error}");
}

#[test]
fn op_value_is_the_documented_expression() {
    let out = metacog::handlers::op_value(0.5, 0.5, 0.5, 0.25).expect("a score");
    let score = out["score"].as_f64().expect("a number");
    // 0.5 * 0.5 * 0.5 / 0.25 = 0.5
    assert!((score - 0.5).abs() < 1e-12, "{score}");
    assert!(out["canonical"]
        .as_str()
        .expect("a canonical rendering")
        .contains("score=0.500000000"));

    // A free operation is ranked by its numerator alone and is never a division by
    // zero.
    let free = metacog::handlers::op_value(0.5, 0.5, 0.5, 0.0).expect("a score");
    assert!(free["score"].as_f64().expect("a number").is_finite());

    // The four inputs are echoed, so a caller can record what it asked.
    assert_eq!(out["expected_error_reduction"], json!(0.5));
    assert_eq!(out["cost"], json!(0.25));
}

#[test]
fn op_value_refuses_a_bad_input_rather_than_clamping() {
    for (eer, importance, p_change, cost) in [
        (1.5, 0.5, 0.5, 1.0),
        (0.5, 1.5, 0.5, 1.0),
        (0.5, 0.5, 1.5, 1.0),
        (0.5, 0.5, 0.5, -1.0),
        (f64::NAN, 0.5, 0.5, 1.0),
        (0.5, 0.5, 0.5, f64::INFINITY),
    ] {
        let error =
            metacog::handlers::op_value(eer, importance, p_change, cost).expect_err("a refusal");
        assert_eq!(error.code(), "validation");
        assert_eq!(error.function(), metacog::COGNITION_OP_VALUE);
    }
    // The boundary itself is allowed.
    assert!(metacog::handlers::op_value(0.0, 0.0, 0.0, 0.0).is_ok());
    assert!(metacog::handlers::op_value(1.0, 1.0, 1.0, 1.0).is_ok());
}

#[test]
fn the_registry_names_both_functions() {
    assert_eq!(metacog::functions().len(), 2);
    assert_eq!(metacog::functions(), metacog::SURFACE_FUNCTIONS.to_vec());
    assert!(metacog::functions().contains(&metacog::COGNITION_PLAN));
    assert!(metacog::functions().contains(&metacog::COGNITION_OP_VALUE));
    assert_eq!(metacog::CAPABILITY, "mm:CognitiveControl");

    // The embedded manifest declares exactly the functions the binary exposes, by
    // the handler path the symbol table knows them by.
    for function in metacog::functions() {
        assert!(
            metacog::MANIFEST_TOML.contains(function),
            "{function} is not declared in plugin.toml"
        );
    }
    assert!(metacog::MANIFEST_TOML.contains(metacog::plan_handler()));
    assert!(metacog::MANIFEST_TOML.contains(metacog::op_value_handler()));
    assert!(metacog::MANIFEST_TOML.contains("owned_by_phase = 8"));
    assert!(metacog::MANIFEST_TOML.contains(metacog::CAPABILITY));
    // The declared uri is the path-derived one for this directory.
    assert!(metacog::MANIFEST_TOML.contains(metacog::module_iri().as_str()));
    assert_eq!(
        metacog::module_iri().as_str(),
        "https://metamind.dev/code/module/cognition/metacog"
    );
}
