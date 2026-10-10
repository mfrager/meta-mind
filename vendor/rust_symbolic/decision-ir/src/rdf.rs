//! The `decision:` RDF codec. `RiskMeasure` variants serialize to constant
//! class IRIs; every other value is a class-scoped property or a list.

use rdf_codec::scalar::Prim;
use rdf_codec::{CodecError, FromRdf, Namespace, RdfContext, RdfReader, ToRdf};

pub const DECISION_NS: &str = "https://example.org/ns/decision#";
pub const DECISION_DATA: &str = "https://example.org/data/decision/";

pub fn schema() -> Namespace {
    Namespace::new(DECISION_NS)
}

pub fn data() -> Namespace {
    Namespace::new(DECISION_DATA)
}

use crate::decision::{
    Action, BayesianAction, BayesianProblem, BayesianVerdict, Constraint, DecisionProblem,
    DecisionReport, DecisionVerdict, Experiment, Mdp, MdpTransition, MdpVerdict,
    MultiObjectiveProblem, ObjectiveAction, Outcome, ParetoVerdict, Preference, RiskMeasure,
    StateLottery, Violation,
};

fn prop(class: &str, field: &str) -> String {
    format!("{DECISION_NS}{class}/{field}")
}

fn open<'a>(graph: &'a rdf_codec::Graph, node: &rdf_codec::NamedNode) -> RdfReader<'a> {
    static NS: std::sync::OnceLock<Namespace> = std::sync::OnceLock::new();
    RdfReader::new(graph, node, NS.get_or_init(|| Namespace::new(DECISION_NS)))
}

fn allowed(props: &[String]) -> Vec<&str> {
    props.iter().map(|s| s.as_str()).collect()
}

fn string_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<String, CodecError> {
    reader
        .literal(&prop(class, field))
        .and_then(|l| match Prim::from_literal(&l)? {
            Prim::Text(s) => Ok(s),
            other => Err(CodecError::Decode(format!(
                "{class}/{field} must be a string, got {other:?}"
            ))),
        })
}

fn int_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<i64, CodecError> {
    reader
        .literal(&prop(class, field))
        .and_then(|l| match Prim::from_literal(&l)? {
            Prim::Int(i) => Ok(i),
            other => Err(CodecError::Decode(format!(
                "{class}/{field} must be an integer, got {other:?}"
            ))),
        })
}

fn float_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<f64, CodecError> {
    reader
        .literal(&prop(class, field))
        .and_then(|l| match Prim::from_literal(&l)? {
            prim => prim.as_float().ok_or_else(|| {
                CodecError::Decode(format!("{class}/{field} must be a float, got {prim:?}"))
            }),
        })
}

fn emit_nodes(
    ctx: &mut RdfContext,
    node: &rdf_codec::NamedNode,
    class: &str,
    field: &str,
    items: Vec<rdf_codec::NamedNode>,
) -> Result<(), CodecError> {
    let heads: Vec<rdf_codec::Term> = items.into_iter().map(rdf_codec::Term::from).collect();
    let list = ctx.list(&heads)?;
    ctx.emit_object(node, class, field, &list);
    Ok(())
}

/// The node list of an optional property: empty when the property is absent
/// (new optional fields must decode documents that predate them).
fn optional_node_list(
    reader: &RdfReader<'_>,
    class: &str,
    field: &str,
) -> Result<Vec<rdf_codec::NamedNode>, CodecError> {
    if reader.optional_object(&prop(class, field))?.is_none() {
        return Ok(Vec::new());
    }
    reader.list_objects(&prop(class, field))
}

// ── RiskMeasure (variant classes with optional payload) ───────────────────

fn risk_emit(ctx: &mut RdfContext, risk: &RiskMeasure) -> Result<rdf_codec::NamedNode, CodecError> {
    let n = ctx.instance();
    match risk {
        RiskMeasure::Variance => {
            ctx.emit_type(&n, "RiskMeasure/Variance");
        }
        RiskMeasure::Var { alpha } | RiskMeasure::Cvar { alpha } => {
            let class = match risk {
                RiskMeasure::Var { .. } => "RiskMeasure/Var",
                _ => "RiskMeasure/Cvar",
            };
            ctx.emit_type(&n, class);
            ctx.emit_literal(
                &n,
                "RiskMeasure",
                "alpha",
                Prim::Float(*alpha).to_literal()?,
            );
        }
    }
    Ok(n)
}

fn risk_read(
    node: &rdf_codec::NamedNode,
    graph: &rdf_codec::Graph,
) -> Result<RiskMeasure, CodecError> {
    let reader = open(graph, node);
    let alpha = match reader.optional_literal(&prop("RiskMeasure", "alpha"))? {
        Some(l) => match Prim::from_literal(&l)? {
            Prim::Float(f) => f,
            other => {
                return Err(CodecError::Decode(format!(
                    "RiskMeasure/alpha must be a float, got {other:?}"
                )))
            }
        },
        None => 0.0,
    };
    if reader.expect_type("RiskMeasure/Variance").is_ok() {
        reader.check_known(&allowed(&[]))?;
        Ok(RiskMeasure::Variance)
    } else if reader.expect_type("RiskMeasure/Var").is_ok() {
        reader.check_known(&allowed(&[prop("RiskMeasure", "alpha")]))?;
        Ok(RiskMeasure::Var { alpha })
    } else if reader.expect_type("RiskMeasure/Cvar").is_ok() {
        reader.check_known(&allowed(&[prop("RiskMeasure", "alpha")]))?;
        Ok(RiskMeasure::Cvar { alpha })
    } else {
        Err(CodecError::Decode(format!(
            "`{}` is not typed as any RiskMeasure variant",
            reader.node()
        )))
    }
}

