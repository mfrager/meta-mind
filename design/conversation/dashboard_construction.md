# Dashboard Construction Engine

> A composable, query-aware dashboard builder that produces layouts optimized for information density, cognitive load, and actionability — hill-climbable for different contexts (desktop, mobile, incident, executive).

---

## 1. Core Concept

Dashboards are **living interfaces** — not static JSON. They have:
- **Structured layout model** (grid, panels, data sources, interactions)
- **Query-aware cost model** (each panel knows its latency, cost, freshness)
- **Context-aware rendering** (desktop, mobile, TV/wallboard, incident overlay)
- **Actionability baked in** (thresholds, alerts, drill-downs, runbooks)
- **Hill-climbable** (layout, panel selection, query optimization)

---

## 2. Dashboard Schema

```rust
pub struct Dashboard {
    pub id: DashboardId,
    pub title: String,
    pub description: Option<String>,
    pub metadata: DashboardMetadata,
    pub layout: LayoutConfig,
    pub panels: Vec<Panel>,
    pub data_sources: Vec<DataSourceRef>,
    pub global_filters: Vec<GlobalFilter>,
    pub cross_panel_links: Vec<CrossPanelLink>,
    pub alerting_rules: Vec<AlertingRule>,
    pub generated_at: Timestamp,
    pub generator_version: String,
}

pub struct DashboardMetadata {
    pub dashboard_type: DashboardType,
    pub context: DashboardContext,
    pub intended_audience: AudienceProfile,
    pub viewing_contexts: Vec<ViewingContext>,  // desktop, mobile, tv, incident
    pub refresh_policy: RefreshPolicy,
    pub cost_budget: CostBudget,
}

pub enum DashboardType {
    Operational,       // Real-time monitoring
    Incident,          // War-room / on-call
    Executive,         // KPI summary
    Debugging,         // Deep-dive troubleshooting
    Business,          // Business metrics
    Security,          // Security posture
    Custom(String),
}

pub enum DashboardContext {
    RealTime,          // Sub-second to seconds
    NearRealTime,      // Minutes
    Periodic,          // Hourly/daily
    OnDemand,          // User-triggered
}

pub struct LayoutConfig {
    pub grid: GridSpec,
    pub breakpoints: Breakpoints,
    pub density: LayoutDensity,
    pub panel_spacing: Spacing,
    pub container: ContainerType,
}

pub struct GridSpec {
    pub columns: usize,           // 12-col default
    pub rows: Option<usize>,      // Auto if None
    pub row_height_px: u32,       // Base row height
    pub gap_px: u32,
}

pub struct Breakpoints {
    pub mobile_max_px: u32,       // < 640
    pub tablet_max_px: u32,       // < 1024
    pub desktop_max_px: u32,      // < 1440
    pub wide_min_px: u32,         // >= 1440
}

pub enum LayoutDensity {
    Compact,     // Minimal spacing, max panels
    Comfortable, // Balanced
    Spacious,    // Room to breathe, fewer panels
}

pub enum ContainerType {
    FixedWidth { max_width_px: u32 },
    FluidWidth { min_width_px: u32, max_width_px: Option<u32> },
}
```

---

## 3. Panel System

