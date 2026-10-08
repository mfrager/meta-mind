//! The XGrammar decode backend.
//!
//! The second constrained-decoding engine behind the same [`DecoderBackend`] trait.
//! It exists so that the choice of engine is a configuration decision rather than
//! an architectural one: `mm-cli llm grammars --score` compiles every fixture
//! schema with both backends and reports which one handles the vocabulary, and
//! nothing else in the substrate changes when the default moves.
//!
//! As with [`crate::backend_llguidance`], the compilation half of the contract
//! ships here and an engine consumes [`GrammarSpec::grammar`]; see the Phase 3
//! report for the deviation.

use serde_json::Value;

use crate::client::{DecoderKind, SchemaId};
use crate::error::LlmError;
use crate::grammar::{self, DecoderBackend, GrammarDialect, GrammarSpec};

/// The XGrammar backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct XGrammarBackend;

impl XGrammarBackend {
    /// The backend.
    pub fn new() -> Self {
        XGrammarBackend
    }

    /// The dialect this backend emits.
    pub fn dialect() -> GrammarDialect {
        GrammarDialect::XGrammar
    }
}

impl DecoderBackend for XGrammarBackend {
    fn kind(&self) -> DecoderKind {
        DecoderKind::XGrammar
    }

    fn available(&self) -> bool {
        true
    }

    fn compile(&self, id: &SchemaId, schema: &Value) -> Result<GrammarSpec, LlmError> {
        grammar::compile(GrammarDialect::XGrammar, id, schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_llguidance::LlguidanceBackend;
    use serde_json::json;

    #[test]
    fn both_backends_agree_on_acceptance_but_not_on_text() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["steps"],
            "properties": {
                "steps": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 1,
                    "maxItems": 4
                }
            }
        });
        let id = SchemaId::new("plan.v1");
        let a = LlguidanceBackend::new().compile(&id, &schema).unwrap();
        let b = XGrammarBackend::new().compile(&id, &schema).unwrap();

        let valid = json!({"steps": ["a", "b"]});
        let too_many = json!({"steps": ["a", "b", "c", "d", "e"]});
        assert!(a.accepts(&valid) && b.accepts(&valid));
        assert!(!a.accepts(&too_many) && !b.accepts(&too_many));
        assert_ne!(a.grammar, b.grammar);
        assert!(b.grammar.contains("array(minItems=1"), "{}", b.grammar);
    }
}
