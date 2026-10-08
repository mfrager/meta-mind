//! SEM compiler (§4.1 of `planning/s_expr/s_expr_design2.md`): document →
//! final RDF `Graph`, directly. There is no cRDF step, no `Vec<RawResource>`,
//! no JSON round-trip anywhere in this lane.
//!
//! Rules implemented: R1 one-document (parser), R2 scope (parser), R3/R11
//! type checks, R4 implicit variables, R5 implicit constants, R6 declared
//! symbols shadow implicit, R7 auto subjects, R8 expression inlining, R9
//! entry auto-rooting (generalized `model_edit` wiring), R10 kind symbols.

use math_core::oxigraph::model::vocab::{rdf, xsd};
use math_core::oxigraph::model::{Graph, Literal, NamedNode, Term, Triple};

use crate::sem::ast::{Atom, Document, Form, Value};
use crate::sem::errors::SemError;
use crate::sem::grammar::{
    grammar, ArgType, FormSpec, Grammar, KindMode, Prop, SemPrefixes, SubjKind,
};
use crate::sem::parse::parse;

/// The compiled artifact: the final RDF graph plus its root node.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub graph: Graph,
    pub root: NamedNode,
}

/// Compile a SEM document to a final RDF `Graph` (and its root subject).
pub fn compile(doc: &str) -> Result<Compiled, SemError> {
    compile_with(doc, &SemPrefixes::default())
}

/// Compile with an explicit prefix table (tests and tool layers inject their
/// own registry; the lane itself never touches the cRDF codec).
pub fn compile_with(doc: &str, prefixes: &SemPrefixes) -> Result<Compiled, SemError> {
    compile_with_seed(doc, prefixes, "")
}

/// Compile with an explicit prefix table and a per-compilation subject seed.
/// The seed is prepended to every *generated* subject local (`ex:{seed}eq0`)
/// so chunks compiled separately (e.g. successive `model_edit` operations on
/// one model) don't collide on the deterministic R7/R9 names. Named subjects
/// (variables, parameters, the model root `ex:M`) are unaffected, so edits
/// still reference the model's declared symbols.
pub fn compile_with_seed(
    doc: &str,
    prefixes: &SemPrefixes,
    seed: &str,
) -> Result<Compiled, SemError> {
    let parsed = parse(doc)?;
    let g =
        grammar(&parsed.module).ok_or_else(|| SemError::UnknownModule(parsed.module.clone()))?;
    let mut c = Compiler {
        g: &g,
        prefixes,
        seed,
        graph: Graph::new(),
        counters: std::collections::BTreeMap::new(),
        declared: std::collections::BTreeMap::new(),
    };
    let root = c.compile_document(&parsed)?;
    Ok(Compiled {
        graph: c.graph,
        root,
    })
}

/// Compile a SEM document and serialize the result to Turtle with the
/// standard prefixes (the convenience the tool layer uses).
pub fn compile_to_turtle(doc: &str) -> Result<String, SemError> {
    compile_to_turtle_seeded(doc, "")
}

/// [`compile_to_turtle`] with a per-compilation subject seed (see
/// [`compile_with_seed`]).
pub fn compile_to_turtle_seeded(doc: &str, seed: &str) -> Result<String, SemError> {
    let prefixes = SemPrefixes::default();
    let compiled = compile_with_seed(doc, &prefixes, seed)?;
    let mut out = String::new();
    let mut decls: Vec<(&str, &str)> = prefixes
        .prefix_to_iri
        .iter()
        .map(|(p, i)| (*p, *i))
        .collect();
    decls.sort_by_key(|(p, _)| *p);
    for (p, i) in &decls {
        out.push_str(&format!("@prefix {p}: <{i}> .\n"));
    }
    if !decls.is_empty() {
        out.push('\n');
    }
    let ttl =
        crate::io::to_turtle(&compiled.graph).map_err(|e| SemError::Lexical(e.to_string()))?;
    out.push_str(&ttl);
    Ok(out)
}

struct Compiler<'a> {
    g: &'a Grammar,
    prefixes: &'a SemPrefixes,
    /// Per-compilation subject seed (R7/R9 uniqueness across chunks).
    seed: &'a str,
    graph: Graph,
    /// Per-semantic-prefix subject counters. Each generated prefix has an
    /// independent sequence, so nested forms and list cells cannot affect one
    /// another's identifiers.
    counters: std::collections::BTreeMap<&'static str, usize>,
    /// Declared names (R6): name → subject IRI (variables and parameters).
    declared: std::collections::BTreeMap<String, String>,
}

