//! The model tuple `M = (Σ, Γ, 𝒯, 𝒮, Q)` (`full_system_design.md` §7.6).

use logic_types::Type;

use crate::formula::Formula;
use crate::probabilistic::EventExpression;
use crate::statement::{Fact, Rule, Statement};
use crate::term::Term;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BaseLogic {
    Propositional,
    FirstOrder,
    HigherOrder,
    DescriptionLogic,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Theory {
    RealArithmetic,
    IntegerArithmetic,
    RationalArithmetic,
    Equality,
    Sets,
    Units,
    DifferentialEquations,
    ProbabilityDistributions,
    Geometry,
    Graphs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionKind {
    Modal,
    Temporal,
    Probabilistic,
    Fuzzy,
    Default,
    Causal,
    Deontic,
    Planning,
    Optimization,
    Game,
    Spatial,
    Event,
    Abstraction,
    Information,
    Complexity,
    Decision,
    Epistemic,
    Tom,
    Mechanism,
    EpistemicMarket,
    Analogy,
    Evolution,
    Learning,
    Memory,
    SymReg,
    Synthesis,
    Argumentation,
    Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NegationSemantics {
    Classical,
    Default,
    StableModel,
    Weak,
}

/// The declared semantics of a model (`full_system_design.md` §8).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticProfile {
    pub base: BaseLogic,
    pub theories: Vec<Theory>,
    pub extensions: Vec<ExtensionKind>,
    pub closed_world: bool,
    pub monotonic: bool,
    pub negation: NegationSemantics,
}

/// A typed variable/function/relation declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VarDecl {
    pub name: String,
    pub ty: Type,
}

/// The signature `Σ`: entity types, variables, functions, relations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Signature {
    pub entities: Vec<String>,
    pub variables: Vec<VarDecl>,
    pub functions: Vec<VarDecl>,
    pub relations: Vec<VarDecl>,
}

/// A query/objective `Q`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Query {
    Prove(Box<Formula>),
    Refute(Box<Formula>),
    Probability {
        proposition: Box<Formula>,
    },
    Explain {
        observation: Box<Formula>,
    },
    ArgMax(Term),
    ArgMin(Term),
    /// The complete ordered distribution of a categorical variable
    /// (`categorical_sem_design.md` §4.2). The result is a distribution
    /// object, not a Boolean truth value.
    CategoricalDistribution {
        variable: String,
    },
    /// One categorical state's marginal probability (§4.3).
    CategoricalState {
        variable: String,
        state: String,
    },
    /// A compound event probability over typed selectors (§4.4).
    Event(EventExpression),
    /// Conditional probability `P(proposition | given)` (§4.5). `given` is
    /// the evidence list the backend must condition on.
    Conditional {
        proposition: Box<Formula>,
        given: Vec<Formula>,
    },
    /// MPE for the supported ground fragment (§4.6).
    MostProbableExplanation {
        proposition: Box<Formula>,
    },
    Outcome(crate::probabilistic::OutcomeQuery),
}

/// Normalized executable Logic program. All authoring surfaces (structured
/// SEM, compatibility clause text, and persisted RDF) lower into this model
/// before execution.
#[derive(Debug, Clone, PartialEq)]
pub struct LogicProgram {
    pub signature: Signature,
    pub facts: Vec<Fact>,
    pub rules: Vec<Rule>,
    pub constraints: Vec<Formula>,
    pub queries: Vec<Query>,
}

impl LogicProgram {
    /// All known constants that can be used as finite-domain candidates for
    /// variable queries and Boolean model finding.
    pub fn domain(&self) -> Vec<String> {
        let mut domain = std::collections::BTreeSet::new();
        for fact in &self.facts {
            collect_atom_constants(&fact.atom, &mut domain);
        }
        for rule in &self.rules {
            collect_atom_constants(&rule.head, &mut domain);
            for formula in &rule.body {
                collect_formula_constants(formula, &mut domain);
            }
        }
        for formula in &self.constraints {
            collect_formula_constants(formula, &mut domain);
        }
        for query in &self.queries {
            collect_query_constants(query, &mut domain);
        }
        domain.into_iter().collect()
    }

    /// Return the maximum observed arity for every predicate, including
    /// predicates nested under formulas and queries.
    pub fn predicate_arities(&self) -> std::collections::BTreeMap<String, usize> {
        let mut arities = std::collections::BTreeMap::new();
        for fact in &self.facts {
            collect_atom_arities(&fact.atom, &mut arities);
        }
        for rule in &self.rules {
            collect_atom_arities(&rule.head, &mut arities);
            for formula in &rule.body {
                collect_formula_arities(formula, &mut arities);
            }
        }
        for formula in &self.constraints {
            collect_formula_arities(formula, &mut arities);
        }
        for query in &self.queries {
            collect_query_arities(query, &mut arities);
        }
        arities
    }
}

fn collect_term_constants(term: &Term, domain: &mut std::collections::BTreeSet<String>) {
    match term {
        Term::Constant(value) => {
            domain.insert(value.clone());
        }
        Term::Application { arguments, .. } => {
            for argument in arguments {
                collect_term_constants(argument, domain);
            }
        }
        Term::Arithmetic(_)
        | Term::Variable(_)
        | Term::Lambda { .. }
        | Term::If { .. }
        | Term::Let { .. } => {}
    }
}

