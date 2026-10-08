//! SEM errors (§3.5 of `planning/s_expr/s_expr_design2.md`): the stable,
//! deterministic, single-line error set with codes `sem:e1`…`sem:e10`.
//! Every failure of lex/parse/compile/render is one of these — the LLM
//! sees the code plus detail in tool results and can repair deterministically.

/// The stable error set. Codes follow the design's §3.5 table; `Lexical`
/// covers malformed input (E10 in the extended table — the design's five
/// lexical cases carry the same stable `sem:e10` code).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemError {
    /// Unknown module (top-level qname prefix).
    UnknownModule(String),
    /// Unknown function/form in the module's vocabulary.
    UnknownFunction(String),
    /// Wrong number of arguments.
    Arity(String),
    /// A bare identifier used where a declared symbol is required but not declared.
    UndeclaredSymbol(String),
    /// Type mismatch (e.g. number where a string/symbol is required).
    TypeMismatch(String),
    /// A value not in the allowed set (e.g. kind).
    BadValue(String),
    /// A qualified name at a nesting level > 0, an unqualified top-level
    /// name, or extra top-level content.
    BadScope(String),
    /// A reference (by name) to something not declared in the document.
    UnboundReference(String),
    /// Two definitions of the same name.
    DuplicateDefinition(String),
    /// Lexical/parse-level failure.
    Lexical(String),
}

impl SemError {
    /// Render the stable one-line `sem:eN` code.
    pub fn code(&self) -> String {
        match self {
            SemError::UnknownModule(_) => "sem:e1".to_string(),
            SemError::UnknownFunction(_) => "sem:e2".to_string(),
            SemError::Arity(_) => "sem:e3".to_string(),
            SemError::UndeclaredSymbol(_) => "sem:e4".to_string(),
            SemError::TypeMismatch(_) => "sem:e5".to_string(),
            SemError::BadValue(_) => "sem:e6".to_string(),
            SemError::BadScope(_) => "sem:e7".to_string(),
            SemError::UnboundReference(_) => "sem:e8".to_string(),
            SemError::DuplicateDefinition(_) => "sem:e9".to_string(),
            SemError::Lexical(_) => "sem:e10".to_string(),
        }
    }
}

impl std::fmt::Display for SemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (code, detail) = match self {
            SemError::UnknownModule(m) => ("sem:e1", format!("unknown module \"{m}\"")),
            SemError::UnknownFunction(fn_) => ("sem:e2", format!("unknown function \"{fn_}\"")),
            SemError::Arity(d) => ("sem:e3", d.clone()),
            SemError::UndeclaredSymbol(s) => ("sem:e4", format!("undeclared symbol \"{s}\"")),
            SemError::TypeMismatch(d) => ("sem:e5", d.clone()),
            SemError::BadValue(d) => ("sem:e6", d.clone()),
            SemError::BadScope(d) => ("sem:e7", d.clone()),
            SemError::UnboundReference(n) => ("sem:e8", format!("unbound reference \"{n}\"")),
            SemError::DuplicateDefinition(n) => ("sem:e9", format!("duplicate definition \"{n}\"")),
            SemError::Lexical(d) => ("sem:e10", d.clone()),
        };
        write!(f, "{code}: {detail}")
    }
}

impl std::error::Error for SemError {}