impl<'a> Compiler<'a> {
    /// Compile the document: entry form or auto-rooted content form (R9).
    fn compile_document(&mut self, doc: &Document) -> Result<NamedNode, SemError> {
        // Pass 1 (R6): collect declared names so declared symbols win over
        // implicit variables regardless of document order.
        self.collect_declared(doc)?;

        let top_name = doc.top.name.as_str();
        if top_name == self.g.entry {
            let root = self.root_node()?;
            if !self.g.entry_props.is_empty() {
                // Entry-level literal props (logic/probabilistic rules,
                // provenance traversal). A legacy atom/list payload
                // (`logic.model(["clause."])`, `provenance.query("d",
                // upstream)`) maps onto them directly; a structured payload
                // (`logic.model(fact(...), rule(...))`) compiles the content
                // forms instead (design §4, entry dispatch). An empty payload
                // stays on the legacy path (compat: a bare rule-less model).
                let has_form_args = doc.top.args.iter().any(|v| matches!(v, Value::Form(_)));
                let empty_payload = doc.top.args.is_empty() && doc.top.kwargs.is_empty();
                if empty_payload || !has_form_args {
                    self.compile_entry_props(&root, &doc.top)?;
                } else if has_form_args {
                    for arg in &doc.top.args {
                        self.compile_content(arg)?;
                    }
                    for (_, v) in &doc.top.kwargs {
                        self.compile_content(v)?;
                    }
                } else {
                    return Err(SemError::TypeMismatch(format!(
                        "{}: entry args cannot mix literals and content forms",
                        self.g.name
                    )));
                }
            } else if let Some(spec) = self.g.form(top_name) {
                // The entry is a real form with props compiled onto the root
                // (temporal `problem(intervals=[…], constraints=[…], ask=…)`).
                self.check_arity(&doc.top, spec)?;
                self.compile_props(&root, &doc.top, spec)?;
            } else {
                // Entry args are content forms (numerical/solver `model(…)`,
                // event `problem(model=…)`).
                for arg in &doc.top.args {
                    self.compile_content(arg)?;
                }
                for (_, v) in &doc.top.kwargs {
                    self.compile_content(v)?;
                }
            }
            Ok(root)
        } else {
            // R9: a root-linked content form at top level auto-roots.
            let spec = self.g.form(top_name).ok_or_else(|| {
                SemError::UnknownFunction(format!(
                    "module \"{}\" has no function \"{top_name}\"",
                    self.g.name
                ))
            })?;
            // Expression forms (`state`, `trial`, `distribution`, formula
            // forms) cannot auto-root: they are typed targets, not statements.
            if matches!(spec.subj, SubjKind::Generated("e")) {
                return Err(SemError::TypeMismatch(format!(
                    "`{top_name}` is an expression form and cannot be a model statement"
                )));
            }
            let root = self.root_node()?;
            self.compile_form_with(&root, &doc.top, spec)?;
            Ok(root)
        }
    }

