//! The promotion gate: the six ordered rules, and the decision written down.
//!
//! The rule the whole phase rests on is that **the gate decides, never the model**. So
//! the gate is arithmetic and policy over an [`EvidenceBundle`]: six rules, evaluated in
//! a fixed order, and the *first* one that fires supplies the reason. Order matters
//! because the reason is what an operator acts on: a candidate that breaks a regression
//! test is not made more interesting by also being slower, so the regression is named.
//!
//! The six rules, in order (plan §4.4):
//!
//! 1. any regression test failed;
//! 2. the candidate is worse than the baseline beyond the noise margin;
//! 3. the risk exceeds the evolution risk budget (or the policy's ceiling);
//! 4. a hard prohibition fires;
//! 5. required evidence is missing — no benchmark, or no rollback plan;
//! 6. the change touches identity invariants or the permission model.
//!
//! Three decisions are worth stating because they are not visible in the shape of the
//! code:
//!
//! * **The noise margin comes from the policy, not from the benchmark spec.** The spec
//!   records the margin it was run with; the *gate* re-derives the boundary from its own
//!   policy, so the margin the gate reports is the margin the gate used, and tightening
//!   the gate does not require editing a fixture.
//! * **A prohibition id is validated against the firewall's own table.** Rule 4 accepts
//!   the verdict `mm-cli` already obtained from `mm-firewall`, and looks the id up in
//!   `mm_firewall::prohibitions::prohibition_check`. An unknown id is an error rather
//!   than a rejection: a typo must not become a rule that silently never fires. The
//!   full six-check evaluation needs an `mm_decision::risk::RiskProfile`, which this
//!   crate deliberately does not depend on, so the *evaluation* belongs to the caller
//!   and the *vocabulary* belongs here.
//! * **Both decisions are recorded in the same table.** A rejection with a written
//!   reason and a promotion are both `promotions` rows; a schema that stored only the
//!   accepted ones would make "why did this not land" unanswerable after the fact.
//!
//! `promote` is the whole commit path: evaluate, record, compute the version the
//! decision produces, append the journal entry, and stage the change set.

use async_trait::async_trait;
use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::benchmark::BenchResult;
use crate::changeset::{ChangeSet, ChangeSetStatus, ChangeSetStore};
use crate::error::{Result, SelfEngError};
use crate::journal::{EvolutionEvent, EvolutionJournal};
use crate::shadow::ShadowReport;

/// The paths that are never auto-evolved, whatever the evidence says.
///
/// The phase plan is explicit — identity invariants and permissions are out of scope
/// for self-modification at all — and the list is configuration rather than a hard-coded
/// `if` so an operator can widen it without a code change. The defaults are the files
/// where those two things actually live.
pub const DEFAULT_IMMUTABLE_PATHS: [&str; 4] = [
    "config/metamind.toml",
    "crates/mm-being/src/invariants.rs",
    "crates/mm-tools/src/permissions.rs",
    "ontology/shapes/mm-shapes.ttl",
];

/// The gate's policy: the numbers and the list it judges with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromotionPolicy {
    /// How much of a benchmark difference is noise.
    pub noise_margin: f64,
    /// The absolute risk ceiling, whatever the budget says.
    pub max_risk: f64,
    /// Paths that are never auto-evolved.
    pub immutable_paths: Vec<String>,
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        PromotionPolicy {
            noise_margin: 0.01,
            max_risk: 1.0,
            immutable_paths: DEFAULT_IMMUTABLE_PATHS
                .iter()
                .map(|path| (*path).to_string())
                .collect(),
        }
    }
}

impl PromotionPolicy {
    /// The first immutable path the change set writes to, if any.
    ///
    /// A file path matches its entry exactly; a path *under* an entry matches too, so a
    /// change to `crates/mm-tools/src/permissions/policy.rs` is caught by the
    /// `.../permissions.rs` entry only if that entry names it — which is why the default
    /// list names files and the check is prefix-based on directory entries as well.
    pub fn touches_immutable(&self, cs: &ChangeSet) -> Option<String> {
        for path in cs.paths() {
            let text = path.to_string_lossy().replace('\\', "/");
            for immutable in &self.immutable_paths {
                let entry = immutable.trim_start_matches("./");
                if text == entry || text.starts_with(&format!("{entry}/")) {
                    return Some(entry.to_string());
                }
            }
        }
        None
    }
}

