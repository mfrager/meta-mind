//! SEM renderer (§4.2 of `planning/s_expr/s_expr_design2.md`): RDF `Graph` →
//! canonical SEM text, walking the stored triples directly — no cRDF
//! intermediate, no JSON round-trip.
//!
//! Walk order (deterministic, never hash-map iteration): root-linked subjects
//! first (root props in the module's canonical root-property order, each list
//! in its own order), then every other referenced subject in first-reference
//! order depth-first. Expression nodes and synthesized constants are inlined
//! (R8/R5 inverse); named subjects keep their name as the form's first arg.

use math_core::oxigraph::model::vocab::{rdf, xsd};
use math_core::oxigraph::model::{
    BlankNode, Graph, Literal, NamedNode, NamedOrBlankNode, NamedOrBlankNodeRef, Term, TermRef,
};

use crate::sem::errors::SemError;
use crate::sem::grammar::{
    grammar, ArgType, FormSpec, Grammar, KindMode, Prop, SemPrefixes, SubjKind,
};

/// URI-annotation mode for SEM output (`planning/updates/sem_uri_annotation_design.md`
/// §6). Annotations are the `[ex:…]` fragments after forms and node-valued
/// atoms — output-only decoration the parser accepts and ignores (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnnotationMode {
    /// Bare canonical SEM (existing behavior; tests, hash comparisons).
    #[default]
    None,
    /// Annotate every named-node element (debug, golden tests, frontend
    /// "full" view). Kind variants and plain literals stay bare.
    Full,
    /// Annotate only addressable nodes — the LLM-facing default:
    /// **Tier 1** root-linked slots (the `model_edit` targets) plus **Tier 2**
    /// declaration forms listed in the grammar's `annotated_forms`.
    Selective,
}

/// Render a Graph rooted at `root` into canonical SEM text for `module`.
/// The module must have a seeded grammar (Phase 0: numerical, solver, logic,
/// probabilistic); anything else is a loud `sem:e1`.
pub fn render(graph: &Graph, root: &NamedNode, module: &str) -> Result<String, SemError> {
    render_with_mode(graph, root, module, AnnotationMode::None)
}

/// [`render`] with URI annotations per `mode`.
pub fn render_with_mode(
    graph: &Graph,
    root: &NamedNode,
    module: &str,
    mode: AnnotationMode,
) -> Result<String, SemError> {
    let g = grammar(module).ok_or_else(|| SemError::UnknownModule(module.to_string()))?;
    let prefixes = SemPrefixes::default();
    Renderer::new(graph, &prefixes, &g)
        .with_mode(mode)
        .render_document(root)
}

/// Render an output-only logic execution report as SEM.
///
/// `logic.report(...)` intentionally is not registered in the compile grammar:
/// reports are results, not model inputs. This renderer therefore reads the
/// report vocabulary directly and preserves repeated-property order. Literal
/// entries are quoted; named-node entries receive Full-mode URI annotations
/// when requested. The returned text can be displayed or copied, but feeding
/// it to `sem::compile` is rejected as an unknown/output-only form.
pub fn render_logic_report(
    graph: &Graph,
    root: &NamedNode,
    mode: AnnotationMode,
) -> Result<String, SemError> {
    let prefixes = SemPrefixes::default();
    let logic_ns = prefixes
        .resolve("logic:LogicReport")
        .ok_or_else(|| SemError::UnknownFunction("logic:LogicReport".to_string()))?
        .trim_end_matches("LogicReport")
        .to_string();
    let sections = [
        ("derived", "derived"),
        ("probability", "probability"),
        ("error", "error"),
        ("warning", "warning"),
    ];
    let mut args = Vec::new();
    for (form, local) in sections {
        let predicate = format!("{logic_ns}{local}");
        let mut values: Vec<Term> = graph
            .iter()
            .filter(|triple| {
                triple.subject == NamedOrBlankNodeRef::NamedNode(root.as_ref())
                    && triple.predicate.as_str() == predicate
            })
            .map(|triple| triple.object.into_owned())
            .collect();
        // Repeated report properties are sets in the RDF report vocabulary;
        // canonicalize literal entries so output does not depend on oxigraph
        // graph iteration order. Named entries retain their stored order when
        // available (their URI is the addressable identity).
        values.sort_by(|left, right| match (left, right) {
            (Term::Literal(a), Term::Literal(b)) => a.value().cmp(b.value()),
            (Term::Literal(_), Term::NamedNode(_)) => std::cmp::Ordering::Less,
            (Term::NamedNode(_), Term::Literal(_)) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        });
        if values.is_empty() {
            continue;
        }
        let rendered = values
            .iter()
            .map(|value| render_report_value(value, &prefixes, mode))
            .collect::<Result<Vec<_>, _>>()?;
        args.push(format!("{form}([{}])", rendered.join(", ")));
    }
    Ok(format!("logic.report({})", args.join(", ")))
}

fn render_report_value(
    value: &Term,
    prefixes: &SemPrefixes,
    mode: AnnotationMode,
) -> Result<String, SemError> {
    match value {
        Term::Literal(literal) => Ok(format!("\"{}\"", escape_sem_string(literal.value()))),
        Term::NamedNode(node) => {
            let compact = prefixes
                .compact(node.as_str())
                .unwrap_or_else(|| format!("<{}>", node.as_str()));
            let annotation = if mode == AnnotationMode::None {
                String::new()
            } else {
                format!("[{}]", compact)
            };
            Ok(format!("{compact}{annotation}"))
        }
        Term::BlankNode(node) => Ok(format!("_:{}", node.as_str())),
    }
}

fn escape_sem_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// ── output-only subsystem reports ──────────────────────────────────────────

