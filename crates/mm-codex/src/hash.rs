//! Content hashing for files and modules.
//!
//! Two facts drive this module: a file's identity is its bytes, and a module's
//! identity is its file set. Both must be reproducible from any directory, on any
//! machine, in any walk order — otherwise `codex verify` and `codex lock --check`
//! would report drift that is only an artefact of iteration order.

use sha1::{Digest, Sha1};

/// The lowercase hex sha256 of a file's bytes.
pub fn file_hash(bytes: &[u8]) -> String {
    mm_core::hash::content_hash(bytes)
}

/// The SWHID of a file's content: `swh:1:cnt:<sha1 of the git blob>`.
///
/// This is the *intrinsic* identifier Software Heritage computes, so a Metamind
/// file can be recognised by an external archive without consulting a registry.
/// It is derived by hashing `"blob <len>\0<bytes>"`, exactly as git addresses a
/// blob, so the value agrees with `git hash-object`.
pub fn swhid_content(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(40);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("swh:1:cnt:{hex}")
}

/// The canonical content hash of a module: the sha256 of its sorted
/// `(rel_path, sha256)` pairs.
///
/// Sorted, so the walk order cannot change the answer; path-and-hash, so adding,
/// removing, renaming, or editing any file changes it. This is the value
/// `codex.lock` records and `codex verify` compares.
pub fn module_content_hash(files: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort();
    let mut joined = String::new();
    for (path, hash) in sorted {
        joined.push_str(path);
        joined.push('\u{1f}');
        joined.push_str(hash);
        joined.push('\n');
    }
    mm_core::hash::content_hash(joined.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn file_hash_is_sha256_hex() {
        let h = file_hash(b"fn main() {}");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(h, file_hash(b"fn main() {}"));
        assert_ne!(h, file_hash(b"fn main() { }"));
    }

    #[test]
    fn swhid_is_the_git_blob_id() {
        // `printf 'hello\n' | git hash-object --stdin` is
        // ce013625030ba8dba906f756967f9e9ca394464a.
        assert_eq!(
            swhid_content(b"hello\n"),
            "swh:1:cnt:ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(
            swhid_content(b""),
            "swh:1:cnt:e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }

    fn pair(path: &str, hash: &str) -> (String, String) {
        (path.to_string(), hash.to_string())
    }

    #[test]
    fn module_hash_is_invariant_under_walk_order_and_sensitive_to_content() {
        let a = vec![
            pair("src/lib.rs", "aa"),
            pair("src/scan.rs", "bb"),
            pair("Cargo.toml", "cc"),
        ];
        let mut reversed = a.clone();
        reversed.reverse();

        assert_eq!(module_content_hash(&a), module_content_hash(&reversed));

        // Every way a module can change produces a new hash.
        let edited = vec![
            pair("src/lib.rs", "a1"),
            pair("src/scan.rs", "bb"),
            pair("Cargo.toml", "cc"),
        ];
        let renamed = vec![
            pair("src/lib_.rs", "aa"),
            pair("src/scan.rs", "bb"),
            pair("Cargo.toml", "cc"),
        ];
        let added = {
            let mut v = a.clone();
            v.push(pair("src/new.rs", "dd"));
            v
        };
        assert_ne!(module_content_hash(&a), module_content_hash(&edited));
        assert_ne!(module_content_hash(&a), module_content_hash(&renamed));
        assert_ne!(module_content_hash(&a), module_content_hash(&added));
    }

    proptest! {
        /// The same file set always hashes the same, whatever order it arrives in.
        #[test]
        fn module_hash_depends_only_on_the_file_set(count in 1usize..20) {
            let files: Vec<(String, String)> = (0..count)
                .map(|i| pair(&format!("src/f{i}.rs"), &format!("{i:064x}")))
                .collect();
            let mut shuffled = files.clone();
            shuffled.reverse();
            prop_assert_eq!(module_content_hash(&files), module_content_hash(&shuffled));
        }
    }
}
