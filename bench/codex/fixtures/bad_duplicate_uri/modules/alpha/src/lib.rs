//! First module of the duplicate-uri fixture.

#[tbox_fn]
pub fn alpha() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    #[test]
    fn alpha_is_one() {
        assert_eq!(super::alpha(), 1);
    }
}
