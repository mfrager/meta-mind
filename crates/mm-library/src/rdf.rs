//! The Turtle codec: every entry writes itself out in one canonical form.
//!
//! One emitter per kind, all built on the same helpers, because the entry shapes
//! differ and the *writing* should not. Two rules hold throughout:
//!
//! * **Explicit datatypes.** Doubles are written `"0.8"^^xsd:double` and integers
//!   `"1"^^xsd:integer`, never bare. A bare `0.8` is an `xsd:decimal` in Turtle, and
//!   the shapes check the datatype — a number that looks right and validates wrong
//!   is the worst kind of wrong.
//! * **Every entry is typed twice**: `mm:LibraryEntry` and its own kind, so the
//!   entry-level shape and the kind shape both apply.
//!
//! The document is then canonicalized by `rdf-codec` (order-independent N-Triples)
//! and hashed with sha256, which is the `content_hash` the duplicate check and the
//! round-trip test both use.

use mm_core::NamedNode;

use crate::case_db::Case;
use crate::entry::Declarative;
use crate::frame::{Frame, FrameInstance};
use crate::policy::Policy;
use crate::skill::{Skill, Workflow};

/// The named graph every entry is written to.
pub const LIBRARY_GRAPH: &str = "library";

/// The prefixes every emitted document declares.
pub const PREFIXES: &str = concat!(
    "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n",
    "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n",
    "@prefix mm: <https://metamind.dev/ontology#> .\n",
    "@prefix prov: <http://www.w3.org/ns/prov#> .\n",
);

const MM: &str = "https://metamind.dev/ontology#";

/// An IRI as Turtle.
fn iri(iri: &str) -> String {
    format!("<{iri}>")
}

/// An `mm:` term as Turtle.
fn mm(local: &str) -> String {
    format!("mm:{local}")
}

/// A string literal, escaped.
fn text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// An `xsd:double` literal.
fn double(value: f32) -> String {
    format!("\"{}\"^^xsd:double", value)
}

/// An `xsd:integer` literal.
fn integer(value: u32) -> String {
    format!("\"{value}\"^^xsd:integer")
}

/// One statement: `subject predicate object .`
fn statement(subject: &str, predicate: &str, object: &str) -> String {
    format!("{subject} {predicate} {object} .\n")
}

/// A named node as Turtle.
fn node(node: &NamedNode) -> String {
    iri(node.as_str())
}

/// The typed-subject line every entry opens with.
fn typed(entry_iri: &str, class: &str) -> String {
    statement(
        &iri(entry_iri),
        "a",
        &format!("mm:LibraryEntry, {}", mm(class)),
    )
}

/// Emit a declarative entry.
pub fn declarative_turtle(entry: &Declarative) -> crate::error::Result<String> {
    crate::entry::LibraryEntry::validate(entry)?;
    let mut out = String::from(PREFIXES);
    let subject = iri(entry.iri.as_str());
    out.push_str(&typed(entry.iri.as_str(), entry.kind.rdf_class()));
    out.push_str(&statement(&subject, &mm("title"), &text(&entry.title)));
    out.push_str(&statement(&subject, &mm("purpose"), &text(&entry.purpose)));
    out.push_str(&statement(
        &subject,
        &mm("version"),
        &integer(entry.version),
    ));
    out.push_str(&statement(
        &subject,
        &mm("activationScore"),
        &double(entry.activation_score),
    ));
    for domain in &entry.domain {
        out.push_str(&statement(&subject, &mm("domain"), &text(domain)));
    }
    for condition in &entry.applicable_when {
        out.push_str(&statement(
            &subject,
            &mm("applicableWhen"),
            &text(condition.as_str()),
        ));
    }
    for condition in &entry.inapplicable_when {
        out.push_str(&statement(
            &subject,
            &mm("inapplicableWhen"),
            &text(condition.as_str()),
        ));
    }
    for condition in &entry.contraindications {
        out.push_str(&statement(
            &subject,
            &mm("contraindication"),
            &text(condition.as_str()),
        ));
    }
    for evidence in &entry.evidence {
        out.push_str(&statement(&subject, &mm("evidence"), &node(evidence)));
    }
    for example in &entry.examples {
        out.push_str(&statement(&subject, &mm("example"), &node(example)));
    }
    for counterexample in &entry.counterexamples {
        out.push_str(&statement(
            &subject,
            &mm("counterexample"),
            &node(counterexample),
        ));
    }
    for test in &entry.tested_by {
        out.push_str(&statement(&subject, &mm("testedBy"), &node(test)));
    }
    for improved in &entry.improves {
        out.push_str(&statement(&subject, &mm("improves"), &node(improved)));
    }
    for replaced in &entry.replaces {
        out.push_str(&statement(&subject, &mm("replaces"), &node(replaced)));
    }
    if let Some(rule) = &entry.corrective_rule {
        out.push_str(&statement(&subject, &mm("correctiveRule"), &text(rule)));
    }
    if let Some(mode) = &entry.failure_mode {
        out.push_str(&statement(&subject, &mm("failureMode"), &text(mode)));
    }
    if let Some(confidence) = entry.confidence {
        out.push_str(&statement(&subject, &mm("confidence"), &double(confidence)));
    }
    for (domain, score) in &entry.domain_fitness {
        out.push_str(&statement(
            &subject,
            &mm("domainFitness"),
            &text(&format!("{domain}={score}")),
        ));
    }
    for trajectory in &entry.derived_from_trajectory {
        out.push_str(&statement(
            &subject,
            &mm("derivedFromTrajectory"),
            &text(trajectory),
        ));
    }
    Ok(out)
}