impl ToRdf for RiskMeasure {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        risk_emit(ctx, self)
    }
}

impl FromRdf for RiskMeasure {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        risk_read(node, graph)
    }
}

// ── trait impls ────────────────────────────────────────────────────────────

impl ToRdf for Outcome {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "Outcome");
        ctx.emit_literal(
            &n,
            "Outcome",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Outcome",
            "probability",
            Prim::Float(self.probability).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Outcome",
            "utility",
            Prim::Float(self.utility).to_literal()?,
        );
        Ok(n)
    }
}

impl FromRdf for Outcome {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("Outcome")?;
        let name = string_field(&reader, "Outcome", "name")?;
        let probability = float_field(&reader, "Outcome", "probability")?;
        let utility = float_field(&reader, "Outcome", "utility")?;
        reader.check_known(&allowed(&[
            prop("Outcome", "name"),
            prop("Outcome", "probability"),
            prop("Outcome", "utility"),
        ]))?;
        Ok(Outcome {
            name,
            probability,
            utility,
        })
    }
}

impl ToRdf for Action {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "Action");
        ctx.emit_literal(
            &n,
            "Action",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        ctx.emit_literal(&n, "Action", "cost", Prim::Float(self.cost).to_literal()?);
        let mut heads = Vec::new();
        for o in &self.outcomes {
            heads.push(o.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "Action", "outcomes", heads)?;
        Ok(n)
    }
}

impl FromRdf for Action {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("Action")?;
        let name = string_field(&reader, "Action", "name")?;
        let cost = float_field(&reader, "Action", "cost")?;
        let mut outcomes = Vec::new();
        for head in reader.list_objects(&prop("Action", "outcomes"))? {
            outcomes.push(Outcome::from_rdf(&head, graph)?);
        }
        reader.check_known(&allowed(&[
            prop("Action", "name"),
            prop("Action", "cost"),
            prop("Action", "outcomes"),
        ]))?;
        Ok(Action {
            name,
            outcomes,
            cost,
        })
    }
}

impl ToRdf for Preference {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "Preference");
        ctx.emit_literal(
            &n,
            "Preference",
            "preferred",
            Prim::Text(self.preferred.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Preference",
            "over",
            Prim::Text(self.over.clone()).to_literal()?,
        );
        Ok(n)
    }
}

impl FromRdf for Preference {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("Preference")?;
        let preferred = string_field(&reader, "Preference", "preferred")?;
        let over = string_field(&reader, "Preference", "over")?;
        reader.check_known(&allowed(&[
            prop("Preference", "preferred"),
            prop("Preference", "over"),
        ]))?;
        Ok(Preference { preferred, over })
    }
}

impl ToRdf for Constraint {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "Constraint");
        ctx.emit_literal(
            &n,
            "Constraint",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Constraint",
            "action",
            Prim::Text(self.action.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Constraint",
            "maxCost",
            Prim::Float(self.max_cost).to_literal()?,
        );
        Ok(n)
    }
}

impl FromRdf for Constraint {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("Constraint")?;
        let name = string_field(&reader, "Constraint", "name")?;
        let action = string_field(&reader, "Constraint", "action")?;
        let max_cost = float_field(&reader, "Constraint", "maxCost")?;
        reader.check_known(&allowed(&[
            prop("Constraint", "name"),
            prop("Constraint", "action"),
            prop("Constraint", "maxCost"),
        ]))?;
        Ok(Constraint {
            name,
            action,
            max_cost,
        })
    }
}

impl ToRdf for Mdp {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "Mdp");
        ctx.emit_literal(
            &n,
            "Mdp",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Mdp",
            "discount",
            Prim::Float(self.discount).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Mdp",
            "horizon",
            Prim::Int(self.horizon as i64).to_literal()?,
        );
        let mut s_heads = Vec::new();
        for s in &self.states {
            let x = ctx.instance();
            ctx.emit_type(&x, "StateEntry");
            ctx.emit_literal(
                &x,
                "StateEntry",
                "state",
                Prim::Text(s.clone()).to_literal()?,
            );
            s_heads.push(x);
        }
        emit_nodes(ctx, &n, "Mdp", "states", s_heads)?;
        let mut a_heads = Vec::new();
        for a in &self.actions {
            let x = ctx.instance();
            ctx.emit_type(&x, "ActionEntry");
            ctx.emit_literal(
                &x,
                "ActionEntry",
                "action",
                Prim::Text(a.clone()).to_literal()?,
            );
            a_heads.push(x);
        }
        emit_nodes(ctx, &n, "Mdp", "actions", a_heads)?;
        let mut t_heads = Vec::new();
        for t in &self.transitions {
            let x = ctx.instance();
            ctx.emit_type(&x, "MdpTransition");
            ctx.emit_literal(
                &x,
                "MdpTransition",
                "state",
                Prim::Text(t.state.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "MdpTransition",
                "action",
                Prim::Text(t.action.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "MdpTransition",
                "nextState",
                Prim::Text(t.next_state.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "MdpTransition",
                "probability",
                Prim::Float(t.probability).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "MdpTransition",
                "reward",
                Prim::Float(t.reward).to_literal()?,
            );
            t_heads.push(x);
        }
        emit_nodes(ctx, &n, "Mdp", "transitions", t_heads)?;
        Ok(n)
    }
}

impl FromRdf for Mdp {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("Mdp")?;
        let name = string_field(&reader, "Mdp", "name")?;
        let discount = float_field(&reader, "Mdp", "discount")?;
        let horizon = int_field(&reader, "Mdp", "horizon")? as u32;
        let mut states = Vec::new();
        for head in optional_node_list(&reader, "Mdp", "states")? {
            let x = open(graph, &head);
            x.expect_type("StateEntry")?;
            states.push(string_field(&x, "StateEntry", "state")?);
        }
        let mut actions = Vec::new();
        for head in optional_node_list(&reader, "Mdp", "actions")? {
            let x = open(graph, &head);
            x.expect_type("ActionEntry")?;
            actions.push(string_field(&x, "ActionEntry", "action")?);
        }
        let mut transitions = Vec::new();
        for head in optional_node_list(&reader, "Mdp", "transitions")? {
            let x = open(graph, &head);
            x.expect_type("MdpTransition")?;
            transitions.push(MdpTransition {
                state: string_field(&x, "MdpTransition", "state")?,
                action: string_field(&x, "MdpTransition", "action")?,
                next_state: string_field(&x, "MdpTransition", "nextState")?,
                probability: float_field(&x, "MdpTransition", "probability")?,
                reward: float_field(&x, "MdpTransition", "reward")?,
            });
        }
        reader.check_known(&allowed(&[
            prop("Mdp", "name"),
            prop("Mdp", "discount"),
            prop("Mdp", "horizon"),
            prop("Mdp", "states"),
            prop("Mdp", "actions"),
            prop("Mdp", "transitions"),
        ]))?;
        Ok(Mdp {
            name,
            states,
            actions,
            transitions,
            discount,
            horizon,
        })
    }
}

