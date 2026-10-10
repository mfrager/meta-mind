//! The experience compiler: trajectories plus insights become **draft** entries.
//!
//! Design §108 / Phase 7 asks for experience to accumulate as something the
//! library can validate and select, not as prose. The compiler is the step between
//! a run and a candidate: it reads what happened (trajectories) and what was
//! concluded (insights), and emits techniques — and, alongside them, policy
//! deltas — that are *candidates only*.
//!
//! Three rules make that safe:
//!
//! 1. **Nothing is promoted here.** Promotion is Phase 11's job and it wants
//!    evidence; [`draft_confidence`] rises with repetition but is capped at
//!    [`MAX_DRAFT_CONFIDENCE`], strictly below [`PROMOTION_THRESHOLD`]. A compiler
//!    that could cross the threshold on its own would be a way to launder a single
//!    run into a general rule.
//! 2. **Repetition is support, not confidence.** The same lesson seen in three
//!    trajectories is *one* candidate with `support = 3`, not three entries — the
//!    library's duplicate-by-content rule applied before the write rather than
//!    after.
//! 3. **Determinism.** Input order in, sorted slugs out, fixed arithmetic
//!    throughout: two compiles of the same input are byte-identical, which is what
//!    the gold file for `experience compile` compares.
//!
//! A lesson with no supporting insight is **not** emitted. A technique needs
//! evidence to pass the shapes, and inventing a citation to satisfy a gate would
//! make the gate meaningless; the caller instead sees the lesson in
//! [`CompiledExperience::support`] with a count of zero, which says exactly what
//! is missing.

use std::collections::{BTreeMap, BTreeSet};

use mm_core::NamedNode;
use serde::Deserialize;

use crate::doctrine::when;
use crate::entry::{EntryKind, LibraryEntry};
use crate::error::{LibraryError, Result};
use crate::policy::{PolicyBehavior, PolicyDelta, Scope};
use crate::technique::technique;

/// The confidence at or above which an entry stops being a draft.
///
/// Phase 11 owns promotion; the compiler is deliberately never allowed to reach
/// this value.
pub const PROMOTION_THRESHOLD: f32 = 0.6;
/// The highest confidence a compiled draft may carry.
pub const MAX_DRAFT_CONFIDENCE: f32 = 0.5;
/// The confidence a single unrepeated lesson starts from.
pub const BASE_CONFIDENCE: f32 = 0.2;
/// How much each further supporting trajectory adds.
pub const CONFIDENCE_STEP: f32 = 0.05;
/// The longest slug the library accepts (`entry::MAX_SLUG_CHARS`).
pub const MAX_SLUG_CHARS: usize = 64;
/// The shortest token that can count as shared evidence.
pub const MIN_TOKEN_CHARS: usize = 4;

/// Words that carry no evidence, in the selector's sense.
const STOPWORDS: [&str; 16] = [
    "the", "and", "for", "with", "that", "this", "into", "from", "when", "then", "than", "each",
    "only", "must", "have", "been",
];

/// One completed (or failed) run, as the compiler sees it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Trajectory {
    /// A stable identifier for the run.
    pub id: String,
    /// How it turned out: `success`, `failure`, or anything the caller records.
    #[serde(default)]
    pub outcome: String,
    /// The steps taken, in order.
    #[serde(default)]
    pub steps: Vec<String>,
    /// What the run taught, in one sentence. This is what becomes a candidate.
    #[serde(default)]
    pub lesson: String,
    /// The domains the run belonged to.
    #[serde(default)]
    pub domains: Vec<String>,
    /// IRIs that back the lesson.
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl Trajectory {
    /// Parse one trajectory per non-empty JSONL line.
    ///
    /// A line that is not a well-formed trajectory is a typed refusal naming the
    /// line, because a silently skipped line is a lesson nobody notices is missing.
    pub fn from_jsonl(text: &str) -> Result<Vec<Self>> {
        let mut out = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let trajectory: Trajectory = serde_json::from_str(line).map_err(|e| {
                LibraryError::validation(
                    "trajectory",
                    format!("line {} is not a trajectory: {e}", index + 1),
                )
            })?;
            if trajectory.id.trim().is_empty() {
                return Err(LibraryError::validation(
                    "trajectory",
                    format!("line {} has an empty id", index + 1),
                ));
            }
            out.push(trajectory);
        }
        Ok(out)
    }
}

/// One insight, as the compiler consumes it.
///
/// Deliberately not [`crate::store::LibraryStore`]'s row type: the compiler is a
/// pure function of its inputs so it can be tested and gold-checked without a
/// store, and the CLI is what reads rows into this shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insight {
    /// The insight's IRI.
    pub iri: String,
    /// What it says.
    pub text: String,
    /// Its kind.
    pub kind: String,
    /// What backs it.
    pub evidence: Vec<String>,
    /// Votes in favour.
    pub upvotes: i64,
    /// Votes against.
    pub downvotes: i64,
}

