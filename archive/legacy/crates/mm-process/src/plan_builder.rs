//! The four-pass plan builder and plan lifecycle helpers.
//!
//! `classify → route → decode → verify` is deterministic shell around the model.
//! Classification and decoding use schema-constrained calls; markdown generation
//! is a prose call. Malformed output gets a bounded repair attempt, never a
//! silent fallback. The deterministic validator runs after every decode, and the
//! review call can force one regeneration.
//!
//! The builder is the only place that writes `mm.plan.structure.v1`; the plan
//! tool, the compiler and the inspector all read that same document.

use std::sync::Arc;

use mm_data::process::{ProcessError, ProcessTemplate};
use mm_data::{Plan, PlanFormat, PlanStore};
use mm_llm::{CacheMode, LlmClient, LlmRequest, Message, Purpose, SchemaId};
use mm_prompt::PromptStore;
use serde::{Deserialize, Serialize};

use crate::config::{LimitConfig, PlanConfig};
use crate::structure::{
    compile_structure, PlanStructure, StructureKind, StructureStep, STRUCTURE_SCHEMA,
};

/// How hard the model thinks the goal is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    /// One obvious reply or action.
    #[default]
    Simple,
    /// A short sequence.
    Standard,
    /// Research plus several dependent actions.
    Complex,
}

/// The classifier's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    /// Complexity.
    pub complexity: Complexity,
    /// Shape to generate.
    pub kind: StructureKind,
    /// Whether the plan should outlive the turn.
    #[serde(default)]
    pub persist: bool,
    /// Why.
    #[serde(default)]
    pub reason: String,
}

/// A generated plan and its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltPlan {
    /// The typed plan.
    pub structure: PlanStructure,
    /// What the classifier decided.
    pub classification: Classification,
    /// Model calls made.
    pub calls: u32,
    /// Generation attempts used, including repairs.
    pub generation_attempts: u32,
    /// Review calls made.
    pub review_attempts: u32,
}

/// The reviewed plan builder.
pub struct PlanBuilder {
    client: Arc<dyn LlmClient>,
    prompts: Option<Arc<PromptStore>>,
    model: Option<String>,
    plan: PlanConfig,
    limits: LimitConfig,
}

impl PlanBuilder {
    /// Build over the shared model client and prompt tree.
    pub fn new(
        client: Arc<dyn LlmClient>,
        prompts: Option<Arc<PromptStore>>,
        model: Option<String>,
        plan: PlanConfig,
        limits: LimitConfig,
    ) -> Self {
        Self {
            client,
            prompts,
            model,
            plan,
            limits,
        }
    }

    /// Build a plan from a goal.
    ///
    /// `requested` overrides the classifier's shape when the caller asked for
    /// one; `persist` overrides its durability recommendation.
    pub async fn build(
        &self,
        goal: &str,
        instructions: Option<&str>,
        requested: Option<StructureKind>,
        persist: Option<bool>,
    ) -> Result<BuiltPlan, ProcessError> {
        if goal.trim().is_empty() || goal.chars().count() > mm_data::plan::MAX_GOAL_CHARS {
            return Err(invalid("plan goal must be 1–4000 characters"));
        }
        let prompts = self
            .prompts
            .as_ref()
            .ok_or_else(|| invalid("prompt tree is unavailable; no plan call was made"))?;
        let mut calls = 0u32;
        let classify_text = self
            .call_json(
                prompts,
                "plan.builder.classify.v1",
                &[
                    ("goal", goal),
                    ("instructions", instructions.unwrap_or("none")),
                ],
                Purpose::Classify,
                "mm.plan.intent.v1",
            )
            .await?;
        calls += 1;
        let mut classification = parse_classification(&classify_text)?;
        if let Some(kind) = requested {
            classification.kind = kind;
        }
        if let Some(persist) = persist {
            classification.persist = persist;
        }
        // `Simple` goals with no operator request still get one executable step;
        // the builder never returns zero work.
        let mut generation_attempts = 0u32;
        let mut review_attempts = 0u32;
        let mut repair: Option<String> = None;
        let mut notes: Vec<String> = Vec::new();
        loop {
            generation_attempts += 1;
            let draft = self
                .generate(
                    prompts,
                    goal,
                    instructions,
                    &classification,
                    repair.as_deref(),
                )
                .await?;
            calls += 1;
            let mut structure = match self.decode(&draft, classification.kind, goal) {
                Ok(structure) => structure,
                Err(error) => {
                    if generation_attempts > self.plan.decode_retries {
                        return Err(invalid(format!(
                            "plan decode failed after {generation_attempts} attempts: {error}"
                        )));
                    }
                    repair = Some(format!("The previous output was invalid: {error}"));
                    continue;
                }
            };
            structure.schema = STRUCTURE_SCHEMA.to_owned();
            if let Err(error) = structure.validate(&self.limits) {
                if generation_attempts > self.plan.decode_retries {
                    return Err(invalid(format!(
                        "plan validation failed after {generation_attempts} attempts: {error}"
                    )));
                }
                repair = Some(format!("The previous plan was invalid: {error}"));
                continue;
            }
            review_attempts += 1;
            let review = self.verify(prompts, goal, &structure, &notes).await?;
            calls += 1;
            if review.pass {
                return Ok(BuiltPlan {
                    structure,
                    classification,
                    calls,
                    generation_attempts,
                    review_attempts,
                });
            }
            if review_attempts > self.plan.review_retries {
                return Err(invalid(format!(
                    "plan review failed: {}",
                    if review.notes.is_empty() {
                        "no reason supplied".to_owned()
                    } else {
                        review.notes.join("; ")
                    }
                )));
            }
            notes = review.notes;
            repair = Some(format!(
                "The reviewer rejected the previous plan: {}",
                notes.join("; ")
            ));
        }
    }

