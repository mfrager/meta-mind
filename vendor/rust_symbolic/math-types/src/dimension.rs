//! Dimensional analysis (master_design.md §8).

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseDimension {
    Length,
    Mass,
    Time,
    Current,
    Temperature,
    Amount,
    LuminousIntensity,
    Angle,
}

impl fmt::Display for BaseDimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BaseDimension::Length => "L",
            BaseDimension::Mass => "M",
            BaseDimension::Time => "T",
            BaseDimension::Current => "I",
            BaseDimension::Temperature => "Θ",
            BaseDimension::Amount => "N",
            BaseDimension::LuminousIntensity => "J",
            BaseDimension::Angle => "rad",
        };
        write!(f, "{s}")
    }
}

/// A dimension as a map of base-dimension exponents.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dimension {
    pub exponents: HashMap<BaseDimension, i32>,
}

impl Dimension {
    pub fn dimensionless() -> Self {
        Self::default()
    }

    pub fn base(b: BaseDimension) -> Self {
        let mut d = Self::dimensionless();
        d.exponents.insert(b, 1);
        d
    }

    pub fn mul(&self, other: &Self) -> Self {
        self.combine(other, 1)
    }

    pub fn div(&self, other: &Self) -> Self {
        self.combine(other, -1)
    }

    pub fn pow(&self, n: i32) -> Self {
        let mut d = Self::dimensionless();
        for (b, e) in &self.exponents {
            d.exponents.insert(*b, e * n);
        }
        d
    }

    pub fn is_dimensionless(&self) -> bool {
        self.exponents.values().all(|&e| e == 0)
    }

    fn combine(&self, other: &Self, sign: i32) -> Self {
        let mut d = self.clone();
        for (b, e) in &other.exponents {
            *d.exponents.entry(*b).or_insert(0) += sign * e;
        }
        d.exponents.retain(|_, &mut e| e != 0);
        d
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "1");
        }
        let mut terms: Vec<String> = Vec::new();
        for (b, e) in &self.exponents {
            if *e == 1 {
                terms.push(b.to_string());
            } else {
                terms.push(format!("{b}^{e}"));
            }
        }
        write!(f, "{}", terms.join("·"))
    }
}
