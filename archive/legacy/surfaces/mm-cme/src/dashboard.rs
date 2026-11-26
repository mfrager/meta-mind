//! Dashboards as first-class, optimizable artifacts.
//!
//! A dashboard is a grid of panels with refresh costs. The metrics cover
//! information density, cognitive load, query cost, actionability and mobile
//! fitness; the neighborhood mutates thresholds, costs, layout and ordering.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::hill_climb::Optimizable;

/// What a panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    /// A single number.
    Metric,
    /// A time series.
    Chart,
    /// A table.
    Table,
    /// A short list.
    List,
}

/// A viewing context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewingContext {
    /// A wide screen.
    Desktop,
    /// A phone.
    Mobile,
    /// An incident overlay.
    Incident,
    /// An executive summary.
    Executive,
}

/// One dashboard panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    /// Stable identifier.
    pub id: String,
    /// Heading.
    pub title: String,
    /// What it shows.
    pub kind: PanelKind,
    /// Estimated cost per refresh, in arbitrary units.
    pub cost_per_refresh: f32,
    /// Whether the panel is critical to the task.
    pub critical: bool,
    /// Whether it carries a threshold or alert.
    pub has_threshold: bool,
    /// 1–3; lower is a more compact presentation.
    pub detail: u8,
}

impl Panel {
    /// A critical metric with no threshold.
    pub fn metric(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: PanelKind::Metric,
            cost_per_refresh: 1.0,
            critical: true,
            has_threshold: false,
            detail: 2,
        }
    }
}

/// A dashboard artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    /// Stable identifier.
    pub id: String,
    /// Title.
    pub title: String,
    /// Panels in display order.
    pub panels: Vec<Panel>,
    /// Grid columns.
    pub columns: u8,
    /// Where it is viewed.
    pub context: ViewingContext,
}

impl Dashboard {
    /// An empty dashboard.
    pub fn new(id: impl Into<String>, title: impl Into<String>, context: ViewingContext) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            panels: Vec::new(),
            columns: 3,
            context,
        }
    }

    /// Append a panel.
    pub fn with_panel(mut self, panel: Panel) -> Self {
        self.panels.push(panel);
        self
    }

    /// Deterministic quality and cost metrics.
    ///
    /// `cognitive_load` and `query_cost` are burdens: lower is better. The other
    /// metrics are fitness scores: higher is better.
    pub fn metrics(&self) -> BTreeMap<String, f32> {
        let count = self.panels.len().max(1) as f32;
        let critical = self.panels.iter().filter(|panel| panel.critical).count() as f32;
        let information_density = (0.35 + 0.65 * (critical / count)).min(1.0);
        let cognitive_load = (self.panels.len() as f32 / 12.0
            + if self.columns > 2 { 0.15 } else { 0.0 })
        .clamp(0.0, 1.0);
        let total_cost: f32 = self.panels.iter().map(|panel| panel.cost_per_refresh).sum();
        let query_cost = (total_cost / 20.0).clamp(0.0, 1.0);
        let mobile_fitness = match self.context {
            ViewingContext::Mobile => {
                if self.columns <= 1 && self.panels.len() <= 5 {
                    0.95
                } else {
                    0.45
                }
            }
            _ => {
                if self.columns <= 3 {
                    0.85
                } else {
                    0.55
                }
            }
        };
        let actionability = self
            .panels
            .iter()
            .filter(|panel| !panel.critical || panel.has_threshold)
            .count() as f32
            / count;
        let kinds: std::collections::BTreeSet<PanelKind> =
            self.panels.iter().map(|panel| panel.kind).collect();
        let consistency = 0.6 + 0.4 * (kinds.len() as f32 / count).min(1.0);
        BTreeMap::from([
            ("information_density".to_owned(), information_density),
            ("cognitive_load".to_owned(), cognitive_load),
            ("query_cost".to_owned(), query_cost),
            ("mobile_fitness".to_owned(), mobile_fitness),
            ("actionability".to_owned(), actionability),
            ("consistency".to_owned(), consistency),
            ("safety".to_owned(), 0.95),
            ("correctness".to_owned(), 0.90),
        ])
    }

    /// A compact JSON summary for telemetry and reports.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "panels": self.panels.len(),
            "columns": self.columns,
            "context": self.context,
            "score": self.score(),
            "metrics": self.metrics(),
        })
    }
}