/// The candidates a compile produced.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledExperience {
    /// Candidate techniques, one per distinct lesson, in slug order.
    pub techniques: Vec<crate::entry::Declarative>,
    /// Candidate policy deltas, one per technique, in the same order.
    pub policies: Vec<PolicyDelta>,
    /// How many trajectories supported each lesson slug. A slug with `0` is a
    /// lesson that was seen but had no supporting insight, and was therefore not
    /// emitted.
    pub support: BTreeMap<String, u32>,
}

impl CompiledExperience {
    /// True when every emitted technique is below the promotion threshold.
    pub fn is_sub_threshold(&self) -> bool {
        self.techniques
            .iter()
            .all(|entry| entry.confidence.is_some_and(|c| c < PROMOTION_THRESHOLD))
    }

    /// The candidates as JSONL: one technique object per line, then one policy
    /// delta object per line.
    ///
    /// Field order is fixed by construction (`serde_json::json!` preserves the
    /// literal's order), so the file is diffable and the gold check is exact.
    pub fn to_jsonl(&self) -> Result<String> {
        let mut out = String::new();
        for entry in &self.techniques {
            let slug = entry.slug();
            let confidence = entry.confidence.unwrap_or(0.0);
            let line = serde_json::json!({
                "kind": "Technique",
                "slug": slug,
                "iri": entry.iri.as_str(),
                "title": entry.title,
                "purpose": entry.purpose,
                "confidence": confidence,
                "support": self.support.get(&slug).copied().unwrap_or(0),
                "evidence": entry
                    .evidence
                    .iter()
                    .map(|node| node.as_str().to_string())
                    .collect::<Vec<_>>(),
                "domains": entry.domain.clone(),
                "applicable_when": entry
                    .applicable_when
                    .iter()
                    .map(|condition| condition.as_str().to_string())
                    .collect::<Vec<_>>(),
            });
            out.push_str(
                &serde_json::to_string(&line)
                    .map_err(|e| LibraryError::Internal(format!("compiled technique: {e}")))?,
            );
            out.push('\n');
        }
        for (index, delta) in self.policies.iter().enumerate() {
            let line = serde_json::json!({
                "kind": "PolicyDelta",
                "slug": self.techniques.get(index).map(|entry| entry.slug()).unwrap_or_default(),
                "behavior": delta.behavior.as_str(),
                "scope": "global",
                "activation": delta.activation.as_str(),
                "confidence": delta.confidence,
                "reason": delta.reason,
            });
            out.push_str(
                &serde_json::to_string(&line)
                    .map_err(|e| LibraryError::Internal(format!("compiled policy: {e}")))?,
            );
            out.push('\n');
        }
        Ok(out)
    }

    /// How many candidates were emitted.
    pub fn len(&self) -> usize {
        self.techniques.len()
    }

    /// True when nothing was emitted.
    pub fn is_empty(&self) -> bool {
        self.techniques.is_empty()
    }
}

/// The confidence a draft with this much support carries.
///
/// Strictly increasing in `support`, capped at [`MAX_DRAFT_CONFIDENCE`] and
/// therefore always below [`PROMOTION_THRESHOLD`]: repetition is evidence that a
/// lesson is worth *reviewing*, never a licence to generalise it.
pub fn draft_confidence(support: u32) -> f32 {
    let raised = BASE_CONFIDENCE + CONFIDENCE_STEP * support.saturating_sub(1) as f32;
    MAX_DRAFT_CONFIDENCE.min(raised)
}

