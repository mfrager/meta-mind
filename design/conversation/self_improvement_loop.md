# Self-Reflection & Self-Improvement Loop

> A general-purpose meta-cognitive framework for continuous self-improvement applicable to any artifact, behavior, policy, or process — including report design, dashboard construction, prompt engineering, and the agent's own cognition.

---

## 1. Core Philosophy

> **Improvement = Reflection × Action × Verification × Learning**

The loop is not "think harder" — it's a **structured, measurable, interruptible process** that can be applied to anything the agent produces or does.

```
┌─────────────────────────────────────────────────────────────────┐
│                    SELF-IMPROVEMENT LOOP                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│    ┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────┐ │
│    │  REFLECT │────▶│   PLAN   │────▶│   ACT    │────▶│VERIFY│ │
│    └──────────┘     └──────────┘     └──────────┘     └────┬──┘ │
│                                                             │     │
│                    ┌─────────────────────────────────────────┘     │
│                    │                                               │
│                    ▼                                               │
│              ┌──────────┐     ┌──────────┐     ┌──────────┐      │
│              │  LEARN   │────▶│  STORE   │────▶│  APPLY   │      │
│              └──────────┘     └──────────┘     └──────────┘      │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Key principles:**
- **Artifact-agnostic**: Works on reports, dashboards, prompts, policies, code, behaviors
- **Measurable**: Every iteration produces a score delta
- **Interruptible**: Can pause/resume at any phase
- **Explainable**: Every decision has a rationale trace
- **Composable**: Multiple loops can run in parallel on different artifacts
- **Budgeted**: Time, compute, and attention budgets enforced

---

## 2. Loop Phases

### Phase 1: REFLECT — Understand Current State

```rust
pub struct Reflection {
    pub artifact: ArtifactRef,
    pub current_state: ArtifactState,
    pub evaluation: EvaluationResult,
    pub issues: Vec<IdentifiedIssue>,
    pub opportunities: Vec<ImprovementOpportunity>,
    pub context: ReflectionContext,
}

pub struct IdentifiedIssue {
    pub id: IssueId,
    pub severity: Severity,
    pub category: IssueCategory,
    pub description: String,
    pub evidence: Vec<Evidence>,
    pub location: Option<Location>,
    pub impact_estimate: ImpactEstimate,
}

pub enum IssueCategory {
    Clarity,           // Hard to understand
    Brevity,           // Too verbose
    Completeness,      // Missing information
    Accuracy,          // Factually wrong
    Actionability,     // No clear next steps
    ModalityFit,       // Wrong modality for content
    SourceCoverage,    // Claims without sources
    Consistency,       // Internal contradictions
    Staleness,         // Outdated information
    Usability,         // Hard to use/navigate
    Performance,       // Slow/expensive
    Correctness,       // Bugs/errors
}

pub struct ImprovementOpportunity {
    pub id: OpportunityId,
    pub type: OpportunityType,
    pub description: String,
    pub expected_impact: ImpactEstimate,
    pub effort_estimate: EffortEstimate,
    pub confidence: f32,
    pub prerequisites: Vec<Prerequisite>,
}

pub enum OpportunityType {
    AddMissingContent,
    RemoveRedundantContent,
    RestructureContent,
    ChangeModality,
    AddSources,
    OptimizeQuery,
    AddThreshold,
    AddDrillDown,
    SimplifyLanguage,
    AddVisualization,
    FixError,
    UpdateStaleContent,
}
```

### Phase 2: PLAN — Generate Improvement Plan

```rust
pub struct ImprovementPlan {
    pub id: PlanId,
    pub artifact: ArtifactRef,
    pub goal: ImprovementGoal,
    pub steps: Vec<PlanStep>,
    pub estimated_total_impact: ImpactEstimate,
    pub estimated_total_effort: EffortEstimate,
    pub risk_assessment: RiskAssessment,
    pub rollback_plan: RollbackPlan,
}