impl Optimizable for Dashboard {
    fn score(&self) -> f32 {
        let metrics = self.metrics();
        metrics["information_density"] * 0.15
            + (1.0 - metrics["cognitive_load"]) * 0.15
            + (1.0 - metrics["query_cost"]) * 0.15
            + metrics["mobile_fitness"] * 0.15
            + metrics["actionability"] * 0.20
            + metrics["consistency"] * 0.10
            + metrics["safety"] * 0.025
            + metrics["correctness"] * 0.075
    }

    fn neighbors(&self) -> Vec<Self> {
        let mut neighbors = Vec::new();

        for (index, panel) in self.panels.iter().enumerate() {
            if panel.critical && !panel.has_threshold {
                let mut candidate = self.clone();
                candidate.panels[index].has_threshold = true;
                neighbors.push(candidate);
            }
            if panel.cost_per_refresh > 0.5 {
                let mut candidate = self.clone();
                candidate.panels[index].cost_per_refresh = (panel.cost_per_refresh - 0.5).max(0.1);
                neighbors.push(candidate);
            }
            if !panel.critical && panel.detail > 1 {
                let mut candidate = self.clone();
                candidate.panels[index].detail -= 1;
                neighbors.push(candidate);
            }
            if index > 0 && panel.critical && !self.panels[index - 1].critical {
                let mut candidate = self.clone();
                candidate.panels.swap(index - 1, index);
                neighbors.push(candidate);
            }
        }
        if self.columns > 1 {
            let mut candidate = self.clone();
            candidate.columns = 1;
            neighbors.push(candidate);
        }
        // Dropping a non-critical panel lowers cost and load, but only when more
        // than one panel remains.
        if self.panels.len() > 1 {
            if let Some(index) = self.panels.iter().position(|panel| !panel.critical) {
                let mut candidate = self.clone();
                candidate.panels.remove(index);
                neighbors.push(candidate);
            }
        }
        neighbors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weak_dashboard() -> Dashboard {
        Dashboard::new("d1", "Service health", ViewingContext::Mobile)
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
            .with_panel(Panel {
                id: "p3".into(),
                title: "Errors".into(),
                kind: PanelKind::Metric,
                cost_per_refresh: 4.0,
                critical: true,
                has_threshold: false,
                detail: 2,
            })
    }

    #[test]
    fn metrics_expose_burdens_and_fitness() {
        let metrics = weak_dashboard().metrics();
        assert!(metrics["query_cost"] > 0.5);
        assert!(metrics["actionability"] < 1.0);
        assert_eq!(metrics["mobile_fitness"], 0.45);
    }

    #[test]
    fn neighbors_reduce_cost_and_add_thresholds() {
        let dashboard = weak_dashboard();
        let neighbors = dashboard.neighbors();
        assert!(neighbors
            .iter()
            .any(|candidate| candidate.panels[0].has_threshold));
        assert!(neighbors
            .iter()
            .any(|candidate| candidate.panels[2].has_threshold));
        assert!(neighbors.iter().any(|candidate| candidate.columns == 1));
        assert!(neighbors
            .iter()
            .any(|candidate| candidate.panels.len() == 2));
    }

    #[test]
    fn summary_includes_score_and_metrics() {
        let summary = weak_dashboard().summary();
        assert_eq!(summary["panels"], 3);
        assert!(summary["score"].as_f64().unwrap() > 0.0);
    }
}
