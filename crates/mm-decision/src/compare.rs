//! Comparison integrity (design §20): two things may only be compared when the
//! comparison is well-formed, and "well-formed" is decided here rather than
//! assumed by whoever wants the answer.
//!
//! Most decisions that look wrong in hindsight are not reasoning failures at all.
//! They are *invalid comparisons*: a latency measured under one load compared with
//! a latency measured under another, a cost in one currency against a cost in
//! another, a benchmark on one version of a model against a benchmark on its
//! successor. A system that will happily answer "which is better" for any pair is
//! not more capable than one that refuses; it is less trustworthy, because the
//! refusal is the information. So this module is a **refusal**, not a scoring
//! function: [`check_contract`] either says the pair is `Valid` — and then
//! [`normalize`] puts both sides into the baseline's units — or it names exactly
//! which premise failed.
//!
//! Three verdict classes, and the distinction between them is the point:
//!
//! * **`Rejected`** — the premise is falsified. The two contracts are not about the
//!   same question: different objective, different object class, units that are
//!   dimensionally inconsistent (a mass against a duration), or no evidence at
//!   all. Answering anyway would be answering a different question.
//! * **`NonComparable`** — the same question under conditions that do not line up:
//!   disjoint timeframes, different operating conditions, different definitions of
//!   a shared term, different scope, a declared dimension on one side only, or a
//!   missing value. The comparison is meaningful *in principle* and cannot be made
//!   from these two contracts.
//! * **`Valid`** — every condition agrees, and the candidate's values can be
//!   expressed in the baseline's units.
//!
//! A mismatch is never coerced into a comparison, and a value is never converted
//! across dimensions. `normalize` converts only *within* a dimension, using the
//! declared `to_base` ratios, and records every conversion it applied so a reader
//! can see what happened to the numbers.
//!
//! Ordering: violations are emitted in the fixed order of [`VIOLATION_CODES`] and
//! then by field name, so the same pair always produces the same report.

use std::collections::{BTreeMap, BTreeSet};

use mm_core::Ulid;
use serde::{Deserialize, Serialize};

use crate::error::{DecisionError, Result};

/// One object under comparison, by name. Names rather than indices because the
/// report is read by a human and cited by a later phase.
pub type ObjRef = String;

/// A quantity the comparison is made on: for example `latency`, measured in `ms`,
/// where lower is better.
///
/// `unit` is a *name* resolved against the contract's [`Unit`] table; the table is
/// what supplies the dimension and the scale factor, so two contracts can name the
/// same unit differently without the comparison silently succeeding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    /// The quantity's name.
    pub name: String,
    /// The unit it is measured in, if the quantity has one.
    pub unit: Option<String>,
    /// The direction that counts as better. Carried for the report; it is not a
    /// comparability condition, because two contracts comparing the same
    /// dimension in opposite directions is a preference difference, not an
    /// invalid comparison.
    pub higher_is_better: bool,
}

/// A unit and the dimension it belongs to.
///
/// `to_base` is the factor that converts a value *in this unit* into the
/// dimension's base unit. Two units are compatible exactly when their `dimension`
/// fields match, and that is the only thing that licenses a conversion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    /// The unit's name, as a [`Dimension`] refers to it.
    pub name: String,
    /// The dimension this unit measures, e.g. `time`, `mass`, `currency`.
    pub dimension: String,
    /// The multiplier into the dimension's base unit.
    pub to_base: f64,
}

impl Unit {
    /// Refuse a unit that cannot convert anything.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(DecisionError::validation("unit.name", "is empty"));
        }
        if self.dimension.trim().is_empty() {
            return Err(DecisionError::validation(
                "unit.dimension",
                format!("unit {:?} declares no dimension", self.name),
            ));
        }
        if !self.to_base.is_finite() || self.to_base <= 0.0 {
            return Err(DecisionError::validation(
                "unit.to_base",
                format!(
                    "unit {:?} needs a finite positive factor, got {}",
                    self.name, self.to_base
                ),
            ));
        }
        Ok(())
    }
}