    async fn call_json(
        &self,
        prompts: &PromptStore,
        strategy: &str,
        variables: &[(&str, &str)],
        purpose: Purpose,
        schema: &str,
    ) -> Result<String, ProcessError> {
        let mut values = std::collections::BTreeMap::new();
        for (name, value) in variables {
            values.insert((*name).to_owned(), (*value).to_owned());
        }
        let instance = prompts
            .render(strategy, &values)
            .map_err(|error| invalid(format!("{strategy} could not render: {error}")))?;
        let mut request = LlmRequest::new(purpose, vec![Message::system(instance.text)])
            .with_schema(SchemaId::new(schema))
            .with_cache(CacheMode::Use);
        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        let response = self
            .client
            .complete(&request)
            .await
            .map_err(|error| invalid(format!("plan model call failed: {error}")))?;
        Ok(response.text)
    }

    async fn generate(
        &self,
        prompts: &PromptStore,
        goal: &str,
        instructions: Option<&str>,
        classification: &Classification,
        repair: Option<&str>,
    ) -> Result<String, ProcessError> {
        let kind = match classification.kind {
            StructureKind::Markdown => "markdown",
            StructureKind::Tree => "tree",
            StructureKind::Flat => "flat",
        };
        let max_steps = self.plan.max_steps.to_string();
        let repair = repair.unwrap_or("none");
        let template = match classification.kind {
            StructureKind::Markdown => "plan.builder.markdown.v1",
            _ => "plan.builder.generate.v1",
        };
        let values = [
            ("goal", goal),
            ("instructions", instructions.unwrap_or("none")),
            ("structure", kind),
            ("max_steps", max_steps.as_str()),
            ("repair", repair),
        ];
        let mut map = std::collections::BTreeMap::new();
        for (name, value) in values {
            map.insert(name.to_owned(), value.to_owned());
        }
        let instance = prompts
            .render(template, &map)
            .map_err(|error| invalid(format!("{template} could not render: {error}")))?;
        let mut request = LlmRequest::new(Purpose::Plan, vec![Message::system(instance.text)])
            .with_cache(CacheMode::Use);
        if classification.kind != StructureKind::Markdown {
            request = request.with_schema(SchemaId::new("mm.plan.steps.v1"));
        }
        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        Ok(self
            .client
            .complete(&request)
            .await
            .map_err(|error| invalid(format!("plan generation failed: {error}")))?
            .text)
    }

    fn decode(
        &self,
        text: &str,
        kind: StructureKind,
        goal: &str,
    ) -> Result<PlanStructure, ProcessError> {
        if kind == StructureKind::Markdown {
            let body = trim_fence(text);
            if body.trim().is_empty() {
                return Err(invalid("markdown plan was empty"));
            }
            let mut structure = PlanStructure::markdown(goal, body);
            structure.summary = goal.to_owned();
            return Ok(structure);
        }
        let json = trim_fence(text);
        let mut structure: PlanStructure = serde_json::from_str(&json)
            .map_err(|error| invalid(format!("plan JSON is invalid: {error}")))?;
        structure.kind = kind;
        if structure.title.trim().is_empty() {
            structure.title = goal.to_owned();
        }
        if structure.summary.trim().is_empty() {
            structure.summary = goal.to_owned();
        }
        // Normalize an omitted `order` by declaration order.
        for (index, step) in structure.steps.iter_mut().enumerate() {
            step.order = index;
        }
        Ok(structure)
    }

