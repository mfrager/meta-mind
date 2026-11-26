//! Observations: the only facts about the world this phase can produce.
//!
//! An [`Observation`] is not a note the being writes about what it thinks happened.
//! It is the record of a call that *ran*, and it can only be built by
//! [`crate::executor::Executor`] from a [`crate::registry::ToolOutcome`], because
//! [`Observation::from_outcome`] demands the execution-sourced fields — the action
//! that ran, the Phase 6 evidence id, and the claim the observation will be admitted
//! under. There is no constructor that takes a plan, a summary, a confidence or a
//! model output, which is the phase's first invariant expressed as an API rather than
//! as a rule someone has to remember.
//!
//! # Why the evidence id is a constructor argument
//!
//! `ValidationBarrier::admit` refuses a claim with no evidence, so an observation
//! without one could be constructed but never admitted — a value that could only ever
//! fail. Making the evidence id required means the failure happens where it is
//! explainable ("this observation has no execution evidence") instead of at the
//! barrier, where the caller has already stopped thinking about the tool call.
//!
//! # The two graphs
//!
//! An observation contributes quads to `/tools` (the action, its outcome, its
//! artifacts) and, as a claim, to `/world` through the barrier. The split matters
//! because `/world` is rewritten from the epistemic snapshot on every mutation: the
//! *claim* belongs there and the *action record* does not, or a later promotion would
//! erase the ledger's RDF mirror.

use serde::{Deserialize, Serialize};

use crate::error::ToolError;
use crate::spec::ToolName;
use mm_core::{Timestamp, Ulid};

/// Where an observation came from.
///
/// `endpoint` names the concrete thing that was reached — a tool and the path it
/// read, a tool and the host it called — because "which endpoint" is the first
/// question a reader of an observation asks, and a record that answered "fs.read" to
/// it would be uninformative about the case that matters (one tool, many endpoints).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// The kind of source: `tool`, `process`, `http`, `store`, `graph`, `tabular`.
    pub kind: String,
    /// The concrete endpoint, e.g. the resolved path or the host.
    pub endpoint: String,
}

impl SourceRef {
    /// A tool call to a filesystem endpoint.
    pub fn tool(endpoint: impl Into<String>) -> Self {
        SourceRef {
            kind: "tool".to_string(),
            endpoint: endpoint.into(),
        }
    }

    /// A call that reached the network.
    pub fn http(host: impl Into<String>) -> Self {
        SourceRef {
            kind: "http".to_string(),
            endpoint: host.into(),
        }
    }

    /// A call that read kernel tables.
    pub fn tabular(table: impl Into<String>) -> Self {
        SourceRef {
            kind: "tabular".to_string(),
            endpoint: table.into(),
        }
    }

    /// A call that read the graph.
    pub fn graph(graph: impl Into<String>) -> Self {
        SourceRef {
            kind: "graph".to_string(),
            endpoint: graph.into(),
        }
    }

    /// Refuse a source that names nothing.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.kind.trim().is_empty() {
            return Err("the observation source has no kind".to_string());
        }
        if self.endpoint.trim().is_empty() {
            return Err("the observation source has no endpoint".to_string());
        }
        Ok(())
    }

    /// `kind:endpoint`, the `observations.source` column.
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.kind, self.endpoint)
    }
}

impl std::fmt::Display for SourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// What the observation carries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservationPayload {
    /// One sentence, from the tool's own outcome.
    pub summary: String,
    /// The machine-readable result.
    pub value: serde_json::Value,
    /// The files or resources the call produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
}

impl ObservationPayload {
    /// A payload.
    pub fn new(
        summary: impl Into<String>,
        value: serde_json::Value,
        artifacts: Vec<String>,
    ) -> Self {
        ObservationPayload {
            summary: summary.into(),
            value,
            artifacts,
        }
    }

    /// A stable hash of the payload, in canonical form.
    pub fn hash(&self) -> String {
        mm_core::hash_fields(&[
            &self.summary,
            &crate::canonical_json(&self.value),
            &self.artifacts.join(","),
        ])
    }

    /// The canonical JSON the `observed_payloads.payload_json` column carries.
    pub fn canonical_json(&self) -> String {
        crate::canonical_json(&serde_json::to_value(self).unwrap_or(serde_json::Value::Null))
    }
}

/// A record of something the environment did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// The observation's own ULID.
    pub id: Ulid,
    /// The action that produced it.
    pub action_id: Ulid,
    /// The tool that ran.
    pub tool: ToolName,
    /// Where it ran.
    pub source: SourceRef,
    /// What it produced.
    pub payload: ObservationPayload,
    /// A hash of the payload, so a re-run is comparable.
    pub payload_hash: String,
    /// The Phase 6 evidence id this observation stands on.
    pub evidence_id: Ulid,
    /// The Phase 6 claim id it is admitted under.
    pub claim_id: Ulid,
    /// When it was observed.
    pub observed_at: Timestamp,
    /// The node IRI in `/tools` that describes it.
    pub node_iri: String,
}

