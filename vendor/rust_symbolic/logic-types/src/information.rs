//! Shared information-theoretic semantic vocabulary (information design §1.4).
//!
//! These types deliberately live in `logic-types`: they describe the meaning
//! and guarantee of an information quantity without depending on a particular
//! calculator, estimator, or execution backend.

use crate::{Exactness, Unit};

/// Units used by logarithmic information quantities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InformationUnit {
    Bits,
    Nats,
    Bans,
    Bytes,
}

impl InformationUnit {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bits => "bits",
            Self::Nats => "nats",
            Self::Bans => "bans",
            Self::Bytes => "bytes",
        }
    }

    /// Convert a value to another information unit.
    pub fn convert(self, value: f64, target: Self) -> f64 {
        let bits = match self {
            Self::Bits => value,
            Self::Nats => value / std::f64::consts::LN_2,
            Self::Bans => value * 10.0 / std::f64::consts::LOG2_10,
            Self::Bytes => value * 8.0,
        };
        match target {
            Self::Bits => bits,
            Self::Nats => bits * std::f64::consts::LN_2,
            Self::Bans => bits * std::f64::consts::LOG10_2,
            Self::Bytes => bits / 8.0,
        }
    }
}

impl std::fmt::Display for InformationUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The semantic source/type of information. These categories are deliberately
/// distinct: a probabilistic sensor report is not a proof merely because both
/// reduce uncertainty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InformationKind {
    Observational,
    Causal,
    Semantic,
    Geometric,
    Temporal,
    Probabilistic,
    Proof,
    Computational,
    Experimental,
    Social,
}

impl InformationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observational => "observational",
            Self::Causal => "causal",
            Self::Semantic => "semantic",
            Self::Geometric => "geometric",
            Self::Temporal => "temporal",
            Self::Probabilistic => "probabilistic",
            Self::Proof => "proof",
            Self::Computational => "computational",
            Self::Experimental => "experimental",
            Self::Social => "social",
        }
    }
}

/// A numeric result with an explicit guarantee and unit.
#[derive(Debug, Clone, PartialEq)]
pub struct InformationValue {
    pub value: f64,
    pub unit: InformationUnit,
    pub exactness: Exactness,
    pub error_bound: Option<f64>,
    pub confidence: Option<f64>,
}

impl InformationValue {
    pub fn exact(value: f64, unit: InformationUnit) -> Self {
        Self {
            value,
            unit,
            exactness: Exactness::Exact,
            error_bound: None,
            confidence: None,
        }
    }

    pub fn numerical(value: f64, unit: InformationUnit, error_bound: f64) -> Self {
        Self {
            value,
            unit,
            exactness: Exactness::Approximate,
            error_bound: Some(error_bound),
            confidence: None,
        }
    }

    pub fn estimated(value: f64, unit: InformationUnit, confidence: f64, error_bound: f64) -> Self {
        Self {
            value,
            unit,
            exactness: Exactness::Sampled,
            error_bound: Some(error_bound),
            confidence: Some(confidence),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.value.is_finite() {
            return Err("information value must be finite".into());
        }
        if self.error_bound.is_some_and(|v| !v.is_finite() || v < 0.0) {
            return Err("information error bound must be finite and non-negative".into());
        }
        if self
            .confidence
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err("information confidence must be in [0, 1]".into());
        }
        Ok(())
    }
}

/// A typed information quantity. The operand references remain opaque at this
/// shared layer; `info-ir` owns their concrete distribution/model structures.
#[derive(Debug, Clone, PartialEq)]
pub struct InformationQuantity {
    pub name: String,
    pub kind: InformationKind,
    pub value: InformationValue,
    pub assumptions: Vec<String>,
    pub source: Option<String>,
}

impl InformationQuantity {
    pub fn new(
        name: impl Into<String>,
        kind: InformationKind,
        value: InformationValue,
    ) -> Result<Self, String> {
        value.validate()?;
        Ok(Self {
            name: name.into(),
            kind,
            value,
            assumptions: Vec::new(),
            source: None,
        })
    }
}

/// A semantic quantity with a conventional information-theoretic name.
#[derive(Debug, Clone, PartialEq)]
pub struct Entropy(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalEntropy(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct JointEntropy(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct MutualInformation(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalMutualInformation(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct KLDivergence(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct JSDivergence(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct CrossEntropy(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct InformationGain(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedInformationGain(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct ValueOfInformation(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedValueOfInformation(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct Surprisal(pub InformationQuantity);

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelCapacity(pub InformationQuantity);

/// Rate-distortion: the minimal channel rate R(D) needed to achieve a given
/// distortion D, or a point on the rate-distortion curve.
#[derive(Debug, Clone, PartialEq)]
pub struct RateDistortion(pub InformationQuantity);

/// Description length of an object (model, data, program) under a specified
/// encoding scheme.
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptionLength(pub InformationQuantity);

/// Minimum description length score: L(model) + L(data | model).
#[derive(Debug, Clone, PartialEq)]
pub struct MDLScore(pub InformationQuantity);

/// Information lost through a transformation, abstraction, or compression.
#[derive(Debug, Clone, PartialEq)]
pub struct InformationLoss(pub InformationQuantity);

/// Compression ratio: original size / compressed size.
#[derive(Debug, Clone, PartialEq)]
pub struct CompressionRatio(pub InformationQuantity);

/// A unit-bearing cost for an information-producing action. The underlying
/// unit may be computational, monetary, temporal, or domain-specific.
#[derive(Debug, Clone, PartialEq)]
pub struct InformationCost {
    pub value: f64,
    pub unit: Unit,
}

impl InformationCost {
    pub fn new(value: f64, unit: Unit) -> Result<Self, String> {
        if !value.is_finite() || value < 0.0 {
            return Err("information cost must be finite and non-negative".into());
        }
        Ok(Self { value, unit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn information_units_convert() {
        assert!((InformationUnit::Bytes.convert(1.0, InformationUnit::Bits) - 8.0).abs() < 1e-12);
        assert!(
            (InformationUnit::Bits.convert(1.0, InformationUnit::Nats) - std::f64::consts::LN_2)
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn values_reject_invalid_metadata() {
        assert!(
            InformationValue::estimated(1.0, InformationUnit::Bits, 1.2, 0.1)
                .validate()
                .is_err()
        );
        assert!(
            InformationValue::numerical(1.0, InformationUnit::Bits, -1.0)
                .validate()
                .is_err()
        );
    }
}
