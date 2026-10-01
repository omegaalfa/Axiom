//! Headless, workspace-contained fingerprint-guarded updates of UTF-8 text files.

use std::{
    fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    str::FromStr,
};

use sha2::{Digest, Sha256};

use crate::project_write::{
    prepare_temporary_text_file, validate_relative_path, ProjectWriteCapability, ProjectWriteError,
    MAX_CREATE_TEXT_BYTES,
};

pub const MAX_UPDATE_TEXT_BYTES: usize = MAX_CREATE_TEXT_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextFileFingerprint([u8; 32]);

impl TextFileFingerprint {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_wire_string(&self) -> String {
        format!("sha256:{self}")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TextFileFingerprintParseError;

impl fmt::Display for TextFileFingerprintParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("fingerprint must be sha256:<64 lowercase hex chars>")
    }
}

impl std::error::Error for TextFileFingerprintParseError {}

impl FromStr for TextFileFingerprint {
    type Err = TextFileFingerprintParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(TextFileFingerprintParseError);
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(TextFileFingerprintParseError);
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (hex_value(pair[0]) << 4) | hex_value(pair[1]);
        }
        Ok(Self(bytes))
    }
}

const fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

impl fmt::Display for TextFileFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProjectUpdateError {
    InvalidPath(String),
    OutsideWorkspace(String),
    ParentNotFound(String),
    ParentNotDirectory(String),
    SymlinkNotAllowed(String),
    AlreadyExists(String),
    NotFound(String),
    NotRegularFile(String),
    UnsupportedEncoding(String),
    CurrentFileTooLarge { limit: usize, actual: usize },
    ContentTooLarge { limit: usize, actual: usize },
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

impl fmt::Display for ProjectUpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid project-relative path: {path}"),
            Self::OutsideWorkspace(path) => {
                write!(f, "path is outside the project workspace: {path}")
            }
            Self::ParentNotFound(path) => write!(f, "parent directory not found: {path}"),
            Self::ParentNotDirectory(path) => {
                write!(f, "parent path is not a directory: {path}")
            }
            Self::SymlinkNotAllowed(path) => {
                write!(f, "symbolic links are not allowed in update paths: {path}")
            }
            Self::AlreadyExists(path) => write!(f, "path already exists: {path}"),
            Self::NotFound(path) => write!(f, "file not found: {path}"),
            Self::NotRegularFile(path) => write!(f, "path is not a regular file: {path}"),
            Self::UnsupportedEncoding(path) => {
                write!(f, "file is not valid UTF-8 text: {path}")
            }
            Self::CurrentFileTooLarge { limit, actual } => write!(
                f,
                "current file is too large: {actual} bytes exceeds {limit}"
            ),
            Self::ContentTooLarge { limit, actual } => write!(
                f,
                "text content is too large: {actual} bytes exceeds {limit}"
            ),
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

impl std::error::Error for ProjectUpdateError {}

impl From<ProjectWriteError> for ProjectUpdateError {
    fn from(error: ProjectWriteError) -> Self {
        match error {
            ProjectWriteError::InvalidPath(path) => Self::InvalidPath(path),
            ProjectWriteError::OutsideWorkspace(path) => Self::OutsideWorkspace(path),
            ProjectWriteError::ParentNotFound(path) => Self::ParentNotFound(path),
            ProjectWriteError::ParentNotDirectory(path) => Self::ParentNotDirectory(path),
            ProjectWriteError::SymlinkNotAllowed(path) => Self::SymlinkNotAllowed(path),
            ProjectWriteError::AlreadyExists(path) => Self::AlreadyExists(path),
            ProjectWriteError::ContentTooLarge { limit, actual } => {
                Self::ContentTooLarge { limit, actual }
            }
            ProjectWriteError::Io {
                operation,
                path,
                message,
            } => Self::Io {
                operation,
                path,
                message,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProjectUpdateCapability {
    writes: ProjectWriteCapability,
}

impl ProjectUpdateCapability {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            writes: ProjectWriteCapability::new(workspace_root)?,
        })
    }

    pub fn fingerprint_text_file(
        &self,
        relative_path: &str,
    ) -> Result<TextFileFingerprint, ProjectUpdateError> {
        let (destination, _) = self.resolve_destination(relative_path)?;
        let bytes = self.read_existing_text(&destination, relative_path)?;
        Ok(fingerprint_bytes(&bytes))
    }

    pub fn update_text_file(
        &self,
        relative_path: &str,
        expected_fingerprint: TextFileFingerprint,
        new_content: String,
    ) -> Result<PathBuf, ProjectUpdateError> {
        if new_content.len() > MAX_UPDATE_TEXT_BYTES {
            return Err(ProjectUpdateError::ContentTooLarge {
                limit: MAX_UPDATE_TEXT_BYTES,
                actual: new_content.len(),
            });
        }

        let (destination, parent) = self.resolve_destination(relative_path)?;
        let current = fingerprint_bytes(&self.read_existing_text(&destination, relative_path)?);
        if current != expected_fingerprint {
            return Err(ProjectUpdateError::FingerprintMismatch {
                expected: expected_fingerprint,
                actual: current,
            });
        }

        let temporary = prepare_temporary_text_file(
            &parent,
            &destination,
            new_content.as_bytes(),
        )?;

        let current = fingerprint_bytes(&self.read_existing_text(&destination, relative_path)?);
        if current != expected_fingerprint {
            return Err(ProjectUpdateError::FingerprintMismatch {
                expected: expected_fingerprint,
                actual: current,
            });
        }

        match temporary.persist(&destination) {
            Ok(file) => {
                drop(file);
                Ok(destination)
            }
            Err(error) => Err(ProjectUpdateError::Io {
                operation: "publish",
                path: destination.display().to_string(),
                message: error.error.to_string(),
            }),
        }
    }

    fn resolve_destination(
        &self,
        relative_path: &str,
    ) -> Result<(PathBuf, PathBuf), ProjectUpdateError> {
        let relative_path = validate_relative_path(relative_path)?;
        let parent_relative = relative_path.parent().unwrap_or_else(|| Path::new(""));
        let parent = self.writes.validate_parent(parent_relative)?;
        let destination = self.writes.workspace_root().join(relative_path);
        Ok((destination, parent))
    }

    fn read_existing_text(
        &self,
        destination: &Path,
        display_path: &str,
    ) -> Result<Vec<u8>, ProjectUpdateError> {
        let metadata = fs::symlink_metadata(destination).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ProjectUpdateError::NotFound(display_path.to_owned())
            } else {
                ProjectUpdateError::Io {
                    operation: "inspect",
                    path: display_path.to_owned(),
                    message: error.to_string(),
                }
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ProjectUpdateError::SymlinkNotAllowed(
                display_path.to_owned(),
            ));
        }
        if !metadata.is_file() {
            return Err(ProjectUpdateError::NotRegularFile(
                display_path.to_owned(),
            ));
        }