/// Emit a skill.
pub fn skill_turtle(skill: &Skill) -> crate::error::Result<String> {
    crate::entry::LibraryEntry::validate(skill)?;
    let mut out = String::from(PREFIXES);
    let subject = iri(skill.iri.as_str());
    out.push_str(&typed(skill.iri.as_str(), "Skill"));
    out.push_str(&statement(&subject, &mm("title"), &text(&skill.name)));
    out.push_str(&statement(
        &subject,
        &mm("purpose"),
        &text(&skill.description),
    ));
    out.push_str(&statement(
        &subject,
        &mm("version"),
        &integer(skill.version),
    ));
    out.push_str(&statement(&subject, &mm("implRef"), &text(&skill.impl_ref)));
    out.push_str(&statement(
        &subject,
        &mm("entrypoint"),
        &text(&skill.entrypoint),
    ));
    out.push_str(&statement(
        &subject,
        &mm("signature"),
        &text(&skill.signature),
    ));
    out.push_str(&statement(
        &subject,
        &mm("verification"),
        &text(skill.verification.as_str()),
    ));
    out.push_str(&statement(
        &subject,
        &mm("testedBy"),
        &iri(&skill.tests_ref),
    ));
    Ok(out)
}

/// Emit a workflow.
pub fn workflow_turtle(workflow: &Workflow) -> crate::error::Result<String> {
    crate::entry::LibraryEntry::validate(workflow)?;
    let mut out = String::from(PREFIXES);
    let subject = iri(workflow.iri.as_str());
    out.push_str(&typed(workflow.iri.as_str(), "Workflow"));
    out.push_str(&statement(&subject, &mm("title"), &text(&workflow.name)));
    out.push_str(&statement(
        &subject,
        &mm("purpose"),
        &text("A procedure induced from a trajectory."),
    ));
    out.push_str(&statement(
        &subject,
        &mm("version"),
        &integer(workflow.version),
    ));
    for step in &workflow.steps {
        out.push_str(&statement(&subject, &mm("scriptStep"), &text(step)));
    }
    out.push_str(&statement(
        &subject,
        &mm("derivedFromTrajectory"),
        &text(&workflow.induced_from),
    ));
    Ok(out)
}

