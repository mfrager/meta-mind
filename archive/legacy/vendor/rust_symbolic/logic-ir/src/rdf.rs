//! RDF codec for the structured arithmetic and complexity-model vocabulary.

use rdf_codec::scalar::Prim;
use rdf_codec::{CodecError, FromRdf, Namespace, RdfContext, RdfReader, ToRdf};

use crate::{ArithmeticExpr, ComplexityExpr, ComplexityKind, ComplexityModel, Term};

const COMPLEXITY_NS: &str = "https://example.org/ns/complexity#";

fn open<'a>(graph: &'a rdf_codec::Graph, node: &rdf_codec::NamedNode) -> RdfReader<'a> {
    static NS: std::sync::OnceLock<Namespace> = std::sync::OnceLock::new();
    RdfReader::new(
        graph,
        node,
        NS.get_or_init(|| Namespace::new(COMPLEXITY_NS)),
    )
}

fn prop(class: &str, field: &str) -> String {
    format!("{COMPLEXITY_NS}{class}/{field}")
}

fn allowed(props: &[String]) -> Vec<&str> {
    props.iter().map(String::as_str).collect()
}

fn string_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<String, CodecError> {
    match Prim::from_literal(&reader.literal(&prop(class, field))?)? {
        Prim::Text(value) => Ok(value),
        other => Err(CodecError::Decode(format!(
            "{class}/{field} must be text, got {other:?}"
        ))),
    }
}

fn bool_field(reader: &RdfReader<'_>, class: &str, field: &str) -> Result<bool, CodecError> {
    match Prim::from_literal(&reader.literal(&prop(class, field))?)? {
        Prim::Bool(value) => Ok(value),
        other => Err(CodecError::Decode(format!(
            "{class}/{field} must be boolean, got {other:?}"
        ))),
    }
}

fn string_list(
    reader: &RdfReader<'_>,
    class: &str,
    field: &str,
) -> Result<Vec<String>, CodecError> {
    reader
        .list(&prop(class, field))?
        .into_iter()
        .map(|term| match term {
            rdf_codec::Term::Literal(literal) => match Prim::from_literal(&literal)? {
                Prim::Text(value) => Ok(value),
                other => Err(CodecError::Decode(format!(
                    "{class}/{field} list must contain text, got {other:?}"
                ))),
            },
            other => Err(CodecError::Decode(format!(
                "{class}/{field} list must contain literals, got {other}"
            ))),
        })
        .collect()
}

fn emit_expr_list(
    ctx: &mut RdfContext,
    node: &rdf_codec::NamedNode,
    class: &str,
    field: &str,
    expressions: &[ArithmeticExpr],
) -> Result<(), CodecError> {
    let mut items = Vec::with_capacity(expressions.len());
    for expression in expressions {
        items.push(rdf_codec::Term::from(expression.to_rdf(ctx)?));
    }
    let list = ctx.list(&items)?;
    ctx.emit_object(node, class, field, &list);
    Ok(())
}

impl ToRdf for ArithmeticExpr {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let node = ctx.instance();
        match self {
            Self::Var(name) => {
                ctx.emit_type(&node, "ArithmeticExpr/Var");
                ctx.emit_literal(
                    &node,
                    "ArithmeticExpr",
                    "name",
                    Prim::Text(name.clone()).to_literal()?,
                );
            }
            Self::Const(value) => {
                ctx.emit_type(&node, "ArithmeticExpr/Const");
                ctx.emit_literal(
                    &node,
                    "ArithmeticExpr",
                    "value",
                    Prim::Text(value.clone()).to_literal()?,
                );
            }
            Self::Add(items) | Self::Mul(items) => {
                ctx.emit_type(
                    &node,
                    if matches!(self, Self::Add(_)) {
                        "ArithmeticExpr/Add"
                    } else {
                        "ArithmeticExpr/Mul"
                    },
                );
                emit_expr_list(ctx, &node, "ArithmeticExpr", "items", items)?;
            }
            Self::Sub(left, right) | Self::Div(left, right) | Self::Pow(left, right) => {
                let kind = match self {
                    Self::Sub(..) => "Sub",
                    Self::Div(..) => "Div",
                    _ => "Pow",
                };
                ctx.emit_type(&node, &format!("ArithmeticExpr/{kind}"));
                let left_node = left.to_rdf(ctx)?;
                let right_node = right.to_rdf(ctx)?;
                ctx.emit_object(&node, "ArithmeticExpr", "left", &left_node);
                ctx.emit_object(&node, "ArithmeticExpr", "right", &right_node);
            }
            Self::Neg(inner) => {
                ctx.emit_type(&node, "ArithmeticExpr/Neg");
                let inner_node = inner.to_rdf(ctx)?;
                ctx.emit_object(&node, "ArithmeticExpr", "inner", &inner_node);
            }
            Self::Function { name, arguments } => {
                ctx.emit_type(&node, "ArithmeticExpr/Function");
                ctx.emit_literal(
                    &node,
                    "ArithmeticExpr",
                    "function",
                    Prim::Text(name.clone()).to_literal()?,
                );
                emit_expr_list(ctx, &node, "ArithmeticExpr", "arguments", arguments)?;
            }
            Self::Lag(inner, dimension) | Self::Index(inner, dimension) => {
                ctx.emit_type(
                    &node,
                    if matches!(self, Self::Lag(..)) {
                        "ArithmeticExpr/Lag"
                    } else {
                        "ArithmeticExpr/Index"
                    },
                );
                let inner_node = inner.to_rdf(ctx)?;
                ctx.emit_object(&node, "ArithmeticExpr", "inner", &inner_node);
                ctx.emit_literal(
                    &node,
                    "ArithmeticExpr",
                    "dimension",
                    Prim::Text(dimension.clone()).to_literal()?,
                );
            }
        }
        Ok(node)
    }
}

