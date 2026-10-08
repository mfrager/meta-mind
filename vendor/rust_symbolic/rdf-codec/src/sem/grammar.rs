//! SEM module grammar tables (§4.3, §5 of `planning/s_expr/s_expr_design2.md`)
//! and the **grammar card** renderer (§6.4).
//!
//! Phase 0 seeds only the modules whose engine surfaces are fully goldened:
//! `numerical`, `solver`, `logic`, `probabilistic`. Every other module is
//! *absent* from the table so the compiler fails loudly (`sem:e1`) instead of
//! guessing — the "loud-missing-class" rule of §4.3.
//!
//! Vocabulary ground truth: this crate's sibling `crates/math-core/src/vocabulary.rs`
//! (flat `model:#prop` predicates, `rdf:List` for `model:operands`, literal
//! `model:sense`/`model:operator` values) and the goldened fixtures under
//! `examples/{numerical,solver,probabilistic}/**/model.crdf.json`. The SEM lane
//! resolves the same IRIs the engine reads — no class-scoped properties exist
//! in the model layer.

use std::collections::BTreeMap;

/// The RDF prefix ↔ namespace registry the SEM lane resolves identifiers
/// against — the invariant half of the design: this table is never derived
/// from the ontology at runtime, it is the single source of truth and must
/// stay in sync via the §4.3 CI diff. (Mirrors `crdf::PrefixTable` but is
/// intentionally independent — the SEM lane never touches the cRDF codec.)
pub struct SemPrefixes {
    pub prefix_to_iri: BTreeMap<&'static str, &'static str>,
}

impl Default for SemPrefixes {
    fn default() -> Self {
        let mut m = BTreeMap::new();
        for (p, i) in [
            ("model", "https://example.org/ns/model#"),
            ("ex", "https://example.org/m/"),
            ("event", "https://example.org/ns/event#"),
            ("event_data", "https://example.org/data/event/"),
            ("info_data", "https://example.org/data/info/"),
            ("learning_data", "https://example.org/data/learning/"),
            ("memory_data", "https://example.org/data/memory/"),
            ("complexity_data", "https://example.org/data/complexity/"),
            ("tensor_data", "https://example.org/data/tensor/"),
            ("evolution_data", "https://example.org/data/evolution/"),
            ("analogy_data", "https://example.org/data/analogy/"),
            ("symreg_data", "https://example.org/data/symreg/"),
            ("synthesis_data", "https://example.org/data/synthesis/"),
            ("causal_data", "https://example.org/data/causal/"),
            ("decision_data", "https://example.org/data/decision/"),
            ("mechanism_data", "https://example.org/data/mechanism/"),
            ("tom_data", "https://example.org/data/tom/"),
            ("epistemic_data", "https://example.org/data/epistemic/"),
            (
                "epistemicmarket_data",
                "https://example.org/data/epistemicmarket/",
            ),
            ("abstraction_data", "https://example.org/data/abstraction/"),
            ("ensemble_data", "https://example.org/data/ensemble/"),
            ("causal", "https://example.org/ns/causal#"),
            ("temporal", "https://example.org/ns/temporal#"),
            ("logic", "https://example.org/ns/logic#"),
            ("epistemic", "https://example.org/ns/epistemic#"),
            ("decision", "https://example.org/ns/decision#"),
            ("mechanism", "https://example.org/ns/mechanism#"),
            ("learning", "https://example.org/ns/learning#"),
            ("evolution", "https://example.org/ns/evolution#"),
            ("analogy", "https://example.org/ns/analogy#"),
            ("symreg", "https://example.org/ns/symreg#"),
            ("synthesis", "https://example.org/ns/synthesis#"),
            ("memory", "https://example.org/ns/memory#"),
            ("argumentation", "https://example.org/ns/argument#"),
            ("argumentation_data", "https://example.org/data/argument/"),
            ("argument_data", "https://example.org/data/argument/"),
            ("provenance", "https://example.org/ns/prov#"),
            ("provenance_data", "https://example.org/data/prov/"),
            ("prov_data", "https://example.org/data/prov/"),
            ("abstraction", "https://example.org/ns/abstraction#"),
            ("ensemble", "https://example.org/ns/ensemble#"),
            ("info", "https://example.org/ns/info#"),
            ("information", "https://example.org/ns/info#"),
            ("complexity", "https://example.org/ns/complexity#"),
            ("tensor", "https://example.org/ns/tensor#"),
            ("tom", "https://example.org/ns/tom#"),
            ("epistemicmarket", "https://example.org/ns/epistemicmarket#"),
            ("solver", "https://example.org/ns/solver#"),
            ("temporal_data", "https://example.org/data/temporal/"),
        ] {
            m.insert(p, i);
        }
        SemPrefixes { prefix_to_iri: m }
    }
}

impl SemPrefixes {
    /// Resolve `prefix:local` → full IRI; `None` for unknown prefixes.
    pub fn resolve(&self, prefixed: &str) -> Option<String> {
        let (p, local) = prefixed.split_once(':')?;
        let iri = self.prefix_to_iri.get(p)?;
        Some(format!("{iri}{local}"))
    }

    /// Compact a full IRI back to `prefix:local` if a known prefix matches.
    pub fn compact(&self, iri: &str) -> Option<String> {
        for (p, ns) in &self.prefix_to_iri {
            if let Some(local) = iri.strip_prefix(ns) {
                return Some(format!("{p}:{local}"));
            }
        }
        None
    }
}

/// Argument type of a grammar row (design §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgType {
    /// `num` — number atom.
    Num,
    /// `str` — string atom.
    Str,
    /// `bool`.
    Bool,
    /// `sym` — bare symbol (name/kind/reference).
    Sym,
    /// `expr` — expression tree: bare ident = variable/param ref (R4/R6),
    /// bare number = implicit constant (R5), nested form = expression node (R8).
    Expr,
    /// `kind` — a bare symbol restricted to the form's `kinds` set (R10).
    Kind,
}

/// How a `Kind` argument lowers to RDF: as a literal string (default), as a
/// constant NamedNode object `{data_prefix}:{enum_lower}/{variant_lower}`
/// (e.g. `event_data:eventkind/instant`), optionally with an `rdf:type`
/// `{schema_class}/{Variant}` (temporal kinds), or as a generated subnode
/// typed `{schema_class}/{Variant}` with a literal operand (event `op`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindMode {
    /// Literal (the default, e.g. `model:sense`).
    Literal,
    /// Constant NamedNode in the data namespace; `typed` adds an rdf:type.
    Object {
        data_prefix: &'static str,
        enum_name: &'static str,
        typed: bool,
        /// Emit the rdf:type variant lowercased (`{schema}:{Enum}/{variant}`),
        /// matching the model-layer vocabulary (`model:DataSourceKind/csv`,
        /// `model:ParameterAggregate/mean`) instead of PascalCase.
        lower_variant: bool,
    },
    /// Generated subnode typed `{schema_class}/{Variant}` with an operand
    /// literal emitted on `{operand_pred}` (event `NumericOp`).
    Node {
        schema_class: &'static str,
        operand_pred: &'static str,
    },
}

/// A property of a grammar row: argument name, the RDF predicate it lowers to
/// (prefixed, resolved via [`SemPrefixes`]), the argument type, whether it is
/// an `rdf:List` (e.g. `model:operands`), whether it lowers to one repeated
/// triple per value (e.g. `logic:rule`), whether the argument is optional,
/// and how a `Kind` argument lowers to RDF.
#[derive(Debug, Clone)]
pub struct Prop {
    pub name: &'static str,
    pub pred: &'static str,
    pub ty: ArgType,
    pub list: bool,
    pub multi: bool,
    pub opt: bool,
    pub kind_mode: KindMode,
    /// Render a list prop with explicit `[...]` brackets (event/temporal),
    /// rather than flattened positional args (numerical operands).
    pub bracketed: bool,
    /// Keyword-style prop: the renderer emits `name=value` for this prop
    /// whenever it is present (and switches the whole form to keyword style).
    /// Used by the probabilistic forms (`categorical(name=..., states=[...])`,
    /// `query(kind=mpe, ...)`, `probability(event=...)`) so their canonical
    /// keyword surface round-trips while plain forms (`query(rain)`,
    /// `probability(0.3, rain)`) keep their positional canonical form.
    pub kwarg: bool,
}

impl Prop {
    pub const fn new(name: &'static str, pred: &'static str, ty: ArgType) -> Self {
        Self {
            name,
            pred,
            ty,
            list: false,
            multi: false,
            opt: false,
            kind_mode: KindMode::Literal,
            bracketed: false,
            kwarg: false,
        }
    }
    /// An `rdf:List`-valued property (one head node; `rdf:first`/`rdf:rest`).
    pub const fn list(name: &'static str, pred: &'static str, ty: ArgType) -> Self {
        Self {
            name,
            pred,
            ty,
            list: true,
            multi: false,
            opt: false,
            kind_mode: KindMode::Literal,
            bracketed: false,
            kwarg: false,
        }
    }
    /// A repeated triple per value (multi-valued property, no rdf:List).
    pub const fn multi(name: &'static str, pred: &'static str, ty: ArgType) -> Self {
        Self {
            name,
            pred,
            ty,
            list: false,
            multi: true,
            opt: false,
            kind_mode: KindMode::Literal,
            bracketed: false,
            kwarg: false,
        }
    }
    /// Render this prop as `name=value` whenever present (keyword style).
    pub const fn kwarg(mut self) -> Self {
        self.kwarg = true;
        self
    }
    pub const fn opt(mut self) -> Self {
        self.opt = true;
        self
    }
    /// Mark as an `rdf:List`-valued property.
    pub const fn as_list(mut self) -> Self {
        self.list = true;
        self
    }
    /// Mark as rendering with explicit `[...]` brackets.
    pub const fn bracketed(mut self) -> Self {
        self.bracketed = true;
        self
    }
    /// A kind arg that lowers to a constant NamedNode object.
    pub const fn kind_object(self, data_prefix: &'static str, enum_name: &'static str) -> Self {
        Prop {
            kind_mode: KindMode::Object {
                data_prefix,
                enum_name,
                typed: false,
                lower_variant: false,
            },
            ..self
        }
    }
    /// A kind arg that lowers to a typed constant NamedNode object.
    pub const fn kind_object_typed(
        self,
        data_prefix: &'static str,
        enum_name: &'static str,
    ) -> Self {
        Prop {
            kind_mode: KindMode::Object {
                data_prefix,
                enum_name,
                typed: true,
                lower_variant: false,
            },
            ..self
        }
    }
    /// A kind arg that lowers to a typed constant NamedNode whose rdf:type
    /// keeps the variant lowercased (model-layer enums: `DataSourceKind`,
    /// `ParameterAggregate`).
    pub const fn kind_object_typed_lower(
        self,
        data_prefix: &'static str,
        enum_name: &'static str,
    ) -> Self {
        Prop {
            kind_mode: KindMode::Object {
                data_prefix,
                enum_name,
                typed: true,
                lower_variant: true,
            },
            ..self
        }
    }
    /// A kind arg that lowers to a generated subnode typed
    /// `{schema_class}/{Variant}` with an operand literal (event `op`).
    pub const fn kind_node(self, schema_class: &'static str, operand_pred: &'static str) -> Self {
        Prop {
            kind_mode: KindMode::Node {
                schema_class,
                operand_pred,
            },
            ..self
        }
    }
}

/// How a form's RDF subject id is derived (design R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjKind {
    /// Subject = `ex:<first argument>` (the declared name), e.g. `var(gdp)`.
    Named,
    /// Subject = `ex:<prefix><i>` by per-class-kind counter, e.g. `eq0`, `e0`,
    /// `c0`, `obj0`, `con0`.
    Generated(&'static str),
}

/// One form in a module vocabulary: its RDF class (prefixed), its properties
/// in canonical argument order, its kind-value sets, and its subject rule.
#[derive(Debug, Clone)]
pub struct FormSpec {
    pub class: &'static str,
    pub props: Vec<Prop>,
    /// Allowed kind values for `Kind`-typed arguments, by arg name (R10).
    pub kinds: BTreeMap<&'static str, &'static [&'static str]>,
    pub subj: SubjKind,
}

/// A complete module grammar.
#[derive(Debug, Clone)]
pub struct Grammar {
    pub name: &'static str,
    /// The schema prefix used for kind-object `rdf:type` triples (e.g. `event`,
    /// `temporal`). Resolved via [`SemPrefixes`] with `{Variant}` PascalCase.
    pub schema_prefix: &'static str,
    /// Entry form name (R9: the auto-rooted top form).
    pub entry: &'static str,
    /// Root class (prefixed), for R9 auto-rooting.
    pub root_class: &'static str,
    /// `(content form name → root predicate)` — the R9 auto-wiring table.
    pub root_links: Vec<(&'static str, &'static str)>,
    /// Dynamic predicate application: when true, any nested form name outside
    /// the module's forms/exprs tables compiles to a predicate node
    /// (`logic:Atom` + `logic:predicate` + ordered `logic:arguments`) instead
    /// of a `sem:e2` unknown-function error. This is the SEM realization of
    /// `(predicate TERM…)` for the logic modules (`logic_sem_design.md` §4.5).
    pub dynamic_predicates: bool,
    /// Entry-level literal props (e.g. `rules` for logic/probabilistic).
    /// When non-empty, the entry form's args map onto these props directly.
    pub entry_props: Vec<Prop>,
    /// Module-level kind definitions for entry props (e.g. provenance's `traversal`).
    pub kinds: BTreeMap<&'static str, &'static [&'static str]>,
    /// Tier 2 forms for URI-annotation output (Selective mode): declaration
    /// forms that carry `[iri]` annotations even when they are not root-linked
    /// slots (`planning/updates/sem_uri_annotation_design.md` §6.2). Tier 1 —
    /// root-linked children — is derived from `root_links` and needs no entry
    /// here.
    pub annotated_forms: Vec<&'static str>,
    /// Content forms (including the entry form when it has `entry_props`).
    pub forms: BTreeMap<&'static str, FormSpec>,
    /// Expression forms (R8): name → spec.
    pub exprs: BTreeMap<&'static str, FormSpec>,
}

impl Grammar {
    pub fn form(&self, name: &str) -> Option<&FormSpec> {
        self.forms.get(name)
    }
    pub fn expr(&self, name: &str) -> Option<&FormSpec> {
        self.exprs.get(name)
    }
}

/// The seeded grammars. Phase 1: numerical/solver. Phase 2: logic,
/// probabilistic, event, temporal. Everything else stays absent (compile
/// fails with `sem:e1`).
/// Every module with a seeded SEM grammar — the Phase 5 rollout list (§9.6).
pub const ALL_MODULES: &[&str] = &[
    "numerical",
    "solver",
    "logic",
    "probabilistic",
    "event",
    "temporal",
    "tensor",
    "information",
    "learning",
    "memory",
    "complexity",
    "argumentation",
    "provenance",
    "evolution",
    "analogy",
    "symreg",
    "synthesis",
    "causal",
    "decision",
    "mechanism",
    "tom",
    "epistemic",
    "epistemicmarket",
    "abstraction",
    "ensemble",
];

pub fn grammar(module: &str) -> Option<Grammar> {
    match module {
        "numerical" => Some(numerical()),
        "solver" => Some(solver()),
        "logic" => Some(logic_like("logic")),
        "probabilistic" => Some(logic_like("probabilistic")),
        "event" => Some(event()),
        "temporal" => Some(temporal()),
        "tensor" => Some(tensor()),
        "information" => Some(information()),
        "learning" => Some(learning()),
        "memory" => Some(memory()),
        "complexity" => Some(complexity()),
        "argumentation" => Some(argumentation()),
        "provenance" => Some(provenance()),
        "evolution" => Some(evolution()),
        "analogy" => Some(analogy()),
        "symreg" => Some(symreg()),
        "synthesis" => Some(synthesis()),
        "causal" => Some(causal()),
        "decision" => Some(decision()),
        "mechanism" => Some(mechanism()),
        "tom" => Some(tom()),
        "epistemic" => Some(epistemic()),
        "epistemicmarket" => Some(epistemicmarket()),
        "abstraction" => Some(abstraction()),
        "ensemble" => Some(ensemble()),
        _ => None,
    }
}

// ── shared algebra (numerical, solver) ──────────────────────────────────────

/// The name argument of a `Named`-subject form: the name IS the subject IRI
/// (`ex:<name>`, design R7) — no `model:name` triple is emitted, matching the
/// goldened fixtures (variables/parameters carry only `model:value`).
fn name_prop() -> Prop {
    Prop::new("name", "", ArgType::Sym)
}

fn var_spec() -> FormSpec {
    FormSpec {
        class: "model:Variable",
        props: vec![name_prop()],
        kinds: BTreeMap::new(),
        subj: SubjKind::Named,
    }
}

fn param_spec() -> FormSpec {
    FormSpec {
        class: "model:Parameter",
        props: vec![
            name_prop(),
            Prop::new("value", "model:value", ArgType::Num).opt(),
        ],
        kinds: BTreeMap::new(),
        subj: SubjKind::Named,
    }
}

fn equation_spec() -> FormSpec {
    FormSpec {
        class: "model:Equation",
        props: vec![
            Prop::new("lhs", "model:lhs", ArgType::Expr),
            Prop::new("rhs", "model:rhs", ArgType::Expr),
        ],
        kinds: BTreeMap::new(),
        subj: SubjKind::Generated("eq"),
    }
}

fn objective_spec() -> FormSpec {
    let mut kinds = BTreeMap::new();
    kinds.insert("sense", &["minimize", "maximize"][..]);
    FormSpec {
        class: "model:Objective",
        props: vec![
            Prop::new("sense", "model:sense", ArgType::Kind),
            Prop::new("rhs", "model:rhs", ArgType::Expr),
        ],
        kinds,
        subj: SubjKind::Generated("obj"),
    }
}

fn constraint_spec() -> FormSpec {
    let mut kinds = BTreeMap::new();
    kinds.insert("op", &["le", "ge", "eq"][..]);
    FormSpec {
        class: "model:Inequality",
        props: vec![
            Prop::new("op", "model:operator", ArgType::Kind),
            Prop::new("lhs", "model:lhs", ArgType::Expr),
            Prop::new("rhs", "model:rhs", ArgType::Num),
        ],
        kinds,
        subj: SubjKind::Generated("con"),
    }
}

fn expr_specs() -> BTreeMap<&'static str, FormSpec> {
    let mut e = BTreeMap::new();
    let mut add = |name: &'static str, class: &'static str, props: Vec<Prop>| {
        e.insert(
            name,
            FormSpec {
                class,
                props,
                kinds: BTreeMap::new(),
                subj: SubjKind::Generated("e"),
            },
        );
    };
    add(
        "sum",
        "model:Addition",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "mul",
        "model:Multiplication",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "sub",
        "model:Subtraction",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "div",
        "model:Division",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "pow",
        "model:Power",
        vec![
            Prop::new("base", "model:base", ArgType::Expr),
            Prop::new("exponent", "model:exponent", ArgType::Expr),
        ],
    );
    // Calculus: unary/transcendental functions (one `model:operands` item).
    for (name, class) in [
        ("sin", "model:Sin"),
        ("cos", "model:Cos"),
        ("exp", "model:Exp"),
        ("log", "model:Log"),
        ("neg", "model:Negation"),
    ] {
        add(
            name,
            class,
            vec![Prop::list("operands", "model:operands", ArgType::Expr)],
        );
    }
    // Time-series shifts: one operand (the series) + optional `model:offset`.
    for (name, class) in [("lag", "model:Lag"), ("lead", "model:Lead")] {
        add(
            name,
            class,
            vec![
                Prop::list("operands", "model:operands", ArgType::Expr),
                Prop::new("offset", "model:offset", ArgType::Num).opt(),
            ],
        );
    }
    e
}

fn generic_expr(_name: &'static str) -> FormSpec {
    FormSpec {
        class: "model:Expression",
        props: vec![],
        kinds: BTreeMap::new(),
        subj: SubjKind::Generated("gx"),
    }
}

/// `model:Differential` — an ODE/DAE equation. ODE nodes carry `model:lhs`
/// (state variable) + `model:rhs` (derivative) plus the integration window
/// (`t0`/`tEnd`/`dt`) and `model:initial`; DAE residual nodes carry
/// `model:state` (comma-separated list literal) + `model:rhs` (residual),
/// `model:initial` and `model:initialDerivative`. All but `rhs` are optional
/// (ODE vs DAE shapes differ), so the form renders as keyword args.
fn differential_spec() -> FormSpec {
    FormSpec {
        class: "model:Differential",
        props: vec![
            Prop::new("lhs", "model:lhs", ArgType::Expr).opt(),
            Prop::new("rhs", "model:rhs", ArgType::Expr),
            Prop::new("state", "model:state", ArgType::Str).opt(),
            Prop::new("t0", "model:t0", ArgType::Num).opt(),
            Prop::new("t_end", "model:tEnd", ArgType::Num).opt(),
            Prop::new("dt", "model:dt", ArgType::Num).opt(),
            Prop::new("initial", "model:initial", ArgType::Num).opt(),
            Prop::new(
                "initial_derivative",
                "model:initialDerivative",
                ArgType::Num,
            )
            .opt(),
        ],
        kinds: BTreeMap::new(),
        subj: SubjKind::Generated("d"),
    }
}

fn algebra_forms() -> BTreeMap<&'static str, FormSpec> {
    let mut f = BTreeMap::new();
    f.insert("var", var_spec());
    f.insert("param", param_spec());
    f.insert("dim", dim_spec());
    f.insert("equation", equation_spec());
    f.insert("differential", differential_spec());
    f.insert("data_source", data_source_spec());
    // Hybrid model-layer documents (§3.5/§4.3 of the SEM golden plan): a
    // single `numerical`/`solver`/`tensor` root may also carry the structured
    // logic statement forms, so one SEM can author equations + a Horn-rule
    // overlay + data sources together. `fact`/`rule` keep the logic grammar's
    // exact class/property shape, so `rdf_to_logic_model` lowers them unchanged.
    f.insert("fact", logic_fact_spec());
    f.insert("rule", logic_rule_spec());
    f
}

