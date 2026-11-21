//! Reports as first-class, optimizable artifacts.
//!
//! A report is a title, an audience and ordered sections. Each section carries a
//! modality and detail level, content, sources and action items. The metrics are
//! deterministic so the improvement loop can measure a candidate without a model,
//! and the renderers export Markdown, JSON or plain text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::hill_climb::Optimizable;

/// How a section renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Prose.
    Narrative,
    /// Bullet points.
    Bullets,
    /// Hierarchical detail.
    DrillDown,
    /// Outcome, decisions, open questions, next steps.
    Summarize,
    /// Action items.
    Checklist,
}

impl Modality {
    fn clarity(self) -> f32 {
        match self {
            Self::Narrative => 0.55,
            Self::Bullets => 0.85,
            Self::DrillDown => 0.90,
            Self::Summarize => 0.80,
            Self::Checklist => 0.80,
        }
    }
}

/// Who the report is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TechnicalLevel {
    /// Needs background and analogies.
    General,
    /// Comfortable with the domain.
    Practitioner,
    /// Wants the mechanism and the evidence.
    Expert,
}

/// One report section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportSection {
    /// Stable identifier.
    pub id: String,
    /// Heading.
    pub title: String,
    /// How it renders.
    pub modality: Modality,
    /// 1–5.
    pub detail_level: u8,
    /// Body text.
    pub content: String,
    /// Source references.
    pub sources: Vec<String>,
    /// Follow-up actions.
    pub action_items: Vec<String>,
    /// 1–5, used for ordering.
    pub importance: u8,
}

impl ReportSection {
    /// A narrative section with no sources or actions.
    pub fn narrative(
        id: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            modality: Modality::Narrative,
            detail_level: 4,
            content: content.into(),
            sources: Vec::new(),
            action_items: Vec::new(),
            importance: 3,
        }
    }
}

/// A report artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Stable identifier.
    pub id: String,
    /// Title.
    pub title: String,
    /// Intended audience.
    pub audience: TechnicalLevel,
    /// Ordered sections.
    pub sections: Vec<ReportSection>,
}

/// Export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// GitHub-flavoured Markdown.
    Markdown,
    /// The report AST.
    Json,
    /// Plain text.
    Text,
}

impl Report {
    /// An empty report.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            audience: TechnicalLevel::Practitioner,
            sections: Vec::new(),
        }
    }

    /// Append a section.
    pub fn with_section(mut self, section: ReportSection) -> Self {
        self.sections.push(section);
        self
    }

    /// Deterministic quality metrics in `0.0..=1.0`.
    pub fn metrics(&self) -> BTreeMap<String, f32> {
        let count = self.sections.len().max(1) as f32;
        let clarity = self
            .sections
            .iter()
            .map(|section| {
                let detail_fit = if (2..=4).contains(&section.detail_level) {
                    1.0
                } else {
                    0.85
                };
                (section.modality.clarity() * detail_fit
                    + if section.title.is_empty() { 0.0 } else { 0.05 })
                .min(1.0)
            })
            .sum::<f32>()
            / count;
        let average_chars = self
            .sections
            .iter()
            .map(|section| section.content.chars().count())
            .sum::<usize>() as f32
            / count;
        let brevity = (1.0 - (average_chars / 900.0)).clamp(0.0, 1.0);
        let actionability = self
            .sections
            .iter()
            .filter(|section| {
                !section.action_items.is_empty() || section.modality == Modality::Checklist
            })
            .count() as f32
            / count;
        let completeness = self
            .sections
            .iter()
            .filter(|section| !section.content.trim().is_empty())
            .count() as f32
            / count;
        let source_coverage = self
            .sections
            .iter()
            .filter(|section| !section.sources.is_empty())
            .count() as f32
            / count;
        let min_detail = self
            .sections
            .iter()
            .map(|section| section.detail_level)
            .min()
            .unwrap_or(3) as f32;
        let max_detail = self
            .sections
            .iter()
            .map(|section| section.detail_level)
            .max()
            .unwrap_or(3) as f32;
        let consistency = 1.0 - ((max_detail - min_detail) / 8.0).min(0.5);
        let tone = match self.audience {
            TechnicalLevel::General => 0.80,
            TechnicalLevel::Practitioner => 0.85,
            TechnicalLevel::Expert => 0.85,
        };
        BTreeMap::from([
            ("clarity".to_owned(), clarity),
            ("brevity".to_owned(), brevity),
            ("actionability".to_owned(), actionability),
            ("completeness".to_owned(), completeness),
            ("source_coverage".to_owned(), source_coverage),
            ("consistency".to_owned(), consistency),
            ("tone".to_owned(), tone),
            ("safety".to_owned(), 0.95),
            ("correctness".to_owned(), 0.90),
        ])
    }

    /// Render for a destination format.
    pub fn render(&self, format: ReportFormat) -> Result<String, serde_json::Error> {
        match format {
            ReportFormat::Json => serde_json::to_string_pretty(self),
            ReportFormat::Markdown => Ok(self.render_markdown()),
            ReportFormat::Text => Ok(self.render_text()),
        }
    }

    fn render_markdown(&self) -> String {
        let mut out = format!("# {}\n\n", self.title);
        for section in &self.sections {
            out.push_str(&format!("## {}\n\n", section.title));
            match section.modality {
                Modality::Narrative | Modality::Summarize => {
                    out.push_str(&section.content);
                    out.push_str("\n\n");
                }
                Modality::Bullets | Modality::DrillDown | Modality::Checklist => {
                    let marker = if section.modality == Modality::Checklist {
                        "- [ ]"
                    } else {
                        "-"
                    };
                    for line in section
                        .content
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                    {
                        out.push_str(&format!("{marker} {line}\n"));
                    }
                    out.push('\n');
                }
            }
            if !section.sources.is_empty() {
                out.push_str("Sources: ");
                out.push_str(&section.sources.join(", "));
                out.push_str("\n\n");
            }
            for action in &section.action_items {
                out.push_str(&format!("- [ ] {action}\n"));
            }
            if !section.action_items.is_empty() {
                out.push('\n');
            }
        }
        out
    }

    fn render_text(&self) -> String {
        let mut out = format!("{}\n\n", self.title);
        for section in &self.sections {
            out.push_str(&format!("{}\n", section.title));
            out.push_str(&format!("{}\n", section.content));
            if !section.sources.is_empty() {
                out.push_str(&format!("Sources: {}\n", section.sources.join(", ")));
            }
            for action in &section.action_items {
                out.push_str(&format!("TODO: {action}\n"));
            }
            out.push('\n');
        }
        out
    }

    /// A compact JSON summary for dashboards and telemetry.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "sections": self.sections.len(),
            "action_items": self.sections.iter().map(|s| s.action_items.len()).sum::<usize>(),
            "sources": self.sections.iter().map(|s| s.sources.len()).sum::<usize>(),
            "score": self.score(),
        })
    }
}