pub enum ImprovementGoal {
    MaximizeScore { objective: Objective, target: f32 },
    FixIssues { issue_ids: Vec<IssueId> },
    OptimizeForContext { context: ViewingContext },
    ReduceCost { max_cost: CostBudget },
    AdaptToAudience { audience: AudienceProfile },
    Custom { description: String, evaluator: Box<dyn CustomEvaluator> },
}

pub struct PlanStep {
    pub id: StepId,
    pub type: StepType,
    pub description: String,
    pub mutation: Box<dyn Mutation>,
    pub expected_impact: ImpactEstimate,
    pub effort: EffortEstimate,
    pub dependencies: Vec<StepId>,
    pub verification: VerificationSpec,
    pub rollback: RollbackStep,
}

pub enum StepType {
    Mutation,           // Apply a mutation
    Verification,       // Run verification
    Evaluation,         // Re-evaluate
    HumanReview,        // Request human input
    ExternalAction,     // Call external tool
    Wait,               // Wait for condition
}

pub struct VerificationSpec {
    pub type: VerificationType,
    pub criteria: Vec<VerificationCriterion>,
    pub pass_threshold: f32,
}

pub enum VerificationType {
    AutomatedTest,      // Run test suite
    LLMJudge,           // LLM evaluates
    MetricCheck,        // Check metrics
    HumanApproval,      // Human reviews
    RegressionTest,     // Run regression suite
    A_BTest,            // Compare with baseline
}
```

### Phase 3: ACT — Execute Plan

```rust
pub struct ExecutionEngine {
    pub executor: Box<dyn MutationExecutor>,
    pub verifier: Box<dyn Verifier>,
    pub monitor: Box<dyn ExecutionMonitor>,
    pub budget_enforcer: BudgetEnforcer,
}

pub trait MutationExecutor: Send + Sync {
    fn execute(&self, mutation: &dyn Mutation, artifact: &mut dyn Artifact) -> ExecutionResult;
    fn can_execute(&self, mutation: &dyn Mutation) -> bool;
}

pub struct ExecutionResult {
    pub success: bool,
    pub artifact_diff: ArtifactDiff,
    pub side_effects: Vec<SideEffect>,
    pub execution_time_ms: u64,
    pub resources_consumed: ResourceUsage,
    pub errors: Vec<ExecutionError>,
}

pub struct ExecutionMonitor {
    pub on_step_start: Box<dyn Fn(&PlanStep) + Send + Sync>,
    pub on_step_complete: Box<dyn Fn(&PlanStep, &ExecutionResult) + Send + Sync>,
    pub on_verification_complete: Box<dyn Fn(&VerificationResult) + Send + Sync>,
    pub on_budget_warning: Box<dyn Fn(&BudgetStatus) + Send + Sync>,
    pub on_error: Box<dyn Fn(&ExecutionError) + Send + Sync>,
}
```

### Phase 4: VERIFY — Confirm Improvement

```rust
pub struct VerificationResult {
    pub passed: bool,
    pub score_delta: ScoreDelta,
    pub criteria_results: Vec<CriterionResult>,
    pub regression_test_results: Vec<RegressionTestResult>,
    pub human_feedback: Option<HumanFeedback>,
    pub confidence: f32,
}

pub struct CriterionResult {
    pub criterion: VerificationCriterion,
    pub passed: bool,
    pub actual_value: f32,
    pub expected_value: f32,
    pub margin: f32,
}

pub struct RegressionTestResult {
    pub test_name: String,
    pub passed: bool,
    pub baseline_score: f32,
    pub current_score: f32,
    pub delta: f32,
}

pub struct HumanFeedback {
    pub approved: bool,
    pub rating: Option<u8>,        // 1-5
    pub comments: String,
    suggested_changes: Vec<String>,
}
```

### Phase 5: LEARN — Update Knowledge

```rust
pub struct LearningEngine {
    pub pattern_extractor: PatternExtractor,
    pub rule_learner: RuleLearner,
    pub preference_updater: PreferenceUpdater,
    pub anti_pattern_db: AntiPatternDatabase,
    pub success_pattern_db: SuccessPatternDatabase,
}

