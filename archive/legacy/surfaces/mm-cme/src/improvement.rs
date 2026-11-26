//! The self-improvement loop: reflect, plan, act, verify and learn.
//!
//! The loop is artifact-agnostic. A [`Reviewable`] artifact supplies named
//! metrics in `0.0..=1.0`; critics turn weak metrics into issues; the hill climber
//! proposes a better variant; verifiers decide whether it may replace the
//! baseline. Governance can require human approval before acting and can roll
//! back a verification failure. The improvement region's state machine records
//! each phase.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dashboard::Dashboard;
use crate::error::Result;
use crate::hill_climb::{climb, HillClimbConfig, HillClimbResult, Optimizable};
use crate::report::Report;
use crate::session::Session;

/// What started a cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementTrigger {
    /// The scheduler fired.
    Scheduled,
    /// An artifact changed.
    OnChange,
    /// A metric regressed.
    OnRegression,
    /// The user gave feedback.
    OnFeedback,
    /// An explicit request.
    Manual,
}

/// How bad an issue is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Cosmetic.
    Low,
    /// Worth fixing.
    Medium,
    /// Should be fixed now.
    High,
    /// Blocks release or use.
    Critical,
}

/// What kind of weakness was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCategory {
    /// Incorrect content.
    Correctness,
    /// Hard to understand.
    Clarity,
    /// No clear next steps.
    Actionability,
    /// Missing information.
    Completeness,
    /// Internal inconsistency.
    Consistency,
    /// Wrong voice for the audience.
    Tone,
    /// Too expensive or verbose.
    Efficiency,
    /// Risky or unsafe.
    Safety,
}

/// One issue found during reflection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    /// Critic that found it.
    pub critic: String,
    /// Category.
    pub category: IssueCategory,
    /// Severity.
    pub severity: Severity,
    /// Human-readable detail.
    pub message: String,
    /// The metric and its value.
    pub metric: String,
    pub value: f32,
}

/// A critic inspects an artifact's metrics.
pub trait Critic: Send + Sync {
    /// Stable name.
    fn name(&self) -> &'static str;
    /// Produce an issue when the relevant metric is outside bounds.
    fn review(&self, metrics: &BTreeMap<String, f32>) -> Option<Issue>;
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    AtLeast,
    AtMost,
}

#[derive(Debug, Clone, Copy)]
struct MetricCritic {
    name: &'static str,
    metric: &'static str,
    threshold: f32,
    direction: Direction,
    category: IssueCategory,
    severity: Severity,
}

impl Critic for MetricCritic {
    fn name(&self) -> &'static str {
        self.name
    }

    fn review(&self, metrics: &BTreeMap<String, f32>) -> Option<Issue> {
        let value = *metrics.get(self.metric)?;
        let weak = match self.direction {
            Direction::AtLeast => value < self.threshold,
            Direction::AtMost => value > self.threshold,
        };
        weak.then(|| Issue {
            critic: self.name.to_owned(),
            category: self.category,
            severity: self.severity,
            message: format!(
                "{} is {:.2}; expected {} {:.2}",
                self.metric,
                value,
                match self.direction {
                    Direction::AtLeast => "at least",
                    Direction::AtMost => "at most",
                },
                self.threshold
            ),
            metric: self.metric.to_owned(),
            value,
        })
    }
}

struct EfficiencyCritic;

impl Critic for EfficiencyCritic {
    fn name(&self) -> &'static str {
        "efficiency"
    }

    fn review(&self, metrics: &BTreeMap<String, f32>) -> Option<Issue> {
        // Dashboards expose query cost as a burden; reports expose brevity as a
        // fitness score. The same critic covers both.
        if let Some(cost) = metrics.get("query_cost") {
            return (*cost > 0.5).then(|| Issue {
                critic: self.name().to_owned(),
                category: IssueCategory::Efficiency,
                severity: Severity::Medium,
                message: format!("query_cost is {cost:.2}; expected at most 0.50"),
                metric: "query_cost".to_owned(),
                value: *cost,
            });
        }
        let brevity = metrics.get("brevity")?;
        (*brevity < 0.55).then(|| Issue {
            critic: self.name().to_owned(),
            category: IssueCategory::Efficiency,
            severity: Severity::Medium,
            message: format!("brevity is {brevity:.2}; expected at least 0.55"),
            metric: "brevity".to_owned(),
            value: *brevity,
        })
    }
}