/// What the gate decided.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PromotionDecision {
    /// Everything held.
    Promote,
    /// Something did not, and this is why.
    Reject {
        /// The written reason: the first rule that fired, with its numbers.
        reason: String,
    },
}

impl PromotionDecision {
    /// The stable wire name, which is also the `promotions.decision` value.
    pub fn as_str(&self) -> &'static str {
        match self {
            PromotionDecision::Promote => "promote",
            PromotionDecision::Reject { .. } => "reject",
        }
    }

    /// The reason, when there is one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            PromotionDecision::Promote => None,
            PromotionDecision::Reject { reason } => Some(reason),
        }
    }

    /// True when the change is promoted.
    pub fn is_promote(&self) -> bool {
        matches!(self, PromotionDecision::Promote)
    }

    /// The status the change set ends in.
    pub fn resulting_status(&self) -> ChangeSetStatus {
        match self {
            PromotionDecision::Promote => ChangeSetStatus::Promoted,
            PromotionDecision::Reject { .. } => ChangeSetStatus::Rejected,
        }
    }
}

/// Everything the gate is allowed to look at.
///
/// Deliberately carries no derives: `regression` is `mm-mistakes`' report type, and a
/// `Clone`/`Debug` here would make this crate's dependencies depend on that crate's
/// derives. The bundle is read by reference and rendered through
/// [`EvidenceBundle::evidence_json`], which is the one rendering that matters.
pub struct EvidenceBundle {
    /// The change set under judgment.
    pub change_set: Ulid,
    /// The regression suite's result.
    pub regression: mm_mistakes::SuiteReport,
    /// The benchmark, absent when none ran.
    pub bench: Option<BenchResult>,
    /// The shadow comparison, absent when none ran.
    pub shadow: Option<ShadowReport>,
    /// The risk the change carries, in the same units as the evolution budget.
    pub risk: f64,
    /// What is left of the evolution budget.
    pub budget_remaining: f64,
    /// The caller's own verdict on whether the change touches identity or permissions.
    pub touches_immutable: bool,
    /// The prohibition `mm-firewall` named, when it ran and named one.
    pub prohibition: Option<String>,
}

impl EvidenceBundle {
    /// A bundle with the regression result and nothing else, for tests and for a call
    /// that has not run a benchmark yet.
    pub fn new(change_set: Ulid, regression: mm_mistakes::SuiteReport) -> Self {
        EvidenceBundle {
            change_set,
            regression,
            bench: None,
            shadow: None,
            risk: 0.0,
            budget_remaining: 0.0,
            touches_immutable: false,
            prohibition: None,
        }
    }

    /// Attach a benchmark result.
    pub fn with_bench(mut self, bench: BenchResult) -> Self {
        self.bench = Some(bench);
        self
    }

    /// Attach a shadow report.
    pub fn with_shadow(mut self, shadow: ShadowReport) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Set the risk and the budget left.
    pub fn with_risk(mut self, risk: f64, budget_remaining: f64) -> Self {
        self.risk = risk;
        self.budget_remaining = budget_remaining;
        self
    }

    /// Set the prohibition the firewall named.
    pub fn with_prohibition(mut self, id: impl Into<String>) -> Self {
        self.prohibition = Some(id.into());
        self
    }

    /// Record the change's own verdict about immutable paths.
    pub fn touching_immutable(mut self) -> Self {
        self.touches_immutable = true;
        self
    }

    /// The JSON the `promotions.evidence_json` and the journal's `evidence_json` carry.
    pub fn evidence_json(&self) -> Result<String> {
        let value = json!({
            "change_set_id": mm_core::ulid_string(&self.change_set),
            "regression": {
                "cases": self.regression.cases.len(),
                "failed": self.regression.failed,
            },
            "bench": self.bench.as_ref().map(BenchResult::canonical),
            "shadow": self.shadow.as_ref().map(ShadowReport::canonical),
            "risk": self.risk,
            "budget_remaining": self.budget_remaining,
            "touches_immutable": self.touches_immutable,
            "prohibition": self.prohibition,
        });
        serde_json::to_string(&value).map_err(SelfEngError::from)
    }
}

