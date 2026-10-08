//! SEM lexer (§3.1 of `planning/s_expr/s_expr_design2.md`): tokens, comments,
//! and trailing commas. Pure, total, deterministic — malformed input yields a
//! stable [`SemError`](`crate::sem::errors::SemError`) (code `sem:e10`), never a panic.

use crate::sem::errors::SemError;

/// A lexical token.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// `ident`, `bool`, `string`-or-symbol word: `[A-Za-z_][A-Za-z0-9_]*` (identifiers),
    /// plus strings `"…"` are lexed as [`Token::String`]. Numbers are lexed separately.
    Ident(String),
    /// `?ident` — an explicit variable reference (`?x`, `?person`), the logic
    /// SEM variable syntax (`logic_sem_design.md` §4.1).
    Var(String),
    /// `-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?` — parsed from the raw text.
    Number(f64),
    /// `"…"` — kept as the raw escaped content (unescaped at parse time).
    String(String),
    /// `true` / `false`.
    Bool(bool),
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `[ex:eq1]` / `[<https://…>]` — a URI annotation following a value
    /// (`planning/updates/sem_uri_annotation_design.md` §5). Annotations are
    /// an *output-only* decoration: the lexer recognizes a bracket whose
    /// content is exactly one IRI reference, and the parser accepts and
    /// discards the token. A bracket holding anything else (commas, bare
    /// idents, whitespace, nesting) lexes as a normal list.
    Annotation(String),
    /// `,`
    Comma,
    /// `.` — only meaningful inside a qname at the top level.
    Dot,
    /// `=` — keyword argument separator (`name=value`).
    Equals,
    /// `/` — declaration arity separator (`relation, took/1`).
    Slash,
}

/// A token with its character offset, for error messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub token: Token,
    pub offset: usize,
}
/// Whether `s` is a valid annotation IRI reference: an absolute IRI in angle
/// brackets (`<https://…>`) or a prefixed name (`prefix:local`). Used to
/// disambiguate an annotation bracket from a list bracket (§5.2).
fn is_annotation_iri(s: &str) -> bool {
    // Absolute IRI: `<…>` with no whitespace or angle brackets inside.
    if let Some(inner) = s.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        return !inner.is_empty()
            && !inner
                .chars()
                .any(|c| c.is_whitespace() || c == '<' || c == '>');
    }
    // Prefixed name: exactly one `:`; `prefix` is an identifier shape and
    // `local` additionally allows dots (e.g. `ex:eq1`, `ex:a-b.c`).
    let (prefix, local) = match s.split_once(':') {
        Some((p, l)) if !l.contains(':') => (p, l),
        _ => return false,
    };
    let ident_start = |c: char| c.is_ascii_alphabetic() || c == '_';
    let ident_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let name_ok = |name: &str, extra: fn(char) -> bool| {
        let mut chars = name.chars();
        match chars.next() {
            Some(c) if ident_start(c) => chars.all(|c| ident_char(c) || extra(c)),
            _ => false,
        }
    };
    name_ok(prefix, |_| false) && name_ok(local, |c| c == '.')
}

/// Scan an annotation bracket starting at `[` (offset `start`): returns the
/// IRI text and the offset just past the closing `]` when the bracket holds
/// exactly one IRI reference with no internal whitespace, commas, or
/// brackets. `[]` (empty) and anything else fall back to the list path.
fn scan_annotation(input: &str, start: usize) -> Option<(&str, usize)> {
    let b = input.as_bytes();
    let close = b[start + 1..].iter().position(|&c| c == b']')? + start + 1;
    let content = &input[start + 1..close];
    if content.is_empty()
        || content
            .chars()
            .any(|c| c.is_whitespace() || c == ',' || c == '[' || c == ']')
    {
        return None;
    }
    if is_annotation_iri(content) {
        Some((content, close + 1))
    } else {
        None
    }
}

