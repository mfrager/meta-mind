//! Attribution for model calls.
//!
//! [`CallContext`] labels a call with the work that caused it. It is attached
//! with [`scope`], a task-local that survives `.await` inside the scoped future.
//! The gateway reads the current context once per call and journals it with the
//! request and outcome; work moved to a task that cannot inherit a scope is
//! still attributed because the gateway falls back to [`CallContext::default`].

use std::future::Future;

/// Where a model call came from and which unit of work it belongs to.
///
/// All fields are cheap to clone because a scope clones its context per call.
/// `source` is a stable subsystem name (`chat`, `agent`, `automation`,
/// `process`, `summarize`, `other`); the remaining fields are absent when the
/// call is not part of a larger piece of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    /// The subsystem that made the call.
    pub source: String,
    /// The conversation session, when the call belongs to one.
    pub session_id: Option<String>,
    /// The chat run id, when the call belongs to one.
    pub run_id: Option<String>,
    /// The three-pass phase (`interpret`, `process`, `verify`, `think`).
    pub pass: Option<String>,
    /// The model round inside the phase.
    pub round: Option<u32>,
}

impl CallContext {
    /// A context with a source and nothing else.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            session_id: None,
            run_id: None,
            pass: None,
            round: None,
        }
    }

    /// Attach the conversation session.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Attach the chat run.
    pub fn with_run(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Attach the pass and round.
    pub fn with_pass(mut self, pass: impl Into<String>, round: Option<u32>) -> Self {
        self.pass = Some(pass.into());
        self.round = round;
        self
    }
}

impl Default for CallContext {
    fn default() -> Self {
        Self::new("other")
    }
}

tokio::task_local! {
    static CALL_CONTEXT: CallContext;
}

/// Run `future` with `context` available to [`current_context`].
///
/// The value is visible for the whole scoped future, including across `.await`.
/// Work moved to a newly spawned task does not inherit it; the gateway then
/// journals the default context instead.
pub fn scope<F>(context: CallContext, future: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    CALL_CONTEXT.scope(context, future)
}

/// The context of the innermost [`scope`] on this task, if any.
pub fn current_context() -> Option<CallContext> {
    CALL_CONTEXT.try_with(Clone::clone).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_defaults_to_other() {
        assert_eq!(CallContext::default().source, "other");
    }

    #[tokio::test]
    async fn a_scope_survives_awaits() {
        let context = CallContext::new("chat")
            .with_session("session-1")
            .with_run("run-1")
            .with_pass("process", Some(2));

        let seen = scope(context.clone(), async {
            tokio::task::yield_now().await;
            current_context()
        })
        .await;

        assert_eq!(seen, Some(context));
    }

    #[tokio::test]
    async fn a_scope_ends_with_its_future() {
        let context = CallContext::new("summarize");
        assert_eq!(
            scope(context, async { current_context() }).await,
            Some(CallContext::new("summarize"))
        );
        assert_eq!(current_context(), None);
    }
}
