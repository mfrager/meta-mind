//! Namespace and core vocabulary constants (master_design.md §5), reorganized
//! to the coherent scheme of `planning/rdf_interface_update_plan.md` §4:
//! one base (`https://example.org/ns/`) for all schema namespaces, with
//! `model:` for the model layer and `units:` for units/dimensions.

pub const MODEL_NS: &str = "https://example.org/ns/model#";
pub const UNITS_NS: &str = "https://example.org/ns/units#";
pub const LOGIC_NS: &str = "https://example.org/ns/logic#";
pub const NUMX_NS: &str = "https://example.org/ns/numerical#";
pub const SOLVER_NS: &str = "https://example.org/ns/solver#";
pub const PROB_NS: &str = "https://example.org/ns/probabilistic#";
pub const CAUS_NS: &str = "https://example.org/ns/causal#";
pub const TENS_NS: &str = "https://example.org/ns/tensor#";
pub const TEMP_NS: &str = "https://example.org/ns/temporal#";
pub const EVENT_NS: &str = "https://example.org/ns/event#";
pub const ABS_NS: &str = "https://example.org/ns/abstraction#";
pub const INFO_NS: &str = "https://example.org/ns/info#";
pub const COMPLEXITY_NS: &str = "https://example.org/ns/complexity#";
pub const DECISION_NS: &str = "https://example.org/ns/decision#";
pub const EPISTEMIC_NS: &str = "https://example.org/ns/epistemic#";
pub const TOM_NS: &str = "https://example.org/ns/tom#";
pub const MECHANISM_NS: &str = "https://example.org/ns/mechanism#";
pub const MARKET_NS: &str = "https://example.org/ns/epistemicmarket#";
pub const LEARNING_NS: &str = "https://example.org/ns/learning#";
pub const EVOLUTION_NS: &str = "https://example.org/ns/evolution#";
pub const ANALOGY_NS: &str = "https://example.org/ns/analogy#";
pub const SYMREG_NS: &str = "https://example.org/ns/symreg#";
pub const SYNTHESIS_NS: &str = "https://example.org/ns/synthesis#";
pub const MEMORY_NS: &str = "https://example.org/ns/memory#";
/// The shared cross-subsystem resource vocabulary (Phase 2): one canonical
/// namespace for resource vectors, budgets, reservations, and capabilities.
pub const RESOURCE_NS: &str = "https://example.org/ns/resource#";
/// The shared information–cost economy vocabulary (Phase 8): one canonical
/// namespace for objectives, cost projections, candidates, and decisions.
pub const ECONOMY_NS: &str = "https://example.org/ns/economy#";
pub const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
pub const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
pub const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";

/// Build a model-namespace IRI for a local name.
pub fn model_iri(local: &str) -> String {
    format!("{MODEL_NS}{local}")
}

// --- classes ---
pub const CLASS_MODEL: &str = "https://example.org/ns/model#Model";
pub const CLASS_VARIABLE: &str = "https://example.org/ns/model#Variable";
pub const CLASS_PARAMETER: &str = "https://example.org/ns/model#Parameter";
pub const CLASS_CONSTANT: &str = "https://example.org/ns/model#Constant";
pub const CLASS_EQUATION: &str = "https://example.org/ns/model#Equation";
pub const CLASS_EXPRESSION: &str = "https://example.org/ns/model#Expression";
pub const CLASS_ADDITION: &str = "https://example.org/ns/model#Addition";
pub const CLASS_SUBTRACTION: &str = "https://example.org/ns/model#Subtraction";
pub const CLASS_MULTIPLICATION: &str = "https://example.org/ns/model#Multiplication";
pub const CLASS_DIVISION: &str = "https://example.org/ns/model#Division";
pub const CLASS_POWER: &str = "https://example.org/ns/model#Power";
pub const CLASS_NEGATION: &str = "https://example.org/ns/model#Negation";
pub const CLASS_SIN: &str = "https://example.org/ns/model#Sin";
pub const CLASS_COS: &str = "https://example.org/ns/model#Cos";
pub const CLASS_EXP: &str = "https://example.org/ns/model#Exp";
pub const CLASS_LOG: &str = "https://example.org/ns/model#Log";
pub const CLASS_INDEX: &str = "https://example.org/ns/model#Index";
pub const CLASS_LAG: &str = "https://example.org/ns/model#Lag";
pub const CLASS_LEAD: &str = "https://example.org/ns/model#Lead";
pub const CLASS_INDEX_DIMENSION: &str = "https://example.org/ns/model#IndexDimension";

// --- equation kinds (master_design.md §5.4) ---
pub const CLASS_IDENTITY: &str = "https://example.org/ns/model#Identity";
pub const CLASS_DEFINITION: &str = "https://example.org/ns/model#Definition";
pub const CLASS_BEHAVIORAL_EQUATION: &str = "https://example.org/ns/model#BehavioralEquation";
pub const CLASS_CONSTRAINT: &str = "https://example.org/ns/model#Constraint";
pub const CLASS_OBJECTIVE: &str = "https://example.org/ns/model#Objective";
pub const CLASS_ACCOUNTING_IDENTITY: &str = "https://example.org/ns/model#AccountingIdentity";
pub const CLASS_DIFFERENTIAL: &str = "https://example.org/ns/model#Differential";
pub const CLASS_INEQUALITY: &str = "https://example.org/ns/model#Inequality";
pub const CLASS_OBSERVATION: &str = "https://example.org/ns/model#Observation";
pub const CLASS_DATASET: &str = "https://example.org/ns/model#Dataset";
pub const CLASS_TABULAR_BINDING: &str = "https://example.org/ns/model#TabularBinding";
pub const CLASS_VARIABLE_BINDING: &str = "https://example.org/ns/model#VariableBinding";
pub const CLASS_INDEX_BINDING: &str = "https://example.org/ns/model#IndexBinding";

