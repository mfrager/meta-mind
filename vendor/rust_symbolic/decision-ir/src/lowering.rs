//! Semantic-IR lowering (`ir_wiring_implementation_plan.md` Phase 3): the
//! decision problem lowers to `Extension::Decision` statements (pairwise
//! preferences) plus `Constraint` statements (each action's cost bound as a
//! `cost(action) <= max_cost` comparison), so the decision's preference and
//! feasibility surface is a decomposable, VERIFY-passing `LogicModel`.

use logic_ir::{
    Atom, BaseLogic, CompareOp, DecisionExpr, Extension, ExtensionKind, Formula, LogicModel,
    NegationSemantics, SemanticProfile, Signature, Statement, Term, VarDecl,
};
use logic_types::Type;

use crate::decision::DecisionProblem;

fn constant(name: &str) -> Term {
    Term::Constant(name.to_string())
}

/// Lower a decision problem into a `LogicModel`.
pub fn to_logic_model(problem: &DecisionProblem) -> Result<LogicModel, String> {
    let mut statements = Vec::new();
    let mut functions = Vec::new();

    // Constraints reference the per-action cost via a declared function
    // (action → real). `Term::Constant` typechecks as `Real`, so the domain
    // is `Real`.
    if !problem.constraints.is_empty() {
        functions.push(VarDecl {
            name: "cost".to_string(),
            ty: Type::Function {
                args: vec![Type::Real],
                result: Box::new(Type::Real),
            },
        });
    }

    for preference in &problem.preferences {
        statements.push(Statement::Formula(Formula::Extension(Extension::Decision(
            DecisionExpr::Prefers {
                preferred: constant(&preference.preferred),
                over: constant(&preference.over),
            },
        ))));
    }

    for constraint in &problem.constraints {
        statements.push(Statement::Constraint(Formula::Atom(Atom::Comparison {
            op: CompareOp::Le,
            left: Term::Application {
                function: "cost".to_string(),
                arguments: vec![constant(&constraint.action)],
            },
            right: constant(&constraint.max_cost.to_string()),
        })));
    }

    Ok(LogicModel {
        signature: Signature {
            entities: Vec::new(),
            variables: Vec::new(),
            functions,
            relations: Vec::new(),
        },
        theories: Vec::new(),
        semantics: SemanticProfile {
            base: BaseLogic::FirstOrder,
            theories: Vec::new(),
            extensions: vec![ExtensionKind::Decision],
            closed_world: false,
            monotonic: true,
            negation: NegationSemantics::Classical,
        },
        declarations: Vec::new(),
        statements,
        queries: Vec::new(),
    })
}
