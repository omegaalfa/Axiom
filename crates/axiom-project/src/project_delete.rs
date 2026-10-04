//! Headless, workspace-contained fingerprint-guarded deletion of one file.

use std::{
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

use crate::project_update::{TextFileFingerprint, fingerprint_bytes};
use crate::project_write::validate_relative_path;

#[derive(Debug, PartialEq, Eq)]
pub enum ProjectDeleteError {
    InvalidPath(String),
    OutsideWorkspace(String),
    NotFound(String),
    NotRegularFile(String),
    SymlinkNotAllowed(String),
    FingerprintMismatch {
        expected: TextFileFingerprint,
        actual: TextFileFingerprint,
    },
    Io {
        operation: &'static str,
        path: String,
        message: String,
    },
}

impl fmt::Display for ProjectDeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid project-relative path: {path}"),
            Self::OutsideWorkspace(path) => {
                write!(f, "path is outside the project workspace: {path}")
            }
            Self::NotFound(path) => write!(f, "file not found: {path}"),
            Self::NotRegularFile(path) => write!(f, "path is not a regular file: {path}"),
            Self::SymlinkNotAllowed(path) => {
                write!(f, "symbolic links are not allowed in delete paths: {path}")
            }
            Self::FingerprintMismatch { expected, actual } => write!(
                f,
                "file fingerprint is stale: expected {}, current {}",
                expected.to_wire_string(),
                actual.to_wire_string()
            ),
            Self::Io {
                operation,
                path,
                message,
            } => write!(f, "failed to {operation} {path}: {message}"),
        }
    }
}

impl std::error::Error for ProjectDeleteError {}

#[derive(Clone, Debug)]
pub struct ProjectDeleteCapability {
    workspace_root: PathBuf,
}

impl ProjectDeleteCapability {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let workspace_root = fs::canonicalize(workspace_root)?;
        if !workspace_root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "workspace root is not a directory",
            ));
        }
        Ok(Self { workspace_root })
    }

    pub fn delete_file(
        &self,
        relative_path: &str,
        expected_fingerprint: TextFileFingerprint,
    ) -> Result<PathBuf, ProjectDeleteError> {
        let relative = validate_relative_path(relative_path)
            .map_err(|error| ProjectDeleteError::InvalidPath(error.to_string()))?;
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ProjectDeleteError::InvalidPath(relative_path.into()));
        }
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        let parent = self.validate_parent(parent, relative_path)?;
        let destination = parent.join(relative.file_name().expect("validated non-empty path"));
        let metadata = fs::symlink_metadata(&destination).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ProjectDeleteError::NotFound(relative_path.into())
            } else {
                ProjectDeleteError::Io {
                    operation: "inspect",
                    path: relative_path.into(),
                    message: error.to_string(),
                }
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ProjectDeleteError::SymlinkNotAllowed(relative_path.into()));
        }
        if !metadata.is_file() {
            return Err(ProjectDeleteError::NotRegularFile(relative_path.into()));
        }
        let bytes = fs::read(&destination).map_err(|error| ProjectDeleteError::Io {
            operation: "read",
            path: relative_path.into(),
            message: error.to_string(),
        })?;
        let actual = fingerprint_bytes(&bytes);
        if actual != expected_fingerprint {
            return Err(ProjectDeleteError::FingerprintMismatch {
                expected: expected_fingerprint,
                actual,
            });
        }
        fs::remove_file(&destination).map_err(|error| ProjectDeleteError::Io {
            operation: "delete",
            path: relative_path.into(),
            message: error.to_string(),
        })?;
        Ok(destination)
    }

    fn validate_parent(
        &self,
        relative: &Path,
        requested: &str,
    ) -> Result<PathBuf, ProjectDeleteError> {
        let mut current = self.workspace_root.clone();
        for component in relative.components() {
            let Component::Normal(component) = component else {
                return Err(ProjectDeleteError::InvalidPath(requested.into()));
            };
            current.push(component);
            let metadata = fs::symlink_metadata(&current).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    ProjectDeleteError::NotFound(requested.into())
                } else {
                    ProjectDeleteError::Io {
                        operation: "inspect parent",
                        path: requested.into(),
                        message: error.to_string(),
                    }
                }
            })?;
            if metadata.file_type().is_symlink() {
                return Err(ProjectDeleteError::SymlinkNotAllowed(requested.into()));
            }
            if !metadata.is_dir() {
                return Err(ProjectDeleteError::NotRegularFile(requested.into()));
            }
        }
        let canonical = fs::canonicalize(&current).map_err(|error| ProjectDeleteError::Io {
            operation: "canonicalize parent",
            path: requested.into(),
            message: error.to_string(),
        })?;
        if !canonical.starts_with(&self.workspace_root) {
            return Err(ProjectDeleteError::OutsideWorkspace(requested.into()));
        }
        Ok(canonical)
    }
}