impl ToRdf for MdpVerdict {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "MdpVerdict");
        ctx.emit_literal(
            &n,
            "MdpVerdict",
            "mdp",
            Prim::Text(self.mdp.clone()).to_literal()?,
        );
        let mut v_heads = Vec::new();
        for (state, value) in &self.values {
            let x = ctx.instance();
            ctx.emit_type(&x, "ValueEntry");
            ctx.emit_literal(
                &x,
                "ValueEntry",
                "state",
                Prim::Text(state.clone()).to_literal()?,
            );
            ctx.emit_literal(&x, "ValueEntry", "value", Prim::Float(*value).to_literal()?);
            v_heads.push(x);
        }
        emit_nodes(ctx, &n, "MdpVerdict", "values", v_heads)?;
        let mut p_heads = Vec::new();
        for (state, action) in &self.policy {
            let x = ctx.instance();
            ctx.emit_type(&x, "PolicyEntry");
            ctx.emit_literal(
                &x,
                "PolicyEntry",
                "state",
                Prim::Text(state.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "PolicyEntry",
                "action",
                Prim::Text(action.clone()).to_literal()?,
            );
            p_heads.push(x);
        }
        emit_nodes(ctx, &n, "MdpVerdict", "policy", p_heads)?;
        Ok(n)
    }
}

impl FromRdf for MdpVerdict {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("MdpVerdict")?;
        let mdp = string_field(&reader, "MdpVerdict", "mdp")?;
        let mut values = Vec::new();
        for head in optional_node_list(&reader, "MdpVerdict", "values")? {
            let x = open(graph, &head);
            x.expect_type("ValueEntry")?;
            let state = string_field(&x, "ValueEntry", "state")?;
            let value = float_field(&x, "ValueEntry", "value")?;
            values.push((state, value));
        }
        let mut policy = Vec::new();
        for head in optional_node_list(&reader, "MdpVerdict", "policy")? {
            let x = open(graph, &head);
            x.expect_type("PolicyEntry")?;
            let state = string_field(&x, "PolicyEntry", "state")?;
            let action = string_field(&x, "PolicyEntry", "action")?;
            policy.push((state, action));
        }
        reader.check_known(&allowed(&[
            prop("MdpVerdict", "mdp"),
            prop("MdpVerdict", "values"),
            prop("MdpVerdict", "policy"),
        ]))?;
        Ok(MdpVerdict {
            mdp,
            values,
            policy,
        })
    }
}