/// The window a measurement covers.
///
/// `start` and `end` are treated as **opaque, lexicographically ordered
/// instants** rather than parsed timestamps. Comparability only ever asks whether
/// two windows overlap and which contains the other, and both of those questions
/// are answered correctly by a total order on the rendered instants. Parsing would
/// add a second time representation to a crate whose job is deciding things, and a
/// parse failure would then have to be a third verdict class. Normalizing to
/// RFC3339 with a fixed width is the caller's job; a caller that does not is
/// caught by the fixtures, not by a silent misread here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeframe {
    /// The window's first instant.
    pub start: String,
    /// The window's last instant.
    pub end: String,
}

impl Timeframe {
    /// A stable rendering, used in the normalized comparison.
    pub fn canonical(&self) -> String {
        format!("{}..{}", self.start, self.end)
    }

    /// True when the two windows share any instant.
    pub fn overlaps(&self, other: &Timeframe) -> bool {
        self.start.as_str() <= other.end.as_str() && other.start.as_str() <= self.end.as_str()
    }

    /// True when `self` contains `other` entirely.
    pub fn covers(&self, other: &Timeframe) -> bool {
        self.start.as_str() <= other.start.as_str() && other.end.as_str() <= self.end.as_str()
    }

    /// The overlap of two windows, when they overlap at all.
    fn intersection(&self, other: &Timeframe) -> Option<Timeframe> {
        if !self.overlaps(other) {
            return None;
        }
        // `.clone()` on the shared side: `Ord::max`/`min` take `self` by value, and
        // neither window may be consumed by reading the overlap out of them.
        Some(Timeframe {
            start: self.start.clone().max(other.start.clone()),
            end: self.end.clone().min(other.end.clone()),
        })
    }
}

/// One operating condition the measurement was taken under, e.g. `load = peak`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    /// The condition's name.
    pub name: String,
    /// The value it held.
    pub value: String,
}

/// Everything that must line up before two objects may be compared.
///
/// `values` is keyed by dimension name: it is the measurement, and keeping it on
/// the contract rather than beside it means a contract without values is a
/// contract that cannot be normalized rather than a contract whose values were
/// forgotten at the call site.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonContract {
    /// What the comparison is for.
    pub objective: String,
    /// The class of thing being compared. Two different classes never compare,
    /// whatever the objective says.
    pub object_class: String,
    /// The objects measured.
    pub objects: Vec<ObjRef>,
    /// The quantities measured.
    pub dimensions: Vec<Dimension>,
    /// The unit table the dimensions resolve against.
    pub units: Vec<Unit>,
    /// The window the measurements cover.
    pub timeframe: Timeframe,
    /// The operating conditions they were taken under.
    pub conditions: Vec<Condition>,
    /// Hard limits the measurement was subject to.
    pub constraints: Vec<String>,
    /// Definitions of the terms the measurement depends on.
    pub definitions: BTreeMap<String, String>,
    /// Claim ULIDs supporting the measurement.
    pub evidence: Vec<Ulid>,
    /// The measurement, keyed by dimension name.
    pub values: BTreeMap<String, f64>,
}

impl ComparisonContract {
    /// Refuse a contract that does not describe a measurement.
    pub fn validate(&self) -> Result<()> {
        if phrase(&self.objective).is_empty() {
            return Err(DecisionError::validation(
                "comparison.objective",
                "is empty",
            ));
        }
        if phrase(&self.object_class).is_empty() {
            return Err(DecisionError::validation(
                "comparison.object_class",
                "is empty",
            ));
        }
        if self.objects.is_empty() {
            return Err(DecisionError::validation(
                "comparison.objects",
                "a contract must name at least one object",
            ));
        }
        let mut seen = BTreeSet::new();
        for object in &self.objects {
            if object.trim().is_empty() {
                return Err(DecisionError::validation(
                    "comparison.objects",
                    "an object name is empty",
                ));
            }
            if !seen.insert(object.clone()) {
                return Err(DecisionError::validation(
                    "comparison.objects",
                    format!("{object:?} is named twice"),
                ));
            }
        }
        for unit in &self.units {
            unit.validate()?;
        }
        Ok(())
    }

