//! Generic internal-action and scheduling contracts.
//!
//! There is no per-action API, route, event, enum, or config key. A kind is a
//! reviewed [`InternalActionSpec`] looked up in [`InternalActionRegistry`]; the
//! same dispatcher and scheduler handle every kind. The one seeded kind is the
//! built-in entertainment action, so the literal appears only in its spec.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::engine::RdfEngine;
use crate::rdf::{self, new_id, now_ms, MMD_NS};

/// Maximum form fields per action.
pub const MAX_FORM_FIELDS: usize = 8;
/// Maximum enum options per field.
pub const MAX_ENUM_OPTIONS: usize = 16;
/// Maximum text input length.
pub const MAX_ACTION_TEXT: usize = 4_096;
/// Maximum scheduled actions per session.
pub const MAX_SCHEDULED_ACTIONS: usize = 64;

/// Form field types. There are deliberately no nested objects or arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldKind {
    /// Single-line or short multi-line text.
    Text,
    /// Integer or decimal number.
    Number,
    /// Boolean.
    Bool,
    /// One of a bounded option set.
    Enum,
}

/// One bounded form field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    /// Stable key.
    pub key: String,
    /// Operator-facing label.
    pub label: String,
    /// Field type.
    pub kind: FormFieldKind,
    /// Whether the field must be supplied.
    #[serde(default)]
    pub required: bool,
    /// Enum options for `enum` fields.
    #[serde(default)]
    pub options: Vec<String>,
    /// Maximum text length.
    pub max_length: Option<usize>,
    /// Minimum numeric value.
    pub minimum: Option<i64>,
    /// Maximum numeric value.
    pub maximum: Option<i64>,
}

/// A bounded form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FormSchema {
    /// Fields in display order.
    #[serde(default)]
    pub fields: Vec<FormField>,
}

/// A validated form value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormValue {
    /// Text.
    Text(String),
    /// Number.
    Number(f64),
    /// Bool.
    Bool(bool),
}

/// Submitted form values, keyed by field key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FormValues(pub BTreeMap<String, FormValue>);

impl FormValues {
    /// Empty values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one value.
    pub fn insert(&mut self, key: impl Into<String>, value: FormValue) -> Option<FormValue> {
        self.0.insert(key.into(), value)
    }

    /// Look up a value.
    pub fn get(&self, key: &str) -> Option<&FormValue> {
        self.0.get(key)
    }
}

/// Where an action's output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputChannel {
    /// Visible conversation aside.
    Chat,
    /// RDF log only.
    Log,
    /// Inspector badge/count.
    Badge,
}

/// An action schedule. Exactly one trigger variant is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleSpec {
    /// One absolute instant.
    At {
        /// Unix ms.
        at: i64,
    },
    /// One relative delay.
    Delay {
        /// Delay in ms.
        delay_ms: u64,
    },
    /// A five-field cron expression.
    Cron {
        /// Standard five-field expression.
        expression: String,
        /// Optional IANA timezone.
        timezone: Option<String>,
    },
}

