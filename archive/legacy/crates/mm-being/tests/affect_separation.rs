//! Affect separation (plan §2, §8): affect biases behaviour and nothing else.
//!
//! The phase's headline invariant is that a simulated mood can never become a
//! fact or a reason. Three independent things are asserted here, because any one
//! of them alone could pass while the guarantee is broken elsewhere: the type has
//! no way to *say* an impulse affects state or reasoning, the schema refuses to
//! *store* one, and the `/being` projection never exposes affect under a
//! status-bearing predicate.

mod common;

use std::time::Duration;

use common::Harness;
use mm_being::{appraise, decay, Affect, AffectImpulse, Agency, AppraisalEvent};
use mm_core::{Params, Tabular};
use mm_log::codes;

/// An impulse carries the two flags false and offers no setter for either. The
/// fields are private with no constructor or `Deserialize`, so this is a proof by
/// construction: there is no API that could turn them on.
#[test]
fn an_impulse_flags_are_always_false_and_have_no_setter() {
    let events = [
        AppraisalEvent::new(0.6, -0.2, 0.3, 0.4, Agency::Other),
        AppraisalEvent::new(0.6, 0.2, 0.3, 0.4, Agency::Self_),
        AppraisalEvent::new(0.9, -0.9, 0.5, 0.1, Agency::Self_),
        AppraisalEvent::new(0.0, 0.0, 0.0, 0.0, Agency::Other),
    ];
    for event in events {
        let impulse = appraise(&event);
        assert!(
            !impulse.affects_internal_state(),
            "affect must never change state"
        );
        assert!(
            !impulse.affects_reasoning(),
            "affect must never change reasoning"
        );
    }
}

/// Re-scaling an impulse keeps the guarantee: `with_intensity` copies the flags
/// rather than being able to set them, and clamps the intensity into `[0, 1]`.
#[test]
fn with_intensity_clamps_and_never_touches_the_flags() {
    let impulse = appraise(&AppraisalEvent::new(0.6, -0.2, 0.3, 0.4, Agency::Other));
    let lowered: AffectImpulse = impulse.with_intensity(-1.0);
    assert!(lowered.intensity().abs() < f32::EPSILON);
    let raised = impulse.with_intensity(2.0);
    assert!((raised.intensity() - 1.0).abs() < f32::EPSILON);
    // Neither copy can claim to affect state or reasoning.
    for copy in [lowered, raised] {
        assert!(!copy.affects_internal_state());
        assert!(!copy.affects_reasoning());
    }
}

/// Through the facade: an appraisal moves the axes and mirrors the affect node,
/// and no axis is ever exposed under a predicate that could read as a fact.
#[tokio::test]
async fn an_appraisal_moves_the_axes_and_the_mirror_carries_no_status() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;
    let before = *facade.affect();

    facade
        .apply(mm_being::BeingOp::Appraise {
            event: AppraisalEvent::new(0.6, -0.2, 0.3, 0.4, Agency::Other),
        })
        .await
        .unwrap();

    let after = *facade.affect();
    let moved = (after.valence - before.valence).abs()
        + (after.arousal - before.arousal).abs()
        + (after.caution - before.caution).abs();
    assert!(moved > 0.0, "an appraisal must move at least one axis");

    // The impulse is audited, and the persisted affect state reflects it.
    assert!(h
        .audit_codes()
        .await
        .contains(&codes::BEING_AFFECT_IMPULSE.to_string()));
    assert_eq!(h.scalar("SELECT count(*) FROM affect_impulses").await, 1);

    let me = mm_core::ulid_string(&facade.identity().id);
    let affect_node = format!("https://metamind.dev/data/{me}#affect");
    let triples = h.graph.triples("being").await.unwrap();
    let affect_rows: Vec<&serde_json::Value> = triples
        .iter()
        .filter(|row| row["s"].as_str() == Some(affect_node.as_str()))
        .collect();
    assert!(
        !affect_rows.is_empty(),
        "the affect node must be mirrored into /being"
    );
    assert!(
        affect_rows
            .iter()
            .all(|row| row["p"].as_str().is_some_and(|p| !p.contains("status"))),
        "affect must never be mirrored under a status-bearing predicate"
    );

    h.shutdown().await;
}

