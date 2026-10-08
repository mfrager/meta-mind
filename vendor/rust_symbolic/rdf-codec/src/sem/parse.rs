//! SEM parser (§3.2 EBNF of `planning/s_expr/s_expr_design2.md`): turns the
//! token stream from [`lex`] into a typed, module-aware AST, using **chumsky**
//! — a recursive-descent parser combinator library. The grammar is declared
//! as combinators (§3.2), scope rules (R2) are enforced by a deterministic
//! token pre-pass, and every failure maps to a stable [`SemError`] code.

use chumsky::extra;
use chumsky::input::{Stream, ValueInput};
use chumsky::prelude::*;
use chumsky::span::SimpleSpan;

use crate::sem::ast::{Atom, Document, Form, Value};
use crate::sem::errors::SemError;
use crate::sem::lex::{lex, Spanned, Token};

/// Parse a SEM document string into a typed AST.
pub fn parse(input: &str) -> Result<Document, SemError> {
    let toks = lex(input)?;
    check_scope(&toks)?;
    parse_tokens(&toks, input.len())
}

/// R2 scope pre-pass (deterministic, exact error codes):
/// - top form must be qualified `<module>.<form>` (else `sem:e7`);
/// - no qualified name may appear below the top level (else `sem:e7`);
/// - the document must be exactly one top form (else `sem:e7`).
fn check_scope(toks: &[Spanned]) -> Result<(), SemError> {
    if toks.is_empty() {
        return Err(SemError::Lexical("empty document".into()));
    }
    // 1. `<module>.<form>` — the first two tokens must be ident, dot.
    match (
        toks.first().map(|s| &s.token),
        toks.get(1).map(|s| &s.token),
    ) {
        (Some(Token::Ident(_)), Some(Token::Dot)) => {}
        _ => {
            return Err(SemError::BadScope(
                "unqualified top-level name: expected <module>.<form>".into(),
            ))
        }
    }
    // 2. No `ident . ident` below the top level: any Dot at nesting > 0.
    let mut depth = 0usize;
    for s in &toks[2..] {
        match s.token {
            Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => depth = depth.saturating_sub(1),
            Token::Dot if depth > 0 => {
                return Err(SemError::BadScope(format!(
                    "qualified name not allowed at nesting level > 0 (offset {})",
                    s.offset
                )))
            }
            _ => {}
        }
    }
    // 3. Exactly one top form: the top-level paren closes at depth 0; nothing
    //    may follow it.
    let mut depth = 0usize;
    let mut closed_at: Option<usize> = None;
    for (i, s) in toks.iter().enumerate().skip(2) {
        match s.token {
            Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => {
                if depth == 0 {
                    closed_at = Some(i);
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    closed_at = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    if let Some(i) = closed_at {
        if i + 1 < toks.len() {
            return Err(SemError::BadScope(format!(
                "document must be exactly one top form, found extra tokens at offset {}",
                toks[i + 1].offset
            )));
        }
    }
    Ok(())
}

/// Parse the (scope-validated) token stream with the chumsky grammar.
fn parse_tokens(toks: &[Spanned], len: usize) -> Result<Document, SemError> {
    let stream = Stream::from_iter(
        toks.iter()
            .map(|s| (s.token.clone(), (s.offset..s.offset).into())),
    )
    .map((0..len).into(), |(t, s): (_, _)| (t, s));
    match sem_parser().parse(stream).into_result() {
        Ok(doc) => Ok(doc),
        Err(errs) => {
            let first = errs
                .into_iter()
                .next()
                .unwrap_or_else(|| Simple::new(None, SimpleSpan::new((), 0..0)));
            let range = first.span().clone().into_range();
            Err(SemError::Lexical(format!(
                "parse error at {range:?}: {first}"
            )))
        }
    }
}

/// The SEM grammar (§3.2), declared as chumsky combinators:
/// `document ::= topform`, `topform ::= qname "(" [values] ")"`,
/// `form ::= ident "(" [values] ")"`, `value ::= atom | form | list`.
fn sem_parser<'src, I>() -> impl Parser<'src, I, Document, extra::Err<Simple<'src, Token>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    let ident = select! { Token::Ident(name) => name };
    let number = select! { Token::Number(n) => n };
    let string = select! { Token::String(s) => s };
    let boolean = select! { Token::Bool(b) => b };
    let var = select! { Token::Var(name) => name };
    let slash = just(Token::Slash);
    let fraction = ident
        .clone()
        .then_ignore(just(Token::LParen))
        .then(number.clone())
        .then_ignore(just(Token::Comma))
        .then(number.clone())
        .then_ignore(just(Token::RParen))
        .try_map(|((name, numerator), denominator), span| {
            if name != "fraction" {
                return Err(Simple::new(None, span));
            }
            if numerator.fract() != 0.0 || denominator.fract() != 0.0 {
                return Err(Simple::new(None, span));
            }
            if denominator == 0.0 {
                return Err(Simple::new(None, span));
            }
            Ok(Value::Atom(Atom::Fraction(
                numerator as i128,
                denominator as i128,
            )))
        });
    // URI annotation following a value core (`eq[ex:eq1](`, `2[ex:c1]`,
    // `?x[ex:v1]`; `planning/updates/sem_uri_annotation_design.md` §5).
    // Annotations are output-only: accepted on input, discarded here — the
    // AST and compiler are untouched, so annotated text parses and compiles
    // identically to the bare text.
    let ann = select! { Token::Annotation(_) => () };

    let value = recursive(|value| {
        let atom = choice((
            fraction.clone(),
            number.map(|n| Value::Atom(Atom::Num(n))),
            boolean.map(|b| Value::Atom(Atom::Bool(b))),
            string.map(|s| Value::Atom(Atom::Str(s))),
            var.clone().map(|name| Value::Atom(Atom::Var(name))),
            ident.clone().map(|name| Value::Atom(Atom::Sym(name))),
        ))
        // Optional trailing URI annotation — consumed and ignored.
        .then(ann.clone().or_not())
        .map(|(v, _)| v);
        let list_item = choice((
            ident
                .clone()
                .then_ignore(slash.clone())
                .then(number.clone())
                .then(ann.clone().or_not())
                .map(|((name, _), _)| Value::Atom(Atom::Sym(name))),
            value.clone(),
        ));
        let list = list_item
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .map(Value::List);
        // A form argument: either `name=value` (keyword), or a plain value.
        let arity_atom = ident
            .clone()
            .then_ignore(slash.clone())
            .then(number.clone())
            .then(ann.clone().or_not())
            .map(|((name, _), _)| Value::Atom(Atom::Sym(name)));
        let value_with_arity = choice((arity_atom, value.clone()));
        let kw = value_with_arity.clone().map(|v| (None, v));
        let kw_named = ident
            .clone()
            .then_ignore(just(Token::Equals))
            .then(value_with_arity.clone())
            .map(|(name, v)| (Some(name), v));
        let kwarg = kw_named.or(kw);
        let form = ident
            .clone()
            // Optional annotation between the form name and its parens:
            // `eq[ex:eq1](…)`.
            .then(ann.clone().or_not())
            .then_ignore(just(Token::LParen))
            .then(
                kwarg
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::RParen))
            // Tolerate a stray annotation after a form's closing paren
            // (`eq(...)[ex:eq1]`) — misshapen pasted input still parses;
            // output always emits the fragment between name and parens.
            .then(ann.clone().or_not())
            .map(|(((name, _pre), args), _post)| {
                let mut positional = Vec::new();
                let mut kwargs = std::collections::BTreeMap::new();
                for (k, v) in args {
                    if let Some(k) = k {
                        kwargs.insert(k, v);
                    } else {
                        positional.push(v);
                    }
                }
                let form = Form {
                    name,
                    args: positional,
                    kwargs,
                };
                // `fraction(n, d)` is an exact-rational literal, not a
                // function application: normalize it to `Atom::Fraction` so
                // it compiles to a `model#Rational` value rather than being
                // mistaken for a form (the generic `form` combinator below
                // shadows the dedicated `fraction` atom production, so this
                // is the reachable path).
                if form.name == "fraction" && form.kwargs.is_empty() && form.args.len() == 2 {
                    if let (
                        Value::Atom(Atom::Num(numerator)),
                        Value::Atom(Atom::Num(denominator)),
                    ) = (&form.args[0], &form.args[1])
                    {
                        if numerator.fract() == 0.0
                            && denominator.fract() == 0.0
                            && *denominator != 0.0
                        {
                            return Value::Atom(Atom::Fraction(
                                *numerator as i128,
                                *denominator as i128,
                            ));
                        }
                    }
                }
                Value::Form(form)
            });
        choice((form, list, atom))
    });

    // Top-level kwarg combinator (clone of the kwarg pattern above).
    let kw_top = value.clone().map(|v| (None, v));
    let kw_top_named = ident
        .clone()
        .then_ignore(just(Token::Equals))
        .then(value)
        .map(|(name, v)| (Some(name), v));
    let kwarg_top = kw_top_named.or(kw_top);
    ident
        .then_ignore(just(Token::Dot))
        .then(ident)
        // Optional annotation on the top form: `numerical.model[ex:m](…)`.
        // Whitespace between the value and the bracket is allowed because
        // the lexer skips it.
        .then(ann.or_not())
        .then_ignore(just(Token::LParen))
        .then(
            kwarg_top
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::RParen))
        .map(|(((module, entry), _ann), args)| {
            let mut positional = Vec::new();
            let mut kwargs = std::collections::BTreeMap::new();
            for (k, v) in args {
                if let Some(k) = k {
                    kwargs.insert(k, v);
                } else {
                    positional.push(v);
                }
            }
            Document {
                module,
                top: Form {
                    name: entry,
                    args: positional,
                    kwargs,
                },
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sem::ast::Form;

    #[test]
    fn parses_simple_document() {
        let d = parse("numerical.equation(sum(x, 2), 7)").unwrap();
        assert_eq!(d.module, "numerical");
        assert_eq!(d.top.name, "equation");
        assert_eq!(d.top.args.len(), 2);
        match &d.top.args[0] {
            Value::Form(Form { name, args, .. }) => {
                assert_eq!(name, "sum");
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected nested form, got {other:?}"),
        }
    }

    #[test]
    fn parses_lists_and_strings() {
        let d = parse("logic.model([\"rain.\", \"wet :- rain.\"])").unwrap();
        match &d.top.args[0] {
            Value::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(&items[0], Value::Atom(Atom::Str(_))));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn parses_variable_atoms() {
        let d = parse("logic.model(fact(took(?who)))").unwrap();
        match &d.top.args[0] {
            Value::Form(f) => match &f.args[0] {
                Value::Form(atom) => match &atom.args[0] {
                    Value::Atom(Atom::Var(name)) => assert_eq!(name, "who"),
                    other => panic!("expected variable atom, got {other:?}"),
                },
                other => panic!("expected atom form, got {other:?}"),
            },
            other => panic!("expected form, got {other:?}"),
        }
    }

    #[test]
    fn trailing_comma_accepted() {
        let d = parse("numerical.equation(sum(x, 2), 7,)").unwrap();
        assert_eq!(d.top.args.len(), 2);
    }

    #[test]
    fn rejects_extra_tokens() {
        let err = parse("a.b() c.d()").unwrap_err();
        assert_eq!(err.code(), "sem:e7");
    }

    #[test]
    fn rejects_missing_module() {
        let err = parse("model()").unwrap_err();
        assert_eq!(err.code(), "sem:e7");
    }

    #[test]
    fn rejects_qualified_nested() {
        let err = parse("numerical.model(a.b())").unwrap_err();
        assert_eq!(err.code(), "sem:e7");
    }

    #[test]
    fn rejects_lexical_garbage() {
        let err = parse("numerical.model(@)").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
    }

    #[test]
    fn empty_document_is_lexical() {
        let err = parse("").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
    }

    // ── URI annotations: accepted on input, ignored (§5 of the design) ──────

    /// Annotations on the top form, forms, and every atom kind (number,
    /// string, bool, symbol, variable, list element) — the AST is identical
    /// to the bare text.
    #[test]
    fn annotated_document_parses_same_as_bare() {
        let bare = "numerical.model(\
             equation(x, sum(mul(2, y), 7)),\
             var(x),\
             var(y),\
             \"note\",\
             true,\
             ?w,\
             [a, b],\
         )";
        let annotated = "numerical.model[ex:m](\
             equation[ex:eq1](\
                 x[ex:x],\
                 sum[ex:s1](\
                     mul[ex:m1](2[ex:c1], y[ex:y]),\
                     7[ex:c2],\
                 ),\
             ),\
             var[ex:vx](x),\
             var[ex:vy](y),\
             \"note\"[ex:s1],\
             true[ex:b1],\
             ?w[ex:w1],\
             [a[ex:a1], b[ex:b1]],\
         )";
        assert_eq!(parse(bare).unwrap(), parse(annotated).unwrap());
    }

    /// Annotated input compiles to the same graph as bare input:
    /// annotations never reach the AST, so the compiled triples are
    /// unchanged. (Turtle *strings* can differ in serialization order —
    /// compare triple sets instead.)
    #[test]
    fn annotated_document_compiles_to_identical_graph() {
        let bare = "numerical.model(\
             equation(x, sum(mul(2, y), 7)), var(x), var(y))";
        let annotated = "numerical.model[ex:m](\
             equation[ex:eq1](\
                 x[ex:x],\
                 sum[ex:s1](mul[ex:m1](2[ex:c1], y[ex:y]), 7[ex:c2]),\
             ),\
             var[ex:vx](x),\
             var[ex:vy](y),\
         )";
        let cmp = |turtle: &str| {
            let mut triples: Vec<String> = crate::io::parse_turtle(turtle)
                .unwrap()
                .iter()
                .map(|t| t.to_string())
                .collect();
            triples.sort();
            triples
        };
        let bare_turtle = crate::sem::compile_to_turtle(bare).unwrap();
        let ann_turtle = crate::sem::compile_to_turtle(annotated).unwrap();
        assert_eq!(cmp(&bare_turtle), cmp(&ann_turtle));
    }

    /// Whitespace between a value and its annotation bracket is tolerated.
    #[test]
    fn annotation_after_whitespace_parses() {
        let d = parse("numerical.model(equation(x [ex:x], 7),)").unwrap();
        match &d.top.args[0] {
            Value::Form(f) => assert_eq!(f.name, "equation"),
            other => panic!("expected form, got {other:?}"),
        }
    }

    /// A single bare-ident bracket is still a list value, as before.
    #[test]
    fn bare_ident_bracket_is_still_a_list() {
        let d = parse("a.b([foo])").unwrap();
        match &d.top.args[0] {
            Value::List(items) => {
                assert_eq!(items.len(), 1);
                assert!(matches!(&items[0], Value::Atom(Atom::Sym(s)) if s == "foo"));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    /// Two annotations on one value are a parse error (at most one is
    /// consumed; output never emits more).
    #[test]
    fn double_annotation_is_parse_error() {
        let err = parse("a.b(x[ex:a][ex:b])").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
    }

    /// An annotation must follow a value — it cannot start one (`f([ex:x])`
    /// was never valid: bracket-position qnames could not be lexed before).
    #[test]
    fn annotation_at_value_start_is_parse_error() {
        let err = parse("a.b([ex:x])").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
    }
}