/// Emit a case.
pub fn case_turtle(case: &Case) -> crate::error::Result<String> {
    crate::entry::LibraryEntry::validate(case)?;
    let mut out = String::from(PREFIXES);
    let subject = iri(case.iri.as_str());
    out.push_str(&typed(case.iri.as_str(), "Case"));
    out.push_str(&statement(
        &subject,
        &mm("title"),
        &text(&format!("{} case", case.kind)),
    ));
    out.push_str(&statement(
        &subject,
        &mm("purpose"),
        &text("A remembered instance: problem, solution, outcome."),
    ));
    out.push_str(&statement(&subject, &mm("version"), &integer(case.version)));
    out.push_str(&statement(
        &subject,
        &mm("problem"),
        &text(&case.problem_ttl),
    ));
    out.push_str(&statement(
        &subject,
        &mm("solution"),
        &text(&case.solution_ttl),
    ));
    out.push_str(&statement(
        &subject,
        &mm("outcome"),
        &text(&case.outcome_ttl),
    ));
    out.push_str(&statement(
        &subject,
        &mm("outcomeQuality"),
        &double(case.outcome_quality),
    ));
    out.push_str(&statement(
        &subject,
        &mm("transferability"),
        &double(case.transferability),
    ));
    out.push_str(&statement(
        &subject,
        &mm("evidenceQuality"),
        &double(case.evidence_quality),
    ));
    Ok(out)
}

/// Emit a frame.
pub fn frame_turtle(frame: &Frame) -> crate::error::Result<String> {
    crate::entry::LibraryEntry::validate(frame)?;
    let mut out = String::from(PREFIXES);
    let subject = iri(frame.iri.as_str());
    out.push_str(&typed(frame.iri.as_str(), "Frame"));
    out.push_str(&statement(&subject, &mm("title"), &text(&frame.title)));
    out.push_str(&statement(&subject, &mm("purpose"), &text(&frame.purpose)));
    out.push_str(&statement(
        &subject,
        &mm("version"),
        &integer(frame.version),
    ));
    for domain in &frame.domain {
        out.push_str(&statement(&subject, &mm("domain"), &text(domain)));
    }
    for (role, description) in &frame.roles {
        out.push_str(&statement(&subject, &mm("role"), &text(role)));
        out.push_str(&statement(&subject, &mm("affordance"), &text(description)));
    }
    for slot in &frame.slots {
        out.push_str(&statement(&subject, &mm("frameSlot"), &text(slot)));
    }
    for action in &frame.typical_actions {
        out.push_str(&statement(&subject, &mm("typicalAction"), &text(action)));
    }
    for mode in &frame.failure_modes {
        out.push_str(&statement(&subject, &mm("failureMode"), &text(mode)));
    }
    for script in &frame.scripts {
        for step in script {
            out.push_str(&statement(&subject, &mm("scriptStep"), &text(step)));
        }
    }
    if let Some(parent) = &frame.parent {
        out.push_str(&statement(&subject, &mm("parentFrame"), &node(parent)));
    }
    Ok(out)
}

/// Emit a frame instance.
pub fn frame_instance_turtle(instance: &FrameInstance) -> crate::error::Result<String> {
    crate::entry::LibraryEntry::validate(instance)?;
    let mut out = String::from(PREFIXES);
    let subject = iri(&mm_core::iri::data(&instance.ulid).into_string());
    out.push_str(&typed(
        &mm_core::iri::data(&instance.ulid).into_string(),
        "FrameInstance",
    ));
    out.push_str(&statement(&subject, &mm("title"), &text(&instance.title)));
    out.push_str(&statement(
        &subject,
        &mm("purpose"),
        &text(&instance.purpose),
    ));
    out.push_str(&statement(&subject, &mm("version"), &integer(1)));
    for parent in &instance.parents {
        out.push_str(&statement(&subject, &mm("parentFrame"), &node(parent)));
    }
    for (slot, value) in &instance.slots {
        match value {
            crate::frame::SlotValue::Known(text_value) => {
                out.push_str(&statement(&subject, &mm("frameSlot"), &text(slot)));
                out.push_str(&statement(
                    &subject,
                    &mm("outcome"),
                    &text(&format!("{slot}={text_value}")),
                ));
            }
            crate::frame::SlotValue::Inferred(text_value) => {
                out.push_str(&statement(&subject, &mm("frameSlot"), &text(slot)));
                out.push_str(&statement(
                    &subject,
                    &mm("evidence"),
                    &text(&format!("{slot}~{text_value}")),
                ));
            }
            crate::frame::SlotValue::Unknown => {
                out.push_str(&statement(&subject, &mm("frameSlot"), &text(slot)));
            }
        }
    }
    Ok(out)
}

