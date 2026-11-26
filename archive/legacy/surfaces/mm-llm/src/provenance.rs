//! Provenance: one `mm:LlmCall` node per call, in the `/provenance` graph.
//!
//! The ledger (`llm_calls`) answers "what did this cost and did it succeed"; the
//! graph answers "what happened", in the same graph as everything else that
//! happened. The two are written from one account so they cannot disagree, and the
//! graph carries no prompt text and no key — only the hash and the numbers.
//!
//! The projection is a *derived* view, keyed by the call's ULID, so re-recording a
//! call is idempotent in RDF terms and a replay of the whole log converges to the
//! same graph.

use mm_store_graph::GraphHandle;

use crate::accounting::CallAccount;
use crate::error::{LlmError, Result};

/// The named graph provenance lives in.
pub const GRAPH: &str = "provenance";

/// What a call left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvenanceRecord {
    /// The call's identifier, and the node's IRI tail.
    pub call_id: String,
    /// Why the call was made.
    pub purpose: String,
    /// Who answered.
    pub provider: String,
    /// Which model answered.
    pub model: String,
    /// sha256 of the request.
    pub prompt_hash: String,
    /// Whether the output satisfied its schema.
    pub schema_ok: bool,
    /// Which decoder produced the output.
    pub decoder: String,
    /// Whether the answer came from a cache.
    pub cached: bool,
    /// Which cache layer served it, when one did.
    pub cache_layer: String,
    /// Prompt tokens.
    pub tokens_in: u32,
    /// Completion tokens.
    pub tokens_out: u32,
    /// Cost in micros.
    pub cost_micros: i64,
    /// Latency in milliseconds.
    pub latency_ms: u32,
    /// How the call ended.
    pub status: String,
    /// When.
    pub at: String,
}

impl ProvenanceRecord {
    /// The record for a finished call.
    pub fn from_account(account: &CallAccount, at: impl Into<String>) -> Self {
        ProvenanceRecord {
            call_id: mm_core::ulid_string(&account.call_id),
            purpose: account.purpose.as_str().to_string(),
            provider: account.provider.clone(),
            model: account.model.clone(),
            prompt_hash: account.prompt_hash.clone(),
            schema_ok: account.schema_ok,
            decoder: account.decoder.as_str().to_string(),
            cached: account.cached,
            cache_layer: account.cache_layer.clone().unwrap_or_default(),
            tokens_in: account.tokens_in,
            tokens_out: account.tokens_out,
            cost_micros: account.cost_micros,
            latency_ms: account.latency_ms,
            status: account.status.as_str().to_string(),
            at: at.into(),
        }
    }

    /// The node's IRI: the instance IRI of the call's ULID.
    pub fn iri(&self) -> String {
        format!("{}{}", mm_core::iri::DATA, self.call_id)
    }
}

/// Escape a Turtle string literal.
fn lit(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Render the triples for one call as plain Turtle.
pub fn to_turtle(record: &ProvenanceRecord) -> String {
    // The subject is an IRIREF, which Turtle spells inside angle brackets.
    let iri = format!("<{}>", record.iri());
    // The namespaces come from the IRI grammar rather than being spelled here, so
    // this module cannot introduce a second spelling of an IRI the kernel owns —
    // and so no endpoint-shaped literal exists in this crate's source at all.
    let mut out = format!(
        "@prefix mm: <{}> .\n@prefix prov: <{}> .\n\n",
        mm_core::iri::MM,
        mm_core::iri::PROV
    );
    let mut write = |predicate: &str, value: String| {
        out.push_str(&format!("{iri} {predicate} {value} .\n"));
    };
    write("a", "mm:LlmCall".to_string());
    write("mm:purpose", lit(&record.purpose));
    write("mm:provider", lit(&record.provider));
    write("mm:model", lit(&record.model));
    write("mm:promptHash", lit(&record.prompt_hash));
    write("mm:decoder", lit(&record.decoder));
    write(
        "mm:cached",
        lit(if record.cached { "true" } else { "false" }),
    );
    if record.cached {
        write("mm:cacheLayer", lit(&record.cache_layer));
    }
    write("mm:tokensIn", lit(&record.tokens_in.to_string()));
    write("mm:tokensOut", lit(&record.tokens_out.to_string()));
    write("mm:costMicros", lit(&record.cost_micros.to_string()));
    write("mm:latencyMs", lit(&record.latency_ms.to_string()));
    write("mm:callStatus", lit(&record.status));
    write("prov:generatedAtTime", lit(&record.at));
    out
}

/// Write one call's node into `/provenance`.
pub async fn record(graph: &GraphHandle, record: &ProvenanceRecord) -> Result<usize> {
    graph
        .insert_turtle(GRAPH, &to_turtle(record))
        .await
        .map_err(LlmError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::CallStatus;
    use crate::client::Purpose;
    use ulid::Ulid;

    fn account() -> CallAccount {
        let mut account = CallAccount::started(
            Ulid::from_parts(1_700_000_000_000, 7),
            Purpose::Extract,
            "mock",
            &"a".repeat(64),
        );
        account.model = "small".into();
        account.tokens_in = 10;
        account.tokens_out = 4;
        account.cost_micros = 12;
        account.latency_ms = 7;
        account.status = CallStatus::Ok;
        account
    }

    #[test]
    fn the_projection_carries_the_numbers_and_never_the_prompt() {
        let record = ProvenanceRecord::from_account(&account(), "2026-01-01T00:00:00Z");
        let turtle = to_turtle(&record);
        assert!(turtle.contains("mm:LlmCall"), "{turtle}");
        assert!(turtle.contains("mm:promptHash \"aaaa"), "{turtle}");
        assert!(turtle.contains("mm:tokensIn \"10\""), "{turtle}");
        assert!(turtle.contains("mm:costMicros \"12\""), "{turtle}");
        assert!(turtle.contains("prov:generatedAtTime"), "{turtle}");
        assert!(turtle.contains(&record.iri()), "{turtle}");
        assert!(
            !turtle.contains("mm:cacheLayer"),
            "an uncached call writes no layer: {turtle}"
        );
        // Every triple ends with a full stop, so the document is valid Turtle.
        for line in turtle
            .lines()
            .filter(|l| !l.starts_with('@') && !l.is_empty())
        {
            assert!(line.ends_with(" ."), "{line}");
        }
    }

    #[test]
    fn a_cached_call_records_its_layer() {
        let mut account = account();
        account.cached = true;
        account.cache_layer = Some("exact".into());
        let turtle = to_turtle(&ProvenanceRecord::from_account(&account, "now"));
        assert!(turtle.contains("mm:cacheLayer \"exact\""), "{turtle}");
    }
}