impl ToRdf for BayesianProblem {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "BayesianProblem");
        ctx.emit_literal(
            &n,
            "BayesianProblem",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        let mut s_heads = Vec::new();
        for s in &self.states {
            let x = ctx.instance();
            ctx.emit_type(&x, "StateEntry");
            ctx.emit_literal(
                &x,
                "StateEntry",
                "state",
                Prim::Text(s.clone()).to_literal()?,
            );
            s_heads.push(x);
        }
        emit_nodes(ctx, &n, "BayesianProblem", "states", s_heads)?;
        let mut b_heads = Vec::new();
        for (state, prob) in &self.belief {
            let x = ctx.instance();
            ctx.emit_type(&x, "BeliefEntry");
            ctx.emit_literal(
                &x,
                "BeliefEntry",
                "state",
                Prim::Text(state.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "BeliefEntry",
                "probability",
                Prim::Float(*prob).to_literal()?,
            );
            b_heads.push(x);
        }
        emit_nodes(ctx, &n, "BayesianProblem", "belief", b_heads)?;
        let mut a_heads = Vec::new();
        for a in &self.actions {
            let x = ctx.instance();
            ctx.emit_type(&x, "BayesianAction");
            ctx.emit_literal(
                &x,
                "BayesianAction",
                "name",
                Prim::Text(a.name.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "BayesianAction",
                "cost",
                Prim::Float(a.cost).to_literal()?,
            );
            let mut l_heads = Vec::new();
            for l in &a.state_lotteries {
                let y = ctx.instance();
                ctx.emit_type(&y, "StateLottery");
                ctx.emit_literal(
                    &y,
                    "StateLottery",
                    "state",
                    Prim::Text(l.state.clone()).to_literal()?,
                );
                let mut o_heads = Vec::new();
                for o in &l.outcomes {
                    o_heads.push(o.to_rdf(ctx)?);
                }
                emit_nodes(ctx, &y, "StateLottery", "outcomes", o_heads)?;
                l_heads.push(y);
            }
            emit_nodes(ctx, &x, "BayesianAction", "stateLotteries", l_heads)?;
            a_heads.push(x);
        }
        emit_nodes(ctx, &n, "BayesianProblem", "actions", a_heads)?;
        if let Some(e) = &self.experiment {
            let e_node = e.to_rdf(ctx)?;
            ctx.emit_object(&n, "BayesianProblem", "experiment", &e_node);
        }
        Ok(n)
    }
}

impl FromRdf for BayesianProblem {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("BayesianProblem")?;
        let name = string_field(&reader, "BayesianProblem", "name")?;
        let mut states = Vec::new();
        for head in optional_node_list(&reader, "BayesianProblem", "states")? {
            let x = open(graph, &head);
            x.expect_type("StateEntry")?;
            states.push(string_field(&x, "StateEntry", "state")?);
        }
        let mut belief = Vec::new();
        for head in optional_node_list(&reader, "BayesianProblem", "belief")? {
            let x = open(graph, &head);
            x.expect_type("BeliefEntry")?;
            let state = string_field(&x, "BeliefEntry", "state")?;
            let probability = float_field(&x, "BeliefEntry", "probability")?;
            belief.push((state, probability));
        }
        let mut actions = Vec::new();
        for head in optional_node_list(&reader, "BayesianProblem", "actions")? {
            let x = open(graph, &head);
            x.expect_type("BayesianAction")?;
            let aname = string_field(&x, "BayesianAction", "name")?;
            let cost = float_field(&x, "BayesianAction", "cost")?;
            let mut state_lotteries = Vec::new();
            for h in optional_node_list(&x, "BayesianAction", "stateLotteries")? {
                let y = open(graph, &h);
                y.expect_type("StateLottery")?;
                let lstate = string_field(&y, "StateLottery", "state")?;
                let mut outcomes = Vec::new();
                for oh in y.list_objects(&prop("StateLottery", "outcomes"))? {
                    outcomes.push(Outcome::from_rdf(&oh, graph)?);
                }
                state_lotteries.push(StateLottery {
                    state: lstate,
                    outcomes,
                });
            }
            actions.push(BayesianAction {
                name: aname,
                cost,
                state_lotteries,
            });
        }
        let experiment = match reader.optional_object(&prop("BayesianProblem", "experiment"))? {
            Some(head) => Some(Experiment::from_rdf(&head, graph)?),
            None => None,
        };
        reader.check_known(&allowed(&[
            prop("BayesianProblem", "name"),
            prop("BayesianProblem", "states"),
            prop("BayesianProblem", "belief"),
            prop("BayesianProblem", "actions"),
            prop("BayesianProblem", "experiment"),
        ]))?;
        Ok(BayesianProblem {
            name,
            states,
            belief,
            actions,
            experiment,
        })
    }
}

impl ToRdf for Experiment {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "Experiment");
        ctx.emit_literal(
            &n,
            "Experiment",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        let mut o_heads = Vec::new();
        for o in &self.observations {
            let x = ctx.instance();
            ctx.emit_type(&x, "ObservationEntry");
            ctx.emit_literal(
                &x,
                "ObservationEntry",
                "observation",
                Prim::Text(o.clone()).to_literal()?,
            );
            o_heads.push(x);
        }
        emit_nodes(ctx, &n, "Experiment", "observations", o_heads)?;
        let mut l_heads = Vec::new();
        for (obs, state, prob) in &self.likelihoods {
            let x = ctx.instance();
            ctx.emit_type(&x, "LikelihoodEntry");
            ctx.emit_literal(
                &x,
                "LikelihoodEntry",
                "observation",
                Prim::Text(obs.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "LikelihoodEntry",
                "state",
                Prim::Text(state.clone()).to_literal()?,
            );
            ctx.emit_literal(
                &x,
                "LikelihoodEntry",
                "probability",
                Prim::Float(*prob).to_literal()?,
            );
            l_heads.push(x);
        }
        emit_nodes(ctx, &n, "Experiment", "likelihoods", l_heads)?;
        Ok(n)
    }
}

impl FromRdf for Experiment {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("Experiment")?;
        let name = string_field(&reader, "Experiment", "name")?;
        let mut observations = Vec::new();
        for head in optional_node_list(&reader, "Experiment", "observations")? {
            let x = open(graph, &head);
            x.expect_type("ObservationEntry")?;
            observations.push(string_field(&x, "ObservationEntry", "observation")?);
        }
        let mut likelihoods = Vec::new();
        for head in optional_node_list(&reader, "Experiment", "likelihoods")? {
            let x = open(graph, &head);
            x.expect_type("LikelihoodEntry")?;
            let obs = string_field(&x, "LikelihoodEntry", "observation")?;
            let state = string_field(&x, "LikelihoodEntry", "state")?;
            let probability = float_field(&x, "LikelihoodEntry", "probability")?;
            likelihoods.push((obs, state, probability));
        }
        reader.check_known(&allowed(&[
            prop("Experiment", "name"),
            prop("Experiment", "observations"),
            prop("Experiment", "likelihoods"),
        ]))?;
        Ok(Experiment {
            name,
            observations,
            likelihoods,
        })
    }
}