/// Emit a policy version.
pub fn policy_turtle(policy: &Policy) -> crate::error::Result<String> {
    crate::entry::LibraryEntry::validate(policy)?;
    let mut out = String::from(PREFIXES);
    let subject = iri(policy.iri.as_str());
    out.push_str(&typed(policy.iri.as_str(), "Policy"));
    out.push_str(&statement(&subject, &mm("title"), &text(&policy.title)));
    out.push_str(&statement(&subject, &mm("purpose"), &text(&policy.purpose)));
    out.push_str(&statement(
        &subject,
        &mm("version"),
        &integer(policy.version),
    ));
    out.push_str(&statement(
        &subject,
        &mm("scope"),
        &text(&policy.scope.as_str()),
    ));
    out.push_str(&statement(
        &subject,
        &mm("activationCondition"),
        &text(policy.activation.as_str()),
    ));
    out.push_str(&statement(
        &subject,
        &mm("behavior"),
        &text(policy.behavior.as_str()),
    ));
    out.push_str(&statement(
        &subject,
        &mm("confidence"),
        &double(policy.confidence),
    ));
    out.push_str(&statement(
        &subject,
        &mm("fitness"),
        &double(policy.fitness.mean_utility),
    ));
    for evidence in &policy.evidence {
        out.push_str(&statement(&subject, &mm("evidence"), &node(evidence)));
    }
    if let Some(parent) = &policy.parent {
        out.push_str(&statement(&subject, &mm("parentPolicy"), &node(parent)));
    }
    if policy.generation > 0 {
        out.push_str(&statement(
            &subject,
            &mm("generation"),
            &integer(policy.generation),
        ));
    }
    Ok(out)
}

/// The canonical content hash of a document.
///
/// sha256 over the canonical N-Triples rendering, so two documents that differ
/// only in statement order (or in which prefix names the same IRI) hash the same
/// and the duplicate check means what it says.
pub fn content_hash(turtle: &str) -> crate::error::Result<String> {
    mm_store_graph::canonical_hash_of_turtle(turtle)
        .map_err(|e| crate::error::LibraryError::Codec(e.to_string()))
}

/// Re-serialize a document through the canonical writer.
pub fn normalize(turtle: &str) -> crate::error::Result<String> {
    mm_store_graph::normalize_turtle(turtle)
        .map_err(|e| crate::error::LibraryError::Codec(e.to_string()))
}

/// The `mm:` IRI of a local name, for callers that need one.
pub fn mm_iri(local: &str) -> String {
    format!("{MM}{local}")
}

