//! The required workspace guard.
//!
//! Every application path must reach a model through `mm-llm-agentstream`. This scan
//! is lexical and path-scoped, so a new direct provider construction is an intentional,
//! reviewable change rather than an easy accident.

use std::path::{Path, PathBuf};

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some("tests") {
                continue;
            }
            rust_sources(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path.canonicalize().unwrap_or(path));
        }
    }
}

fn source_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    rust_sources(&root.join("crates"), &mut files);
    files
}

fn hits(needle: &str) -> Vec<String> {
    source_files()
        .into_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(&path).ok()?;
            source
                .contains(needle)
                .then(|| path.to_string_lossy().into_owned())
        })
        .collect()
}

fn assert_only(needle: &str, allowed: &[&str]) {
    let unexpected: Vec<String> = hits(needle)
        .into_iter()
        .filter(|path| !allowed.iter().any(|prefix| path.contains(prefix)))
        .collect();
    assert!(
        unexpected.is_empty(),
        "`{needle}` must stay inside the gateway; found {unexpected:?}"
    );
}

fn assert_one_gateway_site(needle: &str, expected_suffix: &str) {
    let sites: Vec<String> = hits(needle)
        .into_iter()
        .filter(|path| path.contains("crates/mm-llm-agentstream/"))
        .collect();
    assert_eq!(
        sites.len(),
        1,
        "`{needle}` must have one gateway construction site; found {sites:?}"
    );
    assert!(
        sites[0].ends_with(expected_suffix),
        "`{needle}` moved out of the gateway composition point: {}",
        sites[0]
    );
}

#[test]
fn no_application_crate_constructs_a_live_client() {
    assert_only(
        "OpenAiClient::new",
        &["crates/mm-llm-agentstream/", "crates/mm-llm/src/openai.rs"],
    );
    assert_only(
        "OpenAiClient::from_env",
        &["crates/mm-llm-agentstream/", "crates/mm-llm/src/openai.rs"],
    );
    assert_only(
        "LlmService::new",
        &["crates/mm-llm-agentstream/", "crates/mm-llm/src/service.rs"],
    );
    assert_only(
        "ProviderEnv::from_pairs",
        &[
            "crates/mm-llm-agentstream/",
            "crates/mm-llm/src/openai.rs",
            "crates/mm-llm/src/config.rs",
        ],
    );
    assert_one_gateway_site(
        "OpenAiClient::from_env",
        "crates/mm-llm-agentstream/src/pool.rs",
    );
    assert_one_gateway_site("LlmService::new", "crates/mm-llm-agentstream/src/lib.rs");
}

#[test]
fn hermes_does_not_carry_its_own_provider_transport() {
    let hermes: Vec<String> = hits("TcpStream")
        .into_iter()
        .chain(hits("/chat/completions"))
        .filter(|path| path.contains("crates/mm-agent/"))
        .collect();
    assert!(
        hermes.is_empty(),
        "Hermes must use the gateway, not its own transport: {hermes:?}"
    );
}
