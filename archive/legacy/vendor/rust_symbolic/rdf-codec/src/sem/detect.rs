//! Edge dispatch (design §9.1 Phase 0, `detect.rs`): choose the parser by the
//! first meaningful character. `[` → legacy cRDF JSON; anything else (an
//! identifier) → SEM text. The SEM parser never produces or consumes cRDF —
//! this is purely the format-selection edge.

/// The format family of an incoming payload string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadFormat {
    /// Legacy cRDF JSON (first char `[`).
    Crdf,
    /// SEM document (first char is an identifier start).
    Sem,
}

/// Classify a payload by its first non-whitespace character.
pub fn detect(text: &str) -> PayloadFormat {
    match text.trim_start().chars().next() {
        Some('[') => PayloadFormat::Crdf,
        _ => PayloadFormat::Sem,
    }
}

/// Whether a payload is SEM (an identifier-led document).
pub fn is_sem(text: &str) -> bool {
    detect(text) == PayloadFormat::Sem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bracket_is_crdf() {
        assert_eq!(
            detect("[\"ex:M\", \"model:Model\", {}]"),
            PayloadFormat::Crdf
        );
        assert!(!is_sem("[\"ex:M\", \"model:Model\", {}]"));
    }

    #[test]
    fn ident_is_sem() {
        assert_eq!(detect("numerical.model(var(gdp))"), PayloadFormat::Sem);
        assert!(is_sem("numerical.model(var(gdp))"));
    }

    #[test]
    fn leading_whitespace_is_ignored() {
        assert_eq!(detect("  \n [\"ex:M\"]"), PayloadFormat::Crdf);
        assert_eq!(detect("  \n numerical.model()"), PayloadFormat::Sem);
    }

    #[test]
    fn empty_is_sem_by_default() {
        assert_eq!(detect(""), PayloadFormat::Sem);
        assert_eq!(detect("# only a comment"), PayloadFormat::Sem);
    }
}