/// The authoritative class list of the model layer (bidirectional consistency
/// with the per-class ontology files, rdf_interface_update_plan.md §9.2).
pub const MODEL_CLASSES: &[&str] = &[
    CLASS_MODEL,
    CLASS_VARIABLE,
    CLASS_PARAMETER,
    CLASS_CONSTANT,
    CLASS_EQUATION,
    CLASS_EXPRESSION,
    CLASS_ADDITION,
    CLASS_SUBTRACTION,
    CLASS_MULTIPLICATION,
    CLASS_DIVISION,
    CLASS_POWER,
    CLASS_NEGATION,
    CLASS_SIN,
    CLASS_COS,
    CLASS_EXP,
    CLASS_LOG,
    CLASS_INDEX,
    CLASS_LAG,
    CLASS_LEAD,
    CLASS_INDEX_DIMENSION,
    CLASS_IDENTITY,
    CLASS_DEFINITION,
    CLASS_BEHAVIORAL_EQUATION,
    CLASS_CONSTRAINT,
    CLASS_OBJECTIVE,
    CLASS_ACCOUNTING_IDENTITY,
    CLASS_DIFFERENTIAL,
    CLASS_INEQUALITY,
    CLASS_OBSERVATION,
    CLASS_DATASET,
    CLASS_TABULAR_BINDING,
    CLASS_VARIABLE_BINDING,
    CLASS_INDEX_BINDING,
    CLASS_DATA_SOURCE,
    CLASS_DATA_SOURCE_KIND,
    CLASS_SOURCE_COLUMN,
    CLASS_SOURCE_CELL,
    CLASS_SOURCE_LOAD,
    CLASS_PARAMETER_BINDING,
    CLASS_PARAMETER_AGGREGATE,
];

pub const CLASS_DATA_SOURCE: &str = "https://example.org/ns/model#DataSource";
pub const CLASS_DATA_SOURCE_KIND: &str = "https://example.org/ns/model#DataSourceKind";
pub const CLASS_SOURCE_COLUMN: &str = "https://example.org/ns/model#SourceColumn";
pub const CLASS_SOURCE_CELL: &str = "https://example.org/ns/model#SourceCell";
pub const CLASS_SOURCE_LOAD: &str = "https://example.org/ns/model#SourceLoad";
pub const CLASS_PARAMETER_BINDING: &str = "https://example.org/ns/model#ParameterBinding";
pub const CLASS_PARAMETER_AGGREGATE: &str = "https://example.org/ns/model#ParameterAggregate";

