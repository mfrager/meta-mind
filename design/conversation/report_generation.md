# Report Generation Engine

> A composable, modality-aware report generation system that produces structured, actionable reports optimized for human consumption and machine readability.

---

## 1. Core Concept

Reports are **first-class artifacts** — not just text output. They have:
- **Structured schema** (sections, modalities, sources, metadata)
- **Modality-aware rendering** (each section uses the best modality)
- **Provenance tracking** (every claim linked to sources)
- **Optimizability** (hill-climbable for clarity, brevity, actionability)
- **Multi-format export** (Markdown, HTML, PDF, JSON, Notion, Confluence)

---

## 2. Report Schema

```rust
pub struct Report {
    pub id: ReportId,
    pub title: String,
    pub subtitle: Option<String>,
    pub metadata: ReportMetadata,
    pub sections: Vec<ReportSection>,
    pub artifacts: Vec<ArtifactRef>,
    pub executive_summary: Option<String>,
    pub generated_at: Timestamp,
    pub generator_version: String,
}

pub struct ReportMetadata {
    pub report_type: ReportType,
    pub author: AuthorRef,           // Agent or human
    pub project: Option<ProjectId>,
    pub tags: Vec<String>,
    pub sensitivity: SensitivityLevel,
    pub intended_audience: AudienceProfile,
    pub modality_preferences: ModalityPreferences,
}

pub enum ReportType {
    IncidentReport,
    DesignDocument,
    DecisionRecord,
    ResearchBrief,
    ProgressUpdate,
    AuditFindings,
    ArchitectureReview,
    Custom(String),
}

pub struct AudienceProfile {
    pub technical_level: TechnicalLevel,
    pub roles: Vec<String>,
    pub time_budget_minutes: Option<u32>,
    pub preferred_modalities: Vec<Modality>,
}

pub struct ReportSection {
    pub id: SectionId,
    pub title: String,
    pub subtitle: Option<String>,
    pub modality: ModalitySpec,      // How this section renders
    pub content: SectionContent,
    pub sources: Vec<SourceRef>,
    pub metadata: SectionMetadata,
    pub subsections: Vec<ReportSection>,
}

pub enum SectionContent {
    Markdown(String),
    Structured(StructuredContent),
    Table(TableData),
    Chart(ChartSpec),
    Code(CodeBlock),
    Mixed(Vec<ContentBlock>),
}

pub enum ContentBlock {
    Text(String),
    BulletList(BulletList),
    NumberedList(NumberedList),
    DefinitionList(DefinitionList),
    Callout(Callout),
    Diagram(DiagramSpec),
    Embed(EmbedRef),
}

pub struct BulletList {
    pub items: Vec<BulletItem>,
    pub style: BulletStyle,
    pub max_depth: usize,
}

pub struct BulletItem {
    pub heading: Option<String>,      // Bold prefix
    pub body: String,
    pub sources: Vec<SourceRef>,
    pub detail_level: u8,             // 1-3
    pub sub_items: Vec<BulletItem>,
    pub collapsible: bool,
}

pub struct ModalitySpec {
    pub modality: Modality,
    pub detail_level: u8,             // 1-3
    pub include_headings: bool,
    pub include_sources: bool,
    pub max_bullets: Option<usize>,
    pub heading_style: HeadingStyle,
    pub custom_params: HashMap<String, serde_json::Value>,
}

pub enum Modality {
    Narrative,        // Prose paragraphs
    Bullets,          // Structured bullet points
    DrillDown,        // Hierarchical L1→L2→L3
    Explain,          // Concept → Mechanism → Example → Pitfalls
    ZoomOut,          // Goal → Alternatives → Trade-offs → Recommendation
    Summarize,        // Outcome → Decisions → Open → Next Steps
    Table,            // Structured tabular data
    DecisionMatrix,   // Options × Criteria
    Checklist,        // Action items with status
    Timeline,         // Chronological events
}

pub struct SectionMetadata {
    pub importance: ImportanceLevel,
    pub estimated_read_time_minutes: u32,
    pub modality_rationale: Option<String>,
    pub last_optimized: Option<Timestamp>,
}
```

