//! Observation of a turn while it runs.
//!
//! The chat stream needs to show a turn as it happens: which pass is running,
//! what each round is writing, which tools it called, what verification found,
//! and why a retry was scheduled. None of that may change the turn, so every hook
//! is a no-op by default and an implementation cannot return an error. The
//! runner and the Process pass call these hooks in order; a caller that only
//! needs the final outcome leaves the observer unset.
//!
//! Hooks are synchronous on purpose. A transport that needs to cross an async
//! boundary (the SSE sink in `mm-serve`) hands bytes to a channel, which is
//! non-blocking; making the trait async would force every pass to await display
//! work that must never delay a model call.

use mm_llm::{DeltaSink, ProviderToolCall};

use crate::budget::BudgetSnapshot;
use crate::runner::PassKind;
use crate::verification::VerificationResult;

/// What a finished round cost and whether it produced an answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoundSummary {
    /// `ok`, `error`, or `protocol_error`.
    pub outcome: String,
    /// Prompt tokens the round reported.
    pub tokens_in: u32,
    /// Completion tokens the round reported.
    pub tokens_out: u32,
    /// Wall time of the model call.
    pub latency_ms: u64,
    /// The failure, when the round failed.
    pub error: Option<String>,
}

/// What a finished pass produced.
#[derive(Debug, Clone, PartialEq)]
pub struct PassSummary {
    /// Which pass finished.
    pub pass: Option<PassKind>,
    /// Attempt number, one-based.
    pub attempt: u32,
    /// The pass's terminal state (`completed`, `fast_path`, `failed`, ...).
    pub finished: String,
    /// The pass ledger at the end.
    pub budget: BudgetSnapshot,
    /// Hash of the last rendered prompt, when one was rendered.
    pub prompt_hash: Option<String>,
    /// Prompt tokens the pass reported.
    pub tokens_in: u32,
    /// Completion tokens the pass reported.
    pub tokens_out: u32,
    /// The pass's error, when it failed.
    pub error: Option<String>,
}

impl Default for PassSummary {
    fn default() -> Self {
        Self {
            pass: None,
            attempt: 0,
            finished: String::new(),
            budget: BudgetSnapshot {
                class: "standard",
                rounds: 0,
                steps: 0,
                tokens: 0,
                wall_ms: 0,
                estimated: false,
            },
            prompt_hash: None,
            tokens_in: 0,
            tokens_out: 0,
            error: None,
        }
    }
}

/// A turn's display-side observer.
pub trait PassObserver: Send + Sync {
    /// A pass is starting.
    fn pass_started(&self, _pass: PassKind, _attempt: u32, _model: Option<&str>) {}

    /// A pass ended, however it ended.
    fn pass_finished(&self, _summary: &PassSummary) {}

    /// One model round is starting. `streaming` is false when the round cannot
    /// stream (a structured request), so a consumer can render a pending round
    /// without pretending tokens are on the way.
    fn round_started(
        &self,
        _pass: PassKind,
        _attempt: u32,
        _round: u32,
        _streaming: bool,
        _model: Option<&str>,
    ) {
    }

    /// A fragment of the round's text.
    fn delta(&self, _pass: PassKind, _attempt: u32, _round: u32, _text: &str) {}

    /// One model round ended.
    fn round_finished(&self, _pass: PassKind, _attempt: u32, _round: u32, _summary: &RoundSummary) {
    }

    /// The model asked for a tool.
    fn tool_call(&self, _pass: PassKind, _attempt: u32, _round: u32, _call: &ProviderToolCall) {}

    /// A tool answered. `ok` is the tool's own verdict, not whether it ran.
    fn tool_result(
        &self,
        _pass: PassKind,
        _attempt: u32,
        _round: u32,
        _call: &ProviderToolCall,
        _result: &str,
        _ok: bool,
    ) {
    }

    /// A verification check finished.
    fn check(&self, _pass: PassKind, _attempt: u32, _result: &VerificationResult) {}

    /// Process will run again with `next_attempt`.
    fn retry_scheduled(&self, _pass: PassKind, _attempt: u32, _next_attempt: u32, _reason: &str) {}
}

/// The observer used when a caller did not attach one.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopObserver;

impl PassObserver for NoopObserver {}

/// Forwards model deltas to an observer, tagged with their round.
///
/// This is the only bridge between the provider-neutral [`DeltaSink`] and the
/// pass-level hooks; the runner constructs one per model call and drops it when
/// the call returns.
pub struct ObserverDeltaSink<'a> {
    observer: &'a dyn PassObserver,
    pass: PassKind,
    attempt: u32,
    round: u32,
}

impl<'a> ObserverDeltaSink<'a> {
    /// Build a sink for one round.
    pub fn new(observer: &'a dyn PassObserver, pass: PassKind, attempt: u32, round: u32) -> Self {
        Self {
            observer,
            pass,
            attempt,
            round,
        }
    }
}

impl DeltaSink for ObserverDeltaSink<'_> {
    fn delta(&mut self, delta: &str) {
        self.observer
            .delta(self.pass, self.attempt, self.round, delta);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use mm_llm::DeltaSink;

    use super::*;

    #[derive(Default)]
    struct Recorder {
        deltas: Mutex<Vec<String>>,
    }

    impl PassObserver for Recorder {
        fn delta(&self, pass: PassKind, attempt: u32, round: u32, text: &str) {
            self.deltas
                .lock()
                .unwrap()
                .push(format!("{}:{attempt}:{round}:{text}", pass.as_str()));
        }
    }

    #[test]
    fn a_delta_sink_tags_the_round_it_belongs_to() {
        let recorder = Recorder::default();
        let mut sink = ObserverDeltaSink::new(&recorder, PassKind::Process, 2, 3);
        sink.delta("hello");
        sink.delta(" world");
        assert_eq!(
            recorder.deltas.lock().unwrap().as_slice(),
            ["process:2:3:hello", "process:2:3: world"]
        );
    }

    #[test]
    fn the_default_observer_accepts_every_hook() {
        let observer = NoopObserver;
        observer.pass_started(PassKind::Interpret, 1, Some("small"));
        observer.round_started(PassKind::Interpret, 1, 1, true, Some("small"));
        observer.delta(PassKind::Interpret, 1, 1, "x");
        observer.round_finished(PassKind::Interpret, 1, 1, &RoundSummary::default());
        observer.pass_finished(&PassSummary::default());
    }
}