pub struct LearnedPattern {
    pub id: PatternId,
    pub pattern_type: PatternType,
    pub description: String,
    pub conditions: Vec<Condition>,
    pub action: ActionTemplate,
    pub confidence: f32,
    pub usage_count: u32,
    pub success_rate: f32,
    pub contexts: Vec<Context>,
    pub created_at: Timestamp,
    pub last_used: Timestamp,
}

pub enum PatternType {
    MutationPattern,      // "When X, apply mutation Y"
    EvaluationPattern,    // "When evaluating Z, weight objective W higher"
    VerificationPattern,  // "When verifying X, use method Y"
    ContextPattern,       // "In context C, prefer approach A"
    AntiPattern,          // "Avoid doing X in context C"
    SuccessPattern,       // "Doing X in context C led to outcome O"
}

pub struct LearnedRule {
    pub id: RuleId,
    pub condition: Condition,
    pub action: Action,
    pub priority: u32,
    pub confidence: f32,
    pub source: RuleSource,
    pub exceptions: Vec<Exception>,
}

pub enum RuleSource {
    HumanTaught,
    InducedFromSuccess,
    InducedFromFailure,
    InducedFromObservation,
    DerivedFromPrinciple,
}
```

---

## 3. Budget System (Multi-Dimensional)

```rust
pub struct BudgetEnforcer {
    pub budgets: HashMap<BudgetType, Budget>,
    pub enforcer_mode: EnforcerMode,
}

pub enum BudgetType {
    Time,               // Wall-clock time
    ComputeTokens,      // LLM tokens
    ComputeCycles,      // CPU cycles
    APICalls,           // External API calls
    HumanAttention,     // Human review time
    MonetaryCost,       // USD
    Attention,          // Agent's own attention
}

pub struct Budget {
    pub budget_type: BudgetType,
    pub allocated: f64,
    pub consumed: f64,
    pub reserved: f64,
    pub warning_threshold: f32,    // 0.8 = warn at 80%
    pub hard_limit: bool,          // Hard stop vs soft warning
    pub replenishment: Option<ReplenishmentRule>,
}

pub struct BudgetStatus {
    pub budget_type: BudgetType,
    pub remaining: f64,
    pub utilization: f32,
    pub projected_exhaustion: Option<Timestamp>,
    pub warning: bool,
    pub exhausted: bool,
}
```

---

## 4. Application to Specific Domains

### 4.1 Report Design Improvement Loop

```rust
pub struct ReportImprovementLoop {
    pub report_ref: ReportRef,
    pub loop_config: LoopConfig,
}

impl ReportImprovementLoop {
    pub async fn run(&self) -> LoopResult {
        // 1. REFLECT
        let reflection = self.reflect().await;
        
        // 2. PLAN
        let plan = self.plan(&reflection).await;
        
        // 3. ACT (with hill-climbing)
        let optimized_report = self.hill_climb(&plan).await;
        
        // 4. VERIFY
        let verification = self.verify(&optimized_report).await;
        
        // 5. LEARN
        self.learn(&reflection, &plan, &verification).await;
        
        LoopResult { ... }
    }
    
    async fn reflect(&self) -> Reflection {
        let report = self.load_report().await;
        let evaluation = self.evaluator.evaluate(&report, &ctx).await;
        let issues = self.issue_detector.detect(&report, &evaluation).await;
        let opportunities = self.opportunity_finder.find(&report, &issues).await;
        Reflection { ... }
    }
    
    async fn hill_climb(&self, plan: &ImprovementPlan) -> Report {
        let mut report = self.load_report().await;
        let optimizer = HillClimbOptimizer::new(
            ReportNeighborhoodGenerator::new(),
            ReportEvaluator::new(),
            AcceptanceCriterion::SteepestAscent,
        );
        
        for step in plan.steps {
            if step.type == StepType::Mutation {
                let mutation = step.mutation;
                let result = self.executor.execute(&mutation, &mut report).await;
                if !result.success {
                    self.rollback(&step.rollback).await;
                    break;
                }
                
                // Verify after each mutation
                let verification = self.verifier.verify(&report, &step.verification).await;
                if !verification.passed {
                    self.rollback(&step.rollback).await;
                    break;
                }
            }
        }
        report
    }
}
```

### 4.2 Dashboard Construction Improvement Loop

```rust
pub struct DashboardImprovementLoop {
    pub dashboard_ref: DashboardRef,
    pub target_context: ViewingContext,  // mobile, incident, executive, etc.
}

