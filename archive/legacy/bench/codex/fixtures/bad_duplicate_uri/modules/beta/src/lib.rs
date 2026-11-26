//! Second module of the duplicate-uri fixture.

#[tbox_fn]
pub fn beta() -> u32 {
    2
}

#[cfg(test)]
mod tests {
    #[test]
    fn beta_is_two() {
        assert_eq!(super::beta(), 2);
    }
}
