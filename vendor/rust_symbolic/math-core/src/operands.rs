//! Canonical operand normalization (master_design.md §6.0).
//!
//! All expression operators use a single ordered `model:operands` RDF list.
//! Legacy `math:operand` (repeated) and `math:left`/`math:right` (binary) forms
//! are normalized to the same ordered list here.

use oxigraph::model::{NamedNode, Term};

use crate::model::MathModel;
use crate::vocabulary::*;

/// The canonical (ordered) operands of an expression operator node.
pub fn canonical_operands(model: &MathModel, expr_subject: &str) -> Vec<Term> {
    // 1. Canonical RDF list via math:operands.
    if let Some(head) = model.single_object(expr_subject, PRED_OPERANDS) {
        let items = rdf_list_items(model, &head);
        if !items.is_empty() {
            return items;
        }
    }

    // 2. Legacy repeated math:operand (best-effort order).
    if let Ok(operands) = model.objects(expr_subject, PRED_OPERAND) {
        if !operands.is_empty() {
            return operands;
        }
    }

    // 3. Legacy binary math:left / math:right.
    if let (Some(l), Some(r)) = (
        model.single_object(expr_subject, PRED_LEFT),
        model.single_object(expr_subject, PRED_RIGHT),
    ) {
        return vec![l, r];
    }

    // 4. Unary math:argument.
    if let Some(a) = model.single_object(expr_subject, PRED_ARGUMENT) {
        return vec![a];
    }

    Vec::new()
}

/// Walk an RDF collection (`rdf:first` / `rdf:rest` until `rdf:nil`).
pub fn rdf_list_items(model: &MathModel, head: &Term) -> Vec<Term> {
    let nil: Term = match NamedNode::new(RDF_NIL) {
        Ok(n) => n.into(),
        Err(_) => return Vec::new(),
    };

    let mut items = Vec::new();
    let mut current = head.clone();
    let mut guard = 0usize;

    loop {
        guard += 1;
        if guard > 100_000 || current == nil {
            break;
        }

        let first = model.objects_of_term(&current, RDF_FIRST);
        let rest = model.objects_of_term(&current, RDF_REST);

        match (first.into_iter().next(), rest.into_iter().next()) {
            (Some(f), Some(r)) => {
                items.push(f);
                current = r;
            }
            (Some(f), None) => {
                items.push(f);
                break;
            }
            _ => break,
        }
    }

    items
}
