//! The tier ladder: how much cognition a situation justifies.
//!
//! Two ideas from the phase's research foundation meet here. From the
//! inference-time-scaling taxonomy, a tier selects a **compute policy** — a set of
//! allowed operation classes, a search width, and a budget shape — rather than a
//! fixed script. From the budget-forcing work, the policy is what bounds spending
//! so the controller stops the moment more cognition would not change the
//! decision.
//!
//! The selector is a pure function of four caller-supplied numbers: stakes,
//! uncertainty, irreversibility and novelty. Nothing here observes the world,
//! consults a model, or reads a clock, so `Tier::select` has a boundary golden
//! table rather than a tolerance.
//!
//! Time pressure is deliberately *not* one of the four factors: it does not change
//! how hard a problem is, only how much of the answer has to wait. It caps the
//! tier, which is the honest way to say "whatever this deserves, there is no time
//! for it".

use serde::{Deserialize, Serialize};

use crate::episode::CognitiveEpisode;
use crate::error::{MetacogError, Result};
use crate::op::{OpClass, OP_CLASSES};
use crate::scan::ScanResult;

/// The compute ladder. `T0` is the least cognition that can still decide.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Tier {
    /// Recall and decide. The default: an episode that has not selected a tier yet
    /// is at the least cognition that can still decide.
    #[default]
    T0,
    /// Plus classification and a cheap comparison.
    T1,
    /// Plus assumption checks and precedent.
    T2,
    /// Plus verification, critique and inversion.
    T3,
    /// Plus refinement, decomposition and causality.
    T4,
    /// Plus search, simulation and adversarial critique.
    T5,
}

/// Every tier, cheapest first.
pub const TIERS: [Tier; 6] = [Tier::T0, Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];

/// The score at which each tier begins. Five boundaries cut the `[0,1]` score
/// into six bands.
pub const TIER_THRESHOLDS: [f64; 5] = [0.15, 0.30, 0.45, 0.60, 0.80];

/// How many listed uncertainties saturate the uncertainty factor. Four is the
/// point past which "more things are uncertain" stops meaning "harder".
pub const UNCERTAINTY_SATURATION: f64 = 4.0;

/// The weight of each factor in the selector's score. They sum to 1, so the score
/// is in `[0,1]` and every band is reachable.
pub const SELECTOR_WEIGHTS: [f64; 4] = [0.35, 0.25, 0.25, 0.15];

impl Tier {
    /// Every tier, cheapest first.
    pub const ALL: [Tier; 6] = TIERS;

    /// The tier's number, matching the `episodes.tier` column.
    pub fn as_u8(self) -> u8 {
        self.index() as u8
    }

    /// The tier's position in [`TIERS`].
    pub fn index(self) -> usize {
        match self {
            Tier::T0 => 0,
            Tier::T1 => 1,
            Tier::T2 => 2,
            Tier::T3 => 3,
            Tier::T4 => 4,
            Tier::T5 => 5,
        }
    }

