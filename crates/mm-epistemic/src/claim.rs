//! Claims, evidence, and the record kinds that carry them.
//!
//! A [`Claim`] is a proposition plus the being's attitude toward it: a kind, a
//! status, a confidence, and the evidence it rests on. Everything else in this
//! module is the *shape* of that support:
//!
//! * [`Evidence`] — what made the claim worth holding, with a content hash so the
//!   artefact it came from can be re-checked, and a reliability the caller
//!   supplies (this crate never invents a probability) .
//! * [`Observation`] — the record that makes a promotion to `OBSERVED` legal.
//! * [`Inference`] — the rule and premises a derived claim rests on.
//! * [`Hypothesis`] — a conjecture plus the test that would settle it.
//!
//! Confidence and reliability are validated in `[0,1]` here rather than at the
//! storage edge, because a status without a well-formed confidence is not a
//! judgement anyone can act on.

use mm_core::Ulid;
use oxrdf::NamedNode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{EpistemicError, Result};
use crate::proposition::Proposition;
use crate::status::EpistemicStatus;

/// The kind of claim, which says what sort of thing it is claiming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ClaimKind {
    /// Asserted as a fact about the world.
    Fact,
    /// A record of something seen.
    Observation,
    /// Derived from other claims.
    Inference,
    /// Testable but untested.
    Hypothesis,
    /// About the future.
    Prediction,
    /// Produced by a simulation.
    Simulation,
    /// Invented; never true of the world.
    Fiction,
}

/// Every claim kind, in a stable order.
pub const CLAIM_KINDS: [ClaimKind; 7] = [
    ClaimKind::Fact,
    ClaimKind::Observation,
    ClaimKind::Inference,
    ClaimKind::Hypothesis,
    ClaimKind::Prediction,
    ClaimKind::Simulation,
    ClaimKind::Fiction,
];

impl ClaimKind {
    /// The stable wire name, matching the `claims.kind` CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimKind::Fact => "fact",
            ClaimKind::Observation => "observation",
            ClaimKind::Inference => "inference",
            ClaimKind::Hypothesis => "hypothesis",
            ClaimKind::Prediction => "prediction",
            ClaimKind::Simulation => "simulation",
            ClaimKind::Fiction => "fiction",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        CLAIM_KINDS.into_iter().find(|k| k.as_str() == text)
    }

    /// The `/epistemic` class this kind is typed as.
    pub fn rdf_class(self) -> &'static str {
        match self {
            ClaimKind::Fact => "Claim",
            ClaimKind::Observation => "Observation",
            ClaimKind::Inference => "Inference",
            ClaimKind::Hypothesis => "Hypothesis",
            ClaimKind::Prediction => "Prediction",
            ClaimKind::Simulation => "Simulation",
            ClaimKind::Fiction => "Fiction",
        }
    }

    /// The status a claim of this kind starts at when a caller does not say.
    ///
    /// A default, not a promotion: nothing here raises a status. Fiction never
    /// starts anywhere but `FICTIONAL`.
    pub fn default_status(self) -> EpistemicStatus {
        match self {
            ClaimKind::Observation => EpistemicStatus::Observed,
            ClaimKind::Fiction => EpistemicStatus::Fictional,
            ClaimKind::Simulation => EpistemicStatus::Simulated,
            ClaimKind::Prediction => EpistemicStatus::Predicted,
            ClaimKind::Hypothesis => EpistemicStatus::Hypothetical,
            ClaimKind::Inference => EpistemicStatus::Inferred,
            ClaimKind::Fact => EpistemicStatus::Reported,
        }
    }
}

impl std::fmt::Display for ClaimKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What sort of thing the evidence is.
///
/// The distinction that matters is between evidence that *proves* and evidence
/// that *reports*: [`EvidenceKind::ExternalTool`] is what `Inferred -> VERIFIED`
/// requires, and [`EvidenceKind::Observation`] is what a promotion to `OBSERVED`
/// requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EvidenceKind {
    /// A document, file, or record.
    Document,
    /// An authoritative tool or proof.
    ExternalTool,
    /// Something seen directly.
    Observation,
    /// Someone said so.
    Testimony,
    /// A trace of what happened.
    Trace,
    /// A model's output.
    ModelOutput,
}