/// Subsystem module → report root class (prefixed). The registry drives
/// [`find_report_root`]: an engine-run graph whose root is typed with one of
/// these classes is an output-only SEM report rendered by [`render_report`].
/// `logic` keeps its bespoke renderer ([`render_logic_report`]); every other
/// registered module uses the vocabulary-agnostic walker below.
pub const REPORT_ROOTS: &[(&str, &str)] = &[
    ("logic", "logic:LogicReport"),
    ("temporal", "temporal:TemporalResult"),
    ("event", "event:EventAnalysis"),
    ("causal", "causal:CausalReport"),
    ("information", "info:InformationReport"),
    ("learning", "learning:LearningReport"),
    ("evolution", "evolution:EvolutionReport"),
    ("analogy", "analogy:AnalogyReport"),
    ("symreg", "symreg:SymbolicRegressionReport"),
    ("synthesis", "synthesis:SynthesisReport"),
    ("memory", "memory:MemoryReport"),
    ("complexity", "complexity:ComplexityReport"),
    ("argumentation", "argumentation:ArgumentationReport"),
    ("provenance", "provenance:ProvenanceReport"),
    ("decision", "decision:DecisionReport"),
    ("mechanism", "mechanism:MechanismReport"),
    ("tom", "tom:ToMReport"),
    ("epistemic", "epistemic:EpistemicReport"),
    ("epistemicmarket", "epistemicmarket:MarketReport"),
    ("abstraction", "abstraction:AbstractionReport"),
];

/// Find the report root of a graph: the single subject typed with a
/// registered report class. Returns the subsystem module and the root node,
/// or `None` when no registered report class is present.
pub fn find_report_root(graph: &Graph) -> Option<(&'static str, NamedNode)> {
    for (module, class) in REPORT_ROOTS {
        if let Some(root) = find_root(graph, class) {
            return Some((*module, root));
        }
    }
    None
}

/// Render an output-only subsystem report graph as SEM: `<module>.report(...)`.
///
/// Reports are results, not model inputs — the report vocabulary is never
/// registered in the compile grammar (same rule as `logic.report`), so
/// feeding the rendered text back to `sem::compile` fails loudly. The
/// walker is vocabulary-agnostic: each root property becomes a section named
/// after its field, values render in deterministic order (literals sorted,
/// typed objects render their own properties recursively as
/// `class_local(...)` forms), and repeated/list-valued sections bracket their
/// items `[ ... ]`.
pub fn render_report(
    graph: &Graph,
    root: &NamedNode,
    module: &str,
    mode: AnnotationMode,
) -> Result<String, SemError> {
    let prefixes = SemPrefixes::default();
    let sections = report_sections(
        graph,
        &NamedOrBlankNode::NamedNode(root.clone()),
        &prefixes,
        mode,
    )?;
    Ok(format!("{module}.report({})", sections.join(", ")))
}

/// The `field(value…)` sections of a report subject: one per distinct
/// predicate, in canonical IRI order, values in deterministic order.
fn report_sections(
    graph: &Graph,
    subject: &NamedOrBlankNode,
    prefixes: &SemPrefixes,
    mode: AnnotationMode,
) -> Result<Vec<String>, SemError> {
    let type_pred = rdf::TYPE.into_owned();
    let mut by_pred: std::collections::BTreeMap<String, Vec<Term>> =
        std::collections::BTreeMap::new();
    for t in graph.iter() {
        if t.subject != subject.as_ref() || t.predicate == type_pred.as_ref() {
            continue;
        }
        by_pred
            .entry(t.predicate.as_str().to_string())
            .or_default()
            .push(t.object.into_owned());
    }
    let mut sections = Vec::new();
    for (pred, values) in by_pred {
        // Expand rdf:List heads into their elements.
        let mut items: Vec<Term> = Vec::new();
        let mut is_list = false;
        for v in &values {
            if let Some(chain) = report_list_chain(graph, v, prefixes) {
                is_list = true;
                items.extend(chain);
            } else {
                items.push(v.clone());
            }
        }
        if items.is_empty() {
            continue;
        }
        // Deterministic order regardless of graph iteration order.
        items.sort_by_key(report_term_key);
        let rendered = items
            .iter()
            .map(|t| report_item(graph, t, prefixes, mode))
            .collect::<Result<Vec<_>, _>>()?;
        let body = if rendered.len() > 1 || is_list {
            format!("[{}]", rendered.join(", "))
        } else {
            rendered[0].clone()
        };
        sections.push(format!("{}({body})", report_field_name(&pred)));
    }
    Ok(sections)
}

/// Render one report value: literals stay bare (numbers/bools unquoted,
/// strings quoted); typed nodes render recursively as `class_local(...)`;
/// untyped nodes render as their compact reference.
fn report_item(
    graph: &Graph,
    term: &Term,
    prefixes: &SemPrefixes,
    mode: AnnotationMode,
) -> Result<String, SemError> {
    match term {
        Term::Literal(l) => Ok(literal_to_value(l)),
        Term::NamedNode(n) => report_node(
            graph,
            &NamedOrBlankNode::NamedNode(n.clone()),
            prefixes,
            mode,
        ),
        Term::BlankNode(b) => report_node(
            graph,
            &NamedOrBlankNode::BlankNode(b.clone()),
            prefixes,
            mode,
        ),
    }
}