impl DashboardImprovementLoop {
    async fn reflect(&self) -> Reflection {
        let dashboard = self.load_dashboard().await;
        let eval = self.evaluator.evaluate(&dashboard, &ctx).await;
        
        // Context-specific issues
        let context_issues = match &self.target_context {
            ViewingContext::Mobile { .. } => self.detect_mobile_issues(&dashboard),
            ViewingContext::IncidentOverlay { .. } => self.detect_incident_issues(&dashboard),
            ViewingContext::ExecutiveSummary { .. } => self.detect_executive_issues(&dashboard),
            _ => vec![],
        };
        
        Reflection { issues: eval.issues.extend(context_issues), ... }
    }
    
    async fn plan(&self, reflection: &Reflection) -> ImprovementPlan {
        let mut steps = vec![];
        
        // Prioritize context-critical issues
        for issue in reflection.issues.iter()
            .filter(|i| i.severity >= Severity::High)
            .sorted_by_key(|i| -i.impact_estimate.magnitude) 
        {
            steps.push(self.generate_fix_step(issue));
        }
        
        // Add optimization steps
        if self.loop_config.enable_optimization {
            steps.extend(self.generate_optimization_steps(&reflection));
        }
        
        ImprovementPlan { steps, ... }
    }
}
```

### 4.3 Prompt Engineering Loop

```rust
pub struct PromptImprovementLoop {
    pub prompt_ref: PromptRef,
    pub benchmark_suite: BenchmarkSuite,
}

impl PromptImprovementLoop {
    async fn reflect(&self) -> Reflection {
        let prompt = self.load_prompt().await;
        let results = self.run_benchmark(&prompt).await;
        
        let issues = vec![
            Issue { category: TaskSuccess, severity: High, desc: "Success rate 67% < 80% target", ... },
            Issue { category: TokenEfficiency, severity: Medium, desc: "Avg 2400 tokens/output vs 1500 target", ... },
            Issue { category: Robustness, severity: Medium, desc: "Fails on 3/10 edge cases", ... },
        ];
        
        Reflection { issues, ... }
    }
    
    async fn plan(&self, reflection: &Reflection) -> ImprovementPlan {
        // Generate prompt mutations
        let mut steps = vec![];
        for issue in &reflection.issues {
            match issue.category {
                TaskSuccess => steps.push(Step { 
                    mutation: AddFewShotExamples { examples: self.find_relevant_examples() },
                    verification: VerificationSpec { type: AutomatedTest, ... }
                }),
                TokenEfficiency => steps.push(Step {
                    mutation: CompressPrompt { target_reduction: 0.3 },
                    verification: VerificationSpec { type: MetricCheck, metric: "tokens_per_output", target: 1500 }
                }),
                Robustness => steps.push(Step {
                    mutation: AddEdgeCaseHandling { cases: self.identify_edge_cases() },
                    verification: VerificationSpec { type: AutomatedTest, suite: "edge_cases" }
                }),
            }
        }
        ImprovementPlan { steps, ... }
    }
}
```

### 4.4 Agent Self-Improvement (Meta-Cognitive)

```rust
pub struct AgentSelfImprovementLoop {
    pub agent_state: AgentStateRef,
}

impl AgentSelfImprovementLoop {
    // Runs periodically (e.g., daily) or on trigger
    async fn run_cycle(&self) -> LoopResult {
        // 1. REFLECT on recent behavior
        let reflection = self.reflect_on_behavior().await;
        
        // 2. PLAN improvements
        let plan = self.generate_self_improvement_plan(&reflection).await;
        
        // 3. ACT - apply changes to self
        let result = self.apply_self_changes(&plan).await;
        
        // 4. VERIFY - test against benchmarks
        let verification = self.verify_changes(&result).await;
        
        // 5. LEARN - update self-model
        self.update_self_model(&reflection, &plan, &verification).await;
        
        LoopResult { ... }
    }
    
