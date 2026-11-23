//! Secret refusal shared by every capture path.
//!
//! The CM must not store passwords, tokens, API keys, or credentials in data,
//! memory, logs, or internal-action input. This is deliberately a conservative
//! lexical check, not an entropy model: a false positive is a retry, a false
//! negative is a leaked secret.

/// Whether `text` looks like it contains a credential.
pub fn contains_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "password",
        "passwd",
        "api_key",
        "api-key",
        "apikey",
        "access_token",
        "access-token",
        "refresh_token",
        "refresh-token",
        "client_secret",
        "client-secret",
        "private_key",
        "private-key",
        "authorization: bearer",
        "-----begin rsa private key-----",
        "-----begin openssh private key-----",
        "-----begin private key-----",
    ];
    if MARKERS.iter().any(|marker| lower.contains(marker)) {
        return true;
    }
    // Common provider prefixes.
    text.split_whitespace().any(|word| {
        word.starts_with("sk-")
            || word.starts_with("sk_live_")
            || word.starts_with("ghp_")
            || word.starts_with("github_pat_")
            || word.starts_with("xoxb-")
            || word.starts_with("AKIA")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_credentials_are_detected() {
        assert!(contains_secret("password=hunter2"));
        assert!(contains_secret("Authorization: Bearer abc"));
        assert!(contains_secret("sk-live-123456"));
        assert!(!contains_secret("the refresh test is flaky"));
    }
}