/// The declarative entries a document contains, in subject order.
///
/// The decode half of the codec: the same fields [`declarative_turtle`] writes,
/// read back. It exists because two callers need an *entry* rather than a row —
/// the applicability ranker, which needs `applicable_when` and `domain_fitness`,
/// and the round-trip test, which asserts `hash(decode(encode(e))) == hash(e)`.
/// A field the decoder cannot express is left at its default rather than invented:
/// the shapes have already refused anything the encoder would not have written.
pub fn declaratives_from_turtle(turtle: &str) -> crate::error::Result<Vec<Declarative>> {
    let mut out = Vec::new();
    for (subject, lines) in group_lines(turtle)? {
        if let Some(entry) = declarative_of_lines(&subject, &lines)? {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Group a document's statements by subject, in subject order.
///
/// Each line is rendered `subject predicate object .`, which is exactly the shape
/// the readers below parse; the rendering goes through the term's `Display`, so an
/// IRI stays an IRI and a literal keeps its quotes and datatype.
pub fn group_lines(turtle: &str) -> crate::error::Result<Vec<(String, Vec<String>)>> {
    let graph = rdf_codec::io::parse_turtle(turtle)
        .map_err(|e| crate::error::LibraryError::Codec(format!("entry decode: {e}")))?;
    let mut groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for triple in graph.iter() {
        groups
            .entry(triple.subject.to_string())
            .or_default()
            .push(format!(
                "{} {} {} .",
                triple.subject, triple.predicate, triple.object
            ));
    }
    Ok(groups.into_iter().collect())
}

/// The first declarative entry in a document.
pub fn declarative_from_turtle(turtle: &str) -> crate::error::Result<Declarative> {
    declaratives_from_turtle(turtle)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            crate::error::LibraryError::validation(
                "entry",
                "the document holds no declarative entry",
            )
        })
}

/// Build one entry from the statements grouped under a subject.
fn declarative_of_lines(
    subject: &str,
    lines: &[String],
) -> crate::error::Result<Option<Declarative>> {
    let kind = match kind_of_lines(lines) {
        Some(kind) if kind.is_declarative() => kind,
        _ => return Ok(None),
    };
    let iri_text = subject
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string();
    let slug = crate::entry::iri::slug_of(&iri_text).unwrap_or_default();
    let version = literal_in(lines, "version")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_else(|| crate::entry::iri::version_of(&iri_text).unwrap_or(1));

    let mut entry = Declarative::new(
        kind,
        &slug,
        &literal_in(lines, "title").unwrap_or_default(),
        &literal_in(lines, "purpose").unwrap_or_default(),
    )?;
    entry.iri = crate::entry::iri::versioned(kind, &slug, version);
    entry.version = version;
    entry.domain = literals_in(lines, "domain");
    entry.applicable_when = conditions_in(lines, "applicableWhen")?;
    entry.inapplicable_when = conditions_in(lines, "inapplicableWhen")?;
    entry.contraindications = conditions_in(lines, "contraindication")?;
    entry.evidence = nodes_in(lines, "evidence");
    entry.examples = nodes_in(lines, "example");
    entry.counterexamples = nodes_in(lines, "counterexample");
    entry.tested_by = nodes_in(lines, "testedBy");
    entry.improves = nodes_in(lines, "improves");
    entry.replaces = nodes_in(lines, "replaces");
    entry.corrective_rule = literal_in(lines, "correctiveRule");
    entry.failure_mode = literal_in(lines, "failureMode");
    entry.confidence = literal_in(lines, "confidence").and_then(|value| value.parse().ok());
    entry.activation_score = literal_in(lines, "activationScore")
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.5);
    for value in literals_in(lines, "domainFitness") {
        if let Some((domain, score)) = value.split_once('=') {
            if let Ok(score) = score.parse::<f32>() {
                entry.domain_fitness.insert(domain.to_string(), score);
            }
        }
    }
    entry.derived_from_trajectory = literals_in(lines, "derivedFromTrajectory");
    Ok(Some(entry))
}

/// The library kind a subject's type list names, if it names one.
pub fn kind_of_lines(lines: &[String]) -> Option<crate::entry::EntryKind> {
    for line in lines {
        let Some((predicate, object)) = statement_parts(line) else {
            continue;
        };
        if predicate != "type" {
            continue;
        }
        let local = object.trim_start_matches('<').trim_end_matches('>');
        let local = local.rsplit('#').next().unwrap_or(local);
        if local == "LibraryEntry" {
            continue;
        }
        if let Some(kind) = crate::entry::EntryKind::parse(local) {
            return Some(kind);
        }
    }
    None
}

/// A statement as `(predicate local name, object)`.
///
/// The object is **the rest of the line**, not one whitespace-separated token: a
/// literal is allowed to contain spaces, and a reader that took the third token
/// would silently see `"Order` and give up on every multi-word value — which is
/// every title in the library.
fn statement_parts(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let mut parts = line.splitn(3, ' ');
    let _subject = parts.next()?;
    let predicate = parts.next()?;
    let rest = parts.next()?;
    let object = rest
        .trim_end()
        .strip_suffix(" .")
        .unwrap_or(rest)
        .trim_end();
    Some((local_name(predicate), object))
}

/// The fragment of a predicate or IRI, with the Turtle brackets removed.
fn local_name(term: &str) -> &str {
    let bare = term.trim_start_matches('<').trim_end_matches('>');
    bare.rsplit('#').next().unwrap_or(bare)
}