    async fn reflect_on_behavior(&self) -> Reflection {
        // Analyze recent conversations
        let conversations = self.load_recent_conversations(7 days).await;
        
        let issues = vec![
            // From conversation analysis
            Issue { category: Proactivity, severity: Medium, 
                    desc: "Only 23% of conversations had proactive suggestions", ... },
            Issue { category: InterruptionHandling, severity: Low,
                    desc: "5 interruptions not acknowledged within 1 sentence", ... },
            Issue { category: ModalitySelection, severity: Medium,
                    desc: "Used drill_down when explain would be better (3 cases)", ... },
            Issue { category: Pacing, severity: Low,
                    desc: "Avg chunks/turn 4.2 > target 3.0", ... },
            
            // From benchmark results
            Issue { category: TaskSuccess, severity: High,
                    desc: "Benchmark success rate dropped 5% this week", ... },
            
            // From user feedback
            Issue { category: UserSatisfaction, severity: High,
                    desc: "2 explicit negative feedback items this week", ... },
        ];
        
        Reflection { issues, ... }
    }
}
```

---

## 5. Cross-Loop Learning (Meta-Learning)

```rust
pub struct MetaLearningEngine {
    pub loops: HashMap<LoopType, Box<dyn ImprovementLoop>>,
    pub cross_loop_learner: CrossLoopLearner,
    pub meta_pattern_db: MetaPatternDatabase,
}

impl MetaLearningEngine {
    pub async fn run_all_loops(&self) -> Vec<LoopResult> {
        let mut results = vec![];
        
        // Run loops in parallel
        let handles: Vec<_> = self.loops.values()
            .map(|loop| tokio::spawn(async move { loop.run().await }))
            .collect();
        
        for handle in handles {
            results.push(handle.await.unwrap());
        }
        
        // Cross-loop learning
        self.cross_loop_learner.learn_from_results(&results).await;
        
        results
    }
}

pub struct CrossLoopLearner {
    pub pattern_extractor: CrossLoopPatternExtractor,
    pub transfer_learner: TransferLearner,
}

impl CrossLoopLearner {
    async fn learn_from_results(&self, results: &[LoopResult]) {
        // 1. Extract patterns that worked across loops
        let patterns = self.pattern_extractor.extract(results).await;
        
        for pattern in patterns {
            if pattern.applicable_to.len() > 1 {
                // This pattern works across multiple loop types
                self.transfer_learner.add_transferable_pattern(pattern).await;
            }
        }
        
        // 2. Learn which loop configurations work best
        for result in results {
            if result.success {
                self.record_successful_config(result.loop_type, result.config_used).await;
            }
        }
        
        // 3. Detect anti-patterns that recur
        let anti_patterns = self.detect_recurring_failures(results).await;
        for ap in anti_patterns {
            self.anti_pattern_db.add(ap).await;
        }
    }
}
```

### Cross-Loop Pattern Examples

| Pattern | Source Loop | Transferred To | Description |
|---|---|---|---|
| "Prefer modality switch over content rewrite" | Report | Dashboard | When clarity low, try changing modality first |
| "Reduce detail_level before removing content" | Report | Prompt | Lower detail before cutting content |
| "Add threshold before adding visualization" | Dashboard | Report | Metrics need thresholds before charts add value |
| "Verify after each mutation, not just at end" | Report | Dashboard | Catch regressions early |
| "Optimize for context before optimizing for score" | Dashboard | Report | Mobile/incident context changes objective weights |

---

## 6. Configuration & Governance

```toml
[self_improvement]
enabled = true

# Global budgets
[self_improvement.budgets]
time_per_cycle_minutes = 30
compute_tokens_per_cycle = 50000
api_calls_per_cycle = 100
monetary_cost_usd_per_cycle = 5.0
human_attention_minutes = 10