fn collect_atom_constants(atom: &crate::Atom, domain: &mut std::collections::BTreeSet<String>) {
    if let crate::Atom::Predicate { arguments, .. } = atom {
        for argument in arguments {
            collect_term_constants(argument, domain);
        }
    }
}

fn collect_formula_constants(formula: &Formula, domain: &mut std::collections::BTreeSet<String>) {
    match formula {
        Formula::Atom(atom) => collect_atom_constants(atom, domain),
        Formula::Not(inner) => collect_formula_constants(inner, domain),
        Formula::And(xs) | Formula::Or(xs) => {
            for x in xs {
                collect_formula_constants(x, domain);
            }
        }
        Formula::Xor(a, b) | Formula::Implies(a, b) | Formula::Iff(a, b) => {
            collect_formula_constants(a, domain);
            collect_formula_constants(b, domain);
        }
        Formula::ForAll { body, .. } | Formula::Exists { body, .. } => {
            collect_formula_constants(body, domain)
        }
        _ => {}
    }
}

fn collect_query_constants(query: &Query, domain: &mut std::collections::BTreeSet<String>) {
    match query {
        Query::Prove(f)
        | Query::Refute(f)
        | Query::Probability { proposition: f }
        | Query::Explain { observation: f }
        | Query::MostProbableExplanation { proposition: f } => collect_formula_constants(f, domain),
        Query::Conditional {
            proposition: f,
            given,
        } => {
            collect_formula_constants(f, domain);
            for g in given {
                collect_formula_constants(g, domain);
            }
        }
        Query::Outcome(_) => {}
        Query::CategoricalDistribution { variable } => {
            domain.insert(variable.clone());
        }
        Query::CategoricalState { variable, state } => {
            domain.insert(variable.clone());
            domain.insert(state.clone());
        }
        Query::Event(event) => collect_event_constants(event, domain),
        Query::ArgMax(t) | Query::ArgMin(t) => collect_term_constants(t, domain),
    }
}

fn collect_event_constants(
    event: &EventExpression,
    domain: &mut std::collections::BTreeSet<String>,
) {
    match event {
        EventExpression::And(xs) | EventExpression::Or(xs) => {
            for x in xs {
                collect_event_constants(x, domain);
            }
        }
        EventExpression::Not(inner) => collect_event_constants(inner, domain),
        EventExpression::Trial(t) => {
            domain.insert(t.family.clone());
            domain.insert(t.value.clone());
            domain.insert(t.index.to_string());
        }
        EventExpression::State(s) => {
            domain.insert(s.variable.clone());
            domain.insert(s.state.clone());
        }
    }
}

fn collect_atom_arities(
    atom: &crate::Atom,
    arities: &mut std::collections::BTreeMap<String, usize>,
) {
    if let crate::Atom::Predicate {
        predicate,
        arguments,
    } = atom
    {
        let entry = arities.entry(predicate.clone()).or_insert(0);
        *entry = (*entry).max(arguments.len());
    }
}

fn collect_formula_arities(
    formula: &Formula,
    arities: &mut std::collections::BTreeMap<String, usize>,
) {
    match formula {
        Formula::Atom(atom) => collect_atom_arities(atom, arities),
        Formula::Not(inner) => collect_formula_arities(inner, arities),
        Formula::And(xs) | Formula::Or(xs) => {
            for x in xs {
                collect_formula_arities(x, arities);
            }
        }
        Formula::Xor(a, b) | Formula::Implies(a, b) | Formula::Iff(a, b) => {
            collect_formula_arities(a, arities);
            collect_formula_arities(b, arities);
        }
        Formula::ForAll { body, .. } | Formula::Exists { body, .. } => {
            collect_formula_arities(body, arities)
        }
        _ => {}
    }
}

fn collect_query_arities(query: &Query, arities: &mut std::collections::BTreeMap<String, usize>) {
    match query {
        Query::Prove(f)
        | Query::Refute(f)
        | Query::Probability { proposition: f }
        | Query::Explain { observation: f }
        | Query::MostProbableExplanation { proposition: f } => collect_formula_arities(f, arities),
        Query::Conditional {
            proposition: f,
            given,
        } => {
            collect_formula_arities(f, arities);
            for g in given {
                collect_formula_arities(g, arities);
            }
        }
        Query::CategoricalDistribution { .. }
        | Query::CategoricalState { .. }
        | Query::Event(_)
        | Query::Outcome(_) => {}
        Query::ArgMax(_) | Query::ArgMin(_) => {}
    }
}

impl LogicProgram {
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
            && self.rules.is_empty()
            && self.constraints.is_empty()
            && self.queries.is_empty()
    }
}

/// The complete model: signature + statements + theories + semantics + queries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicModel {
    pub signature: Signature,
    pub theories: Vec<Theory>,
    pub semantics: SemanticProfile,
    pub declarations: Vec<String>,
    pub statements: Vec<Statement>,
    pub queries: Vec<Query>,
}
