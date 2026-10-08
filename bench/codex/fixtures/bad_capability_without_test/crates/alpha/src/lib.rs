//! Planted defect of this fixture: a capability with no test behind it.
//!
//! There is deliberately no `#[test]` anywhere in this tree, and no `mod tests`.
//! Every other property of this workspace is clean, so `capability_without_test`
//! is the only finding.

/// Public surface with nothing verifying it.
pub fn untested() -> u32 {
    42
}