---

## 3. Report Builder (Fluent API)

```rust
let report = ReportBuilder::new("Deploy Pipeline Audit")
    .report_type(ReportType::IncidentReport)
    .project("auth-service-migration")
    .audience(AudienceProfile {
        technical_level: TechnicalLevel::Expert,
        roles: vec!["platform-engineer".into(), "tech-lead".into()],
        time_budget_minutes: Some(10),
        preferred_modalities: vec![Modality::Bullets, Modality::DrillDown],
    })
    .section(SectionBuilder::new("Executive Summary")
        .modality(Modality::Summarize)
        .detail_level(2)
        .content("Deploy pipeline failed 3x this week. Root cause: migration timeout on 47M-row table. Fixed via PR #247. Open: cache invalidation strategy.")
        .sources(vec![pr_247, grafana_deploy])
        .importance(ImportanceLevel::Critical)
    )
    .section(SectionBuilder::new("Root Cause Analysis")
        .modality(Modality::DrillDown)
        .detail_level(2)
        .include_headings(true)
        .include_sources(true)
        .content_structured(vec![
            ContentBlock::BulletList(BulletList {
                items: vec![
                    BulletItem {
                        heading: Some("Migration Timeout".into()),
                        body: "ALTER TABLE users ADD INDEX on 47M rows held lock 47s".into(),
                        sources: vec![commit_3f2a1],
                        sub_items: vec![
                            BulletItem { body: "No partitioning → full table lock".into(), ..default() },
                            BulletItem { body: "Lock timeout default 30s < 47s".into(), ..default() },
                        ],
                        ..default()
                    },
                    BulletItem {
                        heading: Some("Connection Pool Exhaustion".into()),
                        body: "Pool: 20 connections, avg query 12s during migration".into(),
                        sources: vec![grafana_pool],
                        sub_items: vec![
                            BulletItem { body: "No statement_timeout configured".into(), ..default() },
                        ],
                        ..default()
                    },
                ],
                style: BulletStyle::Hierarchical,
                max_depth: 3,
            }),
        ])
        .importance(ImportanceLevel::High)
    )
    .section(SectionBuilder::new("Architecture Decision: Cache Layer")
        .modality(Modality::ZoomOut)
        .detail_level(2)
        .include_headings(true)
        .include_sources(true)
        .content_structured(vec![
            ContentBlock::Text("Goal: Reduce p99 latency from 2.3s → <200ms for org lookup.".into()),
            ContentBlock::BulletList(BulletList {
                items: vec![
                    BulletItem { heading: Some("Option 1: Redis Cache".into()), body: "90% hit rate, sub-ms reads, adds operational complexity".into(), ..default() },
                    BulletItem { heading: Some("Option 2: Materialized View".into()), body: "Zero infra, but stale data risk for org hierarchy changes".into(), ..default() },
                    BulletItem { heading: Some("Option 3: Query Optimization".into()), body: "Fixes root cause, but limited ceiling on 47M-row joins".into(), ..default() },
                ],
                ..default()
            }),
            ContentBlock::Callout(Callout {
                kind: CalloutKind::Recommendation,
                title: "Recommended: Redis Cache".into(),
                body: "Trade-off: Adds infra complexity for 10x latency improvement. Implement as separate PR.".into(),
            }),
        ])
        .sources(vec![design_doc_12, redis_benchmarks])
        .importance(ImportanceLevel::High)
    )
    .section(SectionBuilder::new("Action Items")
        .modality(Modality::Checklist)
        .content_structured(vec![
            ContentBlock::BulletList(BulletList {
                items: vec![
                    BulletItem { heading: Some("PR #247".into()), body: "Migration fix + lock timeout increase".into(), ..default() },
                    BulletItem { heading: Some("PR #248".into()), body: "Add query timeout + partial index".into(), ..default() },
                    BulletItem { heading: Some("Cache Layer".into()), body: "Redis implementation — separate PR, you own".into(), ..default() },
                    BulletItem { heading: Some("Monitoring".into()), body: "Add deploy latency alert at p99 > 500ms".into(), ..default() },
                ],
                style: BulletStyle::Checklist,
                ..default()
            }),
        ])
        .importance(ImportanceLevel::Critical)
    )
    .build();
```