impl ToRdf for BayesianVerdict {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "BayesianVerdict");
        ctx.emit_literal(
            &n,
            "BayesianVerdict",
            "problem",
            Prim::Text(self.problem.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "BayesianVerdict",
            "chosen",
            Prim::Text(self.chosen.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "BayesianVerdict",
            "expectedUtility",
            Prim::Float(self.expected_utility).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "BayesianVerdict",
            "valueOfInformation",
            Prim::Float(self.value_of_information).to_literal()?,
        );
        let mut r_heads = Vec::new();
        for (action, eu) in &self.ranking {
            let x = ctx.instance();
            ctx.emit_type(&x, "RankingEntry");
            ctx.emit_literal(
                &x,
                "RankingEntry",
                "action",
                Prim::Text(action.clone()).to_literal()?,
            );
            ctx.emit_literal(&x, "RankingEntry", "eu", Prim::Float(*eu).to_literal()?);
            r_heads.push(x);
        }
        emit_nodes(ctx, &n, "BayesianVerdict", "ranking", r_heads)?;
        Ok(n)
    }
}

impl FromRdf for BayesianVerdict {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("BayesianVerdict")?;
        let problem = string_field(&reader, "BayesianVerdict", "problem")?;
        let chosen = string_field(&reader, "BayesianVerdict", "chosen")?;
        let expected_utility = float_field(&reader, "BayesianVerdict", "expectedUtility")?;
        let value_of_information = float_field(&reader, "BayesianVerdict", "valueOfInformation")?;
        let mut ranking = Vec::new();
        for head in optional_node_list(&reader, "BayesianVerdict", "ranking")? {
            let x = open(graph, &head);
            x.expect_type("RankingEntry")?;
            let action = string_field(&x, "RankingEntry", "action")?;
            let eu = float_field(&x, "RankingEntry", "eu")?;
            ranking.push((action, eu));
        }
        reader.check_known(&allowed(&[
            prop("BayesianVerdict", "problem"),
            prop("BayesianVerdict", "chosen"),
            prop("BayesianVerdict", "expectedUtility"),
            prop("BayesianVerdict", "valueOfInformation"),
            prop("BayesianVerdict", "ranking"),
        ]))?;
        Ok(BayesianVerdict {
            problem,
            chosen,
            expected_utility,
            value_of_information,
            ranking,
        })
    }
}

impl ToRdf for MultiObjectiveProblem {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "MultiObjectiveProblem");
        ctx.emit_literal(
            &n,
            "MultiObjectiveProblem",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        let mut o_heads = Vec::new();
        for o in &self.objective_names {
            let x = ctx.instance();
            ctx.emit_type(&x, "ObjectiveEntry");
            ctx.emit_literal(
                &x,
                "ObjectiveEntry",
                "objective",
                Prim::Text(o.clone()).to_literal()?,
            );
            o_heads.push(x);
        }
        emit_nodes(ctx, &n, "MultiObjectiveProblem", "objectiveNames", o_heads)?;
        let mut a_heads = Vec::new();
        for a in &self.actions {
            let x = ctx.instance();
            ctx.emit_type(&x, "ObjectiveAction");
            ctx.emit_literal(
                &x,
                "ObjectiveAction",
                "name",
                Prim::Text(a.name.clone()).to_literal()?,
            );
            let mut v_heads = Vec::new();
            for v in &a.objectives {
                let y = ctx.instance();
                ctx.emit_type(&y, "ObjectiveValue");
                ctx.emit_literal(&y, "ObjectiveValue", "value", Prim::Float(*v).to_literal()?);
                v_heads.push(y);
            }
            emit_nodes(ctx, &x, "ObjectiveAction", "objectives", v_heads)?;
            a_heads.push(x);
        }
        emit_nodes(ctx, &n, "MultiObjectiveProblem", "actions", a_heads)?;
        if let Some(order) = &self.lexicographic {
            let mut i_heads = Vec::new();
            for i in order {
                let x = ctx.instance();
                ctx.emit_type(&x, "IndexEntry");
                ctx.emit_literal(
                    &x,
                    "IndexEntry",
                    "index",
                    Prim::Int(*i as i64).to_literal()?,
                );
                i_heads.push(x);
            }
            emit_nodes(ctx, &n, "MultiObjectiveProblem", "lexicographic", i_heads)?;
        }
        Ok(n)
    }
}

impl FromRdf for MultiObjectiveProblem {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("MultiObjectiveProblem")?;
        let name = string_field(&reader, "MultiObjectiveProblem", "name")?;
        let mut objective_names = Vec::new();
        for head in optional_node_list(&reader, "MultiObjectiveProblem", "objectiveNames")? {
            let x = open(graph, &head);
            x.expect_type("ObjectiveEntry")?;
            objective_names.push(string_field(&x, "ObjectiveEntry", "objective")?);
        }
        let mut actions = Vec::new();
        for head in optional_node_list(&reader, "MultiObjectiveProblem", "actions")? {
            let x = open(graph, &head);
            x.expect_type("ObjectiveAction")?;
            let aname = string_field(&x, "ObjectiveAction", "name")?;
            let mut objectives = Vec::new();
            for h in optional_node_list(&x, "ObjectiveAction", "objectives")? {
                let y = open(graph, &h);
                y.expect_type("ObjectiveValue")?;
                objectives.push(float_field(&y, "ObjectiveValue", "value")?);
            }
            actions.push(ObjectiveAction {
                name: aname,
                objectives,
            });
        }
        let mut lexicographic = None;
        let heads = optional_node_list(&reader, "MultiObjectiveProblem", "lexicographic")?;
        if !heads.is_empty() {
            let mut order = Vec::new();
            for h in heads {
                let x = open(graph, &h);
                x.expect_type("IndexEntry")?;
                order.push(int_field(&x, "IndexEntry", "index")? as usize);
            }
            lexicographic = Some(order);
        }
        reader.check_known(&allowed(&[
            prop("MultiObjectiveProblem", "name"),
            prop("MultiObjectiveProblem", "objectiveNames"),
            prop("MultiObjectiveProblem", "actions"),
            prop("MultiObjectiveProblem", "lexicographic"),
        ]))?;
        Ok(MultiObjectiveProblem {
            name,
            objective_names,
            actions,
            lexicographic,
        })
    }
}