/// Every evidence kind, in a stable order.
pub const EVIDENCE_KINDS: [EvidenceKind; 6] = [
    EvidenceKind::Document,
    EvidenceKind::ExternalTool,
    EvidenceKind::Observation,
    EvidenceKind::Testimony,
    EvidenceKind::Trace,
    EvidenceKind::ModelOutput,
];

impl EvidenceKind {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::Document => "document",
            EvidenceKind::ExternalTool => "external_tool",
            EvidenceKind::Observation => "observation",
            EvidenceKind::Testimony => "testimony",
            EvidenceKind::Trace => "trace",
            EvidenceKind::ModelOutput => "model_output",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        EVIDENCE_KINDS.into_iter().find(|k| k.as_str() == text)
    }

    /// True when the evidence can establish a claim as `VERIFIED`.
    pub fn is_authoritative(self) -> bool {
        matches!(self, EvidenceKind::ExternalTool)
    }

    /// True when the evidence *is* an observation record.
    pub fn is_observation(self) -> bool {
        matches!(self, EvidenceKind::Observation)
    }
}

impl std::fmt::Display for EvidenceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One piece of evidence for a claim.
///
/// Like [`crate::proposition::Proposition`], this serializes through a text form
/// rather than through `oxrdf`'s types: the source IRI goes on the wire as a
/// string and the content hash as lowercase hex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "EvidenceText", into = "EvidenceText")]
pub struct Evidence {
    /// The evidence's own ULID.
    pub id: Ulid,
    /// What sort of evidence it is.
    pub kind: EvidenceKind,
    /// Where it came from, when it has a resolvable source.
    pub source_uri: Option<NamedNode>,
    /// The character span it covers in the source, when it has one.
    pub span: Option<(u32, u32)>,
    /// `sha256` of the artefact, so the evidence can be re-checked.
    pub content_hash: [u8; 32],
    /// How much the source is trusted, in `[0,1]`, supplied by the caller.
    pub reliability: f32,
}

impl Evidence {
    /// Build evidence with an explicit content hash.
    pub fn new(
        id: Ulid,
        kind: EvidenceKind,
        source_uri: Option<NamedNode>,
        content_hash: [u8; 32],
        reliability: f32,
    ) -> Result<Self> {
        let evidence = Evidence {
            id,
            kind,
            source_uri,
            span: None,
            content_hash,
            reliability,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    /// Build evidence from the artefact itself, hashing it.
    pub fn from_content(
        id: Ulid,
        kind: EvidenceKind,
        source_uri: Option<NamedNode>,
        content: &str,
        reliability: f32,
    ) -> Result<Self> {
        Evidence::new(
            id,
            kind,
            source_uri,
            sha256_bytes(content.as_bytes()),
            reliability,
        )
    }

    /// The span it covers, when it has one.
    pub fn with_span(mut self, start: u32, end: u32) -> Result<Self> {
        if end < start {
            return Err(EpistemicError::validation(
                "span",
                format!("span end {end} precedes start {start}"),
            ));
        }
        self.span = Some((start, end));
        Ok(self)
    }

    /// Refuse evidence that is not well formed.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil() {
            return Err(EpistemicError::validation("id", "must not be the nil ULID"));
        }
        if !(0.0..=1.0).contains(&self.reliability) {
            return Err(EpistemicError::validation(
                "reliability",
                format!("must be in [0,1], got {}", self.reliability),
            ));
        }
        if let Some((start, end)) = self.span {
            if end < start {
                return Err(EpistemicError::validation(
                    "span",
                    format!("span end {end} precedes start {start}"),
                ));
            }
        }
        Ok(())
    }

    /// The content hash as lowercase hex.
    pub fn content_hash_hex(&self) -> String {
        hex(&self.content_hash)
    }
}