// --- predicates ---
pub const PRED_HAS_VARIABLE: &str = "https://example.org/ns/model#hasVariable";
pub const PRED_HAS_PARAMETER: &str = "https://example.org/ns/model#hasParameter";
pub const PRED_HAS_EQUATION: &str = "https://example.org/ns/model#hasEquation";
pub const PRED_LHS: &str = "https://example.org/ns/model#lhs";
pub const PRED_RHS: &str = "https://example.org/ns/model#rhs";
pub const PRED_OPERANDS: &str = "https://example.org/ns/model#operands";
pub const PRED_OPERAND: &str = "https://example.org/ns/model#operand"; // legacy
pub const PRED_LEFT: &str = "https://example.org/ns/model#left"; // legacy
pub const PRED_RIGHT: &str = "https://example.org/ns/model#right"; // legacy
pub const PRED_ARGUMENT: &str = "https://example.org/ns/model#argument";
pub const PRED_BASE: &str = "https://example.org/ns/model#base";
pub const PRED_EXPONENT: &str = "https://example.org/ns/model#exponent";
pub const PRED_VALUE: &str = "https://example.org/ns/model#value";
pub const PRED_NAME: &str = "https://example.org/ns/model#name";
pub const PRED_SYMBOL: &str = "https://example.org/ns/model#symbol";
pub const PRED_LATEX_SYMBOL: &str = "https://example.org/ns/model#latexSymbol";
pub const PRED_INDEX_OF: &str = "https://example.org/ns/model#indexOf";
pub const PRED_VARIABLE_OF: &str = "https://example.org/ns/model#variable";
pub const PRED_INDEX_DIMENSION: &str = "https://example.org/ns/model#indexDimension";
pub const PRED_PERIOD: &str = "https://example.org/ns/model#period";
pub const PRED_OFFSET: &str = "https://example.org/ns/model#offset";
pub const PRED_HAS_DATASET: &str = "https://example.org/ns/model#hasDataset";
pub const PRED_BINDS: &str = "https://example.org/ns/model#binds";
pub const PRED_COLUMN: &str = "https://example.org/ns/model#column";
pub const PRED_COLUMN_EXPRESSION: &str = "https://example.org/ns/model#columnExpression";
pub const PRED_INDEX_COLUMN: &str = "https://example.org/ns/model#indexColumn";
pub const PRED_KEY_FORMAT: &str = "https://example.org/ns/model#keyFormat";
pub const PRED_ROW_FILTER: &str = "https://example.org/ns/model#rowFilter";
pub const PRED_BINDING_OF: &str = "https://example.org/ns/model#binding";
pub const PRED_PREFERRED_NOTATION: &str = "https://example.org/ns/model#preferredNotation";
pub const PRED_ALTERNATE_NOTATION: &str = "https://example.org/ns/model#alternateNotation";
pub const PRED_HAS_DOMAIN: &str = "https://example.org/ns/model#hasDomain";
pub const PRED_OF_SOURCE: &str = "https://example.org/ns/model#ofSource";
pub const PRED_ROW_COUNT: &str = "https://example.org/ns/model#rowCount";
pub const PRED_COLUMNS: &str = "https://example.org/ns/model#columns";
pub const PRED_USES: &str = "https://example.org/ns/model#uses";
pub const PRED_KIND: &str = "https://example.org/ns/model#kind";
pub const PRED_LOCATION: &str = "https://example.org/ns/model#location";
pub const PRED_FORMAT: &str = "https://example.org/ns/model#format";
pub const PRED_CHECKSUM: &str = "https://example.org/ns/model#checksum";
pub const PRED_DATATYPE: &str = "https://example.org/ns/model#datatype";
pub const PRED_ORDINAL: &str = "https://example.org/ns/model#ordinal";
pub const PRED_ROW: &str = "https://example.org/ns/model#row";
pub const PRED_BACKEND: &str = "https://example.org/ns/model#backend";
pub const PRED_LOADED_AT: &str = "https://example.org/ns/model#loadedAt";
pub const PRED_AGGREGATE: &str = "https://example.org/ns/model#aggregate";
pub const PRED_TABLE: &str = "https://example.org/ns/model#table";
pub const PRED_HAS_ASSUMPTION: &str = "https://example.org/ns/model#hasAssumption";
pub const PRED_LAG: &str = "https://example.org/ns/model#lag";
pub const PRED_LEAD: &str = "https://example.org/ns/model#lead";

// --- optimization / dynamics surface (C4: LP/MILP/ODE/DAE from `model.ttl`) ---
// An `model:Objective` node carries `model:rhs` (the objective expression) and
// `model:sense` ("minimize" | "maximize"). A `model:Inequality` constraint
// carries `model:lhs`/`model:rhs` and `model:operator` ("le" | "ge" | "eq").
// A variable is integer when it carries `model:integer`. A
// `model:Differential` node carries `model:variable` (the state), `model:rhs`
// (the derivative / residual), `model:t0`, `model:tEnd`, and `model:initial`
// (one literal per state's initial value).
pub const PRED_HAS_OBJECTIVE: &str = "https://example.org/ns/model#hasObjective";
pub const PRED_HAS_CONSTRAINT: &str = "https://example.org/ns/model#hasConstraint";
pub const PRED_SENSE: &str = "https://example.org/ns/model#sense";
pub const PRED_OPERATOR: &str = "https://example.org/ns/model#operator";
pub const PRED_INTEGER: &str = "https://example.org/ns/model#integer";
pub const PRED_T0: &str = "https://example.org/ns/model#t0";
pub const PRED_T_END: &str = "https://example.org/ns/model#tEnd";
pub const PRED_DT: &str = "https://example.org/ns/model#dt";
pub const PRED_INITIAL: &str = "https://example.org/ns/model#initial";
pub const PRED_STATE: &str = "https://example.org/ns/model#state";
pub const PRED_INITIAL_DERIVATIVE: &str = "https://example.org/ns/model#initialDerivative";

/// The logic-layer rule predicate: the canonical rule text as a literal on
/// the model IRI (rdf_interface_update_plan.md §8.6, logic/properties.ttl).
pub const LOGIC_RULE: &str = "https://example.org/ns/logic#rule";

/// The logic-layer causal-statement class and its symbol-name predicates
/// (logic/CausalStatement.ttl + logic/properties.ttl).
pub const CLASS_CAUSAL_STATEMENT: &str = "https://example.org/ns/logic#CausalStatement";
pub const PRED_CAUSE: &str = "https://example.org/ns/logic#cause";
pub const PRED_EFFECT: &str = "https://example.org/ns/logic#effect";

/// The logic-layer probabilistic-annotation class and its predicates
/// (logic/ProbabilisticStatement.ttl + logic/properties.ttl).
pub const CLASS_PROBABILISTIC_STATEMENT: &str =
    "https://example.org/ns/logic#ProbabilisticStatement";
pub const PRED_PROBABILITY: &str = "https://example.org/ns/logic#probability";
pub const PRED_PROPOSITION: &str = "https://example.org/ns/logic#proposition";
pub const PRED_PROBABILITY_QUERY: &str = "https://example.org/ns/logic#probabilityQuery";

