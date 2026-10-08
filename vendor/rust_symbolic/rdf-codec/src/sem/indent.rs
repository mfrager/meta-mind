//! Structural SEM indenting for LLM-facing output and the Web UI.
//!
//! The rule is structural rather than width-based:
//! - If a form's arguments contain no nested function calls, the whole call
//!   stays on one line — `mul(3, y)`, `neg(z)`, `var(x)` roll up regardless
//!   of their length.
//! - If an argument is itself a function call, the call expands with each
//!   argument on its own indented line.
//! - Recursively applied, so `sum(mul(...), neg(...))` expands while its
//!   leaves stay rolled up.
//!
//! Lists are containers too: `key=[a, b]` follows the same rule. A list whose
//! elements contain nested calls expands element-by-element
//! (`algorithms=[algorithm(...), algorithm(...)]`), while a list of plain
//! leaves rolls up (`classify=["a", "b"]`).
//!
//! Quoted strings are opaque: commas and parens inside quotes neither split
//! arguments nor count as function calls (so `["parent(a, b)."]` stays one
//! leaf). URI annotations (`mul[ex:m1](`, `2[ex:c1]`) are inert brackets that
//! attach to an atom — they are never treated as lists. The mirror of this
//! module lives at `frontend/src/lib/sem-indent.ts` so the Web UI applies the
//! identical rule when viewing raw SEM inputs.

/// Structurally indent a SEM document (or any s-expression text).
pub fn indent(input: &str) -> String {
    format_expr(input.trim(), 0)
}

/// Whether `s` contains a nested function call — any `(` outside a quoted
/// string.
fn has_nested_call(s: &str) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    for ch in s.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' => return true,
            _ => {}
        }
    }
    false
}

/// Whether a `[` at this position is an annotation fragment
/// (`[prefix:local]` / `[<iri>]`) riding on an atom — `mul[ex:m1](`,
/// `2[ex:c1]`, `took(alice)[ex:a1]` — rather than a list opener. Annotations
/// attach directly to an atom (identifier, number, or closing bracket), while
/// lists follow `=`, `(`, `,`, or the start of the expression.
fn is_annotation_bracket(prev: Option<char>) -> bool {
    matches!(
        prev,
        Some(c) if c.is_alphanumeric() || c == '_' || c == '-' || c == ')' || c == ']'
    )
}

/// Find the first container opener: a call `(` or a list `[` (not an
/// annotation fragment), outside quoted strings. Returns the byte index and
/// the opener character.
fn find_container(s: &str) -> Option<(usize, char)> {
    let mut in_string = false;
    let mut escaped = false;
    let mut prev: Option<char> = None;
    for (i, ch) in s.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' => return Some((i, '(')),
            '[' if !is_annotation_bracket(prev) => return Some((i, '[')),
            _ => {}
        }
        prev = Some(ch);
    }
    None
}