/// The serializable form of [`Evidence`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceText {
    /// The evidence's ULID.
    pub id: String,
    /// The evidence kind's wire name.
    pub kind: String,
    /// The source IRI, when there is one.
    pub source_uri: Option<String>,
    /// The span, when there is one.
    pub span: Option<(u32, u32)>,
    /// `sha256`, lowercase hex.
    pub content_hash: String,
    /// How much the source is trusted.
    pub reliability: f32,
}

impl From<Evidence> for EvidenceText {
    fn from(evidence: Evidence) -> Self {
        EvidenceText {
            id: mm_core::ulid_string(&evidence.id),
            kind: evidence.kind.as_str().to_string(),
            source_uri: evidence.source_uri.as_ref().map(|n| n.as_str().to_string()),
            span: evidence.span,
            content_hash: evidence.content_hash_hex(),
            reliability: evidence.reliability,
        }
    }
}

impl TryFrom<EvidenceText> for Evidence {
    type Error = EpistemicError;

    fn try_from(text: EvidenceText) -> Result<Self> {
        let kind = EvidenceKind::parse(&text.kind).ok_or_else(|| {
            EpistemicError::validation("kind", format!("unknown evidence kind {:?}", text.kind))
        })?;
        let source_uri = text
            .source_uri
            .as_deref()
            .map(crate::proposition::iri_node)
            .transpose()?;
        let mut evidence = Evidence::new(
            mm_core::id::parse_ulid(&text.id)?,
            kind,
            source_uri,
            unhex(&text.content_hash)?,
            text.reliability,
        )?;
        if let Some((start, end)) = text.span {
            evidence = evidence.with_span(start, end)?;
        }
        Ok(evidence)
    }
}

/// A record of something seen; what makes a promotion to `OBSERVED` legal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// The observation's own ULID.
    pub id: Ulid,
    /// The claim it observes.
    pub claim_id: Ulid,
    /// True when the observation came from an authoritative instrument rather
    /// than from the being's own attention.
    pub authoritative: bool,
    /// When it was observed.
    pub observed_at: mm_core::Timestamp,
    /// The event or activity that produced it.
    pub source: Ulid,
}

impl Observation {
    /// Build an observation.
    pub fn new(
        id: Ulid,
        claim_id: Ulid,
        authoritative: bool,
        observed_at: mm_core::Timestamp,
        source: Ulid,
    ) -> Result<Self> {
        if id.is_nil() || claim_id.is_nil() || source.is_nil() {
            return Err(EpistemicError::validation(
                "observation",
                "no part of an observation may be the nil ULID",
            ));
        }
        Ok(Observation {
            id,
            claim_id,
            authoritative,
            observed_at,
            source,
        })
    }
}

/// A derived claim: the rule and the premises it rests on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inference {
    /// The inference's own ULID.
    pub id: Ulid,
    /// The claim it concluded.
    pub claim_id: Ulid,
    /// The rule that was applied.
    pub rule: String,
    /// The claims the rule was applied to.
    pub premises: Vec<Ulid>,
}

impl Inference {
    /// Build an inference.
    pub fn new(id: Ulid, claim_id: Ulid, rule: &str, premises: Vec<Ulid>) -> Result<Self> {
        if rule.trim().is_empty() {
            return Err(EpistemicError::validation(
                "rule",
                "an inference names the rule it applied",
            ));
        }
        if premises.is_empty() {
            return Err(EpistemicError::validation(
                "premises",
                "an inference rests on at least one premise",
            ));
        }
        Ok(Inference {
            id,
            claim_id,
            rule: rule.to_string(),
            premises,
        })
    }
}

/// A conjecture and the test that would settle it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hypothesis {
    /// The hypothesis's own ULID.
    pub id: Ulid,
    /// The claim it conjectures.
    pub claim_id: Ulid,
    /// The test that would settle it.
    pub test: String,
    /// What the test is expected to show, when that is known in advance.
    pub expected: Option<String>,
}

impl Hypothesis {
    /// Build a hypothesis.
    pub fn new(id: Ulid, claim_id: Ulid, test: &str, expected: Option<String>) -> Result<Self> {
        if test.trim().is_empty() {
            return Err(EpistemicError::validation(
                "test",
                "a hypothesis names the test that would settle it",
            ));
        }
        Ok(Hypothesis {
            id,
            claim_id,
            test: test.to_string(),
            expected,
        })
    }
}

