//! Embed the *resolved* Oxigraph version so `doctor` reports the version the
//! binary actually links against, not a hand-maintained constant that can drift
//! from `Cargo.toml`.

use std::path::PathBuf;

fn main() {
    let lock = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("..")
        .join("..")
        .join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());

    let version = read_package_version(&lock, "oxigraph").unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MM_OXIGRAPH_VERSION={version}");
}

/// The `version` of `name`'s entry in a Cargo.lock (`[[package]] name/version`).
fn read_package_version(lock: &std::path::Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(lock).ok()?;
    let wanted = format!("name = \"{name}\"");
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == wanted {
            // The version line follows the name line in Cargo.lock's stable layout.
            for next in lines.by_ref().take(4) {
                let next = next.trim();
                if let Some(rest) = next.strip_prefix("version = ") {
                    return Some(rest.trim_matches('"').to_string());
                }
            }
            return None;
        }
    }
    None
}