---

## 4. Rendering Pipeline

```
Report AST
    │
    ▼
┌─────────────────────┐
│ Modality Renderers  │  (one per Modality)
│  • Narrative        │
│  • Bullets          │
│  • DrillDown        │
│  • Explain          │
│  • ZoomOut          │
│  • Summarize        │
│  • Table            │
│  • DecisionMatrix   │
│  • Checklist        │
│  • Timeline         │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│ Format Adapters     │
│  • Markdown         │
│  • HTML             │
│  • PDF (via Typst)  │
│  • JSON (AST)       │
│  • Notion API       │
│  • Confluence       │
│  • Google Docs      │
│  • Slack Blocks     │
└─────────┬───────────┘
          │
          ▼
    Final Output
```

### 4.1 Modality Renderer Example (DrillDown)

```rust
impl ModalityRenderer for DrillDownRenderer {
    fn render(&self, section: &ReportSection, ctx: &RenderContext) -> RenderedSection {
        let mut output = String::new();
        
        // Section title
        output.push_str(&format!("## {}\n\n", section.title));
        
        if let SectionContent::Structured(blocks) = &section.content {
            for block in blocks {
                match block {
                    ContentBlock::BulletList(list) => {
                        output.push_str(&self.render_bullet_list(list, 0, &section.modality));
                    }
                    ContentBlock::Text(t) => {
                        output.push_str(&format!("{}\n\n", t));
                    }
                    _ => {}
                }
            }
        }
        
        // Sources footer
        if section.modality.include_sources && !section.sources.is_empty() {
            output.push_str("\n---\n**Sources:**\n");
            for src in &section.sources {
                output.push_str(&format!("- [{}] {}", src.id, src.title));
            }
        }
        
        RenderedSection { markdown: output, .. }
    }
    
    fn render_bullet_list(&self, list: &BulletList, depth: usize, modality: &ModalitySpec) -> String {
        let mut out = String::new();
        let indent = "  ".repeat(depth);
        let max_items = modality.max_bullets.unwrap_or(list.items.len()).min(list.items.len());
        
        for (i, item) in list.items.iter().take(max_items).enumerate() {
            let marker = match list.style {
                BulletStyle::Hierarchical if depth == 0 => "•",
                BulletStyle::Hierarchical => "–",
                BulletStyle::Checklist => "☐",
                BulletStyle::Numbered => &format!("{}.", i + 1),
                _ => "•",
            };
            
            let mut line = format!("{}{} ", indent, marker);
            
            if let Some(heading) = &item.heading {
                line.push_str(&format!("**{}** ", heading));
            }
            line.push_str(&item.body);
            
            if modality.include_sources && !item.sources.is_empty() {
                let srcs = item.sources.iter()
                    .map(|s| format!("[{}]", s.id))
                    .collect::<Vec<_>>()
                    .join(", ");
                line.push_str(&format!(" [{}]", srcs));
            }
            
            out.push_str(&format!("{}\n", line));
            
            // Sub-items
            if depth + 1 < modality.max_bullets.unwrap_or(3) && !item.sub_items.is_empty() {
                let sub_modality = ModalitySpec { max_bullets: Some(3), ..modality.clone() };
                out.push_str(&self.render_bullet_list(&BulletList { 
                    items: item.sub_items.clone(), 
                    style: BulletStyle::Hierarchical, 
                    ..Default::default() 
                }, depth + 1, &sub_modality));
            }
        }
        out
    }
}
```

---

## 5. Multi-Format Export

### 5.1 Markdown (Default)
```markdown
# Deploy Pipeline Audit

## Executive Summary
Deploy pipeline failed 3x this week. Root cause: migration timeout on 47M-row table. Fixed via PR #247. Open: cache invalidation strategy.

## Root Cause Analysis
### Migration Timeout
• **Migration Timeout**: ALTER TABLE users ADD INDEX on 47M rows held lock 47s [commit_3f2a1]
  – No partitioning → full table lock
  – Lock timeout default 30s < 47s
• **Connection Pool Exhaustion**
  • Pool: 20 connections, avg query 12s during migration [grafana_pool]
    – No statement_timeout configured

[Sources: commit_3f2a1, grafana_pool, grafana_deploy]
```