impl Observation {
    /// Build an observation from a tool outcome.
    ///
    /// The only constructor, and it refuses the shapes that would make an observation
    /// unfalsifiable: no evidence, no claim to admit it under, an unnamed source, or an
    /// action that is the nil ULID (which no executed action ever is).
    pub fn from_outcome(
        action_id: Ulid,
        tool: ToolName,
        source: SourceRef,
        payload: ObservationPayload,
        evidence_id: Ulid,
        claim_id: Ulid,
    ) -> std::result::Result<Self, ToolError> {
        if action_id.is_nil() {
            return Err(ToolError::Execution(
                "an observation must name the action that produced it".to_string(),
            ));
        }
        if evidence_id.is_nil() {
            return Err(ToolError::Execution(
                "an observation without execution evidence cannot be admitted to /world"
                    .to_string(),
            ));
        }
        if claim_id.is_nil() {
            return Err(ToolError::Execution(
                "an observation must name the claim it is admitted under".to_string(),
            ));
        }
        if let Err(reason) = source.validate() {
            return Err(ToolError::Execution(reason));
        }
        let observed_at = Timestamp::now();
        let id = observation_id(
            &mm_core::ulid_string(&action_id),
            tool.as_str(),
            &source.canonical(),
            &payload.hash(),
        );
        let node_iri = format!("{}{}", mm_core::iri::DATA, mm_core::ulid_string(&id));
        Ok(Observation {
            id,
            action_id,
            tool,
            source,
            payload_hash: payload.hash(),
            payload,
            evidence_id,
            claim_id,
            observed_at,
            node_iri,
        })
    }

    /// Refuse an observation that has lost one of its execution-sourced parts.
    ///
    /// Checked on the way *out* as well as in, because an observation can round-trip
    /// through the `observed_payloads` table, and a row that came back without an
    /// evidence id describes an action nobody can verify happened.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.id.is_nil() {
            return Err("observation id is the nil ULID".to_string());
        }
        if self.action_id.is_nil() {
            return Err("observation has no action id".to_string());
        }
        if self.evidence_id.is_nil() {
            return Err("observation has no evidence id".to_string());
        }
        if self.claim_id.is_nil() {
            return Err("observation has no claim id".to_string());
        }
        self.source.validate()?;
        if self.payload_hash.is_empty() {
            return Err("observation has no payload hash".to_string());
        }
        Ok(())
    }

    /// The graph the action record belongs in: `/tools`.
    ///
    /// Not `/world`: the epistemic mirror rewrites `/world` from its snapshot on every
    /// mutation, so an action record written there would be erased by the next claim
    /// promotion. The claim this observation is admitted under is what reaches
    /// `/world`.
    pub fn graph(&self) -> &'static str {
        crate::rdf::TOOLS_GRAPH
    }

    /// The Phase 6 claim this observation asserts, with the evidence that supports it.
    ///
    /// The proposition is about the *action*: `data/{action} mm:resultedIn
    /// "<payload hash>"`. Rendering the payload as its hash rather than as its JSON is
    /// deliberate: the claim is the world's copy and must be stable, while the payload
    /// itself stays in `observed_payloads` at full fidelity.
    pub fn to_claim(
        &self,
    ) -> std::result::Result<(mm_epistemic::Claim, mm_epistemic::Evidence), ToolError> {
        use mm_epistemic::{
            Claim, ClaimKind, EpistemicStatus, Evidence, EvidenceKind, Proposition,
        };

        let subject = mm_core::iri::data(&self.action_id).as_str().to_string();
        let predicate = mm_core::iri::mm("resultedIn").as_str().to_string();
        let object = self.payload_hash.clone();
        let proposition = Proposition::literal(&subject, &predicate, &object).map_err(|e| {
            ToolError::Execution(format!("the observation proposition is invalid: {e}"))
        })?;
        let mut claim = Claim::new(
            self.claim_id,
            ClaimKind::Observation,
            proposition,
            EpistemicStatus::Observed,
            0.99,
        )
        .map_err(|e| ToolError::Execution(format!("the observation claim is invalid: {e}")))?;
        claim.evidence = vec![self.evidence_id];
        let evidence = Evidence::from_content(
            self.evidence_id,
            EvidenceKind::Observation,
            None,
            &format!(
                "{}|{}|{}|{}",
                mm_core::ulid_string(&self.action_id),
                self.tool,
                self.source.canonical(),
                self.payload_hash
            ),
            0.99,
        )
        .map_err(|e| ToolError::Execution(format!("the observation evidence is invalid: {e}")))?;
        Ok((claim, evidence))
    }

    /// The `observed_payloads.payload_json` value.
    pub fn payload_json(&self) -> String {
        self.payload.canonical_json()
    }
}

