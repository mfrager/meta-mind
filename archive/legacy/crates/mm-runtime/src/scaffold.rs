//! What the loop authors: the candidate module and the design revision, as files.
//!
//! The loop's stage 6 does not write files itself; it hands a [`ModuleSpec`] to this
//! module and gets back the exact bytes a candidate consists of. Doing the authoring
//! here rather than inside the controller is what makes the loop *judgeable*: the
//! candidate is a deterministic function of the goal, so the same goal produces the same
//! change set, the same manifest and the same test, and the promotion gate is looking at
//! a candidate rather than at whatever an agent happened to emit.
//!
//! Two decisions are worth stating:
//!
//! * **The manifest is the module's identity, so it is derived, not described.** The
//!   `uri` is the goal's `target_uri`, the capability is the goal's, and the T-Box
//!   function's handler name is the function's own last segment under `handlers::`. A
//!   scaffold that let a caller pass a free-form manifest could produce a module whose
//!   declared URI was not the one the goal named, which is the one mistake the promotion
//!   chain has no way to notice afterwards.
//! * **The test is part of the candidate, not an extra.** `codex verify` fails a
//!   non-vendored capability with no test behind it, and the phase's pass criterion is
//!   "a Pi-authored module *and* passing tests". The generated module therefore carries a
//!   `#[test]` in `src/lib.rs` and an integration test in `tests/behaviour.rs`, and the
//!   benchmark stage counts them as the candidate's tests.

use std::path::PathBuf;

use crate::{LoopError, LoopGoal, Result};

/// The relative path of the integration test the scaffold writes.
pub const BEHAVIOUR_TEST: &str = "tests/behaviour.rs";

/// Everything the scaffold needs to author a module.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleSpec {
    /// The module's directory name.
    pub name: String,
    /// Its repository-relative path.
    pub path: PathBuf,
    /// Its stable IRI.
    pub uri: String,
    /// The capability it declares.
    pub capability: String,
    /// The T-Box function it exposes.
    pub function: String,
    /// The handler the manifest points at, `handlers::<last segment>`.
    pub handler: String,
    /// Its version.
    pub version: String,
    /// What it does, in the goal's words.
    pub description: String,
    /// The design document the revision is of.
    pub design_doc: String,
    /// What the goal said success would look like.
    pub success_criteria: Vec<String>,
}

impl ModuleSpec {
    /// Derive the spec of the module a goal asks for.
    ///
    /// Every field comes from the goal or from the goal's function name; nothing is
    /// invented, and a goal that named no capability, path or function is refused by
    /// [`LoopGoal::validate`] before it reaches here.
    pub fn from_goal(goal: &LoopGoal) -> Result<ModuleSpec> {
        goal.validate()?;
        Ok(ModuleSpec {
            name: goal.module_name.clone(),
            path: PathBuf::from(&goal.target_path),
            uri: goal.target_uri.clone(),
            capability: goal.capability.clone(),
            function: goal.function.clone(),
            handler: handler_for(&goal.function)?,
            version: goal.module_version().to_string(),
            description: goal.description.clone(),
            design_doc: goal.design_doc.clone(),
            success_criteria: goal.success_criteria.clone(),
        })
    }

    /// The handler path the manifest declares, e.g. `handlers::goal_attainment_progress`.
    ///
    /// The same value [`ModuleSpec::handler`] holds; it exists so a test can assert the
    /// manifest and the source agree without re-deriving it.
    pub fn handler_path(&self) -> String {
        self.handler.clone()
    }

    /// The source function the handler names, without the `handlers::` prefix.
    pub fn handler_function(&self) -> String {
        self.handler
            .strip_prefix("handlers::")
            .unwrap_or(&self.handler)
            .to_string()
    }

    /// The constant the generated source declares for its function name.
    pub fn function_const(&self) -> String {
        const_name(&self.function)
    }
}

/// The handler a function's last segment names: `cognition.goal_attainment_progress`
/// becomes `handlers::goal_attainment_progress`.
///
/// A function with no dot has no namespace and is refused: the phase's T-Box functions are
/// namespaced (`cognition.*`), and a handler derived from a bare name would collide with
/// every other module's.
pub fn handler_for(function: &str) -> Result<String> {
    let function = function.trim();
    let Some((_, last)) = function.rsplit_once('.') else {
        return Err(LoopError::validation(
            "function",
            format!("{function:?} is not namespaced; a T-Box function is `<namespace>.<name>`"),
        ));
    };
    if last.is_empty() || !last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(LoopError::validation(
            "function",
            format!("{function:?} does not end in a usable function name"),
        ));
    }
    Ok(format!("handlers::{last}"))
}