/// Compile trajectories and insights into candidate entries.
///
/// Deterministic: trajectories and insights are read in input order, candidates
/// are emitted in slug order, and every number is fixed arithmetic over the
/// support count.
pub fn compile(trajectories: &[Trajectory], insights: &[Insight]) -> CompiledExperience {
    let mut by_slug: BTreeMap<String, Accumulator> = BTreeMap::new();

    for trajectory in trajectories {
        let lesson = trajectory.lesson.trim();
        if lesson.is_empty() {
            continue;
        }
        let slug = slugify(lesson);
        if slug.is_empty() {
            continue;
        }
        let entry = by_slug.entry(slug).or_insert_with(|| Accumulator {
            lesson: lesson.to_string(),
            support: 0,
            domains: BTreeSet::new(),
            applicable_when: BTreeSet::new(),
            evidence: BTreeSet::new(),
            trajectories: Vec::new(),
        });
        entry.support += 1;
        entry.trajectories.push(trajectory.id.clone());
        for domain in &trajectory.domains {
            let domain = domain.trim();
            if !domain.is_empty() {
                entry.domains.insert(domain.to_string());
                entry.applicable_when.insert(domain.to_string());
            }
        }
        for iri in &trajectory.evidence {
            entry.evidence.insert(iri.clone());
        }
        let lesson_tokens = tokens(lesson);
        for insight in insights {
            if !insight.iri.trim().is_empty() && shares_token(&lesson_tokens, &insight.text) {
                entry.evidence.insert(insight.iri.clone());
            }
        }
    }

    let mut techniques = Vec::new();
    let mut policies = Vec::new();
    let mut support = BTreeMap::new();

    for (slug, accumulator) in &by_slug {
        let evidence = parse_iris(accumulator.evidence.iter().map(String::as_str));
        if evidence.is_empty() {
            // Seen, but nothing backs it: visible to the caller, not emitted.
            support.insert(slug.clone(), 0);
            continue;
        }
        let confidence = draft_confidence(accumulator.support);
        let conditions = accumulator
            .applicable_when
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let conditions = if conditions.is_empty() {
            vec![accumulator.lesson.clone()]
        } else {
            conditions
        };

        let mut entry = match technique(
            slug,
            &title_of(&accumulator.lesson),
            &format!(
                "A technique distilled from {} trajectory(ies).",
                accumulator.support
            ),
        ) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        for condition in &conditions {
            if let Ok(condition) = when(condition) {
                entry = entry.when(condition);
            }
        }
        for domain in &accumulator.domains {
            entry = entry.with_domain(domain);
        }
        for node in &evidence {
            entry = entry.backed_by(node.clone()).illustrated_by(node.clone());
        }
        entry = match entry.with_confidence(confidence) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        for trajectory in &accumulator.trajectories {
            entry = entry.derived_from(trajectory);
        }
        if !entry.referenced_iris().is_empty() && entry.kind() == EntryKind::Technique {
            support.insert(slug.clone(), accumulator.support);
            policies.push(PolicyDelta {
                behavior: PolicyBehavior::from_weights([
                    f64::from(confidence),
                    f64::from(MAX_DRAFT_CONFIDENCE),
                    f64::from(1.0 - MAX_DRAFT_CONFIDENCE),
                ]),
                scope: Scope::Global,
                activation: conditions
                    .first()
                    .and_then(|condition| when(condition).ok())
                    .unwrap_or_else(|| when("unclear").expect("a literal condition parses")),
                confidence,
                reason: format!(
                    "distilled from {} trajectory(ies) with support {}",
                    accumulator.support, accumulator.support
                ),
            });
            techniques.push(entry);
        }
    }

    CompiledExperience {
        techniques,
        policies,
        support,
    }
}

/// A lesson slug: lowercase, `[a-z0-9]` and single `-` separators, bounded.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.chars().count() > MAX_SLUG_CHARS {
        out = out.chars().take(MAX_SLUG_CHARS).collect();
        while out.ends_with('-') {
            out.pop();
        }
    }
    out
}

/// A title that fits comfortably in a document: the first sentence, bounded.
fn title_of(lesson: &str) -> String {
    let first = lesson.split(['.', '\n']).next().unwrap_or(lesson).trim();
    if first.chars().count() <= 120 {
        first.to_string()
    } else {
        first.chars().take(120).collect()
    }
}

/// The evidence-bearing tokens of a lesson.
fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= MIN_TOKEN_CHARS)
        .map(str::to_ascii_lowercase)
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .collect()
}

/// True when an insight shares a non-trivial token with the lesson.
fn shares_token(lesson_tokens: &BTreeSet<String>, insight_text: &str) -> bool {
    if lesson_tokens.is_empty() {
        return false;
    }
    let insight_tokens = tokens(insight_text);
    lesson_tokens
        .iter()
        .any(|token| insight_tokens.contains(token))
}

/// Parse a set of IRIs, skipping anything that is not one rather than inventing a
/// replacement.
fn parse_iris<'a>(iris: impl Iterator<Item = &'a str>) -> Vec<NamedNode> {
    let mut out = Vec::new();
    for iri in iris {
        if let Ok(node) = NamedNode::new(iri.to_string()) {
            out.push(node);
        }
    }
    out
}

/// The accumulator for one lesson slug.
struct Accumulator {
    lesson: String,
    support: u32,
    domains: BTreeSet<String>,
    applicable_when: BTreeSet<String>,
    evidence: BTreeSet<String>,
    trajectories: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trajectory(id: &str, lesson: &str, domains: &[&str]) -> Trajectory {
        Trajectory {
            id: id.to_string(),
            outcome: "success".to_string(),
            steps: vec!["look at the failing test".to_string()],
            lesson: lesson.to_string(),
            domains: domains.iter().map(|d| (*d).to_string()).collect(),
            evidence: vec!["https://metamind.dev/library/case/one".to_string()],
        }
    }

