//! The commit gate: shapes, duplicates, orphans.
//!
//! Invariant 1 of the plan is "SHACL-or-nothing": no entry reaches `/library`
//! without passing its shapes and the orphan check. Three checks, all of which a
//! caller can run *before* writing, which is what makes them a gate rather than a
//! post-mortem:
//!
//! * [`validate_turtle`] runs the real shapes file over a document.
//! * [`duplicates`] finds two entries with the same content hash — the same thing
//!   stored twice, which would double its weight in every rank.
//! * [`orphans`] finds a reference that does not resolve. A technique whose
//!   evidence is a typo is a technique with no evidence, and the difference is
//!   invisible until someone asks.
//!
//! The checks are pure functions over text and lists, so the same rules run in a
//! test, in `mm-cli library validate`, and in `LibraryManager::upsert_entry`. A
//! gate that only existed on the write path could not be asked a question.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mm_core::Config;

use crate::error::{LibraryError, Result, ShapeViolation};

/// The committed shapes file.
pub fn shapes_path() -> PathBuf {
    Config::repo_root()
        .join("ontology")
        .join("shapes")
        .join("library.shacl.ttl")
}

/// The committed shapes document, or a config error naming the path.
pub fn shapes() -> Result<String> {
    let path = shapes_path();
    std::fs::read_to_string(&path)
        .map_err(|e| LibraryError::Config(format!("cannot read {}: {e}", path.display())))
}

/// The violations a data document produces against a shapes document.
///
/// The vendored validator reports the violated *property path* and not the shape
/// that declared it, so the shape is resolved here: the data document names its
/// kind (`mm:Principle`), the shapes file declares one shape per kind
/// (`mm:PrincipleShape`), and for a document of that kind the named shape is the
/// one the violation belongs to. When the document names no kind, or the shapes
/// file declares no shape for it, the property path doubles as the label — a
/// refusal that names the property it was about is still actionable, and inventing
/// a shape name the validation never mentioned would be worse.
pub fn violations(shapes: &str, data: &str, label: &str) -> Result<Vec<ShapeViolation>> {
    let report = mm_store_graph::validate_turtle_text(shapes, data, label)
        .map_err(|e| LibraryError::Codec(e.to_string()))?;
    let kind = kind_class(data);
    Ok(report
        .violations
        .into_iter()
        .map(|violation| {
            let path_local = violation
                .path
                .rsplit('#')
                .next()
                .unwrap_or(&violation.path)
                .to_string();
            let shape = kind
                .as_deref()
                .map(|kind| format!("{kind}Shape"))
                .filter(|shape| shapes.contains(&format!("mm:{shape}")))
                .unwrap_or(path_local);
            ShapeViolation {
                shape,
                message: violation.message,
                path: violation.path,
            }
        })
        .collect())
}

/// The entry kind a data document declares, if it declares one.
fn kind_class(data: &str) -> Option<String> {
    let graph = rdf_codec::io::parse_turtle(data).ok()?;
    let mut kinds: Vec<String> = Vec::new();
    for triple in graph.iter() {
        if triple.predicate.as_str() != "http://www.w3.org/1999/02/22-rdf-syntax-ns#type" {
            continue;
        }
        let object = triple.object.to_string();
        let local = object
            .trim_start_matches('<')
            .trim_end_matches('>')
            .rsplit('#')
            .next()
            .unwrap_or_default()
            .to_string();
        if local != "LibraryEntry" && !local.is_empty() {
            kinds.push(local);
        }
    }
    kinds.sort();
    kinds.dedup();
    kinds.into_iter().next()
}

/// Refuse a document that violates the committed shapes.
pub fn validate_turtle(turtle: &str) -> Result<()> {
    validate_turtle_with(&shapes_path(), turtle, "library")
}

/// Refuse a document that violates the committed shapes, read once.
///
/// The same gate as [`validate_turtle`], for a caller that is about to validate
/// several documents in a row and would otherwise re-read the shapes file per
/// entry.
pub fn validate_turtle_with_shape_bytes(turtle: &str, label: &str) -> Result<()> {
    let found = violations(&shapes()?, turtle, label)?;
    if found.is_empty() {
        Ok(())
    } else {
        Err(LibraryError::Shape {
            entry: label.to_string(),
            violations: found,
        })
    }
}

/// Refuse a document that violates the shapes at `shapes_path`.
pub fn validate_turtle_with(shapes_path: &Path, turtle: &str, label: &str) -> Result<()> {
    let shapes = std::fs::read_to_string(shapes_path)
        .map_err(|e| LibraryError::Config(format!("cannot read {}: {e}", shapes_path.display())))?;
    let found = violations(&shapes, turtle, label)?;
    if found.is_empty() {
        Ok(())
    } else {
        Err(LibraryError::Shape {
            entry: label.to_string(),
            violations: found,
        })
    }
}

