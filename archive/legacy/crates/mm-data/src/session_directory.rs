//! Lazy session-directory service.
//!
//! A session has no directory until the first session-scoped file is created.
//! The directory root is `data/sessions/<session-ulid>/`; `files/` is created
//! only with the first file. The metadata mirror is `mm:sessionDir`, written by
//! `StateService` when the directory is first created.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// The session directory service.
#[derive(Debug, Clone)]
pub struct SessionDirectoryService {
    base_dir: PathBuf,
}

impl SessionDirectoryService {
    /// Build under the data root.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// The root that contains session directories.
    pub fn root(&self) -> PathBuf {
        self.base_dir.join("sessions")
    }

    /// The directory path for a session, whether or not it exists.
    pub fn path(&self, session_id: &str) -> Result<PathBuf, SessionDirectoryError> {
        validate_session_id(session_id)?;
        Ok(self.root().join(session_id))
    }

    /// Whether the directory exists.
    pub fn has_dir(&self, session_id: &str) -> Result<bool, SessionDirectoryError> {
        Ok(self.path(session_id)?.is_dir())
    }

    /// Create the session root on first use. Does not create `files/`.
    pub fn ensure(&self, session_id: &str) -> Result<PathBuf, SessionDirectoryError> {
        let path = self.path(session_id)?;
        std::fs::create_dir_all(&path)?;
        set_private_dir_mode(&path)?;
        Ok(path)
    }

    /// Create the session root and its `files/` child.
    pub fn ensure_files_dir(&self, session_id: &str) -> Result<PathBuf, SessionDirectoryError> {
        let path = self.ensure(session_id)?.join("files");
        std::fs::create_dir_all(&path)?;
        set_private_dir_mode(&path)?;
        Ok(path)
    }

    /// Write one session-scoped file. The relative path is confined beneath the
    /// session directory and cannot contain `..`.
    pub fn write_file(
        &self,
        session_id: &str,
        relative: impl AsRef<Path>,
        bytes: &[u8],
    ) -> Result<PathBuf, SessionDirectoryError> {
        let root = self.ensure(session_id)?;
        let relative = relative.as_ref();
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(SessionDirectoryError::Escape(relative.to_path_buf()));
        }
        let path = root.join("files").join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            set_private_dir_mode(parent)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Remove the entire session directory. Missing is a no-op.
    pub fn remove(&self, session_id: &str) -> Result<(), SessionDirectoryError> {
        let path = self.path(session_id)?;
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

fn set_private_dir_mode(path: &Path) -> Result<(), SessionDirectoryError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<(), SessionDirectoryError> {
    if session_id.trim().is_empty()
        || session_id
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
    {
        return Err(SessionDirectoryError::InvalidSessionId(session_id.into()));
    }
    Ok(())
}

/// Session-directory failures.
#[derive(Debug, Error)]
pub enum SessionDirectoryError {
    /// Filesystem failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Invalid session id.
    #[error("invalid session id {0:?}")]
    InvalidSessionId(String),
    /// A relative path tried to escape the session directory.
    #[error("path {} escapes the session directory", .0.display())]
    Escape(PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_is_lazy_and_files_are_confined() {
        let temp = tempfile::tempdir().unwrap();
        let service = SessionDirectoryService::new(temp.path());
        assert!(!service.has_dir("01abc").unwrap());
        let dir = service.ensure("01abc").unwrap();
        assert!(dir.is_dir());
        assert!(service.has_dir("01abc").unwrap());
        assert!(!dir.join("files").exists());
        let file = service
            .write_file("01abc", "notes/first.txt", b"hello")
            .unwrap();
        assert!(file.exists());
        assert!(service.write_file("01abc", "../escape.txt", b"no").is_err());
        service.remove("01abc").unwrap();
        assert!(!service.has_dir("01abc").unwrap());
    }
}