/// The authoritative class list of the Numerical IR layer (bidirectional
/// consistency with `numerical/*.ttl`).
pub const NUMX_CLASSES: &[&str] = &[
    "https://example.org/ns/numerical#Symbol",
    "https://example.org/ns/numerical#Number",
    "https://example.org/ns/numerical#Add",
    "https://example.org/ns/numerical#Sub",
    "https://example.org/ns/numerical#Mul",
    "https://example.org/ns/numerical#Div",
    "https://example.org/ns/numerical#Pow",
    "https://example.org/ns/numerical#Neg",
    "https://example.org/ns/numerical#Sin",
    "https://example.org/ns/numerical#Cos",
    "https://example.org/ns/numerical#Exp",
    "https://example.org/ns/numerical#Log",
    "https://example.org/ns/numerical#Lag",
];

/// The authoritative class-scoped property list of the Numerical IR layer
/// (`{Class}/{field}` form, consistent with the codec's `Namespace::property`).
pub const NUMX_PROPERTIES: &[&str] = &[
    "https://example.org/ns/numerical#Symbol/name",
    "https://example.org/ns/numerical#Number/value",
    "https://example.org/ns/numerical#Add/operands",
    "https://example.org/ns/numerical#Sub/operands",
    "https://example.org/ns/numerical#Mul/operands",
    "https://example.org/ns/numerical#Div/operands",
    "https://example.org/ns/numerical#Pow/base",
    "https://example.org/ns/numerical#Pow/exponent",
    "https://example.org/ns/numerical#Neg/operand",
    "https://example.org/ns/numerical#Sin/operand",
    "https://example.org/ns/numerical#Cos/operand",
    "https://example.org/ns/numerical#Exp/operand",
    "https://example.org/ns/numerical#Log/operand",
    "https://example.org/ns/numerical#Lag/series",
    "https://example.org/ns/numerical#Lag/offset",
];

/// The authoritative parent-class list of the SolverIR layer (directional
/// consistency with `solver/*.ttl`; variant subclasses live in parent files).
pub const SOLVER_CLASSES: &[&str] = &[
    "https://example.org/ns/solver#Literal",
    "https://example.org/ns/solver#Clause",
    "https://example.org/ns/solver#ClauseSet",
    "https://example.org/ns/solver#Payload",
    "https://example.org/ns/solver#CompiledProblem",
    "https://example.org/ns/solver#CompilationCertificate",
    "https://example.org/ns/solver#FragmentKind",
    "https://example.org/ns/solver#ResultStatus",
    "https://example.org/ns/solver#SmtSort",
    "https://example.org/ns/solver#SmtLogic",
    "https://example.org/ns/solver#SmtTerm",
    "https://example.org/ns/solver#SmtFormula",
    "https://example.org/ns/solver#SmtProblem",
    "https://example.org/ns/solver#FofTerm",
    "https://example.org/ns/solver#FofFormula",
    "https://example.org/ns/solver#TptpProblem",
    "https://example.org/ns/solver#Assignment",
    "https://example.org/ns/solver#BackendResult",
    "https://example.org/ns/solver#Domain",
    "https://example.org/ns/solver#Resources",
    "https://example.org/ns/solver#CapabilityDescriptor",
    "https://example.org/ns/solver#Encoding",
    "https://example.org/ns/solver#Exactness",
    "https://example.org/ns/solver#EquivalenceStatus",
];

/// The authoritative class list of the Probabilistic IR layer (directional
/// consistency with `probabilistic/*.ttl`; variant subclasses live in parent
/// files, and `State`/`Entry` cohabit with their user files).
pub const PROB_CLASSES: &[&str] = &[
    "https://example.org/ns/probabilistic#Probability",
    "https://example.org/ns/probabilistic#WeightedRule",
    "https://example.org/ns/probabilistic#Evidence",
    "https://example.org/ns/probabilistic#QueryKind",
    "https://example.org/ns/probabilistic#Query",
    "https://example.org/ns/probabilistic#ProbabilisticProgram",
    "https://example.org/ns/probabilistic#Interpretation",
    "https://example.org/ns/probabilistic#HybridResult",
    "https://example.org/ns/probabilistic#Entry",
    "https://example.org/ns/probabilistic#CategoricalVariable",
    "https://example.org/ns/probabilistic#State",
    "https://example.org/ns/probabilistic#QueryTarget",
    "https://example.org/ns/probabilistic#EventExpression",
    "https://example.org/ns/probabilistic#StateSelector",
    "https://example.org/ns/probabilistic#TrialSelector",
    "https://example.org/ns/probabilistic#TrialFamily",
];

