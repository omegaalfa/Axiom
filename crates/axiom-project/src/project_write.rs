//! Headless, workspace-contained creation of UTF-8 text files.

use std::{
    fmt, fs, io,
    io::Write,
    path::{Component, Path, PathBuf},
};

use tempfile::NamedTempFile;

pub const MAX_CREATE_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum ProjectWriteError {
    InvalidPath(String),
    OutsideWorkspace(String),
    ParentNotFound(String),
    ParentNotDirectory(String),
    SymlinkNotAllowed(String),
    AlreadyExists(String),
    ContentTooLarge { limit: usize, actual: usize },
    Io {
        operation: &'static str,
        path: String,
        message: String,
    },
}

impl fmt::Display for ProjectWriteError {
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
                write!(f, "symbolic links are not allowed in write paths: {path}")
            }
            Self::AlreadyExists(path) => write!(f, "path already exists: {path}"),
            Self::ContentTooLarge { limit, actual } => write!(
                f,
                "text content is too large: {actual} bytes exceeds {limit}"
            ),
            Self::Io {
                operation,
                path,
                message,
            } => write!(f, "failed to {operation} {path}: {message}"),
        }
    }
}

impl std::error::Error for ProjectWriteError {}

#[derive(Clone, Debug)]
pub struct ProjectWriteCapability {
    workspace_root: PathBuf,
}

impl ProjectWriteCapability {
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

    pub fn create_text_file(
        &self,
        relative_path: &str,
        content: String,
    ) -> Result<PathBuf, ProjectWriteError> {
        if content.len() > MAX_CREATE_TEXT_BYTES {
            return Err(ProjectWriteError::ContentTooLarge {
                limit: MAX_CREATE_TEXT_BYTES,
                actual: content.len(),
            });
        }

        let relative_path = validate_relative_path(relative_path)?;
        let destination = self.workspace_root.join(&relative_path);
        let parent_relative = relative_path.parent().unwrap_or_else(|| Path::new(""));
        let parent = self.validate_parent(parent_relative)?;
        let temporary = prepare_temporary_text_file(&parent, &destination, content.as_bytes())?;

        match temporary.persist_noclobber(&destination) {
            Ok(file) => {
                drop(file);
                Ok(destination)
            }
            Err(error) => {
                let kind = error.error.kind();
                let message = error.to_string();
                if kind == io::ErrorKind::AlreadyExists {
                    Err(ProjectWriteError::AlreadyExists(
                        destination.display().to_string(),
                    ))
                } else {
                    Err(ProjectWriteError::Io {
                        operation: "publish",
                        path: destination.display().to_string(),
                        message,
                    })
                }
            }
        }
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub(crate) fn validate_parent(
        &self,
        relative_parent: &Path,
    ) -> Result<PathBuf, ProjectWriteError> {
        let mut current = self.workspace_root.clone();
        for component in relative_parent.components() {
            let Component::Normal(component) = component else {
                return Err(ProjectWriteError::InvalidPath(
                    relative_parent.display().to_string(),
                ));
            };
            current.push(component);
            let metadata = match fs::symlink_metadata(&current) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ProjectWriteError::ParentNotFound(
                        relative_parent.display().to_string(),
                    ));
                }
                Err(error) => {
                    return Err(ProjectWriteError::Io {
                        operation: "inspect parent directory",
                        path: current.display().to_string(),
                        message: error.to_string(),
                    });
                }
            };
            if metadata.file_type().is_symlink() {
                return Err(ProjectWriteError::SymlinkNotAllowed(
                    relative_parent.display().to_string(),
                ));
            }
            if !metadata.is_dir() {
                return Err(ProjectWriteError::ParentNotDirectory(
                    relative_parent.display().to_string(),
                ));
            }
        }
        let canonical = fs::canonicalize(&current).map_err(|error| ProjectWriteError::Io {
            operation: "canonicalize parent directory",
            path: current.display().to_string(),
            message: error.to_string(),
        })?;
        if !canonical.starts_with(&self.workspace_root) {
            return Err(ProjectWriteError::OutsideWorkspace(
                relative_parent.display().to_string(),
            ));
        }
        Ok(canonical)
    }
}

pub(crate) fn prepare_temporary_text_file(
    parent: &Path,
    destination: &Path,
    content: &[u8],
) -> Result<NamedTempFile, ProjectWriteError> {
    let mut temporary = NamedTempFile::new_in(parent).map_err(|error| ProjectWriteError::Io {
        operation: "create temporary file for",
        path: destination.display().to_string(),
        message: error.to_string(),
    })?;
    temporary.write_all(content).map_err(|error| ProjectWriteError::Io {
        operation: "write temporary file for",
        path: destination.display().to_string(),
        message: error.to_string(),
    })?;
    temporary.flush().map_err(|error| ProjectWriteError::Io {
        operation: "flush temporary file for",
        path: destination.display().to_string(),
        message: error.to_string(),
    })?;
    temporary.as_file().sync_all().map_err(|error| ProjectWriteError::Io {
        operation: "sync temporary file for",
        path: destination.display().to_string(),
        message: error.to_string(),
    })?;
    Ok(temporary)
}

pub(crate) fn validate_relative_path(input: &str) -> Result<PathBuf, ProjectWriteError> {
    if input.is_empty()
        || input.contains('\\')
        || input
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ProjectWriteError::InvalidPath(input.to_owned()));
    }
    let path = Path::new(input);
    if path.is_absolute()
        || path.components().any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ProjectWriteError::InvalidPath(input.to_owned()));
    }
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(ProjectWriteError::InvalidPath(input.to_owned()));
        };
        let Some(component) = component.to_str() else {
            return Err(ProjectWriteError::InvalidPath(input.to_owned()));
        };
        validate_name(component).map_err(|_| ProjectWriteError::InvalidPath(input.to_owned()))?;
    }
    Ok(path.to_path_buf())
}

fn validate_name(name: &str) -> Result<(), ()> {
    let path = Path::new(name);
    let mut components = path.components();
    let single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    let invalid_windows_character = name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    });
    let windows_stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let windows_reserved = matches!(windows_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (windows_stem.len() == 4
            && matches!(&windows_stem[..3], "COM" | "LPT")
            && matches!(windows_stem.as_bytes()[3], b'1'..=b'9'));
    if name.is_empty()
        || name == "."
        || name == ".."
        || !single_normal
        || invalid_windows_character
        || name.ends_with(['.', ' '])
        || windows_reserved
    {
        return Err(());
    }
    Ok(())
}