/// A deterministic observation id.
///
/// Derived from the action, the tool, the source and the payload hash so that the same
/// observation has the same id across processes: a replay of the same call produces
/// byte-identical records, which is what Phase 8's replay gate compares.
pub fn observation_id(action: &str, tool: &str, source: &str, payload_hash: &str) -> Ulid {
    let digest = mm_core::content_hash(
        format!("{action}\u{1f}{tool}\u{1f}{source}\u{1f}{payload_hash}").as_bytes(),
    );
    let bytes = digest.as_bytes();
    let mut parts = [0u8; 16];
    for (index, slot) in parts.iter_mut().enumerate() {
        *slot = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
    }
    parts[0] &= 0b0000_0111;
    Ulid::from_bytes(parts)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Ulid, Ulid, Ulid) {
        (
            Ulid::from_parts(1_700_000_000_000, 1),
            Ulid::from_parts(1_700_000_000_000, 2),
            Ulid::from_parts(1_700_000_000_000, 3),
        )
    }

    fn observation() -> Observation {
        let (action, evidence, claim) = ids();
        Observation::from_outcome(
            action,
            ToolName::new("fs.read").unwrap(),
            SourceRef::tool("data/sandbox/README.md"),
            ObservationPayload::new(
                "read 12 bytes",
                serde_json::json!({ "bytes": 12 }),
                Vec::new(),
            ),
            evidence,
            claim,
        )
        .unwrap()
    }

    #[test]
    fn an_observation_without_evidence_is_refused() {
        let (action, evidence, claim) = ids();
        let payload = ObservationPayload::new("x", serde_json::json!({}), Vec::new());
        let error = Observation::from_outcome(
            action,
            ToolName::new("fs.read").unwrap(),
            SourceRef::tool("data/sandbox/a"),
            payload.clone(),
            Ulid::nil(),
            claim,
        )
        .unwrap_err();
        assert!(error.to_string().contains("execution evidence"), "{error}");
        let error = Observation::from_outcome(
            action,
            ToolName::new("fs.read").unwrap(),
            SourceRef::tool("data/sandbox/a"),
            payload.clone(),
            evidence,
            Ulid::nil(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("claim"), "{error}");
    }

    #[test]
    fn an_observation_without_a_source_is_refused() {
        let (action, evidence, claim) = ids();
        let error = Observation::from_outcome(
            action,
            ToolName::new("fs.read").unwrap(),
            SourceRef {
                kind: "tool".into(),
                endpoint: "  ".into(),
            },
            ObservationPayload::new("x", serde_json::json!({}), Vec::new()),
            evidence,
            claim,
        )
        .unwrap_err();
        assert!(error.to_string().contains("endpoint"), "{error}");
    }

    #[test]
    fn the_same_call_produces_the_same_observation_id() {
        assert_eq!(observation().id, observation().id);
        let mut other = observation();
        other.payload = ObservationPayload::new("different", serde_json::json!({}), Vec::new());
        let rebuilt = Observation::from_outcome(
            other.action_id,
            other.tool.clone(),
            other.source.clone(),
            other.payload.clone(),
            other.evidence_id,
            other.claim_id,
        )
        .unwrap();
        assert_ne!(rebuilt.id, observation().id);
    }

    #[test]
    fn a_valid_observation_becomes_a_world_admissible_claim() {
        let observation = observation();
        observation.validate().unwrap();
        let (claim, evidence) = observation.to_claim().unwrap();
        assert_eq!(claim.kind, mm_epistemic::ClaimKind::Observation);
        assert_eq!(claim.status, mm_epistemic::EpistemicStatus::Observed);
        assert_eq!(claim.evidence, vec![observation.evidence_id]);
        assert_eq!(evidence.kind, mm_epistemic::EvidenceKind::Observation);
        // The barrier is the phase-6 door, and it admits this claim.
        mm_epistemic::ValidationBarrier::admit(&claim).unwrap();
    }

    #[test]
    fn a_tampered_observation_is_refused_on_the_way_in() {
        let mut observation = observation();
        observation.evidence_id = Ulid::nil();
        let reason = observation.validate().unwrap_err();
        assert!(reason.contains("evidence"), "{reason}");
    }

    #[test]
    fn the_payload_hash_is_stable_and_the_json_is_canonical() {
        let one = ObservationPayload::new("s", serde_json::json!({ "a": 1, "b": 2 }), Vec::new());
        let two = ObservationPayload::new("s", serde_json::json!({ "b": 2, "a": 1 }), Vec::new());
        assert_eq!(one.hash(), two.hash());
        assert_eq!(one.canonical_json(), two.canonical_json());
    }

    #[test]
    fn the_action_record_belongs_to_the_tools_graph() {
        assert_eq!(observation().graph(), "tools");
    }

    #[test]
    fn sources_render_canonically() {
        assert_eq!(
            SourceRef::tool("data/sandbox/a").canonical(),
            "tool:data/sandbox/a"
        );
        assert_eq!(
            SourceRef::http("api.metamind.dev").to_string(),
            "http:api.metamind.dev"
        );
        assert!(SourceRef {
            kind: "".into(),
            endpoint: "x".into()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn an_observation_round_trips_through_json() {
        let observation = observation();
        let text = serde_json::to_string(&observation).unwrap();
        let back: Observation = serde_json::from_str(&text).unwrap();
        assert_eq!(observation, back);
        back.validate().unwrap();
    }
}
