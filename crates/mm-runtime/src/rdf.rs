//! The `/self` mirror: the self-model, debt, GC, load and revision records as RDF.
//!
//! Three decisions shape this module:
//!
//! * **Turtle is rendered here, and only here.** The predicates are the ones
//!   `ontology/self_model.ttl` declares; a writer that invented a predicate would produce
//!   a graph `graph validate --graph self` rejects, which is the intended failure mode.
//! * **Literals are typed explicitly.** The shapes require `xsd:double`, `xsd:integer` and
//!   `xsd:boolean`, and a bare `0.5` in Turtle is an `xsd:decimal`. The typed form is
//!   written rather than relied on.
//! * **The merge graph is *not* the ledger.** `/self` holds what the being measured and
//!   did about itself; the developmental ledger is the event log and the append-only
//!   tables. A mirror statement here is a projection, which is why the GC guard is
//!   enforced on the collected *subject*, not on this graph.
//!
//! Nothing here writes: [`quads_for`] renders, and the callers insert through
//! `mm_store_graph`.

use std::collections::BTreeSet;

use mm_core::Ulid;

use crate::debt::DebtFinding;
use crate::gc::GcAction;
use crate::module_loader::LoadReceipt;
use crate::self_model::SelfModelReport;

/// The named graph these records live in, matching `mm_core::iri::NAMED_GRAPHS`.
pub const SELF_GRAPH: &str = "self";

/// The `mm:` prefix, spelled the way the ontology file spells it.
pub const MM: &str = "https://metamind.dev/ontology#";
/// The `xsd:` prefix.
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
/// The `rdf:` prefix.
pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// The instance-data prefix, so a caller that has an IRI can get back to its ULID
/// without hardcoding the namespace a second time.
pub fn data_iri_prefix() -> &'static str {
    "https://metamind.dev/data/"
}

/// The instance IRI of a record.
pub fn data_iri(id: &Ulid) -> String {
    format!("{}{}", data_iri_prefix(), mm_core::ulid_string(id))
}

/// A Turtle string literal.
pub fn string_literal(text: &str) -> String {
    format!("\"{}\"", escape(text))
}

/// A Turtle `xsd:string` literal, spelled out because a shape may ask for it.
pub fn typed_string(text: &str) -> String {
    format!("\"{}\"^^<{XSD}string>", escape(text))
}

/// A Turtle `xsd:double` literal.
pub fn double(value: f64) -> String {
    if value.is_finite() {
        format!("\"{value:e}\"^^<{XSD}double>")
    } else {
        format!("\"0.0\"^^<{XSD}double>")
    }
}

/// A Turtle `xsd:integer` literal.
pub fn integer(value: i64) -> String {
    format!("\"{value}\"^^<{XSD}integer>")
}

/// A Turtle `xsd:boolean` literal.
pub fn boolean(value: bool) -> String {
    if value {
        format!("\"true\"^^<{XSD}boolean>")
    } else {
        format!("\"false\"^^<{XSD}boolean>")
    }
}

/// Escape a literal's text. Backslash before quote, so an embedded quote does not
/// terminate the literal.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// One rendered triple, so a caller can hash or compare it without re-parsing Turtle.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Quad {
    /// The subject IRI.
    pub subject: String,
    /// The predicate IRI.
    pub predicate: String,
    /// The object, already rendered (an `<iri>` or a literal).
    pub object: String,
}

impl Quad {
    /// The predicate's local name, for a caller that wants to read a quad's meaning.
    pub fn predicate_local(&self) -> &str {
        self.predicate
            .rsplit_once('#')
            .map_or(&self.predicate, |(_, l)| l)
    }
}

/// Render quads as a Turtle document.
///
/// The `a` shorthand is used for `rdf:type`, because every shape's `sh:targetClass` reads
/// the type and the shorthand is what a reader expects; predicates are emitted in a fixed
/// order so the document is a function of its quads.
pub fn turtle(quads: &[Quad]) -> String {
    let mut out = String::new();
    let mut by_subject: std::collections::BTreeMap<&str, Vec<&Quad>> = Default::default();
    for quad in quads {
        by_subject
            .entry(quad.subject.as_str())
            .or_default()
            .push(quad);
    }
    for (subject, group) in by_subject {
        for quad in group {
            let predicate = if quad.predicate == format!("{RDF}type") {
                "a".to_string()
            } else {
                format!("<{}>", quad.predicate)
            };
            out.push_str(&format!("<{subject}> {predicate} {} .\n", quad.object));
        }
    }
    out
}

/// A deterministic hash of a quad set, independent of order.
pub fn quads_hash(quads: &[Quad]) -> String {
    let sorted: BTreeSet<String> = quads
        .iter()
        .map(|quad| format!("{} {} {}", quad.subject, quad.predicate, quad.object))
        .collect();
    mm_core::content_hash(sorted.into_iter().collect::<Vec<_>>().join("\n").as_bytes())
}