/// The longest free-text a claim may carry is bounded by the proposition itself;
/// claims carry no prose, which is what keeps this table's rows small and its
/// contents checkable. The bound exists so a caller cannot smuggle prose in
/// through an IRI.
pub const MAX_IRI_CHARS: usize = 2048;

/// A claim: a proposition, an attitude, and the evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    /// The claim's ULID.
    pub id: Ulid,
    /// What sort of claim it is.
    pub kind: ClaimKind,
    /// What it is about.
    pub proposition: Proposition,
    /// Where it stands.
    pub status: EpistemicStatus,
    /// How much it is believed, in `[0,1]`.
    pub confidence: f32,
    /// The evidence it rests on.
    pub evidence: Vec<Ulid>,
    /// When it became true in the world, if it says.
    pub valid_from: Option<mm_core::Timestamp>,
    /// When it stopped being true, if it has.
    pub valid_until: Option<mm_core::Timestamp>,
}

impl Claim {
    /// Build a claim, validating it.
    pub fn new(
        id: Ulid,
        kind: ClaimKind,
        proposition: Proposition,
        status: EpistemicStatus,
        confidence: f32,
    ) -> Result<Self> {
        let claim = Claim {
            id,
            kind,
            proposition,
            status,
            confidence,
            evidence: Vec::new(),
            valid_from: None,
            valid_until: None,
        };
        claim.validate()?;
        Ok(claim)
    }

    /// A claim whose status is the kind's default.
    pub fn of_kind(
        id: Ulid,
        kind: ClaimKind,
        proposition: Proposition,
        confidence: f32,
    ) -> Result<Self> {
        Claim::new(id, kind, proposition, kind.default_status(), confidence)
    }

    /// Attach evidence.
    pub fn with_evidence(mut self, evidence: Vec<Ulid>) -> Self {
        self.evidence = evidence;
        self
    }

    /// Attach a validity interval.
    pub fn with_validity(
        mut self,
        from: Option<mm_core::Timestamp>,
        until: Option<mm_core::Timestamp>,
    ) -> Self {
        self.valid_from = from;
        self.valid_until = until;
        self
    }

    /// Refuse a claim that cannot be reasoned about.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil() {
            return Err(EpistemicError::validation("id", "must not be the nil ULID"));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(EpistemicError::validation(
                "confidence",
                format!("must be in [0,1], got {}", self.confidence),
            ));
        }
        for (name, iri) in [
            ("subject", self.proposition.subject.as_str()),
            ("predicate", self.proposition.predicate.as_str()),
        ] {
            if iri.chars().count() > MAX_IRI_CHARS {
                return Err(EpistemicError::validation(
                    name,
                    format!("must be at most {MAX_IRI_CHARS} characters"),
                ));
            }
        }
        if let (Some(from), Some(until)) = (self.valid_from, self.valid_until) {
            if until < from {
                return Err(EpistemicError::validation(
                    "validity",
                    "valid_until precedes valid_from",
                ));
            }
        }
        Ok(())
    }

    /// The proposition, re-rendered canonically, as the row stores it.
    pub fn proposition_key(&self) -> (String, String) {
        self.proposition.key()
    }

    /// True when this claim may enter `/world` as it stands.
    pub fn is_world_admissible(&self) -> bool {
        self.status.is_world_admissible() && !self.evidence.is_empty()
    }
}