/// Correctness, clarity, actionability, completeness, consistency, tone,
/// efficiency and safety.
pub fn default_critics() -> Vec<Box<dyn Critic>> {
    vec![
        Box::new(MetricCritic {
            name: "correctness",
            metric: "correctness",
            threshold: 0.70,
            direction: Direction::AtLeast,
            category: IssueCategory::Correctness,
            severity: Severity::Critical,
        }),
        Box::new(MetricCritic {
            name: "clarity",
            metric: "clarity",
            threshold: 0.60,
            direction: Direction::AtLeast,
            category: IssueCategory::Clarity,
            severity: Severity::High,
        }),
        Box::new(MetricCritic {
            name: "actionability",
            metric: "actionability",
            threshold: 0.50,
            direction: Direction::AtLeast,
            category: IssueCategory::Actionability,
            severity: Severity::High,
        }),
        Box::new(MetricCritic {
            name: "completeness",
            metric: "completeness",
            threshold: 0.80,
            direction: Direction::AtLeast,
            category: IssueCategory::Completeness,
            severity: Severity::Medium,
        }),
        Box::new(MetricCritic {
            name: "consistency",
            metric: "consistency",
            threshold: 0.70,
            direction: Direction::AtLeast,
            category: IssueCategory::Consistency,
            severity: Severity::Low,
        }),
        Box::new(MetricCritic {
            name: "tone",
            metric: "tone",
            threshold: 0.70,
            direction: Direction::AtLeast,
            category: IssueCategory::Tone,
            severity: Severity::Low,
        }),
        Box::new(EfficiencyCritic),
        Box::new(MetricCritic {
            name: "safety",
            metric: "safety",
            threshold: 0.80,
            direction: Direction::AtLeast,
            category: IssueCategory::Safety,
            severity: Severity::Critical,
        }),
    ]
}

/// What a verifier checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    /// Regression tests.
    UnitTest,
    /// Style and consistency.
    Lint,
    /// Required fields and shape.
    Schema,
    /// A model judge's composite score.
    LlmJudge,
    /// A human approved the change.
    HumanReview,
    /// Simulated cost and load.
    Simulation,
}

/// Inputs to one verification.
#[derive(Debug, Clone, Copy)]
pub struct VerificationContext<'a> {
    /// Metrics before the mutation.
    pub baseline: &'a BTreeMap<String, f32>,
    /// Metrics after the mutation.
    pub candidate: &'a BTreeMap<String, f32>,
    /// Composite score before.
    pub baseline_score: f32,
    /// Composite score after.
    pub candidate_score: f32,
    /// Whether a human approved the change.
    pub approved: bool,
}

/// The result of one verifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionResult {
    /// Which verifier ran.
    pub kind: VerificationKind,
    /// Whether it passed.
    pub passed: bool,
    /// Value before.
    pub baseline: f32,
    /// Value after.
    pub candidate: f32,
}

/// All verifier results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Whether every criterion passed.
    pub passed: bool,
    /// Individual criteria.
    pub criteria: Vec<CriterionResult>,
}

/// A verifier for one acceptance dimension.
pub trait Verifier: Send + Sync {
    /// Which verifier this is.
    fn kind(&self) -> VerificationKind;
    /// Check the candidate.
    fn verify(&self, ctx: &VerificationContext<'_>) -> CriterionResult;
}

struct MetricVerifier {
    kind: VerificationKind,
    metric: &'static str,
    direction: Direction,
    margin: f32,
    fallback: f32,
}

impl Verifier for MetricVerifier {
    fn kind(&self) -> VerificationKind {
        self.kind
    }

    fn verify(&self, ctx: &VerificationContext<'_>) -> CriterionResult {
        let baseline = ctx
            .baseline
            .get(self.metric)
            .copied()
            .unwrap_or(self.fallback);
        let candidate = ctx
            .candidate
            .get(self.metric)
            .copied()
            .unwrap_or(self.fallback);
        let passed = match self.direction {
            Direction::AtLeast => candidate + self.margin >= baseline,
            Direction::AtMost => candidate <= baseline + self.margin,
        };
        CriterionResult {
            kind: self.kind,
            passed,
            baseline,
            candidate,
        }
    }
}

struct HumanReviewVerifier;

impl Verifier for HumanReviewVerifier {
    fn kind(&self) -> VerificationKind {
        VerificationKind::HumanReview
    }