impl ToRdf for ParetoVerdict {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "ParetoVerdict");
        ctx.emit_literal(
            &n,
            "ParetoVerdict",
            "problem",
            Prim::Text(self.problem.clone()).to_literal()?,
        );
        let mut f_heads = Vec::new();
        for f in &self.frontier {
            let x = ctx.instance();
            ctx.emit_type(&x, "FrontierEntry");
            ctx.emit_literal(
                &x,
                "FrontierEntry",
                "action",
                Prim::Text(f.clone()).to_literal()?,
            );
            f_heads.push(x);
        }
        emit_nodes(ctx, &n, "ParetoVerdict", "frontier", f_heads)?;
        let mut d_heads = Vec::new();
        for (action, dominators) in &self.dominated_by {
            let x = ctx.instance();
            ctx.emit_type(&x, "DominatedEntry");
            ctx.emit_literal(
                &x,
                "DominatedEntry",
                "action",
                Prim::Text(action.clone()).to_literal()?,
            );
            let mut dom_heads = Vec::new();
            for d in dominators {
                let y = ctx.instance();
                ctx.emit_type(&y, "DominatorEntry");
                ctx.emit_literal(
                    &y,
                    "DominatorEntry",
                    "action",
                    Prim::Text(d.clone()).to_literal()?,
                );
                dom_heads.push(y);
            }
            emit_nodes(ctx, &x, "DominatedEntry", "dominators", dom_heads)?;
            d_heads.push(x);
        }
        emit_nodes(ctx, &n, "ParetoVerdict", "dominatedBy", d_heads)?;
        let best = self.lexicographic_best.clone().unwrap_or_default();
        ctx.emit_literal(
            &n,
            "ParetoVerdict",
            "lexicographicBest",
            Prim::Text(best).to_literal()?,
        );
        Ok(n)
    }
}

impl FromRdf for ParetoVerdict {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("ParetoVerdict")?;
        let problem = string_field(&reader, "ParetoVerdict", "problem")?;
        let mut frontier = Vec::new();
        for head in optional_node_list(&reader, "ParetoVerdict", "frontier")? {
            let x = open(graph, &head);
            x.expect_type("FrontierEntry")?;
            frontier.push(string_field(&x, "FrontierEntry", "action")?);
        }
        let mut dominated_by = Vec::new();
        for head in optional_node_list(&reader, "ParetoVerdict", "dominatedBy")? {
            let x = open(graph, &head);
            x.expect_type("DominatedEntry")?;
            let action = string_field(&x, "DominatedEntry", "action")?;
            let mut dominators = Vec::new();
            for h in optional_node_list(&x, "DominatedEntry", "dominators")? {
                let y = open(graph, &h);
                y.expect_type("DominatorEntry")?;
                dominators.push(string_field(&y, "DominatorEntry", "action")?);
            }
            dominated_by.push((action, dominators));
        }
        let best = string_field(&reader, "ParetoVerdict", "lexicographicBest")?;
        let lexicographic_best = if best.is_empty() { None } else { Some(best) };
        reader.check_known(&allowed(&[
            prop("ParetoVerdict", "problem"),
            prop("ParetoVerdict", "frontier"),
            prop("ParetoVerdict", "dominatedBy"),
            prop("ParetoVerdict", "lexicographicBest"),
        ]))?;
        Ok(ParetoVerdict {
            problem,
            frontier,
            dominated_by,
            lexicographic_best,
        })
    }
}

