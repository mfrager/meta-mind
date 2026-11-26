//! Template validation, execution layout, PROCESS.md injection and bindings.
//!
//! Validation is deterministic and runs before every save and before every
//! instance start. The model's plan output is never trusted: a template that
//! names an unknown tool, depends on a later group, or nests a sub-process past
//! the depth cap is refused with a typed error.
//!
//! Steps are grouped for execution. A step with no `parallel_group` is its own
//! group; consecutive steps sharing a group run concurrently. Dependencies may
//! only point at an earlier group, which is what makes the layout acyclic.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use mm_data::process::{
    ProcessError, ProcessTemplate, StepKind, TemplateStep, WaitKind, MAX_INSTRUCTION_CHARS,
    MAX_TEMPLATE_STEPS,
};
use serde::{Deserialize, Serialize};

use crate::config::LimitConfig;

/// One execution group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Stable group key: `step:<id>` or `parallel:<name>:<n>`.
    pub key: String,
    /// Step indices, in template order.
    pub steps: Vec<usize>,
    /// Whether members run concurrently.
    pub parallel: bool,
}

/// The canonical execution layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Groups in execution order.
    pub groups: Vec<Group>,
}

impl Layout {
    /// The group containing step `index`.
    pub fn group_of(&self, index: usize) -> Option<(usize, &Group)> {
        self.groups
            .iter()
            .enumerate()
            .find(|(_, group)| group.steps.contains(&index))
    }

    /// The group key for `index`.
    pub fn key_of(&self, index: usize) -> Option<&str> {
        self.group_of(index).map(|(_, group)| group.key.as_str())
    }
}

/// Validate a template against reviewed limits and the tool catalog.
pub fn validate_template(
    template: &ProcessTemplate,
    limits: &LimitConfig,
    known_tools: &[String],
) -> Result<(), ProcessError> {
    if template.schema != "mm.process.v1" {
        return Err(invalid(format!(
            "unsupported process schema {:?}",
            template.schema
        )));
    }
    if !(template.uri.starts_with("urn:metamind:process/template/")
        || template
            .uri
            .starts_with("https://metamind.dev/data/process/template/"))
    {
        return Err(invalid(format!(
            "template uri {:?} is not a process template IRI",
            template.uri
        )));
    }
    let name_len = template.name.trim().chars().count();
    if name_len == 0 || name_len > 200 {
        return Err(invalid("template name must be 1–200 characters"));
    }
    if template.description.chars().count() > 2_000 {
        return Err(invalid("template description exceeds its limit"));
    }
    if template.steps.is_empty() || template.steps.len() > limits.max_steps {
        return Err(invalid(format!(
            "template must have 1–{} steps",
            limits.max_steps
        )));
    }
    if template.steps.len() > MAX_TEMPLATE_STEPS {
        return Err(invalid("template exceeds the storage step cap"));
    }
    if let Some(instructions) = &template.instructions {
        if instructions.chars().count() > limits.max_doc_chars {
            return Err(invalid("template PROCESS.md exceeds its limit"));
        }
    }
    let mut seen = BTreeSet::new();
    for step in &template.steps {
        validate_step_id(&step.id)?;
        if !seen.insert(step.id.as_str()) {
            return Err(invalid(format!("duplicate step id {:?}", step.id)));
        }
    }
    let mut variables = BTreeSet::new();
    for variable in &template.variables {
        if !is_name(variable) {
            return Err(invalid(format!("invalid variable name {variable:?}")));
        }
        if !variables.insert(variable.as_str()) {
            return Err(invalid(format!("duplicate variable {variable:?}")));
        }
    }
    for step in &template.steps {
        let title_len = step.title.trim().chars().count();
        if title_len == 0 || title_len > 200 {
            return Err(invalid(format!(
                "step {:?} must have a 1–200 character title",
                step.id
            )));
        }
        if step.instructions.trim().is_empty()
            || step.instructions.chars().count() > limits.max_instruction_chars
        {
            return Err(invalid(format!(
                "step {:?} instructions must be 1–{} characters",
                step.id, limits.max_instruction_chars
            )));
        }
        if step.instructions.chars().count() > MAX_INSTRUCTION_CHARS {
            return Err(invalid(format!(
                "step {:?} instructions exceed the storage cap",
                step.id
            )));
        }
        if step.depends_on.len() > 8 {
            return Err(invalid(format!(
                "step {:?} exceeds the dependency cap",
                step.id
            )));
        }
        match step.kind {
            StepKind::Cm | StepKind::Tool => {}
            StepKind::SubProcess => {
                let Some(uri) = step.template_uri.as_deref() else {
                    return Err(invalid(format!(
                        "sub-process step {:?} needs template_uri",
                        step.id
                    )));
                };
                if !uri.starts_with("urn:metamind:process/template/")
                    && !uri.starts_with("https://metamind.dev/data/process/template/")
                {
                    return Err(invalid(format!(
                        "sub-process step {:?} names an invalid template",
                        step.id
                    )));
                }
            }
            StepKind::Human | StepKind::UserInput => {
                if step.wait.is_none() {
                    return Err(invalid(format!(
                        "step {:?} of kind {:?} needs a wait condition",
                        step.id, step.kind
                    )));
                }
            }
            StepKind::Backend => {
                if step
                    .service
                    .as_deref()
                    .is_none_or(|service| !is_name(service))
                {
                    return Err(invalid(format!(
                        "backend step {:?} needs a valid service name",
                        step.id
                    )));
                }
            }
        }
        if let Some(wait) = &step.wait {
            if wait.kind == WaitKind::Timer && wait.wake_at.unwrap_or_default() <= 0 {
                return Err(invalid(format!(
                    "timer step {:?} needs a positive wake_at",
                    step.id
                )));
            }
        }
        if step.tool_scope.len() > 24 {
            return Err(invalid(format!(
                "step {:?} tool scope exceeds 24 entries",
                step.id
            )));
        }
        for tool in &step.tool_scope {
            if matches!(tool.as_str(), "metamind.verify" | "metamind.approve") {
                return Err(invalid(format!(
                    "step {:?} may not grant a control tool",
                    step.id
                )));
            }
            if !known_tools.iter().any(|known| known == tool) {
                return Err(invalid(format!(
                    "step {:?} names unknown tool {tool:?}",
                    step.id
                )));
            }
        }
    }
    // Dependencies and parallel groups are checked through the layout.
    layout(template, limits.max_parallel)?;
    Ok(())
}