# Loop-specific configs
[self_improvement.loops.report]
enabled = true
schedule = "0 3 * * *"           # Daily 3am
trigger_on_change = true
max_iterations = 50
time_budget_ms = 60000
objectives = ["clarity", "brevity", "actionability", "completeness"]

[self_improvement.loops.dashboard]
enabled = true
schedule = "0 4 * * *"
max_iterations = 50
time_budget_ms = 60000
context_aware = true

[self_improvement.loops.prompt]
enabled = true
schedule = "0 5 * * 0"           # Weekly
max_iterations = 30
benchmark_suite = "comprehensive"

[self_improvement.loops.agent_self]
enabled = true
schedule = "0 2 * * *"           # Daily 2am
max_iterations = 20
time_budget_ms = 120000

# Learning
[self_improvement.learning]
cross_loop_enabled = true
pattern_min_confidence = 0.7
pattern_min_occurrences = 3
anti_pattern_min_occurrences = 2
success_pattern_min_success_rate = 0.8

# Governance
[self_improvement.governance]
require_human_approval_for = ["agent_self", "policy_change"]
auto_rollback_on_regression = true
max_concurrent_loops = 3
audit_log_retention_days = 90
```

---

## 7. Observability & Audit

```rust
pub struct LoopAuditLogger {
    pub storage: AuditStorage,
}

impl LoopAuditLogger {
    pub async fn log_cycle(&self, cycle: &LoopCycle) {
        let record = LoopAuditRecord {
            cycle_id: cycle.id,
            loop_type: cycle.loop_type,
            started_at: cycle.started_at,
            completed_at: cycle.completed_at,
            reflection: cycle.reflection.clone(),
            plan: cycle.plan.clone(),
            execution_trace: cycle.execution_trace,
            verification: cycle.verification.clone(),
            learning: cycle.learning.clone(),
            outcome: cycle.outcome,
            budgets_consumed: cycle.budgets_consumed,
        };
        self.storage.store(record).await;
    }
}

pub struct LoopCycle {
    pub id: CycleId,
    pub loop_type: LoopType,
    pub started_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub reflection: Reflection,
    pub plan: ImprovementPlan,
    pub execution_trace: Vec<ExecutionTraceEntry>,
    pub verification: VerificationResult,
    pub learning: LearningRecord,
    pub outcome: LoopOutcome,
    pub budgets_consumed: HashMap<BudgetType, f64>,
}

pub enum LoopOutcome {
    Completed { improvement_achieved: f32 },
    Partial { reason: String, improvement_achieved: f32 },
    Failed { reason: String, rolled_back: bool },
    Interrupted { reason: String, progress: f32 },
    Skipped { reason: String },
}
```

---

## 8. Trigger System

```rust
pub enum ImprovementTrigger {
    Scheduled { cron: String },
    OnArtifactChange { artifact_type: ArtifactType, change_threshold: f32 },
    OnMetricRegression { metric: String, threshold: f32 },
    OnUserFeedback { sentiment: Sentiment, min_severity: Severity },
    OnIncident { severity: AlertSeverity },
    OnDeployment { environment: String },
    OnBenchmarkRun { suite: String, regression_threshold: f32 },
    OnUserRequest { explicit: bool },
    OnCrossLoopSignal { from_loop: LoopType, signal: CrossLoopSignal },
    Manual,
}

pub struct TriggerManager {
    pub triggers: Vec<ImprovementTrigger>,
    pub scheduler: Scheduler,
    pub event_bus: EventBus,
}

