//! The `/tools` mirror: tools, actions and observations as RDF.
//!
//! Phase 10's records reach the graph through one named graph, `tools`, added to
//! [`mm_core::iri::NAMED_GRAPHS`] by migration `0010_tools.sql`. It is *not*
//! `/provenance`: Phase 6's epistemic mirror clears and rewrites `/provenance` from its
//! snapshot on every mutation, so an action record written there would be erased by the
//! next claim promotion. The two graphs have different lifetimes, so they are two
//! graphs.
//!
//! The observation's *claim* still reaches `/world`, through the epistemic engine's
//! barrier — that is what makes "the environment decides what is true" true. What lives
//! here is the action-level record: which tool ran, what it was asked for, what the
//! capability was, what it produced and which evidence id stands behind it.
//!
//! Everything is rendered through [`quads_hash`], which hashes the *sorted* rendering
//! of a quad set. Two runs of the same action therefore compare equal whether or not
//! they inserted their quads in the same order, which is what a replay check needs.

use oxrdf::{GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use serde_json::Value;

use crate::action::{ActionResult, ActionSpec};
use crate::observation::Observation;
use crate::permissions::Decision;
use crate::sandbox::SandboxTier;
use crate::spec::ToolSpec;
use mm_core::{iri, Timestamp, Ulid};

/// The named graph Phase 10 writes.
pub const TOOLS_GRAPH: &str = "tools";

/// The PROV-O namespace, for the activity that produced an observation.
const PROV: &str = "http://www.w3.org/ns/prov#";

fn named(text: &str) -> NamedNode {
    NamedNode::new_unchecked(text)
}

fn node(text: &str) -> NamedOrBlankNode {
    NamedOrBlankNode::NamedNode(named(text))
}

fn graph_name(name: &str) -> GraphName {
    GraphName::NamedNode(named(&iri::graph(name)))
}

fn literal(text: impl Into<String>) -> Term {
    Term::Literal(Literal::new_simple_literal(text.into()))
}

fn term(iri_text: &str) -> Term {
    Term::NamedNode(named(iri_text))
}

fn quad(subject: &NamedOrBlankNode, predicate: &str, object: Term, graph: &str) -> Quad {
    Quad::new(subject.clone(), named(predicate), object, graph_name(graph))
}

/// The RDF class every tool is typed as.
fn class_tool() -> String {
    iri::mm("Tool").into_string()
}

/// The RDF class an action is typed as.
fn class_action() -> String {
    iri::mm("Action").into_string()
}

/// The node IRI of an action.
pub fn action_node(id: &Ulid) -> String {
    iri::data(id).into_string()
}

/// The quads one tool spec contributes.
pub fn tool_spec_quads(spec: &ToolSpec) -> Vec<Quad> {
    let subject = node(&iri::module(&spec.name.0).into_string());
    let mut quads = vec![
        quad(
            &subject,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            term(&class_tool()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("toolName").into_string(),
            literal(spec.name.0.clone()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("version").into_string(),
            literal(spec.version.clone()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("reversibility").into_string(),
            literal(spec.reversibility.as_str()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("sideEffectClass").into_string(),
            literal(spec.side_effects.as_str()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("sandboxTier").into_string(),
            literal(spec.sandbox_tier.as_str()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("moduleUri").into_string(),
            literal(spec.module_uri.clone()),
            TOOLS_GRAPH,
        ),
    ];
    for capability in &spec.permissions {
        quads.push(quad(
            &subject,
            &iri::mm("hasCapability").into_string(),
            literal(capability.canonical()),
            TOOLS_GRAPH,
        ));
    }
    dedup(quads)
}

/// The quads one action contributes, with its decision and its result.
pub fn action_quads(
    action_id: &Ulid,
    action: &ActionSpec,
    result: &ActionResult,
    decision: &Decision,
    tier: SandboxTier,
    at: Timestamp,
) -> Vec<Quad> {
    let subject = node(&action_node(action_id));
    let mut quads = vec![
        quad(
            &subject,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            term(&class_action()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            term(&class_action()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("toolName").into_string(),
            literal(action.tool.0.clone()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("argsHash").into_string(),
            literal(action.args_hash()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("performedBy").into_string(),
            literal(action.principal.as_str()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("permissionDecision").into_string(),
            literal(decision.as_str()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("sandboxTier").into_string(),
            literal(tier.as_str()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("status").into_string(),
            literal(result.status.as_str()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("resultHash").into_string(),
            literal(result.result_hash()),
            TOOLS_GRAPH,
        ),
    ];
    quads.push(quad(
        &subject,
        &format!("{PROV}wasStartedAtTime"),
        literal(at.to_rfc3339()),
        TOOLS_GRAPH,
    ));
    if let Some(reason) = &result.reason {
        quads.push(quad(
            &subject,
            &iri::mm("refusalReason").into_string(),
            literal(reason.clone()),
            TOOLS_GRAPH,
        ));
    }
    if !decision.is_allow() {
        if let Some(reason) = decision.deny_reason() {
            quads.push(quad(
                &subject,
                &format!("{PROV}wasInvalidatedBy"),
                literal(reason.code()),
                TOOLS_GRAPH,
            ));
        }
    }
    dedup(quads)
}

/// The quads one observation contributes.
pub fn observation_quads(observation: &Observation) -> Vec<Quad> {
    let subject = node(&observation.node_iri);
    dedup(vec![
        quad(
            &subject,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            term(&iri::mm("ActionObservation").into_string()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("observedIn").into_string(),
            Term::NamedNode(named(&iri::graph(observation.graph()))),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("resultedIn").into_string(),
            term(&action_node(&observation.action_id)),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("producedEvidence").into_string(),
            term(&iri::data(&observation.evidence_id).into_string()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("observedBy").into_string(),
            Term::NamedNode(named(&iri::mm("KernelToolExecutor").into_string())),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("source").into_string(),
            literal(observation.source.canonical()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("payloadHash").into_string(),
            literal(observation.payload_hash.clone()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("summary").into_string(),
            literal(observation.payload.summary.clone()),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &format!("{PROV}wasGeneratedBy"),
            Term::NamedNode(named(&iri::mm("ToolExecution").into_string())),
            TOOLS_GRAPH,
        ),
        quad(
            &subject,
            &iri::mm("observedAt").into_string(),
            literal(observation.observed_at.to_rfc3339()),
            TOOLS_GRAPH,
        ),
    ])
}

/// A canonical rendering of one quad, for hashing and comparison.
pub fn render_quad(quad: &Quad) -> String {
    let subject = match &quad.subject {
        NamedOrBlankNode::NamedNode(n) => format!("<{}>", n.as_str()),
        NamedOrBlankNode::BlankNode(b) => format!("_:{b}"),
    };
    let object = match &quad.object {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::BlankNode(b) => format!("_:{b}"),
        Term::Literal(l) => format!("\"{}\"", l.value()),
    };
    let graph = match &quad.graph_name {
        GraphName::NamedNode(n) => format!("<{}>", n.as_str()),
        GraphName::BlankNode(b) => format!("_:{b}"),
        GraphName::DefaultGraph => "default".to_string(),
    };
    format!("{subject} <{}> {object} {graph}", quad.predicate.as_str())
}

/// A hash of a quad set that does not depend on insertion order.
pub fn quads_hash(quads: &[Quad]) -> String {
    let mut rendered: Vec<String> = quads.iter().map(render_quad).collect();
    rendered.sort();
    rendered.dedup();
    mm_core::content_hash(rendered.join("\n").as_bytes())
}

/// Remove duplicate quads, keeping the first occurrence of each.
fn dedup(quads: Vec<Quad>) -> Vec<Quad> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Quad> = Vec::new();
    for quad in quads {
        let key = render_quad(&quad);
        if !seen.contains(&key) {
            seen.push(key);
            out.push(quad);
        }
    }
    out
}

/// Insert a quad set into `/tools`.
///
/// Insertion is idempotent in RDF, so replaying the same action does not duplicate its
/// record; the graph is not cleared first, because actions accumulate and a clear would
/// erase every earlier action's record.
pub async fn mirror(
    handle: &mm_store_graph::GraphHandle,
    quads: Vec<Quad>,
) -> Result<usize, mm_core::MmError> {
    let count = quads.len();
    for quad in quads {
        handle.insert(quad).await?;
    }
    Ok(count)
}

/// The tool spec as the JSON `mm-cli tool describe` prints it.
pub fn spec_json(spec: &ToolSpec) -> Value {
    serde_json::to_value(spec).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionStatus;
    use crate::permissions::{PermissionAction, PermissionReq, Principal};
    use crate::spec::ToolName;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: ToolName::new("fs.read").unwrap(),
            version: "0.1.0".into(),
            description: "read a file".into(),
            input_schema: crate::spec::JsonSchema::object_with_strings(&["path"]),
            output_schema: crate::spec::JsonSchema::any_object(),
            permissions: vec![PermissionReq::new(
                "fs:read:data/sandbox/**",
                PermissionAction::Read,
            )],
            reversibility: crate::spec::Reversibility::Reversible,
            side_effects: crate::spec::SideEffectClass::None,
            annotations: crate::spec::ToolAnnotations::pure(),
            module_uri: "https://metamind.dev/code/module/tools/fs-read".into(),
            sandbox_tier: SandboxTier::WasmCaps,
        }
    }

    #[test]
    fn a_spec_mirrors_its_contract() {
        let quads = tool_spec_quads(&spec());
        let rendered = quads.iter().map(render_quad).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("mm-tool") || rendered.contains("Tool"));
        assert!(rendered.contains("reversibility"), "{rendered}");
        assert!(rendered.contains("WasmCaps"), "{rendered}");
        assert!(rendered.contains("fs:read:data/sandbox/**"), "{rendered}");
        assert!(
            rendered.contains(&iri::graph(TOOLS_GRAPH)),
            "quads go to /tools"
        );
        assert_eq!(
            tool_spec_quads(&spec()),
            quads,
            "mirroring is deterministic"
        );
    }

    #[test]
    fn an_action_mirrors_its_decision_and_result() {
        let action = ActionSpec::new(
            ToolName::new("fs.read").unwrap(),
            serde_json::json!({ "path": "data/sandbox/a" }),
            Principal::system(),
        );
        let result = ActionResult::ok(
            Ulid::from_parts(1_700_000_000_000, 1),
            serde_json::json!({}),
        );
        let quads = action_quads(
            &Ulid::from_parts(1_700_000_000_000, 1),
            &action,
            &result,
            &Decision::Allow,
            SandboxTier::WasmCaps,
            Timestamp::from_epoch_seconds(1_700_000_000),
        );
        let rendered = quads.iter().map(render_quad).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("permissionDecision"), "{rendered}");
        assert!(rendered.contains("resultHash"), "{rendered}");
        assert!(rendered.contains(result.status.as_str()), "{rendered}");
        assert_eq!(result.status, ActionStatus::Ok);
    }

    #[test]
    fn a_denied_action_names_its_reason() {
        let action = ActionSpec::new(
            ToolName::new("process.exec").unwrap(),
            serde_json::json!({ "cmd": "cargo test" }),
            Principal::system(),
        );
        let result = ActionResult::denied(
            Ulid::from_parts(1_700_000_000_000, 2),
            "no grant covers process:execute:cargo",
        );
        let decision = Decision::Deny {
            reason: crate::permissions::DenyReason::NoGrant {
                tool: Some("process.exec".into()),
                resource: "process:execute:cargo".into(),
            },
        };
        let quads = action_quads(
            &Ulid::from_parts(1_700_000_000_000, 2),
            &action,
            &result,
            &decision,
            SandboxTier::WasmCaps,
            Timestamp::from_epoch_seconds(1_700_000_000),
        );
        let rendered = quads.iter().map(render_quad).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("refusalReason"), "{rendered}");
        assert!(rendered.contains("permission.no_grant"), "{rendered}");
    }

    #[test]
    fn an_observation_always_carries_its_evidence() {
        let observation = Observation::from_outcome(
            Ulid::from_parts(1_700_000_000_000, 1),
            ToolName::new("fs.read").unwrap(),
            crate::observation::SourceRef::tool("data/sandbox/a"),
            crate::observation::ObservationPayload::new(
                "read 1 byte",
                serde_json::json!({}),
                Vec::new(),
            ),
            Ulid::from_parts(1_700_000_000_000, 2),
            Ulid::from_parts(1_700_000_000_000, 3),
        )
        .unwrap();
        let quads = observation_quads(&observation);
        let rendered = quads.iter().map(render_quad).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("producedEvidence"), "{rendered}");
        assert!(rendered.contains("observedIn"), "{rendered}");
        assert!(rendered.contains("ActionObservation"), "{rendered}");
        assert!(rendered.contains(&mm_core::ulid_string(&observation.evidence_id)));
        assert_eq!(observation_quads(&observation), quads);
    }

    #[test]
    fn the_hash_ignores_insertion_order_and_duplicates() {
        let quads = tool_spec_quads(&spec());
        let mut reversed = quads.clone();
        reversed.reverse();
        assert_eq!(quads_hash(&quads), quads_hash(&reversed));

        let mut duplicated = quads.clone();
        duplicated.extend(quads.clone());
        assert_eq!(quads_hash(&quads), quads_hash(&duplicated));

        let mut changed = quads.clone();
        changed.push(quad(
            &node("https://metamind.dev/data/x"),
            &iri::mm("extra").into_string(),
            literal("x"),
            TOOLS_GRAPH,
        ));
        assert_ne!(quads_hash(&quads), quads_hash(&changed));
    }

    #[test]
    fn quads_render_canonically() {
        let q = quad(
            &node("https://metamind.dev/data/a"),
            "https://metamind.dev/ontology#x",
            literal("y"),
            TOOLS_GRAPH,
        );
        assert_eq!(
            render_quad(&q),
            format!(
                "<https://metamind.dev/data/a> <https://metamind.dev/ontology#x> \"y\" <{}>",
                iri::graph(TOOLS_GRAPH)
            )
        );
    }
}