/// A literal value by local predicate name, in `"subject predicate object ."` lines.
pub fn literal_in(lines: &[String], local: &str) -> Option<String> {
    for line in lines {
        if let Some((predicate, object)) = statement_parts(line) {
            if predicate == local {
                if let Some(value) = literal_value(object) {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// Every literal value of a predicate.
pub fn literals_in(lines: &[String], local: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if let Some((predicate, object)) = statement_parts(line) {
            if predicate == local {
                if let Some(value) = literal_value(object) {
                    out.push(value);
                }
            }
        }
    }
    out
}

/// The first named node object of a predicate.
pub fn node_in(lines: &[String], local: &str) -> Option<NamedNode> {
    nodes_in(lines, local).into_iter().next()
}

/// Every named node object of a predicate.
fn nodes_in(lines: &[String], local: &str) -> Vec<NamedNode> {
    let mut out = Vec::new();
    for line in lines {
        if let Some((predicate, object)) = statement_parts(line) {
            if predicate == local && object.starts_with('<') && object.ends_with('>') {
                out.push(NamedNode::new_unchecked(
                    object.trim_start_matches('<').trim_end_matches('>'),
                ));
            }
        }
    }
    out
}

/// Every activation condition of a predicate.
fn conditions_in(
    lines: &[String],
    local: &str,
) -> crate::error::Result<Vec<mm_core::ActivationCondition>> {
    literals_in(lines, local)
        .into_iter()
        .map(|text| mm_core::ActivationCondition::new(text).map_err(Into::into))
        .collect()
}

/// The value of a literal token as Turtle writes it.
pub fn literal_value(token: &str) -> Option<String> {
    let body = token.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                other => out.push(other),
            },
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Declarative, EntryKind, LibraryEntry};

    #[test]
    fn literals_are_escaped_and_datatypes_are_explicit() {
        assert_eq!(text("a \"b\"\n"), "\"a \\\"b\\\"\\n\"");
        assert_eq!(double(0.8), "\"0.8\"^^xsd:double");
        assert_eq!(integer(2), "\"2\"^^xsd:integer");
    }

    #[test]
    fn a_declarative_entry_encodes_to_parsable_turtle_with_its_kind_and_entry_type() {
        let entry = Declarative::new(EntryKind::Technique, "decompose", "Decompose", "Split it")
            .unwrap()
            .when(crate::doctrine::when("parts are independent").unwrap())
            .backed_by(crate::entry::iri::library(EntryKind::Case, "c"))
            .illustrated_by(crate::entry::iri::library(EntryKind::Case, "c"))
            .with_confidence(0.85)
            .unwrap();
        let turtle = entry.to_turtle().unwrap();
        assert!(turtle.contains("a mm:LibraryEntry, mm:Technique"));
        assert!(turtle.contains("mm:applicableWhen \"parts are independent\""));
        assert!(turtle.contains("\"0.85\"^^xsd:double"));
        rdf_codec::io::parse_turtle(&turtle).expect("the document must parse");
        // And the hash is order-independent.
        assert_eq!(content_hash(&turtle).unwrap().len(), 64);
    }

    #[test]
    fn the_seed_policys_fields_read_back_from_its_statements() {
        let seed = std::fs::read_to_string(
            mm_core::Config::repo_root().join("ontology/seed/library_seed.ttl"),
        )
        .expect("the committed seed");
        let groups = group_lines(&seed).unwrap();
        let (_, lines) = groups
            .iter()
            .find(|(subject, _)| subject.contains("prefer_simpler_solution"))
            .expect("the seeded policy");
        assert_eq!(literal_in(lines, "scope").as_deref(), Some("global"));
        assert_eq!(
            literal_in(lines, "activationCondition").as_deref(),
            Some("two candidate solutions are both viable")
        );
        assert_eq!(
            literal_in(lines, "behavior").as_deref(),
            Some("weights:1.0+0.2+0.2")
        );
        assert_eq!(literal_in(lines, "confidence").as_deref(), Some("0.6"));
    }

    #[test]
    fn statement_order_does_not_change_the_content_hash() {
        let entry = Declarative::new(EntryKind::Principle, "p", "P", "P")
            .unwrap()
            .backed_by(crate::entry::iri::library(EntryKind::Case, "c"));
        let turtle = entry.to_turtle().unwrap();
        let reversed = normalize(&turtle).unwrap();
        assert_eq!(
            content_hash(&turtle).unwrap(),
            content_hash(&reversed).unwrap()
        );
    }
}