/// A gate.
#[async_trait]
pub trait PromotionGate {
    /// Judge a change set against its evidence.
    async fn evaluate(&self, cs: &ChangeSet, ev: &EvidenceBundle) -> Result<PromotionDecision>;
}

/// The reason rule 1 gives, or nothing when no case failed.
///
/// Separated from the rule so the arithmetic is testable without building a suite
/// report: the rule itself is one call to this function.
pub fn regression_reason(failed: u32, cases: usize) -> Option<String> {
    if failed == 0 {
        return None;
    }
    Some(format!(
        "regression: {failed} of {cases} case(s) failed, and a candidate that breaks a kept test is not a candidate"
    ))
}

/// The gate: the plan's six rules, over the policy.
#[derive(Default)]
pub struct DeterministicGate {
    policy: PromotionPolicy,
}

impl DeterministicGate {
    /// A gate with the default policy.
    pub fn new() -> Self {
        DeterministicGate::default()
    }

    /// A gate with an explicit policy.
    pub fn with_policy(policy: PromotionPolicy) -> Self {
        DeterministicGate { policy }
    }

    /// The policy it judges with.
    pub fn policy(&self) -> &PromotionPolicy {
        &self.policy
    }

    /// Rule 2, on its own: is the candidate worse than the baseline beyond the margin?
    fn bench_reason(&self, bench: &BenchResult) -> Option<String> {
        if bench.delta < 0.0 && bench.delta.abs() > self.policy.noise_margin {
            Some(format!(
                "bench: candidate {:.9} is worse than baseline {:.9} by {:.9}, beyond the gate's noise margin {:.9}",
                bench.candidate,
                bench.baseline,
                bench.delta.abs(),
                self.policy.noise_margin
            ))
        } else {
            None
        }
    }

    /// Rule 3, on its own: the two bounds risk is measured against.
    fn risk_reason(&self, ev: &EvidenceBundle) -> Option<String> {
        if !ev.risk.is_finite() {
            return Some(format!("risk: {} is not a number", ev.risk));
        }
        // The budget first, then the ceiling. Both bounds exist, and a candidate that
        // trips both is reported against the one the phase plan names — "risk exceeds the
        // evolution risk budget" — so a rejection reason an operator reads points at the
        // budget they can see moving rather than at a constant in a policy file.
        if ev.risk > ev.budget_remaining {
            return Some(format!(
                "risk: {:.9} exceeds the remaining evolution budget {:.9}",
                ev.risk, ev.budget_remaining
            ));
        }
        if ev.risk > self.policy.max_risk {
            return Some(format!(
                "risk: {:.9} exceeds the policy ceiling {:.9}",
                ev.risk, self.policy.max_risk
            ));
        }
        None
    }

    /// Rule 5, on its own: what a candidate must carry to be judged at all.
    fn evidence_reason(&self, cs: &ChangeSet, ev: &EvidenceBundle) -> Option<String> {
        if ev.bench.is_none() {
            return Some(
                "evidence: no benchmark ran, so nothing says the candidate is not worse"
                    .to_string(),
            );
        }
        if cs.benchmarks.is_empty() {
            return Some("evidence: the change set names no benchmark to be judged by".to_string());
        }
        if cs.rollback.steps.is_empty()
            || cs.rollback.steps.iter().all(|step| step.trim().is_empty())
        {
            return Some("evidence: the change set carries no rollback plan".to_string());
        }
        None
    }