/// `fact(head)` — the structured `logic:Fact` statement (logic grammar §4.3),
/// available to the model-layer grammars for hybrid equation+rule documents.
fn logic_fact_spec() -> FormSpec {
    FormSpec {
        class: "logic:Fact",
        props: vec![Prop::new("head", "logic:head", ArgType::Expr)],
        kinds: BTreeMap::new(),
        subj: SubjKind::Generated("s"),
    }
}

/// `rule(head, [body…])` / `rule(head=…, body=[…])` — the structured
/// `logic:Rule` statement, available to the model-layer grammars so a hybrid
/// model can carry a Horn-rule overlay that the planner decomposes to Tensor.
fn logic_rule_spec() -> FormSpec {
    FormSpec {
        class: "logic:Rule",
        props: vec![
            Prop::new("head", "logic:head", ArgType::Expr),
            Prop::list("body", "logic:body", ArgType::Expr).bracketed(),
        ],
        kinds: BTreeMap::new(),
        subj: SubjKind::Generated("s"),
    }
}

/// `data_source(name, kind, "location", format=…)` — a named `model:DataSource`
/// declaration. The SEM is the sole authored input: the source's physical
/// backend (`model:DataSourceKind/…`) and file location live here, while the
/// column list and column↔symbol bindings are synthesized at runtime by the
/// runner from the local data file (runtime-generated, never static fixtures).
fn data_source_spec() -> FormSpec {
    let mut kinds = BTreeMap::new();
    kinds.insert("kind", &["csv", "parquet", "duckdb", "rdf", "api"][..]);
    FormSpec {
        class: "model:DataSource",
        props: vec![
            Prop::new("name", "model:name", ArgType::Str),
            Prop::new("kind", "model:kind", ArgType::Kind)
                .kind_object_typed_lower("model", "DataSourceKind"),
            Prop::new("location", "model:location", ArgType::Str),
            Prop::new("format", "model:format", ArgType::Str).opt(),
            Prop::new("table", "model:table", ArgType::Str).opt(),
        ],
        kinds,
        subj: SubjKind::Generated("ds"),
    }
}

/// `dim(name)` — a named `model:IndexDimension` (e.g. a time period over
/// which variables are indexed). Named subject, mirroring `var`/`param`.
fn dim_spec() -> FormSpec {
    FormSpec {
        class: "model:IndexDimension",
        props: vec![name_prop()],
        kinds: BTreeMap::new(),
        subj: SubjKind::Named,
    }
}

fn solver_forms() -> BTreeMap<&'static str, FormSpec> {
    let mut f = algebra_forms();
    f.insert("objective", objective_spec());
    f.insert("constraint", constraint_spec());
    f
}

fn numerical() -> Grammar {
    Grammar {
        annotated_forms: vec!["var", "param"],
        name: "numerical",
        schema_prefix: "model",
        entry: "model",
        root_class: "model:Model",
        root_links: vec![
            ("equation", "model:hasEquation"),
            ("differential", "model:hasEquation"),
            ("fact", "logic:fact"),
            ("rule", "logic:rule"),
        ],
        dynamic_predicates: false,
        kinds: BTreeMap::new(),
        entry_props: Vec::new(),
        forms: algebra_forms(),
        exprs: expr_specs(),
    }
}

fn solver() -> Grammar {
    Grammar {
        annotated_forms: vec!["var", "param"],
        name: "solver",
        schema_prefix: "model",
        entry: "model",
        root_class: "model:Model",
        root_links: vec![
            ("equation", "model:hasEquation"),
            ("differential", "model:hasEquation"),
            ("objective", "model:hasObjective"),
            ("constraint", "model:hasConstraint"),
            ("fact", "logic:fact"),
            ("rule", "logic:rule"),
        ],
        dynamic_predicates: false,
        kinds: BTreeMap::new(),
        entry_props: Vec::new(),
        forms: solver_forms(),
        exprs: expr_specs(),
    }
}

/// logic and probabilistic share the rule-string surface: a `model:Model` root
/// with `logic:rule` string literals (the goldened fixture shape).
///
/// The grammar also carries the **structured statement surface** from
/// `planning/s_expr/logic_sem_design.md` §4: statement forms (`fact`, `rule`,
/// `deny`, `formula`, `query`, `declare`, `probability`), formula forms
/// (`not`…`iff`, `forall`/`exists`, comparisons, `in`), arithmetic exprs, and
/// dynamic predicate application (any unregistered nested name is a predicate).
/// Legacy string payloads (`logic.model(["clause."])`) compile through
/// `entry_props` exactly as before.
fn logic_like(name: &'static str) -> Grammar {
    Grammar {
        // All seven statement forms are root-linked (Tier 1); `declare`/
        // `query`/`probability` are also listed here so they stay annotated
        // if a future change moves them out of `root_links`.
        annotated_forms: vec!["declare", "query", "probability"],
        name,
        schema_prefix: "logic",
        entry: "model",
        root_class: "model:Model",
        root_links: vec![
            ("fact", "logic:fact"),
            ("rule", "logic:rule"),
            ("deny", "logic:constraint"),
            ("formula", "logic:formula"),
            ("query", "logic:query"),
            ("declare", "logic:declaration"),
            ("probability", "logic:probability"),
            ("categorical", "logic:categorical"),
            ("bernoulli", "logic:bernoulli"),
            ("bernoulli_trials", "logic:bernoulliTrials"),
            ("outcome_space", "logic:outcomeSpace"),
            ("outcome_step", "logic:outcomeStep"),
            ("outcome_measure", "logic:outcomeMeasure"),
            ("outcome_value", "logic:outcomeValue"),
            ("outcome_update", "logic:outcomeUpdate"),
            ("outcome_constraint", "logic:outcomeConstraint"),
            ("outcome_if", "logic:outcomeIf"),
        ],
        dynamic_predicates: true,
        // One `logic:rule` literal per rule on the model IRI (the goldened
        // fixture shape) — a repeated triple, not an rdf:List. Used by the
        // legacy string payload path.
        kinds: BTreeMap::new(),
        entry_props: vec![Prop::multi("rules", "logic:rule", ArgType::Str).opt()],
        forms: logic_forms(),
        exprs: logic_exprs(),
    }
}

/// The structured statement/formula forms of the logic grammar (design §4.3–§4.4).
/// Statement forms link from the root; formula forms are nested expression nodes.
fn logic_forms() -> BTreeMap<&'static str, FormSpec> {
    let mut f = BTreeMap::new();
    let mut add =
        |name: &'static str, class: &'static str, props: Vec<Prop>, prefix: &'static str| {
            f.insert(
                name,
                FormSpec {
                    class,
                    props,
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated(prefix),
                },
            );
        };
    // Statement forms (design §4.3) — root-linked content forms (`s` prefix).
    add(
        "fact",
        "logic:Fact",
        vec![Prop::new("head", "logic:head", ArgType::Expr)],
        "s",
    );
    add(
        "rule",
        "logic:Rule",
        vec![
            Prop::new("head", "logic:head", ArgType::Expr),
            Prop::list("body", "logic:body", ArgType::Expr).bracketed(),
        ],
        "s",
    );
    add(
        "deny",
        "logic:Constraint",
        vec![Prop::new("formula", "logic:formula", ArgType::Expr)],
        "s",
    );
    add(
        "formula",
        "logic:Formula",
        vec![Prop::new("formula", "logic:formula", ArgType::Expr)],
        "s",
    );
    // `query(...)` carries the typed target surface (`categorical_sem_design.md`
    // §4): an ordinary formula (positional `query(rain)`), a nested
    // `distribution(...)`/`state(...)`/`probability(...)` target, or the
    // keyword form `query(kind=mpe, proposition=...)`. `formula` is optional
    // so the keyword-only MPE form compiles; the lowering rejects a target-
    // less query.
    add(
        "query",
        "logic:Query",
        vec![
            Prop::new("formula", "logic:formula", ArgType::Expr).opt(),
            Prop::new("kind", "logic:kind", ArgType::Sym).opt().kwarg(),
            Prop::new("proposition", "logic:proposition", ArgType::Expr)
                .opt()
                .kwarg(),
        ],
        "s",
    );
    add(
        "declare",
        "logic:Declaration",
        vec![
            Prop::multi("entity", "logic:entity", ArgType::Sym),
            Prop::multi("relation", "logic:relation", ArgType::Expr).opt(),
            Prop::multi("variable", "logic:variable", ArgType::Expr).opt(),
            Prop::multi("schema", "logic:schema", ArgType::Sym).opt(),
        ],
        "s",
    );
    // `probability(...)` is a weighted statement at root level
    // (`probability(0.3, rain)`, `probability(0.8, implies(rain, wet))`);
    // nested inside `query(...)` it carries the typed compound-event target
    // (`probability(event=and(...))`) or the conditional target
    // (`probability(proposition=..., given=[...])`). `weight`/`proposition`
    // are optional so the event/conditional forms compile; the lowering
    // rejects a root-level weighted statement without a weight.
    add(
        "probability",
        "logic:Probability",
        vec![
            Prop::new("weight", "logic:weight", ArgType::Num).opt(),
            Prop::new("proposition", "logic:proposition", ArgType::Expr).opt(),
            Prop::new("event", "logic:event", ArgType::Expr)
                .opt()
                .kwarg(),
            Prop::list("given", "logic:given", ArgType::Expr)
                .opt()
                .bracketed()
                .kwarg(),
        ],
        "s",
    );

    // Categorical distributions and Bernoulli shorthand/trials
    // (`categorical_sem_design.md` §3.2–§3.3). `state(...)` is valid only
    // inside `categorical(states=[...])`; the nested `state` expression form
    // below is the typed *selector* used in rule bodies, event expressions,
    // and state queries — the compiler distinguishes them by position.
    add(
        "categorical",
        "logic:Categorical",
        vec![
            Prop::new("name", "logic:name", ArgType::Sym).kwarg(),
            Prop::list("states", "logic:states", ArgType::Expr)
                .bracketed()
                .kwarg(),
        ],
        "s",
    );
    add(
        "bernoulli",
        "logic:Bernoulli",
        vec![
            Prop::new("name", "logic:name", ArgType::Sym).kwarg(),
            Prop::new("success", "logic:success", ArgType::Sym).kwarg(),
            Prop::new("failure", "logic:failure", ArgType::Sym).kwarg(),
            Prop::new("probability", "logic:probability", ArgType::Num).kwarg(),
        ],
        "s",
    );
    add(
        "bernoulli_trials",
        "logic:BernoulliTrials",
        vec![
            Prop::new("name", "logic:name", ArgType::Sym).kwarg(),
            Prop::new("count", "logic:count", ArgType::Num).kwarg(),
            Prop::new("success", "logic:success", ArgType::Sym).kwarg(),
            Prop::new("failure", "logic:failure", ArgType::Sym).kwarg(),
            Prop::new("probability", "logic:probability", ArgType::Num).kwarg(),
            Prop::new("independent", "logic:independent", ArgType::Bool).kwarg(),
        ],
        "s",
    );

    // Generic finite outcome evaluation (`probabilistic_strategy_outcomes_and
    // _exact_rationals_plan.md` §2): `outcome_space`/`outcome_step`/
    // `outcome_measure` are root-linked statement forms carrying the mutable
    // value vector, ordered guarded transitions, and terminal measurements.
    // The nested `outcome_value`/`outcome_update`/`outcome_constraint` and the
    // `outcome_if` expression are `e` forms; the outcome query targets are `e`
    // forms too. Every `outcome_*` construct stays structured — never a
    // dynamic predicate (`model:Outcome*` classes, plan §7.1).
    add(
        "outcome_space",
        "logic:OutcomeSpace",
        vec![
            Prop::list("values", "logic:values", ArgType::Expr)
                .bracketed()
                .kwarg(),
            Prop::list("constraints", "logic:constraints", ArgType::Expr)
                .bracketed()
                .opt()
                .kwarg(),
        ],
        "s",
    );
    add(
        "outcome_step",
        "logic:OutcomeStep",
        vec![
            Prop::new("index", "logic:index", ArgType::Num).kwarg(),
            Prop::new("guard", "logic:guard", ArgType::Expr)
                .opt()
                .kwarg(),
            Prop::list("updates", "logic:updates", ArgType::Expr)
                .bracketed()
                .kwarg(),
        ],
        "s",
    );
    add(
        "outcome_measure",
        "logic:OutcomeMeasure",
        vec![
            Prop::new("name", "logic:name", ArgType::Sym).kwarg(),
            Prop::new("expression", "logic:expression", ArgType::Expr).kwarg(),
        ],
        "s",
    );

    // Formula forms (design §4.4). Each is a nested expression node (`e`
    // prefix): they inline inside statements, never link from the root.
    add(
        "not",
        "logic:Not",
        vec![Prop::new("formula", "logic:formula", ArgType::Expr)],
        "e",
    );
    add(
        "and",
        "logic:And",
        vec![Prop::list("formulas", "logic:formula", ArgType::Expr).bracketed()],
        "e",
    );
    add(
        "or",
        "logic:Or",
        vec![Prop::list("formulas", "logic:formula", ArgType::Expr).bracketed()],
        "e",
    );
    add(
        "xor",
        "logic:Xor",
        vec![
            Prop::new("left", "logic:left", ArgType::Expr),
            Prop::new("right", "logic:right", ArgType::Expr),
        ],
        "e",
    );
    add(
        "implies",
        "logic:Implies",
        vec![
            Prop::new("antecedent", "logic:antecedent", ArgType::Expr),
            Prop::new("consequent", "logic:consequent", ArgType::Expr),
        ],
        "e",
    );
    add(
        "iff",
        "logic:Iff",
        vec![
            Prop::new("left", "logic:left", ArgType::Expr),
            Prop::new("right", "logic:right", ArgType::Expr),
        ],
        "e",
    );
    add(
        "forall",
        "logic:ForAll",
        vec![
            Prop::new("variable", "logic:variable", ArgType::Expr),
            Prop::new("formula", "logic:formula", ArgType::Expr),
        ],
        "e",
    );
    add(
        "exists",
        "logic:Exists",
        vec![
            Prop::new("variable", "logic:variable", ArgType::Expr),
            Prop::new("formula", "logic:formula", ArgType::Expr),
        ],
        "e",
    );
    // Comparisons: `eq(a, b)` … `ge(a, b)` — the operator is encoded in the
    // form name and recorded as a `logic:op` literal on the node for the
    // renderer (the six forms share one `logic:Comparison` class).
    for name in ["eq", "neq", "lt", "le", "gt", "ge"] {
        add(
            name,
            "logic:Comparison",
            vec![
                Prop::new("left", "logic:left", ArgType::Expr),
                Prop::new("right", "logic:right", ArgType::Expr),
            ],
            "e",
        );
    }
    add(
        "in",
        "logic:Membership",
        vec![
            Prop::new("element", "logic:element", ArgType::Expr),
            Prop::new("set", "logic:set", ArgType::Expr),
        ],
        "e",
    );
    // Typed selectors (`categorical_sem_design.md` §4.3, §5): `state(...)`
    // and `trial(...)` in formula position are typed targets — never dynamic
    // predicates. The `state` form doubles as the nested categorical state
    // declaration (`state(name=..., probability=...)` inside
    // `categorical(states=[...])`); the lowering distinguishes them by
    // position.
    add(
        "state",
        "logic:StateSelector",
        vec![
            Prop::new("name", "logic:name", ArgType::Sym).opt().kwarg(),
            Prop::new("probability", "logic:probability", ArgType::Num)
                .opt()
                .kwarg(),
            Prop::new("variable", "logic:variable", ArgType::Sym)
                .opt()
                .kwarg(),
            Prop::new("value", "logic:value", ArgType::Sym)
                .opt()
                .kwarg(),
        ],
        "e",
    );
    add(
        "trial",
        "logic:TrialSelector",
        vec![
            Prop::new("family", "logic:family", ArgType::Sym).kwarg(),
            Prop::new("index", "logic:index", ArgType::Num).kwarg(),
            Prop::new("value", "logic:value", ArgType::Sym).kwarg(),
        ],
        "e",
    );
    // The complete-distribution query target `distribution(variable=...)`
    // (`categorical_sem_design.md` §4.2).
    add(
        "distribution",
        "logic:Distribution",
        vec![Prop::new("variable", "logic:variable", ArgType::Sym).kwarg()],
        "e",
    );
    // Generic outcome items and query targets (§2, §4). These are `e` forms
    // that inline inside the outcome statements and `query(...)` targets and
    // are never dynamic predicates.
    //
    // `outcome_value` is dual-role: a declaration row inside
    // `outcome_space(values=[outcome_value(name=..., initial=...)])` and a
    // numeric *reference* `outcome_value(x)` in measure/constraint/update
    // expressions — the two are distinguished by position and the optional
    // `initial` slot, mirroring the categorical `state` dual.
    add(
        "outcome_value",
        "logic:OutcomeValue",
        vec![
            Prop::new("name", "logic:name", ArgType::Sym).opt().kwarg(),
            Prop::new("initial", "logic:initial", ArgType::Expr)
                .opt()
                .kwarg(),
        ],
        "e",
    );
    add(
        "outcome_update",
        "logic:OutcomeUpdate",
        vec![
            Prop::new("variable", "logic:variable", ArgType::Sym).kwarg(),
            Prop::new("operation", "logic:operation", ArgType::Sym).kwarg(),
            Prop::new("value", "logic:value", ArgType::Expr).kwarg(),
        ],
        "e",
    );
    add(
        "outcome_constraint",
        "logic:OutcomeConstraint",
        vec![Prop::new("expression", "logic:expression", ArgType::Expr).kwarg()],
        "e",
    );
    add(
        "outcome_if",
        "logic:OutcomeIf",
        vec![
            Prop::new("condition", "logic:condition", ArgType::Expr).kwarg(),
            Prop::new("then", "logic:then", ArgType::Expr).kwarg(),
            Prop::new("else", "logic:else", ArgType::Expr).kwarg(),
        ],
        "e",
    );
    // Outcome query targets nested inside `query(...)` (§4.5).
    add(
        "outcome_probability",
        "logic:OutcomeProbability",
        vec![
            Prop::new("measure", "logic:measure", ArgType::Sym).kwarg(),
            Prop::new("relation", "logic:relation", ArgType::Sym).kwarg(),
            Prop::new("threshold", "logic:threshold", ArgType::Expr).kwarg(),
        ],
        "e",
    );
    add(
        "outcome_distribution",
        "logic:OutcomeDistribution",
        vec![Prop::new("measure", "logic:measure", ArgType::Sym).kwarg()],
        "e",
    );
    add(
        "outcome_expected",
        "logic:OutcomeExpected",
        vec![Prop::new("measure", "logic:measure", ArgType::Sym).kwarg()],
        "e",
    );
    add("outcome_validity", "logic:OutcomeValidity", vec![], "e");
    f
}