        let file = fs::File::open(destination).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ProjectUpdateError::NotFound(display_path.to_owned())
            } else {
                ProjectUpdateError::Io {
                    operation: "open",
                    path: display_path.to_owned(),
                    message: error.to_string(),
                }
            }
        })?;
        let file_metadata = file.metadata().map_err(|error| ProjectUpdateError::Io {
            operation: "inspect open file",
            path: display_path.to_owned(),
            message: error.to_string(),
        })?;
        if !file_metadata.is_file() {
            return Err(ProjectUpdateError::NotRegularFile(
                display_path.to_owned(),
            ));
        }
        if file_metadata.len() > MAX_UPDATE_TEXT_BYTES as u64 {
            return Err(ProjectUpdateError::CurrentFileTooLarge {
                limit: MAX_UPDATE_TEXT_BYTES,
                actual: usize::try_from(file_metadata.len()).unwrap_or(usize::MAX),
            });
        }

        let mut bytes = Vec::with_capacity(file_metadata.len() as usize);
        let mut limited = file.take(MAX_UPDATE_TEXT_BYTES as u64 + 1);
        io::Read::read_to_end(&mut limited, &mut bytes).map_err(|error| {
            ProjectUpdateError::Io {
                operation: "read",
                path: display_path.to_owned(),
                message: error.to_string(),
            }
        })?;
        if bytes.len() > MAX_UPDATE_TEXT_BYTES {
            return Err(ProjectUpdateError::CurrentFileTooLarge {
                limit: MAX_UPDATE_TEXT_BYTES,
                actual: bytes.len(),
            });
        }
        if std::str::from_utf8(&bytes).is_err() {
            return Err(ProjectUpdateError::UnsupportedEncoding(
                display_path.to_owned(),
            ));
        }
        Ok(bytes)
    }
}

pub(crate) fn fingerprint_bytes(bytes: &[u8]) -> TextFileFingerprint {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut fingerprint = [0_u8; 32];
    fingerprint.copy_from_slice(&digest);
    TextFileFingerprint(fingerprint)
}