/// Render a report node: `class_local(field(value)…)` for typed objects,
/// compact `ex:…`/`<iri>`/`_:…` otherwise.
fn report_node(
    graph: &Graph,
    subject: &NamedOrBlankNode,
    prefixes: &SemPrefixes,
    mode: AnnotationMode,
) -> Result<String, SemError> {
    let type_pred = rdf::TYPE.into_owned();
    let mut class_local: Option<String> = None;
    for t in graph.iter() {
        if t.subject == subject.as_ref() && t.predicate == type_pred.as_ref() {
            if let TermRef::NamedNode(o) = &t.object {
                class_local = o.as_str().rsplit(['/', '#']).next().map(String::from);
            }
        }
    }
    if let Some(class_local) = class_local {
        let form = snake_case(&class_local);
        let inner = report_sections(graph, subject, prefixes, mode)?;
        let ann = if mode == AnnotationMode::None {
            String::new()
        } else {
            match subject {
                NamedOrBlankNode::NamedNode(n) => match prefixes.compact(n.as_str()) {
                    Some(c) => format!("[{c}]"),
                    None => format!("[<{}>]", n.as_str()),
                },
                NamedOrBlankNode::BlankNode(_) => String::new(),
            }
        };
        return Ok(format!("{form}{ann}({})", inner.join(", ")));
    }
    Ok(match subject {
        NamedOrBlankNode::NamedNode(n) => prefixes
            .compact(n.as_str())
            .unwrap_or_else(|| format!("<{}>", n.as_str())),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    })
}

/// The field name of a report predicate: the last IRI segment, snake_cased
/// (`info:InformationReport/queryAnswers` → `query_answers`).
fn report_field_name(pred: &str) -> String {
    let local = pred.rsplit(['/', '#']).next().unwrap_or(pred);
    snake_case(local)
}

/// camelCase → snake_case (deterministic ASCII transform used for report
/// field and class names: `queryAnswers` → `query_answers`,
/// `SymbolicRegressionReport` → `symbolic_regression_report`).
fn snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Deterministic sort key for report values: literals first (by value), then
/// nodes by IRI.
fn report_term_key(term: &Term) -> String {
    match term {
        Term::Literal(l) => format!("0:{}", l.value()),
        Term::NamedNode(n) => format!("1:{}", n.as_str()),
        Term::BlankNode(b) => format!("1_:{}", b.as_str()),
    }
}