    /// A stable rendering of the contract.
    pub fn canonical(&self) -> String {
        let dimensions: Vec<String> = self
            .dimensions
            .iter()
            .map(|d| match &d.unit {
                Some(unit) => format!("{}[{}]", d.name, unit),
                None => d.name.clone(),
            })
            .collect();
        let units: Vec<String> = self
            .units
            .iter()
            .map(|u| format!("{}:{}:{}", u.name, u.dimension, u.to_base))
            .collect();
        let conditions: Vec<String> = self
            .conditions
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        let values: Vec<String> = self
            .values
            .iter()
            .map(|(k, v)| format!("{k}={v:.9}"))
            .collect();
        let definitions: Vec<String> = self
            .definitions
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!(
            "objective={}\nobject_class={}\nobjects={}\ndimensions={}\nunits={}\n\
             timeframe={}\nconditions={}\nconstraints={}\ndefinitions={}\nevidence={}\nvalues={}",
            phrase(&self.objective),
            phrase(&self.object_class),
            self.objects.join(","),
            dimensions.join(","),
            units.join(","),
            self.timeframe.canonical(),
            conditions.join(","),
            self.constraints.join(","),
            definitions.join(","),
            self.evidence
                .iter()
                .map(mm_core::ulid_string)
                .collect::<Vec<_>>()
                .join(","),
            values.join(",")
        )
    }

    /// The unit a dimension resolves to.
    fn unit_for(&self, dimension: &str) -> Option<&Unit> {
        self.units.iter().find(|unit| unit.name == dimension)
    }

    /// The dimension declared under `name`.
    fn dimension(&self, name: &str) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.name == name)
    }

    /// Conditions as a name→value map. A duplicate name resolves to its last
    /// occurrence; a contract that repeats a condition name is a caller bug the
    /// fixtures should catch, not something to guess about here.
    fn condition_map(&self) -> BTreeMap<String, String> {
        self.conditions
            .iter()
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect()
    }
}

/// The outcome of a comparison check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonVerdict {
    /// The pair may be compared, and a normalized comparison exists.
    Valid,
    /// The same question under conditions that do not line up.
    NonComparable,
    /// The premise is falsified: this pair is not the same question at all.
    Rejected,
}

/// Every verdict, in wire order.
pub const COMPARISON_VERDICTS: [ComparisonVerdict; 3] = [
    ComparisonVerdict::Valid,
    ComparisonVerdict::NonComparable,
    ComparisonVerdict::Rejected,
];

impl ComparisonVerdict {
    /// The stable wire name, which is also the `comparisons.verdict` value.
    pub fn as_str(self) -> &'static str {
        match self {
            ComparisonVerdict::Valid => "valid",
            ComparisonVerdict::NonComparable => "non_comparable",
            ComparisonVerdict::Rejected => "rejected",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        COMPARISON_VERDICTS
            .into_iter()
            .find(|verdict| verdict.as_str() == text)
    }
}

impl std::fmt::Display for ComparisonVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One reason a pair may not be compared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComparisonViolation {
    /// The violation code, one of [`VIOLATION_CODES`].
    pub code: String,
    /// The contract field or dimension the violation is about.
    pub field: String,
    /// What did not line up.
    pub detail: String,
}

/// Every violation code, in the order they are emitted.
pub const VIOLATION_CODES: [&str; 11] = [
    "objective_mismatch",
    "object_class_mismatch",
    "dimensional_mismatch",
    "missing_evidence",
    "timeframe_disjoint",
    "condition_mismatch",
    "definition_mismatch",
    "constraint_mismatch",
    "dimension_mismatch",
    "units_missing",
    "missing_value",
];