impl FromRdf for ArithmeticExpr {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        let type_name = reader
            .type_local()
            .ok_or_else(|| CodecError::Decode(format!("`{node}` has no ArithmeticExpr type")))?;
        let result = match type_name.as_str() {
            "ArithmeticExpr/Var" => Self::Var(string_field(&reader, "ArithmeticExpr", "name")?),
            "ArithmeticExpr/Const" => {
                Self::Const(string_field(&reader, "ArithmeticExpr", "value")?)
            }
            "ArithmeticExpr/Add" | "ArithmeticExpr/Mul" => {
                let items = reader
                    .list_objects(&prop("ArithmeticExpr", "items"))?
                    .into_iter()
                    .map(|head| Self::from_rdf(&head, graph))
                    .collect::<Result<Vec<_>, _>>()?;
                if type_name.ends_with("/Add") {
                    Self::Add(items)
                } else {
                    Self::Mul(items)
                }
            }
            "ArithmeticExpr/Sub" | "ArithmeticExpr/Div" | "ArithmeticExpr/Pow" => {
                let left = Self::from_rdf(&reader.object(&prop("ArithmeticExpr", "left"))?, graph)?;
                let right =
                    Self::from_rdf(&reader.object(&prop("ArithmeticExpr", "right"))?, graph)?;
                match type_name.as_str() {
                    "ArithmeticExpr/Sub" => Self::Sub(Box::new(left), Box::new(right)),
                    "ArithmeticExpr/Div" => Self::Div(Box::new(left), Box::new(right)),
                    _ => Self::Pow(Box::new(left), Box::new(right)),
                }
            }
            "ArithmeticExpr/Neg" => Self::Neg(Box::new(Self::from_rdf(
                &reader.object(&prop("ArithmeticExpr", "inner"))?,
                graph,
            )?)),
            "ArithmeticExpr/Function" => {
                let name = string_field(&reader, "ArithmeticExpr", "function")?;
                let arguments = reader
                    .list_objects(&prop("ArithmeticExpr", "arguments"))?
                    .into_iter()
                    .map(|head| Self::from_rdf(&head, graph))
                    .collect::<Result<Vec<_>, _>>()?;
                Self::Function { name, arguments }
            }
            "ArithmeticExpr/Lag" | "ArithmeticExpr/Index" => {
                let inner =
                    Self::from_rdf(&reader.object(&prop("ArithmeticExpr", "inner"))?, graph)?;
                let dimension = string_field(&reader, "ArithmeticExpr", "dimension")?;
                if type_name.ends_with("/Lag") {
                    Self::Lag(Box::new(inner), dimension)
                } else {
                    Self::Index(Box::new(inner), dimension)
                }
            }
            other => {
                return Err(CodecError::Decode(format!(
                    "unsupported ArithmeticExpr type `{other}"
                )))
            }
        };
        let allowed_props = [
            prop("ArithmeticExpr", "name"),
            prop("ArithmeticExpr", "value"),
            prop("ArithmeticExpr", "items"),
            prop("ArithmeticExpr", "left"),
            prop("ArithmeticExpr", "right"),
            prop("ArithmeticExpr", "inner"),
            prop("ArithmeticExpr", "function"),
            prop("ArithmeticExpr", "arguments"),
            prop("ArithmeticExpr", "dimension"),
        ];
        reader.check_known(&allowed(&allowed_props))?;
        Ok(result)
    }
}