/// If `term` is the head of an `rdf:first`/`rdf:rest` chain, expand it to its
/// element terms. `None` when the term is a plain value or an empty list.
fn report_list_chain(graph: &Graph, term: &Term, prefixes: &SemPrefixes) -> Option<Vec<Term>> {
    let head = match term {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
        _ => return None,
    };
    let first = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
    let rest = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
    let nil = prefixes
        .resolve("rdf:nil")
        .unwrap_or_else(|| "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil".into());
    // `rdf:nil` itself is the empty list (engines point list-valued properties
    // straight at it when there is nothing to report).
    if node_is_nil(&head, &nil) {
        return Some(Vec::new());
    }
    let mut out: Vec<Term> = Vec::new();
    let mut current = head;
    let mut guard = 0usize;
    loop {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        if node_is_nil(&current, &nil) {
            break;
        }
        let firsts: Vec<Term> = graph
            .iter()
            .filter(|t| t.subject == current.as_ref() && t.predicate.as_str() == first)
            .map(|t| t.object.into_owned())
            .collect();
        let rests: Vec<Term> = graph
            .iter()
            .filter(|t| t.subject == current.as_ref() && t.predicate.as_str() == rest)
            .map(|t| t.object.into_owned())
            .collect();
        if let Some(f) = firsts.into_iter().next() {
            out.push(f);
        }
        current = match rests.into_iter().next() {
            Some(Term::NamedNode(n)) => NamedOrBlankNode::NamedNode(n),
            Some(Term::BlankNode(b)) => NamedOrBlankNode::BlankNode(b),
            _ => break,
        };
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Whether a report node is `rdf:nil` (the empty list terminator/head).
fn node_is_nil(node: &NamedOrBlankNode, nil: &str) -> bool {
    match node {
        NamedOrBlankNode::NamedNode(n) => n.as_str() == nil,
        NamedOrBlankNode::BlankNode(_) => false,
    }
}

/// The deterministic walker.
struct Renderer<'a> {
    graph: &'a Graph,
    prefixes: &'a SemPrefixes,
    g: &'a Grammar,
    mode: AnnotationMode,
    order: Vec<NamedNode>,
    visited: std::collections::BTreeSet<String>,
    /// Subjects already rendered inline as a nested form inside a parent
    /// (list items, expr props) — skip them in the top-level pass.
    inline_rendered: std::collections::BTreeSet<String>,
}

impl<'a> Renderer<'a> {
    fn new(graph: &'a Graph, prefixes: &'a SemPrefixes, g: &'a Grammar) -> Self {
        Self {
            graph,
            prefixes,
            g,
            mode: AnnotationMode::None,
            order: Vec::new(),
            visited: std::collections::BTreeSet::new(),
            inline_rendered: std::collections::BTreeSet::new(),
        }
    }

    fn with_mode(mut self, mode: AnnotationMode) -> Self {
        self.mode = mode;
        self
    }

    /// The `[ex:…]` annotation for a node: its IRI rendered in compact
    /// prefixed form, falling back to `<absolute-iri>` when no prefix
    /// matches (§4 of the design).
    fn annotation_for(&self, node: &NamedNode) -> String {
        match self.prefixes.compact(node.as_str()) {
            Some(c) => format!("[{c}]"),
            None => format!("[<{}>]", node.as_str()),
        }
    }

    /// Whether a top-level form node gets an annotation. `tier1` marks a
    /// root-linked slot (always annotated in Selective mode); otherwise the
    /// node's form name must be in the grammar's Tier 2 `annotated_forms`.
    /// Full annotates everything; None nothing.
    fn annotate_form(&self, node: &NamedNode, tier1: bool) -> bool {
        match self.mode {
            AnnotationMode::None => false,
            AnnotationMode::Full => true,
            AnnotationMode::Selective => {
                tier1
                    || self
                        .form_for_class(&self.type_of(node))
                        .is_some_and(|(name, _)| self.g.annotated_forms.contains(&name))
            }
        }
    }

    /// Whether an inline element (expression atom, nested form, referenced
    /// node) gets an annotation: Full mode only — Selective keeps expression
    /// internals bare (§6.2).
    fn annotate_inline(&self) -> bool {
        self.mode == AnnotationMode::Full
    }

    fn render_document(&mut self, root: &NamedNode) -> Result<String, SemError> {
        // Deterministic reference order starting from the root.
        self.walk(root);

        let mut args: Vec<String> = Vec::new();
        let mut emitted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let root_iri = root.as_str().to_string();

        // 0. Root-as-entry: when the root's own class is the entry form's
        //    class (temporal `problem`), render the root's props directly as
        //    the entry args; children are inlined (and marked) by
        //    `render_form_args`. The root itself must not re-emit later.
        let root_class = self.type_of(root);
        let root_is_entry = self.g.form(self.g.entry).is_some_and(|spec| {
            self.prefixes.resolve(spec.class).as_deref() == Some(root_class.as_str())
        });
        if root_is_entry {
            if let Some(spec) = self.g.form(self.g.entry) {
                args = self.render_form_args(root, spec)?;
            }
            emitted.insert(root_iri.clone());
        }

        // 1. Root-linked subjects first. Use a single deterministic ordering
        //    across all root predicates, matching compiler allocation order;
        //    otherwise a probability followed by a rule is reordered because
        //    the grammar groups predicates by form kind.
        let mut root_items: Vec<(usize, String, NamedNode)> = Vec::new();
        for (index, (_, pred)) in self.g.root_links.iter().enumerate() {
            for object in self.objects(root, pred) {
                if let Term::NamedNode(node) = object {
                    root_items.push((index, node.as_str().to_string(), node));
                }
            }
        }
        root_items.sort_by_key(|(_, iri, _)| iri.clone());
        for (_, iri, node) in root_items {
            if emitted.insert(iri) {
                // Tier 1: root-linked slots are always annotated in Selective.
                if let Some(s) = self.render_form(&node, true)? {
                    args.push(s);
                }
            }
        }
        /*
        for (_, pred) in &self.g.root_links {
            for o in self.objects(root, pred) {
                if let Term::NamedNode(n) = o {
                    let iri = n.as_str().to_string();
                    if emitted.insert(iri.clone()) {
                        let node =
                            NamedNode::new(iri).map_err(|e| SemError::Lexical(e.to_string()))?;
                        if let Some(s) = self.render_form(&node)? {
                            args.push(s);
                        }
                    }
                }
            }
        }
        */

        // 2. Entry-level literal props (logic/probabilistic rules, provenance traversal).
        // Structured logic/probability links are handled by the root-link pass;
        // preserve their original order rather than sorting object terms.
        if !self.g.entry_props.is_empty() {
            for p in &self.g.entry_props {
                // Str-typed entry props carry only literals (rule strings);
                // when the same predicate is also a structured root link
                // (`logic:rule` → rule nodes), the node objects belong to the
                // root-link pass, not here.
                let vals: Vec<Term> = self
                    .objects(root, p.pred)
                    .into_iter()
                    .filter(|t| p.ty != ArgType::Str || matches!(t, Term::Literal(_)))
                    .collect();
                let mut vals = vals
                    .iter()
                    .map(|t| self.render_value(t, p))
                    .collect::<Result<Vec<_>, _>>()?;
                if !vals.is_empty() {
                    if p.multi || p.list {
                        args.push(self.format_list(&vals));
                    } else {
                        args.push(vals.remove(0));
                    }
                }
            }
        }

        // 3. Every other referenced subject in canonical form order. Named
        // declarations follow the source's semantic declaration convention;
        // generated nodes retain first-reference order.
        let mut order: Vec<NamedNode> = self.order.clone();
        order.sort_by_key(|node| {
            let class = self.compact_class(&self.type_of(node));
            let local = self.local_name(node).unwrap_or_default();
            let rank = match class.as_str() {
                "model:Parameter" => 0,
                "model:Variable" => 1,
                _ => 2,
            };
            (rank, local, node.as_str().to_string())
        });
        for node in &order {
            let iri = node.as_str().to_string();
            if iri == root_iri || self.inline_rendered.contains(&iri) {
                continue;
            }
            if emitted.insert(iri.clone()) {
                // Tier 2: declarations annotate when their form is listed in
                // the grammar's `annotated_forms`.
                if let Some(s) = self.render_form(node, false)? {
                    args.push(s);
                }
            }
        }

        // 4. Standalone top-level fallback (R9 inverse). A document whose top
        //    form is a content form with no root-link predicate (e.g.
        //    `causal.observe("y")`, `temporal.constraint(a="a",
        //    b="b", relation=before)`) compiles to the synthesized root plus
        //    an *orphaned* form node the walk can never reach. When the entry
        //    path produced nothing, scan every typed subject for exactly one
        //    unconsumed module content form and emit it as a bare top-level
        //    call — never an empty `module.entry()` that cannot recompile.
        if args.is_empty() {
            let mut candidates: Vec<String> = Vec::new();
            let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for t in self.graph.iter() {
                let NamedOrBlankNodeRef::NamedNode(subj) = &t.subject else {
                    continue;
                };
                if t.predicate.as_str() != rdf::TYPE.as_str() {
                    continue;
                }
                let TermRef::NamedNode(class) = &t.object else {
                    continue;
                };
                let iri = subj.as_str().to_string();
                if iri == root_iri
                    || emitted.contains(&iri)
                    || self.inline_rendered.contains(&iri)
                    || !seen.insert(iri.clone())
                {
                    continue;
                }
                let Some((name, _)) = self.form_for_class(class.as_str()) else {
                    continue;
                };
                if !self.g.forms.contains_key(name) {
                    continue; // expression nodes inline; not top-level forms
                }
                let node = match NamedNode::new(iri) {
                    Ok(n) => n,
                    Err(e) => return Err(SemError::Lexical(e.to_string())),
                };
                if let Some(s) = self.render_form(&node, true)? {
                    candidates.push(s);
                }
            }
            if candidates.len() == 1 {
                return Ok(format!("{}.{}", self.g.name, candidates[0]));
            }
        }

        Ok(format!(
            "{}.{}({})",
            self.g.name,
            self.g.entry,
            args.join(", ")
        ))
    }

    /// Deterministic first-reference order walk (BFS by triple reference).
    /// Pushes both named-node and blank-node subjects.
    fn walk(&mut self, root: &NamedNode) {
        let mut queue: Vec<NamedOrBlankNode> = vec![NamedOrBlankNode::NamedNode(root.clone())];
        while let Some(node) = queue.pop() {
            let key = match &node {
                NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
                NamedOrBlankNode::BlankNode(b) => b.as_str().to_string(),
            };
            if !self.visited.insert(key.clone()) {
                continue;
            }
            if let NamedOrBlankNode::NamedNode(n) = &node {
                self.order.push(n.clone());
            }
            let mut refs: Vec<Term> = Vec::new();
            for t in self.graph.iter() {
                if t.subject == node.as_ref() {
                    refs.push(t.object.into_owned());
                }
            }
            for r in refs.into_iter().rev() {
                match r {
                    Term::NamedNode(n) => queue.push(NamedOrBlankNode::NamedNode(n)),
                    Term::BlankNode(b) => queue.push(NamedOrBlankNode::BlankNode(b)),
                    _ => {}
                }
            }
        }
    }

    /// Render one form for a subject; `None` when the subject is not a
    /// top-level form (an expression node or the root itself — those are
    /// inlined or handled separately). `tier1` selects the annotation tier
    /// (root-linked slot vs. grammar-declared declaration).
    fn render_form(&mut self, node: &NamedNode, tier1: bool) -> Result<Option<String>, SemError> {
        let class = self.type_of(node);
        let Some((name, spec)) = self.form_for_class(&class) else {
            return Ok(None);
        };
        if self.g.exprs.contains_key(name) {
            // Expression nodes inline inside their parent form (R8 inverse).
            return Ok(None);
        }
        let spec = spec.clone();
        let args = self.render_form_args(node, &spec)?;
        // The fragment sits between the form name and its parens —
        // `eq[ex:eq1](…)` (§1 of the design).
        let ann = if self.annotate_form(node, tier1) {
            self.annotation_for(node)
        } else {
            String::new()
        };
        Ok(Some(format!("{name}{ann}({})", args.join(", "))))
    }

    /// Render the args of a form (or the entry form on the root): the
    /// subject-derived name first for `Named` subjects, then one arg per prop
    /// in canonical order. List props bracket when the grammar marks them
    /// bracketed (sequential lists need the delimiter); unbracketed Expr lists
    /// flatten into positional args (R8 inverse, e.g. `sum(a, b, c)`).
    ///
    /// Forms with any optional prop render all props as keyword args — a
    /// skipped optional prop would otherwise shift later positionals (e.g.
    /// `event_model`'s empty `initial` sliding `numeric_fluents` into its
    /// slot). Keyword binding is order-independent, so the round-trip holds.
    fn render_form_args(
        &mut self,
        node: &NamedNode,
        spec: &FormSpec,
    ) -> Result<Vec<String>, SemError> {
        let kwarg_style = if spec.props.iter().any(|p| p.kwarg) {
            // Keyword-style props: render `name=value` whenever a kwarg-marked
            // prop is present, or when a required prop is missing (positional
            // alignment would otherwise break). Plain forms (`query(rain)`,
            // `probability(0.3, rain)`) keep their positional canonical form.
            spec.props
                .iter()
                .any(|p| p.kwarg && !self.prop_values(node, p).is_empty())
                || spec
                    .props
                    .iter()
                    .any(|p| !p.opt && self.prop_values(node, p).is_empty())
        } else {
            // Conservative: forms with any optional prop render all props as
            // keyword args — a skipped optional prop would otherwise shift
            // later positionals (e.g. `event_model`'s empty `initial`).
            spec.props.iter().any(|p| p.opt)
        };
        let mut args: Vec<String> = Vec::new();
        match spec.subj {
            SubjKind::Named => {
                // First arg is the name (subject-derived, R7 inverse).
                if let Some(local) = self.local_name(node) {
                    args.push(local);
                }
            }
            SubjKind::Generated(_) => {}
        }

        for p in &spec.props {
            if p.pred.is_empty() {
                continue; // name prop — already emitted as the subject's local name
            }
            let vals = self
                .prop_values(node, p)
                .into_iter()
                .map(|t| self.render_value(&t, p))
                .collect::<Result<Vec<_>, _>>()?;
            if vals.is_empty() {
                continue;
            }
            let rendered = if p.list {
                if p.bracketed {
                    self.format_list(&vals)
                } else if p.ty == ArgType::Expr {
                    // Expr list props render as positional args (R8 inverse).
                    args.extend(vals);
                    continue;
                } else {
                    self.format_list(&vals)
                }
            } else {
                vals.join(", ")
            };
            if kwarg_style {
                args.push(format!("{}={}", p.name, rendered));
            } else {
                args.push(rendered);
            }
        }
        Ok(args)
    }

    /// Render one property value; Expr-typed props inline (R8 inverse), Kind
    /// literals/objects render as bare symbols (R10 inverse). KindNode props
    /// return 2 values (kind, operand). Node references (e.g. an equation's
    /// `lhs` variable) carry a Full-mode URI annotation; kinds and literals
    /// stay bare.
    fn render_value(&mut self, term: &Term, prop: &Prop) -> Result<String, SemError> {
        if prop.ty == ArgType::Expr {
            return self.render_expr(term);
        }
        if prop.ty == ArgType::Kind {
            match term {
                Term::Literal(l) => return Ok(l.value().to_string()),
                Term::NamedNode(n) => {
                    // A nested form/expression in a kind list (e.g. epistemic
                    // `asks=[ask(...)]`) renders as the call; kind-object
                    // constants (typed `{schema}/{Variant}` or untyped data
                    // IRIs) never match a spec class, so they fall through to
                    // the variant extraction below.
                    let class = self.type_of(n);
                    if self.form_for_class(&class).is_some() {
                        return self.render_expr(term);
                    }
                    // Kind-object constant node: extract the variant name.
                    if let Some(variant) = self.kind_variant_from_node(n, prop) {
                        return Ok(variant);
                    }
                    return self.term_to_value(term);
                }
                _ => {}
            }
        }
        // A nested list (e.g. info channel `transition` rows): the item is an
        // `rdf:List` head — render it as a bracketed sub-list.
        if let Some(list) = self.nested_list(term)? {
            return Ok(self.format_list(&list));
        }
        let value = self.term_to_value(term)?;
        if let Term::NamedNode(n) = term {
            if self.annotate_inline() {
                return Ok(format!("{value}{}", self.annotation_for(n)));
            }
        }
        Ok(value)
    }

    /// If `term` is the head of an `rdf:first`/`rdf:rest` chain, expand it to
    /// its element strings (used for nested list props like channel
    /// transitions). `None` when the term is a plain value.
    fn nested_list(&self, term: &Term) -> Result<Option<Vec<String>>, SemError> {
        let head = match term {
            Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
            Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
            _ => return Ok(None),
        };
        let chain = self.list_chain(&head);
        if chain.is_empty() {
            return Ok(None);
        }
        chain
            .iter()
            .map(|t| self.term_to_value(t))
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// Append a Full-mode URI annotation to an inline rendered element.
    fn ann_inline(&self, text: String, node: &NamedNode) -> String {
        if self.annotate_inline() {
            format!("{text}{}", self.annotation_for(node))
        } else {
            text
        }
    }

    /// Render a referenced term in expression position.
    fn render_expr(&mut self, term: &Term) -> Result<String, SemError> {
        let Term::NamedNode(node) = term else {
            return self.term_to_value(term);
        };
        let full_class = self.type_of(node);
        let class = self.compact_class(&full_class);
        match class.as_str() {
            "model:Constant" => {
                // R5 inverse: constant → its numeric value.
                if let Some(v) = self.first_object(node, "model:value") {
                    return Ok(self.ann_inline(self.term_to_value(&v)?, node));
                }
                Ok(self.ann_inline(self.local_name(node).unwrap_or_default(), node))
            }
            "model:Variable" | "model:Parameter" => {
                // R4/R6 inverse: bare name.
                Ok(self.ann_inline(self.local_name(node).unwrap_or_default(), node))
            }
            "logic:Variable" => {
                // `?name` — the explicit variable reference (logic SEM §4.1
                // inverse: the `?` must round-trip).
                let name = self
                    .first_object(node, "logic:name")
                    .and_then(|t| match t {
                        Term::Literal(l) => Some(l.value().to_string()),
                        _ => None,
                    })
                    .unwrap_or_default();
                Ok(self.ann_inline(format!("?{name}"), node))
            }
            "logic:Atom" => {
                // Dynamic predicate application (§4.5 inverse): read the
                // predicate name + ordered arguments and render `pred(a, b)`.
                let name = self
                    .first_object(node, "logic:predicate")
                    .and_then(|t| match t {
                        Term::Literal(l) => Some(l.value().to_string()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let mut args: Vec<String> = Vec::new();
                for o in self.objects(node, "logic:arguments") {
                    match &o {
                        Term::NamedNode(n) => {
                            let head = NamedOrBlankNode::NamedNode(n.clone());
                            let chain = self.list_chain(&head);
                            if chain.is_empty() {
                                // Single argument: render it as an expression
                                // (variables → `?name`, constants → name,
                                // numbers → literal) rather than its local id.
                                args.push(self.render_expr(&o)?);
                            } else {
                                for t in chain {
                                    args.push(self.render_expr(&t)?);
                                }
                            }
                        }
                        Term::BlankNode(b) => {
                            let head = NamedOrBlankNode::BlankNode(b.clone());
                            let chain = self.list_chain(&head);
                            for t in chain {
                                args.push(self.render_expr(&t)?);
                            }
                        }
                        other => args.push(self.term_to_value(other)?),
                    }
                }
                // Keyword arguments: `name=value` pairs appended after the
                // positional args.
                let mut kwargs: Vec<String> = Vec::new();
                for o in self.objects(node, "logic:kwarguments") {
                    if let Term::NamedNode(n) = &o {
                        let head = NamedOrBlankNode::NamedNode(n.clone());
                        let chain = self.list_chain(&head);
                        let mut it = chain.into_iter();
                        while let Some(k) = it.next() {
                            if let Some(v) = it.next() {
                                if let Term::Literal(l) = &k {
                                    let val = self.render_expr(&v)?;
                                    kwargs.push(format!("{}={val}", l.value()));
                                }
                            }
                        }
                    }
                }
                let mut parts = args;
                parts.extend(kwargs);
                self.inline_rendered.insert(node.as_str().to_string());
                let ann = if self.annotate_inline() {
                    self.annotation_for(node)
                } else {
                    String::new()
                };
                Ok(format!("{name}{ann}({})", parts.join(", ")))
            }
            "model:List" => {
                // A list-valued expression position (`data=[...]`, an
                // `actual_world` list): render `[item, ...]` with each item
                // as an expression.
                let mut items: Vec<String> = Vec::new();
                for o in self.objects(node, "model:items") {
                    if let Term::NamedNode(n) = &o {
                        for t in self.list_chain(&NamedOrBlankNode::NamedNode(n.clone())) {
                            items.push(self.render_expr(&t)?);
                        }
                    } else if let Term::BlankNode(b) = &o {
                        for t in self.list_chain(&NamedOrBlankNode::BlankNode(b.clone())) {
                            items.push(self.render_expr(&t)?);
                        }
                    }
                }
                self.inline_rendered.insert(node.as_str().to_string());
                let ann = if self.annotate_inline() {
                    self.annotation_for(node)
                } else {
                    String::new()
                };
                Ok(format!("[{}]{ann}", items.join(", ")))
            }
            "logic:Comparison" => {
                // The six comparison forms share this class; the operator is
                // stored as a `logic:op` literal (§4.4 inverse).
                let op = self
                    .first_object(node, "logic:op")
                    .and_then(|t| match t {
                        Term::Literal(l) => Some(l.value().to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "eq".to_string());
                let mut parts: Vec<String> = Vec::new();
                for o in self.objects(node, "logic:left") {
                    parts.push(self.render_expr(&o)?);
                }
                for o in self.objects(node, "logic:right") {
                    parts.push(self.render_expr(&o)?);
                }
                self.inline_rendered.insert(node.as_str().to_string());
                let ann = if self.annotate_inline() {
                    self.annotation_for(node)
                } else {
                    String::new()
                };
                Ok(format!("{op}{ann}({})", parts.join(", ")))
            }
            _ => {
                // Expression node → nested form, with its own props inlined.
                let Some((name, spec)) = self.form_for_class(&full_class) else {
                    return Err(SemError::UnknownFunction(format!(
                        "no expression form for class {class}"
                    )));
                };
                let spec = spec.clone();
                let args = self.render_form_args(node, &spec)?;
                // This subject is now part of a parent form; skip it in the
                // top-level pass (duplicate-emission guard).
                self.inline_rendered.insert(node.as_str().to_string());
                let ann = if self.annotate_inline() {
                    self.annotation_for(node)
                } else {
                    String::new()
                };
                Ok(format!("{name}{ann}({})", args.join(", ")))
            }
        }
    }

    /// Compact a class IRI (`…#Variable`) back to its prefixed form.
    fn compact_class(&self, class: &str) -> String {
        self.prefixes
            .compact(class)
            .unwrap_or_else(|| class.to_string())
    }

    fn form_for_class(&self, class: &str) -> Option<(&'static str, &FormSpec)> {
        for (name, spec) in self.g.forms.iter().chain(self.g.exprs.iter()) {
            if self.prefixes.resolve(spec.class).as_deref() == Some(class) {
                return Some((name, spec));
            }
        }
        None
    }

    fn type_of(&self, node: &NamedNode) -> String {
        let type_pred = rdf::TYPE.into_owned();
        for t in self.graph.iter() {
            if t.subject == NamedOrBlankNode::from(node.clone()).as_ref()
                && t.predicate == type_pred.as_ref()
            {
                if let TermRef::NamedNode(o) = &t.object {
                    return o.as_str().to_string();
                }
            }
        }
        String::new()
    }

    fn objects(&self, node: &NamedNode, pred: &str) -> Vec<Term> {
        let pred_iri = self.lookup_pred(pred);
        let mut out: Vec<Term> = self
            .graph
            .iter()
            .filter(|t| {
                t.subject == NamedOrBlankNode::from(node.clone()).as_ref()
                    && t.predicate.as_str() == pred_iri
            })
            .map(|t| t.object.into_owned())
            .collect();
        out.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
        out
    }

    /// Like `objects` but for blank-node subjects.
    fn blank_objects(&self, node: &BlankNode, pred: &str) -> Vec<Term> {
        let pred_iri = self.lookup_pred(pred);
        let out: Vec<Term> = self
            .graph
            .iter()
            .filter(|t| {
                t.subject == NamedOrBlankNode::from(node.clone()).as_ref()
                    && t.predicate.as_str() == pred_iri
            })
            .map(|t| t.object.into_owned())
            .collect();
        out
    }

    fn lookup_pred(&self, pred: &str) -> String {
        if pred.starts_with("rdf:") {
            format!("http://www.w3.org/1999/02/22-rdf-syntax-ns#{}", &pred[4..])
        } else {
            self.prefixes
                .resolve(pred)
                .unwrap_or_else(|| pred.to_string())
        }
    }

    /// The values of a property, expanding `rdf:List` chains for list props
    /// (e.g. `model:operands` → the operand terms in order). Handles both
    /// named and blank-node list cells. For KindNode props, returns [kind, operand].
    fn prop_values(&self, node: &NamedNode, p: &Prop) -> Vec<Term> {
        let vals = self.objects(node, p.pred);
        if matches!(p.kind_mode, KindMode::Node { .. }) {
            // KindNode: the value is the op subnode; extract kind + operand.
            return vals
                .into_iter()
                .flat_map(|v| self.expand_kind_node(&v))
                .collect();
        }
        if !p.list {
            return vals;
        }
        // Expand each head into its rdf:first/rdf:rest chain.
        let mut out = Vec::new();
        for v in vals {
            match &v {
                Term::NamedNode(n) => {
                    out.extend(self.list_chain(&NamedOrBlankNode::NamedNode(n.clone())))
                }
                Term::BlankNode(b) => {
                    out.extend(self.list_chain(&NamedOrBlankNode::BlankNode(b.clone())))
                }
                other => out.push(other.clone()),
            }
        }
        out
    }

    /// Walk an `rdf:first`/`rdf:rest` chain from a list head. Handles both
    /// NamedNode and BlankNode cells (some Turtle input uses blank-node list
    /// syntax).
    fn list_chain(&self, head: &NamedOrBlankNode) -> Vec<Term> {
        let mut out = Vec::new();
        let nil = self
            .prefixes
            .resolve("rdf:nil")
            .unwrap_or_else(|| "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil".into());
        let mut current: NamedOrBlankNode = head.clone();
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            if current.as_ref().to_string() == nil {
                break;
            }
            let firsts: Vec<Term> = match &current {
                NamedOrBlankNode::NamedNode(n) => self.objects(n, "rdf:first"),
                NamedOrBlankNode::BlankNode(b) => self.blank_objects(b, "rdf:first"),
            };
            let rests: Vec<Term> = match &current {
                NamedOrBlankNode::NamedNode(n) => self.objects(n, "rdf:rest"),
                NamedOrBlankNode::BlankNode(b) => self.blank_objects(b, "rdf:rest"),
            };
            if let Some(f) = firsts.into_iter().next() {
                out.push(f);
            }
            current = match rests.into_iter().next() {
                Some(Term::NamedNode(n)) => NamedOrBlankNode::NamedNode(n),
                Some(Term::BlankNode(b)) => NamedOrBlankNode::BlankNode(b),
                _ => break,
            };
        }
        out
    }

    fn first_object(&self, node: &NamedNode, pred: &str) -> Option<Term> {
        self.objects(node, pred).into_iter().next()
    }

    /// Extract the variant name from a kind-object constant node by reading the
    /// constant's `rdf:type` (`{schema}/{Enum/Variant}`) or the IRI's last path
    /// segment.
    fn kind_variant_from_node(&self, node: &NamedNode, prop: &Prop) -> Option<String> {
        let KindMode::Object { enum_name, .. } = prop.kind_mode else {
            return None;
        };
        // The rdf:type is `{schema}{Enum}/{Variant}`; take the last segment.
        let class = self.type_of(node);
        if !class.is_empty() {
            if let Some(local) = class.rsplit(['#', '/']).next() {
                if local.to_lowercase() != enum_name.to_lowercase() {
                    return Some(local.to_lowercase());
                }
            }
        }
        // Fall back to the IRI's last segment.
        node.as_str()
            .rsplit(['#', '/'])
            .next()
            .map(|s| s.to_lowercase())
    }

    /// For a KindNode prop (event `op`), expand the op subnode into its kind
    /// symbol and its operand literal.
    fn expand_kind_node(&self, term: &Term) -> Vec<Term> {
        let Term::NamedNode(node) = term else {
            return vec![term.clone()];
        };
        let full_class = self.type_of(node);
        let compact = self.compact_class(&full_class);
        let kind = match compact.rsplit('/').next() {
            Some(k) => k.to_lowercase(),
            None => String::new(),
        };
        let mut out = vec![Term::from(Literal::new_simple_literal(kind))];
        // The operand is a literal on the op node (find the non-type object).
        for t in self.graph.iter() {
            if t.subject == NamedOrBlankNode::from(node.clone()).as_ref()
                && t.predicate.as_str() != rdf::TYPE.as_str()
            {
                if let TermRef::Literal(_) = &t.object {
                    out.push(t.object.into_owned());
                }
            }
        }
        out
    }

    fn local_name(&self, node: &NamedNode) -> Option<String> {
        self.prefixes
            .compact(node.as_str())
            .and_then(|c| c.strip_prefix("ex:").map(|s| s.to_string()))
    }

    fn term_to_value(&self, term: &Term) -> Result<String, SemError> {
        match term {
            Term::NamedNode(n) => {
                if let Some(local) = self.local_name(n) {
                    return Ok(local);
                }
                Ok(format!("<{}>", n.as_str()))
            }
            Term::BlankNode(_) => Ok("_".to_string()),
            Term::Literal(l) => Ok(literal_to_value(l)),
        }
    }

    fn format_list(&self, vals: &[String]) -> String {
        format!("[{}]", vals.join(", "))
    }
}

fn literal_to_value(l: &Literal) -> String {
    let dt = l.datatype();
    if dt.as_str() == xsd::BOOLEAN.as_str() {
        l.value().to_string()
    } else if dt.as_str() == xsd::INTEGER.as_str()
        || dt.as_str() == xsd::DOUBLE.as_str()
        || dt.as_str() == xsd::FLOAT.as_str()
        || dt.as_str() == xsd::DECIMAL.as_str()
        || dt.as_str() == xsd::LONG.as_str()
        || dt.as_str() == xsd::INT.as_str()
        || dt.as_str() == xsd::SHORT.as_str()
        || dt.as_str() == xsd::BYTE.as_str()
        || dt.as_str() == xsd::UNSIGNED_LONG.as_str()
        || dt.as_str() == xsd::UNSIGNED_INT.as_str()
        || dt.as_str() == xsd::UNSIGNED_SHORT.as_str()
        || dt.as_str() == xsd::UNSIGNED_BYTE.as_str()
    {
        l.value().to_string()
    } else if dt.as_str() == "https://example.org/ns/model#Rational" {
        // Exact rational: `fraction(numerator, denominator)` so the SEM
        // round-trip preserves the value as a typed fraction, not a string.
        match l.value().split_once('/') {
            Some((n, d)) => format!("fraction({n}, {d})"),
            None => format!("\"{}\"", l.value()),
        }
    } else {
        format!("\"{}\"", l.value())
    }
}

/// Find the root subject of a graph: the single subject typed with the
/// module's root class (e.g. `model:Model`). Mirrors `reader::find_root`'s
/// contract but is computed from the SEM lane's own prefix table.
pub fn find_root(graph: &Graph, root_class: &str) -> Option<NamedNode> {
    let prefixes = SemPrefixes::default();
    let root_iri = prefixes.resolve(root_class)?;
    let type_pred = rdf::TYPE.into_owned();
    let mut found: Vec<NamedNode> = Vec::new();
    for t in graph.iter() {
        if t.predicate == type_pred.as_ref() {
            if let (NamedOrBlankNodeRef::NamedNode(s), TermRef::NamedNode(o)) =
                (&t.subject, &t.object)
            {
                if o.as_str() == root_iri {
                    found.push(NamedNode::new(s.as_str()).ok()?);
                }
            }
        }
    }
    found.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    found.into_iter().next()
}
