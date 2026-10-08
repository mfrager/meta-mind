//! Content hashing. Two byte streams are the same fact iff their hashes match.
//!
//! Metamind hashes facts, not representations: callers normalize first (canonical
//! JSON, canonical N-Triples) so that a hash is stable across processes, runs, and
//! machines.

use sha2::{Digest, Sha256};

/// The separator used when hashing several fields as one fact. It is a control
/// character that cannot appear unescaped in our canonical JSON, so
/// `["a", "bc"]` and `["ab", "c"]` cannot collide.
const FIELD_SEP: char = '\u{1f}';

/// The lowercase hex sha256 of `bytes`.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

/// The hash of an ordered tuple of string fields.
pub fn hash_fields(fields: &[&str]) -> String {
    let mut joined = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            joined.push(FIELD_SEP);
        }
        joined.push_str(f);
    }
    content_hash(joined.as_bytes())
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
