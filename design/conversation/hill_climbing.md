# Hill-Climbing Optimization Engine

> A general-purpose hill-climbing optimizer for iterative improvement of artifacts, configurations, policies, and behaviors. Designed to be compositional and applicable across domains: report design, dashboard layout, policy tuning, prompt optimization, etc.

---

## 1. Core Concept

Hill-climbing is a simple but powerful optimization paradigm:

```
Current State → Generate Neighbors → Evaluate → Select Best → Repeat
```

**Key properties for our use case:**
- **Anytime algorithm**: Can stop at any iteration and return current best
- **Composable**: Works on any artifact with a neighborhood function and evaluation
- **Explainable**: Each step has a clear rationale (why this neighbor?)
- **Configurable**: Neighborhood, evaluation, acceptance criteria all pluggable

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    HILL-CLIMBING ENGINE                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐     │
│  │   ARTIFACT   │───▶│  NEIGHBORHOOD│───▶│  EVALUATOR   │     │
│  │   (State)    │    │  GENERATOR   │    │  (Objective) │     │
│  └──────────────┘    └──────────────┘    └──────┬───────┘     │
│                                                  │             │
│                                                  ▼             │
│                                          ┌──────────────┐     │
│                                          │  ACCEPTANCE  │     │
│                                          │   CRITERION  │     │
│                                          └──────┬───────┘     │
│                                                 │             │
│                                      ┌──────────┴──────────┐  │
│                                      ▼                     ▼  │
│                                 ACCEPTED              REJECTED │
│                                      │                     │   │
│                                      ▼                     │   │
│                              ┌──────────────┐              │   │
│                              │  NEW STATE   │              │   │
│                              └──────┬───────┘              │   │
│                                     │                      │   │
│                                     └──────────────────────┘   │
│                                                   (loop)         │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. Core Types

```rust
// The thing being optimized (report, dashboard, policy, prompt, etc.)
pub trait Optimizable: Clone + Send + Sync {
    type Config;
    type Metadata;
    
    fn initial(config: Self::Config) -> Self;
    fn metadata(&self) -> Self::Metadata;
}

// Generates neighboring states
pub trait NeighborhoodGenerator<T: Optimizable>: Send + Sync {
    fn neighbors(&self, current: &T, context: &OptimizationContext) -> Vec<T>;
    fn neighborhood_size(&self) -> usize;
}

// Evaluates a state (can be multi-objective)
pub trait Evaluator<T: Optimizable>: Send + Sync {
    type Score: Score;
    
    fn evaluate(&self, candidate: &T, context: &OptimizationContext) -> Self::Score;
    fn objectives(&self) -> Vec<Objective>;
}

// Decides whether to accept a neighbor
pub trait AcceptanceCriterion<T: Optimizable, S: Score>: Send + Sync {
    fn accept(&self, current: &T, current_score: &S, candidate: &T, candidate_score: &S, 
              context: &OptimizationContext) -> AcceptanceDecision;
}

pub struct OptimizationContext {
    pub iteration: usize,
    pub max_iterations: usize,
    pub time_budget_ms: Option<u64>,
    pub artifacts: HashMap<String, ArtifactRef>,
    pub constraints: Vec<Constraint>,
    pub history: OptimizationHistory,
}

pub enum AcceptanceDecision {
    Accept,
    Reject,
    AcceptWithProbability(f32),  // For simulated annealing
}

pub struct OptimizationHistory {
    pub states: Vec<StateSnapshot>,
    pub best_state: Option<StateSnapshot>,
    pub score_trajectory: Vec<ScoreSnapshot>,
}

pub struct StateSnapshot {
    pub iteration: usize,
    pub state_hash: String,
    pub score: ScoreSnapshot,
    pub accepted: bool,
    pub neighbor_type: String,
}
```

---

## 4. Hill-Climbing Variants