/// Find the index of the closer matching the opener at `open_idx`, counting
/// both paren and bracket nesting and skipping quoted strings.
fn matching_close(s: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in s.char_indices().skip(open_idx) {
        if i == open_idx {
            depth = 1;
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split top-level (depth-0) comma-separated arguments, ignoring commas inside
/// quoted strings and nested parens.
fn split_args(s: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        args.push(s[start..].trim());
    }
    args
}

fn format_expr(s: &str, indent_level: usize) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    let Some((open, open_ch)) = find_container(s) else {
        return s.to_string();
    };
    let Some(close) = matching_close(s, open) else {
        return s.to_string();
    };
    let close_ch = if open_ch == '(' { ')' } else { ']' };
    let name = s[..open].trim();
    let args = split_args(&s[open + 1..close]);
    let tail = s[close + 1..].trim();

    // Leaf calls roll up regardless of length.
    if !args.iter().any(|arg| has_nested_call(arg)) {
        return format!("{name}{open_ch}{}{close_ch}{tail}", args.join(", "));
    }

    let child_indent = indent_level + 1;
    let pad = "    ".repeat(child_indent);
    let formatted: Vec<String> = args
        .iter()
        .map(|arg| format!("{pad}{}", format_expr(arg, child_indent)))
        .collect();
    format!(
        "{name}{open_ch}\n{}\n{}{close_ch}{tail}",
        formatted.join(",\n"),
        "    ".repeat(indent_level),
    )
}

#[cfg(test)]
mod tests {
    use super::indent;

    #[test]
    fn leaves_roll_up_regardless_of_length() {
        assert_eq!(indent("var(x)"), "var(x)");
        assert_eq!(indent("mul(3, y)"), "mul(3, y)");
        assert_eq!(indent("neg(z)"), "neg(z)");
        assert_eq!(indent("param(a, 2.0)"), "param(a, 2.0)");
    }

    #[test]
    fn nested_calls_expand() {
        assert_eq!(
            indent("equation(x, sum(a, b))"),
            "equation(\n    x,\n    sum(a, b)\n)"
        );
    }

    #[test]
    fn deep_nesting_expands_recursively() {
        let input = "numerical.model(equation(x, div(sub(7, sum(mul(3, y), neg(z))), 2)), var(x), var(y), var(z))";
        let expected = "\
numerical.model(
    equation(
        x,
        div(
            sub(
                7,
                sum(
                    mul(3, y),
                    neg(z)
                )
            ),
            2
        )
    ),
    var(x),
    var(y),
    var(z)
)";
        assert_eq!(indent(input), expected);
    }

    #[test]
    fn quoted_strings_are_opaque() {
        // Commas and parens inside quotes neither split args nor force
        // expansion.
        assert_eq!(
            indent("logic.model([\"parent(a, bob).\"])"),
            "logic.model([\"parent(a, bob).\"])"
        );
        assert_eq!(indent("model(text=\"x, (y)\")"), "model(text=\"x, (y)\")");
    }

    #[test]
    fn lists_are_containers() {
        // A list of plain leaves rolls up regardless of length.
        assert_eq!(
            indent("model(classify=[\"a\", \"b\"])"),
            "model(classify=[\"a\", \"b\"])"
        );
        // A list whose elements contain calls expands element-by-element.
        let input = "complexity.query(size=1024, budget=budget(time_seconds=300000, unit_system=\"abstract\"), algorithms=[algorithm(\"linear-scan\", linear(), constant()), algorithm(\"pairwise-analysis\", quadratic(), linear())], classify=[\"linear-scan\", \"pairwise-analysis\"], assess=[\"pairwise-analysis\"], hardware_factor=1)";
        let expected = "\
complexity.query(
    size=1024,
    budget=budget(time_seconds=300000, unit_system=\"abstract\"),
    algorithms=[
        algorithm(
            \"linear-scan\",
            linear(),
            constant()
        ),
        algorithm(
            \"pairwise-analysis\",
            quadratic(),
            linear()
        )
    ],
    classify=[\"linear-scan\", \"pairwise-analysis\"],
    assess=[\"pairwise-analysis\"],
    hardware_factor=1
)";
        assert_eq!(indent(input), expected);
    }

    #[test]
    fn bare_list_argument_expands() {
        let input = "event.problem(log([case(\"case-a\", [\"Receive\", \"Validate\", \"Process\"]), case(\"case-b\", [\"Receive\", \"Process\"])]), process(\"expected-flow\", [\"Receive\", \"Validate\", \"Process\"]))";
        let expected = "\
event.problem(
    log(
        [
            case(\"case-a\", [\"Receive\", \"Validate\", \"Process\"]),
            case(\"case-b\", [\"Receive\", \"Process\"])
        ]
    ),
    process(\"expected-flow\", [\"Receive\", \"Validate\", \"Process\"])
)";
        assert_eq!(indent(input), expected);
    }

    #[test]
    fn non_sem_text_passes_through() {
        assert_eq!(indent("parent(alice, bob)."), "parent(alice, bob).");
        assert_eq!(indent("just text"), "just text");
    }

    // ── URI annotations are inert (§6.3 of the design) ──────────────────────

    /// Annotation brackets contain no parens or commas: leaves with
    /// annotations roll up, annotated nested calls expand exactly like their
    /// bare counterparts, and the fragment rides along as header text.
    #[test]
    fn annotations_are_inert() {
        assert_eq!(
            indent("mul[ex:m1](2[ex:c1], x[ex:x])"),
            "mul[ex:m1](2[ex:c1], x[ex:x])"
        );
        assert_eq!(
            indent("equation[ex:eq1](x[ex:x], sum[ex:s1](a[ex:a], b[ex:b]))"),
            "equation[ex:eq1](\n    x[ex:x],\n    sum[ex:s1](a[ex:a], b[ex:b])\n)"
        );
        // Bare annotated atoms pass through untouched.
        assert_eq!(indent("2[ex:c1]"), "2[ex:c1]");
        // Inline atoms with annotations expand exactly like bare nested
        // calls (the arg contains a call, so the parent expands).
        assert_eq!(
            indent("fact[ex:s0](took(alice)[ex:a1])"),
            "fact[ex:s0](\n    took(alice)[ex:a1]\n)"
        );
        assert_eq!(
            indent("x[<https://example.org/m/x>]")
                .matches("x[<https://example.org/m/x>]")
                .count(),
            1
        );
    }

    /// Indenting annotated text yields the bare layout plus inline
    /// annotations: stripping the fragments gives the bare result exactly.
    #[test]
    fn annotated_indent_matches_bare_layout() {
        let annotated = "numerical.model[ex:m](equation[ex:eq0](x[ex:x], \
             sum[ex:e0](mul[ex:e1](2[ex:c0], y[ex:y]), 7[ex:c1])), \
             var[ex:x](x), var[ex:y](y))";
        let bare = "numerical.model(equation(x, sum(mul(2, y), 7)), var(x), var(y))";
        assert_eq!(strip_fragments(&indent(annotated)), indent(bare));
    }

    /// Remove `[prefix:…]` / `[<iri>]` annotation fragments (outside quotes)
    /// from indented SEM text.
    fn strip_fragments(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.char_indices().peekable();
        let mut in_string = false;
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    in_string = !in_string;
                    out.push(c);
                }
                '[' if !in_string => {
                    if let Some(close) = s[i..].find(']') {
                        let content = &s[i + 1..i + close];
                        let is_iri = content.starts_with('<')
                            || content.split_once(':').is_some_and(|(p, l)| {
                                !p.is_empty()
                                    && !l.is_empty()
                                    && !content.contains(char::is_whitespace)
                            });
                        if is_iri {
                            // Consume the fragment content plus `]` — the
                            // `[` was already consumed above.
                            for _ in 0..close {
                                chars.next();
                            }
                            continue;
                        }
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }
}
