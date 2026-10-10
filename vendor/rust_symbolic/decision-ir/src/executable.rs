//! Executable computation lowering (`engine_ir_execution_plan.md` Phase 2):
//! the decision problem's expected utilities lower to *executable* equation
//! statements the Numerical target can genuinely solve, alongside the
//! descriptive `Extension::Decision` statements: one `eu_<action> = Σ p·u −
//! cost` equation per action, with the lottery arithmetic written out over
//! the action's outcome probabilities and utilities.
//!
//! The IR lane therefore computes each action's expected utility from its
//! lottery — an independent numerical path from the engine's closed form —
//! and the cross-check fires on the `eu_<action>` propositions.

use logic_ir::{
    ArithmeticExpr, BaseLogic, Equation, EquationKind, ExtensionKind, LogicModel,
    NegationSemantics, SemanticProfile, Signature, Statement, Term, VarDecl,
};
use logic_types::Type;

use crate::decision::DecisionProblem;

/// The canonical proposition name for an action's expected utility — matches
/// the equation LHS variable name exactly.
pub fn eu_proposition(action: &str) -> String {
    format!("eu_{action}")
}

/// Lower the decision problem's executable surface: the expected-utility
/// equations for every action.
pub fn to_executable_model(problem: &DecisionProblem) -> Result<LogicModel, String> {
    let mut statements = Vec::new();
    let mut variables: Vec<VarDecl> = Vec::new();

    for action in &problem.actions {
        let mut terms = Vec::new();
        for outcome in &action.outcomes {
            // p · u — every factor is a constant, so the fragment stays
            // linear and the system acyclic.
            terms.push(ArithmeticExpr::mul(vec![
                ArithmeticExpr::constant(outcome.probability.to_string()),
                ArithmeticExpr::constant(outcome.utility.to_string()),
            ]));
        }
        let gross = ArithmeticExpr::add(terms);
        let eu = if action.cost == 0.0 {
            gross
        } else {
            ArithmeticExpr::subtract(gross, ArithmeticExpr::constant(action.cost.to_string()))
        };
        let name = eu_proposition(&action.name);
        variables.push(VarDecl {
            name: name.clone(),
            ty: Type::Real,
        });
        statements.push(Statement::Equation(Equation {
            kind: EquationKind::Definition,
            lhs: Term::variable(&name),
            rhs: Term::arithmetic(eu),
        }));
    }

    Ok(LogicModel {
        signature: Signature {
            entities: Vec::new(),
            variables,
            functions: Vec::new(),
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