### 4.1 Steepest Ascent (Default)
```rust
pub struct SteepestAscent<T: Optimizable> {
    pub generator: Box<dyn NeighborhoodGenerator<T>>,
    pub evaluator: Box<dyn Evaluator<T>>,
    pub criterion: Box<dyn AcceptanceCriterion<T, <Evaluator>::Score>>,
}

impl<T: Optimizable> Optimizer<T> for SteepestAscent<T> {
    fn step(&mut self, current: &T, context: &mut OptimizationContext) -> Result<T> {
        let neighbors = self.generator.neighbors(current, context);
        let mut best = current.clone();
        let mut best_score = self.evaluator.evaluate(current, context);
        
        for neighbor in neighbors {
            let score = self.evaluator.evaluate(&neighbor, context);
            if self.criterion.accept(current, &best_score, &neighbor, &score, context) == AcceptanceDecision::Accept {
                if score.better_than(&best_score) {
                    best = neighbor;
                    best_score = score;
                }
            }
        }
        Ok(best)
    }
}
```

### 4.2 First Improvement (Faster)
```rust
pub struct FirstImprovement<T: Optimizable> { ... }
// Accepts first neighbor that improves over current
```

### 4.3 Stochastic Hill-Climbing (Escape Local Optima)
```rust
pub struct StochasticHillClimb<T: Optimizable> {
    pub temperature_schedule: TemperatureSchedule,
    // Accepts worse neighbors with probability e^(-ΔE/T)
}
```

### 4.4 Multi-Objective (Pareto)
```rust
pub struct ParetoHillClimb<T: Optimizable> {
    // Maintains Pareto frontier of non-dominated solutions
    pub frontier: ParetoFrontier<<Evaluator>::Score>,
}
```

---

## 5. Domain-Specific Instantiations

### 5.1 Report Design Optimization
```rust
pub struct ReportDesign {
    pub sections: Vec<ReportSection>,
    pub layout: LayoutConfig,
    pub styling: StylingConfig,
    pub modality_sequence: Vec<ModalitySpec>,
}

pub struct ReportNeighborhoodGenerator {
    // Mutation operators:
    // - add_section, remove_section, reorder_sections
    // - change_modality(section, new_modality)
    // - adjust_detail_level(section, delta)
    // - add/remove_source_citation(section)
    // - split_section, merge_sections
    // - change_heading_style
}

pub struct ReportEvaluator {
    // Objectives (weighted):
    // - clarity_score: LLM-based readability assessment
    // - completeness: coverage of required topics
    // - actionability: presence of next-steps, decisions
    // - brevity: token count vs information density
    // - modality_appropriateness: match between content and modality
    // - source_coverage: claims backed by sources
    // - user_preference_alignment: matches learned prefs
}
```

### 5.2 Dashboard Layout Optimization
```rust
pub struct DashboardLayout {
    pub panels: Vec<Panel>,
    pub grid: GridConfig,
    pub data_sources: Vec<DataSourceRef>,
    pub interactions: Vec<InteractionSpec>,
}

pub struct DashboardNeighborhoodGenerator {
    // - add_panel, remove_panel, move_panel
    // - resize_panel, change_panel_type
    // - add/remove_filter, change_time_range
    // - reorder_panels, change_grid_layout
    // - add_cross_panel_link, add_drill_down
}

pub struct DashboardEvaluator {
    // - information_density: insights per pixel
    // - cognitive_load: panels, metrics, complexity
    // - query_cost: total API/DB calls per refresh
    // - latency_p99: worst-case panel load time
    // - actionability: panels with clear thresholds/alerts
    // - consistency: naming, units, color coding
    // - mobile_fitness: readability on small screens
}
```

### 5.3 Policy/Configuration Tuning
```rust
pub struct PolicyConfig {
    pub parameters: HashMap<String, ParameterValue>,
    pub rules: Vec<Rule>,
}

pub struct PolicyEvaluator {
    // - effectiveness: outcome improvement rate
    // - stability: variance in outcomes
    // - cost: resource consumption
    // - compliance: constraint satisfaction
    // - user_satisfaction: implicit/explicit feedback
}
```

### 5.4 Prompt Optimization
```rust
pub struct PromptTemplate {
    pub sections: Vec<PromptSection>,
    pub variables: Vec<VariableSpec>,
    pub few_shot_examples: Vec<Example>,
}

pub struct PromptEvaluator {
    // - task_success_rate: on benchmark suite
    // - token_efficiency: output quality per input token
    // - robustness: variance across input variations
    // - latency: time to first token
    // - cost: $ per 1k calls
}
```

---

## 6. Integration with CME