/// The codes that falsify the premise, and so make a pair `Rejected` rather than
/// merely `NonComparable`. Every other code leaves the comparison meaningful in
/// principle.
const PREMISE_CODES: [&str; 4] = [
    "objective_mismatch",
    "object_class_mismatch",
    "dimensional_mismatch",
    "missing_evidence",
];

/// The pair put into one set of units, with every conversion recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedComparison {
    /// The (normalized) objective both sides are measured against.
    pub objective: String,
    /// The object(s) the candidate was compared against.
    pub baseline: ObjRef,
    /// The object(s) normalized onto the baseline.
    pub candidate: ObjRef,
    /// Every conversion applied, in a fixed order.
    pub conversions: Vec<String>,
    /// The candidate's values, expressed in the baseline's units.
    pub values: BTreeMap<String, f64>,
    /// A stable rendering of the whole thing.
    pub canonical: String,
}

/// The report of one comparison check.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonCheck {
    /// What the check concluded.
    pub verdict: ComparisonVerdict,
    /// Why, in the fixed code order. Empty exactly when the verdict is `Valid`.
    pub violations: Vec<ComparisonViolation>,
    /// The normalized comparison, present exactly when the verdict is `Valid`.
    pub normalized: Option<NormalizedComparison>,
}

impl ComparisonCheck {
    /// True when the pair may be compared.
    pub fn is_comparable(&self) -> bool {
        self.verdict == ComparisonVerdict::Valid
    }

    /// The violation codes, for a log field.
    pub fn violation_codes(&self) -> Vec<String> {
        self.violations.iter().map(|v| v.code.clone()).collect()
    }
}