/// The authoritative class-scoped property list of the Probabilistic IR layer.
pub const PROB_PROPERTIES: &[&str] = &[
    "https://example.org/ns/probabilistic#Probability/value",
    "https://example.org/ns/probabilistic#Probability/name",
    "https://example.org/ns/probabilistic#WeightedRule/head",
    "https://example.org/ns/probabilistic#WeightedRule/body",
    "https://example.org/ns/probabilistic#WeightedRule/probability",
    "https://example.org/ns/probabilistic#Evidence/proposition",
    "https://example.org/ns/probabilistic#Evidence/observed",
    "https://example.org/ns/probabilistic#Evidence/source",
    "https://example.org/ns/probabilistic#Query/kind",
    "https://example.org/ns/probabilistic#Query/proposition",
    "https://example.org/ns/probabilistic#Query/target",
    "https://example.org/ns/probabilistic#ProbabilisticProgram/rules",
    "https://example.org/ns/probabilistic#ProbabilisticProgram/categoricals",
    "https://example.org/ns/probabilistic#ProbabilisticProgram/trialFamilies",
    "https://example.org/ns/probabilistic#ProbabilisticProgram/evidence",
    "https://example.org/ns/probabilistic#ProbabilisticProgram/queries",
    "https://example.org/ns/probabilistic#ProbabilisticProgram/parameters",
    "https://example.org/ns/probabilistic#Interpretation/atoms",
    "https://example.org/ns/probabilistic#Interpretation/weight",
    "https://example.org/ns/probabilistic#HybridResult/numerical",
    "https://example.org/ns/probabilistic#HybridResult/probabilities",
    "https://example.org/ns/probabilistic#Entry/name",
    "https://example.org/ns/probabilistic#Entry/value",
    "https://example.org/ns/probabilistic#CategoricalVariable/name",
    "https://example.org/ns/probabilistic#CategoricalVariable/states",
    "https://example.org/ns/probabilistic#State/name",
    "https://example.org/ns/probabilistic#State/probability",
    "https://example.org/ns/probabilistic#QueryTarget/variable",
    "https://example.org/ns/probabilistic#QueryTarget/state",
    "https://example.org/ns/probabilistic#QueryTarget/event",
    "https://example.org/ns/probabilistic#QueryTarget/given",
    "https://example.org/ns/probabilistic#EventExpression/operands",
    "https://example.org/ns/probabilistic#EventExpression/state",
    "https://example.org/ns/probabilistic#EventExpression/trial",
    "https://example.org/ns/probabilistic#StateSelector/variable",
    "https://example.org/ns/probabilistic#StateSelector/state",
    "https://example.org/ns/probabilistic#TrialSelector/family",
    "https://example.org/ns/probabilistic#TrialSelector/index",
    "https://example.org/ns/probabilistic#TrialSelector/value",
    "https://example.org/ns/probabilistic#TrialFamily/name",
    "https://example.org/ns/probabilistic#TrialFamily/count",
    "https://example.org/ns/probabilistic#TrialFamily/success",
    "https://example.org/ns/probabilistic#TrialFamily/failure",
    "https://example.org/ns/probabilistic#TrialFamily/probability",
    "https://example.org/ns/probabilistic#TrialFamily/independent",
];

/// The authoritative class list of the Causal IR layer (bidirectional
/// consistency with `causal/*.ttl`; variant subclasses are declared in their
/// parent file).
pub const CAUS_CLASSES: &[&str] = &[
    "https://example.org/ns/causal#Scm",
    "https://example.org/ns/causal#ExogenousVar",
    "https://example.org/ns/causal#Outcome",
    "https://example.org/ns/causal#StructuralEquation",
    "https://example.org/ns/causal#CausalGraph",
    "https://example.org/ns/causal#Edge",
    "https://example.org/ns/causal#Intervention",
    "https://example.org/ns/causal#CausalPrimitive",
    "https://example.org/ns/causal#CausalQuery",
    "https://example.org/ns/causal#Assumption",
    "https://example.org/ns/causal#CounterfactualQuery",
    "https://example.org/ns/causal#CounterfactualResult",
    "https://example.org/ns/causal#ResultBinding",
    "https://example.org/ns/causal#Estimand",
    "https://example.org/ns/causal#Identification",
    "https://example.org/ns/causal#CausalProblem",
    "https://example.org/ns/causal#CausalReport",
    "https://example.org/ns/causal#EstimandResult",
    "https://example.org/ns/causal#QueryResult",
    "https://example.org/ns/causal#Violation",
];

/// The authoritative class-scoped property list of the Causal IR layer
/// (`{Class}/{field}` form, consistent with the codec's `Namespace::property`).
pub const CAUS_PROPERTIES: &[&str] = &[
    "https://example.org/ns/causal#Scm/variables",
    "https://example.org/ns/causal#Scm/exogenous",
    "https://example.org/ns/causal#Scm/equations",
    "https://example.org/ns/causal#Scm/graph",
    "https://example.org/ns/causal#ExogenousVar/name",
    "https://example.org/ns/causal#ExogenousVar/distribution",
    "https://example.org/ns/causal#Outcome/value",
    "https://example.org/ns/causal#Outcome/probability",
    "https://example.org/ns/causal#StructuralEquation/variable",
    "https://example.org/ns/causal#StructuralEquation/formula",
    "https://example.org/ns/causal#CausalGraph/nodes",
    "https://example.org/ns/causal#CausalGraph/edges",
    "https://example.org/ns/causal#Edge/parent",
    "https://example.org/ns/causal#Edge/child",
    "https://example.org/ns/causal#Intervention/variable",
    "https://example.org/ns/causal#Intervention/value",
    "https://example.org/ns/causal#CausalQuery/variable",
    "https://example.org/ns/causal#CausalQuery/value",
    "https://example.org/ns/causal#CausalQuery/intervention",
    "https://example.org/ns/causal#CausalQuery/assumptions",
    "https://example.org/ns/causal#Assumption/variable",
    "https://example.org/ns/causal#Assumption/value",
    "https://example.org/ns/causal#CounterfactualQuery/intervention",
    "https://example.org/ns/causal#CounterfactualQuery/outcome",
    "https://example.org/ns/causal#CounterfactualResult/actual",
    "https://example.org/ns/causal#CounterfactualResult/counterfactual",
    "https://example.org/ns/causal#CounterfactualResult/delta",
    "https://example.org/ns/causal#ResultBinding/variable",
    "https://example.org/ns/causal#ResultBinding/value",
    "https://example.org/ns/causal#Estimand/treatment",
    "https://example.org/ns/causal#Estimand/outcome",
    "https://example.org/ns/causal#Identification/assumptions",
    "https://example.org/ns/causal#CausalProblem/name",
    "https://example.org/ns/causal#CausalProblem/scm",
    "https://example.org/ns/causal#CausalProblem/actualWorld",
    "https://example.org/ns/causal#CausalProblem/counterfactuals",
    "https://example.org/ns/causal#CausalProblem/estimands",
    "https://example.org/ns/causal#CausalProblem/queries",
    "https://example.org/ns/causal#CausalReport/problem",
    "https://example.org/ns/causal#CausalReport/actualWorld",
    "https://example.org/ns/causal#CausalReport/counterfactuals",
    "https://example.org/ns/causal#CausalReport/estimands",
    "https://example.org/ns/causal#CausalReport/queries",
    "https://example.org/ns/causal#CausalReport/violations",
    "https://example.org/ns/causal#EstimandResult/estimand",
    "https://example.org/ns/causal#EstimandResult/identification",
    "https://example.org/ns/causal#QueryResult/query",
    "https://example.org/ns/causal#QueryResult/value",
    "https://example.org/ns/causal#Violation/rule",
    "https://example.org/ns/causal#Violation/detail",
];