/// The arithmetic term forms of the logic grammar (design §4.6), lowering to
/// `model:*` expression nodes so the math pipeline and SEM decoder agree.
fn logic_exprs() -> BTreeMap<&'static str, FormSpec> {
    let mut e = BTreeMap::new();
    let mut add = |name: &'static str, class: &'static str, props: Vec<Prop>| {
        e.insert(
            name,
            FormSpec {
                class,
                props,
                kinds: BTreeMap::new(),
                subj: SubjKind::Generated("e"),
            },
        );
    };
    add(
        "add",
        "model:Addition",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "sub",
        "model:Subtraction",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "mul",
        "model:Multiplication",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "div",
        "model:Division",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "neg",
        "model:Negation",
        vec![Prop::list("operands", "model:operands", ArgType::Expr)],
    );
    add(
        "pow",
        "model:Power",
        vec![
            Prop::new("base", "model:base", ArgType::Expr),
            Prop::new("exponent", "model:exponent", ArgType::Expr),
        ],
    );
    // Unary / transcendental functions (one operand each).
    for (name, class) in [
        ("abs", "model:Abs"),
        ("sqrt", "model:Sqrt"),
        ("mod", "model:Mod"),
        ("min", "model:Min"),
        ("max", "model:Max"),
        ("sin", "model:Sin"),
        ("cos", "model:Cos"),
        ("tan", "model:Tan"),
        ("log", "model:Log"),
        ("ln", "model:Ln"),
        ("exp", "model:Exp"),
    ] {
        add(
            name,
            class,
            vec![Prop::list("operands", "model:operands", ArgType::Expr)],
        );
    }
    e
}

// ── event (§5.6) ────────────────────────────────────────────────────────────