```rust
pub struct Panel {
    pub id: PanelId,
    pub title: String,
    pub panel_type: PanelType,
    pub position: GridPosition,
    pub size: GridSize,
    pub data_source: DataSourceRef,
    pub query: QuerySpec,
    pub visualization: VisualizationSpec,
    pub thresholds: Vec<Threshold>,
    pub interactions: Vec<InteractionSpec>,
    pub refresh: PanelRefresh,
    pub metadata: PanelMetadata,
}

pub struct GridPosition {
    pub col_start: usize,     // 0-indexed
    pub row_start: usize,
}

pub struct GridSize {
    pub col_span: usize,      // 1-12
    pub row_span: usize,
    pub min_width_px: Option<u32>,
    pub min_height_px: Option<u32>,
}

pub enum PanelType {
    TimeSeries,           // Line/area chart
    Stat,                 // Single value + sparkline
    Gauge,                // Gauge with thresholds
    BarChart,             // Bar/column chart
    Heatmap,              // 2D heatmap
    Table,                // Tabular data
    Logs,                 // Log stream
    Traces,               // Trace waterfall
    Flamegraph,           // CPU/memory profile
    Map,                  // Geographic
    Sankey,               // Flow diagram
    Topology,             // Service graph
    Markdown,             // Static content
    Iframe,               // Embedded URL
    Custom(String),
}

pub struct QuerySpec {
    pub query: String,              // PromQL, SQL, LogQL, etc.
    pub variables: HashMap<String, String>,
    pub time_range: TimeRange,
    pub step: Option<Duration>,
    pub max_data_points: Option<usize>,
    pub transformations: Vec<QueryTransformation>,
}

pub struct VisualizationSpec {
    pub viz_type: VisualizationType,
    pub config: VisualizationConfig,
    pub color_scheme: ColorScheme,
    pub axes: Option<AxesConfig>,
    pub legend: LegendConfig,
    pub tooltip: TooltipConfig,
}

pub enum VisualizationType {
    Line,
    Area,
    Bar,
    StackedBar,
    Scatter,
    Heatmap,
    Gauge,
    SingleStat,
    Table,
    Logs,
    Flamegraph,
    Topology,
    Map,
    Sankey,
}

pub struct Threshold {
    pub field: String,
    pub operator: ThresholdOperator,
    pub value: f64,
    pub severity: AlertSeverity,
    pub label: String,
    pub action: Option<ThresholdAction>,
}

pub enum ThresholdOperator { Gt, Gte, Lt, Lte, Eq, Ne, Between, Outside }

pub struct ThresholdAction {
    pub action_type: ActionType,
    pub config: HashMap<String, serde_json::Value>,
}

pub enum ActionType {
    Alert,              // Fire alert
    DrillDown,          // Navigate to detail dashboard
    Runbook,            // Open runbook
    Annotation,         // Add annotation to chart
    ColorChange,        // Change series color
    HideSeries,         // Hide series
    ScaleChange,        // Log/linear scale toggle
}

pub enum InteractionSpec {
    ClickDrillDown { target_dashboard: DashboardId, carry_filters: bool },
    HoverDetail { fields: Vec<String> },
    BrushZoom { x_axis: bool, y_axis: bool },
    ContextMenu { actions: Vec<ContextAction> },
    DragToPanel { target_panel: PanelId },
    VariableAssignment { variable: String, field: String },
}

pub struct PanelRefresh {
    pub interval: Duration,
    pub strategy: RefreshStrategy,
    pub priority: RefreshPriority,
}

pub enum RefreshStrategy {
    Polling,
    Streaming,          // WebSocket/SSE
    OnDemand,
    Adaptive,           // Based on data volatility
}

pub enum RefreshPriority { Critical, High, Normal, Low, Background }
```

---

## 4. Cost-Aware Query Model

```rust
pub struct QueryCostModel {
    pub estimated_latency_p50_ms: u32,
    pub estimated_latency_p99_ms: u32,
    pub estimated_cost_usd_per_query: f64,
    pub estimated_rows_scanned: u64,
    pub estimated_bytes_processed: u64,
    pub cacheability: Cacheability,
    pub volatility: DataVolatility,
}

pub enum Cacheability {
    HighlyCacheable,    // Static dimensions, rare updates
    ModeratelyCacheable,
    LowCacheability,    // High-cardinality, frequent updates
    Uncacheable,        // Real-time, unique per query
}

pub enum DataVolatility {
    Static,             // Never changes
    Low,                // Changes hourly
    Medium,             // Changes minutely
    High,               // Changes per second
    RealTime,           // Sub-second changes
}
```

### Cost Budget
```rust
pub struct CostBudget {
    pub max_total_latency_p99_ms: u32,
    pub max_total_cost_usd_per_hour: f64,
    pub max_concurrent_queries: usize,
    pub max_rows_per_second: u64,
    pub panel_budgets: HashMap<PanelId, PanelCostBudget>,
}

pub struct PanelCostBudget {
    pub max_latency_p99_ms: u32,
    pub max_cost_usd_per_hour: f64,
    pub max_data_points: usize,
}
```