### 6.1 As Proactive Plugin
```rust
pub struct HillClimbPlugin {
    pub optimizer: Box<dyn Optimizer<ReportDesign>>,
    pub trigger: ProactiveTrigger::Scheduled { cron: "0 2 * * *" },
    pub config: HillClimbPluginConfig,
}

impl ProactivePlugin for HillClimbPlugin {
    async fn evaluate(&self, ctx: &ProactiveContext) -> Vec<ProactiveItem> {
        if let Some(report) = ctx.current_report_design() {
            let optimized = self.optimizer.optimize(report, OptimizationConfig {
                max_iterations: 50,
                time_budget_ms: 30_000,
            }).await;
            
            if optimized.improvement > 0.1 {
                vec![ProactiveItem::Suggestion {
                    title: "Report design improved".into(),
                    description: format!("Clarity +{:.0}%, brevity +{:.0}%", 
                        optimized.clarity_delta * 100.0, optimized.brevity_delta * 100.0),
                    action: ActionRef::new("apply_report_optimization", json!({ "design": optimized })),
                }]
            } else { vec![] }
        } else { vec![] }
    }
}
```

### 6.2 In-Conversation Optimization
```
User: "This report is too verbose"
Agent: "I'll optimize it for brevity while preserving key insights..."
[Runs hill-climb with brevity-weighted evaluator]
Agent: "Optimized! Reduced 40% tokens, kept all key decisions and sources.
        Changes: merged 3 sections, switched 2 to bullets modality, 
        removed redundant context. Want to review diff?"
```

### 6.3 Dashboard Auto-Layout
```
User: "Make this dashboard work on mobile"
Agent: [Runs hill-climb with mobile_fitness objective]
Agent: "Done. Reordered panels by priority, collapsed secondary metrics 
        into expandable drawers, increased font sizes, switched to 
        single-column grid. Query cost reduced 30%. Preview?"
```

---

## 7. Convergence & Stopping Criteria

```rust
pub enum StoppingCriterion {
    MaxIterations(usize),
    TimeBudget(Duration),
    NoImprovementFor(usize),      // Plateau detection
    ScoreThreshold(Score),         // Target reached
    ConvergenceRate(f32),          // Δscore/iteration < ε
    BudgetExhausted,               // Token/compute budget
    UserInterrupted,               // External signal
}

pub struct ConvergenceMonitor {
    pub plateau_window: usize,
    pub min_improvement: f32,
    pub convergence_history: VecDeque<f32>,
}
```

---

## 8. Explainability & Audit

Every step produces an audit record:
```json
{
  "iteration": 23,
  "neighbor_type": "change_modality",
  "target_section": "deployment_metrics",
  "change": "modality: narrative → bullets (detail_level=2)",
  "score_before": { "clarity": 0.62, "brevity": 0.45, "actionability": 0.71 },
  "score_after":  { "clarity": 0.71, "brevity": 0.78, "actionability": 0.73 },
  "accepted": true,
  "rationale": "Bullets modality improves scanability for metrics-heavy section; detail_level=2 preserves context"
}
```

---

## 9. Configuration

```toml
[hill_climb]
default_variant = "steepest_ascent"
default_max_iterations = 100
default_time_budget_ms = 30000

[hill_climb.neighborhood]
max_neighbors_per_step = 20
mutation_probability = 0.3
crossover_probability = 0.1

[hill_climb.evaluation]
parallel_evaluation = true
cache_scores = true
llm_evaluator_model = "gpt-4o-mini"

[hill_climb.acceptance]
allow_equal = true
allow_worse_probability = 0.0  # 0 = strict hill-climb
simulated_annealing = false

[hill_climb.convergence]
plateau_window = 10
min_improvement = 0.005
score_threshold = null
```

---

## 10. Applications Summary

| Domain | Artifact | Key Objectives | Typical Iterations |
|---|---|---|---|
| **Report Design** | ReportDesign | clarity, brevity, actionability, completeness | 20–50 |
| **Dashboard Layout** | DashboardLayout | info_density, cognitive_load, query_cost, mobile_fitness | 30–80 |
| **Policy Tuning** | PolicyConfig | effectiveness, stability, cost, compliance | 50–200 |
| **Prompt Engineering** | PromptTemplate | task_success, token_efficiency, robustness, cost | 10–30 |
| **Modality Selection** | ModalitySequence | appropriateness, user_preference, token_efficiency | 5–15 |

---

*The hill-climbing engine turns "make it better" into a measurable, auditable, interruptible process — applicable to any artifact with a neighborhood and an evaluation function.*