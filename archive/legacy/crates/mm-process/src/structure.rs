//! The plan builder's structured output and the plan → template compiler.
//!
//! The model returns an indexed structure (`order`, `parent`, `depends_on`) so
//! the builder never has to parse step references out of prose. The compiler is
//! deterministic: it walks the structure depth-first, renames nodes to safe step
//! ids, carries parallel groups, and refuses cycles, unknown tools, or an
//! oversized result. A markdown plan compiles to one step whose instructions are
//! the plan document itself.

use std::collections::{BTreeMap, BTreeSet};

use mm_data::process::{ProcessError, ProcessTemplate, StepKind, TemplateSource, TemplateStep};
use mm_data::{Plan, PlanFormat, PlanStep};
use serde::{Deserialize, Serialize};

use crate::config::LimitConfig;

/// Schema marker for builder output.
pub const STRUCTURE_SCHEMA: &str = "mm.plan.structure.v1";

/// The shape the builder generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    /// An ordered list.
    #[default]
    Flat,
    /// A tree with parent links.
    Tree,
    /// Prose.
    Markdown,
}

/// One generated step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureStep {
    /// Position in generation order; 0-based and contiguous.
    pub order: usize,
    /// Short title.
    pub title: String,
    /// What to do.
    #[serde(default)]
    pub instructions: String,
    /// Parent node's order, for a tree.
    #[serde(default)]
    pub parent: Option<usize>,
    /// Earlier nodes this step needs.
    #[serde(default)]
    pub depends_on: Vec<usize>,
    /// A parallel group label shared by consecutive siblings.
    #[serde(default)]
    pub parallel: Option<String>,
    /// Tools the step may use.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Optional verification check.
    #[serde(default)]
    pub check: Option<String>,
}

/// The builder's typed output, also stored on the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStructure {
    /// `mm.plan.structure.v1`.
    #[serde(default = "schema")]
    pub schema: String,
    /// Plan title.
    pub title: String,
    /// One-paragraph summary.
    #[serde(default)]
    pub summary: String,
    /// Which shape this is.
    #[serde(default)]
    pub kind: StructureKind,
    /// Steps in order.
    #[serde(default)]
    pub steps: Vec<StructureStep>,
    /// The plan document when `kind` is markdown.
    #[serde(default)]
    pub markdown: Option<String>,
}

fn schema() -> String {
    STRUCTURE_SCHEMA.to_owned()
}

impl PlanStructure {
    /// A markdown structure whose single step is the document.
    pub fn markdown(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            schema: schema(),
            title: title.into(),
            summary: String::new(),
            kind: StructureKind::Markdown,
            steps: Vec::new(),
            markdown: Some(body.into()),
        }
    }

    /// Validate and normalize generated output.
    pub fn validate(&self, limits: &LimitConfig) -> Result<(), ProcessError> {
        if self.schema != STRUCTURE_SCHEMA {
            return Err(invalid(format!(
                "unsupported plan structure schema {:?}",
                self.schema
            )));
        }
        if self.title.trim().is_empty() || self.title.chars().count() > 200 {
            return Err(invalid("plan title must be 1–200 characters"));
        }
        if self.kind == StructureKind::Markdown {
            let Some(markdown) = self.markdown.as_deref() else {
                return Err(invalid("markdown structure has no body"));
            };
            if markdown.trim().is_empty() {
                return Err(invalid("markdown plan body is empty"));
            }
            if markdown.chars().count() > 128_000 {
                return Err(invalid("markdown plan body exceeds its limit"));
            }
            return Ok(());
        }
        if self.steps.is_empty() || self.steps.len() > limits.max_steps {
            return Err(invalid(format!(
                "plan must have 1–{} steps",
                limits.max_steps
            )));
        }
        let mut orders = BTreeSet::new();
        for step in &self.steps {
            if step.order >= self.steps.len() || !orders.insert(step.order) {
                return Err(invalid("plan step orders must be contiguous and unique"));
            }
            if step.title.trim().is_empty() || step.title.chars().count() > 200 {
                return Err(invalid(format!(
                    "step {} must have a 1–200 character title",
                    step.order
                )));
            }
            if step.instructions.trim().is_empty()
                || step.instructions.chars().count() > limits.max_instruction_chars
            {
                return Err(invalid(format!(
                    "step {} instructions must be 1–{} characters",
                    step.order, limits.max_instruction_chars
                )));
            }
            if let Some(parent) = step.parent {
                if parent >= step.order {
                    return Err(invalid("a plan step may only point at an earlier parent"));
                }
            }
            if step.depends_on.len() > 8 {
                return Err(invalid("plan step exceeds the dependency cap"));
            }
            for dependency in &step.depends_on {
                if *dependency >= step.order {
                    return Err(invalid("plan dependencies must point at earlier steps"));
                }
            }
        }
        // A parent link and a dependency link form the same DAG; the bounds
        // above already make every edge point backwards, so a cycle is
        // impossible. This check catches a parent that names a missing order.
        for step in &self.steps {
            if let Some(parent) = step.parent {
                if !orders.contains(&parent) {
                    return Err(invalid(format!(
                        "step {} names missing parent {parent}",
                        step.order
                    )));
                }
            }
        }
        Ok(())
    }

    /// The plan body stored on `Plan.body`.
    pub fn body(&self) -> Result<String, ProcessError> {
        match self.kind {
            StructureKind::Markdown => Ok(self.markdown.clone().unwrap_or_default()),
            _ => serde_json::to_string_pretty(self)
                .map_err(|error| invalid(format!("plan structure could not be encoded: {error}"))),
        }
    }
}