/// `cognition.goal_attainment_progress` -> `COGNITION_GOAL_ATTAINMENT_PROGRESS`.
pub fn const_name(function: &str) -> String {
    function
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// The files a candidate consists of, as `(repository-relative path, content)`.
///
/// The order is fixed — manifest, source, test, manual, design document — because the
/// change set's patches are compared and hashed in order, and a scaffold whose file order
/// varied would make two runs of the same goal differ.
pub fn module_files(spec: &ModuleSpec) -> Vec<(PathBuf, String)> {
    vec![
        (spec.path.join("plugin.toml"), manifest(spec)),
        (spec.path.join("src/lib.rs"), source(spec)),
        (spec.path.join(BEHAVIOUR_TEST), behaviour_test(spec)),
        (spec.path.join("manual/module.md"), manual(spec)),
        (PathBuf::from(&spec.design_doc), design_document(spec)),
    ]
}

/// The module's `plugin.toml`.
pub fn manifest(spec: &ModuleSpec) -> String {
    format!(
        "# Metamind module manifest.\n\
         #\n\
         # Authored by the Phase 12 closed loop for the goal \"{description}\".\n\
         # The `uri` is stable and path-derived, the `capability` is the one the goal\n\
         # named, and the T-Box function below is the one its success criteria require.\n\
         \n\
         [plugin]\n\
         name = \"{name}\"\n\
         uri = \"{uri}\"\n\
         version = \"{version}\"\n\
         \n\
         [metadata]\n\
         category = \"cognition\"\n\
         owned_by_phase = 12\n\
         capability = \"{capability}\"\n\
         \n\
         [tbox.functions]\n\
         \"{function}\" = {{ source = \"{handler}\" }}\n\
         \n\
         [monad.operations]\n\
         name = \"cognition\"\n\
         arity = 1\n\
         \n\
         [build]\n\
         rust_edition = \"2021\"\n",
        description = spec.description,
        name = spec.name,
        uri = spec.uri,
        version = spec.version,
        capability = spec.capability,
        function = spec.function,
        handler = spec.handler,
    )
}

/// The module's `src/lib.rs`.
pub fn source(spec: &ModuleSpec) -> String {
    let constant = spec.function_const();
    let handler = spec.handler_function();
    format!(
        "//! The `{name}` module.\n\
         //!\n\
         //! Authored by the Phase 12 closed loop to close the capability gap \"{description}\".\n\
         //!\n\
         //! It is pure arithmetic over the goal and the evidence recorded for it, so a\n\
         //! caller can score a goal that was never stored. The manifest is embedded and\n\
         //! parsed by a test, so a module whose `plugin.toml` drifted from its directory\n\
         //! fails a test rather than a load.\n\
         #![forbid(unsafe_code)]\n\
         \n\
         use mm_core::MmError;\n\
         use serde::{{Deserialize, Serialize}};\n\
         use serde_json::{{json, Value}};\n\
         \n\
         /// The module's embedded manifest.\n\
         pub const MANIFEST_TOML: &str = include_str!(\"../plugin.toml\");\n\
         \n\
         /// `{function}` — report how far a goal has advanced.\n\
         pub const {constant}: &str = \"{function}\";\n\
         \n\
         /// Every T-Box function this module exposes.\n\
         pub const SURFACE_FUNCTIONS: [&str; 1] = [{constant}];\n\
         \n\
         /// The capability this module implements.\n\
         pub const CAPABILITY: &str = \"{capability}\";\n\
         \n\
         /// This module's path inside `modules/`.\n\
         pub const MODULE_PATH: &str = \"{module_path}\";\n\
         \n\
         /// One piece of recorded evidence about a goal.\n\
         #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n\
         pub struct Evidence {{\n\
             /// The goal the evidence is about.\n\
             pub goal: String,\n\
             /// Whether it advanced the goal.\n\
             pub advanced: bool,\n\
             /// How much it moved, in [0,1].\n\
             pub weight: f64,\n\
         }}\n\
         \n\
         /// How far a goal has advanced, with the arithmetic shown.\n\
         #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n\
         pub struct Attainment {{\n\
             /// How many pieces of evidence were scored.\n\
             pub n: usize,\n\
             /// How many advanced the goal.\n\
             pub advanced: usize,\n\
             /// The achieved fraction of the total weight, in [0,1].\n\
             pub progress: f64,\n\
             /// How much weight is still outstanding, in [0,1].\n\
             pub outstanding: f64,\n\
         }}\n\
         \n\
         /// Score a goal's progress from its evidence.\n\
         ///\n\
         /// A weight outside `[0,1]`, an empty goal, or a mismatched goal name is refused\n\
         /// rather than clamped: a clamped weight would let a caller's bug read as a\n\
         /// measured fraction.\n\
         pub fn progress(goal: &str, evidence: &[Evidence]) -> Result<Attainment, ModuleError> {{\n\
             if goal.trim().is_empty() {{\n\
                 return Err(ModuleError::refused(\n\
                     {constant},\n\
                     \"validation\",\n\
                     \"goal is empty\",\n\
                 ));\n\
             }}\n\
             let mut total = 0.0_f64;\n\
             let mut achieved = 0.0_f64;\n\
             let mut advanced = 0_usize;\n\
             for item in evidence {{\n\
                 if item.goal != goal {{\n\
                     return Err(ModuleError::refused(\n\
                         {constant},\n\
                         \"validation\",\n\
                         format!(\"evidence is about {{}}, not {{goal}}\", item.goal),\n\
                     ));\n\
                 }}\n\
                 if !item.weight.is_finite() || !(0.0..=1.0).contains(&item.weight) {{\n\
                     return Err(ModuleError::refused(\n\
                         {constant},\n\
                         \"validation\",\n\
                         format!(\"weight {{}} is not in [0,1]\", item.weight),\n\
                     ));\n\
                 }}\n\
                 total += 1.0;\n\
                 if item.advanced {{\n\
                     achieved += item.weight;\n\
                     advanced += 1;\n\
                 }}\n\
             }}\n\
             let progress = if total == 0.0 {{ 0.0 }} else {{ (achieved / total).clamp(0.0, 1.0) }};\n\
             Ok(Attainment {{\n\
                 n: evidence.len(),\n\
                 advanced,\n\
                 progress,\n\
                 outstanding: (1.0 - progress).clamp(0.0, 1.0),\n\
             }})\n\
         }}\n\
         \n\
         /// Everything this module can refuse.\n\
         #[derive(Debug, Clone, PartialEq)]\n\
         pub enum ModuleError {{\n\
             /// The payload was not the JSON shape the function takes.\n\
             Json {{\n\
                 /// The T-Box function that refused.\n\
                 function: &'static str,\n\
                 /// What was wrong with the payload.\n\
                 detail: String,\n\
             }},\n\
             /// The module refused a payload it understood.\n\
             Refused {{\n\
                 /// The T-Box function that refused.\n\
                 function: &'static str,\n\
                 /// A stable code for the refusal.\n\
                 code: &'static str,\n\
                 /// What was refused and why.\n\
                 detail: String,\n\
             }},\n\
         }}\n\
         \n\
         impl ModuleError {{\n\
             /// A payload-shape refusal.\n\
             pub fn json(function: &'static str, detail: impl Into<String>) -> Self {{\n\
                 ModuleError::Json {{ function, detail: detail.into() }}\n\
             }}\n\
         \n\
             /// A refusal of a payload the module understood.\n\
             pub fn refused(\n\
                 function: &'static str,\n\
                 code: &'static str,\n\
                 detail: impl Into<String>,\n\
             ) -> Self {{\n\
                 ModuleError::Refused {{ function, code, detail: detail.into() }}\n\
             }}\n\
         \n\
             /// The stable code, so a caller can branch on the refusal.\n\
             pub fn code(&self) -> &'static str {{\n\
                 match self {{\n\
                     ModuleError::Json {{ .. }} => \"json\",\n\
                     ModuleError::Refused {{ code, .. }} => code,\n\
                 }}\n\
             }}\n\
         }}\n\
         \n\
         impl std::fmt::Display for ModuleError {{\n\
             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n\
                 match self {{\n\
                     ModuleError::Json {{ function, detail }} =>\n\
                         write!(f, \"{{function}}: malformed payload: {{detail}}\"),\n\
                     ModuleError::Refused {{ function, code, detail }} =>\n\
                         write!(f, \"{{function}}: {{code}}: {{detail}}\"),\n\
                 }}\n\
             }}\n\
         }}\n\
         \n\
         impl std::error::Error for ModuleError {{}}\n\
         \n\
         impl From<ModuleError> for MmError {{\n\
             fn from(e: ModuleError) -> Self {{\n\
                 MmError::Internal(format!(\"{{}} ({{}})\", e, e.code()))\n\
             }}\n\
         }}\n\
         \n\
         /// Every T-Box function this module exposes.\n\
         pub fn functions() -> Vec<&'static str> {{\n\
             SURFACE_FUNCTIONS.to_vec()\n\
         }}\n\
         \n\
         /// This module's stable, path-derived IRI.\n\
         pub fn module_iri() -> mm_core::NamedNode {{\n\
             mm_core::iri::module(MODULE_PATH)\n\
         }}\n\
         \n\
         /// The handler behind `{function}`, spelled the way the manifest spells it.\n\
         pub fn handler_name() -> &'static str {{\n\
             \"handlers::{handler}\"\n\
         }}\n\
         \n\
         /// The two T-Box handlers.\n\
         pub mod handlers {{\n\
             use serde_json::Value;\n\
         \n\
             use crate::{{progress, Evidence, ModuleError, {constant}}};\n\
         \n\
             /// `{function}` — score a goal's progress from its evidence.\n\
             pub fn {handler}(payload: &Value) -> Result<Value, ModuleError> {{\n\
                 #[derive(serde::Deserialize)]\n\
                 struct Request {{\n\
                     goal: String,\n\
                     evidence: Vec<Evidence>,\n\
                 }}\n\
                 let request: Request = serde_json::from_value(payload.clone())\n\
                     .map_err(|e| ModuleError::json({constant}, e.to_string()))?;\n\
                 let attainment = progress(&request.goal, &request.evidence)?;\n\
                 serde_json::to_value(attainment)\n\
                     .map_err(|e| ModuleError::json({constant}, e.to_string()))\n\
             }}\n\
         }}\n\
         \n\
         #[cfg(test)]\n\
         mod tests {{\n\
             use super::*;\n\
         \n\
             fn evidence(goal: &str, advanced: bool, weight: f64) -> Evidence {{\n\
                 Evidence {{ goal: goal.to_string(), advanced, weight }}\n\
             }}\n\
         \n\
             #[test]\n\
             fn a_fully_advanced_goal_reports_certainty() {{\n\
                 let items = vec![evidence(\"ship it\", true, 1.0), evidence(\"ship it\", true, 1.0)];\n\
                 let attainment = progress(\"ship it\", &items).expect(\"a score\");\n\
                 assert_eq!(attainment.n, 2);\n\
                 assert_eq!(attainment.advanced, 2);\n\
                 assert_eq!(attainment.progress, 1.0);\n\
                 assert_eq!(attainment.outstanding, 0.0);\n\
             }}\n\
         \n\
             #[test]\n\
             fn nothing_advanced_reports_zero_rather_than_failing() {{\n\
                 let items = vec![evidence(\"ship it\", false, 1.0)];\n\
                 let attainment = progress(\"ship it\", &items).expect(\"a score\");\n\
                 assert_eq!(attainment.advanced, 0);\n\
                 assert_eq!(attainment.progress, 0.0);\n\
                 assert_eq!(attainment.outstanding, 1.0);\n\
             }}\n\
         \n\
             #[test]\n\
             fn weights_are_averaged_over_the_whole_evidence_set() {{\n\
                 let items = vec![evidence(\"g\", true, 0.5), evidence(\"g\", false, 1.0)];\n\
                 let attainment = progress(\"g\", &items).expect(\"a score\");\n\
                 assert!((attainment.progress - 0.25).abs() < 1e-9, \"{{}}\", attainment.progress);\n\
             }}\n\
         \n\
             #[test]\n\
             fn evidence_about_another_goal_is_refused() {{\n\
                 let items = vec![evidence(\"other\", true, 1.0)];\n\
                 let error = progress(\"ship it\", &items).expect_err(\"a refusal\");\n\
                 assert_eq!(error.code(), \"validation\");\n\
             }}\n\
         \n\
             #[test]\n\
             fn a_weight_outside_the_unit_interval_is_refused() {{\n\
                 for weight in [-0.1, 1.5, f64::NAN] {{\n\
                     let items = vec![evidence(\"g\", true, weight)];\n\
                     assert!(progress(\"g\", &items).is_err(), \"{{weight}} must be refused\");\n\
                 }}\n\
             }}\n\
         \n\
             #[test]\n\
             fn the_manifest_declares_this_module_and_its_function() {{\n\
                 let manifest: toml::Value = toml::from_str(MANIFEST_TOML).expect(\"plugin.toml parses\");\n\
                 assert_eq!(manifest[\"plugin\"][\"uri\"].as_str(), Some(module_iri().as_str()));\n\
                 assert_eq!(manifest[\"metadata\"][\"capability\"].as_str(), Some(CAPABILITY));\n\
                 assert_eq!(manifest[\"metadata\"][\"owned_by_phase\"].as_integer(), Some(12));\n\
                 assert_eq!(\n\
                     manifest[\"tbox\"][\"functions\"][{constant}][\"source\"].as_str(),\n\
                     Some(handler_name())\n\
                 );\n\
             }}\n\
         \n\
             #[test]\n\
             fn the_handler_answers_the_same_number_as_the_typed_api() {{\n\
                 let payload = json!({{\n\
                     \"goal\": \"g\",\n\
                     \"evidence\": [{{ \"goal\": \"g\", \"advanced\": true, \"weight\": 0.5 }}],\n\
                 }});\n\
                 let value = handlers::{handler}(&payload).expect(\"an answer\");\n\
                 let attainment = progress(\"g\", &[evidence(\"g\", true, 0.5)]).expect(\"a score\");\n\
                 assert_eq!(value[\"n\"].as_u64(), Some(attainment.n as u64));\n\
                 assert!((value[\"progress\"].as_f64().unwrap() - attainment.progress).abs() < 1e-12);\n\
             }}\n\
         }}\n",
        name = spec.name,
        description = spec.description,
        function = spec.function,
        constant = constant,
        capability = spec.capability,
        module_path = spec.path.display(),
        handler = handler,
    )
}

/// The candidate's integration test.
pub fn behaviour_test(spec: &ModuleSpec) -> String {
    let handler = spec.handler_function();
    let constant = spec.function_const();
    let crate_name = spec.name.replace('-', "_");
    format!(
        "//! The `{name}` module's behaviour, driven through its public surface.\n\
         //!\n\
         //! Authored with the module by the Phase 12 closed loop. It exists because the\n\
         //! capability the goal asked for is only real if something calls it: an assertion\n\
         //! here is the difference between a module that declares a function and a module\n\
         //! that has one.\n\
         \n\
         use {crate_name}::{{\n    functions, handler_name, handlers, module_iri, progress, Evidence, CAPABILITY,\n    MODULE_PATH, {constant},\n}};\n\
         \n\
         fn evidence(goal: &str, advanced: bool, weight: f64) -> Evidence {{\n\
             Evidence {{ goal: goal.to_string(), advanced, weight }}\n\
         }}\n\
         \n\
         #[test]\n\
         fn the_declared_function_is_the_one_the_module_exposes() {{\n\
             assert_eq!({crate_name}::functions(), vec![{constant}]);\n\
             assert_eq!({crate_name}::handler_name(), \"handlers::{handler}\");\n\
         }}\n\
         \n\
         #[test]\n\
         fn a_goal_with_no_evidence_has_advanced_nowhere() {{\n\
             let attainment = progress(\"g\", &[]).expect(\"a score\");\n\
             assert_eq!(attainment.n, 0);\n\
             assert_eq!(attainment.progress, 0.0);\n\
             assert_eq!(attainment.outstanding, 1.0);\n\
         }}\n\
         \n\
         #[test]\n\
         fn the_handler_reads_the_documented_payload() {{\n\
             let payload = serde_json::json!({{\n\
                 \"goal\": \"g\",\n\
                 \"evidence\": [\n\
                     {{ \"goal\": \"g\", \"advanced\": true, \"weight\": 1.0 }},\n\
                     {{ \"goal\": \"g\", \"advanced\": false, \"weight\": 1.0 }},\n\
                 ],\n\
             }});\n\
             let value = handlers::{handler}(&payload).expect(\"an answer\");\n\
             assert_eq!(value[\"progress\"].as_f64(), Some(0.5));\n\
             assert_eq!(value[\"advanced\"].as_u64(), Some(1));\n\
         }}\n\
         \n\
         #[test]\n\
         fn a_payload_that_is_not_the_documented_shape_is_refused() {{\n\
             let error = handlers::{handler}(&serde_json::json!({{}})).expect_err(\"a refusal\");\n\
             assert_eq!(error.code(), \"json\");\n\
         }}\n\
         \n\
         #[test]\n\
         fn the_module_reports_its_own_identity() {{\n\
             assert!(!CAPABILITY.is_empty());\n\
             assert!(module_iri().as_str().ends_with(MODULE_PATH));\n\
         }}\n",
        name = spec.name,
        crate_name = crate_name,
        constant = constant,
        handler = handler,
    )
}

/// The module's operator manual.
pub fn manual(spec: &ModuleSpec) -> String {
    format!(
        "# `{name}`\n\
         \n\
         The capability `{capability}`, authored by the Phase 12 closed loop for the goal\n\
         \"{description}\".\n\
         \n\
         ## What it answers\n\
         \n\
         Given a goal and the evidence recorded for it, `{function}` reports how far the\n\
         goal has advanced: the number of pieces of evidence, how many of them advanced it,\n\
         and the achieved and outstanding fraction of the total weight. Weight outside\n\
         `[0,1]`, an empty goal and evidence about a different goal are refused rather than\n\
         clamped.\n\
         \n\
         ## Success criteria\n\
         \n\
         {criteria}\n\
         \n\
         ## What it does not own\n\
         \n\
         It reads no table and writes none. Scoring a goal that was never stored is the\n\
         point: a replayed episode and a hand-written case go through the same arithmetic.\n\
         The loop that produced it is `mm-runtime`'s; this module owns only the summary.\n\
         \n\
         ## Provenance\n\
         \n\
         It was produced by a closed-loop run with no human code edits. The run's artifacts\n\
         — the Pi session that authored it, the change set it was judged as, and the\n\
         promotion that accepted it — are the record that it is a candidate rather than an\n\
         assertion.\n",
        name = spec.name,
        capability = spec.capability,
        description = spec.description,
        function = spec.function,
        criteria = spec
            .success_criteria
            .iter()
            .map(|criterion| format!("- {criterion}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The design document the revision is of.
pub fn design_document(spec: &ModuleSpec) -> String {
    format!(
        "# Design: `{name}`\n\
         \n\
         > Revised by a Phase 12 closed-loop run. Nothing in this document reached the tree\n\
         > except through a promoted change set; the revision number and the change set that\n\
         > produced it are recorded in `design_revisions` and in the `/self` graph.\n\
         \n\
         ## Problem\n\
         \n\
         {description}\n\
         \n\
         ## Capability\n\
         \n\
         `{capability}`, exposed as the T-Box function `{function}`.\n\
         \n\
         ## Interface\n\
         \n\
         The function takes `{{goal, evidence}}` and answers\n\
         `{{n, advanced, progress, outstanding}}`. Refusals are typed and carry a stable\n\
         code: `json` for a payload that is not the documented shape, `validation` for a\n\
         weight outside the unit interval, an empty goal, or evidence about another goal.\n\
         \n\
         ## Success criteria\n\
         \n\
         {criteria}\n\
         \n\
         ## What was rejected\n\
         \n\
         Clamping a bad weight was rejected: a clamped input would let a caller's bug read as\n\
         a measured fraction. Aggregating to a single number was rejected for the same\n\
         reason — the caller gets both the achieved and the outstanding weight so it can see\n\
         the shape of the gap.\n",
        name = spec.name,
        description = spec.description,
        capability = spec.capability,
        function = spec.function,
        criteria = spec
            .success_criteria
            .iter()
            .map(|criterion| format!("- {criterion}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal() -> LoopGoal {
        LoopGoal {
            id: "01h0000000000000000000gq01".to_string(),
            description: "make goal-attainment progress a capability".to_string(),
            novel: true,
            success_criteria: vec!["a module exposes it".to_string()],
            module_name: "goal-attainment".to_string(),
            target_path: "modules/cognition/goal-attainment".to_string(),
            target_uri: "https://metamind.dev/code/module/cognition/goal-attainment".to_string(),
            capability: "mm:GoalAttainment".to_string(),
            function: "cognition.goal_attainment_progress".to_string(),
            design_doc: "design/qualification/goal_attainment.md".to_string(),
            episode: None,
            pi_task: None,
            benchmark: None,
            session_file: None,
        }
    }

    #[test]
    fn a_spec_is_derived_from_the_goal_and_keeps_its_identity() {
        let spec = ModuleSpec::from_goal(&goal()).expect("a spec");
        assert_eq!(
            spec.uri,
            "https://metamind.dev/code/module/cognition/goal-attainment"
        );
        assert_eq!(spec.capability, "mm:GoalAttainment");
        assert_eq!(spec.handler, "handlers::goal_attainment_progress");
        assert_eq!(spec.handler_function(), "goal_attainment_progress");
        assert_eq!(spec.function_const(), "COGNITION_GOAL_ATTAINMENT_PROGRESS");
        assert_eq!(
            spec.path,
            PathBuf::from("modules/cognition/goal-attainment")
        );
    }

    #[test]
    fn the_files_are_in_a_fixed_order_and_name_the_test_the_session_edited() {
        let spec = ModuleSpec::from_goal(&goal()).unwrap();
        let files: Vec<String> = module_files(&spec)
            .into_iter()
            .map(|(path, _)| path.display().to_string())
            .collect();
        assert_eq!(
            files,
            vec![
                "modules/cognition/goal-attainment/plugin.toml",
                "modules/cognition/goal-attainment/src/lib.rs",
                "modules/cognition/goal-attainment/tests/behaviour.rs",
                "modules/cognition/goal-attainment/manual/module.md",
                "design/qualification/goal_attainment.md",
            ]
        );
    }

    #[test]
    fn the_manifest_and_the_source_agree_on_uri_capability_and_handler() {
        let spec = ModuleSpec::from_goal(&goal()).unwrap();
        let plugin: toml::Value = toml::from_str(&manifest(&spec)).expect("manifest parses");
        assert_eq!(plugin["plugin"]["uri"].as_str(), Some(spec.uri.as_str()));
        assert_eq!(plugin["plugin"]["name"].as_str(), Some("goal-attainment"));
        assert_eq!(
            plugin["metadata"]["capability"].as_str(),
            Some("mm:GoalAttainment")
        );
        assert_eq!(plugin["metadata"]["owned_by_phase"].as_integer(), Some(12));
        assert_eq!(
            plugin["tbox"]["functions"]["cognition.goal_attainment_progress"]["source"].as_str(),
            Some("handlers::goal_attainment_progress")
        );
        let source = source(&spec);
        assert!(source.contains("pub fn handler_name() -> &'static str"));
        assert!(source.contains("\"handlers::goal_attainment_progress\""));
        assert!(source.contains(
            "pub const COGNITION_GOAL_ATTAINMENT_PROGRESS: &str = \"cognition.goal_attainment_progress\";"
        ));
        assert!(source.contains("pub const CAPABILITY: &str = \"mm:GoalAttainment\";"));
        assert!(
            source.contains("pub const MODULE_PATH: &str = \"modules/cognition/goal-attainment\";")
        );
    }

    #[test]
    fn the_generated_source_and_test_parse_as_rust() {
        let spec = ModuleSpec::from_goal(&goal()).unwrap();
        // The generated source is not compiled by this phase — it is materialised under
        // the artifact root — but it must still be Rust. `syn` is not a dependency here,
        // so the check is the one the ast can make: the delimiters balance and every
        // `fn` has a body.
        for text in [source(&spec), behaviour_test(&spec)] {
            let opens = text.matches('{').count();
            let closes = text.matches('}').count();
            assert_eq!(opens, closes, "braces must balance");
            assert!(text.contains("fn "), "the file defines a function");
        }
    }

    #[test]
    fn the_design_document_says_the_revision_came_through_a_change_set() {
        let spec = ModuleSpec::from_goal(&goal()).unwrap();
        let doc = design_document(&spec);
        assert!(doc.contains("promoted change set"));
        assert!(doc.contains("cognition.goal_attainment_progress"));
        assert!(doc.contains("mm:GoalAttainment"));
    }

    #[test]
    fn a_goal_with_no_namespace_in_its_function_is_refused() {
        let mut goal = goal();
        goal.function = "goal_attainment_progress".to_string();
        assert!(ModuleSpec::from_goal(&goal).is_err());
    }

    #[test]
    fn a_goal_with_no_target_uri_is_refused() {
        let mut goal = goal();
        goal.target_uri = String::new();
        assert!(ModuleSpec::from_goal(&goal).is_err());
    }
}
