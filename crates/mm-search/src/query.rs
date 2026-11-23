//! The bounded query parser.
//!
//! The model never supplies Lucene or Tantivy syntax. [`sanitize_tokens`] turns
//! free text into plain alphanumeric words and [`build_query`] composes only
//! term queries, filters, and boosts from them. Anything the model sends that
//! looks like syntax is treated as a word.

use std::collections::BTreeSet;

use tantivy::query::{BooleanQuery, EmptyQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::IndexRecordOption;
use tantivy::{Index, Term};

use crate::schema::SearchFields;

/// Maximum words taken from one query.
pub const MAX_QUERY_TOKENS: usize = 12;
/// A token longer than this is truncated.
pub const MAX_TOKEN_CHARS: usize = 64;

/// Normalize free text into bounded, lowercase alphanumeric tokens.
pub fn sanitize_tokens(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut tokens = Vec::new();
    for raw in text.split_whitespace() {
        let token: String = raw
            .chars()
            .filter(|ch| ch.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .take(MAX_TOKEN_CHARS)
            .collect();
        if token.is_empty() {
            continue;
        }
        if seen.insert(token.clone()) {
            tokens.push(token);
        }
        if tokens.len() >= MAX_QUERY_TOKENS {
            break;
        }
    }
    tokens
}

/// Build the query for a search: all tokens are `SHOULD` over label and body,
/// with the label boosted threefold, plus exact filters for class, graph, and
/// namespace, and an active-status filter unless deleted rows are requested.
pub fn build_query(
    index: &Index,
    fields: &SearchFields,
    query: &crate::SearchQuery,
) -> Box<dyn Query> {
    let tokens = sanitize_tokens(&query.text);
    if tokens.is_empty() {
        return Box::new(EmptyQuery);
    }

    // The parser is what applies the field tokenizer (English stemming) to the
    // query words. The words were already sanitized, so no syntax survives.
    let mut parser = QueryParser::for_index(index, vec![fields.label, fields.body]);
    parser.set_field_boost(fields.label, 3.0);
    let text = match parser.parse_query(&tokens.join(" ")) {
        Ok(query) => query,
        Err(_) => return Box::new(EmptyQuery),
    };

    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    clauses.push((Occur::Must, text));
    if let Some(class) = query.class.as_deref().filter(|s| !s.trim().is_empty()) {
        clauses.push((Occur::Must, exact(fields.types, class)));
    }
    if let Some(graph) = query.graph.as_deref().filter(|s| !s.trim().is_empty()) {
        clauses.push((Occur::Must, exact(fields.graph, graph)));
    }
    if let Some(namespace) = query.namespace.as_deref().filter(|s| !s.trim().is_empty()) {
        clauses.push((Occur::Must, exact(fields.namespace, namespace)));
    }
    if !query.include_deleted {
        clauses.push((Occur::Must, exact(fields.status, "active")));
    }
    Box::new(BooleanQuery::new(clauses))
}

fn exact(field: tantivy::schema::Field, value: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, value),
        IndexRecordOption::Basic,
    ))
}

/// A short, deterministic snippet around the first matching token.
pub fn snippet(body: &str, text: &str) -> Option<String> {
    const MAX: usize = 240;
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    let tokens = sanitize_tokens(text);
    let lower = body.to_lowercase();
    let offset = tokens
        .iter()
        .filter_map(|token| lower.find(token))
        .min()
        .unwrap_or(0);
    let start = body
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|&idx| idx <= offset)
        .last()
        .unwrap_or(0);
    let snippet: String = body[start..].chars().take(MAX).collect();
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        None
    } else if body.chars().count() > MAX {
        Some(format!("{trimmed}…"))
    } else {
        Some(trimmed.to_string())
    }
}