### 5.2 HTML (Styled)
```html
<article class="report incident-report">
  <header>
    <h1>Deploy Pipeline Audit</h1>
    <div class="meta">Generated 2026-11-20 · Type: IncidentReport · Audience: Expert</div>
  </header>
  <section class="section executive-summary">
    <h2>Executive Summary</h2>
    <p>Deploy pipeline failed 3x this week...</p>
  </section>
  <section class="section root-cause modality-drilldown">
    <h2>Root Cause Analysis</h2>
    <div class="bullet-list hierarchical">
      <div class="bullet-item l1">
        <strong>Migration Timeout</strong> ALTER TABLE users ADD INDEX...
        <ul class="l2">
          <li>No partitioning → full table lock</li>
        </ul>
      </div>
    </div>
  </section>
</article>
```

### 5.3 PDF (via Typst)
```typst
#import "@preview/report-template:0.1.0": *

#show: report-template.with(
  title: "Deploy Pipeline Audit",
  metadata: (type: "IncidentReport", date: today()),
  theme: "technical",
)

#section("Executive Summary")[
  Deploy pipeline failed 3x this week. Root cause: migration timeout on 47M-row table. Fixed via PR #247. Open: cache invalidation strategy.
]

#section("Root Cause Analysis")[
  #drilldown((
    ("Migration Timeout", [
      "ALTER TABLE users ADD INDEX on 47M rows held lock 47s [commit_3f2a1]",
      ("No partitioning → full table lock", []),
      ("Lock timeout default 30s < 47s", []),
    ]),
    ("Connection Pool Exhaustion", [
      "Pool: 20 connections, avg query 12s during migration [grafana_pool]",
      ("No statement_timeout configured", []),
    ]),
  ))
]
```

### 5.4 JSON (Structured AST)
```json
{
  "id": "rpt_abc123",
  "title": "Deploy Pipeline Audit",
  "type": "IncidentReport",
  "sections": [
    {
      "id": "sec_1",
      "title": "Executive Summary",
      "modality": "summarize",
      "detail_level": 2,
      "content": "Deploy pipeline failed 3x...",
      "sources": ["pr_247", "grafana_deploy"]
    },
    {
      "id": "sec_2",
      "title": "Root Cause Analysis",
      "modality": "drill_down",
      "detail_level": 2,
      "content": {
        "type": "bullet_list",
        "items": [
          {
            "heading": "Migration Timeout",
            "body": "ALTER TABLE users ADD INDEX...",
            "sources": ["commit_3f2a1"],
            "sub_items": [...]
          }
        ]
      },
      "sources": ["commit_3f2a1", "grafana_pool"]
    }
  ],
  "artifacts": ["PR #247", "PR #248", "Design doc #12"]
}
```

### 5.5 Notion/Confluence/Slack Blocks
```rust
// Notion API payload
let notion_blocks = report.to_notion_blocks();
// [
//   { "object": "block", "type": "heading_1", "heading_1": { "rich_text": [{ "text": { "content": "Deploy Pipeline Audit" }}] } },
//   { "object": "block", "type": "heading_2", "heading_2": { "rich_text": [{ "text": { "content": "Executive Summary" }}] } },
//   { "object": "block", "type": "paragraph", "paragraph": { "rich_text": [{ "text": { "content": "Deploy pipeline failed 3x this week..." }}] } },
//   { "object": "block", "type": "heading_2", "heading_2": { "rich_text": [{ "text": { "content": "Root Cause Analysis" }}] } },
//   { "object": "block", "type": "bulleted_list_item", "bulleted_list_item": { "rich_text": [{ "text": { "content": "Migration Timeout" }}, { "type": "text", "text": { "content": " ALTER TABLE users ADD INDEX..." }, "annotations": { "bold": true }}] } },
//   ...
// ]

// Slack Block Kit
let slack_blocks = report.to_slack_blocks();
// [
//   { "type": "header", "text": { "type": "plain_text", "text": "Deploy Pipeline Audit" } },
//   { "type": "section", "text": { "type": "mrkdwn", "text": "*Executive Summary*\nDeploy pipeline failed 3x..." } },
//   { "type": "divider" },
//   { "type": "section", "text": { "type": "mrkdwn", "text": "*Root Cause Analysis*" } },
//   { "type": "section", "text": { "type": "mrkdwn", "text": "• *Migration Timeout*: ALTER TABLE users ADD INDEX..." } },
//   { "type": "context", "elements": [{ "type": "mrkdwn", "text": "Sources: commit_3f2a1, grafana_pool" }] },
// ]
```

