//! ID generation (§7.6): lowercase ULID strings for instance IRIs.
//!
//! - `UlidRandom` — `ulid::Ulid::new()` lowercased (26-char Crockford base32).
//! - `FromContentHash` — a ULID derived from the FNV-1a-128 of the canonical
//!   structure, so the same structure always yields the same IRI (needed for
//!   reproducible artifacts and content addressing).

use ulid::Ulid;

/// The ID policy for instance nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdPolicy {
    /// Random lowercase ULIDs (nexus default; ephemeral instances).
    UlidRandom,
    /// Deterministic: ULID derived from the FNV-1a-128 of the canonical
    /// structure (stored / content-addressed artifacts).
    FromContentHash,
}

/// A fresh lowercase ULID string, e.g. `01arz3ndektsv4rrffq69g5fav`.
pub fn new_ulid() -> String {
    let mut generator = ulid::Generator::new();
    let ulid = match generator.generate() {
        Ok(u) => u,
        Err(overflow) => overflow.commit_overflow_random(),
    };
    ulid.to_string().to_lowercase()
}

/// A deterministic lowercase ULID derived from `content`: same content in,
/// same ULID out.
pub fn ulid_from_content(content: &str) -> String {
    Ulid::from_bytes(fnv1a_128(content))
        .to_string()
        .to_lowercase()
}

/// FNV-1a 128-bit hash, returned as big-endian bytes (usable as a ULID seed).
pub fn fnv1a_128(s: &str) -> [u8; 16] {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for b in s.as_bytes() {
        hash ^= u128::from(*b);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    hash.to_be_bytes()
}

/// FNV-1a 64-bit hash as lowercase hex (matches `math-manager/registry.rs`).
pub fn fnv1a_hex(s: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in s.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_shape() {
        let id = new_ulid();
        assert_eq!(id.len(), 26, "ULID must be 26 chars, got `{id}`");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "ULID must be lowercase Crockford base32, got `{id}`"
        );
    }

    #[test]
    fn deterministic_ulid() {
        assert_eq!(ulid_from_content("a"), ulid_from_content("a"));
        assert_ne!(ulid_from_content("a"), ulid_from_content("b"));
        assert_eq!(ulid_from_content("a").len(), 26);
    }

    #[test]
    fn fnv_128_hashes_are_stable() {
        assert_eq!(fnv1a_128("x"), fnv1a_128("x"));
        assert_eq!(fnv1a_128("x").len(), 16);
        assert_eq!(fnv1a_hex("hello"), fnv1a_hex("hello"));
    }
}