/// The quads of one self-model report: three divergences, exactly.
pub fn self_model_quads(report: &SelfModelReport) -> Vec<Quad> {
    let mut quads = Vec::new();
    let subject = data_iri(&report.id);
    push_type(&mut quads, &subject, "SelfModelReport");
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}inRun"),
        object: format!("<{}>", data_iri(&report.run_id)),
    });
    for divergence in &report.dims {
        let node = format!("{}#{}", subject, divergence.dimension);
        quads.push(Quad {
            subject: subject.clone(),
            predicate: format!("{MM}hasDivergence"),
            object: format!("<{node}>"),
        });
        quads.push(Quad {
            subject: node.clone(),
            predicate: format!("{RDF}type"),
            object: format!("<{MM}Divergence>"),
        });
        quads.push(Quad {
            subject: node.clone(),
            predicate: format!("{MM}divergenceDimension"),
            object: typed_string(&divergence.dimension),
        });
        quads.push(Quad {
            subject: node,
            predicate: format!("{MM}divergenceValue"),
            object: double(f64::from(divergence.value)),
        });
    }
    quads
}

/// The quads of one debt finding.
pub fn debt_quads(finding: &DebtFinding) -> Vec<Quad> {
    let mut quads = Vec::new();
    let subject = data_iri(&finding.id);
    push_type(&mut quads, &subject, "DebtFinding");
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}subjectOf"),
        object: format!("<{}>", finding.subject),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}debtKind"),
        object: typed_string(finding.kind.as_str()),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}severity"),
        object: double(f64::from(finding.severity)),
    });
    for evidence in &finding.evidence {
        quads.push(Quad {
            subject: subject.clone(),
            predicate: format!("{MM}debtEvidence"),
            object: typed_string(evidence),
        });
    }
    if let Some(run) = &finding.run {
        quads.push(Quad {
            subject: subject.clone(),
            predicate: format!("{MM}inRun"),
            object: format!("<{}>", data_iri(run)),
        });
    }
    quads
}

/// The quads of one GC action, including the refusals.
///
/// A refused action is still a node. The phase's rule is that a refusal is *recorded*, so
/// the graph has to hold the action that was not applied; a graph that only held applied
/// actions would make "nothing touched the ledger" an unfalsifiable claim.
pub fn gc_quads(action: &GcAction) -> Vec<Quad> {
    let mut quads = Vec::new();
    let subject = data_iri(&action.id);
    push_type(&mut quads, &subject, "GcAction");
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}subjectOf"),
        object: format!("<{}>", action.subject),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}gcKind"),
        object: typed_string(action.kind.as_str()),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}reversible"),
        object: boolean(action.reversible),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}protectsLedger"),
        object: boolean(action.protects_ledger),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}ledgerImpact"),
        object: typed_string(action.ledger_impact.as_str()),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}addressesFinding"),
        object: format!("<{}>", data_iri(&action.finding)),
    });
    quads
}

/// The quads of one module load, with the change set it belongs to typed as an
/// `mm:ChangeSet` so a design revision's `sh:class` reference resolves.
pub fn module_load_quads(receipt: &LoadReceipt) -> Vec<Quad> {
    let mut quads = Vec::new();
    let subject = data_iri(&receipt.id);
    push_type(&mut quads, &subject, "ModuleLoad");
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}loadedModule"),
        object: format!("<{}>", receipt.uri),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}moduleVersion"),
        object: typed_string(&receipt.version),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}loadStatus"),
        object: typed_string(receipt.status.as_str()),
    });
    quads
}

/// The quads of one design revision, plus its change set's node.
///
/// The change set is emitted as a typed node here because
/// `mm:DesignRevisionShape` requires `mm:hasChangeSet` to point at an `mm:ChangeSet`, and
/// the `/self` graph is where the reference is made. Only the type is asserted: the change
/// set's own fields are `/selfeng`'s record, and duplicating them here would create a
/// second copy of the same fact.
pub fn design_revision_quads(
    id: &Ulid,
    doc_uri: &str,
    revision: i64,
    change_set: &Ulid,
    promoted: bool,
) -> Vec<Quad> {
    let mut quads = Vec::new();
    let subject = data_iri(id);
    push_type(&mut quads, &subject, "DesignRevision");
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}designDoc"),
        object: format!("<{doc_uri}>"),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}generatedRevision"),
        object: integer(revision),
    });
    quads.push(Quad {
        subject: subject.clone(),
        predicate: format!("{MM}hasChangeSet"),
        object: format!("<{}>", data_iri(change_set)),
    });
    if promoted {
        let cs = data_iri(change_set);
        push_type(&mut quads, &cs, "ChangeSet");
    }
    quads
}