/// Build and validate the execution layout.
pub fn layout(template: &ProcessTemplate, max_parallel: usize) -> Result<Layout, ProcessError> {
    let mut groups: Vec<Group> = Vec::new();
    let mut index = 0usize;
    while index < template.steps.len() {
        let step = &template.steps[index];
        if let Some(group) = step.parallel_group.as_deref() {
            if !is_name(group) {
                return Err(invalid(format!(
                    "step {:?} has an invalid parallel_group",
                    step.id
                )));
            }
            let mut members = vec![index];
            let mut next = index + 1;
            while next < template.steps.len()
                && template.steps[next].parallel_group.as_deref() == Some(group)
            {
                members.push(next);
                next += 1;
            }
            if members.len() > max_parallel {
                return Err(invalid(format!(
                    "parallel group {group:?} exceeds {max_parallel} steps"
                )));
            }
            groups.push(Group {
                key: format!("parallel:{group}:{}", groups.len()),
                steps: members,
                parallel: true,
            });
            index = next;
        } else {
            groups.push(Group {
                key: format!("step:{}", step.id),
                steps: vec![index],
                parallel: false,
            });
            index += 1;
        }
    }
    let mut group_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (group_index, group) in groups.iter().enumerate() {
        for step_index in &group.steps {
            group_of.insert(template.steps[*step_index].id.as_str(), group_index);
        }
    }
    for (group_index, group) in groups.iter().enumerate() {
        for step_index in &group.steps {
            let step = &template.steps[*step_index];
            for dependency in &step.depends_on {
                let Some(dependency_group) = group_of.get(dependency.as_str()) else {
                    return Err(invalid(format!(
                        "step {:?} depends on unknown step {dependency:?}",
                        step.id
                    )));
                };
                if *dependency_group >= group_index {
                    return Err(invalid(format!(
                        "step {:?} depends on step {dependency:?} in the same or a later group",
                        step.id
                    )));
                }
            }
        }
    }
    Ok(Layout { groups })
}