    /// Parse a tier number.
    pub fn from_u8(n: u8) -> Option<Self> {
        TIERS.get(n as usize).copied()
    }

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T0 => "T0",
            Tier::T1 => "T1",
            Tier::T2 => "T2",
            Tier::T3 => "T3",
            Tier::T4 => "T4",
            Tier::T5 => "T5",
        }
    }

    /// Parse a wire name, accepting `T3` and `3`.
    pub fn parse(text: &str) -> Option<Self> {
        let bare = text.strip_prefix(['T', 't']).unwrap_or(text);
        bare.parse::<u8>().ok().and_then(Tier::from_u8)
    }

    /// Select the tier a situation justifies.
    ///
    /// `score = 0.35 * stakes + 0.25 * uncertainty + 0.25 * irreversibility +
    /// 0.15 * novelty`, all factors clamped into `[0,1]`, and then the score picks
    /// a band from [`TIER_THRESHOLDS`]. Time pressure caps the result: at 0.75 the
    /// answer may not go above `T2`, at 0.90 not above `T1`.
    pub fn select(episode: &CognitiveEpisode, scan: &ScanResult) -> Tier {
        let stakes = clamp_unit(scan.stakes);
        let irreversibility = clamp_unit(scan.irreversibility);
        let novelty = clamp_unit(episode.context.novelty);
        let uncertainty = uncertainty_factor(episode, scan);

        let score = SELECTOR_WEIGHTS[0] * stakes
            + SELECTOR_WEIGHTS[1] * uncertainty
            + SELECTOR_WEIGHTS[2] * irreversibility
            + SELECTOR_WEIGHTS[3] * novelty;

        let mut tier = Tier::T0;
        for (index, threshold) in TIER_THRESHOLDS.iter().enumerate() {
            if score >= *threshold {
                tier = TIERS[index + 1];
            }
        }
        tier.min(Tier::pressure_cap(episode.context.time_pressure))
    }

    /// The highest tier a given time pressure allows.
    pub fn pressure_cap(time_pressure: f64) -> Tier {
        if time_pressure >= 0.90 {
            Tier::T1
        } else if time_pressure >= 0.75 {
            Tier::T2
        } else {
            Tier::T5
        }
    }

    /// The compute policy this tier selects.
    ///
    /// The allowed sets are cumulative, which is what makes the ladder monotone:
    /// a harder problem may always use everything an easier one could.
    pub fn policy(self) -> ComputePolicy {
        ComputePolicy {
            tier: self,
            allowed_mask: allowed_mask(self),
            search_width: match self {
                Tier::T0 | Tier::T1 | Tier::T2 => 0,
                Tier::T3 => 2,
                Tier::T4 => 3,
                Tier::T5 => 5,
            },
            max_ops: match self {
                Tier::T0 => 2,
                Tier::T1 => 4,
                Tier::T2 => 7,
                Tier::T3 => 11,
                Tier::T4 => 16,
                Tier::T5 => 24,
            },
            cost_multiplier: match self {
                Tier::T0 => 0.5,
                Tier::T1 => 0.75,
                Tier::T2 => 1.0,
                Tier::T3 => 1.5,
                Tier::T4 => 2.0,
                Tier::T5 => 3.0,
            },
        }
    }

    /// The classes this tier forbids that the tier below allowed, for reporting.
    pub fn added_by(self) -> Vec<OpClass> {
        let lower = TIERS
            .get(self.index().wrapping_sub(1))
            .copied()
            .filter(|t| *t < self)
            .map(|t| t.policy());
        let here = self.policy();
        here.allowed_classes()
            .into_iter()
            .filter(|class| lower.is_none_or(|lower| !lower.allows(*class)))
            .collect()
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which operation classes each tier allows, as a bitmask over [`OP_CLASSES`].
fn allowed_mask(tier: Tier) -> u32 {
    let mut mask = 0u32;
    for class in allowed_classes(tier) {
        mask |= 1 << class.index();
    }
    mask
}

/// The classes each tier allows, in [`OP_CLASSES`] order.
pub fn allowed_classes(tier: Tier) -> Vec<OpClass> {
    let names: &[OpClass] = match tier {
        Tier::T0 => &[OpClass::Observe, OpClass::Recall, OpClass::Decide],
        Tier::T1 => &[
            OpClass::Observe,
            OpClass::Recall,
            OpClass::Classify,
            OpClass::Compare,
            OpClass::CheckConstraints,
            OpClass::Simplify,
            OpClass::Decide,
        ],
        Tier::T2 => &[
            OpClass::Observe,
            OpClass::Recall,
            OpClass::Clarify,
            OpClass::Classify,
            OpClass::Compare,
            OpClass::SearchPrecedent,
            OpClass::CheckConstraints,
            OpClass::CheckAssumptions,
            OpClass::Simplify,
            OpClass::Synthesize,
            OpClass::Decide,
        ],
        Tier::T3 => &[
            OpClass::Observe,
            OpClass::Recall,
            OpClass::Clarify,
            OpClass::Classify,
            OpClass::Compare,
            OpClass::SearchPrecedent,
            OpClass::Invert,
            OpClass::Predict,
            OpClass::Verify,
            OpClass::Critique,
            OpClass::CheckConstraints,
            OpClass::CheckAssumptions,
            OpClass::Simplify,
            OpClass::Synthesize,
            OpClass::Decide,
        ],
        Tier::T4 => &[
            OpClass::Observe,
            OpClass::Recall,
            OpClass::Clarify,
            OpClass::Classify,
            OpClass::Decompose,
            OpClass::Compare,
            OpClass::Analogize,
            OpClass::SearchPrecedent,
            OpClass::Invert,
            OpClass::Predict,
            OpClass::Verify,
            OpClass::Critique,
            OpClass::Refine,
            OpClass::CheckConstraints,
            OpClass::CheckAssumptions,
            OpClass::CheckContradictions,
            OpClass::CheckCausality,
            OpClass::Simplify,
            OpClass::Synthesize,
            OpClass::Decide,
            OpClass::Measure,
        ],
        Tier::T5 => &OP_CLASSES,
    };
    names.to_vec()
}

/// The compute policy a tier selects.
///
/// `allowed_mask` is a bitmask over [`OP_CLASSES`]: bit *i* set means that class is
/// allowed. A mask rather than a vector because the policy is `Copy` — it is
/// passed to the forcer on every iteration — and because a policy's identity is
/// its allowed set, which a mask states exactly.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputePolicy {
    /// The tier this policy belongs to.
    pub tier: Tier,
    /// Bit *i* set means [`OP_CLASSES`]`[i]` is allowed.
    pub allowed_mask: u32,
    /// How wide a `Search` may branch. Zero means search is forbidden in practice.
    pub search_width: u8,
    /// The operation ceiling this tier grants.
    pub max_ops: u32,
    /// What the tier multiplies a `CostClass` by.
    pub cost_multiplier: f64,
}

