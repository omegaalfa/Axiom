//! Safe, bounded, non-recursive directory listings inside a workspace.

use std::{
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

pub const MAX_DIRECTORY_ENTRIES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: DirectoryEntryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListDirectoryOutput {
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListDirectoryError {
    InvalidPath(String),
    NotFound(String),
    NotDirectory(String),
    TooManyEntries { path: String, limit: usize },
    Io { path: String, message: String },
}

impl fmt::Display for ListDirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid directory path: {path}"),
            Self::NotFound(path) => write!(f, "directory not found: {path}"),
            Self::NotDirectory(path) => write!(f, "path is not a directory: {path}"),
            Self::TooManyEntries { path, limit } => {
                write!(f, "directory has more than {limit} entries: {path}")
            }
            Self::Io { path, message } => write!(f, "failed to list {path}: {message}"),
        }
    }
}

impl std::error::Error for ListDirectoryError {}

#[derive(Clone, Debug)]
pub struct ProjectDirectoryCapability {
    workspace_root: PathBuf,
}

impl ProjectDirectoryCapability {
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

    pub fn list_directory(
        &self,
        path: impl AsRef<str>,
    ) -> Result<ListDirectoryOutput, ListDirectoryError> {
        let input = path.as_ref();
        let resolved = self.resolve_inside_workspace(input)?;
        let metadata = fs::metadata(&resolved).map_err(|error| ListDirectoryError::Io {
            path: input.to_owned(),
            message: error.to_string(),
        })?;
        if !metadata.is_dir() {
            return Err(ListDirectoryError::NotDirectory(input.to_owned()));
        }

        let mut entries = Vec::new();
        let mut directory = fs::read_dir(&resolved).map_err(|error| ListDirectoryError::Io {
            path: input.to_owned(),
            message: error.to_string(),
        })?;
        while let Some(entry) = directory.next() {
            let entry = entry.map_err(|error| ListDirectoryError::Io {
                path: input.to_owned(),
                message: error.to_string(),
            })?;
            if entries.len() >= MAX_DIRECTORY_ENTRIES {
                return Err(ListDirectoryError::TooManyEntries {
                    path: input.to_owned(),
                    limit: MAX_DIRECTORY_ENTRIES,
                });
            }
            let kind = entry.file_type().map_err(|error| ListDirectoryError::Io {
                path: input.to_owned(),
                message: error.to_string(),
            })?;
            let kind = if kind.is_symlink() {
                DirectoryEntryKind::Symlink
            } else if kind.is_dir() {
                DirectoryEntryKind::Directory
            } else if kind.is_file() {
                DirectoryEntryKind::File
            } else {
                DirectoryEntryKind::Other
            };
            entries.push(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind,
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(ListDirectoryOutput {
            path: input.to_owned(),
            entries,
        })
    }

    fn resolve_inside_workspace(&self, input: &str) -> Result<PathBuf, ListDirectoryError> {
        let path = Path::new(input);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        {
            return Err(ListDirectoryError::InvalidPath(input.to_owned()));
        }
        let mut lexical = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir if !lexical.pop() => {
                    return Err(ListDirectoryError::InvalidPath(input.to_owned()));
                }
                Component::ParentDir => {}
                Component::Normal(value) => lexical.push(value),
                Component::Prefix(_) | Component::RootDir => unreachable!(),
            }
        }
        let candidate = self.workspace_root.join(lexical);
        let resolved = fs::canonicalize(&candidate).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ListDirectoryError::NotFound(input.to_owned())
            } else {
                ListDirectoryError::Io {
                    path: input.to_owned(),
                    message: error.to_string(),
                }
            }
        })?;
        if !resolved.starts_with(&self.workspace_root) {
            return Err(ListDirectoryError::InvalidPath(input.to_owned()));
        }
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn capability() -> (tempfile::TempDir, ProjectDirectoryCapability) {
        let dir = tempdir().unwrap();
        let capability = ProjectDirectoryCapability::new(dir.path()).unwrap();
        (dir, capability)
    }

    #[test]
    fn lists_sorted_entries_without_recursion() {
        let (dir, capability) = capability();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("z.txt"), "z").unwrap();
        fs::write(dir.path().join("A.php"), "a").unwrap();
        let output = capability.list_directory(".").unwrap();
        assert_eq!(
            output
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["A.php", "nested", "z.txt"]
        );
        assert_eq!(output.entries[1].kind, DirectoryEntryKind::Directory);
    }

    #[test]
    fn lists_empty_directory_and_unicode_names() {
        let (dir, capability) = capability();
        fs::create_dir(dir.path().join("App")).unwrap();
        assert!(capability.list_directory("App").unwrap().entries.is_empty());
        fs::write(dir.path().join("\u{00e1}.php"), "x").unwrap();
        assert!(
            capability
                .list_directory(".")
                .unwrap()
                .entries
                .iter()
                .any(|entry| entry.name == "\u{00e1}.php")
        );
    }

    #[test]
    fn preserves_nested_paths_and_distinguishes_root_app() {
        let (dir, capability) = capability();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::create_dir_all(dir.path().join("src/App")).unwrap();
        fs::write(dir.path().join("App/Root.php"), "root").unwrap();
        fs::write(dir.path().join("src/App/FileStone.php"), "nested").unwrap();

        let src = capability.list_directory("src").unwrap();
        assert_eq!(src.path, "src");
        assert!(
            src.entries
                .iter()
                .any(|entry| entry.name == "App" && entry.kind == DirectoryEntryKind::Directory)
        );

        let nested = capability.list_directory("src/App").unwrap();
        assert_eq!(nested.path, "src/App");
        assert!(
            nested
                .entries
                .iter()
                .any(|entry| entry.name == "FileStone.php")
        );
        assert!(!nested.entries.iter().any(|entry| entry.name == "Root.php"));
    }

    #[test]
    fn rejects_invalid_paths_files_and_limits() {
        let (dir, capability) = capability();
        assert!(matches!(
            capability.list_directory("missing"),
            Err(ListDirectoryError::NotFound(_))
        ));
        fs::write(dir.path().join("file"), "x").unwrap();
        assert!(matches!(
            capability.list_directory("file"),
            Err(ListDirectoryError::NotDirectory(_))
        ));
        assert!(matches!(
            capability.list_directory("../outside"),
            Err(ListDirectoryError::InvalidPath(_))
        ));
        assert!(matches!(
            capability.list_directory(&dir.path().display().to_string()),
            Err(ListDirectoryError::InvalidPath(_))
        ));
        for index in 0..=MAX_DIRECTORY_ENTRIES {
            fs::write(dir.path().join(format!("entry-{index}")), "x").unwrap();
        }
        assert!(matches!(
            capability.list_directory("."),
            Err(ListDirectoryError::TooManyEntries { .. })
        ));
    }

    #[test]
    fn rejects_symlink_escape_without_following_outside_workspace() {
        let (workspace, capability) = capability();
        let outside = tempdir().unwrap();
        fs::create_dir(outside.path().join("secret")).unwrap();
        let link = workspace.path().join("escape");
        let created = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(outside.path().join("secret"), &link)
            }
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(outside.path().join("secret"), &link)
            }
        };
        if created.is_err() {
            return;
        }
        assert!(matches!(
            capability.list_directory("escape"),
            Err(ListDirectoryError::InvalidPath(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_drive_and_unc_paths() {
        let (_dir, capability) = capability();
        assert!(matches!(
            capability.list_directory("C:\\Windows"),
            Err(ListDirectoryError::InvalidPath(_))
        ));
        assert!(matches!(
            capability.list_directory("\\\\server\\share"),
            Err(ListDirectoryError::InvalidPath(_))
        ));
    }
}
