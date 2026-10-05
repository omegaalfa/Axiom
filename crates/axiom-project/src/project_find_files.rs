//! Bounded workspace-relative file-name discovery.

use std::{fmt, fs, io, path::{Component, Path, PathBuf}};

use crate::project_search::{glob_matches, is_ignored_directory};

pub const MAX_FIND_FILES_MATCHES: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindFilesRequest {
    pub pattern: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindFilesOutput {
    pub matches: Vec<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindFilesError {
    EmptyPattern,
    InvalidPattern(String),
    OutsideWorkspace(String),
    Io { path: String, message: String },
}

impl fmt::Display for FindFilesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPattern => f.write_str("pattern must not be empty"),
            Self::InvalidPattern(pattern) => write!(f, "invalid file pattern: {pattern}"),
            Self::OutsideWorkspace(pattern) => {
                write!(f, "pattern is outside workspace: {pattern}")
            }
            Self::Io { path, message } => write!(f, "failed to find files in {path}: {message}"),
        }
    }
}

impl std::error::Error for FindFilesError {}

#[derive(Clone, Debug)]
pub struct ProjectFindFilesCapability {
    workspace_root: PathBuf,
}

impl ProjectFindFilesCapability {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let workspace_root = fs::canonicalize(workspace_root)?;
        if !workspace_root.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "workspace root is not a directory"));
        }
        Ok(Self { workspace_root })
    }

    pub fn find_files(&self, request: FindFilesRequest) -> Result<FindFilesOutput, FindFilesError> {
        self.find_files_with_cancel(request, || false)
    }

    pub fn find_files_with_cancel<F>(
        &self,
        request: FindFilesRequest,
        cancelled: F,
    ) -> Result<FindFilesOutput, FindFilesError>
    where
        F: Fn() -> bool,
    {
        validate_pattern(&request.pattern)?;
        let mut matches = Vec::new();
        let mut truncated = false;
        self.visit(&self.workspace_root, &request.pattern, &cancelled, &mut matches, &mut truncated)?;
        Ok(FindFilesOutput { matches, truncated })
    }

    fn visit<F>(
        &self,
        path: &Path,
        pattern: &str,
        cancelled: &F,
        matches: &mut Vec<String>,
        truncated: &mut bool,
    ) -> Result<(), FindFilesError>
    where
        F: Fn() -> bool,
    {
        if cancelled() || *truncated {
            return Ok(());
        }
        let metadata = fs::symlink_metadata(path).map_err(|error| FindFilesError::Io {
            path: self.display_path(path),
            message: error.to_string(),
        })?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if metadata.is_dir() {
            let mut entries = fs::read_dir(path)
                .map_err(|error| FindFilesError::Io {
                    path: self.display_path(path),
                    message: error.to_string(),
                })?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                if is_ignored_directory(&entry.file_name()) {
                    continue;
                }
                self.visit(&entry.path(), pattern, cancelled, matches, truncated)?;
                if cancelled() || *truncated {
                    break;
                }
            }
            return Ok(());
        }
        if !metadata.is_file() {
            return Ok(());
        }
        let relative = path.strip_prefix(&self.workspace_root).unwrap_or(path);
        let relative = relative.to_string_lossy().replace('\\', "/");
        let candidate = if pattern.contains('/') {
            relative.clone()
        } else {
            Path::new(&relative)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned()
        };
        if glob_matches(pattern, &candidate) {
            if matches.len() == MAX_FIND_FILES_MATCHES {
                *truncated = true;
            } else {
                matches.push(relative);
            }
        }
        Ok(())
    }

    fn display_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.workspace_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

fn validate_pattern(pattern: &str) -> Result<(), FindFilesError> {
    if pattern.is_empty() {
        return Err(FindFilesError::EmptyPattern);
    }
    if pattern.contains('\0') {
        return Err(FindFilesError::InvalidPattern(pattern.into()));
    }
    let path = Path::new(pattern);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(component, Component::Prefix(_) | Component::RootDir | Component::ParentDir)
        })
    {
        return Err(FindFilesError::OutsideWorkspace(pattern.into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn capability() -> (tempfile::TempDir, ProjectFindFilesCapability) {
        let dir = tempdir().unwrap();
        let capability = ProjectFindFilesCapability::new(dir.path()).unwrap();
        (dir, capability)
    }

    #[test]
    fn supports_required_globs_and_deterministic_order() {
        let (dir, capability) = capability();
        fs::create_dir_all(dir.path().join("src/App")).unwrap();
        fs::create_dir_all(dir.path().join("Lib")).unwrap();
        for path in ["z.php", "App/ProductService.php", "App/UserService.php", "Lib/Service.php", "src/App/Service.php", "src/lib.rs"] {
            let path = dir.path().join(path);
            if let Some(parent) = path.parent() { fs::create_dir_all(parent).unwrap(); }
            fs::write(path, "").unwrap();
        }
        assert_eq!(capability.find_files(FindFilesRequest { pattern: "*.php".into() }).unwrap().matches, vec!["App/ProductService.php", "App/UserService.php", "Lib/Service.php", "src/App/Service.php", "z.php"]);
        assert_eq!(capability.find_files(FindFilesRequest { pattern: "*Service.php".into() }).unwrap().matches.len(), 4);
        assert_eq!(capability.find_files(FindFilesRequest { pattern: "**/*Service.php".into() }).unwrap().matches.len(), 4);
        assert_eq!(capability.find_files(FindFilesRequest { pattern: "src/**/*.rs".into() }).unwrap().matches, vec!["src/lib.rs"]);
        assert_eq!(capability.find_files(FindFilesRequest { pattern: "App/*.php".into() }).unwrap().matches, vec!["App/ProductService.php", "App/UserService.php"]);
    }

    #[test]
    fn bounds_ignored_and_rejects_escape() {
        let (dir, capability) = capability();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/ignored.php"), "").unwrap();
        for index in 0..201 { fs::write(dir.path().join(format!("{index:03}.php")), "").unwrap(); }
        let output = capability.find_files(FindFilesRequest { pattern: "*.php".into() }).unwrap();
        assert_eq!(output.matches.len(), 200);
        assert!(output.truncated);
        assert!(matches!(capability.find_files(FindFilesRequest { pattern: "../*.php".into() }), Err(FindFilesError::OutsideWorkspace(_))));
        let no_matches = capability.find_files(FindFilesRequest { pattern: "*.does-not-exist".into() }).unwrap();
        assert!(no_matches.matches.is_empty());
        assert!(!no_matches.truncated);
    }

    #[test]
    fn cancellation_stops_traversal() {
        let (_dir, capability) = capability();
        let output = capability.find_files_with_cancel(FindFilesRequest { pattern: "*.php".into() }, || true).unwrap();
        assert!(output.matches.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_not_returned() {
        use std::os::unix::fs::symlink;

        let (dir, capability) = capability();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("outside.php"), "").unwrap();
        symlink(outside.path().join("outside.php"), dir.path().join("escaped.php")).unwrap();

        let output = capability.find_files(FindFilesRequest { pattern: "*.php".into() }).unwrap();
        assert!(output.matches.is_empty());
    }
}