impl ComputePolicy {
    /// True when this class may be compiled at this tier.
    pub fn allows(&self, class: OpClass) -> bool {
        self.allowed_mask & (1 << class.index()) != 0
    }

    /// The allowed classes, in [`OP_CLASSES`] order.
    pub fn allowed_classes(&self) -> Vec<OpClass> {
        OP_CLASSES
            .into_iter()
            .filter(|class| self.allows(*class))
            .collect()
    }

    /// True when the operation's class may be compiled.
    pub fn allows_op(&self, op: &crate::op::CognitiveOp) -> bool {
        self.allows(op.class())
    }

    /// The cost band's price under this policy.
    pub fn price_of(&self, class: crate::op::CostClass) -> f64 {
        class.cost() * self.cost_multiplier
    }

    /// A stable rendering, for the `metacog.tier.select` log field.
    pub fn canonical(&self) -> String {
        format!(
            "tier={},allowed=[{}],search_width={},max_ops={},cost_multiplier={:.2}",
            self.tier.as_str(),
            self.allowed_classes()
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(","),
            self.search_width,
            self.max_ops,
            self.cost_multiplier
        )
    }

    /// Refuse a policy that cannot bound anything.
    pub fn validate(&self) -> Result<()> {
        if self.max_ops == 0 {
            return Err(MetacogError::validation(
                "policy.max_ops",
                "must be at least 1: a tier that forbids every operation is not a tier",
            ));
        }
        if !self.cost_multiplier.is_finite() || self.cost_multiplier <= 0.0 {
            return Err(MetacogError::validation(
                "policy.cost_multiplier",
                format!("must be finite and positive, got {}", self.cost_multiplier),
            ));
        }
        Ok(())
    }
}

/// The uncertainty factor the selector and the loop share.
///
/// It is the larger of the scan's reported magnitudes and the fraction of the
/// saturation point that the episode's own list of open questions reaches, so a
/// caller that names its uncertainties without a scan is still heard. The loop
/// starts from the same number, which is why it lives here rather than in either.
pub fn uncertainty_factor(episode: &CognitiveEpisode, scan: &ScanResult) -> f64 {
    let listed = clamp_unit(episode.uncertainties.len() as f64 / UNCERTAINTY_SATURATION);
    clamp_unit(scan.mean_uncertainty()).max(listed)
}