impl ToRdf for DecisionProblem {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "DecisionProblem");
        ctx.emit_literal(
            &n,
            "DecisionProblem",
            "name",
            Prim::Text(self.name.clone()).to_literal()?,
        );
        let mut a_heads = Vec::new();
        for a in &self.actions {
            a_heads.push(a.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionProblem", "actions", a_heads)?;
        let mut p_heads = Vec::new();
        for p in &self.preferences {
            p_heads.push(p.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionProblem", "preferences", p_heads)?;
        let mut c_heads = Vec::new();
        for c in &self.constraints {
            c_heads.push(c.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionProblem", "constraints", c_heads)?;
        let risk = risk_emit(ctx, &self.risk)?;
        ctx.emit_object(&n, "DecisionProblem", "risk", &risk);
        let mut m_heads = Vec::new();
        for m in &self.mdps {
            m_heads.push(m.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionProblem", "mdps", m_heads)?;
        let mut b_heads = Vec::new();
        for b in &self.bayesian {
            b_heads.push(b.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionProblem", "bayesian", b_heads)?;
        let mut mo_heads = Vec::new();
        for mo in &self.multi_objective {
            mo_heads.push(mo.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionProblem", "multiObjective", mo_heads)?;
        Ok(n)
    }
}

impl FromRdf for DecisionProblem {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("DecisionProblem")?;
        let name = string_field(&reader, "DecisionProblem", "name")?;
        let mut actions = Vec::new();
        for head in reader.list_objects(&prop("DecisionProblem", "actions"))? {
            actions.push(Action::from_rdf(&head, graph)?);
        }
        let mut preferences = Vec::new();
        for head in reader.list_objects(&prop("DecisionProblem", "preferences"))? {
            preferences.push(Preference::from_rdf(&head, graph)?);
        }
        let mut constraints = Vec::new();
        for head in reader.list_objects(&prop("DecisionProblem", "constraints"))? {
            constraints.push(Constraint::from_rdf(&head, graph)?);
        }
        let risk = risk_read(&reader.object(&prop("DecisionProblem", "risk"))?, graph)?;
        let mut mdps = Vec::new();
        for head in optional_node_list(&reader, "DecisionProblem", "mdps")? {
            mdps.push(Mdp::from_rdf(&head, graph)?);
        }
        let mut bayesian = Vec::new();
        for head in optional_node_list(&reader, "DecisionProblem", "bayesian")? {
            bayesian.push(BayesianProblem::from_rdf(&head, graph)?);
        }
        let mut multi_objective = Vec::new();
        for head in optional_node_list(&reader, "DecisionProblem", "multiObjective")? {
            multi_objective.push(MultiObjectiveProblem::from_rdf(&head, graph)?);
        }
        reader.check_known(&allowed(&[
            prop("DecisionProblem", "name"),
            prop("DecisionProblem", "actions"),
            prop("DecisionProblem", "preferences"),
            prop("DecisionProblem", "constraints"),
            prop("DecisionProblem", "risk"),
            prop("DecisionProblem", "mdps"),
            prop("DecisionProblem", "bayesian"),
            prop("DecisionProblem", "multiObjective"),
        ]))?;
        Ok(DecisionProblem {
            name,
            actions,
            preferences,
            constraints,
            risk,
            mdps,
            bayesian,
            multi_objective,
        })
    }
}

impl ToRdf for DecisionVerdict {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "DecisionVerdict");
        ctx.emit_literal(
            &n,
            "DecisionVerdict",
            "action",
            Prim::Text(self.action.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "DecisionVerdict",
            "expectedUtility",
            Prim::Float(self.expected_utility).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "DecisionVerdict",
            "risk",
            Prim::Float(self.risk).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "DecisionVerdict",
            "regret",
            Prim::Float(self.regret).to_literal()?,
        );
        let mut r_heads = Vec::new();
        for (action, eu) in &self.ranking {
            let r = ctx.instance();
            ctx.emit_type(&r, "RankEntry");
            ctx.emit_literal(
                &r,
                "RankEntry",
                "action",
                Prim::Text(action.clone()).to_literal()?,
            );
            ctx.emit_literal(&r, "RankEntry", "value", Prim::Float(*eu).to_literal()?);
            r_heads.push(r);
        }
        emit_nodes(ctx, &n, "DecisionVerdict", "ranking", r_heads)?;
        Ok(n)
    }
}

impl FromRdf for DecisionVerdict {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("DecisionVerdict")?;
        let action = string_field(&reader, "DecisionVerdict", "action")?;
        let expected_utility = float_field(&reader, "DecisionVerdict", "expectedUtility")?;
        let risk = float_field(&reader, "DecisionVerdict", "risk")?;
        let regret = float_field(&reader, "DecisionVerdict", "regret")?;
        let mut ranking = Vec::new();
        for head in reader.list_objects(&prop("DecisionVerdict", "ranking"))? {
            let r = open(graph, &head);
            let name = string_field(&r, "RankEntry", "action")?;
            let value = float_field(&r, "RankEntry", "value")?;
            ranking.push((name, value));
        }
        reader.check_known(&allowed(&[
            prop("DecisionVerdict", "action"),
            prop("DecisionVerdict", "expectedUtility"),
            prop("DecisionVerdict", "risk"),
            prop("DecisionVerdict", "regret"),
            prop("DecisionVerdict", "ranking"),
        ]))?;
        Ok(DecisionVerdict {
            action,
            expected_utility,
            risk,
            regret,
            ranking,
        })
    }
}

impl ToRdf for Violation {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "Violation");
        ctx.emit_literal(
            &n,
            "Violation",
            "rule",
            Prim::Text(self.rule.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &n,
            "Violation",
            "detail",
            Prim::Text(self.detail.clone()).to_literal()?,
        );
        Ok(n)
    }
}

impl FromRdf for Violation {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("Violation")?;
        let rule = string_field(&reader, "Violation", "rule")?;
        let detail = string_field(&reader, "Violation", "detail")?;
        reader.check_known(&allowed(&[
            prop("Violation", "rule"),
            prop("Violation", "detail"),
        ]))?;
        Ok(Violation { rule, detail })
    }
}

impl ToRdf for DecisionReport {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let n = ctx.instance();
        ctx.emit_type(&n, "DecisionReport");
        ctx.emit_literal(
            &n,
            "DecisionReport",
            "problem",
            Prim::Text(self.problem.clone()).to_literal()?,
        );
        let verdict = self.verdict.to_rdf(ctx)?;
        ctx.emit_object(&n, "DecisionReport", "verdict", &verdict);
        let mut m_heads = Vec::new();
        for m in &self.mdp_verdicts {
            m_heads.push(m.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionReport", "mdpVerdicts", m_heads)?;
        let mut b_heads = Vec::new();
        for b in &self.bayesian_verdicts {
            b_heads.push(b.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionReport", "bayesianVerdicts", b_heads)?;
        let mut p_heads = Vec::new();
        for p in &self.pareto_verdicts {
            p_heads.push(p.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionReport", "paretoVerdicts", p_heads)?;
        let mut v_heads = Vec::new();
        for v in &self.violations {
            v_heads.push(v.to_rdf(ctx)?);
        }
        emit_nodes(ctx, &n, "DecisionReport", "violations", v_heads)?;
        Ok(n)
    }
}

impl FromRdf for DecisionReport {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("DecisionReport")?;
        let problem = string_field(&reader, "DecisionReport", "problem")?;
        let verdict =
            DecisionVerdict::from_rdf(&reader.object(&prop("DecisionReport", "verdict"))?, graph)?;
        let mut mdp_verdicts = Vec::new();
        for head in optional_node_list(&reader, "DecisionReport", "mdpVerdicts")? {
            mdp_verdicts.push(MdpVerdict::from_rdf(&head, graph)?);
        }
        let mut bayesian_verdicts = Vec::new();
        for head in optional_node_list(&reader, "DecisionReport", "bayesianVerdicts")? {
            bayesian_verdicts.push(BayesianVerdict::from_rdf(&head, graph)?);
        }
        let mut pareto_verdicts = Vec::new();
        for head in optional_node_list(&reader, "DecisionReport", "paretoVerdicts")? {
            pareto_verdicts.push(ParetoVerdict::from_rdf(&head, graph)?);
        }
        let mut violations = Vec::new();
        for head in reader.list_objects(&prop("DecisionReport", "violations"))? {
            violations.push(Violation::from_rdf(&head, graph)?);
        }
        reader.check_known(&allowed(&[
            prop("DecisionReport", "problem"),
            prop("DecisionReport", "verdict"),
            prop("DecisionReport", "mdpVerdicts"),
            prop("DecisionReport", "bayesianVerdicts"),
            prop("DecisionReport", "paretoVerdicts"),
            prop("DecisionReport", "violations"),
        ]))?;
        Ok(DecisionReport {
            problem,
            verdict,
            mdp_verdicts,
            bayesian_verdicts,
            pareto_verdicts,
            violations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DecisionEngine;

    fn roundtrip<T: ToRdf + FromRdf + PartialEq + std::fmt::Debug>(value: &T) {
        let mut ctx = RdfContext::new(schema(), data());
        let node = value.to_rdf(&mut ctx).unwrap();
        let turtle =
            rdf_codec::io::to_turtle_with_prefixes(&ctx.graph, &[("decision", DECISION_NS)])
                .unwrap();
        let parsed = rdf_codec::io::parse_turtle(&turtle).unwrap();
        let back = T::from_rdf(&node, &parsed).unwrap();
        assert_eq!(value, &back);
    }

    fn sample_problem() -> DecisionProblem {
        DecisionProblem {
            name: "invest".into(),
            actions: vec![
                Action {
                    name: "bond".into(),
                    outcomes: vec![Outcome {
                        name: "payoff".into(),
                        probability: 1.0,
                        utility: 4.0,
                    }],
                    cost: 0.0,
                },
                Action {
                    name: "stock".into(),
                    outcomes: vec![
                        Outcome {
                            name: "up".into(),
                            probability: 0.5,
                            utility: 12.0,
                        },
                        Outcome {
                            name: "down".into(),
                            probability: 0.5,
                            utility: -4.0,
                        },
                    ],
                    cost: 1.0,
                },
            ],
            preferences: vec![Preference {
                preferred: "bond".into(),
                over: "stock".into(),
            }],
            constraints: vec![],
            risk: RiskMeasure::Cvar { alpha: 0.1 },
            mdps: vec![],
            bayesian: vec![],
            multi_objective: vec![],
        }
    }

    #[test]
    fn problem_roundtrip() {
        roundtrip(&sample_problem());
    }

    #[test]
    fn report_roundtrip() {
        let report = DecisionEngine::analyze(&sample_problem());
        roundtrip(&report);
    }

    fn sample_rich_problem() -> DecisionProblem {
        let mut p = sample_problem();
        p.mdps = vec![Mdp {
            name: "m".into(),
            states: vec!["s0".into(), "s1".into()],
            actions: vec!["a".into()],
            transitions: vec![MdpTransition {
                state: "s0".into(),
                action: "a".into(),
                next_state: "s1".into(),
                probability: 1.0,
                reward: 2.0,
            }],
            discount: 0.9,
            horizon: 2,
        }];
        p.bayesian = vec![BayesianProblem {
            name: "b".into(),
            states: vec!["g".into(), "b".into()],
            belief: vec![("g".into(), 0.5), ("b".into(), 0.5)],
            actions: vec![BayesianAction {
                name: "bet".into(),
                cost: 0.0,
                state_lotteries: vec![StateLottery {
                    state: "g".into(),
                    outcomes: vec![Outcome {
                        name: "win".into(),
                        probability: 1.0,
                        utility: 10.0,
                    }],
                }],
            }],
            experiment: Some(Experiment {
                name: "e".into(),
                observations: vec!["pos".into()],
                likelihoods: vec![("pos".into(), "g".into(), 0.9)],
            }),
        }];
        p.multi_objective = vec![MultiObjectiveProblem {
            name: "mo".into(),
            objective_names: vec!["gdp".into(), "emp".into()],
            actions: vec![ObjectiveAction {
                name: "x".into(),
                objectives: vec![1.0, 2.0],
            }],
            lexicographic: Some(vec![0, 1]),
        }];
        p
    }

    #[test]
    fn rich_problem_roundtrip() {
        roundtrip(&sample_rich_problem());
    }

    #[test]
    fn rich_report_roundtrip() {
        let report = DecisionEngine::analyze(&sample_rich_problem());
        roundtrip(&report);
    }

    #[test]
    fn risk_variants_roundtrip() {
        for risk in [
            RiskMeasure::Variance,
            RiskMeasure::Var { alpha: 0.05 },
            RiskMeasure::Cvar { alpha: 0.1 },
        ] {
            roundtrip(&risk);
        }
    }
}