---

## 5. Viewing Contexts (Responsive + Contextual)

```rust
pub struct DashboardRenderer {
    pub layout_engine: LayoutEngine,
    pub viewport: Viewport,
    pub context: ViewingContext,
}

pub struct Viewport {
    pub width_px: u32,
    pub height_px: u32,
    pub device_pixel_ratio: f32,
    pub orientation: Orientation,
}

pub enum ViewingContext {
    Desktop { density: LayoutDensity },
    Tablet { density: LayoutDensity },
    Mobile { density: LayoutDensity },
    TVWallboard { 
        viewing_distance_m: f32,
        font_scale: f32,
    },
    IncidentOverlay {
        triggering_alert: AlertRef,
        focus_panels: Vec<PanelId>,
    },
    ExecutiveSummary {
        max_panels: usize,
        kpi_focus: bool,
    },
    Embedded {
        container_width_px: u32,
        container_height_px: u32,
    },
}

impl DashboardRenderer {
    pub fn render(&self, dashboard: &Dashboard) -> RenderedDashboard {
        let effective_layout = self.layout_engine.compute_layout(
            &dashboard.layout,
            &dashboard.panels,
            &self.viewport,
            &self.context,
        );
        
        let rendered_panels = dashboard.panels.par_iter()
            .map(|panel| self.render_panel(panel, &effective_layout))
            .collect();
        
        RenderedDashboard {
            layout: effective_layout,
            panels: rendered_panels,
            global_filters: self.render_global_filters(&dashboard.global_filters),
            cross_panel_links: self.render_cross_panel_links(&dashboard.cross_panel_links),
            metadata: RenderMetadata {
                viewport: self.viewport.clone(),
                context: self.context.clone(),
                rendered_at: now(),
            },
        }
    }
}
```

### 5.1 Layout Adaptation per Context

| Context | Grid | Panel Sizing | Interactions | Density |
|---|---|---|---|---|
| **Desktop** | 12-col | Full sizing | Hover, click, drag, context menu | Comfortable |
| **Tablet** | 8-col | Touch-friendly | Tap, long-press, swipe | Comfortable |
| **Mobile** | 4-col | Full-width stack | Tap, swipe, pull-to-refresh | Compact |
| **TV/Wallboard** | 12-col | Large fonts, high contrast | None (view-only) | Spacious |
| **Incident** | 6-col | Focus panels enlarged | Minimal | Compact |
| **Executive** | 4-col | KPI stats prominent | Minimal | Spacious |

---

## 6. Hill-Climbing Integration

```rust
impl Optimizable for Dashboard {
    type Config = DashboardGenerationConfig;
    type Metadata = DashboardMetadata;
}

pub struct DashboardNeighborhoodGenerator {
    mutations: Vec<Box<dyn DashboardMutation>>,
}

pub trait DashboardMutation: Send + Sync {
    fn apply(&self, dashboard: &mut Dashboard) -> MutationResult;
    fn description(&self) -> String;
    fn cost_impact(&self) -> CostImpact;
}

pub struct AddPanel { /* panel_template */ }
pub struct RemovePanel { /* panel_id */ }
pub struct MovePanel { /* panel_id, new_position */ }
pub struct ResizePanel { /* panel_id, new_size */ }
pub struct ChangePanelType { /* panel_id, new_type */ }
pub struct ChangeVisualization { /* panel_id, new_viz_config */ }
pub struct AddThreshold { /* panel_id, threshold */ }
pub struct RemoveThreshold { /* panel_id, threshold_id */ }
pub struct AddCrossPanelLink { /* source_panel, target_panel, link_type */ }
pub struct AddGlobalFilter { /* filter_spec */ }
pub struct ReorderPanels { /* new_order */ }
pub struct ChangeGridLayout { /* new_grid_spec */ }
pub struct OptimizeQuery { /* panel_id, optimization_hint */ }
pub struct AddDrillDown { /* source_panel, target_dashboard */ }
pub struct ChangeRefreshInterval { /* panel_id, new_interval */ }
pub struct EnableStreaming { /* panel_id */ }
pub struct AddGlobalVariable { /* variable_spec */ }
```

