//! Half of the dependency cycle: this crate uses `alpha`.

use alpha::alpha;

/// Uses `alpha`, which uses this crate back.
pub fn helper() -> u32 {
    alpha()
}

#[cfg(test)]
mod tests {
    #[test]
    fn beta_has_a_test() {
        assert!(super::helper() >= 0);
    }
}