/// `sha256` of a byte slice.
pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Lowercase hex, the form the row stores.
pub fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Parse lowercase hex back into a hash.
pub fn unhex(text: &str) -> Result<[u8; 32]> {
    if text.len() != 64 {
        return Err(EpistemicError::Codec(format!(
            "a content hash is 64 hex characters, got {}",
            text.len()
        )));
    }
    let mut out = [0u8; 32];
    for (index, chunk) in text.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(chunk).map_err(|e| EpistemicError::Codec(e.to_string()))?;
        out[index] = u8::from_str_radix(pair, 16)
            .map_err(|e| EpistemicError::Codec(format!("bad hex {pair:?}: {e}")))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Timestamp;
    use ulid::Ulid;

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn proposition() -> Proposition {
        Proposition::literal("https://x/s", "https://x/p", "v").unwrap()
    }

    #[test]
    fn claim_kinds_round_trip_and_default_sensibly() {
        for kind in CLAIM_KINDS {
            assert_eq!(ClaimKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(
            ClaimKind::Observation.default_status(),
            EpistemicStatus::Observed
        );
        assert_eq!(
            ClaimKind::Fiction.default_status(),
            EpistemicStatus::Fictional
        );
        assert_eq!(ClaimKind::parse("nope"), None);
    }

    #[test]
    fn evidence_kinds_carry_the_two_deciding_predicates() {
        for kind in EVIDENCE_KINDS {
            assert_eq!(EvidenceKind::parse(kind.as_str()), Some(kind));
        }
        let authoritative: Vec<&str> = EVIDENCE_KINDS
            .into_iter()
            .filter(|k| k.is_authoritative())
            .map(EvidenceKind::as_str)
            .collect();
        assert_eq!(authoritative, vec!["external_tool"]);
        let observing: Vec<&str> = EVIDENCE_KINDS
            .into_iter()
            .filter(|k| k.is_observation())
            .map(EvidenceKind::as_str)
            .collect();
        assert_eq!(observing, vec!["observation"]);
    }

    #[test]
    fn a_claim_out_of_range_confidence_is_refused() {
        let error = Claim::new(
            id(1),
            ClaimKind::Fact,
            proposition(),
            EpistemicStatus::Reported,
            1.5,
        )
        .unwrap_err();
        assert_eq!(error.code(), "epistemic.validation");
        assert!(matches!(
            error,
            EpistemicError::Validation { ref field, .. } if field == "confidence"
        ));
    }

    #[test]
    fn an_inference_and_a_hypothesis_name_what_supports_them() {
        assert!(Inference::new(id(1), id(2), "  ", vec![id(3)]).is_err());
        assert!(Inference::new(id(1), id(2), "modus ponens", Vec::new()).is_err());
        assert!(Inference::new(id(1), id(2), "modus ponens", vec![id(3)]).is_ok());
        assert!(Hypothesis::new(id(1), id(2), "", None).is_err());
    }

    #[test]
    fn an_observation_needs_its_parts() {
        let at = Timestamp::from_epoch_seconds(1);
        assert!(Observation::new(id(1), id(2), true, at, id(3)).is_ok());
        assert!(Observation::new(id(1), Ulid::nil(), true, at, id(3)).is_err());
    }

    #[test]
    fn evidence_hashes_its_content_and_refuses_a_bad_reliability() {
        let evidence =
            Evidence::from_content(id(1), EvidenceKind::Document, None, "the text", 0.5).unwrap();
        assert_eq!(evidence.content_hash_hex().len(), 64);
        assert_eq!(
            unhex(&evidence.content_hash_hex()).unwrap(),
            evidence.content_hash
        );
        assert!(Evidence::from_content(id(1), EvidenceKind::Document, None, "x", 1.5).is_err());
        // A span that ends before it starts is a span nobody can address.
        assert!(
            Evidence::from_content(id(1), EvidenceKind::Trace, None, "x", 0.5)
                .unwrap()
                .with_span(10, 3)
                .is_err()
        );
    }

    #[test]
    fn only_a_supported_observed_or_verified_claim_is_world_admissible() {
        let mut claim = Claim::of_kind(id(1), ClaimKind::Fact, proposition(), 0.5).unwrap();
        assert!(!claim.is_world_admissible(), "a report is not the world");
        claim.status = EpistemicStatus::Observed;
        assert!(
            !claim.is_world_admissible(),
            "an observation needs its record"
        );
        claim.evidence = vec![id(9)];
        assert!(claim.is_world_admissible());
        claim.status = EpistemicStatus::Assumed;
        assert!(!claim.is_world_admissible());
    }
}
