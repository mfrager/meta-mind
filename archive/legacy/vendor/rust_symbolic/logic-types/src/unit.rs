//! Units as semantic metadata (`full_system_design.md` §5.3).

/// A unit symbol (`USD`, `percent`, `year`, `person`, …).
///
/// Units are attached to values/types as metadata; they are not interchangeable
/// with dimensions or with one another. `USD + percent` must be rejected before
/// execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Unit(pub String);

impl Unit {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self(symbol.into())
    }

    pub fn usd() -> Self {
        Self::new("USD")
    }

    pub fn percent() -> Self {
        Self::new("percent")
    }

    pub fn year() -> Self {
        Self::new("year")
    }

    pub fn person() -> Self {
        Self::new("person")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
