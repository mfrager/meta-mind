//! Tracing hooks: one span per instance, and a span per transition.
//!
//! The spans carry the instance id, machine name and state endpoints, so an
//! OpenTelemetry subscriber sees the state-machine hierarchy as nested spans and
//! every taken transition as a child span of its instance. When no subscriber is
//! installed these are cheap no-ops.

use ::tracing::Span;

use crate::machine::StateId;

/// A span for one running instance.
pub(crate) fn instance_span(instance: &str, machine: &str) -> Span {
    ::tracing::debug_span!("sm_instance", instance = instance, machine = machine)
}

/// A span for one taken transition, visible while its action runs.
pub(crate) fn transition_span(
    instance: &str,
    machine: &str,
    from: &StateId,
    to: &StateId,
    event: &str,
) -> Span {
    ::tracing::debug_span!(
        "sm_transition",
        instance = instance,
        machine = machine,
        from = %from,
        to = %to,
        event = event
    )
}