/// Clamp a caller-supplied factor into `[0,1]`, mapping a non-finite value to `0`.
///
/// A `NaN` novelty is not "extremely novel", it is an unusable input; propagating
/// it would make the tier depend on a caller bug in a way no test could pin.
fn clamp_unit(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::CognitiveBudget;
    use crate::episode::{Context, Timescale};
    use crate::scan::{ScanIssue, ScanResult, Uncertainty};
    use std::collections::BTreeSet;

    fn episode(novelty: f64, time_pressure: f64, uncertainties: usize) -> CognitiveEpisode {
        let mut ep = CognitiveEpisode::new(
            mm_core::Ulid::from_parts(1, 1),
            mm_core::Ulid::from_parts(1, 2),
            Context::new("decide"),
            CognitiveBudget::from_spec(10, 5, 1.0, 1000).unwrap(),
        )
        .unwrap();
        // Set the caller-supplied factors after construction: `Context::validate`
        // refuses a non-finite novelty, and the point of one test below is that the
        // selector survives one that arrived anyway.
        ep.context.novelty = novelty;
        ep.context.time_pressure = time_pressure;
        ep.uncertainties = (0..uncertainties).map(|i| format!("u{i}")).collect();
        assert_eq!(ep.timescale, Timescale::Minutes);
        ep
    }

    fn scan(stakes: f64, irreversibility: f64, uncertainty: f64) -> ScanResult {
        ScanResult {
            stakes,
            irreversibility,
            uncertainties: if uncertainty > 0.0 {
                vec![Uncertainty {
                    id: "u".into(),
                    description: "d".into(),
                    magnitude: uncertainty,
                }]
            } else {
                Vec::new()
            },
            ..ScanResult::default()
        }
    }

    #[test]
    fn tiers_round_trip_and_the_ladder_is_monotone() {
        for tier in TIERS {
            assert_eq!(Tier::from_u8(tier.as_u8()), Some(tier));
            assert_eq!(Tier::parse(tier.as_str()), Some(tier));
            assert_eq!(Tier::parse(&tier.as_u8().to_string()), Some(tier));
        }
        assert_eq!(Tier::parse("T9"), None);
        assert_eq!(Tier::default(), Tier::T0);

        // Every tier allows everything the tier below it allows.
        for pair in TIERS.windows(2) {
            let (lower, higher) = (pair[0], pair[1]);
            let low = lower.policy().allowed_classes();
            let high: BTreeSet<OpClass> = higher.policy().allowed_classes().into_iter().collect();
            for class in low {
                assert!(high.contains(&class), "{higher} drops {class} from {lower}");
            }
            assert!(higher.policy().max_ops >= lower.policy().max_ops);
            assert!(higher.policy().cost_multiplier >= lower.policy().cost_multiplier);
            assert!(higher.policy().search_width >= lower.policy().search_width);
        }
        // `T5` allows every class, so no class is unreachable from some tier.
        assert_eq!(Tier::T5.policy().allowed_classes().len(), OP_CLASSES.len());
    }

    #[test]
    fn the_boundary_table_is_a_golden() {
        // (stakes, irreversibility, uncertainty, novelty, expected tier)
        let table: &[(f64, f64, f64, f64, Tier)] = &[
            (0.0, 0.0, 0.0, 0.0, Tier::T0),
            (0.5, 0.0, 0.0, 0.0, Tier::T1),
            (0.5, 0.5, 0.0, 0.0, Tier::T2),
            (0.6, 0.6, 0.6, 0.0, Tier::T3),
            (0.8, 0.8, 0.8, 0.4, Tier::T4),
            (1.0, 1.0, 1.0, 1.0, Tier::T5),
        ];
        for (stakes, irreversibility, uncertainty, novelty, expected) in table {
            let ep = episode(*novelty, 0.0, 0);
            let scan = scan(*stakes, *irreversibility, *uncertainty);
            assert_eq!(
                Tier::select(&ep, &scan),
                *expected,
                "stakes={stakes} irreversibility={irreversibility} \
                 uncertainty={uncertainty} novelty={novelty}"
            );
        }
    }

    #[test]
    fn the_selector_is_monotone_in_every_factor() {
        let base = Tier::select(&episode(0.0, 0.0, 0), &scan(0.2, 0.2, 0.0));
        for (novelty, scan) in [
            (0.5, scan(0.2, 0.2, 0.0)),
            (0.0, scan(0.8, 0.2, 0.0)),
            (0.0, scan(0.2, 0.8, 0.0)),
            (0.0, scan(0.2, 0.2, 0.8)),
        ] {
            let raised = Tier::select(&episode(novelty, 0.0, 0), &scan);
            assert!(raised >= base, "raising a factor lowered the tier");
        }
        // Uncertainty may also arrive as a list of open questions.
        let listed = Tier::select(&episode(0.0, 0.0, 4), &scan(0.0, 0.0, 0.0));
        assert!(listed > Tier::T0);
    }

    #[test]
    fn time_pressure_caps_the_tier_but_never_raises_it() {
        let hard = scan(1.0, 1.0, 1.0);
        assert_eq!(Tier::select(&episode(1.0, 0.0, 0), &hard), Tier::T5);
        assert_eq!(Tier::select(&episode(1.0, 0.8, 0), &hard), Tier::T2);
        assert_eq!(Tier::select(&episode(1.0, 0.95, 0), &hard), Tier::T1);

        let easy = scan(0.0, 0.0, 0.0);
        assert_eq!(Tier::select(&episode(0.0, 0.95, 0), &easy), Tier::T0);
        assert_eq!(Tier::pressure_cap(0.0), Tier::T5);
    }

    #[test]
    fn a_caller_supplied_nan_does_not_propagate() {
        // A `NaN` novelty is an unusable input, not "extremely novel", so it weighs
        // nothing; a non-finite irreversibility clamps to its ceiling.
        assert_eq!(
            Tier::select(&episode(f64::NAN, 0.0, 0), &scan(0.0, 0.0, 0.0)),
            Tier::T0
        );
        let mut weird = scan(f64::NAN, f64::INFINITY, 0.0);
        weird.uncertainties.clear();
        assert_eq!(Tier::select(&episode(0.0, 0.0, 0), &weird), Tier::T1);
        let selected = Tier::select(&episode(f64::NAN, f64::NAN, 0), &weird);
        assert!(selected <= Tier::T5, "the selector must stay total");
    }

    #[test]
    fn every_tier_adds_something_over_the_one_below() {
        for tier in &TIERS[1..] {
            assert!(
                !tier.added_by().is_empty(),
                "{tier} adds nothing over the tier below it"
            );
        }
        // `T0` has nothing below it, so everything it allows is something it adds.
        assert_eq!(
            Tier::T0.added_by().len(),
            Tier::T0.policy().allowed_classes().len()
        );
    }

    #[test]
    fn a_policy_states_its_allowed_set_and_refuses_a_meaningless_one() {
        let policy = Tier::T3.policy();
        assert!(policy.allows(OpClass::Verify));
        assert!(!policy.allows(OpClass::Search));
        assert!(policy.allows_op(&crate::op::CognitiveOp::Verify {
            claim: mm_core::Ulid::from_parts(1, 3)
        }));
        assert!(policy.canonical().contains("verify"));
        assert_eq!(policy.price_of(crate::op::CostClass::Moderate), 0.05 * 1.5);
        policy.validate().unwrap();

        let mut broken = policy;
        broken.max_ops = 0;
        assert!(broken.validate().is_err());
        broken.max_ops = 11;
        broken.cost_multiplier = 0.0;
        assert!(broken.validate().is_err());
    }

    #[test]
    fn a_scan_issue_is_not_needed_for_the_selector_but_is_recorded() {
        // The selector reads the aggregates, not the issue list; a scan that names
        // issues without raising a factor must not change the tier.
        let with_issues = ScanResult {
            issues: vec![ScanIssue {
                kind: "ambiguous".into(),
                materiality: 0.9,
                target: "x".into(),
            }],
            stakes: 0.1,
            ..ScanResult::default()
        };
        assert_eq!(
            Tier::select(&episode(0.0, 0.0, 0), &with_issues),
            Tier::select(&episode(0.0, 0.0, 0), &ScanResult::default())
        );
    }
}
