//! The llguidance decode backend.
//!
//! This is the preferred backend: when the endpoint exposes logits, the grammar
//! compiled here is applied as a token mask, so a structured call cannot produce an
//! invalid value and needs no retry loop.
//!
//! The engine itself is not vendored into this repository, so what ships here is
//! the compilation half of the contract — a deterministic schema→grammar
//! compiler in the llguidance dialect, plus the conformance predicate
//! ([`DecoderBackend::accepts`]) the gate uses to prove the grammar is tight. Where
//! an actual engine is available in the deployment, it consumes
//! [`GrammarSpec::grammar`] unchanged; see the Phase 3 report for the deviation.

use serde_json::Value;

use crate::client::{DecoderKind, SchemaId};
use crate::error::LlmError;
use crate::grammar::{self, DecoderBackend, GrammarDialect, GrammarSpec};

/// The llguidance backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct LlguidanceBackend;

impl LlguidanceBackend {
    /// The backend.
    pub fn new() -> Self {
        LlguidanceBackend
    }

    /// The dialect this backend emits.
    pub fn dialect() -> GrammarDialect {
        GrammarDialect::Llguidance
    }
}

impl DecoderBackend for LlguidanceBackend {
    fn kind(&self) -> DecoderKind {
        DecoderKind::Llguidance
    }

    fn available(&self) -> bool {
        true
    }

    fn compile(&self, id: &SchemaId, schema: &Value) -> Result<GrammarSpec, LlmError> {
        grammar::compile(GrammarDialect::Llguidance, id, schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_backend_compiles_and_reports_itself_as_constrained() {
        let backend = LlguidanceBackend::new();
        assert!(backend.available());
        assert_eq!(backend.kind(), DecoderKind::Llguidance);
        assert!(backend.kind().is_constrained());

        let id = SchemaId::new("rating.v1");
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["label"],
            "properties": { "label": { "type": "string", "enum": ["low", "high"] } }
        });
        let spec = backend.compile(&id, &schema).unwrap();
        assert_eq!(spec.decoder, DecoderKind::Llguidance);
        assert_eq!(spec.schema_id, id);
        assert!(spec.grammar.starts_with("# llguidance grammar"));
        assert!(backend.accepts(&spec, &json!({"label": "low"})));
        assert!(!backend.accepts(&spec, &json!({"label": "mid"})));
    }
}