    fn verify(&self, ctx: &VerificationContext<'_>) -> CriterionResult {
        CriterionResult {
            kind: self.kind(),
            passed: ctx.approved,
            baseline: 0.0,
            candidate: if ctx.approved { 1.0 } else { 0.0 },
        }
    }
}

struct JudgeVerifier;

impl Verifier for JudgeVerifier {
    fn kind(&self) -> VerificationKind {
        VerificationKind::LlmJudge
    }

    fn verify(&self, ctx: &VerificationContext<'_>) -> CriterionResult {
        CriterionResult {
            kind: self.kind(),
            passed: ctx.candidate_score + 0.001 >= ctx.baseline_score,
            baseline: ctx.baseline_score,
            candidate: ctx.candidate_score,
        }
    }
}

/// The six verifiers from the design.
pub fn default_verifiers() -> Vec<Box<dyn Verifier>> {
    vec![
        Box::new(MetricVerifier {
            kind: VerificationKind::UnitTest,
            metric: "correctness",
            direction: Direction::AtLeast,
            margin: 0.05,
            fallback: 0.9,
        }),
        Box::new(MetricVerifier {
            kind: VerificationKind::Lint,
            metric: "consistency",
            direction: Direction::AtLeast,
            margin: 0.02,
            fallback: 0.8,
        }),
        Box::new(MetricVerifier {
            kind: VerificationKind::Schema,
            metric: "completeness",
            direction: Direction::AtLeast,
            margin: 0.0,
            fallback: 1.0,
        }),
        Box::new(JudgeVerifier),
        Box::new(HumanReviewVerifier),
        Box::new(MetricVerifier {
            kind: VerificationKind::Simulation,
            metric: "query_cost",
            direction: Direction::AtMost,
            margin: 0.05,
            fallback: 0.0,
        }),
    ]
}

/// Artifacts the loop can improve.
pub trait Reviewable: Optimizable {
    /// Human-readable title.
    fn title(&self) -> &str;
    /// Artifact kind (`report`, `dashboard`).
    fn kind(&self) -> &'static str;
    /// Named metrics.
    fn metrics(&self) -> BTreeMap<String, f32>;
}

impl Reviewable for Report {
    fn title(&self) -> &str {
        &self.title
    }

    fn kind(&self) -> &'static str {
        "report"
    }

    fn metrics(&self) -> BTreeMap<String, f32> {
        Report::metrics(self)
    }
}

impl Reviewable for Dashboard {
    fn title(&self) -> &str {
        &self.title
    }

    fn kind(&self) -> &'static str {
        "dashboard"
    }

    fn metrics(&self) -> BTreeMap<String, f32> {
        Dashboard::metrics(self)
    }
}

/// Budgets and approval gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceConfig {
    /// Refuse to act without explicit approval.
    pub require_human_approval: bool,
    /// Restore the baseline when verification fails.
    pub auto_rollback: bool,
    /// Cycles allowed over the loop's lifetime.
    pub max_cycles: u32,
}

impl Default for GovernanceConfig {
    fn default() -> Self {
        Self {
            require_human_approval: false,
            auto_rollback: true,
            max_cycles: 100,
        }
    }
}

/// A pattern learned from a successful mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedPattern {
    /// Artifact kind.
    pub artifact: String,
    /// Category that motivated the change.
    pub category: IssueCategory,
    /// What was improved.
    pub detail: String,
    /// Score gain.
    pub gain: f32,
}

/// The outcome of one loop cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImprovementOutcome {
    /// Artifact kind.
    pub artifact: String,
    /// What triggered the cycle.
    pub trigger: ImprovementTrigger,
    /// Score before.
    pub initial_score: f32,
    /// Score after.
    pub final_score: f32,
    /// Whether the hill climber found a better variant.
    pub accepted: bool,
    /// Whether the baseline was restored.
    pub rolled_back: bool,
    /// Reflection findings.
    pub issues: Vec<Issue>,
    /// Verification details.
    pub verification: VerificationReport,
    /// Why the cycle did nothing, when it did nothing.
    pub skipped: Option<String>,
    /// Patterns learned this cycle.
    pub patterns: Vec<LearnedPattern>,
}

impl ImprovementOutcome {
    /// Improvement in composite score.
    pub fn improvement(&self) -> f32 {
        self.final_score - self.initial_score
    }
}