impl ScheduleSpec {
    /// Validate the trigger.
    pub fn validate(&self) -> Result<(), ActionError> {
        match self {
            Self::At { at } => {
                if *at <= 0 {
                    return Err(ActionError::Invalid(
                        "scheduled instant must be positive".into(),
                    ));
                }
            }
            Self::Delay { delay_ms } => {
                if *delay_ms == 0 || *delay_ms > 30 * 24 * 60 * 60 * 1_000 {
                    return Err(ActionError::Invalid(
                        "delay must be between 1 ms and 30 days".into(),
                    ));
                }
            }
            Self::Cron {
                expression,
                timezone,
            } => {
                validate_cron(expression)?;
                if let Some(timezone) = timezone {
                    if timezone.trim().is_empty() {
                        return Err(ActionError::Invalid(
                            "cron timezone must not be empty".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether this is a recurring trigger.
    pub fn is_recurring(&self) -> bool {
        matches!(self, Self::Cron { .. })
    }
}

fn validate_cron(expression: &str) -> Result<(), ActionError> {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(ActionError::Invalid(
            "cron expression must have five fields".into(),
        ));
    }
    for field in fields {
        if field.is_empty()
            || !field
                .chars()
                .all(|ch| ch.is_ascii_digit() || matches!(ch, '*' | '/' | ',' | '-' | '?'))
        {
            return Err(ActionError::Invalid(format!(
                "invalid cron field {field:?}"
            )));
        }
        if field.matches('/').count() > 1 {
            return Err(ActionError::Invalid(format!("invalid cron step {field:?}")));
        }
        if let Some((_, step)) = field.split_once('/') {
            let step = step
                .parse::<u32>()
                .map_err(|_| ActionError::Invalid(format!("invalid cron step in {field:?}")))?;
            if step == 0 {
                return Err(ActionError::Invalid("cron step must not be zero".into()));
            }
        }
    }
    Ok(())
}

/// One registered action kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalActionSpec {
    /// Data kind, for example the built-in `joke`.
    pub kind: String,
    /// Operator-facing title.
    pub title: String,
    /// Bounded form.
    #[serde(default)]
    pub form: FormSchema,
    /// Prompt strategy for LLM-backed kinds.
    pub prompt_strategy: Option<String>,
    /// Output destination.
    pub output: OutputChannel,
    /// Interjection policy reference.
    pub interjection_policy: String,
    /// Default schedule, if any.
    pub default_schedule: Option<ScheduleSpec>,
    /// Maximum runs.
    pub max_runs: Option<u32>,
    /// Whether operator confirmation is required for external side effects.
    #[serde(default)]
    pub requires_confirmation: bool,
}

impl InternalActionSpec {
    /// Validate the spec.
    pub fn validate(&self) -> Result<(), ActionError> {
        validate_kind_name(&self.kind)?;
        if self.title.trim().is_empty() {
            return Err(ActionError::Invalid(
                "action title must not be empty".into(),
            ));
        }
        if self.interjection_policy.trim().is_empty() {
            return Err(ActionError::Invalid(
                "interjection policy must not be empty".into(),
            ));
        }
        if let Some(strategy) = &self.prompt_strategy {
            if strategy.trim().is_empty() {
                return Err(ActionError::Invalid(
                    "prompt strategy must not be empty".into(),
                ));
            }
        }
        if let Some(schedule) = &self.default_schedule {
            schedule.validate()?;
        }
        self.form.validate()
    }

    /// Validate submitted values against this form.
    pub fn validate_values(&self, values: &FormValues) -> Result<(), ActionError> {
        self.form.validate_values(values)
    }
}

impl FormSchema {
    /// Validate field declarations.
    pub fn validate(&self) -> Result<(), ActionError> {
        if self.fields.len() > MAX_FORM_FIELDS {
            return Err(ActionError::Invalid("form exceeds the field limit".into()));
        }
        let mut keys = BTreeSet::new();
        for field in &self.fields {
            validate_field_name(&field.key)?;
            if !keys.insert(field.key.as_str()) {
                return Err(ActionError::Invalid(format!(
                    "duplicate form field {}",
                    field.key
                )));
            }
            if field.label.trim().is_empty() {
                return Err(ActionError::Invalid(format!(
                    "form field {} has an empty label",
                    field.key
                )));
            }
            if matches!(field.kind, FormFieldKind::Enum) {
                if field.options.is_empty() {
                    return Err(ActionError::Invalid(format!(
                        "enum field {} has no options",
                        field.key
                    )));
                }
                if field.options.len() > MAX_ENUM_OPTIONS {
                    return Err(ActionError::Invalid(format!(
                        "enum field {} exceeds the option limit",
                        field.key
                    )));
                }
                let mut options = BTreeSet::new();
                for option in &field.options {
                    if option.trim().is_empty() || !options.insert(option.as_str()) {
                        return Err(ActionError::Invalid(format!(
                            "enum field {} has an empty or duplicate option",
                            field.key
                        )));
                    }
                }
            }
            if field
                .max_length
                .is_some_and(|max| max == 0 || max > MAX_ACTION_TEXT)
            {
                return Err(ActionError::Invalid(format!(
                    "field {} has an invalid max_length",
                    field.key
                )));
            }
            if field
                .minimum
                .zip(field.maximum)
                .is_some_and(|(min, max)| min > max)
            {
                return Err(ActionError::Invalid(format!(
                    "field {} has minimum > maximum",
                    field.key
                )));
            }
        }
        Ok(())
    }

    /// Validate values: no unknown keys, required keys present, and every
    /// value matches its declared type and bounds.
    pub fn validate_values(&self, values: &FormValues) -> Result<(), ActionError> {
        let known: BTreeSet<&str> = self.fields.iter().map(|field| field.key.as_str()).collect();
        if let Some(unknown) = values.0.keys().find(|key| !known.contains(key.as_str())) {
            return Err(ActionError::Invalid(format!(
                "unknown form field {unknown}"
            )));
        }
        for field in &self.fields {
            match values.get(&field.key) {
                None if field.required => {
                    return Err(ActionError::Invalid(format!(
                        "missing required form field {}",
                        field.key
                    )))
                }
                None => {}
                Some(value) => match (field.kind, value) {
                    (FormFieldKind::Text, FormValue::Text(text)) => {
                        let max = field.max_length.unwrap_or(MAX_ACTION_TEXT);
                        if text.chars().count() > max {
                            return Err(ActionError::Invalid(format!(
                                "form field {} exceeds {max} characters",
                                field.key
                            )));
                        }
                    }
                    (FormFieldKind::Number, FormValue::Number(number)) => {
                        if !number.is_finite() {
                            return Err(ActionError::Invalid(format!(
                                "form field {} must be finite",
                                field.key
                            )));
                        }
                        if let Some(min) = field.minimum {
                            if *number < min as f64 {
                                return Err(ActionError::Invalid(format!(
                                    "form field {} is below its minimum",
                                    field.key
                                )));
                            }
                        }
                        if let Some(max) = field.maximum {
                            if *number > max as f64 {
                                return Err(ActionError::Invalid(format!(
                                    "form field {} is above its maximum",
                                    field.key
                                )));
                            }
                        }
                    }
                    (FormFieldKind::Bool, FormValue::Bool(_)) => {}
                    (FormFieldKind::Enum, FormValue::Text(text))
                        if field.options.iter().any(|option| option == text) => {}
                    _ => {
                        return Err(ActionError::Invalid(format!(
                            "form field {} has the wrong type",
                            field.key
                        )))
                    }
                },
            }
        }
        Ok(())
    }
}

fn validate_kind_name(kind: &str) -> Result<(), ActionError> {
    if kind.is_empty()
        || kind.len() > 32
        || !kind
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(ActionError::Invalid(format!(
            "invalid action kind {kind:?}"
        )));
    }
    Ok(())
}

fn validate_field_name(key: &str) -> Result<(), ActionError> {
    if key.is_empty()
        || key.len() > 32
        || !key
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(ActionError::Invalid(format!(
            "invalid form field name {key:?}"
        )));
    }
    Ok(())
}

/// Registry of reviewed action kinds. Every kind has exactly one spec.
#[derive(Debug, Clone, Default)]
pub struct InternalActionRegistry {
    specs: BTreeMap<String, InternalActionSpec>,
}

impl InternalActionRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The production registry. The built-in entertainment kind is one ordinary
    /// spec; its literal does not appear anywhere else in this module.
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry
            .register(InternalActionSpec {
                kind: "joke".into(),
                title: "Short diversion".into(),
                form: FormSchema::default(),
                prompt_strategy: Some("action.short_diversion.v1".into()),
                output: OutputChannel::Chat,
                interjection_policy: "default".into(),
                default_schedule: None,
                max_runs: None,
                requires_confirmation: false,
            })
            .expect("built-in spec is valid");
        registry
    }

    /// Register one spec, replacing an identical kind only if the caller
    /// explicitly builds a new registry.
    pub fn register(&mut self, spec: InternalActionSpec) -> Result<(), ActionError> {
        spec.validate()?;
        if self.specs.contains_key(&spec.kind) {
            return Err(ActionError::DuplicateKind(spec.kind));
        }
        self.specs.insert(spec.kind.clone(), spec);
        Ok(())
    }

    /// Look up a kind.
    pub fn get(&self, kind: &str) -> Option<&InternalActionSpec> {
        self.specs.get(kind)
    }

    /// Resolve a kind or return `ACTION_UNKNOWN`.
    pub fn require(&self, kind: &str) -> Result<&InternalActionSpec, ActionError> {
        self.get(kind)
            .ok_or_else(|| ActionError::UnknownKind(kind.into()))
    }

    /// All registered kinds, sorted.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.specs.keys().map(String::as_str)
    }
}

