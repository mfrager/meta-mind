//! Half of the dependency cycle: this crate uses `beta`.

use beta::helper;

/// Uses `beta`, which uses this crate back.
pub fn alpha() -> u32 {
    helper()
}

#[cfg(test)]
mod tests {
    #[test]
    fn alpha_has_a_test() {
        assert!(super::alpha() >= 0);
    }
}