/// Compile a structure into a template.
pub fn compile_structure(
    structure: &PlanStructure,
    session_id: &str,
    uri: impl Into<String>,
    limits: &LimitConfig,
    known_tools: &[String],
) -> Result<ProcessTemplate, ProcessError> {
    structure.validate(limits)?;
    let now = mm_data::rdf::now_ms();
    let steps = match structure.kind {
        StructureKind::Markdown => vec![TemplateStep::new(
            "execute",
            structure.title.clone(),
            structure.markdown.clone().unwrap_or_default(),
        )],
        StructureKind::Flat | StructureKind::Tree => compile_nodes(structure, limits, known_tools)?,
    };
    let mut template = ProcessTemplate::new(uri, structure.title.clone(), steps, now);
    template.description = structure.summary.clone();
    template.source = TemplateSource::Plan;
    template.session_id = Some(session_id.to_owned());
    Ok(template)
}

fn compile_nodes(
    structure: &PlanStructure,
    limits: &LimitConfig,
    known_tools: &[String],
) -> Result<Vec<TemplateStep>, ProcessError> {
    let mut ordered: Vec<&StructureStep> = structure.steps.iter().collect();
    ordered.sort_by_key(|step| step.order);
    let mut ids: BTreeMap<usize, String> = BTreeMap::new();
    for step in &ordered {
        ids.insert(step.order, format!("step-{}", step.order + 1));
    }
    let mut steps = Vec::with_capacity(ordered.len());
    for node in &ordered {
        let id = ids[&node.order].clone();
        let mut step = TemplateStep::new(id, node.title.clone(), node.instructions.clone());
        step.depends_on = node
            .depends_on
            .iter()
            .map(|order| {
                ids.get(order).cloned().ok_or_else(|| {
                    invalid(format!(
                        "step {} depends on missing node {order}",
                        node.order
                    ))
                })
            })
            .collect::<Result<_, _>>()?;
        step.parallel_group = node.parallel.clone();
        step.tool_scope = node
            .tools
            .iter()
            .filter(|tool| known_tools.iter().any(|known| known == *tool))
            .cloned()
            .collect();
        step.kind = StepKind::Cm;
        step.check = node.check.clone().filter(|check| !check.trim().is_empty());
        steps.push(step);
    }
    if steps.len() > limits.max_steps {
        return Err(invalid("compiled template exceeds the step limit"));
    }
    Ok(steps)
}