/// A lowercased, whitespace-collapsed phrase. Free text that means the same thing
/// must compare equal, so `"Reduce  latency"` and `"reduce latency"` are one
/// objective rather than two.
fn phrase(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Violations collected by code, so the final ordering is a property of the
/// collector rather than of the order the checks happen to run in.
#[derive(Default)]
struct Violations {
    buckets: BTreeMap<&'static str, Vec<ComparisonViolation>>,
}

impl Violations {
    fn push(&mut self, code: &'static str, field: impl Into<String>, detail: impl Into<String>) {
        debug_assert!(VIOLATION_CODES.contains(&code), "undeclared code {code}");
        self.buckets
            .entry(code)
            .or_default()
            .push(ComparisonViolation {
                code: code.to_string(),
                field: field.into(),
                detail: detail.into(),
            });
    }

    /// Flatten in [`VIOLATION_CODES`] order, each bucket sorted by field.
    fn flatten(self) -> Vec<ComparisonViolation> {
        let mut buckets = self.buckets;
        let mut out = Vec::new();
        for code in VIOLATION_CODES {
            if let Some(mut bucket) = buckets.remove(code) {
                bucket.sort_by(|a, b| a.field.cmp(&b.field).then_with(|| a.detail.cmp(&b.detail)));
                out.extend(bucket);
            }
        }
        out
    }
}

/// Decide whether two contracts describe a comparison that may be made, and why
/// not when they do not.
///
/// Both contracts are expected to satisfy [`ComparisonContract::validate`]. The
/// malformed cases that could reach here anyway map onto the closest declared
/// code rather than inventing one: a contract that names no object contributes no
/// evidence for anything, a declared unit that resolves to nothing is
/// `units_missing`, and a dimension with no value is `missing_value`.
pub fn check_contract(a: &ComparisonContract, b: &ComparisonContract) -> ComparisonCheck {
    let (verdict, violations) = classify(a, b);
    let normalized = if verdict == ComparisonVerdict::Valid {
        Some(normalize_impl(a, b))
    } else {
        None
    };
    ComparisonCheck {
        verdict,
        violations,
        normalized,
    }
}

/// Put the candidate into the baseline's units, or `None` when the pair may not
/// be compared.
///
/// Pure and deterministic: the same pair always produces the same conversions and
/// the same numbers. It returns `None` rather than a best-effort normalization
/// because a normalized comparison of a pair that is not comparable is exactly the
/// silently-wrong answer this module exists to refuse.
pub fn normalize(a: &ComparisonContract, b: &ComparisonContract) -> Option<NormalizedComparison> {
    let (verdict, _) = classify(a, b);
    if verdict != ComparisonVerdict::Valid {
        return None;
    }
    Some(normalize_impl(a, b))
}

/// The comparison without re-checking: callers must have established `Valid`.
fn normalize_impl(a: &ComparisonContract, b: &ComparisonContract) -> NormalizedComparison {
    let mut conversions = Vec::new();
    let mut values = BTreeMap::new();

    for (name, value) in &b.values {
        let factor = match (a.dimension(name), b.dimension(name)) {
            (Some(da), Some(db)) => match (da.unit.as_ref(), db.unit.as_ref()) {
                (Some(ua), Some(ub)) => match (a.unit_for(ua), b.unit_for(ub)) {
                    (Some(unit_a), Some(unit_b)) => {
                        let factor = unit_b.to_base / unit_a.to_base;
                        conversions.push(format!("b.{ub} -> a.{ua} (x{factor:.9})"));
                        factor
                    }
                    _ => 1.0,
                },
                _ => 1.0,
            },
            _ => 1.0,
        };
        values.insert(name.clone(), value * factor);
    }

    let overlap = if a.timeframe == b.timeframe {
        None
    } else {
        a.timeframe.intersection(&b.timeframe)
    };
    if let Some(overlap) = overlap {
        conversions.push(format!("timeframe -> {}", overlap.canonical()));
    }

    let objective = phrase(&a.objective);
    let baseline = a.objects.join(",");
    let candidate = b.objects.join(",");
    let conversions_rendered = conversions.join(";");
    let values_rendered = values
        .iter()
        .map(|(k, v)| format!("{k}={v:.9}"))
        .collect::<Vec<_>>()
        .join(",");
    let canonical = format!(
        "objective={objective}\nbaseline={baseline}\ncandidate={candidate}\n\
         conversions={conversions_rendered}\nvalues={values_rendered}"
    );

    NormalizedComparison {
        objective,
        baseline,
        candidate,
        conversions,
        values,
        canonical,
    }
}

/// Classify a pair and collect every violation.
fn classify(
    a: &ComparisonContract,
    b: &ComparisonContract,
) -> (ComparisonVerdict, Vec<ComparisonViolation>) {
    let mut v = Violations::default();

    // ── the premise ────────────────────────────────────────────────────────────
    if phrase(&a.objective) != phrase(&b.objective) {
        v.push(
            "objective_mismatch",
            "objective",
            format!("{:?} != {:?}", a.objective, b.objective),
        );
    }
    if phrase(&a.object_class) != phrase(&b.object_class) {
        v.push(
            "object_class_mismatch",
            "object_class",
            format!("{:?} != {:?}", a.object_class, b.object_class),
        );
    }
    if a.objects.is_empty() || b.objects.is_empty() {
        v.push(
            "missing_evidence",
            "objects",
            "a contract with no object offers nothing whose values could be compared".to_string(),
        );
    }
    if a.evidence.is_empty() || b.evidence.is_empty() {
        v.push(
            "missing_evidence",
            "evidence",
            format!(
                "a carries {} claims, b carries {}",
                a.evidence.len(),
                b.evidence.len()
            ),
        );
    }

    // ── units and dimensions ───────────────────────────────────────────────────
    let names: BTreeSet<&String> = a
        .dimensions
        .iter()
        .map(|d| &d.name)
        .chain(b.dimensions.iter().map(|d| &d.name))
        .collect();
    for name in &names {
        let name = name.as_str();
        match (a.dimension(name), b.dimension(name)) {
            (Some(da), Some(db)) => match (da.unit.as_ref(), db.unit.as_ref()) {
                (Some(ua), Some(ub)) => {
                    let resolved = (a.unit_for(ua.as_str()), b.unit_for(ub.as_str()));
                    match resolved {
                        (Some(unit_a), Some(unit_b)) => {
                            if unit_a.dimension != unit_b.dimension {
                                v.push(
                                    "dimensional_mismatch",
                                    name,
                                    format!(
                                        "{ua} measures {} but {ub} measures {}",
                                        unit_a.dimension, unit_b.dimension
                                    ),
                                );
                            }
                        }
                        _ => v.push(
                            "units_missing",
                            name,
                            format!("{ua}/{ub} is not declared in both unit tables"),
                        ),
                    }
                }
                _ => v.push(
                    "units_missing",
                    name,
                    "one side declares no unit for this dimension".to_string(),
                ),
            },
            _ => v.push(
                "dimension_mismatch",
                name,
                "the dimension is declared by one contract only".to_string(),
            ),
        }

        let shared = a.dimension(name).is_some() && b.dimension(name).is_some();
        let unvalued = !a.values.contains_key(name) || !b.values.contains_key(name);
        if shared && unvalued {
            v.push(
                "missing_value",
                name,
                format!(
                    "a has a value: {}, b has a value: {}",
                    a.values.contains_key(name),
                    b.values.contains_key(name)
                ),
            );
        }
    }

    // ── conditions ─────────────────────────────────────────────────────────────
    if !a.timeframe.overlaps(&b.timeframe) {
        v.push(
            "timeframe_disjoint",
            "timeframe",
            format!(
                "{} does not overlap {}",
                a.timeframe.canonical(),
                b.timeframe.canonical()
            ),
        );
    }

    let conditions_a = a.condition_map();
    let conditions_b = b.condition_map();
    let condition_names: BTreeSet<&String> =
        conditions_a.keys().chain(conditions_b.keys()).collect();
    for name in condition_names {
        let name = name.as_str();
        match (conditions_a.get(name), conditions_b.get(name)) {
            (Some(va), Some(vb)) => {
                if va != vb {
                    v.push("condition_mismatch", name, format!("{va:?} != {vb:?}"));
                }
            }
            _ => v.push(
                "condition_mismatch",
                name,
                "the condition is set on one side only".to_string(),
            ),
        }
    }

    // ── definitions and scope ──────────────────────────────────────────────────
    let definition_keys: BTreeSet<&String> =
        a.definitions.keys().chain(b.definitions.keys()).collect();
    for key in definition_keys {
        let key = key.as_str();
        match (a.definitions.get(key), b.definitions.get(key)) {
            (Some(da), Some(db)) => {
                if phrase(da) != phrase(db) {
                    v.push("definition_mismatch", key, format!("{da:?} != {db:?}"));
                }
            }
            _ => v.push(
                "definition_mismatch",
                key,
                "the term is defined on one side only".to_string(),
            ),
        }
    }

    let constraints_a: BTreeSet<&String> = a.constraints.iter().collect();
    let constraints_b: BTreeSet<&String> = b.constraints.iter().collect();
    if constraints_a != constraints_b {
        v.push(
            "constraint_mismatch",
            "constraints",
            format!(
                "a has [{}], b has [{}]",
                a.constraints.join(", "),
                b.constraints.join(", ")
            ),
        );
    }

    let violations = v.flatten();
    let verdict = if violations
        .iter()
        .any(|violation| PREMISE_CODES.contains(&violation.code.as_str()))
    {
        ComparisonVerdict::Rejected
    } else if violations.is_empty() {
        ComparisonVerdict::Valid
    } else {
        ComparisonVerdict::NonComparable
    };
    (verdict, violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn unit(name: &str, dimension: &str, to_base: f64) -> Unit {
        Unit {
            name: name.into(),
            dimension: dimension.into(),
            to_base,
        }
    }

    /// The baseline: p95 latency in milliseconds, measured over one hour at peak.
    fn baseline() -> ComparisonContract {
        let mut values = BTreeMap::new();
        values.insert("latency".to_string(), 120.0);
        let mut definitions = BTreeMap::new();
        definitions.insert(
            "latency".to_string(),
            "p95 server processing time".to_string(),
        );
        ComparisonContract {
            objective: "reduce request latency".into(),
            object_class: "http_service".into(),
            objects: vec!["api-v1".into()],
            dimensions: vec![Dimension {
                name: "latency".into(),
                unit: Some("ms".into()),
                higher_is_better: false,
            }],
            units: vec![unit("ms", "time", 1.0), unit("s", "time", 1000.0)],
            timeframe: Timeframe {
                start: "2026-01-01T00:00:00Z".into(),
                end: "2026-01-01T01:00:00Z".into(),
            },
            conditions: vec![Condition {
                name: "load".into(),
                value: "peak".into(),
            }],
            constraints: vec!["same hardware".into()],
            definitions,
            evidence: vec![ulid(7)],
            values,
        }
    }

    /// The candidate: the same measurement expressed in seconds.
    fn candidate_seconds() -> ComparisonContract {
        let mut values = BTreeMap::new();
        values.insert("latency".to_string(), 0.09);
        let mut contract = baseline();
        contract.objects = vec!["api-v2".into()];
        contract.dimensions[0].unit = Some("s".into());
        contract.evidence = vec![ulid(8)];
        contract.values = values;
        contract
    }

    #[test]
    fn a_matched_pair_is_valid_and_converts_units() {
        let check = check_contract(&baseline(), &candidate_seconds());
        assert_eq!(check.verdict, ComparisonVerdict::Valid, "{check:?}");
        assert!(check.violations.is_empty(), "{check:?}");
        assert!(check.is_comparable());

        let normalized = check.normalized.expect("a valid pair normalizes");
        assert_eq!(normalized.baseline, "api-v1");
        assert_eq!(normalized.candidate, "api-v2");
        // 0.09 s = 90 ms, because s has to_base 1000 and ms has to_base 1.
        let latency = normalized.values.get("latency").copied().unwrap();
        assert!((latency - 90.0).abs() < 1e-9, "{latency}");
        assert_eq!(normalized.conversions.len(), 1, "{normalized:?}");
        assert!(
            normalized.conversions[0].contains("x1000.000000000"),
            "{:?}",
            normalized.conversions
        );
    }

    #[test]
    fn a_different_objective_is_rejected() {
        let mut other = candidate_seconds();
        other.objective = "reduce memory use".into();
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::Rejected);
        assert!(check
            .violation_codes()
            .contains(&"objective_mismatch".to_string()));
        assert!(check.normalized.is_none());
        assert!(normalize(&baseline(), &other).is_none());
    }

    #[test]
    fn a_different_object_class_is_rejected() {
        let mut other = candidate_seconds();
        other.object_class = "database".into();
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::Rejected);
        assert!(check
            .violation_codes()
            .contains(&"object_class_mismatch".to_string()));
    }

    #[test]
    fn comparing_a_mass_with_a_duration_is_rejected() {
        let mut other = candidate_seconds();
        // `kg` is a new unit in the candidate's own table, measuring mass.
        other.dimensions[0].unit = Some("kg".into());
        other.units = vec![unit("ms", "time", 1.0), unit("kg", "mass", 1.0)];
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::Rejected, "{check:?}");
        assert!(check
            .violation_codes()
            .contains(&"dimensional_mismatch".to_string()));
    }

    #[test]
    fn disjoint_timeframes_are_non_comparable() {
        let mut other = candidate_seconds();
        other.timeframe = Timeframe {
            start: "2026-02-01T00:00:00Z".into(),
            end: "2026-02-01T01:00:00Z".into(),
        };
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::NonComparable);
        assert!(check
            .violation_codes()
            .contains(&"timeframe_disjoint".to_string()));
        assert!(check.normalized.is_none());
    }

    #[test]
    fn a_differing_condition_is_non_comparable() {
        let mut other = candidate_seconds();
        other.conditions = vec![Condition {
            name: "load".into(),
            value: "off_peak".into(),
        }];
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::NonComparable, "{check:?}");
        assert!(check
            .violation_codes()
            .contains(&"condition_mismatch".to_string()));
    }

    #[test]
    fn a_missing_value_is_non_comparable() {
        let mut other = candidate_seconds();
        other.values.remove("latency");
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::NonComparable, "{check:?}");
        assert!(check
            .violation_codes()
            .contains(&"missing_value".to_string()));
    }

    #[test]
    fn a_missing_evidence_chain_is_rejected() {
        let mut other = candidate_seconds();
        other.evidence.clear();
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::Rejected);
        assert!(check
            .violation_codes()
            .contains(&"missing_evidence".to_string()));
    }

    #[test]
    fn a_dimension_declared_on_one_side_only_is_non_comparable() {
        let mut other = candidate_seconds();
        other.dimensions.push(Dimension {
            name: "throughput".into(),
            unit: None,
            higher_is_better: true,
        });
        other.values.insert("throughput".to_string(), 900.0);
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::NonComparable, "{check:?}");
        assert!(check
            .violation_codes()
            .contains(&"dimension_mismatch".to_string()));
    }

    #[test]
    fn an_overlapping_but_unequal_timeframe_is_valid_and_recorded() {
        let mut other = candidate_seconds();
        other.timeframe = Timeframe {
            start: "2026-01-01T00:30:00Z".into(),
            end: "2026-01-01T01:30:00Z".into(),
        };
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::Valid, "{check:?}");
        let normalized = check.normalized.expect("valid pairs normalize");
        assert!(
            normalized
                .conversions
                .iter()
                .any(|c| c == "timeframe -> 2026-01-01T00:30:00Z..2026-01-01T01:00:00Z"),
            "{:?}",
            normalized.conversions
        );
    }

    #[test]
    fn violations_are_emitted_in_the_declared_order() {
        let mut other = candidate_seconds();
        other.objective = "something else".into();
        other.timeframe = Timeframe {
            start: "2027-01-01T00:00:00Z".into(),
            end: "2027-01-01T01:00:00Z".into(),
        };
        let check = check_contract(&baseline(), &other);
        assert_eq!(check.verdict, ComparisonVerdict::Rejected);
        let codes = check.violation_codes();
        let positions: Vec<usize> = codes
            .iter()
            .map(|code| {
                VIOLATION_CODES
                    .iter()
                    .position(|declared| declared == code)
                    .expect("every emitted code is declared")
            })
            .collect();
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "{codes:?}");
    }

    #[test]
    fn an_empty_objective_is_refused_and_duplicate_objects_are_refused() {
        let mut contract = baseline();
        contract.objective = "   ".into();
        assert_eq!(contract.validate().unwrap_err().code(), "validation");

        let mut contract = baseline();
        contract.objects = vec!["api-v1".into(), "api-v1".into()];
        assert!(contract.validate().is_err());

        let mut contract = baseline();
        contract.units = vec![unit("ms", "time", 0.0)];
        assert!(contract.validate().is_err());

        assert!(baseline().validate().is_ok());
    }

    #[test]
    fn timeframes_compare_and_intersect() {
        let a = Timeframe {
            start: "2026-01-01T00:00:00Z".into(),
            end: "2026-01-01T02:00:00Z".into(),
        };
        let inside = Timeframe {
            start: "2026-01-01T01:00:00Z".into(),
            end: "2026-01-01T01:30:00Z".into(),
        };
        let away = Timeframe {
            start: "2026-03-01T00:00:00Z".into(),
            end: "2026-03-01T01:00:00Z".into(),
        };
        assert!(a.covers(&inside));
        assert!(!inside.covers(&a));
        assert!(a.overlaps(&inside));
        assert!(!a.overlaps(&away));
        assert_eq!(
            a.intersection(&inside).unwrap().canonical(),
            inside.canonical()
        );
    }

    #[test]
    fn the_same_pair_classifies_identically_twice() {
        let first = check_contract(&baseline(), &candidate_seconds());
        let second = check_contract(&baseline(), &candidate_seconds());
        assert_eq!(first, second);
        assert_eq!(
            first.normalized.unwrap().canonical,
            second.normalized.unwrap().canonical
        );
    }
}