impl ToRdf for ComplexityModel {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let node = ctx.instance();
        ctx.emit_type(&node, "ComplexityModel");
        ctx.emit_literal(
            &node,
            "ComplexityModel",
            "subject",
            Prim::Text(self.subject.clone()).to_literal()?,
        );
        ctx.emit_literal(
            &node,
            "ComplexityModel",
            "kind",
            Prim::Text(self.kind.as_str().to_string()).to_literal()?,
        );
        let variables = self
            .variables
            .iter()
            .map(|value| rdf_codec::Term::from(Prim::Text(value.clone()).to_literal().unwrap()))
            .collect::<Vec<_>>();
        let variable_list = ctx.list(&variables)?;
        ctx.emit_object(&node, "ComplexityModel", "variables", &variable_list);
        let expression = self.expression.to_rdf(ctx)?;
        ctx.emit_object(&node, "ComplexityModel", "expression", &expression);
        ctx.emit_literal(
            &node,
            "ComplexityModel",
            "asymptotic",
            Prim::Bool(self.asymptotic).to_literal()?,
        );
        Ok(node)
    }
}

impl FromRdf for ComplexityModel {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        reader.expect_type("ComplexityModel")?;
        let subject = string_field(&reader, "ComplexityModel", "subject")?;
        let kind_name = string_field(&reader, "ComplexityModel", "kind")?;
        let kind = ComplexityKind::from_name(&kind_name)
            .ok_or_else(|| CodecError::Decode(format!("unknown complexity kind `{kind_name}`")))?;
        let variables = string_list(&reader, "ComplexityModel", "variables")?;
        let expression = ArithmeticExpr::from_rdf(
            &reader.object(&prop("ComplexityModel", "expression"))?,
            graph,
        )?;
        let asymptotic = bool_field(&reader, "ComplexityModel", "asymptotic")?;
        reader.check_known(&allowed(&[
            prop("ComplexityModel", "subject"),
            prop("ComplexityModel", "kind"),
            prop("ComplexityModel", "variables"),
            prop("ComplexityModel", "expression"),
            prop("ComplexityModel", "asymptotic"),
        ]))?;
        Ok(Self::new(kind, variables, expression, asymptotic).for_subject(subject))
    }
}

fn term_to_rdf(ctx: &mut RdfContext, term: &Term) -> Result<rdf_codec::NamedNode, CodecError> {
    let node = ctx.instance();
    match term {
        Term::Variable(name) => {
            ctx.emit_type(&node, "TermNode/Variable");
            ctx.emit_literal(
                &node,
                "TermNode",
                "value",
                Prim::Text(name.clone()).to_literal()?,
            );
        }
        Term::Constant(value) => {
            ctx.emit_type(&node, "TermNode/Constant");
            ctx.emit_literal(
                &node,
                "TermNode",
                "value",
                Prim::Text(value.clone()).to_literal()?,
            );
        }
        Term::Arithmetic(expression) => {
            ctx.emit_type(&node, "TermNode/Arithmetic");
            let expression = expression.to_rdf(ctx)?;
            ctx.emit_object(&node, "TermNode", "expression", &expression);
        }
        Term::Application {
            function,
            arguments,
        } => {
            ctx.emit_type(&node, "TermNode/Application");
            ctx.emit_literal(
                &node,
                "TermNode",
                "function",
                Prim::Text(function.clone()).to_literal()?,
            );
            let items = arguments
                .iter()
                .map(|argument| term_to_rdf(ctx, argument).map(rdf_codec::Term::from))
                .collect::<Result<Vec<_>, _>>()?;
            let list = ctx.list(&items)?;
            ctx.emit_object(&node, "TermNode", "arguments", &list);
        }
        other => {
            return Err(CodecError::Unsupported(format!(
                "complexity operand {other:?}"
            )))
        }
    }
    Ok(node)
}

fn term_from_rdf(
    node: &rdf_codec::NamedNode,
    graph: &rdf_codec::Graph,
) -> Result<Term, CodecError> {
    let reader = open(graph, node);
    let type_name = reader
        .type_local()
        .ok_or_else(|| CodecError::Decode(format!("`{node}` has no TermNode type")))?;
    let term = match type_name.as_str() {
        "TermNode/Variable" | "TermNode/Constant" => {
            let value = string_field(&reader, "TermNode", "value")?;
            if type_name.ends_with("/Variable") {
                Term::Variable(value)
            } else {
                Term::Constant(value)
            }
        }
        "TermNode/Arithmetic" => Term::Arithmetic(ArithmeticExpr::from_rdf(
            &reader.object(&prop("TermNode", "expression"))?,
            graph,
        )?),
        "TermNode/Application" => {
            let function = string_field(&reader, "TermNode", "function")?;
            let arguments = reader
                .list_objects(&prop("TermNode", "arguments"))?
                .into_iter()
                .map(|head| term_from_rdf(&head, graph))
                .collect::<Result<Vec<_>, _>>()?;
            Term::Application {
                function,
                arguments,
            }
        }
        other => {
            return Err(CodecError::Decode(format!(
                "unsupported TermNode type `{other}"
            )))
        }
    };
    reader.check_known(&allowed(&[
        prop("TermNode", "value"),
        prop("TermNode", "expression"),
        prop("TermNode", "function"),
        prop("TermNode", "arguments"),
    ]))?;
    Ok(term)
}