/// The five-phase loop.
pub struct ImprovementLoop {
    critics: Vec<Box<dyn Critic>>,
    verifiers: Vec<Box<dyn Verifier>>,
    hill: HillClimbConfig,
    governance: GovernanceConfig,
    cycles: u32,
    patterns: Vec<LearnedPattern>,
}

impl std::fmt::Debug for ImprovementLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImprovementLoop")
            .field("critics", &self.critics.len())
            .field("verifiers", &self.verifiers.len())
            .field("hill", &self.hill)
            .field("governance", &self.governance)
            .field("cycles", &self.cycles)
            .finish()
    }
}

impl Default for ImprovementLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl ImprovementLoop {
    /// A loop with the default critics, verifiers and governance.
    pub fn new() -> Self {
        Self {
            critics: default_critics(),
            verifiers: default_verifiers(),
            hill: HillClimbConfig::default(),
            governance: GovernanceConfig::default(),
            cycles: 0,
            patterns: Vec::new(),
        }
    }

    /// Replace the governance settings.
    pub fn with_governance(mut self, governance: GovernanceConfig) -> Self {
        self.governance = governance;
        self
    }

    /// Replace the critics.
    pub fn with_critics(mut self, critics: Vec<Box<dyn Critic>>) -> Self {
        self.critics = critics;
        self
    }

    /// Replace the verifiers.
    pub fn with_verifiers(mut self, verifiers: Vec<Box<dyn Verifier>>) -> Self {
        self.verifiers = verifiers;
        self
    }

    /// Replace the hill-climb settings.
    pub fn with_hill_config(mut self, hill: HillClimbConfig) -> Self {
        self.hill = hill;
        self
    }

    /// Cycles completed.
    pub fn cycles(&self) -> u32 {
        self.cycles
    }

    /// Patterns learned so far.
    pub fn patterns(&self) -> &[LearnedPattern] {
        &self.patterns
    }