/// A durable durable scheduled-action row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledActionSpec {
    /// Owning session.
    pub session_id: String,
    /// Registered action kind.
    pub kind: String,
    /// Optional operator input.
    pub input: Option<String>,
    /// Validated form values.
    #[serde(default)]
    pub parameters: FormValues,
    /// Trigger.
    pub schedule: ScheduleSpec,
    /// Idempotency key; repeated writes with the same key are no-ops.
    pub idempotency_key: String,
    /// Status.
    pub status: ScheduledActionStatus,
    /// Creation instant.
    pub created_at: i64,
    /// Last run instant.
    pub last_run_at: Option<i64>,
    /// Completed runs.
    pub runs: u32,
}

/// Scheduled-action status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledActionStatus {
    /// Waiting for its trigger.
    Scheduled,
    /// Due/claimed.
    Claimed,
    /// Running.
    Running,
    /// Finished one successful run.
    Done,
    /// Cancelled.
    Cancelled,
    /// Failed.
    Failed,
}

impl ScheduledActionSpec {
    /// Validate against the registry and form.
    pub fn validate(&self, registry: &InternalActionRegistry) -> Result<(), ActionError> {
        if self.session_id.trim().is_empty() {
            return Err(ActionError::Invalid(
                "scheduled action must name a session".into(),
            ));
        }
        if self.idempotency_key.trim().is_empty() {
            return Err(ActionError::Invalid(
                "idempotency key must not be empty".into(),
            ));
        }
        let spec = registry.require(&self.kind)?;
        spec.validate_values(&self.parameters)?;
        self.schedule.validate()?;
        if let Some(input) = &self.input {
            if input.chars().count() > MAX_ACTION_TEXT {
                return Err(ActionError::Invalid(
                    "action input exceeds its limit".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Execution context passed to every handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionContext {
    /// Owning session.
    pub session_id: String,
    /// Authenticated user.
    pub user_id: String,
    /// Agentstream trace.
    pub trace_id: Option<String>,
    /// Origin of the request.
    pub origin: ActionOrigin,
    /// Remaining session budget, in arbitrary accounting units.
    pub budget_remaining: Option<u64>,
}

/// Who triggered the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionOrigin {
    /// The CM tool.
    Cm,
    /// Scheduler/proactive tick.
    Scheduler,
    /// Operator.
    Operator,
}

/// One execution receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    /// Action row id, when scheduled.
    pub action_id: Option<String>,
    /// Registered kind.
    pub kind: String,
    /// Status.
    pub status: ScheduledActionStatus,
    /// Compact output or error.
    pub message: String,
    /// RDF log entry id, when written.
    pub log_entry_id: Option<String>,
}

/// Action-contract failures.
#[derive(Debug, Error)]
pub enum ActionError {
    /// Invalid field/invariant.
    #[error("invalid action: {0}")]
    Invalid(String),
    /// Unknown registry kind (`ACTION_UNKNOWN`).
    #[error("action unknown: {0}")]
    UnknownKind(String),
    /// Kind already registered.
    #[error("duplicate action kind {0}")]
    DuplicateKind(String),
    /// Scheduled-action cap.
    #[error("scheduled action limit reached")]
    Limit,
    /// Store/scheduler unavailable.
    #[error("action scheduler unavailable: {0}")]
    Unavailable(String),
}

/// Result alias for actions.
pub type Result<T, E = ActionError> = std::result::Result<T, E>;

/// The `action` graph alias.
pub const ACTION_GRAPH: &str = "action";
/// `mm:ScheduledAction`.
pub const MM_SCHEDULED_ACTION: &str = "https://metamind.dev/ontology#ScheduledAction";

/// How often a scheduled action repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Recurrence {
    /// One run.
    #[default]
    Once,
    /// Every day at the same local time.
    Daily,
    /// Every week at the same local time.
    Weekly,
    /// The cron expression's own cadence.
    Cron,
}

impl Recurrence {
    /// Parse the tool value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "once" => Some(Self::Once),
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "cron" => Some(Self::Cron),
            _ => None,
        }
    }
}

/// One durable scheduled action row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledAction {
    /// Row id (`mmd:action/<ulid>`).
    pub id: String,
    /// Kind, form values, schedule, and idempotency key.
    pub spec: ScheduledActionSpec,
    /// Recurrence mode.
    pub recurrence: Recurrence,
    /// Next run instant.
    pub next_run_at: Option<i64>,
    /// Run cap.
    pub max_runs: Option<u32>,
    /// Last update.
    pub updated_at: i64,
}