    /// Pass 1: register every var/param name in the document (R6). Duplicate
    /// declarations are a `sem:e9`.
    fn collect_declared(&mut self, doc: &Document) -> Result<(), SemError> {
        let mut forms: Vec<&Form> = Vec::new();
        if doc.top.name == self.g.entry && self.g.entry_props.is_empty() {
            for arg in &doc.top.args {
                if let Value::Form(f) = arg {
                    forms.push(f);
                }
            }
        } else {
            forms.push(&doc.top);
        }
        for f in forms {
            let Some(spec) = self.g.form(&f.name) else {
                continue;
            };
            if spec.subj == SubjKind::Named {
                if let Some(Value::Atom(Atom::Sym(name))) = f.args.first() {
                    let iri = self.resolve_subject(name)?;
                    if self
                        .declared
                        .insert(name.clone(), iri.as_str().to_string())
                        .is_some()
                    {
                        return Err(SemError::DuplicateDefinition(format!(
                            "symbol \"{name}\" declared twice"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn root_node(&mut self) -> Result<NamedNode, SemError> {
        let iri = self.resolve_subject("M")?;
        self.emit_type(&iri, self.g.root_class)?;
        Ok(iri)
    }

    /// Compile one top-level content form argument of the entry form.
    fn compile_content(&mut self, value: &Value) -> Result<(), SemError> {
        let Value::Form(form) = value else {
            return Err(SemError::TypeMismatch(format!(
                "{}: entry form args must be content forms, got {value:?}",
                self.g.name
            )));
        };
        let spec = self.g.form(&form.name).ok_or_else(|| {
            SemError::UnknownFunction(format!(
                "module \"{}\" has no function \"{}\"",
                self.g.name, form.name
            ))
        })?;
        // Expression/formula forms (generated `e` subjects) are never valid
        // model statements: `state(...)`, `trial(...)`, `distribution(...)`,
        // `and(...)`, … are typed targets that only make sense nested inside
        // statements/queries. Rejecting them here prevents root-level
        // `state(...)` from compiling into an orphan node.
        if matches!(spec.subj, SubjKind::Generated("e")) {
            return Err(SemError::TypeMismatch(format!(
                "`{}` is an expression form and cannot be a model statement; use it inside query(...), rule(...), or probability(...)",
                form.name
            )));
        }
        let root = self.root_iri()?;
        self.compile_form_with(&root, form, spec)
    }

    /// Compile a form whose subject is known (root auto-wiring, R9) or a
    /// content form: emit the type, the properties, and the root link.
    fn compile_form_with(
        &mut self,
        root: &NamedNode,
        form: &Form,
        spec: &FormSpec,
    ) -> Result<(), SemError> {
        let subject = self.subject_for(form, spec)?;
        self.emit_type(&subject, spec.class)?;
        self.maybe_emit_compare_op(&subject, form, spec)?;

        // Property args map onto the spec's props in order; a list-typed prop
        // consumes all remaining args (e.g. `sum(a, b, c)` → operands list).
        self.check_arity(form, spec)?;
        self.compile_props(&subject, form, spec)?;

        // R9: root link for root-linked content forms.
        if let Some((_, pred)) = self.g.root_links.iter().find(|(f, _)| *f == form.name) {
            let p = self.resolve_pred(pred)?;
            self.graph
                .insert(&Triple::new(root.clone(), p, Term::from(subject.clone())));
        }
        Ok(())
    }

    /// R11 arity: required scalar props must be present; a list-typed prop
    /// consumes any remaining args; without a list prop, extra args are an error.
    /// KindNode props count as 2 args (kind + operand).
    fn check_arity(&self, form: &Form, spec: &FormSpec) -> Result<(), SemError> {
        let mut required = 0usize;
        let mut max_args = 0usize;
        let mut open_ended = false;
        for p in &spec.props {
            // Properties with an empty predicate are metadata/name slots and
            // are not emitted; they still consume one positional value for a
            // named form. Optional properties may be omitted entirely.
            let slots = if matches!(p.kind_mode, KindMode::Node { .. }) {
                2
            } else {
                1
            };
            if p.list || p.multi {
                open_ended = true;
                if !p.opt {
                    required += 1;
                }
            } else if !p.opt {
                required += slots;
                max_args += slots;
            } else {
                max_args += slots;
            }
        }
        // Keywords bind named properties and do not consume positional slots.
        // Count only positional values for positional arity, while counting
        // recognized keywords toward required properties.
        let positional_count = if form.args.len() == 1 && !form.kwargs.is_empty() {
            // A single positional list plus keyword properties is still one
            // logical argument bundle for list-oriented entry forms.
            1
        } else {
            form.args.len()
        };
        // A positional list may be followed by keyword arguments belonging to
        // the same form. It remains a single logical slot; keyword values are
        // counted only when that property was not already supplied positionally.
        let keyword_count = form
            .kwargs
            .iter()
            .filter_map(|(k, _v)| {
                spec.props.iter().find(|p| p.name == *k).map(|p| {
                    if matches!(p.kind_mode, KindMode::Node { .. }) {
                        2
                    } else {
                        1
                    }
                })
            })
            .sum::<usize>();
        let positional_keyword_names = form
            .args
            .first()
            .and_then(|v| matches!(v, Value::List(_)).then_some(()));
        let bound = if positional_keyword_names.is_some() {
            // The list is the positional value; keyword properties are already
            // represented by the same form and must not inflate list-form
            // arity.
            positional_count
        } else {
            positional_count + keyword_count
        };
        let has_list_positional = form.args.iter().any(|v| matches!(v, Value::List(_)));
        let required_bound = if has_list_positional {
            required.saturating_sub(form.kwargs.len())
        } else {
            required
        };
        if bound < required_bound {
            return Err(SemError::Arity(format!(
                "{} expects {} args, got {}",
                form.name, required_bound, bound
            )));
        }
        if !open_ended && !has_list_positional && form.kwargs.len() > 0 {
            // Keyword-only optional tails do not increase positional arity.
        } else if !open_ended && !has_list_positional && bound > max_args {
            return Err(SemError::Arity(format!(
                "{} expects {} args, got {}",
                form.name, max_args, bound
            )));
        }
        // Unknown keyword names are an arity error.
        for (k, _) in &form.kwargs {
            if !spec.props.iter().any(|p| p.name == k) {
                return Err(SemError::Arity(format!(
                    "{} has no argument named `{k}`",
                    form.name
                )));
            }
        }
        Ok(())
    }

    /// Emit property triples. Positional args bind to props in order; a
    /// keyword arg (`name=value`) binds to the prop of that name and is
    /// removed from the positional queue (so `label=load, 0.0, 2.0` and
    /// `load, 0.0, 2.0` both work). A list prop consumes the remaining args;
    /// a single list-valued arg contributes its items.
    /// A KindNode prop consumes 2 positional args (kind, operand).
    fn compile_props(
        &mut self,
        subject: &NamedNode,
        form: &Form,
        spec: &FormSpec,
    ) -> Result<(), SemError> {
        let mut ai = 0usize;
        let positional: Vec<&Value> = form.args.iter().collect();
        for p in &spec.props {
            // Keyword binding takes precedence over the positional queue.
            if let Some(v) = form.kwargs.get(p.name) {
                if p.list || p.multi {
                    self.emit_list_prop(subject, p, v, spec)?;
                } else if matches!(p.kind_mode, KindMode::Node { .. }) {
                    // KindNode from kwarg: the operand comes from the next
                    // positional arg (or another kwarg).
                    let operand = form.kwargs.get("operand");
                    self.emit_kind_node(subject, p, v, operand, spec)?;
                } else {
                    self.emit_prop(subject, v, p, spec)?;
                }
                continue;
            }
            if p.pred.is_empty() {
                // Name argument of a Named-subject form. When supplied as a
                // keyword it is already consumed; otherwise consume the
                // positional name slot.
                if !form.kwargs.contains_key(p.name) && ai < positional.len() {
                    ai += 1;
                }
                continue;
            }
            if matches!(p.kind_mode, KindMode::Node { .. }) {
                // KindNode consumes 2 positional args (kind, operand).
                if ai >= positional.len() {
                    return Err(SemError::Arity(format!(
                        "{} expects kind arg for {}",
                        form.name, p.name
                    )));
                }
                let kind = positional[ai];
                ai += 1;
                let operand = if ai < positional.len() {
                    Some(positional[ai])
                } else {
                    None
                };
                if operand.is_some() {
                    ai += 1;
                }
                self.emit_kind_node(subject, p, kind, operand, spec)?;
                continue;
            }
            if p.list || p.multi {
                // List/multi prop: if the next positional is a List value,
                // consume ONLY that list (it belongs to this prop). Otherwise
                // consume all remaining positional args (legacy for modules
                // that express all list items positionally).
                let rest: Vec<&Value> = positional[ai..].to_vec();
                let (items, consumed) = match rest.as_slice() {
                    [Value::List(l_items), ..] => (l_items.clone(), 1),
                    other => (other.iter().map(|v| (*v).clone()).collect(), other.len()),
                };
                ai += consumed;
                self.emit_list_items(subject, p, &items, spec)?;
                continue;
            }
            if ai >= positional.len() {
                // No positional arg left for this prop. A kwarg-style form
                // (event/temporal) may simply omit an optional prop; a missing
                // required prop is an arity error, not a panic.
                if p.opt {
                    continue;
                }
                return Err(SemError::Arity(format!(
                    "{} is missing argument `{}`",
                    form.name, p.name
                )));
            }
            let arg = positional[ai];
            ai += 1;
            self.emit_prop(subject, arg, p, spec)?;
        }
        Ok(())
    }

    /// Emit a kind-object list item (memory `asks=[retrieve, …]`): the kind
    /// symbol becomes a constant node typed `{schema}{Enum}/{Variant}` (R10
    /// inverse), returned as the list term.
    fn emit_kind_object_term(
        &mut self,
        p: &Prop,
        v: &Value,
        spec: &FormSpec,
    ) -> Result<Term, SemError> {
        let Atom::Sym(kind) = self.expect_atom(v, ArgType::Kind)? else {
            unreachable!()
        };
        let allowed = spec.kinds.get(p.name);
        if !allowed.is_some_and(|ks| ks.contains(&kind.as_str())) {
            let known = allowed.map(|ks| ks.join("|")).unwrap_or_default();
            return Err(SemError::BadValue(format!(
                "{} expects {}|…, got \"{kind}\"",
                p.name, known
            )));
        }
        let KindMode::Object {
            data_prefix,
            enum_name,
            typed,
            lower_variant,
        } = p.kind_mode
        else {
            return Err(SemError::TypeMismatch(format!(
                "{} list items must be kind objects",
                p.name
            )));
        };
        let node = self.kind_object_node(data_prefix, enum_name, &kind)?;
        if typed {
            let variant = if lower_variant {
                kind.to_lowercase()
            } else {
                kind_pascal(&kind)
            };
            let class = format!("{}:{}/{}", self.g.schema_prefix, enum_name, variant);
            self.emit_type(&node, &class)?;
        }
        Ok(Term::from(node))
    }

    /// Emit a KindNode prop: op node typed `{schema}/{Variant}` with the operand
    /// literal on `operand_pred`. Consumed from the op kind + next positional arg.
    fn emit_kind_node(
        &mut self,
        subject: &NamedNode,
        p: &Prop,
        kind: &Value,
        operand: Option<&Value>,
        spec: &FormSpec,
    ) -> Result<(), SemError> {
        let Atom::Sym(kind_name) = self.expect_atom(kind, ArgType::Kind)? else {
            unreachable!()
        };
        let allowed = spec.kinds.get(p.name);
        if !allowed.is_some_and(|ks| ks.contains(&kind_name.as_str())) {
            let known = allowed.map(|ks| ks.join("|")).unwrap_or_default();
            return Err(SemError::BadValue(format!(
                "{} expects {}|…, got \"{kind_name}\"",
                p.name, known
            )));
        }
        match p.kind_mode {
            KindMode::Node {
                schema_class,
                operand_pred,
            } => {
                let node = self.generated("nop")?;
                self.emit_type(&node, &format!("{schema_class}/{}", kind_pascal(kind_name)))?;
                if let Some(op_val) = operand {
                    let Atom::Num(n) = self.expect_atom(op_val, ArgType::Num)? else {
                        unreachable!()
                    };
                    self.emit_literal(&node, operand_pred, number_literal(*n))?;
                }
                self.emit_object(subject, p.pred, &node)?;
            }
            _ => unreachable!("emit_kind_node called for non-Node kind_mode"),
        }
        Ok(())
    }

    /// A list/multi prop given as a single list value or a set of items.
    fn emit_list_prop(
        &mut self,
        subject: &NamedNode,
        p: &Prop,
        v: &Value,
        spec: &FormSpec,
    ) -> Result<(), SemError> {
        let items: Vec<Value> = match v {
            Value::List(items) => items.clone(),
            other => vec![other.clone()],
        };
        self.emit_list_items(subject, p, &items, spec)
    }

    /// Emit a list (rdf:List) or multi (repeated triple) prop from items.
    fn emit_list_items(
        &mut self,
        subject: &NamedNode,
        p: &Prop,
        items: &[Value],
        spec: &FormSpec,
    ) -> Result<(), SemError> {
        if p.list {
            let terms: Vec<Term> = match p.ty {
                ArgType::Expr => self.compile_expr_items(items)?,
                ArgType::Str => items
                    .iter()
                    .map(|v| {
                        let Atom::Str(s) = self.expect_atom(v, ArgType::Str)? else {
                            unreachable!()
                        };
                        Ok(Term::from(Literal::new_simple_literal(s)))
                    })
                    .collect::<Result<Vec<_>, SemError>>()?,
                ArgType::Kind => items
                    .iter()
                    .map(|v| match v {
                        Value::Form(f) => {
                            let nested = self
                                .g
                                .expr(&f.name)
                                .or_else(|| self.g.form(&f.name))
                                .ok_or_else(|| SemError::UnknownFunction(f.name.clone()))?;
                            let node = self.subject_for(f, nested)?;
                            self.emit_type(&node, nested.class)?;
                            self.check_arity(f, nested)?;
                            self.compile_props(&node, f, nested)?;
                            Ok(Term::from(node))
                        }
                        _ => self.emit_kind_object_term(p, v, spec),
                    })
                    .collect::<Result<Vec<_>, SemError>>()?,
                ArgType::Num => items
                    .iter()
                    .map(|v| {
                        // Nested list item (info channel transition rows): a
                        // `[[…], […]]` matrix renders as nested rdf:Lists.
                        if let Value::List(row) = v {
                            let row_terms: Vec<Term> = row
                                .iter()
                                .map(|rv| {
                                    let Atom::Num(n) = self.expect_atom(rv, ArgType::Num)? else {
                                        unreachable!()
                                    };
                                    Ok(Term::from(Literal::new_typed_literal(
                                        format!("{n}"),
                                        NamedNode::new(xsd::DOUBLE.as_str()).unwrap(),
                                    )))
                                })
                                .collect::<Result<Vec<_>, SemError>>()?;
                            self.build_rdf_list(&row_terms).map(Term::from)
                        } else {
                            let Atom::Num(n) = self.expect_atom(v, ArgType::Num)? else {
                                unreachable!()
                            };
                            Ok(Term::from(number_literal(*n)))
                        }
                    })
                    .collect::<Result<Vec<_>, SemError>>()?,
                _ => {
                    return Err(SemError::TypeMismatch(format!(
                        "list prop {} has unsupported element type {:?}",
                        p.name, p.ty
                    )))
                }
            };
            let head = self.build_rdf_list(&terms)?;
            self.emit_object(subject, p.pred, &head)?;
        } else {
            // multi: one triple per value.
            for v in items {
                self.emit_prop(subject, v, p, spec)?;
            }
        }
        Ok(())
    }

    /// R7: the subject of a form.
    fn subject_for(&mut self, form: &Form, spec: &FormSpec) -> Result<NamedNode, SemError> {
        match spec.subj {
            SubjKind::Named => {
                // The name is normally the first positional argument, but
                // Named forms also accept `name=...` keyword syntax.
                let name_value = form.kwargs.get("name").or_else(|| form.args.first());
                let Some(Value::Atom(Atom::Sym(name))) = name_value else {
                    return Err(SemError::TypeMismatch(format!(
                        "{}: first argument must be a name",
                        form.name
                    )));
                };
                let iri = self
                    .declared
                    .get(name)
                    .cloned()
                    .unwrap_or(self.resolve_subject(name)?.as_str().to_string());
                Ok(NamedNode::new(iri).map_err(|e| SemError::Lexical(e.to_string()))?)
            }
            SubjKind::Generated(prefix) => {
                let n = self.next_counter(prefix);
                let iri = self.resolve_subject(&format!("{}{prefix}{n}", self.seed))?;
                Ok(iri)
            }
        }
    }

    /// Compile one property argument. `owner` is the form/expression spec the
    /// prop belongs to (needed for kind sets, R10).
    fn emit_prop(
        &mut self,
        subject: &NamedNode,
        arg: &Value,
        prop: &Prop,
        owner: &FormSpec,
    ) -> Result<(), SemError> {
        // The name argument of a Named-subject form is the subject itself
        // (R7) — nothing to emit.
        if prop.pred.is_empty() {
            return Ok(());
        }
        match prop.ty {
            ArgType::Num => {
                let lit = numeric_literal(arg)?;
                self.emit_literal(subject, prop.pred, lit)?;
            }
            ArgType::Str => {
                let Atom::Str(s) = self.expect_atom(arg, ArgType::Str)? else {
                    unreachable!()
                };
                let lit = Literal::new_simple_literal(s);
                self.emit_literal(subject, prop.pred, lit)?;
            }
            ArgType::Bool => {
                let Atom::Bool(b) = self.expect_atom(arg, ArgType::Bool)? else {
                    unreachable!()
                };
                let lit = Literal::new_typed_literal(
                    b.to_string(),
                    NamedNode::new(xsd::BOOLEAN.as_str()).unwrap(),
                );
                self.emit_literal(subject, prop.pred, lit)?;
            }
            ArgType::Sym => {
                // A bare name reference (e.g. a declared variable elsewhere).
                let Atom::Sym(name) = self.expect_atom(arg, ArgType::Sym)? else {
                    unreachable!()
                };
                let node = self.named_node(name)?;
                self.emit_object(subject, prop.pred, &node)?;
            }
            ArgType::Kind => {
                let Atom::Sym(kind) = self.expect_atom(arg, ArgType::Kind)? else {
                    unreachable!()
                };
                let allowed = owner.kinds.get(prop.name);
                if !allowed.is_some_and(|ks| ks.contains(&kind.as_str())) {
                    let known = allowed.map(|ks| ks.join("|")).unwrap_or_default();
                    return Err(SemError::BadValue(format!(
                        "{} expects {}|…, got \"{kind}\"",
                        prop.name, known
                    )));
                }
                match prop.kind_mode {
                    KindMode::Literal => {
                        let lit = Literal::new_simple_literal(kind);
                        self.emit_literal(subject, prop.pred, lit)?;
                    }
                    KindMode::Object {
                        data_prefix,
                        enum_name,
                        typed,
                        lower_variant,
                    } => {
                        // Constant NamedNode: `{data}:{enum}/{variant}`.
                        let node = self.kind_object_node(data_prefix, enum_name, &kind)?;
                        if typed {
                            // `rdf:type {schema}{enum}/{Variant}` (PascalCase,
                            // or lowercase for model-layer enums).
                            let variant = if lower_variant {
                                kind.to_lowercase()
                            } else {
                                kind_pascal(&kind)
                            };
                            let class =
                                format!("{}:{}/{}", self.g.schema_prefix, enum_name, variant);
                            self.emit_type(&node, &class)?;
                        }
                        self.emit_object(subject, prop.pred, &node)?;
                    }
                    KindMode::Node { .. } => {
                        // KindNode is consumed in compile_props, before emit_prop.
                        unreachable!("KindNode should be consumed in compile_props")
                    }
                }
            }
            ArgType::Expr => {
                // R8: expression position — inline a constant, a bare symbol
                // reference, or a nested expression form.
                if prop.list {
                    let items = match arg {
                        Value::List(items) => items.clone(),
                        other => vec![other.clone()],
                    };
                    let terms = self.compile_expr_items(&items)?;
                    let head = self.build_rdf_list(&terms)?;
                    self.emit_object(subject, prop.pred, &head)?;
                } else {
                    let term = self.compile_expr(arg)?;
                    match term {
                        Term::NamedNode(n) => {
                            self.emit_object(subject, prop.pred, &n)?;
                        }
                        _ => {
                            return Err(SemError::TypeMismatch(format!(
                                "expression position must lower to a node, got {term:?}"
                            )))
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Compile a list of expression items → terms (e.g. `model:operands`).
    fn compile_expr_items(&mut self, items: &[Value]) -> Result<Vec<Term>, SemError> {
        items.iter().map(|v| self.compile_expr(v)).collect()
    }

    /// Compile one expression value to a term (R4, R5, R8).
    fn compile_expr(&mut self, value: &Value) -> Result<Term, SemError> {
        match value {
            Value::Atom(Atom::Num(n)) => {
                // R5: implicit constant.
                let node = self.generated("c")?;
                self.emit_type(&node, "model:Constant")?;
                self.emit_literal(&node, "model:value", number_literal(*n))?;
                Ok(Term::from(node))
            }
            Value::Atom(Atom::Fraction(numerator, denominator)) => {
                let (numerator, denominator) = normalize_fraction(*numerator, *denominator)?;
                let node = self.generated("c")?;
                self.emit_type(&node, "model:Constant")?;
                self.emit_literal(
                    &node,
                    "model:value",
                    Literal::new_typed_literal(
                        format!("{numerator}/{denominator}"),
                        NamedNode::new("https://example.org/ns/model#Rational").unwrap(),
                    ),
                )?;
                Ok(Term::from(node))
            }
            Value::Atom(Atom::Sym(name)) => {
                // R4/R6: declared parameter/variable, or implicit variable.
                if let Some(iri) = self.declared.get(name) {
                    let node = NamedNode::new(iri.clone())
                        .map_err(|e| SemError::Lexical(e.to_string()))?;
                    return Ok(Term::from(node));
                }
                let node = self.named_node(name)?;
                self.emit_type(&node, "model:Variable")?;
                self.declared
                    .insert(name.clone(), node.as_str().to_string());
                Ok(Term::from(node))
            }
            Value::Atom(Atom::Bool(b)) => {
                // A bare boolean in expression position (e.g. the guarded
                // always-true outcome step `guard=true`) lowers to a typed
                // constant the renderer reproduces as a bare `true`/`false`.
                let node = self.generated("c")?;
                self.emit_type(&node, "model:Constant")?;
                self.emit_literal(
                    &node,
                    "model:value",
                    Literal::new_typed_literal(
                        b.to_string(),
                        NamedNode::new(xsd::BOOLEAN.as_str()).unwrap(),
                    ),
                )?;
                Ok(Term::from(node))
            }
            Value::Atom(Atom::Var(name)) => {
                // `?name` — an explicit variable reference (logic SEM §4.1).
                // In the logic modules it lowers to a `logic:Variable` node so
                // canonical SEM text round-trips the `?`. In other modules the
                // `?` is not part of the vocabulary — behave like a bare
                // symbol reference (R4/R6).
                if self.g.dynamic_predicates {
                    let node = self.generated("v")?;
                    self.emit_type(&node, "logic:Variable")?;
                    self.emit_literal(
                        &node,
                        "logic:name",
                        Literal::new_simple_literal(name.clone()),
                    )?;
                    Ok(Term::from(node))
                } else {
                    if let Some(iri) = self.declared.get(name) {
                        let node = NamedNode::new(iri.clone())
                            .map_err(|e| SemError::Lexical(e.to_string()))?;
                        return Ok(Term::from(node));
                    }
                    let node = self.named_node(name)?;
                    self.emit_type(&node, "model:Variable")?;
                    self.declared
                        .insert(name.clone(), node.as_str().to_string());
                    Ok(Term::from(node))
                }
            }
            Value::Form(form) => {
                // R8: inline a nested expression or content form.
                // Check forms first (event `fluent`, temporal `interval`), then exprs.
                if let Some(spec) = self.g.form(&form.name) {
                    let node = self.subject_for(form, spec)?;
                    self.emit_type(&node, spec.class)?;
                    self.maybe_emit_compare_op(&node, form, spec)?;
                    self.check_arity(form, spec)?;
                    self.compile_props(&node, form, spec)?;
                    return Ok(Term::from(node));
                }
                // Dynamic predicate application (logic modules, design §4.5):
                // an unregistered nested form name is a predicate node —
                // `fact(took(alice))` compiles `took(alice)` to a logic:Atom
                // with ordered arguments. Only when the module opts in;
                // otherwise the unknown name remains a sem:e2 error.
                if self.g.dynamic_predicates {
                    let node = self.generated("p")?;
                    self.emit_type(&node, "logic:Atom")?;
                    self.emit_literal(
                        &node,
                        "logic:predicate",
                        Literal::new_simple_literal(form.name.clone()),
                    )?;
                    let mut ordered: Vec<Term> = Vec::new();
                    for arg in &form.args {
                        ordered.push(self.compile_expr(arg)?);
                    }
                    if ordered.len() <= 1 {
                        if let Some(t) = ordered.into_iter().next() {
                            self.emit_object_term(&node, "logic:arguments", &t)?;
                        }
                    } else {
                        let list = self.build_rdf_list(&ordered)?;
                        self.emit_object(&node, "logic:arguments", &list)?;
                    }
                    if !form.kwargs.is_empty() {
                        // Keywords on a predicate: append as `name=value`
                        // pairs on the arguments list (deterministic order).
                        let mut kw_terms: Vec<Term> = Vec::new();
                        for (k, v) in &form.kwargs {
                            kw_terms.push(Term::from(Literal::new_simple_literal(k.clone())));
                            kw_terms.push(self.compile_expr(v)?);
                        }
                        let head = self.build_rdf_list(&kw_terms)?;
                        self.emit_object(&node, "logic:kwarguments", &head)?;
                    }
                    return Ok(Term::from(node));
                }
                let spec = self.g.expr(&form.name).ok_or_else(|| {
                    SemError::UnknownFunction(format!(
                        "module \"{}\" has no expression \"{}\"",
                        self.g.name, form.name
                    ))
                })?;
                let node = match spec.subj {
                    SubjKind::Generated(prefix) => self.generated(prefix)?,
                    SubjKind::Named => self.subject_for(form, spec)?,
                };
                self.emit_type(&node, spec.class)?;
                self.check_arity(form, spec)?;
                self.compile_props(&node, form, spec)?;
                Ok(Term::from(node))
            }
            Value::List(items) => {
                let terms = self.compile_expr_items(items)?;
                let node = self.generated("list")?;
                self.emit_type(&node, "model:List")?;
                let head = self.build_rdf_list(&terms)?;
                self.emit_object(&node, "model:items", &head)?;
                Ok(Term::from(node))
            }
            other => Err(SemError::TypeMismatch(format!(
                "expression position got {other:?}"
            ))),
        }
    }

    /// A generated subject (`ex:<seed><prefix><n>`), R7.
    fn generated(&mut self, prefix: &'static str) -> Result<NamedNode, SemError> {
        let n = self.next_counter(prefix);
        let node = self.resolve_subject(&format!("{}{prefix}{n}", self.seed))?;
        Ok(node)
    }

    /// The six comparison forms share the `logic:Comparison` class; record the
    /// operator (`eq`…`ge`) as a `logic:op` literal so the renderer can
    /// recover the form name lost in the shared-class RDF.
    fn maybe_emit_compare_op(
        &mut self,
        subject: &NamedNode,
        form: &Form,
        spec: &FormSpec,
    ) -> Result<(), SemError> {
        if spec.class == "logic:Comparison" {
            self.emit_literal(
                subject,
                "logic:op",
                Literal::new_simple_literal(form.name.clone()),
            )?;
        }
        Ok(())
    }

    /// A kind-object constant node: `{data_prefix}:{enum_lower}/{variant_lower}`
    /// (R10 inverse — the engine parses the variant from the constant IRI).
    fn kind_object_node(
        &self,
        data_prefix: &str,
        enum_name: &str,
        variant: &str,
    ) -> Result<NamedNode, SemError> {
        let iri = self
            .prefixes
            .resolve(&format!(
                "{data_prefix}:{}/{}",
                enum_name.to_lowercase(),
                variant.to_lowercase()
            ))
            .ok_or_else(|| SemError::Lexical(format!("unknown data prefix `{data_prefix}`")))?;
        NamedNode::new(iri).map_err(|e| SemError::Lexical(e.to_string()))
    }

    /// Allocate the next identifier in one semantic-prefix sequence (R7).
    /// Keeping this as the only allocation path makes nested emission
    /// deterministic and prevents unrelated prefixes from colliding.
    fn next_counter(&mut self, prefix: &'static str) -> usize {
        let counter = self.counters.entry(prefix).or_insert(0);
        let value = *counter;
        *counter += 1;
        value
    }

    /// A named subject in the data namespace (`ex:<name>`).
    fn named_node(&self, name: &str) -> Result<NamedNode, SemError> {
        self.resolve_subject(name)
    }

    fn resolve_subject(&self, local: &str) -> Result<NamedNode, SemError> {
        let iri = self
            .prefixes
            .resolve(&format!("ex:{local}"))
            .ok_or_else(|| SemError::Lexical("data prefix ex: is missing".into()))?;
        NamedNode::new(iri).map_err(|e| SemError::Lexical(e.to_string()))
    }

    fn resolve_pred(&self, prefixed: &str) -> Result<NamedNode, SemError> {
        let iri = self.prefixes.resolve(prefixed).ok_or_else(|| {
            SemError::Lexical(format!("unknown predicate prefix in `{prefixed}`"))
        })?;
        NamedNode::new(iri).map_err(|e| SemError::Lexical(e.to_string()))
    }

    fn root_iri(&self) -> Result<NamedNode, SemError> {
        self.resolve_subject("M")
    }

    fn emit_type(&mut self, node: &NamedNode, class: &str) -> Result<(), SemError> {
        let class_iri = self.resolve_pred(class)?;
        self.graph.insert(&Triple::new(
            node.clone(),
            rdf::TYPE.into_owned(),
            Term::from(class_iri),
        ));
        Ok(())
    }

    fn emit_literal(&mut self, node: &NamedNode, pred: &str, lit: Literal) -> Result<(), SemError> {
        let p = self.resolve_pred(pred)?;
        self.graph
            .insert(&Triple::new(node.clone(), p, Term::from(lit)));
        Ok(())
    }

    fn emit_object(
        &mut self,
        node: &NamedNode,
        pred: &str,
        obj: &NamedNode,
    ) -> Result<(), SemError> {
        let p = self.resolve_pred(pred)?;
        self.graph
            .insert(&Triple::new(node.clone(), p, Term::from(obj.clone())));
        Ok(())
    }

    /// Emit `node pred term` where the object may be a node *or* a literal
    /// (used for predicate arguments that are bare numbers/strings).
    fn emit_object_term(
        &mut self,
        node: &NamedNode,
        pred: &str,
        obj: &Term,
    ) -> Result<(), SemError> {
        let p = self.resolve_pred(pred)?;
        self.graph
            .insert(&Triple::new(node.clone(), p, obj.clone()));
        Ok(())
    }

    fn expect_atom<'v>(&self, arg: &'v Value, want: ArgType) -> Result<&'v Atom, SemError> {
        let Value::Atom(a) = arg else {
            return Err(SemError::TypeMismatch(format!(
                "expected {want:?} atom, got {arg:?}"
            )));
        };
        let matches = match (want, a) {
            (ArgType::Num, Atom::Num(_) | Atom::Fraction(_, _)) => true,
            (ArgType::Str, Atom::Str(_)) => true,
            (ArgType::Bool, Atom::Bool(_)) => true,
            (ArgType::Sym, Atom::Sym(_)) => true,
            (ArgType::Kind, Atom::Sym(_)) => true,
            _ => false,
        };
        if !matches {
            return Err(SemError::TypeMismatch(format!(
                "expected {want:?}, got {a:?}"
            )));
        }
        Ok(a)
    }

    /// Compile the entry-level literal props (logic/probabilistic rules).
    fn compile_entry_props(&mut self, root: &NamedNode, form: &Form) -> Result<(), SemError> {
        let entry_spec = FormSpec {
            class: self.g.root_class,
            props: self.g.entry_props.clone(),
            kinds: self.g.kinds.clone(),
            subj: SubjKind::Generated(""),
        };
        self.check_arity(form, &entry_spec)?;
        self.compile_props(root, form, &entry_spec)?;
        Ok(())
    }

    /// Build an `rdf:List` chain of terms, returning the head term.
    fn build_rdf_list(&mut self, items: &[Term]) -> Result<NamedNode, SemError> {
        if items.is_empty() {
            return Ok(
                NamedNode::new(rdf::NIL.as_str()).map_err(|e| SemError::Lexical(e.to_string()))?
            );
        }
        let head = self.generated("l")?;
        let mut cell = head.clone();
        for (i, item) in items.iter().enumerate() {
            self.graph.insert(&Triple::new(
                cell.clone(),
                rdf::FIRST.into_owned(),
                item.clone(),
            ));
            if i + 1 < items.len() {
                let next = self.generated("l")?;
                self.graph.insert(&Triple::new(
                    cell.clone(),
                    rdf::REST.into_owned(),
                    Term::from(next.clone()),
                ));
                cell = next;
            }
        }
        self.graph.insert(&Triple::new(
            cell,
            rdf::REST.into_owned(),
            Term::from(
                NamedNode::new(rdf::NIL.as_str()).map_err(|e| SemError::Lexical(e.to_string()))?,
            ),
        ));
        Ok(head)
    }
}

/// `before` → `Before`, `metby` → `MetBy` (the engine's PascalCase variant
/// names for kind-object classes).
fn kind_pascal(variant: &str) -> String {
    let mut chars = variant.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let mut s = first.to_uppercase().collect::<String>();
            s.push_str(chars.as_str());
            // Engine variant names that split the single kind token into
            // PascalCase/camelCase words — the strict decoders spell these
            // exactly (learning algorithms, evolution asks, Allen relations,
            // mechanism asks, …).
            let lower = s.to_lowercase();
            match lower.as_str() {
                "metby" => "MetBy".to_string(),
                "overlappedby" => "OverlappedBy".to_string(),
                "startedby" => "StartedBy".to_string(),
                "finishedby" => "FinishedBy".to_string(),
                "incentivecompatibility" => "IncentiveCompatibility".to_string(),
                "individualrationality" => "IndividualRationality".to_string(),
                "qlearning" => "QLearning".to_string(),
                "fixedpoints" => "FixedPoints".to_string(),
                "valueiteration" => "ValueIteration".to_string(),
                "replicatorstep" => "ReplicatorStep".to_string(),
                "replicatormutatorstep" => "ReplicatorMutatorStep".to_string(),
                _ => s,
            }
        }
    }
}

/// A numeric literal: integers as `xsd:integer`, floats as `xsd:double`
/// (mirroring the cRDF codec's rule so goldens line up).
fn numeric_literal(value: &Value) -> Result<Literal, SemError> {
    match value {
        Value::Atom(Atom::Num(n)) => Ok(number_literal(*n)),
        Value::Atom(Atom::Fraction(numerator, denominator)) => {
            let (numerator, denominator) = normalize_fraction(*numerator, *denominator)?;
            Ok(Literal::new_typed_literal(
                format!("{numerator}/{denominator}"),
                NamedNode::new("https://example.org/ns/model#Rational").unwrap(),
            ))
        }
        other => Err(SemError::TypeMismatch(format!(
            "expected numeric value, got {other:?}"
        ))),
    }
}

fn normalize_fraction(numerator: i128, denominator: i128) -> Result<(i128, i128), SemError> {
    if denominator == 0 {
        return Err(SemError::BadValue(
            "fraction denominator must not be zero".into(),
        ));
    }
    let gcd = gcd_i128(numerator, denominator);
    let mut numerator = numerator / gcd;
    let mut denominator = denominator / gcd;
    if denominator < 0 {
        numerator = -numerator;
        denominator = -denominator;
    }
    Ok((numerator, denominator))
}

fn gcd_i128(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    if a == 0 {
        1
    } else {
        a
    }
}

pub fn number_literal(n: f64) -> Literal {
    let dt = if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        NamedNode::new(xsd::INTEGER.as_str()).unwrap()
    } else {
        NamedNode::new(xsd::DOUBLE.as_str()).unwrap()
    };
    let text = if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        format!("{}", n as i64)
    } else {
        ryu::Buffer::new().format(n).to_string()
    };
    Literal::new_typed_literal(text, dt)
}
