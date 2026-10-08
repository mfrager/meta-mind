//! Fixture crate that depends on `beta` and uses it.

use beta::helper;

/// Twice whatever `beta` produces.
pub fn alpha() -> u32 {
    helper() * 2
}

#[cfg(test)]
mod tests {
    #[test]
    fn alpha_doubles_beta() {
        assert_eq!(super::alpha(), 4);
    }
}