    /// Run one cycle over `artifact`, driving the improvement region.
    pub fn run<T: Reviewable>(
        &mut self,
        session: &mut Session,
        artifact: &mut T,
        trigger: ImprovementTrigger,
        approved: bool,
    ) -> Result<ImprovementOutcome> {
        if session.region_state("improvement") != Some("idle") {
            return Ok(skipped_outcome(
                artifact,
                trigger,
                "improvement region is not idle",
            ));
        }
        if self.cycles >= self.governance.max_cycles {
            return Ok(skipped_outcome(artifact, trigger, "cycle budget exhausted"));
        }

        let baseline = artifact.clone();
        let initial_score = baseline.score();
        let baseline_metrics = baseline.metrics();

        // 1. Reflect.
        session.dispatch_regions("start", None)?;
        let issues: Vec<Issue> = self
            .critics
            .iter()
            .filter_map(|critic| critic.review(&baseline_metrics))
            .collect();
        session.dispatch_regions(
            "reflection_done",
            Some(serde_json::json!({
                "findings": issues.iter().map(|issue| issue.message.clone()).collect::<Vec<_>>()
            })),
        )?;

        // 2. Plan.
        let plan = if issues.is_empty() {
            "no issues found".to_owned()
        } else {
            format!("address {} issue(s)", issues.len())
        };
        session.dispatch_regions("plan_ready", Some(serde_json::json!({ "plan": plan })))?;

        // Governance gate before any mutation.
        if self.governance.require_human_approval && !approved {
            session.dispatch_regions(
                "error",
                Some(serde_json::json!({ "detail": "approval required" })),
            )?;
            return Ok(ImprovementOutcome {
                artifact: artifact.kind().to_owned(),
                trigger,
                initial_score,
                final_score: initial_score,
                accepted: false,
                rolled_back: false,
                issues,
                verification: VerificationReport {
                    passed: false,
                    criteria: Vec::new(),
                },
                skipped: Some("human approval required".to_owned()),
                patterns: Vec::new(),
            });
        }

        // 3. Act: one steepest-ascent walk.
        let climb_result: HillClimbResult<T> = climb(&baseline, self.hill);
        session.dispatch_regions("action_done", None)?;
        let accepted = climb_result.improved();
        let gain = climb_result.improvement();
        let steps = climb_result.steps.len();
        let candidate = climb_result.best;
        let candidate_metrics = candidate.metrics();
        let final_score = candidate.score();

        // 4. Verify. A human review passes when governance does not require one,
        //    so the default loop is not blocked on an approval that is not asked
        //    for.
        let verification_approved = approved || !self.governance.require_human_approval;
        let ctx = VerificationContext {
            baseline: &baseline_metrics,
            candidate: &candidate_metrics,
            baseline_score: initial_score,
            candidate_score: final_score,
            approved: verification_approved,
        };
        let criteria: Vec<CriterionResult> = self
            .verifiers
            .iter()
            .map(|verifier| verifier.verify(&ctx))
            .collect();
        let passed = criteria.iter().all(|criterion| criterion.passed);
        let verification = VerificationReport { passed, criteria };

        if !passed {
            if self.governance.auto_rollback {
                session.dispatch_regions("rollback_triggered", None)?;
                session.dispatch_regions("rollback_done", None)?;
                return Ok(ImprovementOutcome {
                    artifact: artifact.kind().to_owned(),
                    trigger,
                    initial_score,
                    final_score: initial_score,
                    accepted: false,
                    rolled_back: true,
                    issues,
                    verification,
                    skipped: None,
                    patterns: Vec::new(),
                });
            }
            session.dispatch_regions(
                "error",
                Some(serde_json::json!({ "detail": "verification failed" })),
            )?;
            return Ok(ImprovementOutcome {
                artifact: artifact.kind().to_owned(),
                trigger,
                initial_score,
                final_score,
                accepted: false,
                rolled_back: false,
                issues,
                verification,
                skipped: Some("verification failed".to_owned()),
                patterns: Vec::new(),
            });
        }

        *artifact = candidate;
        session.dispatch_regions("verification_done", None)?;

        // 5. Learn.
        let mut patterns = Vec::new();
        if accepted {
            if let Some(issue) = issues.first() {
                let pattern = LearnedPattern {
                    artifact: artifact.kind().to_owned(),
                    category: issue.category,
                    detail: format!(
                        "{} improved by {:.3} over {} step(s)",
                        artifact.kind(),
                        gain,
                        steps
                    ),
                    gain,
                };
                self.patterns.push(pattern.clone());
                patterns.push(pattern);
            }
        }
        session.dispatch_regions("learned", Some(serde_json::json!({ "patterns": patterns })))?;
        session.dispatch_regions("cycle_complete", None)?;
        self.cycles = self.cycles.saturating_add(1);

        Ok(ImprovementOutcome {
            artifact: artifact.kind().to_owned(),
            trigger,
            initial_score,
            final_score,
            accepted,
            rolled_back: false,
            issues,
            verification,
            skipped: None,
            patterns,
        })
    }
}

fn skipped_outcome<T: Reviewable>(
    artifact: &T,
    trigger: ImprovementTrigger,
    reason: &str,
) -> ImprovementOutcome {
    ImprovementOutcome {
        artifact: artifact.kind().to_owned(),
        trigger,
        initial_score: artifact.score(),
        final_score: artifact.score(),
        accepted: false,
        rolled_back: false,
        issues: Vec::new(),
        verification: VerificationReport {
            passed: false,
            criteria: Vec::new(),
        },
        skipped: Some(reason.to_owned()),
        patterns: Vec::new(),
    }
}

/// A tick-based schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImprovementScheduler {
    /// Run every N ticks.
    pub every_ticks: u64,
    ticks: u64,
}

impl ImprovementScheduler {
    /// Build a scheduler; `every_ticks` is clamped to one.
    pub fn new(every_ticks: u64) -> Self {
        Self {
            every_ticks: every_ticks.max(1),
            ticks: 0,
        }
    }

    /// Advance one tick and report whether a cycle is due.
    pub fn due(&mut self) -> bool {
        self.ticks = self.ticks.saturating_add(1);
        self.ticks.is_multiple_of(self.every_ticks)
    }
}

/// Scheduled improvement over the report and dashboard artifacts.
pub struct ImprovementEngine {
    loop_: ImprovementLoop,
    scheduler: ImprovementScheduler,
    report: Report,
    dashboard: Dashboard,
}

impl std::fmt::Debug for ImprovementEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImprovementEngine")
            .field("scheduler", &self.scheduler)
            .field("report", &self.report.title)
            .field("dashboard", &self.dashboard.title)
            .finish()
    }
}