/// The authoritative class list of the TensorIR layer (bidirectional
/// consistency with `tensor/*.ttl`; variant subclasses are declared in their
/// parent file).
pub const TENS_CLASSES: &[&str] = &[
    "https://example.org/ns/tensor#Domain",
    "https://example.org/ns/tensor#Tensor",
    "https://example.org/ns/tensor#TensorKind",
    "https://example.org/ns/tensor#TensorExpr",
    "https://example.org/ns/tensor#ElementOp",
    "https://example.org/ns/tensor#ReductionOp",
    "https://example.org/ns/tensor#Nonlinearity",
    "https://example.org/ns/tensor#TensorEquation",
    "https://example.org/ns/tensor#Target",
    "https://example.org/ns/tensor#ExecutionRegion",
    "https://example.org/ns/tensor#Relaxation",
    "https://example.org/ns/tensor#PhysicalLayout",
    "https://example.org/ns/tensor#TuckerRepresentation",
    "https://example.org/ns/tensor#Embedding",
];

/// The authoritative class-scoped property list of the TensorIR layer
/// (`{Class}/{field}` form, consistent with the codec's `Namespace::property`).
pub const TENS_PROPERTIES: &[&str] = &[
    "https://example.org/ns/tensor#Domain/name",
    "https://example.org/ns/tensor#Domain/cardinality",
    "https://example.org/ns/tensor#Tensor/name",
    "https://example.org/ns/tensor#Tensor/indices",
    "https://example.org/ns/tensor#Tensor/kind",
    "https://example.org/ns/tensor#TensorExpr/name",
    "https://example.org/ns/tensor#TensorExpr/left",
    "https://example.org/ns/tensor#TensorExpr/right",
    "https://example.org/ns/tensor#TensorExpr/operand",
    "https://example.org/ns/tensor#TensorExpr/op",
    "https://example.org/ns/tensor#TensorExpr/indices",
    "https://example.org/ns/tensor#TensorExpr/factor",
    "https://example.org/ns/tensor#TensorEquation/output",
    "https://example.org/ns/tensor#TensorEquation/expr",
    "https://example.org/ns/tensor#ExecutionRegion/name",
    "https://example.org/ns/tensor#ExecutionRegion/equations",
    "https://example.org/ns/tensor#ExecutionRegion/target",
    "https://example.org/ns/tensor#Relaxation/threshold",
    "https://example.org/ns/tensor#TuckerRepresentation/semantic",
    "https://example.org/ns/tensor#TuckerRepresentation/physical",
    "https://example.org/ns/tensor#TuckerRepresentation/errorBound",
    "https://example.org/ns/tensor#Embedding/entityDomain",
    "https://example.org/ns/tensor#Embedding/dimension",
];