### 6.1 Dashboard Evaluator (Multi-Objective)

```rust
pub struct DashboardEvaluator {
    objectives: Vec<Objective>,
}

pub enum Objective {
    InformationDensity { weight: f32 },      // Insights per pixel
    CognitiveLoad { weight: f32 },           // Panel count, metric count, complexity
    QueryCost { weight: f32 },               // Total $/hour, latency
    LatencyP99 { weight: f32 },              // Worst-case panel load
    Actionability { weight: f32 },           // Panels with thresholds/alerts/drill-downs
    MobileFitness { weight: f32 },           // Mobile readability score
    Consistency { weight: f32 },             // Naming, units, colors, formatting
    AlertCoverage { weight: f32 },           // Critical metrics have thresholds
    DataFreshness { weight: f32 },           // Max staleness across panels
    Custom { name: String, evaluator: Box<dyn CustomEvaluator>, weight: f32 },
}

pub struct DashboardScore {
    pub objectives: HashMap<String, f32>,
    pub composite: f32,
    pub Pareto_frontier_rank: Option<usize>,
}

impl Evaluator<Dashboard> for DashboardEvaluator {
    type Score = DashboardScore;
    
    fn evaluate(&self, dashboard: &Dashboard, ctx: &OptimizationContext) -> DashboardScore {
        let mut scores = HashMap::new();
        for obj in &self.objectives {
            scores.insert(obj.name(), obj.score(dashboard, ctx));
        }
        let composite = self.weighted_sum(&scores);
        DashboardScore { objectives: scores, composite, pareto_frontier_rank: None }
    }
}
```

### 6.2 Mutation Examples

```rust
// Mobile optimization mutation
pub struct OptimizeForMobile {
    pub target_context: ViewingContext::Mobile,
}
impl DashboardMutation for OptimizeForMobile {
    fn apply(&self, dashboard: &mut Dashboard) -> MutationResult {
        let mut changes = vec![];
        
        // 1. Switch to 4-col grid
        changes.push(ChangeGridLayout { 
            new_grid: GridSpec { columns: 4, ..default() } 
        });
        
        // 2. Stack panels vertically (col_span = 4)
        for panel in &mut dashboard.panels {
            if panel.size.col_span > 4 {
                changes.push(ResizePanel { 
                    panel_id: panel.id, 
                    new_size: GridSize { col_span: 4, ..panel.size } 
                });
            }
        }
        
        // 3. Increase font sizes for panels
        // (handled by renderer via context)
        
        // 4. Collapse secondary panels into drawers
        for panel in &dashboard.panels {
            if panel.metadata.importance == ImportanceLevel::Low {
                changes.push(AddInteraction {
                    panel_id: panel.id,
                    interaction: InteractionSpec::ClickDrillDown { 
                        target_dashboard: format!("{}_detail", panel.id),
                        carry_filters: true 
                    }
                });
            }
        }
        
        MutationResult { changes, cost_impact: CostImpact::ReducesCost }
    }
}

// Cost optimization mutation
pub struct OptimizeQueryCosts {
    pub max_total_cost_usd_per_hour: f64,
}
impl DashboardMutation for OptimizeQueryCosts {
    fn apply(&self, dashboard: &mut Dashboard) -> MutationResult {
        let mut changes = vec![];
        let mut current_cost = dashboard.estimate_hourly_cost();
        
        for panel in &dashboard.panels {
            if current_cost <= self.max_total_cost_usd_per_hour { break; }
            
            // 1. Increase refresh interval for low-priority panels
            if panel.refresh.priority == RefreshPriority::Low 
                && panel.refresh.interval < Duration::minutes(5) {
                changes.push(ChangeRefreshInterval {
                    panel_id: panel.id,
                    new_interval: Duration::minutes(5),
                });
                current_cost *= 0.9;
            }
            
            // 2. Reduce data points for high-resolution time series
            if panel.query.max_data_points.unwrap_or(1000) > 500 {
                changes.push(OptimizeQuery {
                    panel_id: panel.id,
                    hint: QueryOptimizationHint::ReduceDataPoints(500),
                });
                current_cost *= 0.95;
            }
            
            // 3. Switch to cached query for static-ish data
            if panel.cost_model.cacheability == Cacheability::HighlyCacheable {
                changes.push(ChangeQuery {
                    panel_id: panel.id,
                    new_query: add_cache_hints(&panel.query),
                });
                current_cost *= 0.5;
            }
        }
        
        MutationResult { changes, cost_impact: CostImpact::ReducesCost }
    }
}
```