impl ImprovementEngine {
    /// Build an engine over existing artifacts.
    pub fn new(
        report: Report,
        dashboard: Dashboard,
        every_ticks: u64,
        improvement: ImprovementLoop,
    ) -> Self {
        Self {
            loop_: improvement,
            scheduler: ImprovementScheduler::new(every_ticks),
            report,
            dashboard,
        }
    }

    /// The report artifact.
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// The dashboard artifact.
    pub fn dashboard(&self) -> &Dashboard {
        &self.dashboard
    }

    /// The loop.
    pub fn loop_(&self) -> &ImprovementLoop {
        &self.loop_
    }

    /// Run both loops when the schedule is due.
    pub fn on_tick(&mut self, session: &mut Session) -> Result<Vec<ImprovementOutcome>> {
        if !self.scheduler.due() {
            return Ok(Vec::new());
        }
        let approved = !self.loop_.governance.require_human_approval;
        let report = self.loop_.run(
            session,
            &mut self.report,
            ImprovementTrigger::Scheduled,
            approved,
        )?;
        let dashboard = self.loop_.run(
            session,
            &mut self.dashboard,
            ImprovementTrigger::Scheduled,
            approved,
        )?;
        Ok(vec![report, dashboard])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::{Panel, PanelKind, ViewingContext};
    use crate::report::{Modality, ReportSection};
    use mm_data::UserId;

    fn weak_report() -> Report {
        Report::new("r", "Audit")
            .with_section(ReportSection::narrative("s1", "Findings", "x".repeat(700)))
            .with_section(ReportSection {
                id: "s2".into(),
                title: "Next".into(),
                modality: Modality::Narrative,
                detail_level: 5,
                content: String::new(),
                sources: Vec::new(),
                action_items: Vec::new(),
                importance: 5,
            })
    }

    fn weak_dashboard() -> Dashboard {
        Dashboard::new("d", "Health", ViewingContext::Mobile)
            .with_panel(Panel {
                id: "p1".into(),
                title: "Latency".into(),
                kind: PanelKind::Chart,
                cost_per_refresh: 8.0,
                critical: true,
                has_threshold: false,
                detail: 3,
            })
            .with_panel(Panel {
                id: "p2".into(),
                title: "Notes".into(),
                kind: PanelKind::List,
                cost_per_refresh: 6.0,
                critical: false,
                has_threshold: false,
                detail: 3,
            })
    }

    #[test]
    fn a_weak_report_improves_and_learns_a_pattern() {
        let mut session = Session::new(UserId::new("u1"), None).unwrap();
        session
            .dispatch(crate::session::SessionEvent::InitComplete)
            .unwrap();
        let mut report = weak_report();
        let before = report.score();
        let mut improvement = ImprovementLoop::new();
        let outcome = improvement
            .run(&mut session, &mut report, ImprovementTrigger::Manual, true)
            .unwrap();
        assert!(outcome.verification.passed);
        assert!(outcome.accepted);
        assert!(report.score() > before);
        assert!(!outcome.patterns.is_empty());
        assert_eq!(session.region_state("improvement"), Some("idle"));
    }

    #[test]
    fn approval_is_required_before_acting() {
        let mut session = Session::new(UserId::new("u1"), None).unwrap();
        session
            .dispatch(crate::session::SessionEvent::InitComplete)
            .unwrap();
        let mut report = weak_report();
        let before = report.clone();
        let mut improvement = ImprovementLoop::new().with_governance(GovernanceConfig {
            require_human_approval: true,
            ..GovernanceConfig::default()
        });
        let outcome = improvement
            .run(&mut session, &mut report, ImprovementTrigger::Manual, false)
            .unwrap();
        assert_eq!(outcome.skipped.as_deref(), Some("human approval required"));
        assert_eq!(report, before);
    }

    #[test]
    fn scheduled_engine_improves_both_artifacts() {
        let mut session = Session::new(UserId::new("u1"), None).unwrap();
        session
            .dispatch(crate::session::SessionEvent::InitComplete)
            .unwrap();
        let report = weak_report();
        let dashboard = weak_dashboard();
        let report_before = report.score();
        let dashboard_before = dashboard.score();
        let mut engine = ImprovementEngine::new(report, dashboard, 1, ImprovementLoop::new());
        let outcomes = engine.on_tick(&mut session).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|outcome| outcome.verification.passed));
        assert!(engine.report().score() > report_before);
        assert!(engine.dashboard().score() > dashboard_before);
    }
}