/// One duplicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    /// The entry being written.
    pub iri: String,
    /// The entry already holding the content.
    pub existing: String,
    /// Their shared content hash.
    pub content_hash: String,
}

/// Find entries whose bodies are already stored under another IRI.
///
/// The first occurrence of a hash wins, so the report is a function of the input
/// order and a rerun over the same rows names the same collisions.
pub fn duplicates(entries: &[(String, String)]) -> Vec<Duplicate> {
    let mut seen: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    let mut found = Vec::new();
    for (iri, hash) in entries {
        match seen.get(hash.as_str()) {
            Some(existing) => found.push(Duplicate {
                iri: iri.clone(),
                existing: (*existing).to_string(),
                content_hash: hash.clone(),
            }),
            None => {
                seen.insert(hash.as_str(), iri.as_str());
            }
        }
    }
    found
}

/// One dangling reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanReport {
    /// The referring entry.
    pub entry: String,
    /// The references that do not resolve, in order.
    pub missing: Vec<String>,
}

/// True when `iri` is not a library entry and does not need to be one.
///
/// The ontology's own terms (`mm:`), the standards the documents use (`rdf:`,
/// `xsd:`, `prov:`), the code namespace (`mmc:`), the skill artefact paths and the
/// bench fixtures a skill names as its test are all legitimate targets that are not
/// entries. Everything else — a `https://metamind.dev/library/...` or
/// `https://metamind.dev/policy/...` IRI — must resolve.
pub fn is_external(iri: &str) -> bool {
    const PREFIXES: [&str; 7] = [
        "https://metamind.dev/ontology#",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
        "http://www.w3.org/2001/XMLSchema#",
        "http://www.w3.org/ns/prov#",
        "https://metamind.dev/code#",
        "https://metamind.dev/bench/",
        "data/library/skills/",
    ];
    PREFIXES.iter().any(|prefix| iri.starts_with(prefix))
}

/// The references that do not resolve, entry by entry.
pub fn orphans(
    references: &[(String, Vec<String>)],
    known: &BTreeSet<String>,
) -> Vec<OrphanReport> {
    let mut found = Vec::new();
    for (entry, targets) in references {
        let mut missing: Vec<String> = targets
            .iter()
            .filter(|target| !is_external(target))
            .filter(|target| !known.contains(*target))
            .cloned()
            .collect();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            found.push(OrphanReport {
                entry: entry.clone(),
                missing,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_shapes_load() {
        let shapes = shapes().unwrap();
        assert!(shapes.contains("mm:LibraryEntryShape"));
        assert!(shapes.contains("mm:TechniqueShape"));
    }

    #[test]
    fn a_document_that_violates_the_shapes_is_refused_with_the_shape() {
        let broken = r#"
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/principle/x> a mm:Principle ;
    mm:title "X" .
"#;
        let error = validate_turtle(broken).unwrap_err();
        assert_eq!(error.code(), "shape");
        assert!(error.to_string().contains("PrincipleShape"), "{error}");
    }

    #[test]
    fn duplicates_are_found_by_hash_not_by_iri() {
        let entries = vec![
            ("a".to_string(), "h1".to_string()),
            ("b".to_string(), "h1".to_string()),
            ("c".to_string(), "h2".to_string()),
        ];
        let found = duplicates(&entries);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].iri, "b");
        assert_eq!(found[0].existing, "a");
    }

    #[test]
    fn the_ontology_namespace_is_not_an_orphan_but_a_library_iri_is() {
        assert!(is_external("https://metamind.dev/ontology#unknownSource"));
        assert!(is_external("http://www.w3.org/ns/prov#Agent"));
        assert!(!is_external("https://metamind.dev/library/case/x"));

        let known: BTreeSet<String> = ["https://metamind.dev/library/case/x".to_string()]
            .into_iter()
            .collect();
        let references = vec![(
            "https://metamind.dev/library/technique/t".to_string(),
            vec![
                "https://metamind.dev/library/case/x".to_string(),
                "https://metamind.dev/library/case/missing".to_string(),
                "https://metamind.dev/ontology#Open".to_string(),
            ],
        )];
        let found = orphans(&references, &known);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].missing,
            vec!["https://metamind.dev/library/case/missing".to_string()]
        );
    }

    #[test]
    fn a_resolved_reference_is_not_an_orphan() {
        let known: BTreeSet<String> = ["a".to_string()].into_iter().collect();
        let references = vec![("e".to_string(), vec!["a".to_string()])];
        assert!(orphans(&references, &known).is_empty());
    }
}