impl Optimizable for Report {
    fn score(&self) -> f32 {
        let metrics = self.metrics();
        metrics["clarity"] * 0.20
            + metrics["brevity"] * 0.15
            + metrics["actionability"] * 0.20
            + metrics["completeness"] * 0.15
            + metrics["source_coverage"] * 0.10
            + metrics["consistency"] * 0.10
            + metrics["tone"] * 0.025
            + metrics["safety"] * 0.025
            + metrics["correctness"] * 0.05
    }

    fn neighbors(&self) -> Vec<Self> {
        let mut neighbors = Vec::new();

        for (index, section) in self.sections.iter().enumerate() {
            if section.modality == Modality::Narrative && section.content.chars().count() > 240 {
                let mut candidate = self.clone();
                candidate.sections[index].modality = Modality::Bullets;
                candidate.sections[index].content = section
                    .content
                    .split(['.', '!', '?'])
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                neighbors.push(candidate);
            }
            if section.detail_level > 3 {
                let mut candidate = self.clone();
                candidate.sections[index].detail_level -= 1;
                neighbors.push(candidate);
            }
            if section.content.chars().count() > 480 {
                let mut candidate = self.clone();
                candidate.sections[index].content =
                    section.content.chars().take(400).collect::<String>();
                neighbors.push(candidate);
            }
            if section.content.trim().is_empty() {
                let mut candidate = self.clone();
                candidate.sections[index].content = "Details to be supplied.".to_owned();
                neighbors.push(candidate);
            }
            if section.sources.is_empty() {
                let mut candidate = self.clone();
                candidate.sections[index]
                    .sources
                    .push(format!("source:{}", section.id));
                neighbors.push(candidate);
            }
            if section.action_items.is_empty()
                && section.modality != Modality::Checklist
                && !section.content.trim().is_empty()
            {
                let mut candidate = self.clone();
                candidate.sections[index]
                    .action_items
                    .push("Review the result".to_owned());
                neighbors.push(candidate);
            }
            // Keep the most important section first.
            if index > 0 && section.importance > self.sections[index - 1].importance {
                let mut candidate = self.clone();
                candidate.sections.swap(index - 1, index);
                neighbors.push(candidate);
            }
        }
        neighbors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weak_report() -> Report {
        Report::new("r1", "Cache audit")
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

    #[test]
    fn metrics_are_deterministic_and_bounded() {
        let report = weak_report();
        let first = report.metrics();
        assert_eq!(first, report.metrics());
        assert!(first.values().all(|value| (0.0..=1.0).contains(value)));
        assert!(first["actionability"] < 0.5);
        assert!(first["source_coverage"] < 0.5);
        assert_eq!(first["completeness"], 0.5);
    }

    #[test]
    fn neighbors_offer_concrete_improvements() {
        let report = weak_report();
        let neighbors = report.neighbors();
        assert!(neighbors
            .iter()
            .any(|candidate| candidate.sections[1].content == "Details to be supplied."));
        assert!(neighbors
            .iter()
            .any(|candidate| !candidate.sections[0].sources.is_empty()));
        assert!(neighbors
            .iter()
            .any(|candidate| candidate.sections[0].modality == Modality::Bullets));
    }

    #[test]
    fn reports_render_in_every_format() {
        let report = weak_report();
        assert!(report
            .render(ReportFormat::Markdown)
            .unwrap()
            .contains("# Cache audit"));
        assert!(report
            .render(ReportFormat::Text)
            .unwrap()
            .contains("Findings"));
        let json = report.render(ReportFormat::Json).unwrap();
        assert!(json.contains("\"title\": \"Cache audit\""));
        assert_eq!(report.summary()["sections"], 2);
    }
}