/// An RDF-backed scheduled-action store.
#[derive(Debug, Clone)]
pub struct ActionStore {
    engine: Arc<RdfEngine>,
    registry: Arc<InternalActionRegistry>,
}

impl ActionStore {
    /// Build over the data engine and a reviewed registry.
    pub fn new(engine: Arc<RdfEngine>, registry: Arc<InternalActionRegistry>) -> Self {
        Self { engine, registry }
    }

    /// The registry.
    pub fn registry(&self) -> &Arc<InternalActionRegistry> {
        &self.registry
    }

    /// Schedule one action. A repeated idempotency key returns the existing row.
    pub async fn schedule(
        &self,
        spec: ScheduledActionSpec,
        recurrence: Recurrence,
        max_runs: Option<u32>,
        requested_at: i64,
    ) -> Result<ScheduledAction> {
        spec.validate(&self.registry)?;
        if let Some(input) = &spec.input {
            if crate::privacy::contains_secret(input) {
                return Err(ActionError::Invalid(
                    "action input looks like a secret and was refused".into(),
                ));
            }
        }
        let existing = self.list(&spec.session_id).await?;
        if let Some(found) = existing
            .iter()
            .find(|action| action.spec.idempotency_key == spec.idempotency_key)
        {
            return Ok(found.clone());
        }
        let live = existing
            .iter()
            .filter(|action| {
                matches!(
                    action.spec.status,
                    ScheduledActionStatus::Scheduled | ScheduledActionStatus::Claimed
                )
            })
            .count();
        if live >= MAX_SCHEDULED_ACTIONS {
            return Err(ActionError::Limit);
        }
        if matches!(spec.schedule, ScheduleSpec::Cron { .. }) {
            let cap = std::env::var("MM_AUTOMATION_MAX_CRON_JOBS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(20);
            let cron_live = existing
                .iter()
                .filter(|action| {
                    matches!(action.spec.schedule, ScheduleSpec::Cron { .. })
                        && matches!(
                            action.spec.status,
                            ScheduledActionStatus::Scheduled | ScheduledActionStatus::Claimed
                        )
                })
                .count();
            if cron_live >= cap {
                return Err(ActionError::Limit);
            }
        }
        if max_runs.is_some_and(|max| max == 0 || max > 1_000) {
            return Err(ActionError::Invalid("max_runs must be 1–1000".into()));
        }
        let next = next_run_at(&spec.schedule, recurrence, requested_at)?;
        if next <= requested_at && recurrence == Recurrence::Once {
            return Err(ActionError::Invalid(
                "a one-shot action must be scheduled in the future".into(),
            ));
        }
        let mut action = ScheduledAction {
            id: format!("{MMD_NS}action/{}", new_id()),
            spec,
            recurrence,
            next_run_at: Some(next),
            max_runs,
            updated_at: requested_at,
        };
        action.spec.status = ScheduledActionStatus::Scheduled;
        self.write(&action).await?;
        Ok(action)
    }

    /// One row by id.
    pub async fn get(&self, action_id: &str) -> Result<Option<ScheduledAction>> {
        rdf::load_as(&self.engine, ACTION_GRAPH, action_id)
            .await
            .map_err(store_error)
    }

    /// Every row for a session, newest first.
    pub async fn list(&self, session_id: &str) -> Result<Vec<ScheduledAction>> {
        let mut actions: Vec<ScheduledAction> =
            rdf::list_as(&self.engine, ACTION_GRAPH, MM_SCHEDULED_ACTION)
                .await
                .map_err(store_error)?;
        actions.retain(|action| action.spec.session_id == session_id);
        actions.sort_by_key(|action| std::cmp::Reverse(action.spec.created_at));
        Ok(actions)
    }

    /// Cancel a row. Repeated cancellation is a no-op.
    pub async fn cancel(&self, action_id: &str) -> Result<bool> {
        let Some(mut action) = self.get(action_id).await? else {
            return Ok(false);
        };
        if matches!(
            action.spec.status,
            ScheduledActionStatus::Cancelled | ScheduledActionStatus::Done
        ) {
            return Ok(true);
        }
        action.spec.status = ScheduledActionStatus::Cancelled;
        action.next_run_at = None;
        action.updated_at = now_ms();
        self.write(&action).await?;
        Ok(true)
    }

    /// Claim every due row, marking it running.
    pub async fn claim_due(&self, at: i64) -> Result<Vec<ScheduledAction>> {
        let mut all: Vec<ScheduledAction> =
            rdf::list_as(&self.engine, ACTION_GRAPH, MM_SCHEDULED_ACTION)
                .await
                .map_err(store_error)?;
        all.sort_by_key(|action| action.spec.created_at);
        self.claim_rows(all, None, at).await
    }

    /// Claim due rows for one session. A session supervisor never consumes
    /// another session's actions.
    pub async fn claim_due_for_session(
        &self,
        session_id: &str,
        at: i64,
    ) -> Result<Vec<ScheduledAction>> {
        let mut all = self.list(session_id).await?;
        all.sort_by_key(|action| action.spec.created_at);
        self.claim_rows(all, Some(session_id), at).await
    }

    async fn claim_rows(
        &self,
        all: Vec<ScheduledAction>,
        session_id: Option<&str>,
        at: i64,
    ) -> Result<Vec<ScheduledAction>> {
        let mut claimed = Vec::new();
        for mut action in all {
            if session_id.is_some_and(|id| action.spec.session_id != id) {
                continue;
            }
            if action.spec.status != ScheduledActionStatus::Scheduled {
                continue;
            }
            if action.next_run_at.is_none_or(|due| due > at) {
                continue;
            }
            action.spec.status = ScheduledActionStatus::Claimed;
            action.updated_at = at;
            self.write(&action).await?;
            claimed.push(action);
        }
        Ok(claimed)
    }

    /// Record one successful run and advance or finish the schedule.
    pub async fn finish_run(&self, action_id: &str, at: i64) -> Result<Option<ScheduledAction>> {
        let Some(mut action) = self.get(action_id).await? else {
            return Ok(None);
        };
        action.spec.runs = action.spec.runs.saturating_add(1);
        action.spec.last_run_at = Some(at);
        action.updated_at = at;
        let capped = action.max_runs.is_some_and(|max| action.spec.runs >= max);
        if action.recurrence == Recurrence::Once || capped {
            action.spec.status = ScheduledActionStatus::Done;
            action.next_run_at = None;
        } else {
            action.spec.status = ScheduledActionStatus::Scheduled;
            action.next_run_at = Some(next_run_at(&action.spec.schedule, action.recurrence, at)?);
        }
        self.write(&action).await?;
        Ok(Some(action))
    }

    /// Record a failed run.
    pub async fn fail_run(&self, action_id: &str, at: i64) -> Result<()> {
        let Some(mut action) = self.get(action_id).await? else {
            return Ok(());
        };
        action.spec.status = ScheduledActionStatus::Failed;
        action.next_run_at = None;
        action.updated_at = at;
        self.write(&action).await
    }

    async fn write(&self, action: &ScheduledAction) -> Result<()> {
        let graph = crate::graphs::resolve_graph(ACTION_GRAPH);
        let payload = serde_json::to_value(action)
            .map_err(|error| ActionError::Invalid(error.to_string()))?;
        let payload = serde_json::Value::Object({
            let mut map = payload.as_object().cloned().unwrap_or_default();
            map.insert(
                "_rev".to_owned(),
                serde_json::Value::from(rdf::next_revision()),
            );
            map
        });
        let quads = vec![
            RdfEngine::quad(
                &graph,
                &action.id,
                rdf::RDF_TYPE,
                rdf::iri(MM_SCHEDULED_ACTION),
            ),
            RdfEngine::quad(
                &graph,
                &action.id,
                rdf::PAYLOAD,
                rdf::lit(serde_json::to_string(&payload).unwrap_or_default()),
            ),
            RdfEngine::quad(
                &graph,
                &action.id,
                &format!("{}kind", rdf::MM_NS),
                rdf::lit(action.spec.kind.clone()),
            ),
            RdfEngine::quad(
                &graph,
                &action.id,
                &format!("{}actionStatus", rdf::MM_NS),
                rdf::lit(format!("{:?}", action.spec.status).to_lowercase()),
            ),
        ];
        let deletes = self
            .engine
            .store()
            .quads_for_subject(&graph, &action.id)
            .await
            .map_err(store_error)?;
        self.engine
            .store()
            .handle()
            .apply_batch(mm_store_graph::GraphBatch {
                deletes,
                inserts: quads,
            })
            .await
            .map_err(store_error)?;
        Ok(())
    }
}

fn next_run_at(schedule: &ScheduleSpec, recurrence: Recurrence, from: i64) -> Result<i64> {
    let base = match schedule {
        ScheduleSpec::At { at } => *at,
        ScheduleSpec::Delay { delay_ms } => from.saturating_add(*delay_ms as i64),
        ScheduleSpec::Cron { .. } => from,
    };
    let next = match recurrence {
        Recurrence::Once => base,
        Recurrence::Daily => {
            if base > from {
                base
            } else {
                from.saturating_add(24 * 60 * 60 * 1_000)
            }
        }
        Recurrence::Weekly => {
            if base > from {
                base
            } else {
                from.saturating_add(7 * 24 * 60 * 60 * 1_000)
            }
        }
        Recurrence::Cron => match schedule {
            ScheduleSpec::Cron { expression, .. } => next_cron(expression, from)?,
            _ => from.saturating_add(60_000),
        },
    };
    Ok(next)
}

/// Minimal five-field cron next-run for the common step/minute forms.
fn next_cron(expression: &str, from: i64) -> Result<i64> {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(ActionError::Invalid("cron must have five fields".into()));
    }
    let minute_step = if fields[0] == "*" {
        1
    } else if let Some(step) = fields[0].strip_prefix("*/") {
        step.parse::<i64>()
            .map_err(|_| ActionError::Invalid("invalid cron minute step".into()))?
    } else {
        1
    };
    let step_ms = minute_step.max(1) * 60_000;
    Ok(from.saturating_add(step_ms))
}