/// Every stored impulse has both flags 0, and the schema refuses to make either
/// 1: the migration's `CHECK` pins them, so a bug in a higher layer cannot
/// promote a mood into state.
#[tokio::test]
async fn a_stored_impulse_cannot_be_made_to_affect_state_or_reasoning() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;
    facade
        .apply(mm_being::BeingOp::Appraise {
            event: AppraisalEvent::new(0.6, 0.2, 0.3, 0.4, Agency::Self_),
        })
        .await
        .unwrap();

    let rows = h
        .query("SELECT affects_internal_state, affects_reasoning FROM affect_impulses")
        .await;
    assert_eq!(rows.len(), 1);
    for row in &rows {
        assert_eq!(row["affects_internal_state"].as_i64(), Some(0));
        assert_eq!(row["affects_reasoning"].as_i64(), Some(0));
    }

    // Direct writes that a buggy layer might attempt are aborted by the schema.
    let state_write = Tabular::execute(
        &h.store,
        "UPDATE affect_impulses SET affects_internal_state = 1",
        Params::new(),
    )
    .await;
    assert!(state_write.is_err(), "the schema must refuse the flag");

    let reasoning_write = Tabular::execute(
        &h.store,
        "UPDATE affect_impulses SET affects_reasoning = 1",
        Params::new(),
    )
    .await;
    assert!(reasoning_write.is_err(), "the schema must refuse the flag");
    assert_eq!(
        h.scalar(
            "SELECT count(*) FROM affect_impulses \
             WHERE affects_internal_state <> 0 OR affects_reasoning <> 0"
        )
        .await,
        0
    );

    h.shutdown().await;
}

/// Decay relaxes the axes toward neutral and spends the impulse, and the affect
/// stays inside its range the whole way.
#[test]
fn decay_relaxes_the_axes_and_keeps_them_in_range() {
    let impulse = appraise(&AppraisalEvent::new(0.9, -0.6, 0.4, 0.2, Agency::Other));
    let mut affect = Affect::neutral();
    affect.apply(&impulse);
    let before = affect;
    assert!(before.valence < 0.0, "anger must move valence down");

    let mut impulses = vec![impulse];
    decay(&mut affect, &mut impulses, Duration::from_secs(120));

    assert!(
        affect.valence.abs() < before.valence.abs() + f32::EPSILON,
        "valence must move back toward neutral"
    );
    assert!(affect.valence.abs() < 0.01, "a long decay reaches neutral");
    assert!(impulses.is_empty(), "a spent impulse is dropped");
    for axis in [
        affect.valence,
        affect.arousal,
        affect.engagement,
        affect.warmth,
        affect.caution,
        affect.curiosity,
        affect.energy,
    ] {
        assert!((-1.0..=1.0).contains(&axis), "axis out of range: {axis}");
    }
}

/// `clamped()` is the last line of defence: no value, however far outside the
/// range, survives it.
#[test]
fn clamped_keeps_every_axis_in_range() {
    let wild = Affect {
        valence: 5.0,
        arousal: -9.0,
        engagement: 2.5,
        warmth: -1.5,
        caution: 0.25,
        curiosity: -0.75,
        energy: 100.0,
    };
    let clamped = wild.clamped();
    assert!((clamped.valence - 1.0).abs() < f32::EPSILON);
    assert!((clamped.arousal + 1.0).abs() < f32::EPSILON);
    assert!((clamped.engagement - 1.0).abs() < f32::EPSILON);
    assert!((clamped.warmth + 1.0).abs() < f32::EPSILON);
    assert!((clamped.energy - 1.0).abs() < f32::EPSILON);
    // A value already in range is unchanged.
    assert_eq!(
        Affect::neutral().clamped(),
        Affect::neutral(),
        "neutral is a fixed point"
    );
}
