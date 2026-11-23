//! Generic internal-action and scheduling contracts.
//!
//! There is no per-action API, route, event, enum, or config key. A kind is a
//! reviewed [`InternalActionSpec`] looked up in [`InternalActionRegistry`]; the
//! same dispatcher and scheduler handle every kind. The one seeded kind is the
//! built-in entertainment action, so the literal appears only in its spec.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