    async fn verify(
        &self,
        prompts: &PromptStore,
        goal: &str,
        structure: &PlanStructure,
        previous_notes: &[String],
    ) -> Result<Review, ProcessError> {
        let body = structure.body()?;
        let notes = if previous_notes.is_empty() {
            "none".to_owned()
        } else {
            previous_notes.join("; ")
        };
        let mut map = std::collections::BTreeMap::new();
        map.insert("goal".to_owned(), goal.to_owned());
        map.insert("plan".to_owned(), body);
        map.insert("previous_notes".to_owned(), notes);
        let instance = prompts
            .render("plan.builder.verify.v1", &map)
            .map_err(|error| invalid(format!("plan review prompt could not render: {error}")))?;
        let mut request = LlmRequest::new(Purpose::Critique, vec![Message::system(instance.text)])
            .with_schema(SchemaId::new("mm.plan.review.v1"))
            .with_cache(CacheMode::Use);
        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        let text = self
            .client
            .complete(&request)
            .await
            .map_err(|error| invalid(format!("plan review failed: {error}")))?
            .text;
        serde_json::from_str::<Review>(&trim_fence(&text))
            .map_err(|error| invalid(format!("plan review output is invalid: {error}")))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Review {
    #[serde(default)]
    pass: bool,
    #[serde(default)]
    notes: Vec<String>,
}

fn parse_classification(text: &str) -> Result<Classification, ProcessError> {
    #[derive(Deserialize)]
    struct Raw {
        complexity: Complexity,
        #[serde(default)]
        structure: Option<String>,
        #[serde(default)]
        persist: bool,
        #[serde(default)]
        reason: String,
    }
    let raw: Raw = serde_json::from_str(&trim_fence(text))
        .map_err(|error| invalid(format!("plan classification is invalid: {error}")))?;
    let kind = match raw.structure.as_deref() {
        Some("tree") => StructureKind::Tree,
        Some("markdown") => StructureKind::Markdown,
        Some("flat") | None => StructureKind::Flat,
        Some(other) => {
            return Err(invalid(format!(
                "plan classification names unknown structure {other:?}"
            )))
        }
    };
    Ok(Classification {
        complexity: raw.complexity,
        kind,
        persist: raw.persist,
        reason: raw.reason,
    })
}

fn trim_fence(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.split_once('\n').map_or(rest, |(_, body)| body);
        return rest.strip_suffix("```").map_or(rest, str::trim).to_owned();
    }
    trimmed.to_owned()
}

/// Turn a built plan into a persisted plan and compiled template.
pub fn materialize(
    session_id: &str,
    goal: &str,
    built: &BuiltPlan,
    template_uri: impl Into<String>,
    plan_config: &PlanConfig,
    limits: &LimitConfig,
    known_tools: &[String],
) -> Result<(Plan, ProcessTemplate), ProcessError> {
    let now = mm_data::rdf::now_ms();
    let template_uri = template_uri.into();
    let template = compile_structure(
        &built.structure,
        session_id,
        template_uri.clone(),
        limits,
        known_tools,
    )?;
    let mut plan = Plan::new(mm_data::rdf::new_id(), session_id, goal, now);
    plan.description = Some(built.structure.summary.clone());
    plan.format = if built.structure.kind == StructureKind::Markdown {
        PlanFormat::Markdown
    } else {
        PlanFormat::Structured
    };
    plan.body = built.structure.body()?;
    plan.structure = Some(
        serde_json::to_value(&built.structure)
            .map_err(|error| invalid(format!("plan structure could not be stored: {error}")))?,
    );
    plan.process_uri = Some(template_uri.clone());
    plan.derived_template_uri = Some(template_uri);
    plan.never_expires = built.classification.persist;
    plan.expires_at =
        if built.classification.persist {
            None
        } else {
            Some(now.saturating_add(
                i64::from(plan_config.default_expiry_hours).saturating_mul(3_600_000),
            ))
        };
    Ok((plan, template))
}

/// Expire plans past their deadline and return the number changed.
///
/// A plan that was explicitly marked durable is never expired. Monitoring and
/// running plans are expired too: the deadline is the contract, and the operator
/// can persist a plan to opt out.
pub async fn expire_due(plans: &dyn PlanStore, now: i64) -> Result<usize, ProcessError> {
    let mut expired = 0usize;
    for plan in plans.list_all().await.map_err(map_plan)? {
        if plan.never_expires || plan.status.is_terminal() {
            continue;
        }
        if plan.expires_at.is_none_or(|deadline| deadline > now) {
            continue;
        }
        let mut next = plan.clone();
        next.status = mm_data::PlanStatus::Expired;
        next.current_step = None;
        next.monitor = false;
        next.updated_at = now;
        plans.save(&next, plan.revision).await.map_err(map_plan)?;
        expired += 1;
    }
    Ok(expired)
}

fn map_plan(error: mm_data::PlanError) -> ProcessError {
    match error {
        mm_data::PlanError::NotFound(id) => ProcessError::NotFound { kind: "plan", id },
        mm_data::PlanError::Conflict { expected, current } => {
            ProcessError::Conflict { expected, current }
        }
        other => ProcessError::Unavailable(other.to_string()),
    }
}

/// A markdown structure used when the caller supplied prose directly.
pub fn markdown_structure(goal: &str, body: &str) -> PlanStructure {
    let mut structure = PlanStructure::markdown(goal, body);
    structure.summary = goal.to_owned();
    structure
}

/// A flat structure from existing plan steps.
pub fn flat_from_steps(title: &str, steps: &[StructureStep]) -> PlanStructure {
    PlanStructure {
        schema: STRUCTURE_SCHEMA.to_owned(),
        title: title.to_owned(),
        summary: String::new(),
        kind: StructureKind::Flat,
        steps: steps.to_vec(),
        markdown: None,
    }
}

fn invalid(message: impl Into<String>) -> ProcessError {
    ProcessError::Invalid(message.into())
}