/// Add an `rdf:type` triple, unless it is already there.
fn push_type(quads: &mut Vec<Quad>, subject: &str, class: &str) {
    let quad = Quad {
        subject: subject.to_string(),
        predicate: format!("{RDF}type"),
        object: format!("<{MM}{class}>"),
    };
    if !quads.contains(&quad) {
        quads.push(quad);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debt::DebtKind;
    use crate::gc::{GcActionKind, LedgerImpact};

    fn ulid(seed: u64) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, u128::from(seed))
    }

    fn report() -> SelfModelReport {
        SelfModelReport {
            id: ulid(1),
            run_id: ulid(2),
            actual_model: 0.3,
            actual_ideal: 0.2,
            model_ideal: 0.5,
            dims: vec![
                crate::Divergence {
                    dimension: "actual_model".to_string(),
                    value: 0.3,
                },
                crate::Divergence {
                    dimension: "actual_ideal".to_string(),
                    value: 0.2,
                },
                crate::Divergence {
                    dimension: "model_ideal".to_string(),
                    value: 0.5,
                },
            ],
        }
    }

    #[test]
    fn a_report_renders_exactly_three_divergences() {
        let quads = self_model_quads(&report());
        let count = quads
            .iter()
            .filter(|quad| quad.predicate == format!("{MM}hasDivergence"))
            .count();
        assert_eq!(count, 3, "the shape requires exactly three");
        let turtle = turtle(&quads);
        assert!(turtle.contains("<https://metamind.dev/ontology#SelfModelReport>"));
        assert!(turtle.contains("a <https://metamind.dev/ontology#Divergence>"));
        assert!(turtle.contains("divergenceValue"));
        assert!(quads_hash(&quads).len() == 64);
    }

    #[test]
    fn literals_are_typed_the_way_the_shapes_require() {
        // The datatype is written as an absolute IRI — `turtle` may declare an `xsd:`
        // prefix for readability, but a literal must not depend on one being declared.
        assert!(double(0.3).contains(&format!("<{XSD}double>")));
        assert!(integer(3).contains(&format!("<{XSD}integer>")));
        assert!(boolean(true).contains(&format!("<{XSD}boolean>")));
        assert!(typed_string("x").contains(&format!("<{XSD}string>")));
        // A non-finite number is still a well-formed double, because a shape that asks
        // for one must not be satisfied by an unparseable literal.
        assert!(double(f64::NAN).contains("0.0"));
    }

    #[test]
    fn escaping_keeps_a_quote_inside_its_literal() {
        let text = string_literal("a \"quoted\" \\ word");
        assert!(text.starts_with('"') && text.ends_with('"'));
        assert!(text.contains("\\\"quoted\\\""));
        assert!(text.contains("\\\\"));
    }

    #[test]
    fn a_gc_action_renders_both_flags() {
        let action = GcAction {
            id: ulid(3),
            finding: ulid(4),
            kind: GcActionKind::Data,
            subject: "https://metamind.dev/data/01h0000000000000000000db02".to_string(),
            reversible: true,
            ledger_impact: LedgerImpact::Metadata,
            protects_ledger: true,
            applied: false,
        };
        let turtle = turtle(&gc_quads(&action));
        assert!(turtle.contains("protectsLedger"));
        assert!(turtle.contains("reversible"));
        assert!(turtle.contains("ledgerImpact"));
    }

    #[test]
    fn a_debt_finding_carries_at_least_one_evidence_literal() {
        let finding = DebtFinding {
            id: ulid(5),
            run: None,
            kind: DebtKind::StaleMemory,
            subject: "https://metamind.dev/data/01h0000000000000000000db02".to_string(),
            severity: 0.6,
            evidence: vec!["memories: one row is protected and stale".to_string()],
            protects_ledger: true,
            detail: "a developmental memory".to_string(),
        };
        let quads = debt_quads(&finding);
        assert_eq!(
            quads
                .iter()
                .filter(|quad| quad.predicate == format!("{MM}debtEvidence"))
                .count(),
            1
        );
        assert!(turtle(&quads).contains("DebtFinding"));
    }

    #[test]
    fn a_design_revision_types_its_change_set() {
        let quads = design_revision_quads(
            &ulid(6),
            "https://metamind.dev/design/qualification/goal_attainment.md",
            1,
            &ulid(7),
            true,
        );
        let promoted = turtle(&quads);
        assert!(promoted.contains("DesignRevision"));
        assert!(promoted.contains(&format!("<{MM}ChangeSet>")));
        assert!(promoted.contains("generatedRevision"));
        // An unpromoted revision does not *type* a change set it never had. The
        // `mm:hasChangeSet` edge remains, because the change set exists either way —
        // what a rejection withholds is the claim that it produced the revision.
        let unpromoted = design_revision_quads(&ulid(6), "https://x/y", 1, &ulid(7), false);
        assert!(!turtle(&unpromoted).contains(&format!("<{MM}ChangeSet>")));
    }
}