    fn insight(iri: &str, text: &str) -> Insight {
        Insight {
            iri: iri.to_string(),
            text: text.to_string(),
            kind: "pattern".to_string(),
            evidence: vec![],
            upvotes: 3,
            downvotes: 0,
        }
    }

    #[test]
    fn repeated_lessons_merge_into_one_candidate() {
        let trajectories = vec![
            trajectory("01", "Check the cache before the network", &["software"]),
            trajectory("02", "Check the cache before the network", &["software"]),
            trajectory("03", "Read the error before the stack", &["debugging"]),
        ];
        let insights = vec![
            insight(
                "https://metamind.dev/data/cache",
                "checking the cache saves a network call",
            ),
            insight(
                "https://metamind.dev/data/error",
                "reading the error first narrows the search",
            ),
        ];
        let compiled = compile(&trajectories, &insights);
        assert_eq!(compiled.techniques.len(), 2);
        assert_eq!(
            compiled.support["check-the-cache-before-the-network"], 2,
            "identical lessons are one candidate with support 2"
        );
        let repeated = compiled
            .techniques
            .iter()
            .find(|entry| entry.slug() == "check-the-cache-before-the-network")
            .unwrap();
        assert_eq!(repeated.derived_from_trajectory.len(), 2);
    }

    #[test]
    fn confidence_rises_with_support_and_never_reaches_the_threshold() {
        assert!(draft_confidence(1) < draft_confidence(2));
        assert!(draft_confidence(2) < draft_confidence(3));
        for support in 1..50u32 {
            assert!(
                draft_confidence(support) < PROMOTION_THRESHOLD,
                "support {support} must stay below the promotion threshold"
            );
        }
        let trajectories = vec![
            trajectory("01", "Keep the change small", &["software"]),
            trajectory("02", "Keep the change small", &["software"]),
            trajectory("03", "Keep the change small", &["software"]),
        ];
        let insights = vec![insight(
            "https://metamind.dev/data/small",
            "small changes land faster",
        )];
        assert!(compile(&trajectories, &insights).is_sub_threshold());
    }

    #[test]
    fn two_compiles_of_the_same_input_are_byte_identical() {
        let trajectories = vec![
            trajectory("01", "Name the assumption before the plan", &["planning"]),
            trajectory("02", "Name the assumption before the plan", &["planning"]),
            trajectory("03", "Say what would falsify it", &["planning", "science"]),
        ];
        let insights = vec![
            insight(
                "https://metamind.dev/data/assumption",
                "an unnamed assumption cannot be checked",
            ),
            insight(
                "https://metamind.dev/data/falsify",
                "saying what would falsify an idea sharpens it",
            ),
        ];
        let first = compile(&trajectories, &insights);
        let second = compile(&trajectories, &insights);
        assert_eq!(first, second);
        assert_eq!(first.to_jsonl().unwrap(), second.to_jsonl().unwrap());
        assert!(first.to_jsonl().unwrap().contains("\"kind\":\"Technique\""));
        assert!(first
            .to_jsonl()
            .unwrap()
            .contains("\"kind\":\"PolicyDelta\""));
    }

    #[test]
    fn a_lesson_with_no_supporting_insight_is_counted_but_not_emitted() {
        let mut bare = trajectory("01", "Rotate the credentials quarterly", &["ops"]);
        // Nothing backs this lesson: no trajectory evidence, no insight that
        // shares a token with it.
        bare.evidence.clear();
        let trajectories = vec![bare];
        let compiled = compile(&trajectories, &[]);
        assert!(
            compiled.is_empty(),
            "nothing backs it, so nothing is written"
        );
        assert_eq!(compiled.support["rotate-the-credentials-quarterly"], 0);
    }

    #[test]
    fn a_malformed_trajectory_line_is_a_typed_error_naming_the_line() {
        let text = "{\"id\":\"01\",\"lesson\":\"Keep it small\"}\nnot json\n";
        let error = Trajectory::from_jsonl(text).unwrap_err();
        assert_eq!(error.code(), "validation");
        assert!(error.to_string().contains("line 2"), "{error}");
    }

    #[test]
    fn slugs_are_lowercase_bounded_and_dash_separated() {
        assert_eq!(slugify("Check  THE cache!!"), "check-the-cache");
        assert_eq!(slugify("---"), "");
        assert!(slugify(&"a".repeat(200)).chars().count() <= MAX_SLUG_CHARS);
    }
}