/// The authoritative class-scoped property list of the SolverIR layer
/// (`{Class}/{field}` form, consistent with the codec's `Namespace::property`).
pub const SOLVER_PROPERTIES: &[&str] = &[
    "https://example.org/ns/solver#Literal/var",
    "https://example.org/ns/solver#Literal/negated",
    "https://example.org/ns/solver#ClauseSet/variables",
    "https://example.org/ns/solver#ClauseSet/clauses",
    "https://example.org/ns/solver#Payload/cnf",
    "https://example.org/ns/solver#Payload/smt",
    "https://example.org/ns/solver#Payload/tptp",
    "https://example.org/ns/solver#CompiledProblem/fragment",
    "https://example.org/ns/solver#CompiledProblem/encoding",
    "https://example.org/ns/solver#CompiledProblem/certificate",
    "https://example.org/ns/solver#CompiledProblem/payload",
    "https://example.org/ns/solver#CompilationCertificate/sourceFragment",
    "https://example.org/ns/solver#CompilationCertificate/encoding",
    "https://example.org/ns/solver#CompilationCertificate/strength",
    "https://example.org/ns/solver#CompilationCertificate/exactness",
    "https://example.org/ns/solver#SmtTerm/name",
    "https://example.org/ns/solver#SmtTerm/value",
    "https://example.org/ns/solver#SmtTerm/terms",
    "https://example.org/ns/solver#SmtTerm/scalar",
    "https://example.org/ns/solver#SmtTerm/term",
    "https://example.org/ns/solver#SmtFormula/name",
    "https://example.org/ns/solver#SmtFormula/left",
    "https://example.org/ns/solver#SmtFormula/right",
    "https://example.org/ns/solver#SmtFormula/formula",
    "https://example.org/ns/solver#SmtFormula/formulas",
    "https://example.org/ns/solver#SmtFormula/antecedent",
    "https://example.org/ns/solver#SmtFormula/consequent",
    "https://example.org/ns/solver#SmtFormula/variable",
    "https://example.org/ns/solver#SmtFormula/sort",
    "https://example.org/ns/solver#SmtFormula/body",
    "https://example.org/ns/solver#SmtProblem/logic",
    "https://example.org/ns/solver#SmtProblem/formula",
    "https://example.org/ns/solver#FofTerm/name",
    "https://example.org/ns/solver#FofTerm/function",
    "https://example.org/ns/solver#FofTerm/arguments",
    "https://example.org/ns/solver#FofFormula/predicate",
    "https://example.org/ns/solver#FofFormula/arguments",
    "https://example.org/ns/solver#FofFormula/formula",
    "https://example.org/ns/solver#FofFormula/formulas",
    "https://example.org/ns/solver#FofFormula/antecedent",
    "https://example.org/ns/solver#FofFormula/consequent",
    "https://example.org/ns/solver#FofFormula/variable",
    "https://example.org/ns/solver#FofFormula/body",
    "https://example.org/ns/solver#TptpProblem/premises",
    "https://example.org/ns/solver#TptpProblem/conjecture",
    "https://example.org/ns/solver#Assignment/variable",
    "https://example.org/ns/solver#Assignment/value",
    "https://example.org/ns/solver#BackendResult/backend",
    "https://example.org/ns/solver#BackendResult/status",
    "https://example.org/ns/solver#BackendResult/exactness",
    "https://example.org/ns/solver#BackendResult/model",
    "https://example.org/ns/solver#BackendResult/unsatCore",
    "https://example.org/ns/solver#BackendResult/derivation",
    "https://example.org/ns/solver#Resources/memoryMb",
    "https://example.org/ns/solver#Resources/cpu",
    "https://example.org/ns/solver#Resources/gpu",
    "https://example.org/ns/solver#Resources/network",
    "https://example.org/ns/solver#CapabilityDescriptor/backend",
    "https://example.org/ns/solver#CapabilityDescriptor/capability",
    "https://example.org/ns/solver#CapabilityDescriptor/fragments",
    "https://example.org/ns/solver#CapabilityDescriptor/domains",
    "https://example.org/ns/solver#CapabilityDescriptor/quantifiers",
    "https://example.org/ns/solver#CapabilityDescriptor/recursion",
    "https://example.org/ns/solver#CapabilityDescriptor/proofs",
    "https://example.org/ns/solver#CapabilityDescriptor/exactness",
    "https://example.org/ns/solver#CapabilityDescriptor/resources",
    // Phase 4: computational contracts and solver profiles.
    "https://example.org/ns/solver#Contract/maxInputSize",
    "https://example.org/ns/solver#Contract/expectedRuntime",
    "https://example.org/ns/solver#Contract/worstCaseRuntime",
    "https://example.org/ns/solver#Contract/memoryRequirement",
    "https://example.org/ns/solver#Contract/accuracyGuarantee",
    "https://example.org/ns/solver#Contract/failureProbability",
    "https://example.org/ns/solver#Contract/hardwareRequirements",
    "https://example.org/ns/solver#Contract/resourceRequirement",
    "https://example.org/ns/solver#SolverProfile/backend",
    "https://example.org/ns/solver#SolverProfile/problemClass",
    "https://example.org/ns/solver#SolverProfile/supportedLogic",
    "https://example.org/ns/solver#SolverProfile/exactness",
    "https://example.org/ns/solver#SolverProfile/completeness",
    "https://example.org/ns/solver#SolverProfile/soundness",
    "https://example.org/ns/solver#SolverProfile/timeComplexity",
    "https://example.org/ns/solver#SolverProfile/memoryComplexity",
    "https://example.org/ns/solver#SolverProfile/parallelism",
    "https://example.org/ns/solver#SolverProfile/hardwareRequirements",
    "https://example.org/ns/solver#SolverProfile/inputLimits",
    "https://example.org/ns/solver#SolverProfile/numericalPrecision",
    "https://example.org/ns/solver#SolverProfile/failureModes",
    "https://example.org/ns/solver#InputLimits/maxInputSize",
    "https://example.org/ns/solver#InputLimits/maxMemoryBytes",
];