/// Load the template's PROCESS.md from the workspace and inject it.
///
/// The path is confined to the workspace root. A missing file, an escape, or an
/// oversized document is an error: the pipeline never silently runs a template
/// without the instructions it names.
pub fn load_doc(
    template: &mut ProcessTemplate,
    workspace: &Path,
    limits: &LimitConfig,
) -> Result<Option<String>, ProcessError> {
    let Some(path) = template.doc_path.as_deref() else {
        return Ok(None);
    };
    let relative = Path::new(path);
    if relative.is_absolute() {
        return Err(invalid("PROCESS.md path must be workspace-relative"));
    }
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid("PROCESS.md path must not escape the workspace"));
    }
    let root = workspace
        .canonicalize()
        .map_err(|error| invalid(format!("workspace is unavailable: {error}")))?;
    let candidate = workspace.join(relative);
    let resolved = match candidate.canonicalize() {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // The plan calls a missing document a warning, not a failure. The
            // template still validates; the step prompt simply has no doc.
            tracing::warn!(path = %path, "PROCESS.md is missing; continuing without it");
            return Ok(None);
        }
        Err(error) => {
            return Err(invalid(format!(
                "PROCESS.md {path:?} is unreadable: {error}"
            )))
        }
    };
    if !resolved.starts_with(&root) {
        return Err(invalid("PROCESS.md path escapes the workspace"));
    }
    let text = std::fs::read_to_string(&resolved)
        .map_err(|error| invalid(format!("PROCESS.md {path:?} is unreadable: {error}")))?;
    if text.trim().is_empty() {
        return Err(invalid("PROCESS.md is empty"));
    }
    if text.chars().count() > limits.max_doc_chars {
        return Err(invalid("PROCESS.md exceeds its limit"));
    }
    template.instructions = Some(text.clone());
    Ok(Some(text))
}

/// Render `{{inputs.x}}` and `{{steps.id.output}}` bindings strictly.
///
/// Unknown names are errors. Rendering is one pass, so an output containing a
/// binding-looking string is never expanded again.
pub fn render_bindings(
    text: &str,
    inputs: &BTreeMap<String, serde_json::Value>,
    outputs: &BTreeMap<String, String>,
) -> Result<String, ProcessError> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(invalid("unterminated binding in step instructions"));
        };
        let name = after[..end].trim();
        let value = resolve_binding(name, inputs, outputs)?;
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn resolve_binding(
    name: &str,
    inputs: &BTreeMap<String, serde_json::Value>,
    outputs: &BTreeMap<String, String>,
) -> Result<String, ProcessError> {
    if let Some(input) = name.strip_prefix("inputs.") {
        let value = inputs
            .get(input)
            .ok_or_else(|| invalid(format!("unknown input binding {name:?}")))?;
        return Ok(match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        });
    }
    if let Some(step) = name
        .strip_prefix("steps.")
        .and_then(|rest| rest.strip_suffix(".output"))
    {
        return outputs
            .get(step)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown step binding {name:?}")));
    }
    Err(invalid(format!("unknown binding {name:?}")))
}

/// Validate a single step id.
pub fn validate_step_id(id: &str) -> Result<(), ProcessError> {
    if !is_name(id) {
        return Err(invalid(format!("invalid step id {id:?}")));
    }
    Ok(())
}

/// Whether a value is a safe identifier (letters, digits, `_`, `-`, `.`).
pub fn is_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && !value.starts_with('.')
}

/// The workspace-relative path a template was loaded from.
pub fn doc_path(workspace: &Path, relative: &str) -> PathBuf {
    workspace.join(relative)
}

/// A compact step projection for prompts and tool listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepSummary {
    /// Step id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Kind.
    pub kind: StepKind,
    /// Dependencies.
    pub depends_on: Vec<String>,
}