fn event() -> Grammar {
    let mut kinds = BTreeMap::new();
    kinds.insert(
        "kind",
        &["instant", "interval", "recurring", "composite"][..],
    );
    let mut op_kinds = BTreeMap::new();
    op_kinds.insert("op", &["set", "add", "scale"][..]);

    let event_form = FormSpec {
        class: "event:Event",
        props: vec![
            Prop::new("name", "event:Event/name", ArgType::Str),
            Prop::new("time", "event:Event/time", ArgType::Num),
            Prop::new("kind", "event:Event/kind", ArgType::Kind)
                .kind_object("event_data", "EventKind"),
            Prop::new("inputs", "event:Event/inputs", ArgType::Str)
                .as_list()
                .opt()
                .bracketed(),
            Prop::new("outputs", "event:Event/outputs", ArgType::Str)
                .as_list()
                .opt()
                .bracketed(),
            Prop::new("preconditions", "event:Event/preconditions", ArgType::Str)
                .as_list()
                .opt()
                .bracketed(),
        ],
        kinds,
        subj: SubjKind::Generated("ev"),
    };

    let mut forms = BTreeMap::new();
    forms.insert(
        "event_model",
        FormSpec {
            class: "event:EventModel",
            props: vec![
                Prop::new("timeline", "event:EventModel/timeline", ArgType::Num).opt(),
                Prop::new(
                    "timeline_label",
                    "event:EventModel/timelineLabel",
                    ArgType::Str,
                )
                .opt(),
                // List props bracket in canonical form so sequential lists
                // (fluents, events, initiates, …) stay delimited on round-trip.
                Prop::new("fluents", "event:EventModel/fluents", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("events", "event:EventModel/events", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("initiates", "event:EventModel/initiates", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("terminates", "event:EventModel/terminates", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("initial", "event:EventModel/initial", ArgType::Str)
                    .as_list()
                    .opt()
                    .bracketed(),
                Prop::new(
                    "numeric_fluents",
                    "event:EventModel/numericFluents",
                    ArgType::Expr,
                )
                .as_list()
                .opt()
                .bracketed(),
                Prop::new(
                    "numeric_updates",
                    "event:EventModel/numericUpdates",
                    ArgType::Expr,
                )
                .as_list()
                .opt()
                .bracketed(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("em"),
        },
    );
    forms.insert(
        "fluent",
        FormSpec {
            class: "event:Fluent",
            props: vec![Prop::new("name", "event:Fluent/name", ArgType::Str)],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("fl"),
        },
    );
    forms.insert("event", event_form);
    forms.insert(
        "log",
        FormSpec {
            class: "event:EventLog",
            props: vec![Prop::new("cases", "event:EventLog/cases", ArgType::Expr)
                .as_list()
                .bracketed()],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("log"),
        },
    );
    forms.insert(
        "process",
        FormSpec {
            class: "event:ProcessDefinition",
            props: vec![
                Prop::new("name", "event:ProcessDefinition/name", ArgType::Str),
                Prop::new(
                    "activities",
                    "event:ProcessDefinition/activities",
                    ArgType::Str,
                )
                .as_list()
                .bracketed(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("proc"),
        },
    );
    forms.insert(
        "initiates",
        FormSpec {
            class: "event:Initiates",
            props: vec![
                Prop::new("event", "event:Initiates/event", ArgType::Str),
                Prop::new("fluent", "event:Initiates/fluent", ArgType::Str),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("ini"),
        },
    );
    forms.insert(
        "terminates",
        FormSpec {
            class: "event:Terminates",
            props: vec![
                Prop::new("event", "event:Terminates/event", ArgType::Str),
                Prop::new("fluent", "event:Terminates/fluent", ArgType::Str),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("ter"),
        },
    );
    forms.insert(
        "numeric_fluent",
        FormSpec {
            class: "event:NumericFluent",
            props: vec![
                Prop::new("name", "event:NumericFluent/name", ArgType::Str),
                Prop::new("initial", "event:NumericFluent/initial", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("nf"),
        },
    );
    forms.insert(
        "numeric_update",
        FormSpec {
            class: "event:NumericUpdate",
            props: vec![
                Prop::new("event", "event:NumericUpdate/event", ArgType::Str),
                Prop::new("fluent", "event:NumericUpdate/fluent", ArgType::Str),
                // op=set, operand=2.0 merged into one KindNode arg:
                // `numeric_update(ev, fl, set, 2.0)` — the op kind + operand
                // are consumed together and lowered to a NumericOp subnode.
                Prop::new("op", "event:NumericUpdate/op", ArgType::Kind)
                    .kind_node("event:NumericOp", "event:NumericOp/operand"),
            ],
            kinds: op_kinds,
            subj: SubjKind::Generated("nu"),
        },
    );

    Grammar {
        annotated_forms: Vec::new(),
        name: "event",
        schema_prefix: "event",
        entry: "problem",
        root_class: "event:EventProblem",
        root_links: vec![
            ("event_model", "event:EventProblem/model"),
            ("log", "event:EventProblem/log"),
            ("process", "event:EventProblem/process"),
        ],
        dynamic_predicates: false,
        kinds: BTreeMap::new(),
        entry_props: Vec::new(),
        forms,
        exprs: {
            let mut e = BTreeMap::new();
            e.insert(
                "case",
                FormSpec {
                    class: "event:Case",
                    props: vec![
                        Prop::new("id", "event:Case/id", ArgType::Str),
                        Prop::new("activities", "event:Case/activities", ArgType::Str)
                            .as_list()
                            .bracketed(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("cse"),
                },
            );
            e
        },
    }
}

// ── temporal (§5.5) ─────────────────────────────────────────────────────────

fn temporal() -> Grammar {
    let mut rel_kinds = BTreeMap::new();
    rel_kinds.insert(
        "relation",
        &[
            "before",
            "meets",
            "overlaps",
            "starts",
            "during",
            "finishes",
            "equals",
            "after",
            "metby",
            "overlappedby",
            "startedby",
            "contains",
            "finishedby",
        ][..],
    );
    let mut boundary_kinds = BTreeMap::new();
    boundary_kinds.insert("start_boundary", &["closed", "open"][..]);
    boundary_kinds.insert("end_boundary", &["closed", "open"][..]);
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert("ask", &["relations", "consistency"][..]);

    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "temporal:TemporalProblem",
            props: vec![
                Prop::new(
                    "intervals",
                    "temporal:TemporalProblem/intervals",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "constraints",
                    "temporal:TemporalProblem/constraints",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("ask", "temporal:TemporalProblem/ask", ArgType::Kind)
                    .kind_object_typed("temporal_data", "TemporalAsk")
                    .opt(),
            ],
            kinds: ask_kinds,
            subj: SubjKind::Generated("tp"),
        },
    );
    forms.insert(
        "point",
        FormSpec {
            class: "temporal:TimePoint",
            props: vec![
                Prop::new("label", "temporal:TimePoint/label", ArgType::Str),
                Prop::new("value", "temporal:TimePoint/value", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("pt"),
        },
    );
    forms.insert(
        "interval",
        FormSpec {
            class: "temporal:TimeInterval",
            props: vec![
                Prop::new("label", "temporal:TimeInterval/label", ArgType::Str),
                Prop::new("start", "temporal:TimeInterval/start", ArgType::Expr),
                Prop::new("end", "temporal:TimeInterval/end", ArgType::Expr),
                Prop::new(
                    "start_boundary",
                    "temporal:TimeInterval/startBoundary",
                    ArgType::Kind,
                )
                .kind_object_typed("temporal_data", "Boundary")
                .opt(),
                Prop::new(
                    "end_boundary",
                    "temporal:TimeInterval/endBoundary",
                    ArgType::Kind,
                )
                .kind_object_typed("temporal_data", "Boundary")
                .opt(),
            ],
            kinds: boundary_kinds,
            subj: SubjKind::Generated("iv"),
        },
    );
    forms.insert(
        "constraint",
        FormSpec {
            class: "temporal:TemporalConstraint",
            props: vec![
                Prop::new("a", "temporal:TemporalConstraint/a", ArgType::Str),
                Prop::new("b", "temporal:TemporalConstraint/b", ArgType::Str),
                Prop::new(
                    "relation",
                    "temporal:TemporalConstraint/relation",
                    ArgType::Kind,
                )
                .kind_object_typed("temporal_data", "TemporalRelation"),
                Prop::new("hard", "temporal:TemporalConstraint/hard", ArgType::Bool).opt(),
                Prop::new(
                    "certainty",
                    "temporal:TemporalConstraint/certainty",
                    ArgType::Num,
                )
                .opt(),
                Prop::new("weight", "temporal:TemporalConstraint/weight", ArgType::Num).opt(),
            ],
            kinds: rel_kinds,
            subj: SubjKind::Generated("tc"),
        },
    );

    Grammar {
        annotated_forms: Vec::new(),
        name: "temporal",
        schema_prefix: "temporal",
        entry: "problem",
        root_class: "temporal:TemporalProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── tensor (§5.7, engine surface) ───────────────────────────────────────────

/// The tensor engine currently runs on the numerical pipeline (see the
/// tensor SubsystemGuide: root `model:Model` of equations), so the tensor
/// module shares the algebra grammar. The aspirational `tensor.network(
/// tensors=[…])` surface lands with the tensor engine wiring; until then the
/// LLM-facing SEM is `tensor.model(equation(…))` and the goldened fixtures
/// are the same equation models.
fn tensor() -> Grammar {
    Grammar {
        annotated_forms: Vec::new(),
        name: "tensor",
        schema_prefix: "model",
        entry: "model",
        root_class: "model:Model",
        // Same root wiring as numerical: both `equation` and `differential`
        // link via `model:hasEquation`. Missing `differential` here would
        // compile the differential without a root link, and the renderer
        // (which walks root_links) would drop it entirely. The logic
        // statement forms link via `logic:fact`/`logic:rule` for hybrid
        // equation+rule documents.
        root_links: vec![
            ("equation", "model:hasEquation"),
            ("differential", "model:hasEquation"),
            ("fact", "logic:fact"),
            ("rule", "logic:rule"),
        ],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms: algebra_forms(),
        exprs: {
            let mut e = expr_specs();
            e.insert("outcome", generic_expr("outcome"));
            e.insert("bidirected", generic_expr("bidirected"));
            e
        },
    }
}

// ── information (§5.9) ──────────────────────────────────────────────────────

fn information() -> Grammar {
    let mut measure_kinds = BTreeMap::new();
    measure_kinds.insert(
        "kind",
        &[
            "entropy",
            "mutualinformation",
            "kldivergence",
            "jsdivergence",
            "crossentropy",
            "channelcapacity",
        ][..],
    );

    let mut forms = BTreeMap::new();
    forms.insert(
        "query",
        FormSpec {
            class: "info:InformationQuery",
            props: vec![
                Prop::new(
                    "distributions",
                    "info:InformationQuery/distributions",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("joints", "info:InformationQuery/joints", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("joints3", "info:InformationQuery/joints3", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("channels", "info:InformationQuery/channels", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("measures", "info:InformationQuery/measures", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("iq"),
        },
    );
    forms.insert(
        "distribution",
        FormSpec {
            class: "info:CategoricalDistribution",
            props: vec![
                Prop::new("name", "info:CategoricalDistribution/name", ArgType::Str),
                Prop::new(
                    "outcomes",
                    "info:CategoricalDistribution/outcomes",
                    ArgType::Str,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "probabilities",
                    "info:CategoricalDistribution/probabilities",
                    ArgType::Num,
                )
                .as_list()
                .bracketed(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("cd"),
        },
    );
    forms.insert(
        "joint",
        FormSpec {
            class: "info:JointDistribution",
            props: vec![
                Prop::new("name", "info:JointDistribution/name", ArgType::Str),
                Prop::new(
                    "x_outcomes",
                    "info:JointDistribution/xOutcomes",
                    ArgType::Str,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "y_outcomes",
                    "info:JointDistribution/yOutcomes",
                    ArgType::Str,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "probabilities",
                    "info:JointDistribution/probabilities",
                    ArgType::Num,
                )
                .as_list()
                .bracketed(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("jd"),
        },
    );
    forms.insert(
        "channel",
        FormSpec {
            class: "info:Channel",
            props: vec![
                Prop::new("name", "info:Channel/name", ArgType::Str),
                Prop::new("input", "info:Channel/input", ArgType::Str)
                    .as_list()
                    .bracketed(),
                Prop::new("output", "info:Channel/output", ArgType::Str)
                    .as_list()
                    .bracketed(),
                Prop::new("transition", "info:Channel/transition", ArgType::Num)
                    .as_list()
                    .bracketed(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("ch"),
        },
    );
    forms.insert(
        "measure",
        FormSpec {
            class: "info:MeasureRequest",
            props: vec![
                // measurekind nodes are untyped constants (no rdf:type in the
                // goldened fixtures) — R10 inverse, `kind_object` not typed.
                Prop::new("kind", "info:MeasureRequest/kind", ArgType::Kind)
                    .kind_object("info_data", "MeasureKind"),
                Prop::new("first", "info:MeasureRequest/first", ArgType::Str),
                Prop::new("second", "info:MeasureRequest/second", ArgType::Str).opt(),
                Prop::new("joint", "info:MeasureRequest/joint", ArgType::Str).opt(),
            ],
            kinds: measure_kinds,
            subj: SubjKind::Generated("mr"),
        },
    );

    Grammar {
        annotated_forms: Vec::new(),
        name: "information",
        schema_prefix: "info",
        entry: "query",
        root_class: "info:InformationQuery",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── learning (§5.8) ─────────────────────────────────────────────────────────

fn learning() -> Grammar {
    let mut algo_kinds = BTreeMap::new();
    algo_kinds.insert("algorithm", &["valueiteration", "qlearning"][..]);

    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "learning:LearningProblem",
            props: vec![
                Prop::new("name", "learning:LearningProblem/name", ArgType::Str),
                Prop::new("states", "learning:LearningProblem/states", ArgType::Str)
                    .as_list()
                    .bracketed(),
                Prop::new("actions", "learning:LearningProblem/actions", ArgType::Str)
                    .as_list()
                    .bracketed(),
                Prop::new(
                    "algorithm",
                    "learning:LearningProblem/algorithm",
                    ArgType::Kind,
                )
                .kind_object_typed("learning_data", "LearningAlgorithm"),
                Prop::new(
                    "learning_rate",
                    "learning:LearningProblem/learningRate",
                    ArgType::Num,
                )
                .opt(),
                Prop::new(
                    "discount",
                    "learning:LearningProblem/discount",
                    ArgType::Num,
                )
                .opt(),
                Prop::new(
                    "experiences",
                    "learning:LearningProblem/experiences",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
            ],
            kinds: algo_kinds,
            subj: SubjKind::Generated("lp"),
        },
    );
    forms.insert(
        "experience",
        FormSpec {
            class: "learning:Experience",
            props: vec![
                Prop::new("state", "learning:Experience/state", ArgType::Str),
                Prop::new("action", "learning:Experience/action", ArgType::Str),
                Prop::new("next_state", "learning:Experience/nextState", ArgType::Str),
                Prop::new("reward", "learning:Experience/reward", ArgType::Num),
                Prop::new("terminal", "learning:Experience/terminal", ArgType::Bool),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("ex"),
        },
    );

    Grammar {
        annotated_forms: Vec::new(),
        name: "learning",
        schema_prefix: "learning",
        entry: "problem",
        root_class: "learning:LearningProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── memory (§5.15) ──────────────────────────────────────────────────────────

fn memory() -> Grammar {
    let mut mem_kinds = BTreeMap::new();
    mem_kinds.insert(
        "kind",
        &[
            "episodic",
            "semantic",
            "working",
            "procedural",
            "metacognitive",
        ][..],
    );
    let mut ask_kinds = BTreeMap::new();
    // Keyed by the prop name the validator looks up (`asks`), not the arg name.
    ask_kinds.insert(
        "asks",
        &[
            "retrieve",
            "consolidate",
            "trajectory",
            "forget",
            "patterncomplete",
        ][..],
    );
    let mut outcome_kinds = BTreeMap::new();
    outcome_kinds.insert("outcome", &["success", "failure"][..]);

    let mut forms = BTreeMap::new();
    forms.insert(
        "memory",
        FormSpec {
            class: "memory:Memory",
            props: vec![
                Prop::new("id", "memory:Memory/id", ArgType::Str),
                Prop::new("content", "memory:Memory/content", ArgType::Str),
                Prop::new("strength", "memory:Memory/strength", ArgType::Num),
                Prop::new("confidence", "memory:Memory/confidence", ArgType::Num),
                Prop::new("kind", "memory:Memory/kind", ArgType::Kind)
                    .kind_object_typed("memory_data", "MemoryKind"),
                Prop::new("evidence", "memory:Memory/evidence", ArgType::Str)
                    .as_list()
                    .opt(),
            ],
            kinds: mem_kinds,
            subj: SubjKind::Generated("mem"),
        },
    );
    forms.insert(
        "trajectory",
        FormSpec {
            class: "memory:ReasoningTrajectory",
            props: vec![
                Prop::new("id", "memory:ReasoningTrajectory/id", ArgType::Str),
                Prop::new("steps", "memory:ReasoningTrajectory/steps", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new("edges", "memory:ReasoningTrajectory/edges", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("rt"),
        },
    );
    forms.insert(
        "step",
        FormSpec {
            class: "memory:TrajectoryStep",
            props: vec![
                Prop::new("id", "memory:TrajectoryStep/id", ArgType::Str),
                Prop::new("label", "memory:TrajectoryStep/label", ArgType::Str),
                Prop::new("outcome", "memory:TrajectoryStep/outcome", ArgType::Kind)
                    .kind_object_typed("memory_data", "StepOutcome"),
            ],
            kinds: outcome_kinds,
            subj: SubjKind::Generated("ts"),
        },
    );
    forms.insert(
        "edge",
        FormSpec {
            class: "memory:TrajectoryEdge",
            props: vec![
                Prop::new("from", "memory:TrajectoryEdge/from", ArgType::Str),
                Prop::new("to", "memory:TrajectoryEdge/to", ArgType::Str),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("te"),
        },
    );
    // `asks` is a list of kind-object constants (`MemoryAsk/Retrieve`, …),
    // rendered as bare kind symbols — no separate `ask()` form exists in the
    // goldened fixtures.
    let mut problem_props = vec![
        Prop::new("name", "memory:MemoryProblem/name", ArgType::Str),
        Prop::new("query", "memory:MemoryProblem/query", ArgType::Str),
        Prop::new("memories", "memory:MemoryProblem/memories", ArgType::Expr)
            .as_list()
            .bracketed(),
        Prop::new(
            "trajectories",
            "memory:MemoryProblem/trajectories",
            ArgType::Expr,
        )
        .as_list()
        .bracketed()
        .opt(),
    ];
    problem_props.push(
        Prop::new("asks", "memory:MemoryProblem/asks", ArgType::Kind)
            .as_list()
            .bracketed()
            .opt()
            .kind_object_typed("memory_data", "MemoryAsk"),
    );
    forms.insert(
        "problem",
        FormSpec {
            class: "memory:MemoryProblem",
            props: problem_props,
            kinds: ask_kinds,
            subj: SubjKind::Generated("mp"),
        },
    );

    Grammar {
        annotated_forms: Vec::new(),
        name: "memory",
        schema_prefix: "memory",
        entry: "problem",
        root_class: "memory:MemoryProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── complexity (§5.10) ──────────────────────────────────────────────────────

fn complexity() -> Grammar {
    let mut forms = BTreeMap::new();
    forms.insert(
        "query",
        FormSpec {
            class: "complexity:ComplexityQuery",
            props: vec![
                Prop::new("size", "complexity:ComplexityQuery/size", ArgType::Num),
                Prop::new("budget", "complexity:ComplexityQuery/budget", ArgType::Expr).opt(),
                Prop::new(
                    "algorithms",
                    "complexity:ComplexityQuery/algorithms",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed(),
                Prop::new("select", "complexity:ComplexityQuery/select", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new(
                    "classify",
                    "complexity:ComplexityQuery/classify",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("assess", "complexity:ComplexityQuery/assess", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new(
                    "predict",
                    "complexity:ComplexityQuery/predict",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "hardware_factor",
                    "complexity:ComplexityQuery/hardwareFactor",
                    ArgType::Num,
                )
                .opt(),
                // Reduction-derived hardness verdict (§12): the problems that
                // carry an NP-hard/#P-hard/PSPACE verdict, so selection demotes
                // their exact general routes under budget pressure and prefers
                // parametric/approximate ones. `complexity.query(hard=["routing"], …)`.
                Prop::new(
                    "hard",
                    "complexity:ComplexityQuery/hardProblems",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("cq"),
        },
    );
    forms.insert(
        "algorithm",
        FormSpec {
            class: "complexity:Algorithm",
            props: vec![
                Prop::new("name", "complexity:Algorithm/name", ArgType::Str),
                Prop::new("time", "complexity:Algorithm/time", ArgType::Expr),
                Prop::new("space", "complexity:Algorithm/space", ArgType::Expr),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("algo"),
        },
    );
    forms.insert(
        "budget",
        FormSpec {
            class: "complexity:ResourceBudget",
            props: vec![
                Prop::new(
                    "time_seconds",
                    "complexity:ResourceBudget/timeSeconds",
                    ArgType::Num,
                )
                .opt(),
                Prop::new(
                    "unit_system",
                    "complexity:ResourceBudget/unitSystem",
                    ArgType::Str,
                ),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("rb"),
        },
    );
    forms.insert(
        "selection_group",
        FormSpec {
            class: "complexity:SelectionGroup",
            props: vec![
                Prop::new("problem", "complexity:SelectionGroup/problem", ArgType::Str),
                Prop::new(
                    "candidates",
                    "complexity:SelectionGroup/candidates",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("sg"),
        },
    );
    forms.insert(
        "candidate",
        FormSpec {
            class: "complexity:SelectionCandidateSpec",
            props: vec![
                Prop::new(
                    "algorithm",
                    "complexity:SelectionCandidateSpec/algorithm",
                    ArgType::Str,
                ),
                Prop::new(
                    "static_cost",
                    "complexity:SelectionCandidateSpec/staticCost",
                    ArgType::Num,
                ),
                Prop::new(
                    "exact_general",
                    "complexity:SelectionCandidateSpec/exactGeneral",
                    ArgType::Bool,
                ),
                Prop::new(
                    "approximate_route",
                    "complexity:SelectionCandidateSpec/approximateRoute",
                    ArgType::Bool,
                ),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("sc"),
        },
    );

    // Complexity expressions (§5.10): inlined R8 forms typed as
    // ComplexityExpr/<Kind> — classes with a single optional scalar prop.
    let mut exprs = BTreeMap::new();
    let mut add_expr = |name: &'static str, class: &'static str, props: Vec<Prop>| {
        exprs.insert(
            name,
            FormSpec {
                class,
                props,
                kinds: BTreeMap::new(),
                subj: SubjKind::Generated("e"),
            },
        );
    };
    add_expr("linear", "complexity:ComplexityExpr/Linear", vec![]);
    add_expr("quadratic", "complexity:ComplexityExpr/Quadratic", vec![]);
    add_expr(
        "logarithmic",
        "complexity:ComplexityExpr/Logarithmic",
        vec![],
    );
    add_expr(
        "linearithmic",
        "complexity:ComplexityExpr/Linearithmic",
        vec![],
    );
    add_expr("constant", "complexity:ComplexityExpr/Constant", vec![]);
    add_expr(
        "invariant",
        "abstraction:Invariant",
        vec![Prop::new("name", "abstraction:Invariant/name", ArgType::Str).opt()],
    );
    add_expr(
        "exponential",
        "complexity:ComplexityExpr/Exponential",
        vec![Prop::new("base", "complexity:ComplexityExpr/base", ArgType::Num).opt()],
    );
    add_expr(
        "polynomial",
        "complexity:ComplexityExpr/Polynomial",
        vec![Prop::new(
            "degree",
            "complexity:ComplexityExpr/degree",
            ArgType::Num,
        )],
    );

    Grammar {
        annotated_forms: Vec::new(),
        name: "complexity",
        schema_prefix: "complexity",
        entry: "query",
        root_class: "complexity:ComplexityQuery",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs,
    }
}

// ── argumentation (§5.11) ────────────────────────────────────────────────────

fn argumentation() -> Grammar {
    let mut forms = BTreeMap::new();
    forms.insert(
        "argument",
        FormSpec {
            class: "argumentation:Argument",
            props: vec![
                Prop::new("id", "argumentation:Argument/id", ArgType::Str),
                Prop::new("claim", "argumentation:Argument/conclusion", ArgType::Str).opt(),
                Prop::new("premises", "argumentation:Argument/premises", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new(
                    "assumptions",
                    "argumentation:Argument/assumptions",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("evidence", "argumentation:Argument/evidence", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("scheme", "argumentation:Argument/scheme", ArgType::Kind)
                    .opt()
                    .kind_object_typed("argumentation_data", "ArgumentScheme"),
            ],
            kinds: {
                let mut m = BTreeMap::new();
                m.insert(
                    "scheme",
                    &[
                        "empirical",
                        "authority",
                        "rebuttal",
                        "modusponens",
                        "causal",
                        "defeasible",
                    ][..],
                );
                m
            },
            subj: SubjKind::Generated("arg"),
        },
    );
    forms.insert(
        "claim",
        FormSpec {
            class: "argumentation:Claim",
            props: vec![
                Prop::new("id", "argumentation:Claim/id", ArgType::Str),
                Prop::new("statement", "argumentation:Claim/statement", ArgType::Str),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("cl"),
        },
    );
    forms.insert(
        "attack",
        FormSpec {
            class: "argumentation:Attack",
            props: vec![
                Prop::new("attacker", "argumentation:Attack/from", ArgType::Str),
                Prop::new("attacked", "argumentation:Attack/to", ArgType::Str),
                Prop::new("kind", "argumentation:Attack/kind", ArgType::Kind)
                    .opt()
                    .kind_object_typed("argumentation_data", "AttackKind"),
            ],
            kinds: {
                let mut k = BTreeMap::new();
                k.insert("kind", &["rebuttal", "undercut", "undermine"][..]);
                k
            },
            subj: SubjKind::Generated("atk"),
        },
    );
    forms.insert(
        "framework",
        FormSpec {
            class: "argumentation:ArgumentationProblem",
            props: vec![Prop::new(
                "arguments",
                "argumentation:ArgumentationProblem/arguments",
                ArgType::Expr,
            )
            .as_list()
            .bracketed()
            .opt()],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("ap"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "argumentation",
        schema_prefix: "argumentation",
        entry: "problem",
        root_class: "argumentation:ArgumentationProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms: {
            forms.insert(
                "problem",
                FormSpec {
                    class: "argumentation:ArgumentationProblem",
                    props: vec![
                        Prop::new(
                            "name",
                            "argumentation:ArgumentationProblem/name",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "claims",
                            "argumentation:ArgumentationProblem/claims",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed(),
                        Prop::new(
                            "arguments",
                            "argumentation:ArgumentationProblem/arguments",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed(),
                        Prop::new(
                            "attacks",
                            "argumentation:ArgumentationProblem/attacks",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed(),
                        Prop::new(
                            "supports",
                            "argumentation:ArgumentationProblem/supports",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "semantics",
                            "argumentation:ArgumentationProblem/semantics",
                            ArgType::Kind,
                        )
                        .kind_object("argument_data", "ArgumentationSemantics"),
                        Prop::new(
                            "asks",
                            "argumentation:ArgumentationProblem/asks",
                            ArgType::Kind,
                        )
                        .as_list()
                        .bracketed()
                        .opt()
                        .kind_object_typed("argumentation_data", "ArgumentationAsk"),
                    ],
                    kinds: {
                        let mut k = BTreeMap::new();
                        k.insert("semantics", &["grounded", "preferred", "stable"][..]);
                        k.insert("asks", &["accepted", "evaluation", "extensions"][..]);
                        k
                    },
                    subj: SubjKind::Generated("ap"),
                },
            );
            forms
        },
        exprs: {
            let mut e = BTreeMap::new();
            for name in ["claim", "truth_entry", "proposition", "outcome"] {
                e.insert(name, generic_expr(name));
            }
            e.insert(
                "support",
                FormSpec {
                    class: "argumentation:Support",
                    props: vec![
                        Prop::new("from", "argumentation:Support/from", ArgType::Str),
                        Prop::new("to", "argumentation:Support/to", ArgType::Str),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("sp"),
                },
            );
            e
        },
    }
}

// ── provenance (§5.12) ──────────────────────────────────────────────────────

fn provenance() -> Grammar {
    let mut kinds = BTreeMap::new();
    kinds.insert("traversal", &["upstream", "downstream"][..]);
    kinds.insert(
        "asks",
        &[
            "trustscores",
            "epistemicstatus",
            "conflictresolution",
            "sourceupdate",
            "lineage",
            "contradictions",
        ][..],
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "provenance",
        schema_prefix: "provenance",
        entry: "problem",
        root_class: "provenance:ProvenanceProblem",
        root_links: vec![],
        dynamic_predicates: false,
        kinds: {
            let mut k = kinds;
            k.insert("asks", &["trustscores", "lineage", "contradictions"][..]);
            k
        },
        entry_props: Vec::new(),
        forms: {
            let mut f = BTreeMap::new();
            f.insert(
                "problem",
                FormSpec {
                    class: "provenance:ProvenanceProblem",
                    props: vec![
                        Prop::new("name", "provenance:ProvenanceProblem/name", ArgType::Str),
                        Prop::new(
                            "sources",
                            "provenance:ProvenanceProblem/sources",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "claims",
                            "provenance:ProvenanceProblem/claims",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "observations",
                            "provenance:ProvenanceProblem/observations",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "contradictions",
                            "provenance:ProvenanceProblem/contradictions",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new("asks", "provenance:ProvenanceProblem/asks", ArgType::Kind)
                            .as_list()
                            .bracketed()
                            .opt()
                            .kind_object_typed("provenance_data", "ProvenanceAsk"),
                    ],
                    kinds: {
                        let mut k = BTreeMap::new();
                        k.insert(
                            "asks",
                            &[
                                "trustscores",
                                "epistemicstatus",
                                "conflictresolution",
                                "sourceupdate",
                                "lineage",
                                "contradictions",
                            ][..],
                        );
                        k
                    },
                    subj: SubjKind::Generated("pp"),
                },
            );
            f
        },
        exprs: {
            let mut e = BTreeMap::new();
            e.insert(
                "source",
                FormSpec {
                    class: "provenance:KnowledgeSource",
                    props: vec![
                        Prop::new("id", "provenance:KnowledgeSource/id", ArgType::Str),
                        Prop::new(
                            "reliability",
                            "provenance:KnowledgeSource/reliability",
                            ArgType::Num,
                        ),
                        Prop::new("kind", "provenance:KnowledgeSource/kind", ArgType::Kind)
                            .kind_object("provenance_data", "sourcekind"),
                    ],
                    kinds: {
                        let mut k = BTreeMap::new();
                        k.insert(
                            "kind",
                            &[
                                "sensor", "document", "human", "model", "expert", "llm", "database",
                            ][..],
                        );
                        k
                    },
                    subj: SubjKind::Generated("src"),
                },
            );
            e.insert(
                "claim",
                FormSpec {
                    class: "provenance:KnowledgeClaim",
                    props: vec![
                        Prop::new("id", "provenance:KnowledgeClaim/id", ArgType::Str),
                        Prop::new(
                            "statement",
                            "provenance:KnowledgeClaim/statement",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "reported_by",
                            "provenance:KnowledgeClaim/reportedBy",
                            ArgType::Str,
                        )
                        .as_list()
                        .bracketed()
                        .opt()
                        .kwarg(),
                        Prop::new(
                            "derived_from",
                            "provenance:KnowledgeClaim/derivedFrom",
                            ArgType::Str,
                        )
                        .as_list()
                        .bracketed()
                        .opt()
                        .kwarg(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("cl"),
                },
            );
            e.insert(
                "contradiction",
                FormSpec {
                    class: "provenance:Contradiction",
                    props: vec![
                        Prop::new("a", "provenance:Contradiction/a", ArgType::Str),
                        Prop::new("b", "provenance:Contradiction/b", ArgType::Str),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("con"),
                },
            );
            e.insert(
                "observation",
                FormSpec {
                    class: "provenance:EvidenceObservation",
                    props: vec![
                        Prop::new(
                            "source",
                            "provenance:EvidenceObservation/source",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "claim",
                            "provenance:EvidenceObservation/claim",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "asserts",
                            "provenance:EvidenceObservation/asserts",
                            ArgType::Bool,
                        )
                        .kwarg(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("obs"),
                },
            );
            e
        },
    }
}

// ── evolution (§5.13) ───────────────────────────────────────────────────────

fn evolution() -> Grammar {
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert(
        "asks",
        &[
            "replicatorstep",
            "replicatormutatorstep",
            "fixedpoints",
            "ess",
        ][..],
    );
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "evolution:EvolutionProblem",
            props: vec![
                Prop::new("name", "evolution:EvolutionProblem/name", ArgType::Str),
                Prop::new("steps", "evolution:EvolutionProblem/steps", ArgType::Num),
                Prop::new("traits", "evolution:EvolutionProblem/traits", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new(
                    "payoffs",
                    "evolution:EvolutionProblem/payoffs",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed(),
                Prop::new("asks", "evolution:EvolutionProblem/asks", ArgType::Kind)
                    .as_list()
                    .bracketed()
                    .opt()
                    .kind_object_typed("evolution_data", "EvolutionAsk"),
            ],
            kinds: ask_kinds,
            subj: SubjKind::Generated("evp"),
        },
    );
    forms.insert(
        "trait",
        FormSpec {
            class: "evolution:Trait",
            props: vec![
                Prop::new("name", "evolution:Trait/name", ArgType::Str),
                Prop::new("frequency", "evolution:Trait/frequency", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("tr"),
        },
    );
    forms.insert(
        "payoff",
        FormSpec {
            class: "evolution:Payoff",
            props: vec![
                Prop::new("row", "evolution:Payoff/row", ArgType::Str),
                Prop::new("column", "evolution:Payoff/column", ArgType::Str),
                Prop::new("value", "evolution:Payoff/value", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("po"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "evolution",
        schema_prefix: "evolution",
        entry: "problem",
        root_class: "evolution:EvolutionProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── analogy (§5.14) ─────────────────────────────────────────────────────────

fn analogy() -> Grammar {
    let mut el_kinds = BTreeMap::new();
    el_kinds.insert(
        "kind",
        &["variable", "equation", "transition", "entity", "relation"][..],
    );
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert("asks", &["correspondences", "similarity", "transfer"][..]);
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "analogy:AnalogyProblem",
            props: vec![
                Prop::new("name", "analogy:AnalogyProblem/name", ArgType::Str),
                Prop::new("source", "analogy:AnalogyProblem/source", ArgType::Expr),
                Prop::new("target", "analogy:AnalogyProblem/target", ArgType::Expr),
                Prop::new("asks", "analogy:AnalogyProblem/asks", ArgType::Kind)
                    .as_list()
                    .bracketed()
                    .opt()
                    .kind_object_typed("analogy_data", "AnalogyAsk"),
            ],
            kinds: ask_kinds,
            subj: SubjKind::Generated("ap"),
        },
    );
    forms.insert(
        "case",
        FormSpec {
            class: "analogy:Case",
            props: vec![
                Prop::new("name", "analogy:Case/name", ArgType::Str),
                Prop::new("elements", "analogy:Case/elements", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("cs"),
        },
    );
    forms.insert(
        "element",
        FormSpec {
            class: "analogy:CaseElement",
            props: vec![
                Prop::new("id", "analogy:CaseElement/id", ArgType::Str),
                Prop::new("label", "analogy:CaseElement/label", ArgType::Str).opt(),
                Prop::new("kind", "analogy:CaseElement/kind", ArgType::Kind)
                    .kind_object_typed("analogy_data", "ElementKind"),
                Prop::new("arity", "analogy:CaseElement/arity", ArgType::Num).opt(),
            ],
            kinds: el_kinds,
            subj: SubjKind::Generated("el"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "analogy",
        schema_prefix: "analogy",
        entry: "problem",
        root_class: "analogy:AnalogyProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── symreg (§5.17) ──────────────────────────────────────────────────────────

fn symreg() -> Grammar {
    let mut obj_kinds = BTreeMap::new();
    obj_kinds.insert("objectives", &["accuracy", "complexity"][..]);
    let mut op_kinds = BTreeMap::new();
    op_kinds.insert("operators", &["add", "mul", "sub", "pow"][..]);
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "symreg:SymbolicRegressionProblem",
            props: vec![
                Prop::new(
                    "name",
                    "symreg:SymbolicRegressionProblem/name",
                    ArgType::Str,
                ),
                Prop::new(
                    "variables",
                    "symreg:SymbolicRegressionProblem/variables",
                    ArgType::Str,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "data",
                    "symreg:SymbolicRegressionProblem/data",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "grammar",
                    "symreg:SymbolicRegressionProblem/grammar",
                    ArgType::Expr,
                ),
                Prop::new(
                    "objectives",
                    "symreg:SymbolicRegressionProblem/objectives",
                    ArgType::Kind,
                )
                .as_list()
                .bracketed()
                .kind_object_typed("symreg_data", "Objective"),
                Prop::new(
                    "max_candidates",
                    "symreg:SymbolicRegressionProblem/maxCandidates",
                    ArgType::Num,
                )
                .opt(),
                Prop::new(
                    "max_depth",
                    "symreg:SymbolicRegressionProblem/maxDepth",
                    ArgType::Num,
                )
                .opt(),
            ],
            kinds: obj_kinds,
            subj: SubjKind::Generated("srp"),
        },
    );
    forms.insert(
        "datapoint",
        FormSpec {
            class: "symreg:DataPoint",
            props: vec![
                Prop::new("inputs", "symreg:DataPoint/inputs", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new("output", "symreg:DataPoint/output", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("dp"),
        },
    );
    forms.insert(
        "binding",
        FormSpec {
            class: "symreg:Binding",
            props: vec![
                Prop::new("name", "symreg:Binding/name", ArgType::Str),
                Prop::new("value", "symreg:Binding/value", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("bd"),
        },
    );
    forms.insert(
        "grammar",
        FormSpec {
            class: "symreg:Grammar",
            props: vec![
                Prop::new("constants", "symreg:Grammar/constants", ArgType::Num)
                    .as_list()
                    .bracketed(),
                Prop::new("operators", "symreg:Grammar/operators", ArgType::Kind)
                    .as_list()
                    .bracketed()
                    .kind_object_typed("symreg_data", "Operator"),
            ],
            kinds: op_kinds,
            subj: SubjKind::Generated("gr"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "symreg",
        schema_prefix: "symreg",
        entry: "problem",
        root_class: "symreg:SymbolicRegressionProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── synthesis (§5.17) ───────────────────────────────────────────────────────

fn synthesis() -> Grammar {
    let mut op_kinds = BTreeMap::new();
    op_kinds.insert("operators", &["add", "mul", "sub"][..]);
    let mut cmp_kinds = BTreeMap::new();
    cmp_kinds.insert("comparisons", &["lt", "gt", "le", "ge", "eq"][..]);
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "synthesis:SynthesisProblem",
            props: vec![
                Prop::new("name", "synthesis:SynthesisProblem/name", ArgType::Str),
                Prop::new(
                    "variables",
                    "synthesis:SynthesisProblem/variables",
                    ArgType::Str,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "examples",
                    "synthesis:SynthesisProblem/examples",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "grammar",
                    "synthesis:SynthesisProblem/grammar",
                    ArgType::Expr,
                ),
                Prop::new(
                    "specification",
                    "synthesis:SynthesisProblem/specification",
                    ArgType::Expr,
                )
                .opt(),
                Prop::new(
                    "max_depth",
                    "synthesis:SynthesisProblem/maxDepth",
                    ArgType::Num,
                )
                .opt(),
                Prop::new(
                    "max_programs",
                    "synthesis:SynthesisProblem/maxPrograms",
                    ArgType::Num,
                )
                .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("sp"),
        },
    );
    forms.insert(
        "example",
        FormSpec {
            class: "synthesis:Example",
            props: vec![
                Prop::new("inputs", "synthesis:Example/inputs", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new("output", "synthesis:Example/output", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("exm"),
        },
    );
    forms.insert(
        "binding",
        FormSpec {
            class: "synthesis:Binding",
            props: vec![
                Prop::new("name", "synthesis:Binding/name", ArgType::Str),
                Prop::new("value", "synthesis:Binding/value", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("bnd"),
        },
    );
    // Both kind sets keyed by prop name (operators + comparisons) live on the
    // same form, so merge them into the single `kinds` map.
    let mut grammar_kinds = BTreeMap::new();
    for (k, v) in op_kinds.iter() {
        grammar_kinds.insert(*k, *v);
    }
    for (k, v) in cmp_kinds.iter() {
        grammar_kinds.insert(*k, *v);
    }
    forms.insert(
        "grammar",
        FormSpec {
            class: "synthesis:ProgramGrammar",
            props: vec![
                Prop::new(
                    "constants",
                    "synthesis:ProgramGrammar/constants",
                    ArgType::Num,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "operators",
                    "synthesis:ProgramGrammar/operators",
                    ArgType::Kind,
                )
                .as_list()
                .bracketed()
                .kind_object_typed("synthesis_data", "Operator"),
                Prop::new(
                    "comparisons",
                    "synthesis:ProgramGrammar/comparisons",
                    ArgType::Kind,
                )
                .as_list()
                .bracketed()
                .opt()
                .kind_object_typed("synthesis_data", "CmpOp"),
            ],
            kinds: grammar_kinds,
            subj: SubjKind::Generated("pg"),
        },
    );
    forms.insert(
        "specification",
        FormSpec {
            class: "synthesis:Specification",
            props: vec![
                Prop::new(
                    "preconditions",
                    "synthesis:Specification/preconditions",
                    ArgType::Str,
                )
                .as_list()
                .opt(),
                Prop::new(
                    "postconditions",
                    "synthesis:Specification/postconditions",
                    ArgType::Str,
                )
                .as_list()
                .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("spc"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "synthesis",
        schema_prefix: "synthesis",
        entry: "problem",
        root_class: "synthesis:SynthesisProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

// ── causal (§5.4, provisional engine surface) ────────────────────────────────

fn causal() -> Grammar {
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert("ask", &["estimateeffect", "identify", "counterfactual"][..]);
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "causal:CausalProblem",
            props: vec![
                Prop::new("name", "causal:CausalProblem/name", ArgType::Str),
                Prop::new("scm", "causal:CausalProblem/scm", ArgType::Expr),
                Prop::new(
                    "actual_world",
                    "causal:CausalProblem/actualWorld",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "counterfactuals",
                    "causal:CausalProblem/counterfactuals",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("estimands", "causal:CausalProblem/estimands", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("queries", "causal:CausalProblem/queries", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("ask", "causal:CausalProblem/ask", ArgType::Kind)
                    .opt()
                    .kind_object_typed("causal_data", "CausalAsk"),
            ],
            kinds: ask_kinds.clone(),
            subj: SubjKind::Generated("cp"),
        },
    );
    forms.insert(
        "scm",
        FormSpec {
            class: "causal:Scm",
            props: vec![
                Prop::new("variables", "causal:Scm/variables", ArgType::Str)
                    .as_list()
                    .bracketed(),
                Prop::new("graph", "causal:Scm/graph", ArgType::Expr).opt(),
                Prop::new("exogenous", "causal:Scm/exogenous", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("equations", "causal:Scm/equations", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("scm"),
        },
    );
    // `observe` doubles as a content form so a bare top-level
    // `causal.observe("y")` auto-roots (R9) and the renderer can emit it as
    // a standalone call; nested inside `queries=[...]` it still compiles via
    // the same spec.
    forms.insert(
        "observe",
        FormSpec {
            class: "causal:CausalQuery/Observe",
            props: vec![Prop::new(
                "variable",
                "causal:CausalQuery/variable",
                ArgType::Str,
            )],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("qo"),
        },
    );
    forms.insert(
        "bidirected",
        FormSpec {
            class: "causal:BidirectedEdge",
            props: vec![
                Prop::new("left", "causal:BidirectedEdge/left", ArgType::Str),
                Prop::new("right", "causal:BidirectedEdge/right", ArgType::Str),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("be"),
        },
    );
    forms.insert(
        "outcome",
        FormSpec {
            class: "causal:Outcome",
            props: vec![
                Prop::new("value", "causal:Outcome/value", ArgType::Num),
                Prop::new("probability", "causal:Outcome/probability", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("oc"),
        },
    );
    forms.insert(
        "graph",
        FormSpec {
            class: "causal:CausalGraph",
            props: vec![
                Prop::new("nodes", "causal:CausalGraph/nodes", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("bidirected", "causal:CausalGraph/bidirected", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("edges", "causal:CausalGraph/edges", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("cg"),
        },
    );
    forms.insert(
        "exogenous_var",
        FormSpec {
            class: "causal:ExogenousVar",
            props: vec![
                Prop::new("name", "causal:ExogenousVar/name", ArgType::Str),
                Prop::new(
                    "distribution",
                    "causal:ExogenousVar/distribution",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("exv"),
        },
    );
    forms.insert(
        "structural_equation",
        FormSpec {
            class: "causal:StructuralEquation",
            props: vec![
                Prop::new(
                    "variable",
                    "causal:StructuralEquation/variable",
                    ArgType::Str,
                ),
                Prop::new(
                    "formula",
                    "causal:StructuralEquation/formula",
                    ArgType::Expr,
                ),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("se"),
        },
    );
    forms.insert(
        "edge",
        FormSpec {
            class: "causal:Edge",
            props: vec![
                Prop::new("parent", "causal:Edge/parent", ArgType::Str),
                Prop::new("child", "causal:Edge/child", ArgType::Str),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("ed"),
        },
    );
    forms.insert(
        "counterfactual",
        FormSpec {
            class: "causal:CounterfactualQuery",
            props: vec![
                Prop::new(
                    "intervention",
                    "causal:CounterfactualQuery/intervention",
                    ArgType::Expr,
                ),
                Prop::new(
                    "outcome",
                    "causal:CounterfactualQuery/outcome",
                    ArgType::Str,
                ),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("cf"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "causal",
        schema_prefix: "causal",
        entry: "problem",
        root_class: "causal:CausalProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: {
            let mut e = expr_specs();
            e.insert(
                "imagine",
                FormSpec {
                    class: "causal:CausalQuery/Imagine",
                    props: vec![
                        Prop::new(
                            "intervention",
                            "causal:CausalQuery/intervention",
                            ArgType::Expr,
                        ),
                        Prop::new(
                            "assumptions",
                            "causal:CausalQuery/assumptions",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("qi"),
                },
            );
            e.insert(
                "outcome",
                FormSpec {
                    class: "causal:Outcome",
                    props: vec![
                        Prop::new("value", "causal:Outcome/value", ArgType::Num),
                        Prop::new("probability", "causal:Outcome/probability", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("oc"),
                },
            );
            e.insert(
                "set",
                FormSpec {
                    class: "causal:CausalQuery/Set",
                    props: vec![
                        Prop::new("variable", "causal:CausalQuery/variable", ArgType::Str),
                        Prop::new("value", "causal:CausalQuery/value", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("qs"),
                },
            );
            e.insert(
                "assume_query",
                FormSpec {
                    class: "causal:CausalQuery/Assume",
                    props: vec![
                        Prop::new("variable", "causal:CausalQuery/variable", ArgType::Str),
                        Prop::new("value", "causal:CausalQuery/value", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("qa"),
                },
            );
            e.insert(
                "estimand",
                FormSpec {
                    class: "causal:Estimand",
                    props: vec![
                        Prop::new("treatment", "causal:Estimand/treatment", ArgType::Str),
                        Prop::new("outcome", "causal:Estimand/outcome", ArgType::Str),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("est"),
                },
            );
            e.insert(
                "binding",
                FormSpec {
                    class: "causal:ResultBinding",
                    props: vec![
                        Prop::new("variable", "causal:ResultBinding/variable", ArgType::Str),
                        Prop::new("value", "causal:ResultBinding/value", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("rb"),
                },
            );
            e.insert(
                "intervention",
                FormSpec {
                    class: "causal:Intervention",
                    props: vec![
                        Prop::new("variable", "causal:Intervention/variable", ArgType::Str),
                        Prop::new("value", "causal:Intervention/value", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("iv"),
                },
            );
            e.insert(
                "assumption",
                FormSpec {
                    class: "causal:Assumption",
                    props: vec![
                        Prop::new("variable", "causal:Assumption/variable", ArgType::Str),
                        Prop::new("value", "causal:Assumption/value", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("asmp"),
                },
            );
            e
        },
    }
}

// ── decision / mechanism / tom / epistemic (shallow, provisional §5.16) ─────

fn decision() -> Grammar {
    let mut forms = BTreeMap::new();
    let mut risk_kinds = BTreeMap::new();
    risk_kinds.insert("risk", &["variance", "cvar", "mean"][..]);
    forms.insert(
        "outcome",
        FormSpec {
            class: "decision:Outcome",
            props: vec![
                Prop::new("name", "decision:Outcome/name", ArgType::Str),
                Prop::new("probability", "decision:Outcome/probability", ArgType::Num),
                Prop::new("utility", "decision:Outcome/utility", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("out"),
        },
    );
    forms.insert(
        "problem",
        FormSpec {
            class: "decision:DecisionProblem",
            props: vec![
                Prop::new("name", "decision:DecisionProblem/name", ArgType::Str),
                Prop::new("risk", "decision:DecisionProblem/risk", ArgType::Kind)
                    .opt()
                    .kind_object_typed("decision_data", "RiskMeasure"),
                Prop::new("actions", "decision:DecisionProblem/actions", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new(
                    "preferences",
                    "decision:DecisionProblem/preferences",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "constraints",
                    "decision:DecisionProblem/constraints",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
            ],
            kinds: risk_kinds,
            subj: SubjKind::Generated("dp"),
        },
    );
    forms.insert(
        "action",
        FormSpec {
            class: "decision:Action",
            props: vec![
                Prop::new("name", "decision:Action/name", ArgType::Str),
                Prop::new("cost", "decision:Action/cost", ArgType::Num).opt(),
                Prop::new("outcomes", "decision:Action/outcomes", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("act"),
        },
    );
    forms.insert(
        "risk_measure",
        FormSpec {
            class: "decision:RiskMeasure",
            props: vec![Prop::new("name", "decision:RiskMeasure/name", ArgType::Str).opt()],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("rm"),
        },
    );
    forms.insert(
        "preference",
        FormSpec {
            class: "decision:Preference",
            props: vec![
                Prop::new("preferred", "decision:Preference/preferred", ArgType::Str),
                Prop::new("over", "decision:Preference/over", ArgType::Str),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("prf"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "decision",
        schema_prefix: "decision",
        entry: "problem",
        root_class: "decision:DecisionProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

fn mechanism() -> Grammar {
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert(
        "asks",
        &[
            "nash",
            "nashequilibrium",
            "incentivecompatibility",
            "welfare",
            "individualrationality",
            "vcg",
        ][..],
    );
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "mechanism:MechanismProblem",
            props: vec![
                Prop::new("name", "mechanism:MechanismProblem/name", ArgType::Str),
                Prop::new(
                    "players",
                    "mechanism:MechanismProblem/players",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed(),
                Prop::new(
                    "payoffs",
                    "mechanism:MechanismProblem/payoffs",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "outside_options",
                    "mechanism:MechanismProblem/outsideOptions",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("asks", "mechanism:MechanismProblem/asks", ArgType::Kind)
                    .as_list()
                    .bracketed()
                    .opt()
                    .kind_object_typed("mechanism_data", "MechanismAsk"),
            ],
            kinds: ask_kinds,
            subj: SubjKind::Generated("mp"),
        },
    );
    forms.insert(
        "player",
        FormSpec {
            class: "mechanism:Player",
            props: vec![
                Prop::new("name", "mechanism:Player/name", ArgType::Str),
                Prop::new("strategies", "mechanism:Player/strategies", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("type", "mechanism:Player/type", ArgType::Kind)
                    .opt()
                    .kind_object_typed("mechanism_data", "PlayerType"),
                Prop::new("value", "mechanism:Player/value", ArgType::Num).opt(),
            ],
            kinds: {
                let mut m = BTreeMap::new();
                m.insert("type", &["buyer", "seller", "bidding"][..]);
                m
            },
            subj: SubjKind::Generated("pl"),
        },
    );
    forms.insert(
        "outcome",
        FormSpec {
            class: "mechanism:Outcome",
            props: vec![],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("out"),
        },
    );
    forms.insert(
        "payoff",
        FormSpec {
            class: "mechanism:Payoff",
            props: vec![
                Prop::new("player", "mechanism:Payoff/player", ArgType::Str).kwarg(),
                Prop::new("profile", "mechanism:Payoff/profile", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .kwarg(),
                Prop::new("utility", "mechanism:Payoff/utility", ArgType::Num).kwarg(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("mpy"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "mechanism",
        schema_prefix: "mechanism",
        entry: "problem",
        root_class: "mechanism:MechanismProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: BTreeMap::new(),
    }
}

fn tom() -> Grammar {
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert(
        "asks",
        &["nestedbelief", "inverseplan", "intention", "falsebelief"][..],
    );
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "tom:ToMProblem",
            props: vec![
                Prop::new("agents", "tom:ToMProblem/agents", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new("propositions", "tom:ToMProblem/propositions", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("beliefs", "tom:ToMProblem/beliefs", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("trust", "tom:ToMProblem/trust", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("observations", "tom:ToMProblem/observations", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("hypotheses", "tom:ToMProblem/hypotheses", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("asks", "tom:ToMProblem/asks", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt()
                    .kind_object_typed("tom_data", "BeliefAsk"),
            ],
            kinds: ask_kinds,
            subj: SubjKind::Generated("tp"),
        },
    );
    forms.insert(
        "agent",
        FormSpec {
            class: "tom:Agent",
            props: vec![Prop::new("name", "tom:Agent/name", ArgType::Str)],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("agt"),
        },
    );
    forms.insert(
        "ask",
        FormSpec {
            class: "tom:BeliefAsk",
            props: vec![
                Prop::new("agent", "tom:BeliefAsk/agent", ArgType::Str),
                Prop::new("proposition", "tom:BeliefAsk/proposition", ArgType::Str),
                Prop::new("depth", "tom:BeliefAsk/depth", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("ask"),
        },
    );
    forms.insert(
        "belief",
        FormSpec {
            class: "tom:Belief",
            props: vec![
                Prop::new("agent", "tom:Belief/agent", ArgType::Str),
                Prop::new("proposition", "tom:Belief/proposition", ArgType::Str),
                Prop::new("probability", "tom:Belief/probability", ArgType::Num).opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("blf"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "tom",
        schema_prefix: "tom",
        entry: "problem",
        root_class: "tom:ToMProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: {
            let mut e = BTreeMap::new();
            e.insert("outcome", generic_expr("outcome"));
            e
        },
    }
}

fn epistemic() -> Grammar {
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert("asks", &["knowledge", "belief"][..]);
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "epistemic:EpistemicProblem",
            props: vec![
                Prop::new("agents", "epistemic:EpistemicProblem/agents", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new("worlds", "epistemic:EpistemicProblem/worlds", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new(
                    "propositions",
                    "epistemic:EpistemicProblem/propositions",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed(),
                Prop::new("states", "epistemic:EpistemicProblem/states", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new(
                    "evidence",
                    "epistemic:EpistemicProblem/evidence",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "observations",
                    "epistemic:EpistemicProblem/observations",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("asks", "epistemic:EpistemicProblem/asks", ArgType::Kind)
                    .as_list()
                    .bracketed()
                    .opt()
                    .kind_object_typed("epistemic_data", "Ask"),
            ],
            kinds: ask_kinds,
            subj: SubjKind::Generated("ep"),
        },
    );
    forms.insert(
        "agent",
        FormSpec {
            class: "epistemic:Agent",
            props: vec![Prop::new("name", "epistemic:Agent/name", ArgType::Str)],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("agt"),
        },
    );
    forms.insert(
        "proposition",
        FormSpec {
            class: "epistemic:Proposition",
            props: vec![
                Prop::new("name", "epistemic:Proposition/name", ArgType::Str).kwarg(),
                Prop::new("truth", "epistemic:Proposition/truth", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("prop"),
        },
    );
    forms.insert(
        "world",
        FormSpec {
            class: "epistemic:World",
            props: vec![Prop::new("name", "epistemic:World/name", ArgType::Str).opt()],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("wld"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "epistemic",
        schema_prefix: "epistemic",
        entry: "problem",
        root_class: "epistemic:EpistemicProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: {
            let mut e = BTreeMap::new();
            e.insert(
                "proposition",
                FormSpec {
                    class: "epistemic:Proposition",
                    props: vec![
                        Prop::new("name", "epistemic:Proposition/name", ArgType::Str).kwarg(),
                        Prop::new("truth", "epistemic:Proposition/truth", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("prop"),
                },
            );
            e.insert(
                "truth_entry",
                FormSpec {
                    class: "epistemic:TruthEntry",
                    props: vec![
                        Prop::new("world", "epistemic:TruthEntry/world", ArgType::Str),
                        Prop::new("truth", "epistemic:TruthEntry/holds", ArgType::Bool),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("tr"),
                },
            );
            e.insert(
                "belief_state",
                FormSpec {
                    class: "epistemic:BeliefState",
                    props: vec![
                        Prop::new("agent", "epistemic:BeliefState/agent", ArgType::Str).kwarg(),
                        Prop::new(
                            "probability",
                            "epistemic:BeliefState/probability",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .kwarg(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("bs"),
                },
            );
            e.insert(
                "probability_entry",
                FormSpec {
                    class: "epistemic:ProbabilityEntry",
                    props: vec![
                        Prop::new("world", "epistemic:ProbabilityEntry/world", ArgType::Str),
                        Prop::new("value", "epistemic:ProbabilityEntry/value", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("pe"),
                },
            );
            e.insert(
                "evidence",
                FormSpec {
                    class: "epistemic:Evidence",
                    props: vec![
                        Prop::new("name", "epistemic:Evidence/name", ArgType::Str).kwarg(),
                        Prop::new(
                            "proposition",
                            "epistemic:Evidence/proposition",
                            ArgType::Str,
                        )
                        .kwarg(),
                        Prop::new(
                            "likelihood_if_true",
                            "epistemic:Evidence/likelihoodIfTrue",
                            ArgType::Num,
                        )
                        .kwarg(),
                        Prop::new(
                            "likelihood_if_false",
                            "epistemic:Evidence/likelihoodIfFalse",
                            ArgType::Num,
                        )
                        .kwarg(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("ev"),
                },
            );
            e.insert(
                "observation",
                FormSpec {
                    class: "epistemic:Observation",
                    props: vec![
                        Prop::new("agent", "epistemic:Observation/agent", ArgType::Str),
                        Prop::new("evidence", "epistemic:Observation/evidence", ArgType::Str),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("obs"),
                },
            );
            e.insert(
                "ask",
                FormSpec {
                    class: "epistemic:Ask",
                    props: vec![
                        Prop::new("agent", "epistemic:Ask/agent", ArgType::Str),
                        Prop::new("proposition", "epistemic:Ask/proposition", ArgType::Str),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("ask"),
                },
            );
            e.insert(
                "state",
                FormSpec {
                    class: "epistemic:EpistemicState",
                    props: vec![
                        Prop::new("agent", "epistemic:EpistemicState/agent", ArgType::Str).kwarg(),
                        Prop::new(
                            "possible_worlds",
                            "epistemic:EpistemicState/possibleWorlds",
                            ArgType::Str,
                        )
                        .as_list()
                        .bracketed()
                        .kwarg(),
                        Prop::new("belief", "epistemic:EpistemicState/belief", ArgType::Expr)
                            .kwarg(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("st"),
                },
            );
            e
        },
    }
}

fn epistemicmarket() -> Grammar {
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert(
        "asks",
        &[
            "price",
            "trade",
            "score",
            "reputation",
            "priceinformation",
            "equilibrium",
        ][..],
    );
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "epistemicmarket:Market",
            props: vec![
                Prop::new("name", "epistemicmarket:Market/name", ArgType::Str),
                Prop::new(
                    "forecasters",
                    "epistemicmarket:Market/forecasters",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "propositions",
                    "epistemicmarket:Market/propositions",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "predictions",
                    "epistemicmarket:Market/predictions",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "resolutions",
                    "epistemicmarket:Market/resolutions",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("states", "epistemicmarket:Market/states", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("orders", "epistemicmarket:Market/orders", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new(
                    "liquidity",
                    "epistemicmarket:Market/liquidity",
                    ArgType::Num,
                )
                .opt(),
                Prop::new("asks", "epistemicmarket:Market/asks", ArgType::Kind)
                    .as_list()
                    .bracketed()
                    .opt()
                    .kind_object_typed("epistemicmarket_data", "MarketAsk"),
            ],
            kinds: ask_kinds,
            subj: SubjKind::Generated("emp"),
        },
    );
    forms.insert(
        "agent",
        FormSpec {
            class: "epistemicmarket:Agent",
            props: vec![Prop::new(
                "name",
                "epistemicmarket:Agent/name",
                ArgType::Str,
            )],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("agt"),
        },
    );
    forms.insert(
        "forecaster",
        FormSpec {
            class: "epistemicmarket:Forecaster",
            props: vec![Prop::new(
                "name",
                "epistemicmarket:Forecaster/name",
                ArgType::Str,
            )],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("agt"),
        },
    );
    forms.insert(
        "security",
        FormSpec {
            class: "epistemicmarket:Security",
            props: vec![Prop::new(
                "name",
                "epistemicmarket:Security/name",
                ArgType::Str,
            )],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("sec"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "epistemicmarket",
        schema_prefix: "epistemicmarket",
        entry: "problem",
        root_class: "epistemicmarket:Market",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: {
            let mut e = BTreeMap::new();
            e.insert(
                "prediction",
                FormSpec {
                    class: "epistemicmarket:Prediction",
                    props: vec![
                        Prop::new(
                            "forecaster",
                            "epistemicmarket:Prediction/forecaster",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "proposition",
                            "epistemicmarket:Prediction/proposition",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "probability",
                            "epistemicmarket:Prediction/probability",
                            ArgType::Num,
                        ),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("pred"),
                },
            );
            e.insert(
                "share_state",
                FormSpec {
                    class: "epistemicmarket:ShareState",
                    props: vec![
                        Prop::new(
                            "proposition",
                            "epistemicmarket:ShareState/proposition",
                            ArgType::Str,
                        ),
                        Prop::new("yes", "epistemicmarket:ShareState/yes", ArgType::Num),
                        Prop::new("no", "epistemicmarket:ShareState/no", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("sh"),
                },
            );
            e.insert(
                "order",
                FormSpec {
                    class: "epistemicmarket:Order",
                    props: vec![
                        Prop::new(
                            "forecaster",
                            "epistemicmarket:Order/forecaster",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "proposition",
                            "epistemicmarket:Order/proposition",
                            ArgType::Str,
                        ),
                        Prop::new("shares", "epistemicmarket:Order/shares", ArgType::Num),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("ord"),
                },
            );
            e.insert(
                "resolution",
                FormSpec {
                    class: "epistemicmarket:Resolution",
                    props: vec![
                        Prop::new(
                            "proposition",
                            "epistemicmarket:Resolution/proposition",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "outcome",
                            "epistemicmarket:Resolution/outcome",
                            ArgType::Num,
                        ),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("res"),
                },
            );
            e.insert("ask", generic_expr("ask"));
            e
        },
    }
}

// ── abstraction (§5.17, provisional) ────────────────────────────────────────

fn abstraction() -> Grammar {
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "abstraction:AbstractionStudy",
            props: vec![
                Prop::new("name", "abstraction:AbstractionStudy/name", ArgType::Str).opt(),
                Prop::new(
                    "representations",
                    "abstraction:AbstractionStudy/representations",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("source", "abstraction:source", ArgType::Expr).opt(),
                Prop::new("target", "abstraction:target", ArgType::Expr).opt(),
                Prop::new(
                    "invariants",
                    "abstraction:AbstractionStudy/invariants",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "abstractions",
                    "abstraction:AbstractionStudy/abstractions",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "transformations",
                    "abstraction:AbstractionStudy/transformations",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "equivalence_checks",
                    "abstraction:AbstractionStudy/equivalenceChecks",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "operator_applications",
                    "abstraction:AbstractionStudy/operatorApplications",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "vocabulary",
                    "abstraction:AbstractionStudy/vocabulary",
                    ArgType::Expr,
                )
                .opt(),
                Prop::new(
                    "contracts",
                    "abstraction:AbstractionStudy/contracts",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "proof_obligations",
                    "abstraction:AbstractionStudy/proofObligations",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("abp"),
        },
    );
    forms.insert(
        "invariant",
        FormSpec {
            class: "abstraction:Invariant",
            props: vec![
                Prop::new("name", "abstraction:Invariant/name", ArgType::Str),
                Prop::new("kind", "abstraction:Invariant/kind", ArgType::Kind)
                    .opt()
                    .kind_object("abstraction_data", "InvariantKind"),
                Prop::new("symbols", "abstraction:Invariant/symbols", ArgType::Str)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: {
                let mut k = BTreeMap::new();
                k.insert(
                    "kind",
                    &[
                        "equality",
                        "conservation",
                        "positivity",
                        "custom",
                        "connectivity",
                        "normalization",
                        "feasibleregion",
                        "energyconservation",
                        "interventionsemantics",
                        "observablebehavior",
                        "stability",
                    ][..],
                );
                k
            },
            subj: SubjKind::Generated("inv"),
        },
    );
    forms.insert(
        "representation",
        FormSpec {
            class: "abstraction:Representation",
            props: vec![
                Prop::new("name", "abstraction:Representation/name", ArgType::Str),
                Prop::new("family", "abstraction:Representation/family", ArgType::Kind)
                    .kind_object("abstraction_data", "RepresentationFamily"),
                Prop::new(
                    "symbols",
                    "abstraction:Representation/symbols",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "invariants",
                    "abstraction:Representation/invariants",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "operations",
                    "abstraction:Representation/operations",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "compatible_solvers",
                    "abstraction:Representation/compatibleSolvers",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "provenance",
                    "abstraction:Representation/provenance",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
            ],
            kinds: {
                let mut m = BTreeMap::new();
                m.insert(
                    "family",
                    &[
                        "logical",
                        "graph",
                        "tensor",
                        "vector",
                        "matrix",
                        "geometric",
                        "symbolic",
                        "probabilistic",
                        "differential",
                        "statespace",
                        "programmatic",
                        "spatial",
                        "temporal",
                        "embedding",
                        "simulation",
                    ][..],
                );
                m
            },
            subj: SubjKind::Generated("rep"),
        },
    );
    forms.insert(
        "abstraction",
        FormSpec {
            class: "abstraction:Abstraction",
            props: vec![
                Prop::new("name", "abstraction:Abstraction/name", ArgType::Str),
                Prop::new("source", "abstraction:Abstraction/source", ArgType::Str),
                Prop::new("target", "abstraction:Abstraction/target", ArgType::Str),
                Prop::new(
                    "invariants",
                    "abstraction:Abstraction/invariants",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("pairs", "abstraction:Abstraction/pairs", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new("task", "abstraction:Abstraction/task", ArgType::Str).opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("abs"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "abstraction",
        schema_prefix: "abstraction",
        entry: "problem",
        root_class: "abstraction:AbstractionStudy",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms: {
            let mut forms = forms;
            if let Some(inv) = forms.get_mut("invariant") {
                inv.kinds.insert(
                    "kind",
                    &[
                        "equality",
                        "conservation",
                        "positivity",
                        "custom",
                        "connectivity",
                        "normalization",
                        "feasibleregion",
                        "energyconservation",
                        "interventionsemantics",
                        "observablebehavior",
                        "stability",
                    ][..],
                );
            }
            forms
        },
        exprs: {
            let mut e = BTreeMap::new();
            e.insert("abstraction", generic_expr("abstraction"));
            e.insert("representation", generic_expr("representation"));
            e.insert(
                "vocabulary",
                FormSpec {
                    class: "abstraction:Vocabulary",
                    props: vec![
                        Prop::new("spaces", "abstraction:Vocabulary/spaces", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                        Prop::new(
                            "semantic_spaces",
                            "abstraction:Vocabulary/semanticSpaces",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "coordinate_systems",
                            "abstraction:Vocabulary/coordinateSystems",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new("bases", "abstraction:Vocabulary/bases", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                        Prop::new(
                            "projections",
                            "abstraction:Vocabulary/projections",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "embeddings",
                            "abstraction:Vocabulary/embeddings",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "factorizations",
                            "abstraction:Vocabulary/factorizations",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "sufficient_statistics",
                            "abstraction:Vocabulary/sufficientStatistics",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new("features", "abstraction:Vocabulary/features", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                        Prop::new(
                            "state_variables",
                            "abstraction:Vocabulary/stateVariables",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "observables",
                            "abstraction:Vocabulary/observables",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "hidden_variables",
                            "abstraction:Vocabulary/hiddenVariables",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "encodings",
                            "abstraction:Vocabulary/encodings",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "decodings",
                            "abstraction:Vocabulary/decodings",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "decompositions",
                            "abstraction:Vocabulary/decompositions",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "compositions",
                            "abstraction:Vocabulary/compositions",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "approximations",
                            "abstraction:Vocabulary/approximations",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new("worlds", "abstraction:Vocabulary/worlds", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                        Prop::new("models", "abstraction:Vocabulary/models", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                        Prop::new("views", "abstraction:Vocabulary/views", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                        Prop::new(
                            "computations",
                            "abstraction:Vocabulary/computations",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "world_models",
                            "abstraction:Vocabulary/worldModels",
                            ArgType::Expr,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("voc"),
                },
            );
            e.insert(
                "symbol_pair",
                FormSpec {
                    class: "abstraction:SymbolPair",
                    props: vec![
                        Prop::new("source", "abstraction:SymbolPair/source", ArgType::Str),
                        Prop::new("target", "abstraction:SymbolPair/target", ArgType::Str),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("sp"),
                },
            );
            e.insert(
                "transformation",
                FormSpec {
                    class: "abstraction:Transformation",
                    props: vec![
                        Prop::new("name", "abstraction:Transformation/name", ArgType::Str),
                        Prop::new(
                            "input_representation",
                            "abstraction:Transformation/inputRepresentation",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "output_representation",
                            "abstraction:Transformation/outputRepresentation",
                            ArgType::Str,
                        ),
                        Prop::new("pairs", "abstraction:Transformation/pairs", ArgType::Expr)
                            .as_list()
                            .bracketed()
                            .opt(),
                        Prop::new(
                            "assumptions",
                            "abstraction:Transformation/assumptions",
                            ArgType::Str,
                        )
                        .as_list()
                        .bracketed()
                        .opt(),
                        Prop::new(
                            "computational_cost",
                            "abstraction:Transformation/computationalCost",
                            ArgType::Num,
                        )
                        .opt(),
                        Prop::new(
                            "invertible",
                            "abstraction:Transformation/invertible",
                            ArgType::Bool,
                        )
                        .opt(),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("tf"),
                },
            );
            e.insert(
                "equivalence_check",
                FormSpec {
                    class: "abstraction:EquivalenceCheck",
                    props: vec![
                        Prop::new("a", "abstraction:EquivalenceCheck/a", ArgType::Str),
                        Prop::new("b", "abstraction:EquivalenceCheck/b", ArgType::Str),
                    ],
                    kinds: BTreeMap::new(),
                    subj: SubjKind::Generated("ec"),
                },
            );
            e.insert(
                "operator_application",
                FormSpec {
                    class: "abstraction:OperatorApplication",
                    props: vec![
                        Prop::new(
                            "operator",
                            "abstraction:OperatorApplication/operator",
                            ArgType::Kind,
                        )
                        .kind_object("abstraction_data", "AbstractionOperator"),
                        Prop::new(
                            "source",
                            "abstraction:OperatorApplication/source",
                            ArgType::Str,
                        ),
                        Prop::new(
                            "target",
                            "abstraction:OperatorApplication/target",
                            ArgType::Str,
                        ),
                    ],
                    kinds: {
                        let mut m = BTreeMap::new();
                        m.insert(
                            "operator",
                            &[
                                "abstract",
                                "generalize",
                                "specialize",
                                "aggregate",
                                "coarsegrain",
                                "factorize",
                                "project",
                                "embed",
                                "compress",
                                "discretize",
                                "continuize",
                                "marginalize",
                                "eliminate",
                                "summarize",
                            ][..],
                        );
                        m
                    },
                    subj: SubjKind::Generated("oa"),
                },
            );
            for name in [
                "invariant",
                "claim",
                "truth_entry",
                "proposition",
                "forecaster",
                "prediction",
                "resolution",
                "share_state",
                "order",
                "outcome",
                "ask",
            ] {
                e.insert(name, generic_expr(name));
            }
            e
        },
    }
}

// ── ensemble (§5.18) — multi-model ensemble reasoning ───────────────────────

fn ensemble() -> Grammar {
    let mut ask_kinds = BTreeMap::new();
    ask_kinds.insert(
        "asks",
        &[
            "prediction",
            "predictive-distribution",
            "disagreement",
            "consensus",
            "model-risk",
            "leave-one-out",
            "robust",
            "experiment",
        ][..],
    );
    let mut combo_kinds = BTreeMap::new();
    combo_kinds.insert(
        "combination",
        &[
            "bayesian-model-averaging",
            "predictive-averaging",
            "stacking",
            "voting",
            "mixture-of-experts",
            "robust",
        ][..],
    );
    let mut family_kinds = BTreeMap::new();
    family_kinds.insert(
        "family",
        &[
            "structural",
            "empirical",
            "statistical",
            "machine-learning",
            "causal",
            "agent-based",
        ][..],
    );
    let mut lifecycle_kinds = BTreeMap::new();
    lifecycle_kinds.insert(
        "lifecycle",
        &[
            "proposed",
            "formalized",
            "validated",
            "candidate",
            "active",
            "degraded",
            "retired",
        ][..],
    );
    let mut forms = BTreeMap::new();
    forms.insert(
        "problem",
        FormSpec {
            class: "ensemble:EnsembleProblem",
            props: vec![
                Prop::new("name", "ensemble:EnsembleProblem/name", ArgType::Str),
                Prop::new("target", "ensemble:EnsembleProblem/target", ArgType::Str),
                Prop::new("members", "ensemble:EnsembleProblem/members", ArgType::Expr)
                    .as_list()
                    .bracketed(),
                Prop::new(
                    "observations",
                    "ensemble:EnsembleProblem/observations",
                    ArgType::Expr,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new(
                    "evaluation",
                    "ensemble:EnsembleProblem/evaluation",
                    ArgType::Expr,
                )
                .opt(),
                Prop::new(
                    "combination",
                    "ensemble:EnsembleProblem/combination",
                    ArgType::Kind,
                )
                .kind_object_typed("ensemble_data", "CombinationStrategy"),
                Prop::new("asks", "ensemble:EnsembleProblem/asks", ArgType::Kind)
                    .as_list()
                    .bracketed()
                    .opt()
                    .kind_object_typed("ensemble_data", "EnsembleAsk"),
            ],
            kinds: {
                let mut m = BTreeMap::new();
                m.extend(ask_kinds.clone());
                m.extend(combo_kinds.clone());
                m
            },
            subj: SubjKind::Generated("ens"),
        },
    );
    forms.insert(
        "model",
        FormSpec {
            class: "ensemble:EnsembleMember",
            props: vec![
                Prop::new("name", "ensemble:EnsembleMember/name", ArgType::Str),
                Prop::new("family", "ensemble:EnsembleMember/family", ArgType::Kind)
                    .kind_object_typed("ensemble_data", "ModelFamily"),
                Prop::new("weights", "ensemble:EnsembleMember/weights", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
                Prop::new(
                    "assumptions",
                    "ensemble:EnsembleMember/assumptions",
                    ArgType::Str,
                )
                .as_list()
                .bracketed()
                .opt(),
                Prop::new("document", "ensemble:EnsembleMember/document", ArgType::Str),
                Prop::new(
                    "lifecycle",
                    "ensemble:EnsembleMember/lifecycle",
                    ArgType::Kind,
                )
                .opt()
                .kind_object_typed("ensemble_data", "ModelLifecycle"),
            ],
            kinds: {
                let mut m = BTreeMap::new();
                m.extend(family_kinds.clone());
                m.extend(lifecycle_kinds.clone());
                m
            },
            subj: SubjKind::Generated("mem"),
        },
    );
    forms.insert(
        "weight",
        FormSpec {
            class: "ensemble:ModelWeight",
            props: vec![
                Prop::new("condition", "ensemble:ModelWeight/condition", ArgType::Str),
                Prop::new("value", "ensemble:ModelWeight/value", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("wt"),
        },
    );
    forms.insert(
        "observation",
        FormSpec {
            class: "ensemble:Observation",
            props: vec![
                Prop::new("time", "ensemble:Observation/time", ArgType::Str),
                Prop::new("bindings", "ensemble:Observation/bindings", ArgType::Expr)
                    .as_list()
                    .bracketed()
                    .opt(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("obs"),
        },
    );
    forms.insert(
        "binding",
        FormSpec {
            class: "ensemble:Binding",
            props: vec![
                Prop::new("name", "ensemble:Binding/name", ArgType::Str),
                Prop::new("value", "ensemble:Binding/value", ArgType::Num),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("bd"),
        },
    );
    forms.insert(
        "protocol",
        FormSpec {
            class: "ensemble:EvaluationProtocol",
            props: vec![
                Prop::new("train", "ensemble:EvaluationProtocol/train", ArgType::Num).kwarg(),
                Prop::new(
                    "validation",
                    "ensemble:EvaluationProtocol/validation",
                    ArgType::Num,
                )
                .kwarg(),
                Prop::new("test", "ensemble:EvaluationProtocol/test", ArgType::Num).kwarg(),
            ],
            kinds: BTreeMap::new(),
            subj: SubjKind::Generated("prt"),
        },
    );
    Grammar {
        annotated_forms: Vec::new(),
        name: "ensemble",
        schema_prefix: "ensemble",
        entry: "problem",
        root_class: "ensemble:EnsembleProblem",
        root_links: vec![],
        kinds: BTreeMap::new(),
        dynamic_predicates: false,
        entry_props: Vec::new(),
        forms,
        exprs: {
            let mut e = BTreeMap::new();
            for name in ["model", "weight", "observation", "binding", "protocol"] {
                e.insert(name, generic_expr(name));
            }
            e
        },
    }
}

// ── grammar card (§6.4) ─────────────────────────────────────────────────────

/// Grammar card renderer: the compact Markdown documentation element served
/// by `subsystem_semantics`. Deterministic, budget-checked, golden-able.
/// Engine-surface status of a module (Phase 4 §9.5): modules with committed
/// golden `.sem` fixtures are `confirmed`; the thin/aspirational surfaces
/// without engine goldens carry an explicit pending badge on their card.
pub fn module_status(_module: &str) -> &'static str {
    // Every seeded module now has committed golden `.sem` fixtures under
    // `examples/<module>`, so none carry the pending badge.
    "confirmed"
}

pub fn render_grammar_card(grammar: &Grammar) -> String {
    let mut out = String::new();
    out.push_str(&format!("# module {}\n\n", grammar.name));
    if module_status(grammar.name) != "confirmed" {
        out.push_str(
            "> ⚠️ **pending engine golden** — this table follows the engine guide but \
has no goldened fixture yet; freeze it when `examples/<sub>` goldens exist.\n\n",
        );
    }
    out.push_str("## How to write\n");
    out.push_str(&format!(
        "Top-level payload is exactly one call: `{}.{}` or a root-linked form like `{}.equation(...)`. Every nested call is unqualified.\n\n",
        grammar.name, grammar.entry, grammar.name
    ));

    out.push_str("## Entry\n");
    if grammar.entry_props.is_empty() {
        out.push_str(&format!(
            "| form | args | → class |\n|---|---|---|\n| `{}` | content forms | `{}` |\n",
            grammar.entry, grammar.root_class
        ));
        out.push_str("Auto-rooting: the entry form's args are content forms; each root-linked form is wired via its root property.\n\n");
    } else {
        out.push_str(&format!(
            "| form | args | → class |\n|---|---|---|\n| `{}` | {} | `{}` |\n",
            grammar.entry,
            entry_args_line(grammar),
            grammar.root_class
        ));
        out.push('\n');
    }

    let mut content: Vec<(&str, &FormSpec)> = grammar
        .forms
        .iter()
        .filter(|(k, _)| **k != grammar.entry)
        .map(|(k, v)| (*k, v))
        .collect();
    content.sort_by_key(|(k, _)| *k);
    if !content.is_empty() {
        out.push_str("## Forms\n| form | args | → class |\n|---|---|---|\n");
        for (name, spec) in content {
            out.push_str(&format!(
                "| `{name}` | {} | `{}` |\n",
                args_line(spec),
                spec.class
            ));
        }
        out.push('\n');
    }

    if !grammar.exprs.is_empty() {
        out.push_str("## Expressions\n");
        let mut names: Vec<&str> = grammar.exprs.keys().copied().collect();
        names.sort_unstable();
        out.push_str(&format!(
            "`{}` — bare ident = variable/param, bare number = constant.\n\n",
            names.join("`, `")
        ));
    }

    out.push_str("## Example\n```\n");
    out.push_str(grammar_example(grammar.name));
    out.push_str("\n```\n");
    out
}

fn args_line(spec: &FormSpec) -> String {
    spec.props
        .iter()
        .map(|p| {
            let t = match p.ty {
                ArgType::Num => "num",
                ArgType::Str => "str",
                ArgType::Bool => "bool",
                ArgType::Sym => "sym",
                ArgType::Expr => "expr",
                ArgType::Kind => "kind",
            };
            let mut s = format!("{}: {t}", p.name);
            if p.opt {
                s.push('?');
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn entry_args_line(grammar: &Grammar) -> String {
    grammar
        .entry_props
        .iter()
        .map(|p| {
            format!(
                "{}: [{}]",
                p.name,
                if p.ty == ArgType::Str { "str" } else { "sym" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Canonical example per seeded module: a complete worked SEM model that
/// exercises the module's key forms. Logic and probabilistic examples use
/// structured forms; legacy string clauses are compatibility-only. Used by
/// the grammar card, subsystem_semantics output, and LLM prompt examples.
pub fn grammar_example(module: &str) -> &'static str {
    match module {
        // ── numerical: two-equation system (IS-LM) with nested expressions ──
        "numerical" => "numerical.model(equation(y, sum(c, i, g)), \
            equation(c, mul(t, y)), var(y), var(c), param(i, 10.0), param(g, 20.0), param(t, 0.8))",
        // ── solver: LP with two variables, objective, and constraints ──────
        "solver" => "solver.model(var(x), var(y), \
            objective(maximize, sum(x, y)), \
            constraint(le, x, 2.0), constraint(le, y, 3.0))",
        // ── logic: structured SEM ancestry chain ─────────────────────────
        "logic" => "logic.model(fact(parent(alice, bob)), fact(parent(bob, carol)), \
            rule(head=ancestor(?x, ?y), body=[parent(?x, ?y)]), \
            rule(head=ancestor(?x, ?z), body=[parent(?x, ?y), ancestor(?y, ?z)]))" ,
        // ── probabilistic: structured weighted statements ────────────────
        "probabilistic" => "probabilistic.model(probability(0.3, rain), \
            probability(0.8, implies(rain, wet)), rule(head=mud, body=[wet]), query(wet))" ,
        // ── event: fluents, initiates/terminates, numeric fluent + update ─
        "event" => "event.problem(event_model(timeline=10, \
            fluents=[fluent(\"raining\"), numeric_fluent(\"temp\", 20.0)], \
            events=[event(name=\"rain_start\", time=1, kind=instant), \
                    event(name=\"heating\", time=3, kind=instant)], \
            initiates=[initiates(\"rain_start\", \"raining\")], \
            terminates=[terminates(\"rain_stop\", \"raining\")], \
            numeric_updates=[numeric_update(\"heating\", \"temp\", set, 30.0)], \
            initial=[\"raining\"]))" ,
        // ── temporal: two intervals, a constraint between them, ask ───────
        "temporal" => "temporal.problem(intervals=[interval(label=\"load\", \
            start=point(\"load-start\", 0), end=point(\"load-end\", 2)), \
            interval(label=\"ship\", \
            start=point(\"ship-start\", 3), end=point(\"ship-end\", 5))], \
            constraints=[constraint(a=\"load\", b=\"ship\", relation=before, hard=true)], \
            ask=consistency)" ,
        // ── tensor: reuses algebra (two-equation system) ─────────────────
        "tensor" => "tensor.model(var(x), var(y), param(a, 2.0), param(b, 3.0), \
            equation(x, sum(mul(a, y), b)), equation(y, mul(a, x)))" ,
        // ── information: distribution + channel + mutual information ──────
        "information" => "information.query(distributions=[distribution(\"rain\", \
            [\"yes\", \"no\"], [0.3, 0.7])], \
            channels=[channel(\"bsc\", [\"0\", \"1\"], [\"0\", \"1\"], \
            [[0.9, 0.1], [0.1, 0.9]])], \
            measures=[measure(kind=mutualinformation, first=\"rain\", second=\"signal\")])" ,
        // ── learning: MDP with two states and experiences ─────────────────
        "learning" => "learning.problem(name=\"grid-maze\", states=[\"a\", \"b\", \"goal\"], \
            actions=[\"north\", \"east\"], algorithm=valueiteration, \
            learning_rate=0.9, discount=0.95, experiences=[experience(state=\"a\", action=\"north\", \
            next_state=\"b\", reward=0.0, terminal=false), \
            experience(state=\"b\", action=\"east\", \
            next_state=\"goal\", reward=1.0, terminal=true)])" ,
        // ── memory: two memories, trajectory, and retrieval ───────────────
        "memory" => "memory.problem(name=\"market\", query=\"price\", \
            memories=[memory(id=\"e1\", content=\"price elasticity is negative\", \
            strength=0.9, confidence=0.95, kind=semantic, evidence=[\"episode_1\"]), \
            memory(id=\"e2\", content=\"demand increased after price drop\", \
            strength=0.7, confidence=0.8, kind=episodic)], \
            trajectories=[trajectory(\"rt1\", steps=[step(\"s1\", \"observe\", outcome=success)])], \
            asks=[retrieve])" ,
        // ── complexity: two algorithms with comparison ─────────────────────
        "complexity" => "complexity.query(size=1024, \
            algorithms=[algorithm(\"quick-sort\", time=linearithmic(), space=linear()), \
                        algorithm(\"merge-sort\", time=linearithmic(), space=linearithmic())], \
            budget=budget(time_seconds=0.5, unit_system=\"seconds\"), \
            classify=[\"quick-sort\"], assess=[\"merge-sort\"])" ,
        // ── argumentation: claims, arguments, attacks, semantics ──────────
        "argumentation" => "argumentation.problem(name=\"rain-debate\", \
            claims=[claim(\"rain\", \"it will rain\"), claim(\"norain\", \"it will not rain\")], \
            arguments=[argument(\"barometer\", \"rain\", premises=[], scheme=empirical), \
                       argument(\"clear-sky\", \"norain\", premises=[], scheme=authority)], \
            attacks=[attack(\"clear-sky\", \"barometer\", kind=rebuttal)], \
            semantics=grounded, asks=[accepted, evaluation])" ,
        // ── provenance: sources, claims, observations, asks ───────────────
        "provenance" => "provenance.problem(name=\"weather-lineage\", \
            sources=[source(\"barometer\", 0.9, kind=sensor)], \
            claims=[claim(\"rain\", \"it will rain\", reported_by=[\"barometer\"])], \
            observations=[observation(\"barometer\", \"rain\", asserts=true)], \
            asks=[trustscores])" ,
        // ── evolution: three traits, full payoff matrix, fixed points ─────
        "evolution" => "evolution.problem(name=\"rock-paper-scissors\", steps=200, \
            traits=[trait(\"rock\", 0.33), trait(\"paper\", 0.33), trait(\"scissors\", 0.34)], \
            payoffs=[payoff(\"rock\", \"rock\", 0.0), payoff(\"rock\", \"paper\", -1.0), \
                     payoff(\"rock\", \"scissors\", 1.0)], asks=[fixedpoints])" ,
        // ── analogy: source/target cases with elements ────────────────────
        "analogy" => "analogy.problem(name=\"flow\", \
            source=case(\"circuit\", [element(\"v\", kind=variable, arity=1), \
                element(\"i\", kind=variable, arity=1), element(\"ohm\", kind=equation)]), \
            target=case(\"hydraulics\", [element(\"p\", kind=variable, arity=1), \
                element(\"q\", kind=variable, arity=1), element(\"hagen\", kind=equation)]), \
            asks=[correspondences])" ,
        // ── symreg: data points + grammar with operators ──────────────────
        "symreg" => "symreg.problem(name=\"line\", variables=[\"x\"], \
            data=[datapoint([binding(\"x\", 0.0)], 1.0), \
                  datapoint([binding(\"x\", 1.0)], 2.0)], \
            grammar=grammar(constants=[1.0], operators=[add, mul]), \
            objectives=[accuracy], max_depth=3)" ,
        // ── synthesis: I/O examples + grammar + spec ──────────────────────
        "synthesis" => "synthesis.problem(name=\"plus-one\", variables=[\"x\"], \
            examples=[example([binding(\"x\", 0.0)], 1.0), \
                      example([binding(\"x\", 1.0)], 2.0)], \
            grammar=grammar(constants=[1.0], operators=[add]), \
            specification=specification(preconditions=[\"x >= 0\"]), \
            max_depth=4, max_programs=1000)" ,
        // ── causal: SCM with variables, graph edges, and ask ──────────────
        "causal" => "causal.problem(name=\"smoking-study\", scm=scm([\"smoking\", \"cancer\"], \
            graph=graph(edges=[edge(\"smoking\", \"cancer\")])), \
            ask=estimateeffect)" ,
        // ── decision: two actions + risk + preferences ────────────────────
        "decision" => "decision.problem(name=\"invest\", \
            risk=cvar, actions=[action(\"buy\", 5.0), action(\"hold\", 1.0)], \
            preferences=[preference(preferred=\"buy\", over=\"hold\")])" ,
        // ── mechanism: players + payoffs + Nash equilibrium ───────────────
        "mechanism" => "mechanism.problem(name=\"auction\", \
            players=[player(name=\"buyer1\", type=buyer, value=100.0), \
                     player(name=\"buyer2\", type=buyer, value=80.0)], \
            payoffs=[payoff(player=\"buyer1\", profile=[\"object\"], utility=50.0)], \
            asks=[nashequilibrium])" ,
        // ── tom: agents, beliefs, false-belief ask ────────────────────────
        "tom" => "tom.problem(agents=[agent(\"Alice\"), agent(\"Bob\")], \
            beliefs=[belief(\"Alice\", \"it is raining\"), \
                     belief(\"Bob\", \"the box is empty\")], \
            asks=[falsebelief])" ,
        // ── epistemic: two agents, two worlds, propositions, knowledge ask ─
        "epistemic" => "epistemic.problem(agents=[agent(\"Alice\"), agent(\"Bob\")], \
            worlds=[world(\"w1\"), world(\"w2\")], \
            propositions=[proposition(name=\"p\"), proposition(name=\"q\")], asks=[knowledge])" ,
        // ── epistemicmarket: forecasters + propositions + price/score asks ─
        "epistemicmarket" => "epistemicmarket.problem(name=\"rain-market\", \
            forecasters=[forecaster(\"a1\"), forecaster(\"a2\")], \
            propositions=[\"rain\", \"snow\"], \
            predictions=[prediction(\"a1\", \"rain\", 0.7), prediction(\"a2\", \"rain\", 0.3)], \
            resolutions=[resolution(\"rain\", 1.0)], \
            states=[share_state(\"rain\", 0.65, 0.35)], \
            orders=[order(\"a1\", \"rain\", 10.0)], liquidity=100.0, asks=[price, score])" ,
        // ── abstraction: source → target representations ─────────────────
        "abstraction" => "abstraction.problem(\
            representations=[representation(name=\"boolean-circuit\", family=logical), \
                             representation(name=\"logic-program\", family=programmatic)])" ,
        // ── ensemble: members, weights, observations, combination, asks ───
        "ensemble" => "ensemble.problem(name=\"gdp-forecast\", target=\"gdp-growth\", \
            members=[model(\"keynesian\", family=structural, weights=[weight(\"prior\", 0.4)], \
                assumptions=[\"rational-expectations\"], document=\"gdp_model1.sem\"), \
                model(\"ml-forecast\", family=machine-learning, weights=[weight(\"prior\", 0.6)], \
                document=\"gdp_model3.sem\")], \
            observations=[observation(\"2024Q1\", bindings=[binding(\"gdp\", 2.1)])], \
            evaluation=protocol(train=0.7, validation=0.15, test=0.15), \
            combination=bayesian-model-averaging, asks=[prediction, consensus, disagreement])" ,
        _ => "",
    }
}

// ── generator scaffold (§4.3): class_form table + loud-missing guard ────────

/// The single human-reviewed constant of §4.3: ontology class local name →
/// SEM form name. `None` marks classes that are not (yet) LLM-facing forms.
/// Phase 0 seeds only the model-layer classes the shared algebra uses plus
/// the classes of the four seeded modules; anything else fails the coverage
/// guard loudly instead of silently compiling.
pub fn class_form(class_local: &str) -> Option<&'static str> {
    let form = match class_local {
        "Model" => "model",
        "Variable" => "var",
        "Parameter" => "param",
        "Constant" => "const",
        "Equation" => "equation",
        "Addition" => "sum",
        "Multiplication" => "mul",
        "Subtraction" => "sub",
        "Division" => "div",
        "Power" => "pow",
        "Objective" => "objective",
        "Inequality" => "constraint",
        // Calculus / time-series expression forms.
        "Differential" => "differential",
        "Negation" => "neg",
        "Sin" => "sin",
        "Cos" => "cos",
        "Exp" => "exp",
        "Log" => "log",
        "Lag" => "lag",
        "Lead" => "lead",
        _ => return None,
    };
    Some(form)
}

/// The classes deliberately *not* mapped to SEM forms (abstract, engine-side,
/// or structural). Entries here are a reviewed decision, not a gap.
pub fn non_form_classes() -> &'static [&'static str] {
    &[
        "Expression",
        "Identity",
        "Definition",
        "BehavioralEquation",
        "AccountingIdentity",
        "Constraint",
        "Index",
        "IndexDimension",
        "Observation",
        "Dataset",
        "TabularBinding",
        "VariableBinding",
        "IndexBinding",
        "DataSource",
        "DataSourceKind",
        "SourceColumn",
        "SourceCell",
        "SourceLoad",
        "ParameterBinding",
        "ParameterAggregate",
    ]
}

/// The §4.3 CI-diff guard: every class shipped in the model-layer ontology
/// (`math_core::vocabulary::MODEL_CLASSES` — the authoritative class list of
/// the model layer) must be either mapped to a SEM form via [`class_form`] or
/// deliberately listed in [`non_form_classes`]. A new class in the ontology
/// that has neither mapping fails the build loudly — the "loud-missing-class"
/// rule — instead of silently compiling.
pub fn model_coverage_gap() -> Vec<String> {
    let mut missing = Vec::new();
    for iri in math_core::vocabulary::MODEL_CLASSES {
        let stem = iri.rsplit(['#', '/']).next().unwrap_or(iri).to_string();
        if class_form(&stem).is_none() && !non_form_classes().contains(&stem.as_str()) {
            missing.push(stem);
        }
    }
    missing
}

/// The grammar card budget (§6.4): hard caps per module family. The card
/// renderer asserts it stays within budget so the LLM-facing doc never
/// balloons.
pub fn card_budget(module: &str) -> usize {
    match module {
        "numerical" | "solver" => 2_000,
        // logic/probabilistic now carry the full structured statement surface
        // (statement forms + formula forms + arithmetic + the probabilistic
        // categorical/Bernoulli/trial forms) and the generic outcome vocabulary
        // (`outcome_*`, plan §2/§7.1).
        "logic" | "probabilistic" => 3_800,
        _ => 2_000,
    }
}