---

## 6. Hill-Climbing Integration

```rust
impl Optimizable for Report {
    type Config = ReportGenerationConfig;
    type Metadata = ReportMetadata;
}

pub struct ReportNeighborhoodGenerator {
    // Mutation operators:
    mutations: Vec<Box<dyn ReportMutation>>,
}

pub trait ReportMutation: Send + Sync {
    fn apply(&self, report: &mut Report) -> MutationResult;
    fn description(&self) -> String;
}

pub struct ChangeSectionModality { /* section_id, new_modality */ }
pub struct AdjustDetailLevel { /* section_id, delta */ }
pub struct AddSourceCitation { /* section_id, source_ref */ }
pub struct ReorderSections { /* new_order */ }
pub struct SplitSection { /* section_id, split_point */ }
pub struct MergeSections { /* section_ids */ }
pub struct AddSection { /* template */ }
pub struct RemoveSection { /* section_id */ }
pub struct RewriteSectionContent { /* section_id, new_content */ }
pub struct AdjustModalityParams { /* section_id, new_params */ }
```

### Optimization Objectives (ReportEvaluator)

```rust
pub struct ReportEvaluator {
    objectives: Vec<Objective>,
}

pub enum Objective {
    Clarity { weight: f32 },                    // LLM-based readability
    Brevity { weight: f32 },                    // Tokens per insight
    Actionability { weight: f32 },              // Has next-steps, owners, deadlines
    Completeness { weight: f32 },               // Covers required topics
    ModalityAppropriateness { weight: f32 },    // Content-modality fit
    SourceCoverage { weight: f32 },             // Claims backed by sources
    UserPreferenceAlignment { weight: f32 },    // Matches learned prefs
    ModalityDiversity { weight: f32 },          // Uses appropriate modalities
    ActionItemQuality { weight: f32 },          // Specific, owned, dated
}

impl Evaluator<Report> for ReportEvaluator {
    type Score = ReportScore;
    
    fn evaluate(&self, report: &Report, ctx: &OptimizationContext) -> ReportScore {
        let mut scores = HashMap::new();
        for obj in &self.objectives {
            scores.insert(obj.name(), obj.score(report, ctx));
        }
        ReportScore { objectives: scores, composite: self.weighted_sum(&scores) }
    }
}
```

---

## 6. In-Conversation Report Generation

### 6.1 User Requests Report
```
User: "Generate an incident report for the deploy failures this week"
Agent: [Gathers context: deploy logs, commits, PRs, monitoring]
Agent: "Generating incident report... this will take a moment."
[Runs report builder with gathered context]
Agent: "Done. Report generated with 5 sections: Executive Summary, Root Cause (drill-down), Architecture Decision (zoom-out), Action Items (checklist), Open Questions. 
        Formats: Markdown, HTML, PDF, Notion. Want me to optimize for brevity?"
```

### 6.2 Iterative Refinement
```
User: "Make it more concise, switch root cause to bullets"
Agent: [Applies mutations: adjust_detail_level(sec_2, -1), change_modality(sec_2, bullets)]
Agent: "Done. Reduced 35% tokens. Root cause now uses bullets modality with detail_level=1. 
        Want me to run a full hill-climb optimization for clarity + brevity?"
```

