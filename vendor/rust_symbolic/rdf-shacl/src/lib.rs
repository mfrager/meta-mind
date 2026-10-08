//! `rdf-shacl` — the OWL → SHACL validation pipeline (rdf_interface_update_plan.md
//! §10). Three stages, all native (consistent with the workspace's native
//! SAT/SMT/TPTP backends — the `shacl` crate was not adopted because the
//! fragment is small and a native validator keeps unsupported constructs a
//! hard error, never a silent skip):
//!
//! 1. `owl_check` — structural validation of the ontology hierarchy
//!    (`ValidateOntology`): well-formedness, one class per file, vocabulary
//!    consistency, axiom sanity, and IRI-shape rules.
//! 2. `generate` — deterministic OWL → SHACL shapes graph (`GenerateShacl`),
//!    cached by ontology content hash.
//! 3. `validate` — applies a shapes graph to an instance graph
//!    (`ValidateShacl`): exactly the SHACL-core fragment the generator emits;
//!    any other construct is a hard error.

pub mod generate;
pub mod owl_check;
pub mod validate;

pub use generate::generate_shacl;
pub use owl_check::{check_ontology, OntologyReport};
pub use validate::{validate, ShaclReport, ShaclViolation, Shapes};