    /// Record the decision, returning the row's id.
    pub async fn record_decision(
        &self,
        cs: &ChangeSet,
        decision: &PromotionDecision,
        ev: &EvidenceBundle,
        store: &mm_store_sqlite::SqliteStore,
        logger: &Logger,
        ids: &UlidFactory,
    ) -> Result<Ulid> {
        let id = ids.next();
        let reason = decision
            .reason()
            .unwrap_or("the gate found nothing to reject")
            .to_string();
        let evidence = ev.evidence_json()?;
        sqlx::query(
            "INSERT INTO promotions (id, changeset_ulid, decision, reason, evidence_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(mm_core::ulid_string(&id))
        .bind(mm_core::ulid_string(&cs.id))
        .bind(decision.as_str())
        .bind(&reason)
        .bind(&evidence)
        .bind(Timestamp::now().to_rfc3339())
        .execute(store.pool())
        .await
        .map_err(|e| SelfEngError::Store(format!("cannot record the decision: {e}")))?;
        logger
            .audit(
                Level::Warn,
                codes::GATE_DECIDE,
                crate::TARGET,
                Some(id),
                json!({
                    "change_set_id": mm_core::ulid_string(&cs.id),
                    "promotion_id": mm_core::ulid_string(&id),
                    "decision": decision.as_str(),
                    "reason": reason,
                    "risk": ev.risk,
                    "budget_remaining": ev.budget_remaining,
                    "evidence": evidence,
                }),
            )
            .await?;
        Ok(id)
    }

    /// Judge, record, version and journal a change set.
    pub async fn promote(
        &self,
        cs: &ChangeSet,
        ev: &EvidenceBundle,
        change_sets: &ChangeSetStore,
        journal: &EvolutionJournal,
    ) -> Result<PromotionOutcome> {
        cs.validate()?;
        if ev.change_set != cs.id {
            return Err(SelfEngError::Gate(format!(
                "the evidence is for {} but the change set is {}",
                mm_core::ulid_string(&ev.change_set),
                mm_core::ulid_string(&cs.id)
            )));
        }
        let decision = self.evaluate(cs, ev).await?;
        let self_version = journal.next_version(decision.as_str(), cs.id).await?;
        let id = self
            .record_decision(
                cs,
                &decision,
                ev,
                journal.store(),
                journal.logger(),
                change_sets.ids(),
            )
            .await?;
        let outcome = match &decision {
            PromotionDecision::Promote => match &ev.bench {
                Some(bench) => format!("promoted; benchmark read {}", bench.canonical()),
                None => "promoted; no benchmark was required by the policy".to_string(),
            },
            PromotionDecision::Reject { reason } => reason.clone(),
        };
        let mut event = EvolutionEvent::new(
            change_sets.ids(),
            cs.reason.clone(),
            cs.hypothesis.clone(),
            outcome,
            decision.as_str(),
        )
        .with_changeset(cs.id);
        event.self_version = self_version.clone();
        event.evidence_json = ev.evidence_json()?;
        journal.append(&event).await?;
        change_sets
            .set_status(cs.id, decision.resulting_status())
            .await?;
        Ok(PromotionOutcome {
            id,
            decision,
            self_version,
        })
    }
}

#[async_trait]
impl PromotionGate for DeterministicGate {
    async fn evaluate(&self, cs: &ChangeSet, ev: &EvidenceBundle) -> Result<PromotionDecision> {
        let reject = |reason: String| Ok(PromotionDecision::Reject { reason });

        // 1. A kept test that now fails ends the question.
        if let Some(reason) = regression_reason(ev.regression.failed, ev.regression.cases.len()) {
            return reject(reason);
        }

        // 2. Worse than the baseline, beyond the gate's margin.
        if let Some(bench) = &ev.bench {
            if let Some(reason) = self.bench_reason(bench) {
                return reject(reason);
            }
        }

        // 3. Risk above the ceiling or the remaining budget.
        if let Some(reason) = self.risk_reason(ev) {
            return reject(reason);
        }

        // 4. A hard prohibition from the firewall. The id is checked against the
        //    firewall's own table, so a typo is an error rather than a rule that never
        //    fires.
        if let Some(id) = &ev.prohibition {
            if mm_firewall::prohibitions::prohibition_check(id).is_none() {
                return Err(SelfEngError::Gate(format!(
                    "{id:?} is not one of the firewall's prohibitions ({:?})",
                    mm_firewall::prohibitions::PROHIBITION_IDS
                )));
            }
            return reject(format!(
                "prohibition: {id} fired, and a hard prohibition is not weighed against a benefit"
            ));
        }

        // 5. Required evidence missing.
        if let Some(reason) = self.evidence_reason(cs, ev) {
            return reject(reason);
        }

        // 6. Identity invariants and permissions are never auto-evolved.
        if ev.touches_immutable {
            return reject(
                "immutable: the caller reported that the change touches identity or permissions"
                    .to_string(),
            );
        }
        if let Some(path) = self.policy.touches_immutable(cs) {
            return reject(format!(
                "immutable: {path} is never auto-evolved, whatever the evidence says"
            ));
        }

        Ok(PromotionDecision::Promote)
    }
}

/// What a promotion produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromotionOutcome {
    /// The `promotions` row's id.
    pub id: Ulid,
    /// The decision.
    pub decision: PromotionDecision,
    /// The version the decision produced.
    pub self_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::{from_gap, BenchmarkId, Gap, Patch, TestId};
    use mm_core::Config;
    use mm_store_sqlite::SqliteStore;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn gap() -> Gap {
        Gap {
            gap_id: Ulid::from_parts(1_700_000_000_000, 1),
            kind: "capability".to_string(),
            statement: "no module summarizes a drifting calibration bin".to_string(),
            evidence: vec![Ulid::from_parts(1_700_000_000_000, 2)],
            target_uri: "https://metamind.dev/code/module/cognition/calibration".to_string(),
            target_path: PathBuf::from("modules/cognition/calibration"),
        }
    }