impl TriggerManager {
    pub async fn evaluate(&self) -> Vec<LoopType> {
        let mut triggered = vec![];
        for trigger in &self.triggers {
            if trigger.should_fire(&self.get_context()).await {
                triggered.push(trigger.target_loop());
            }
        }
        triggered
    }
}
```

---

## 8. Integration Points

| System | Integration |
|---|---|
| **CME** | Improvement loops as proactive plugins; in-conversation refinement |
| **Hill-Climbing** | ACT phase uses hill-climbing; verification uses hill-climb evaluator |
| **Report Generation** | Report as optimizable artifact; loops improve reports |
| **Dashboard Construction** | Dashboard as optimizable artifact; context-aware loops |
| **Proactive Plugins** | Plugins trigger loops; loops generate proactive items |
| **Cross-Session Memory** | Learning stored, retrieved across sessions |
| **Synthetic Testing** | Loops tested against synthetic scenarios |
| **Observability** | Loop cycles logged, traced, metered |

---

## 9. Example: End-to-End Report Improvement

```
Trigger: Scheduled (daily 3am) + OnArtifactChange (report modified > 20%)

Cycle Start: 2026-11-20T03:00:00Z

REFLECT (30s):
  - Load report "Auth Service Deploy Audit"
  - Evaluate: clarity=0.62, brevity=0.45, actionability=0.71, completeness=0.58
  - Issues: 
    * Brevity (High): 3 sections > 200 tokens, could be bullets
    * ModalityFit (Medium): Root cause uses narrative, should be drill_down
    * SourceCoverage (Medium): 3 claims without citations
  - Opportunities:
    * Switch root cause to drill_down (+0.15 clarity)
    * Convert 3 sections to bullets (+0.20 brevity)
    * Add 3 missing citations (+0.10 source_coverage)

PLAN (10s):
  1. change_modality(root_cause, drill_down) [impact: +0.15 clarity]
  2. adjust_detail_level(3 sections, -1) [impact: +0.20 brevity]
  3. change_modality(3 sections, bullets) [impact: +0.20 brevity]
  4. add_citations(3 claims) [impact: +0.10 source_coverage]
  Total estimated: +0.35 composite, 4 mutations, ~45s

ACT (45s):
  - Mutation 1: change_modality(root_cause, drill_down, detail=2) ✓ verified
  - Mutation 2: adjust_detail_level(section_3, -1) ✓ verified
  - Mutation 3: change_modality(section_5, bullets) ✓ verified
  - Mutation 4: add_citations(3 claims from deploy logs) ✓ verified

VERIFY (15s):
  - Re-evaluate: clarity=0.78 (+0.16), brevity=0.72 (+0.27), actionability=0.73 (+0.02), completeness=0.68 (+0.10), source_coverage=0.75 (+0.17)
  - Composite: 0.62 → 0.79 (+0.17, 27% improvement)
  - Regression tests: all pass
  - Human review: not required (confidence 0.92 > 0.8)

LEARN (5s):
  - Pattern: "When root cause in narrative, switch to drill_down first" (confidence 0.87)
  - Pattern: "Brevity gains from modality switch > detail reduction" (confidence 0.82)
  - Update ReportNeighborhoodGenerator mutation priorities
  - Update ReportEvaluator objective weights (brevity +0.05)

Cycle Complete: 2026-11-20T03:01:35Z
Outcome: Completed, improvement_achieved=0.17
```

---

## 10. Summary: Loop Characteristics by Domain

| Domain | Artifact | Reflection Focus | Typical Mutations | Verification | Cycle Time |
|---|---|---|---|---|---|
| **Report** | ReportDesign | Clarity, brevity, modality fit, sources | Modality switch, detail adjust, citation add, restructure | LLM judge + regression tests | 1–2 min |
| **Dashboard** | DashboardLayout | Cognitive load, cost, mobile fitness, actionability | Panel add/remove/move, query optimize, threshold add | Metric check + regression | 1–2 min |
| **Prompt** | PromptTemplate | Task success, tokens, robustness, cost | Example add/remove, compression, edge case handling | Benchmark suite | 30–60s |
| **Agent Self** | AgentState | Proactivity, pacing, modality, user satisfaction | Behavior policy change, preference update | Benchmark + human feedback | 2–5 min |

---

*The self-improvement loop turns "make it better" into a universal, measurable, auditable, and composable process — applicable to any artifact the agent creates or any behavior it exhibits.*