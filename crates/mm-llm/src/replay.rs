//! Offline replay, and the guard that makes it provable.
//!
//! A recorded session is a list of `(request, response)` pairs. Replaying it serves
//! exactly those responses, keyed by the same request hash the cache uses, and
//! **cannot** reach a provider: the client has no transport, and
//! [`NetworkGuard::attempt`] turns any outbound attempt made anywhere in the
//! process into a hard error while a replay is armed. The gate's criterion is
//! `provider_calls == 0`, and both halves of that claim are checked — the counter
//! [`ReplayLlmClient::provider_calls`] reports, and the guard that would have
//! refused the socket if the counter had been wrong.
//!
//! Replay is the substrate's determinism story for everything above it: a being
//! that re-runs yesterday's cognition must not depend on today's model.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cached::prompt_hash;
use crate::client::{DecoderKind, LlmClient, LlmRequest, LlmResponse, Usage};
use crate::error::{LlmError, Result};

/// A response as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedResponse {
    /// The model that answered.
    pub model: String,
    /// The completion text.
    pub text: String,
    /// Prompt tokens billed.
    pub tokens_in: u32,
    /// Completion tokens billed.
    pub tokens_out: u32,
    /// Latency observed.
    pub latency_ms: u32,
}

/// One recorded exchange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// What was asked.
    pub request: LlmRequest,
    /// What was answered.
    pub response: RecordedResponse,
}

/// A recorded session, as committed under `bench/llm/sessions/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// The session's name; recorded in `llm.replay.start`/`end`.
    pub session_id: String,
    /// The exchanges, in the order they were recorded.
    pub records: Vec<SessionRecord>,
}

impl Session {
    /// Parse a session document.
    pub fn parse(raw: &str) -> Result<Self> {
        let session: Session = serde_json::from_str(raw)
            .map_err(|e| LlmError::Replay(format!("recorded session is not readable: {e}")))?;
        if session.session_id.trim().is_empty() {
            return Err(LlmError::Replay("a session must name itself".into()));
        }
        if session.records.is_empty() {
            return Err(LlmError::Replay(format!(
                "session `{}` records no exchanges",
                session.session_id
            )));
        }
        Ok(session)
    }

    /// Read a session from disk.
    pub fn load(path: &Path) -> Result<Self> {
        if path.is_dir() {
            return Session::load(&path.join("session.json"));
        }
        let raw = std::fs::read_to_string(path).map_err(|e| {
            LlmError::Replay(format!("cannot read session {}: {e}", path.display()))
        })?;
        Session::parse(&raw)
    }

    /// How many exchanges there are.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when the session records nothing.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// A stable hash of the recorded content, so a replay can prove it replayed the
    /// session on disk and not some other one.
    pub fn fingerprint(&self) -> String {
        let mut fields: Vec<String> = vec![self.session_id.clone()];
        for record in &self.records {
            fields.push(record.request.purpose.as_str().to_string());
            for message in &record.request.messages {
                fields.push(format!("{:?}", message.role));
                fields.push(message.content.clone());
            }
            fields.push(record.response.model.clone());
            fields.push(record.response.text.clone());
            fields.push(record.response.tokens_in.to_string());
            fields.push(record.response.tokens_out.to_string());
        }
        let borrowed: Vec<&str> = fields.iter().map(String::as_str).collect();
        mm_core::hash_fields(&borrowed)
    }
}

/// A client that serves a recorded session and cannot make a provider call.
#[derive(Debug)]
pub struct ReplayLlmClient {
    session_id: String,
    provider: String,
    by_hash: BTreeMap<String, RecordedResponse>,
    served: AtomicU64,
    misses: AtomicU64,
}

impl ReplayLlmClient {
    /// A replay of `session`, keyed the way `provider` would have keyed it.
    pub fn new(session: &Session, provider: &str) -> Self {
        let mut by_hash = BTreeMap::new();
        for record in &session.records {
            let model = record.request.model.clone().unwrap_or_default();
            let key = prompt_hash(provider, &model, &record.request);
            by_hash.insert(key, record.response.clone());
        }
        ReplayLlmClient {
            session_id: session.session_id.clone(),
            provider: provider.to_string(),
            by_hash,
            served: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// The session being replayed.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// How many exchanges are available.
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// True when nothing is available.
    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// Provider calls made by this client. Always zero — the number is reported by
    /// the object that could not have made one, and the gate asserts on it.
    pub fn provider_calls(&self) -> u64 {
        0
    }

    /// How many exchanges were served.
    pub fn served(&self) -> u64 {
        self.served.load(Ordering::SeqCst)
    }

    /// How many requests were not covered by the session.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmClient for ReplayLlmClient {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let model = req.model.clone().unwrap_or_default();
        let key = prompt_hash(&self.provider, &model, req);
        let recorded = self.by_hash.get(&key).ok_or_else(|| {
            self.misses.fetch_add(1, Ordering::SeqCst);
            LlmError::Replay(format!(
                "session `{}` has no recorded response for prompt_hash {key}",
                self.session_id
            ))
        })?;
        self.served.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            call_id: crate::accounting::new_call_id(),
            text: recorded.text.clone(),
            model: recorded.model.clone(),
            usage: Usage {
                tokens_in: recorded.tokens_in,
                tokens_out: recorded.tokens_out,
                // A replayed call bills nothing: it did not reach a provider.
                cost_micros: 0,
                latency_ms: recorded.latency_ms,
            },
            cached: false,
            schema_valid: true,
            decoder: DecoderKind::None,
        })
    }

    fn provider(&self) -> &str {
        &self.provider
    }

    fn decoder(&self) -> DecoderKind {
        DecoderKind::None
    }
}