---

## 7. In-Conversation Dashboard Operations

### 7.1 Create Dashboard
```
User: "Build me a dashboard for the auth service — latency, errors, throughput, cache hit rate"
Agent: [Creates dashboard with 4 panels, appropriate visualizations, thresholds from SLOs]
Agent: "Done. 4-panel operational dashboard: latency (p50/p95/p99), error rate, throughput, cache hit rate. 
        All with SLO thresholds. Refresh: 15s. Mobile-ready. Want drill-downs for each?"
```

### 7.2 Contextual Adaptation
```
User: "Make it work for on-call — I'll view this on my phone at 3am"
Agent: [Applies OptimizeForMobile mutation]
Agent: "Done. Stacked to single column, enlarged fonts, secondary metrics collapsed into drawers. 
        Tap any panel for full-screen drill-down. Query cost reduced 40%. 
        Incident overlay mode: tap alert → auto-focus relevant panel."
```

### 7.3 Cost Optimization
```
User: "This dashboard is too expensive — cut the query cost in half"
Agent: [Runs OptimizeForCost with max_total_cost_usd_per_hour = current/2]
Agent: "Done. Cut cost 52%: increased refresh intervals on 3 low-priority panels, 
        reduced data points on 2 time series, enabled caching for 2 static queries. 
        Latency p99 improved from 2.1s → 800ms. Mobile fitness improved 15%. 
        Review changes?"
```

### 7.4 Incident Mode
```
Alert fires → Agent auto-switches dashboard to incident mode:
Agent: "Incident detected: auth-service latency p99 > 2s. Switching to incident overlay."
[Dashboard context switches to IncidentOverlay:
  - Focus panels: latency, error rate, active connections
  - Other panels collapsed
  - Auto-refresh 5s
  - Runbook links injected
  - Cross-panel drill-downs to traces/logs
]
Agent: "Incident dashboard ready. Focus: latency (p99 3.2s), errors (2.3%), connections (1.2k). 
        Runbook: 'Latency Spike' linked. Want to drill into trace?"
```

---

## 8. Multi-Format Export

```rust
impl DashboardExporter for Dashboard {
    fn to_grafana_json(&self) -> serde_json::Value { ... }
    fn to_prometheus_alerting_yaml(&self) -> String { ... }
    fn to_datadog_json(&self) -> serde_json::Value { ... }
    fn to_grafonnet(&self) -> String { ... }      // Jsonnet for Grafana
    fn to_kubernetes_configmap(&self) -> ConfigMap { ... }
    fn to_terraform(&self) -> String { ... }
    fn to_markdown(&self) -> String { ... }
    fn to_html(&self, theme: Theme) -> String { ... }
    fn to_slack_blocks(&self) -> Vec<SlackBlock> { ... }
}
```

### Grafana JSON Example
```json
{
  "dashboard": {
    "title": "Auth Service - Operational",
    "uid": "auth-service-op",
    "tags": ["operational", "auth-service", "generated"],
    "timezone": "utc",
    "refresh": "15s",
    "panels": [
      {
        "id": 1,
        "title": "Latency (p50/p95/p99)",
        "type": "timeseries",
        "gridPos": { "x": 0, "y": 0, "w": 12, "h": 8 },
        "targets": [
          { "expr": "histogram_quantile(0.50, rate(http_request_duration_seconds_bucket{service=\"auth\"}[5m]))", "legendFormat": "p50" },
          { "expr": "histogram_quantile(0.95, rate(http_request_duration_seconds_bucket{service=\"auth\"}[5m]))", "legendFormat": "p95" },
          { "expr": "histogram_quantile(0.99, rate(http_request_duration_seconds_bucket{service=\"auth\"}[5m]))", "legendFormat": "p99" }
        ],
        "fieldConfig": {
          "defaults": {
            "thresholds": {
              "mode": "absolute",
              "steps": [
                { "color": "green", "value": null },
                { "color": "yellow", "value": 0.5 },
                { "color": "red", "value": 1.0 }
              ]
            },
            "unit": "s"
          }
        }
      }
    ],
    "templating": {
      "list": [
        { "name": "service", "type": "query", "query": "label_values(up, service)" }
      ]
    }
  }
}
```