### 6.3 Proactive Optimization
```
Agent: [Runs hill-climb overnight on draft report]
Agent: "I optimized the report while you were away. Clarity +22%, brevity +18%, actionability +15%.
        Changes: merged 2 redundant sections, switched 3 sections to better modalities, 
        added 4 missing source citations, converted action items to checklist modality.
        Review diff?"
```

---

## 7. Template Library

```rust
pub struct ReportTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub report_type: ReportType,
    pub required_sections: Vec<SectionTemplate>,
    pub optional_sections: Vec<SectionTemplate>,
    pub default_modalities: HashMap<String, ModalitySpec>,
    pub audience_defaults: AudienceProfile,
}

pub struct SectionTemplate {
    pub id: String,
    pub title: String,
    pub modality: ModalitySpec,
    pub content_template: Option<String>,  // With variable interpolation
    pub required_sources: Vec<SourceType>,
    pub importance: ImportanceLevel,
    pub depends_on: Vec<String>,  // Other section IDs
}

// Built-in templates
pub mod templates {
    pub const INCIDENT_REPORT: ReportTemplate = ReportTemplate {
        id: "incident_report",
        name: "Incident Report",
        report_type: ReportType::IncidentReport,
        required_sections: vec![
            SectionTemplate { id: "executive_summary", title: "Executive Summary", modality: Modality::Summarize, ... },
            SectionTemplate { id: "root_cause", title: "Root Cause Analysis", modality: Modality::DrillDown, ... },
            SectionTemplate { id: "timeline", title: "Timeline", modality: Modality::Timeline, ... },
            SectionTemplate { id: "action_items", title: "Action Items", modality: Modality::Checklist, ... },
        ],
        ...
    };
    
    pub const DESIGN_DOC: ReportTemplate = ReportTemplate {
        id: "design_doc",
        name: "Design Document",
        report_type: ReportType::DesignDocument,
        required_sections: vec![
            SectionTemplate { id: "context", title: "Context & Goals", modality: Modality::Narrative, ... },
            SectionTemplate { id: "proposed_solution", title: "Proposed Solution", modality: Modality::Explain, ... },
            SectionTemplate { id: "alternatives", title: "Alternatives Considered", modality: Modality::DecisionMatrix, ... },
            SectionTemplate { id: "tradeoffs", title: "Trade-offs & Risks", modality: Modality::ZoomOut, ... },
            SectionTemplate { id: "implementation", title: "Implementation Plan", modality: Modality::Checklist, ... },
        ],
        ...
    };
    
    pub const DECISION_RECORD: ReportTemplate = ReportTemplate { ... };
    pub const ARCHITECTURE_REVIEW: ReportTemplate = ReportTemplate { ... };
    pub const RESEARCH_BRIEF: ReportTemplate = ReportTemplate { ... };
}
```

---

## 8. Configuration

```toml
[report_generation]
default_template = "incident_report"
default_format = "markdown"
default_detail_level = 2

[report_generation.modalities]
default_detail_level = 2
default_include_headings = true
default_include_sources = true
default_max_bullets = 7

[report_generation.rendering]
markdown_dialect = "gfm"
html_theme = "technical"
pdf_engine = "typst"
pdf_theme = "technical"

[report_generation.export]
notion_integration = true
confluence_integration = false
slack_blocks = true
google_docs = false

[report_generation.optimization]
enabled = true
default_objectives = ["clarity", "brevity", "actionability", "completeness"]
max_optimization_iterations = 50
time_budget_ms = 30000
```

---

## 7. Integration Points

| System | Integration |
|---|---|
| **CME** | Report as StructuredCard in chat; in-conversation refinement |
| **Hill-Climbing** | Report as Optimizable artifact; nightly auto-optimization |
| **Proactive Plugins** | Generate reports on triggers (deploy failure, security scan) |
| **Cross-Session Memory** | Report artifacts stored, linked to project/user context |
| **Voice** | Report sections read aloud with modality-aware prosody |
| **Synthetic Testing** | Report generation test cases with expected structure |

---

*The report generation engine turns raw context into structured, modality-optimized, hill-climbable artifacts — making every report a living document that improves over time.*