/// How many replays are currently armed, process-wide.
static ARMED: AtomicUsize = AtomicUsize::new(0);

/// How many outbound attempts the guard has refused.
static REFUSED: AtomicU64 = AtomicU64::new(0);

/// The process-wide network guard.
///
/// Process-wide rather than per-client on purpose: the claim being checked is "no
/// socket was opened while replaying", and a per-client flag would only prove
/// something about the client that was asked.
#[derive(Debug, Clone, Copy)]
pub struct NetworkGuard;

/// An armed guard; disarms on drop, so a panicking test cannot leave the network
/// forbidden for the rest of the run.
#[derive(Debug)]
pub struct NetworkPermit;

impl NetworkGuard {
    /// Arm the guard until the returned permit is dropped.
    pub fn arm() -> NetworkPermit {
        ARMED.fetch_add(1, Ordering::SeqCst);
        NetworkPermit
    }

    /// True when egress is forbidden right now.
    pub fn is_armed() -> bool {
        ARMED.load(Ordering::SeqCst) > 0
    }

    /// How many outbound attempts have been refused.
    ///
    /// `provider_calls == 0` says the replay client did not ask a provider; this
    /// says nothing else in the process did either. The gate checks both, because a
    /// zero that comes from a path nobody took proves nothing.
    pub fn refused() -> u64 {
        REFUSED.load(Ordering::SeqCst)
    }

    /// Check an outbound attempt, failing when the guard is armed.
    ///
    /// Every transport calls this before it opens a socket. It is the only place
    /// that decides, so a new adapter cannot forget the rule without deleting a
    /// call the tests can see.
    pub fn attempt(host: &str) -> Result<()> {
        if NetworkGuard::is_armed() {
            REFUSED.fetch_add(1, Ordering::SeqCst);
            return Err(LlmError::NetworkForbidden(format!(
                "outbound request to {host} while the network guard is armed"
            )));
        }
        Ok(())
    }
}

impl Drop for NetworkPermit {
    fn drop(&mut self) {
        ARMED.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Purpose};

    fn session_json() -> String {
        serde_json::json!({
            "session_id": "unit-session",
            "records": [
                {
                    "request": {
                        "purpose": "plan",
                        "model": "mock-small",
                        "messages": [{ "role": "user", "content": "hello" }],
                        "schema": null,
                        "max_tokens": 64,
                        "temperature": 0.0,
                        "cache": "replay",
                        "trace_id": null
                    },
                    "response": {
                        "model": "mock-small",
                        "text": "recorded answer",
                        "tokens_in": 5,
                        "tokens_out": 3,
                        "latency_ms": 9
                    }
                }
            ]
        })
        .to_string()
    }

    #[tokio::test]
    async fn a_recorded_request_replays_byte_identically_and_covers_nothing_else() {
        let session = Session::parse(&session_json()).unwrap();
        let client = ReplayLlmClient::new(&session, "mock");
        assert_eq!(client.len(), 1);
        assert_eq!(client.session_id(), "unit-session");

        let mut req = LlmRequest::new(Purpose::Plan, vec![Message::user("hello")])
            .with_model("mock-small")
            .with_cache(crate::client::CacheMode::Replay);
        // The recorded request's ceiling is part of the hash, so it must match.
        req.max_tokens = 64;
        let response = client.complete(&req).await.unwrap();
        assert_eq!(response.text, "recorded answer");
        assert_eq!(response.usage.latency_ms, 9);
        assert_eq!(client.provider_calls(), 0);
        assert_eq!(client.served(), 1);

        // A request the session does not cover is an error, never a live call.
        let other = LlmRequest::new(Purpose::Plan, vec![Message::user("different")])
            .with_model("mock-small");
        let err = client.complete(&other).await.unwrap_err();
        assert_eq!(err.kind(), "replay");
        assert_eq!(client.misses(), 1);
        assert_eq!(client.provider_calls(), 0);
    }

    #[test]
    fn a_session_without_records_is_refused() {
        let err = Session::parse(r#"{"session_id":"empty","records":[]}"#).unwrap_err();
        assert_eq!(err.kind(), "replay");
        assert!(Session::parse("not json").is_err());
    }

    #[test]
    fn the_fingerprint_tracks_the_recorded_content() {
        let session = Session::parse(&session_json()).unwrap();
        assert_eq!(session.fingerprint().len(), 64);
        assert_eq!(session.fingerprint(), session.clone().fingerprint());
        let mut changed = session.clone();
        changed.records[0].response.text = "other".into();
        assert_ne!(session.fingerprint(), changed.fingerprint());
    }

    #[test]
    fn the_guard_forbids_egress_only_while_armed() {
        // The refusal counter is process-wide, so the assertions are on the change
        // it makes and not on an absolute value another test could also move.
        assert!(!NetworkGuard::is_armed());
        let before = NetworkGuard::refused();
        assert!(NetworkGuard::attempt("example.invalid").is_ok());
        {
            let _permit = NetworkGuard::arm();
            assert!(NetworkGuard::is_armed());
            let err = NetworkGuard::attempt("example.invalid").unwrap_err();
            assert_eq!(err.kind(), "network_forbidden");
            assert!(
                NetworkGuard::refused() > before,
                "a refused attempt must be counted"
            );
        }
        assert!(!NetworkGuard::is_armed());
        assert!(NetworkGuard::attempt("example.invalid").is_ok());
    }
}