---

## 9. Template Library

```rust
pub mod templates {
    pub const OPERATIONAL: DashboardTemplate = DashboardTemplate {
        id: "operational",
        name: "Operational Dashboard",
        dashboard_type: DashboardType::Operational,
        panels: vec![
            PanelTemplate { title: "Latency (p50/p95/p99)", panel_type: PanelType::TimeSeries, ... },
            PanelTemplate { title: "Error Rate", panel_type: PanelType::Stat, ... },
            PanelTemplate { title: "Throughput (req/s)", panel_type: PanelType::TimeSeries, ... },
            PanelTemplate { title: "Saturation (CPU/Mem)", panel_type: PanelType::Gauge, ... },
        ],
        default_thresholds: vec![
            ThresholdTemplate { field: "latency_p99", operator: Gt, value: 1.0, severity: Warning },
            ThresholdTemplate { field: "error_rate", operator: Gt, value: 0.01, severity: Critical },
        ],
        cross_panel_links: vec![
            LinkTemplate { from: "latency", to: "traces", type: DrillDown },
            LinkTemplate { from: "error_rate", to: "logs", type: DrillDown },
        ],
    };
    
    pub const INCIDENT: DashboardTemplate = DashboardTemplate { ... };
    pub const EXECUTIVE: DashboardTemplate = DashboardTemplate { ... };
    pub const DEBUGGING: DashboardTemplate = DashboardTemplate { ... };
    pub const BUSINESS_KPI: DashboardTemplate = DashboardTemplate { ... };
    pub const SECURITY_POSTURE: DashboardTemplate = DashboardTemplate { ... };
}
```

---

## 10. Configuration

```toml
[dashboard_construction]
default_template = "operational"
default_context = "desktop"
default_density = "comfortable"

[dashboard_construction.layout]
default_columns = 12
default_row_height_px = 80
default_gap_px = 8

[dashboard_construction.cost_budget]
max_total_latency_p99_ms = 2000
max_total_cost_usd_per_hour = 10.0
max_concurrent_queries = 50

[dashboard_construction.refresh]
default_interval_seconds = 15
default_strategy = "polling"
adaptive_refresh_enabled = true

[dashboard_construction.contexts.mobile]
grid_columns = 4
font_scale = 1.3
density = "compact"
collapsible_panels = true

[dashboard_construction.contexts.incident]
grid_columns = 6
focus_panel_enlargement = 1.5
auto_refresh_seconds = 5
runbook_injection = true

[dashboard_construction.hill_climb]
enabled = true
default_objectives = ["information_density", "cognitive_load", "query_cost", "mobile_fitness"]
max_iterations = 50
time_budget_ms = 30000
```

---

## 11. Integration Points

| System | Integration |
|---|---|
| **CME** | Dashboard as StructuredCard; in-conversation mutations |
| **Hill-Climbing** | Dashboard as Optimizable; auto-optimization per context |
| **Report Generation** | Dashboard panels as report sections; shared data sources |
| **Proactive Plugins** | Auto-generate dashboards on triggers (new service, incident) |
| **Cross-Session Memory** | Dashboard artifacts stored, linked to project |
| **Voice** | Dashboard read aloud with panel-aware prosody |
| **Synthetic Testing** | Dashboard generation test cases with expected panels |

---

*The dashboard construction engine turns "I need visibility" into a cost-aware, context-responsive, hill-climbable interface — making every dashboard a living control surface that adapts to its viewer.*