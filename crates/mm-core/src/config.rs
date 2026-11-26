//! Process configuration.
//!
//! The retained core needs a data root, an HTTP bind address, and the two model
//! names a turn uses. Everything else that used to live here (ontology paths,
//! module registry, codex lock) belonged to retired systems.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::MmError;

/// The configuration file `mm-serve` reads when no path is given.
pub const DEFAULT_CONFIG_PATH: &str = "config/server.toml";
/// The new server's data root. Legacy `data/` is deliberately untouched.
pub const DEFAULT_DATA_DIR: &str = "data/core";

/// The server configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Directory holding `server.lock` and `agentstream/`.
    pub data_dir: PathBuf,
    /// Interface to bind.
    pub bind: String,
    /// Port to bind.
    pub port: u16,
    /// The model every turn uses unless a pass overrides it.
    pub model: Option<String>,
    /// The model the verification judge uses, when different.
    pub judge_model: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(DEFAULT_DATA_DIR),
            bind: "127.0.0.1".into(),
            port: 8899,
            model: None,
            judge_model: None,
        }
    }
}

impl Config {
    /// Read a configuration file. A missing file uses the defaults.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, MmError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| MmError::Config(format!("cannot read {}: {error}", path.display())))?;
        Self::from_toml(&text)
    }

    /// Read `config/server.toml` relative to the process cwd.
    pub fn load_default() -> Result<Self, MmError> {
        Self::load(DEFAULT_CONFIG_PATH)
    }

    /// Parse a TOML document.
    pub fn from_toml(text: &str) -> Result<Self, MmError> {
        toml::from_str(text).map_err(|error| MmError::Config(error.to_string()))
    }

    /// A configuration for a self-contained data directory.
    pub fn for_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            ..Self::default()
        }
    }
}

/// Load `.env` from the process cwd, if it exists.
pub fn load_project_env() -> Result<(), MmError> {
    let path = Path::new(".env");
    if path.exists() {
        dotenvy::from_path(path)
            .map_err(|error| MmError::Config(format!("cannot read .env: {error}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_the_new_data_root() {
        let config = Config::default();
        assert_eq!(config.data_dir, Path::new("data/core"));
        assert_eq!(config.bind, "127.0.0.1");
        assert_eq!(config.port, 8899);
    }

    #[test]
    fn a_partial_document_keeps_the_defaults() {
        let config = Config::from_toml("port = 9000\n").unwrap();
        assert_eq!(config.port, 9000);
        assert_eq!(config.data_dir, Path::new("data/core"));
    }

    #[test]
    fn unknown_keys_are_refused() {
        assert!(Config::from_toml("bogus = 1\n").is_err());
    }

    #[test]
    fn a_missing_file_is_the_default() {
        let config = Config::load("/definitely/not/server.toml").unwrap();
        assert_eq!(config, Config::default());
    }
}