impl ToRdf for ComplexityExpr {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<rdf_codec::NamedNode, CodecError> {
        let node = ctx.instance();
        let (kind, subject, model) = match self {
            Self::TimeComplexity { subject, model } => ("TimeComplexity", subject, model),
            Self::SpaceComplexity { subject, model } => ("SpaceComplexity", subject, model),
            Self::CommunicationComplexity { subject, model } => {
                ("CommunicationComplexity", subject, model)
            }
            Self::EnergyComplexity { subject, model } => ("EnergyComplexity", subject, model),
            Self::InClass { algorithm, class } => {
                ctx.emit_type(&node, "Complexity/InClass");
                let algorithm_node = term_to_rdf(ctx, algorithm)?;
                let class_node = term_to_rdf(ctx, class)?;
                ctx.emit_object(&node, "Complexity", "algorithm", &algorithm_node);
                ctx.emit_object(&node, "Complexity", "class", &class_node);
                return Ok(node);
            }
            Self::Feasible {
                algorithm,
                size,
                budget,
            } => {
                ctx.emit_type(&node, "Complexity/Feasible");
                let algorithm_node = term_to_rdf(ctx, algorithm)?;
                let size_node = term_to_rdf(ctx, size)?;
                let budget_node = term_to_rdf(ctx, budget)?;
                ctx.emit_object(&node, "Complexity", "algorithm", &algorithm_node);
                ctx.emit_object(&node, "Complexity", "size", &size_node);
                ctx.emit_object(&node, "Complexity", "budget", &budget_node);
                return Ok(node);
            }
        };
        ctx.emit_type(&node, &format!("Complexity/{kind}"));
        let subject_node = term_to_rdf(ctx, subject)?;
        let model_node = model.to_rdf(ctx)?;
        ctx.emit_object(&node, "Complexity", "subject", &subject_node);
        ctx.emit_object(&node, "Complexity", "model", &model_node);
        Ok(node)
    }
}

impl FromRdf for ComplexityExpr {
    fn from_rdf(node: &rdf_codec::NamedNode, graph: &rdf_codec::Graph) -> Result<Self, CodecError> {
        let reader = open(graph, node);
        let type_name = reader
            .type_local()
            .ok_or_else(|| CodecError::Decode(format!("`{node}` has no Complexity type")))?;
        let result = match type_name.as_str() {
            "Complexity/InClass" => Self::InClass {
                algorithm: term_from_rdf(&reader.object(&prop("Complexity", "algorithm"))?, graph)?,
                class: term_from_rdf(&reader.object(&prop("Complexity", "class"))?, graph)?,
            },
            "Complexity/Feasible" => Self::Feasible {
                algorithm: term_from_rdf(&reader.object(&prop("Complexity", "algorithm"))?, graph)?,
                size: term_from_rdf(&reader.object(&prop("Complexity", "size"))?, graph)?,
                budget: term_from_rdf(&reader.object(&prop("Complexity", "budget"))?, graph)?,
            },
            "Complexity/TimeComplexity"
            | "Complexity/SpaceComplexity"
            | "Complexity/CommunicationComplexity"
            | "Complexity/EnergyComplexity" => {
                let subject =
                    term_from_rdf(&reader.object(&prop("Complexity", "subject"))?, graph)?;
                let model = ComplexityModel::from_rdf(
                    &reader.object(&prop("Complexity", "model"))?,
                    graph,
                )?;
                match type_name.as_str() {
                    "Complexity/TimeComplexity" => Self::TimeComplexity { subject, model },
                    "Complexity/SpaceComplexity" => Self::SpaceComplexity { subject, model },
                    "Complexity/CommunicationComplexity" => {
                        Self::CommunicationComplexity { subject, model }
                    }
                    _ => Self::EnergyComplexity { subject, model },
                }
            }
            other => {
                return Err(CodecError::Decode(format!(
                    "unsupported Complexity type `{other}"
                )))
            }
        };
        reader.check_known(&allowed(&[
            prop("Complexity", "algorithm"),
            prop("Complexity", "class"),
            prop("Complexity", "size"),
            prop("Complexity", "budget"),
            prop("Complexity", "subject"),
            prop("Complexity", "model"),
        ]))?;
        Ok(result)
    }
}