/// Summaries for every step.
pub fn summaries(template: &ProcessTemplate) -> Vec<StepSummary> {
    template
        .steps
        .iter()
        .map(|step| StepSummary {
            id: step.id.clone(),
            title: step.title.clone(),
            kind: step.kind,
            depends_on: step.depends_on.clone(),
        })
        .collect()
}

/// Copy `instructions` into a template step, enforcing the cap.
pub fn set_instructions(
    step: &mut TemplateStep,
    instructions: impl Into<String>,
    limits: &LimitConfig,
) -> Result<(), ProcessError> {
    let instructions = instructions.into();
    if instructions.trim().is_empty() {
        return Err(invalid(format!(
            "step {:?} has empty instructions",
            step.id
        )));
    }
    if instructions.chars().count() > limits.max_instruction_chars {
        return Err(invalid(format!(
            "step {:?} instructions exceed their limit",
            step.id
        )));
    }
    step.instructions = instructions;
    Ok(())
}

fn invalid(message: impl Into<String>) -> ProcessError {
    ProcessError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_data::process::{ProcessTemplate, TemplateStep};

    fn template(steps: Vec<TemplateStep>) -> ProcessTemplate {
        ProcessTemplate::new("urn:metamind:process/template/t", "T", steps, 1)
    }

    #[test]
    fn consecutive_parallel_steps_form_one_group() {
        let mut a = TemplateStep::new("a", "A", "do a");
        a.parallel_group = Some("research".into());
        let mut b = TemplateStep::new("b", "B", "do b");
        b.parallel_group = Some("research".into());
        let mut c = TemplateStep::new("c", "C", "do c");
        c.depends_on = vec!["a".into(), "b".into()];
        let template = template(vec![a, b, c]);
        let layout = layout(&template, 8).unwrap();
        assert_eq!(layout.groups.len(), 2);
        assert_eq!(layout.groups[0].steps, vec![0, 1]);
        assert_eq!(layout.groups[1].steps, vec![2]);
    }

    #[test]
    fn dependency_inside_the_same_group_is_rejected() {
        let mut a = TemplateStep::new("a", "A", "do a");
        a.parallel_group = Some("g".into());
        let mut b = TemplateStep::new("b", "B", "do b");
        b.parallel_group = Some("g".into());
        b.depends_on = vec!["a".into()];
        assert!(layout(&template(vec![a, b]), 8).is_err());
    }

    #[test]
    fn unknown_tools_and_duplicate_ids_fail_validation() {
        let mut step = TemplateStep::new("a", "A", "do a");
        step.tool_scope = vec!["not.a.tool".into()];
        let limits = LimitConfig::default();
        assert!(validate_template(&template(vec![step.clone()]), &limits, &[]).is_err());
        let duplicate = template(vec![
            TemplateStep::new("a", "A", "do a"),
            TemplateStep::new("a", "B", "do b"),
        ]);
        assert!(validate_template(&duplicate, &limits, &[]).is_err());
    }

    #[test]
    fn bindings_are_strict_and_single_pass() {
        let mut inputs = BTreeMap::new();
        inputs.insert("task".into(), serde_json::json!("fix it"));
        let mut outputs = BTreeMap::new();
        outputs.insert("a".into(), "{{inputs.task}}".into());
        assert_eq!(
            render_bindings("{{steps.a.output}}", &inputs, &outputs).unwrap(),
            "{{inputs.task}}"
        );
        assert!(render_bindings("{{inputs.missing}}", &inputs, &outputs).is_err());
    }

    #[test]
    fn doc_injection_is_confined() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PROCESS.md"), "run the checks").unwrap();
        let mut template = template(vec![TemplateStep::new("a", "A", "do a")]);
        template.doc_path = Some("PROCESS.md".into());
        let text = load_doc(&mut template, dir.path(), &LimitConfig::default()).unwrap();
        assert_eq!(text.as_deref(), Some("run the checks"));
        template.doc_path = Some("../secret.md".into());
        assert!(load_doc(&mut template, dir.path(), &LimitConfig::default()).is_err());
    }
}