    fn change_set() -> ChangeSet {
        from_gap(
            &gap(),
            vec![Patch::create(
                "modules/cognition/calibration/src/lib.rs",
                "// scaffolded\n",
            )],
            vec![TestId::new("bench/regression/seeded_bug_01")],
            vec![BenchmarkId::new("promotion_bench")],
            &UlidFactory::new(),
        )
        .unwrap()
    }

    fn bench(candidate: f64, baseline: f64, delta: f64, within_noise: bool) -> BenchResult {
        BenchResult {
            candidate,
            baseline,
            delta,
            n: 3,
            within_noise,
        }
    }

    async fn suite(dir: &std::path::Path) -> mm_mistakes::SuiteReport {
        mm_mistakes::run_suite(dir).await.expect("a suite report")
    }

    async fn clean_bundle(cs: &ChangeSet, dir: &std::path::Path) -> EvidenceBundle {
        EvidenceBundle::new(cs.id, suite(dir).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(0.5, 10.0)
    }

    fn empty_case_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[tokio::test]
    async fn a_clean_candidate_promotes() {
        let dir = empty_case_dir();
        let cs = change_set();
        let gate = DeterministicGate::new();
        let decision = gate
            .evaluate(&cs, &clean_bundle(&cs, dir.path()).await)
            .await
            .unwrap();
        assert_eq!(decision, PromotionDecision::Promote);
        assert_eq!(decision.as_str(), "promote");
        assert!(decision.reason().is_none());
        assert_eq!(decision.resulting_status(), ChangeSetStatus::Promoted);
    }

    #[tokio::test]
    async fn rule_one_names_the_regression() {
        assert!(regression_reason(0, 3).is_none());
        let reason = regression_reason(2, 3).unwrap();
        assert!(reason.starts_with("regression:"), "{reason}");
        assert!(reason.contains("2 of 3"), "{reason}");
    }

    #[tokio::test]
    async fn rule_two_needs_a_real_degradation_beyond_the_margin() {
        let dir = empty_case_dir();
        let cs = change_set();
        let gate = DeterministicGate::new();

        // Better, or worse but inside the margin: no rejection.
        for result in [
            bench(0.6, 0.5, 0.1, false),
            bench(0.4995, 0.5, -0.0005, true),
        ] {
            let decision = gate
                .evaluate(
                    &cs,
                    &EvidenceBundle::new(cs.id, suite(dir.path()).await)
                        .with_bench(result)
                        .with_risk(0.0, 0.0),
                )
                .await
                .unwrap();
            assert!(decision.is_promote(), "{decision:?}");
        }

        // Worse beyond the margin: rejected, with the numbers.
        let bad = bench(0.40, 0.50, -0.10, false);
        let decision = gate
            .evaluate(
                &cs,
                &EvidenceBundle::new(cs.id, suite(dir.path()).await)
                    .with_bench(bad)
                    .with_risk(0.0, 0.0),
            )
            .await
            .unwrap();
        let reason = decision.reason().unwrap();
        assert!(reason.starts_with("bench:"), "{reason}");
        assert!(reason.contains("0.100000000"), "{reason}");
    }

    #[tokio::test]
    async fn rule_three_compares_risk_with_the_budget_and_the_ceiling() {
        let dir = empty_case_dir();
        let cs = change_set();
        let gate = DeterministicGate::new();
        let over_budget = EvidenceBundle::new(cs.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(5.0, 1.0);
        let reason = gate.evaluate(&cs, &over_budget).await.unwrap();
        assert!(
            reason.reason().unwrap().contains("evolution budget"),
            "{reason:?}"
        );

        let over_ceiling = EvidenceBundle::new(cs.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(2.0, 100.0);
        let reason = gate.evaluate(&cs, &over_ceiling).await.unwrap();
        assert!(reason.reason().unwrap().contains("ceiling"), "{reason:?}");

        let not_a_number = EvidenceBundle::new(cs.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(f64::NAN, 100.0);
        assert!(gate
            .evaluate(&cs, &not_a_number)
            .await
            .unwrap()
            .reason()
            .unwrap()
            .contains("not a number"));
    }

    #[tokio::test]
    async fn rule_four_validates_the_prohibition_against_the_firewall() {
        let dir = empty_case_dir();
        let cs = change_set();
        let gate = DeterministicGate::new();
        let bundle = EvidenceBundle::new(cs.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(0.0, 10.0)
            .with_prohibition("spend_over_budget");
        let decision = gate.evaluate(&cs, &bundle).await.unwrap();
        let reason = decision.reason().unwrap();
        assert!(reason.starts_with("prohibition:"), "{reason}");
        assert!(reason.contains("spend_over_budget"), "{reason}");

        // A typo is an error, not a rejection: a rule that silently never fires is
        // worse than no rule.
        let typo = EvidenceBundle::new(cs.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(0.0, 10.0)
            .with_prohibition("spend_over_budgt");
        let error = gate.evaluate(&cs, &typo).await.unwrap_err();
        assert_eq!(error.code(), "gate");
    }

    #[tokio::test]
    async fn rule_five_requires_a_benchmark_and_a_rollback_plan() {
        let dir = empty_case_dir();
        let cs = change_set();
        let gate = DeterministicGate::new();

        let no_bench = EvidenceBundle::new(cs.id, suite(dir.path()).await).with_risk(0.0, 10.0);
        let reason = gate.evaluate(&cs, &no_bench).await.unwrap();
        assert!(
            reason.reason().unwrap().contains("no benchmark"),
            "{reason:?}"
        );

        let mut unnamed = change_set();
        unnamed.benchmarks.clear();
        let without = EvidenceBundle::new(unnamed.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(0.0, 10.0);
        let reason = gate.evaluate(&unnamed, &without).await.unwrap();
        assert!(
            reason.reason().unwrap().contains("names no benchmark"),
            "{reason:?}"
        );

        let mut without_plan = change_set();
        without_plan.rollback.steps = vec!["   ".to_string()];
        let bundle = EvidenceBundle::new(without_plan.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(0.0, 10.0);
        let reason = gate.evaluate(&without_plan, &bundle).await.unwrap();
        assert!(
            reason.reason().unwrap().contains("no rollback plan"),
            "{reason:?}"
        );
    }

    #[tokio::test]
    async fn rule_six_refuses_the_immutable_paths_even_with_perfect_evidence() {
        let dir = empty_case_dir();
        let gate = DeterministicGate::new();

        let mut touching = change_set();
        touching.code = vec![Patch::modify(
            "crates/mm-tools/src/permissions.rs",
            "// no\n",
        )];
        touching.rollback.state_hash = crate::changeset::patches_state_hash(&touching.code);
        let bundle = EvidenceBundle::new(touching.id, suite(dir.path()).await)
            .with_bench(bench(0.6, 0.5, 0.1, false))
            .with_risk(0.0, 100.0);
        let reason = gate.evaluate(&touching, &bundle).await.unwrap();
        let text = reason.reason().unwrap();
        assert!(text.starts_with("immutable:"), "{text}");
        assert!(text.contains("permissions.rs"), "{text}");

        // The caller's own verdict is honoured too.
        let clean = change_set();
        let bundle = EvidenceBundle::new(clean.id, suite(dir.path()).await)
            .with_bench(bench(0.5, 0.5, 0.0, true))
            .with_risk(0.0, 100.0)
            .touching_immutable();
        let reason = gate.evaluate(&clean, &bundle).await.unwrap();
        assert!(reason.reason().unwrap().contains("identity"), "{reason:?}");
    }

    #[test]
    fn the_policy_matches_a_file_and_a_directory_entry() {
        let policy = PromotionPolicy::default();
        let mut cs = change_set();
        cs.code = vec![Patch::create(
            "crates/mm-tools/src/permissions/extra.rs",
            "x",
        )];
        assert_eq!(
            policy.touches_immutable(&cs),
            None,
            "a sibling file is not the immutable one"
        );
        cs.code = vec![Patch::create("crates/mm-tools/src/permissions.rs", "x")];
        assert!(policy.touches_immutable(&cs).is_some());
        cs.code = vec![Patch::create("modules/x/lib.rs", "x")];
        assert!(policy.touches_immutable(&cs).is_none());
    }

    #[tokio::test]
    async fn a_promotion_records_the_decision_versions_it_and_appends_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir.path());
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        let change_sets = ChangeSetStore::new(store.clone(), logger.clone(), ids.clone());
        let journal = EvolutionJournal::new(store.clone(), logger, ids);
        let cs = change_set();
        change_sets.create(&cs).await.unwrap();

        let case_dir = empty_case_dir();
        let bundle = clean_bundle(&cs, case_dir.path()).await;
        let gate = DeterministicGate::new();
        let outcome = gate
            .promote(&cs, &bundle, &change_sets, &journal)
            .await
            .unwrap();
        assert!(outcome.decision.is_promote());
        assert!(
            outcome.self_version.starts_with("self-1-"),
            "{}",
            outcome.self_version
        );
        assert_eq!(
            change_sets.status(cs.id).await.unwrap(),
            Some(ChangeSetStatus::Promoted)
        );
        let events = journal.events().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].self_version, outcome.self_version);
        assert_eq!(events[0].changeset, Some(cs.id));
        let promotions: i64 = sqlx::query_scalar("SELECT count(*) FROM promotions")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(promotions, 1);
    }

    #[tokio::test]
    async fn a_rejection_is_recorded_with_its_reason_and_the_change_set_is_staged_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir.path());
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        let change_sets = ChangeSetStore::new(store.clone(), logger.clone(), ids.clone());
        let journal = EvolutionJournal::new(store.clone(), logger, ids);
        let cs = change_set();
        change_sets.create(&cs).await.unwrap();

        let case_dir = empty_case_dir();
        let bundle = EvidenceBundle::new(cs.id, suite(case_dir.path()).await)
            .with_bench(bench(0.4, 0.5, -0.1, false))
            .with_risk(0.0, 10.0);
        let gate = DeterministicGate::new();
        let outcome = gate
            .promote(&cs, &bundle, &change_sets, &journal)
            .await
            .unwrap();
        let reason = outcome.decision.reason().unwrap();
        assert!(reason.starts_with("bench:"), "{reason}");
        assert_eq!(
            change_sets.status(cs.id).await.unwrap(),
            Some(ChangeSetStatus::Rejected)
        );
        let stored: String = sqlx::query_scalar("SELECT reason FROM promotions")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(stored, reason, "the written reason is what the row carries");
    }

    #[tokio::test]
    async fn evidence_for_another_change_set_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir.path());
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        let change_sets = ChangeSetStore::new(store.clone(), logger.clone(), ids.clone());
        let journal = EvolutionJournal::new(store.clone(), logger, ids);
        let cs = change_set();
        change_sets.create(&cs).await.unwrap();
        let case_dir = empty_case_dir();
        let mut bundle = clean_bundle(&cs, case_dir.path()).await;
        bundle.change_set = Ulid::from_parts(1_700_000_000_000, 77);
        let error = DeterministicGate::new()
            .promote(&cs, &bundle, &change_sets, &journal)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "gate");
    }

    #[test]
    fn the_evidence_bundle_renders_to_json() {
        let cs = change_set();
        let bundle = EvidenceBundle {
            change_set: cs.id,
            regression: mm_mistakes::SuiteReport {
                cases: Vec::new(),
                failed: 0,
            },
            bench: Some(bench(0.5, 0.5, 0.0, true)),
            shadow: Some(ShadowReport {
                runs: 1,
                divergences: 0,
                detail: "1 case(s) identical".to_string(),
            }),
            risk: 0.25,
            budget_remaining: 19.75,
            touches_immutable: false,
            prohibition: None,
        };
        let json = bundle.evidence_json().unwrap();
        assert!(json.contains("\"failed\":0"), "{json}");
        assert!(json.contains("\"risk\":0.25"), "{json}");
        assert!(json.contains("identical"), "{json}");
    }
}