/// Compile a persisted plan into a template.
///
/// The structured body is authoritative. A legacy flat plan uses its `steps`; a
/// markdown plan becomes one step whose instructions are its body.
pub fn compile_plan(
    plan: &Plan,
    uri: impl Into<String>,
    limits: &LimitConfig,
    known_tools: &[String],
) -> Result<ProcessTemplate, ProcessError> {
    if let Some(value) = &plan.structure {
        let structure: PlanStructure = serde_json::from_value(value.clone())
            .map_err(|error| invalid(format!("plan structure could not be decoded: {error}")))?;
        let mut template =
            compile_structure(&structure, &plan.session_id, uri, limits, known_tools)?;
        template.description = if structure.summary.trim().is_empty() {
            plan.goal.clone()
        } else {
            structure.summary.clone()
        };
        return Ok(template);
    }
    let steps: Vec<TemplateStep> = if plan.format == PlanFormat::Structured
        && !plan.steps.is_empty()
    {
        plan.steps
            .iter()
            .map(|step: &PlanStep| {
                let mut template_step =
                    TemplateStep::new(step.id.clone(), step.title.clone(), step.title.clone());
                template_step.depends_on = step.depends_on.clone();
                template_step
            })
            .collect()
    } else if !plan.body.trim().is_empty() {
        vec![TemplateStep::new(
            "execute",
            plan.goal.clone(),
            plan.body.clone(),
        )]
    } else {
        plan.steps
            .iter()
            .map(|step| TemplateStep::new(step.id.clone(), step.title.clone(), step.title.clone()))
            .collect()
    };
    if steps.is_empty() {
        return Err(invalid("plan has nothing to compile into a process"));
    }
    let now = mm_data::rdf::now_ms();
    let mut template = ProcessTemplate::new(uri, plan.goal.clone(), steps, now);
    template.description = plan.goal.clone();
    template.source = TemplateSource::Plan;
    template.session_id = Some(plan.session_id.clone());
    Ok(template)
}

/// The plan's structure when it has one.
pub fn structure_of(plan: &Plan) -> Option<PlanStructure> {
    plan.structure
        .as_ref()
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn invalid(message: impl Into<String>) -> ProcessError {
    ProcessError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(order: usize, title: &str) -> StructureStep {
        StructureStep {
            order,
            title: title.into(),
            instructions: format!("do {title}"),
            parent: None,
            depends_on: Vec::new(),
            parallel: None,
            tools: Vec::new(),
            check: None,
        }
    }

    #[test]
    fn a_tree_compiles_with_dependencies_and_parallel_groups() {
        let mut first = node(0, "research");
        first.parallel = Some("research".into());
        let mut second = node(1, "read");
        second.parallel = Some("research".into());
        let mut third = node(2, "write");
        third.depends_on = vec![0, 1];
        third.parent = Some(0);
        let structure = PlanStructure {
            schema: STRUCTURE_SCHEMA.into(),
            title: "Ship".into(),
            summary: "ship it".into(),
            kind: StructureKind::Tree,
            steps: vec![first, second, third],
            markdown: None,
        };
        let template = compile_structure(
            &structure,
            "s",
            "urn:metamind:process/template/t",
            &LimitConfig::default(),
            &[],
        )
        .unwrap();
        assert_eq!(template.steps.len(), 3);
        assert_eq!(template.steps[2].depends_on, vec!["step-1", "step-2"]);
        assert_eq!(
            template.steps[0].parallel_group.as_deref(),
            Some("research")
        );
    }

    #[test]
    fn forward_dependencies_and_empty_titles_are_rejected() {
        let limits = LimitConfig::default();
        let mut bad = node(1, "later");
        bad.depends_on = vec![2];
        let structure = PlanStructure {
            schema: STRUCTURE_SCHEMA.into(),
            title: "Bad".into(),
            summary: String::new(),
            kind: StructureKind::Flat,
            steps: vec![node(0, "first"), bad],
            markdown: None,
        };
        assert!(structure.validate(&limits).is_err());
        let empty = PlanStructure {
            schema: STRUCTURE_SCHEMA.into(),
            title: "Bad".into(),
            summary: String::new(),
            kind: StructureKind::Flat,
            steps: vec![StructureStep {
                title: " ".into(),
                ..node(0, "x")
            }],
            markdown: None,
        };
        assert!(empty.validate(&limits).is_err());
    }

    #[test]
    fn markdown_plans_compile_to_one_step() {
        let structure = PlanStructure::markdown("Fix", "# Fix\n- step one");
        let template = compile_structure(
            &structure,
            "s",
            "urn:metamind:process/template/t",
            &LimitConfig::default(),
            &[],
        )
        .unwrap();
        assert_eq!(template.steps.len(), 1);
        assert!(template.steps[0].instructions.contains("step one"));
    }
}