/// The authoritative datatype-property list of the shared resource layer
/// (`resource/properties.ttl`): one sparse vector of canonical dimensions
/// consumed by complexity-ir, solver-ir, event-ir, info-ir, and the
/// orchestrator.
pub const RESOURCE_PROPERTIES: &[&str] = &[
    "https://example.org/ns/resource#wallTime",
    "https://example.org/ns/resource#cpuTime",
    "https://example.org/ns/resource#memory",
    "https://example.org/ns/resource#gpuMemory",
    "https://example.org/ns/resource#energy",
    "https://example.org/ns/resource#energyCpu",
    "https://example.org/ns/resource#energyGpu",
    "https://example.org/ns/resource#energyMemory",
    "https://example.org/ns/resource#energyNetwork",
    "https://example.org/ns/resource#networkBytes",
    "https://example.org/ns/resource#storage",
    "https://example.org/ns/resource#monetaryCost",
    "https://example.org/ns/resource#latency",
    "https://example.org/ns/resource#physicalRisk",
    "https://example.org/ns/resource#computeUnits",
    "https://example.org/ns/resource#iterations",
];

/// The authoritative class list of the logic layer (bidirectional consistency
/// with `logic/*.ttl`; variant subclasses are declared in their parent file).
pub const LOGIC_CLASSES: &[&str] = &[
    "https://example.org/ns/logic#LogicModel",
    "https://example.org/ns/logic#Signature",
    "https://example.org/ns/logic#VarDecl",
    "https://example.org/ns/logic#SemanticProfile",
    "https://example.org/ns/logic#BaseLogic",
    "https://example.org/ns/logic#Theory",
    "https://example.org/ns/logic#ExtensionKind",
    "https://example.org/ns/logic#NegationSemantics",
    "https://example.org/ns/logic#ProbabilisticStatement",
    "https://example.org/ns/logic#Statement",
    "https://example.org/ns/logic#Fact",
    "https://example.org/ns/logic#Rule",
    "https://example.org/ns/logic#Atom",
    "https://example.org/ns/logic#Term",
    "https://example.org/ns/logic#Formula",
    "https://example.org/ns/logic#CompareOp",
    "https://example.org/ns/logic#CausalStatement",
];

/// The authoritative property list of the logic layer (bidirectional
/// consistency with `logic/properties.ttl`).
pub const LOGIC_PROPERTIES: &[&str] = &[
    "https://example.org/ns/logic#rule",
    "https://example.org/ns/logic#head",
    "https://example.org/ns/logic#body",
    "https://example.org/ns/logic#atom",
    "https://example.org/ns/logic#predicate",
    "https://example.org/ns/logic#arguments",
    "https://example.org/ns/logic#op",
    "https://example.org/ns/logic#left",
    "https://example.org/ns/logic#right",
    "https://example.org/ns/logic#name",
    "https://example.org/ns/logic#function",
    "https://example.org/ns/logic#value",
    "https://example.org/ns/logic#hasStatement",
    "https://example.org/ns/logic#cause",
    "https://example.org/ns/logic#effect",
    "https://example.org/ns/logic#probability",
    "https://example.org/ns/logic#probabilityQuery",
    "https://example.org/ns/logic#proposition",
];

/// The authoritative predicate list of the model layer (bidirectional
/// consistency with the ontology's `properties.ttl`).
pub const MODEL_PROPERTIES: &[&str] = &[
    PRED_HAS_VARIABLE,
    PRED_HAS_PARAMETER,
    PRED_HAS_EQUATION,
    PRED_LHS,
    PRED_RHS,
    PRED_OPERANDS,
    PRED_OPERAND,
    PRED_LEFT,
    PRED_RIGHT,
    PRED_ARGUMENT,
    PRED_BASE,
    PRED_EXPONENT,
    PRED_VALUE,
    PRED_NAME,
    PRED_SYMBOL,
    PRED_LATEX_SYMBOL,
    PRED_INDEX_OF,
    PRED_VARIABLE_OF,
    PRED_INDEX_DIMENSION,
    PRED_PERIOD,
    PRED_OFFSET,
    PRED_HAS_DATASET,
    PRED_BINDS,
    PRED_COLUMN,
    PRED_COLUMN_EXPRESSION,
    PRED_INDEX_COLUMN,
    PRED_KEY_FORMAT,
    PRED_ROW_FILTER,
    PRED_BINDING_OF,
    PRED_PREFERRED_NOTATION,
    PRED_ALTERNATE_NOTATION,
    PRED_HAS_DOMAIN,
    PRED_HAS_ASSUMPTION,
    PRED_LAG,
    PRED_LEAD,
    PRED_HAS_OBJECTIVE,
    PRED_HAS_CONSTRAINT,
    PRED_SENSE,
    PRED_OPERATOR,
    PRED_INTEGER,
    PRED_T0,
    PRED_T_END,
    PRED_DT,
    PRED_INITIAL,
    PRED_STATE,
    PRED_INITIAL_DERIVATIVE,
    PRED_OF_SOURCE,
    PRED_ROW_COUNT,
    PRED_COLUMNS,
    PRED_USES,
    PRED_KIND,
    PRED_LOCATION,
    PRED_FORMAT,
    PRED_CHECKSUM,
    PRED_DATATYPE,
    PRED_ORDINAL,
    PRED_ROW,
    PRED_BACKEND,
    PRED_LOADED_AT,
    PRED_AGGREGATE,
    PRED_TABLE,
];