fn store_error(error: impl std::fmt::Display) -> ActionError {
    ActionError::Unavailable(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_the_entertainment_kind_only() {
        let registry = InternalActionRegistry::builtin();
        assert_eq!(registry.kinds().collect::<Vec<_>>(), vec!["joke"]);
        assert!(registry.require("joke").is_ok());
        assert!(matches!(
            registry.require("lookup"),
            Err(ActionError::UnknownKind(_))
        ));
    }

    #[test]
    fn unknown_form_fields_are_rejected() {
        let mut schema = FormSchema::default();
        schema.fields.push(FormField {
            key: "style".into(),
            label: "Style".into(),
            kind: FormFieldKind::Enum,
            required: true,
            options: vec!["dry".into(), "warm".into()],
            max_length: None,
            minimum: None,
            maximum: None,
        });
        schema.validate().unwrap();
        let mut values = FormValues::new();
        values.insert("style", FormValue::Text("dry".into()));
        assert!(schema.validate_values(&values).is_ok());
        values.insert("unexpected", FormValue::Text("x".into()));
        assert!(schema.validate_values(&values).is_err());
    }

    #[test]
    fn scheduling_validates_cron_and_delay() {
        assert!(ScheduleSpec::Cron {
            expression: "*/5 * * * *".into(),
            timezone: None,
        }
        .validate()
        .is_ok());
        assert!(ScheduleSpec::Cron {
            expression: "* * * *".into(),
            timezone: None,
        }
        .validate()
        .is_err());
        assert!(ScheduleSpec::Delay { delay_ms: 0 }.validate().is_err());
    }

    #[test]
    fn scheduling_is_generic_for_any_registered_kind() {
        let registry = InternalActionRegistry::builtin();
        let action = ScheduledActionSpec {
            session_id: "s".into(),
            kind: "joke".into(),
            input: None,
            parameters: FormValues::new(),
            schedule: ScheduleSpec::Cron {
                expression: "*/5 * * * *".into(),
                timezone: None,
            },
            idempotency_key: "joke:5m".into(),
            status: ScheduledActionStatus::Scheduled,
            created_at: 1,
            last_run_at: None,
            runs: 0,
        };
        assert!(action.validate(&registry).is_ok());
    }
}