/// Lex a SEM document into a token stream. Comments (`# …`) and whitespace
/// are skipped; a trailing comma is allowed and dropped here, matching the
/// grammar's `values ::= value ("," value)* [","]`.
pub fn lex(input: &str) -> Result<Vec<Spanned>, SemError> {
    let b = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' => {
                out.push(Spanned {
                    token: Token::LParen,
                    offset: i,
                });
                i += 1;
            }
            b')' => {
                out.push(Spanned {
                    token: Token::RParen,
                    offset: i,
                });
                i += 1;
            }
            b'[' => {
                // Annotation bracket (`[ex:eq1]`, `[<https://…>]`) or a list?
                // An annotation holds exactly one IRI reference with no
                // internal whitespace/commas/brackets; anything else —
                // including `[]` (empty list) — takes the list path (§5.2).
                match scan_annotation(input, i) {
                    Some((content, end)) => {
                        out.push(Spanned {
                            token: Token::Annotation(content.to_string()),
                            offset: i,
                        });
                        i = end;
                    }
                    None => {
                        out.push(Spanned {
                            token: Token::LBracket,
                            offset: i,
                        });
                        i += 1;
                    }
                }
            }
            b']' => {
                out.push(Spanned {
                    token: Token::RBracket,
                    offset: i,
                });
                i += 1;
            }
            b',' => {
                out.push(Spanned {
                    token: Token::Comma,
                    offset: i,
                });
                i += 1;
            }
            b'=' => {
                out.push(Spanned {
                    token: Token::Equals,
                    offset: i,
                });
                i += 1;
            }
            b'/' => {
                out.push(Spanned {
                    token: Token::Slash,
                    offset: i,
                });
                i += 1;
            }
            b'.' => {
                out.push(Spanned {
                    token: Token::Dot,
                    offset: i,
                });
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < b.len() {
                    match b[i] {
                        b'\\' => {
                            if i + 1 < b.len() {
                                match b[i + 1] {
                                    b'\\' => s.push('\\'),
                                    b'"' => s.push('"'),
                                    b'n' => s.push('\n'),
                                    b't' => s.push('\t'),
                                    // Unknown escapes (`\+`, `\d`, …) pass
                                    // through literally instead of failing:
                                    // LLM-written strings like Prolog `\+`
                                    // must compile, and the value is kept
                                    // byte-identical so rendering round-trips.
                                    other => {
                                        s.push('\\');
                                        s.push(other as char);
                                    }
                                }
                                i += 2;
                            } else {
                                return Err(SemError::Lexical(format!(
                                    "unterminated escape at offset {start}"
                                )));
                            }
                        }
                        b'"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        other => {
                            s.push(other as char);
                            i += 1;
                        }
                    }
                }
                if !closed {
                    return Err(SemError::Lexical(format!(
                        "unterminated string at offset {start}"
                    )));
                }
                out.push(Spanned {
                    token: Token::String(s),
                    offset: start,
                });
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                if c == b'-' {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i < b.len() && b[i] == b'.' {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                    let mut j = i + 1;
                    if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                        j += 1;
                    }
                    if j < b.len() && b[j].is_ascii_digit() {
                        i = j;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text = &input[start..i];
                let val: f64 = text.parse().map_err(|_| {
                    SemError::Lexical(format!("invalid number \"{text}\" at offset {start}"))
                })?;
                out.push(Spanned {
                    token: Token::Number(val),
                    offset: start,
                });
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'-')
                {
                    i += 1;
                }
                let word = &input[start..i];
                let token = match word {
                    "true" => Token::Bool(true),
                    "false" => Token::Bool(false),
                    _ => Token::Ident(word.to_string()),
                };
                out.push(Spanned {
                    token,
                    offset: start,
                });
            }
            b'?' => {
                let start = i;
                i += 1;
                if i >= b.len() || !(b[i].is_ascii_alphabetic() || b[i] == b'_') {
                    return Err(SemError::Lexical(format!(
                        "expected a name after `?` at offset {start}"
                    )));
                }
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                out.push(Spanned {
                    token: Token::Var(input[start + 1..i].to_string()),
                    offset: start,
                });
            }
            other => {
                return Err(SemError::Lexical(format!(
                    "unexpected character {:?} at offset {i}",
                    other as char
                )))
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_numbers_and_idents() {
        let toks = lex("a 1.5 -3 [x, y] \"hi\"").unwrap();
        let kinds: Vec<&str> = toks
            .iter()
            .map(|t| match &t.token {
                Token::Ident(_) => "ident",
                Token::Number(_) => "num",
                Token::String(_) => "str",
                Token::Bool(_) => "bool",
                _ => "punct",
            })
            .collect();
        // `-3` lexes as a number (the `-` number branch).
        assert_eq!(
            kinds,
            vec!["ident", "num", "num", "punct", "ident", "punct", "ident", "punct", "str"]
        );
    }

    #[test]
    fn skips_comments() {
        let toks = lex("# a comment\n(a, # trailing\n2)").unwrap();
        assert_eq!(toks.len(), 5);
    }

    #[test]
    fn number_lexeme_covers_decimal_and_exponent() {
        let toks = lex("-1.5e3").unwrap();
        match &toks[0].token {
            Token::Number(n) => assert_eq!(*n, -1500.0),
            other => panic!("expected number, got {other:?}"),
        }
    }

    #[test]
    fn unterminated_string_is_lexical_error() {
        let err = lex("\"abc").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
    }

    #[test]
    fn unknown_escapes_pass_through_literally() {
        // Prolog's `\+` (and any other unknown escape) must compile and keep
        // its bytes, so LLM-written strings round-trip instead of failing.
        let toks = lex("\"a \\+ b \\d\"").unwrap();
        match &toks[0].token {
            Token::String(s) => assert_eq!(*s, "a \\+ b \\d"),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn standard_escapes_still_decode() {
        let toks = lex("\"a \\n b\\\"\"").unwrap();
        match &toks[0].token {
            Token::String(s) => assert_eq!(*s, "a \n b\""),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn question_mark_lexes_as_variable() {
        let toks = lex("?x").unwrap();
        assert_eq!(
            toks[0].token,
            Token::Var("x".to_string()),
            "got {:?}",
            toks[0].token
        );
        let toks = lex("p(?person, alice)").unwrap();
        assert_eq!(toks[2].token, Token::Var("person".to_string()));
        assert!(matches!(toks[4].token, Token::Ident(_)));
    }

    #[test]
    fn bare_question_mark_is_lexical_error() {
        let err = lex("?").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
        let err = lex("?1x").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
    }

    #[test]
    fn question_mark_inside_string_stays_string() {
        let toks = lex("\"a ?x b\"").unwrap();
        match &toks[0].token {
            Token::String(s) => assert_eq!(*s, "a ?x b"),
            other => panic!("expected string, got {other:?}"),
        }
    }

    // ── URI annotation brackets (§5.2 of the design) ────────────────────────

    #[test]
    fn annotation_bracket_lexes_as_annotation() {
        let toks = lex("x[ex:c1]").unwrap();
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].token, Token::Ident("x".to_string()));
        assert_eq!(
            toks[1].token,
            Token::Annotation("ex:c1".to_string()),
            "got {:?}",
            toks[1].token
        );
        assert_eq!(toks[1].offset, 1);
    }

    #[test]
    fn annotation_absolute_iri_lexes() {
        let toks = lex("eq[<https://example.org/ns/ex#eq1>]").unwrap();
        assert_eq!(
            toks[1].token,
            Token::Annotation("<https://example.org/ns/ex#eq1>".to_string())
        );
    }

    #[test]
    fn annotation_local_allows_dots_and_dashes() {
        let toks = lex("x[ex:a-b.c]").unwrap();
        assert_eq!(toks[1].token, Token::Annotation("ex:a-b.c".to_string()));
    }

    #[test]
    fn whitespace_between_value_and_annotation_ok() {
        let toks = lex("eq [ex:eq1]").unwrap();
        assert_eq!(toks.len(), 2);
        assert!(matches!(toks[1].token, Token::Annotation(_)));
    }

    #[test]
    fn qname_list_bracket_is_a_list_not_an_annotation() {
        // `x[ex:x, ex:y]` — two terms inside the bracket: list path. The
        // bare colon then fails to lex (qname lists were never valid SEM);
        // the important guarantee is that it is NOT silently an annotation.
        assert!(lex("x[ex:x, ex:y]").is_err());
    }

    #[test]
    fn bare_ident_bracket_takes_the_list_path() {
        // `x[foo]` — no colon: a list bracket, exactly as before this change.
        let toks = lex("x[foo]").unwrap();
        assert_eq!(toks[1].token, Token::LBracket);
    }

    #[test]
    fn empty_bracket_takes_the_list_path() {
        // `[]` keeps its existing empty-list meaning.
        let toks = lex("x[]").unwrap();
        assert_eq!(toks[1].token, Token::LBracket);
        assert_eq!(toks[2].token, Token::RBracket);
    }

    #[test]
    fn colon_without_annotation_shape_is_a_lex_error() {
        // `[ex:]` — empty local name: not an annotation, and a bare colon
        // cannot be lexed (surface as the stable lexical error).
        let err = lex("x[ex:]").unwrap_err();
        assert_eq!(err.code(), "sem:e10");
    }

    #[test]
    fn annotation_inside_string_stays_string() {
        let toks = lex("\"took(alice) [ex:not-an-annotation].\"").unwrap();
        match &toks[0].token {
            Token::String(s) => {
                assert_eq!(*s, "took(alice) [ex:not-an-annotation].")
            }
            other => panic!("expected string, got {other:?}"),
        }
    }
}
