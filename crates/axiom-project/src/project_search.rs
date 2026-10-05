//! Bounded, literal text search inside a workspace.

use std::{fmt, fs, io, path::{Component, Path, PathBuf}};

use crate::project_read::MAX_READ_FILE_BYTES;

pub const MAX_SEARCH_MATCHES: usize = 100;
pub const MAX_SEARCH_PREVIEW_BYTES: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextRequest {
    pub query: String,
    pub path: Option<String>,
    pub file_pattern: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextMatch {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextOutput {
    pub matches: Vec<SearchTextMatch>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchTextError {
    EmptyQuery,
    InvalidPath(String),
    OutsideWorkspace(String),
    NotFound(String),
    InvalidPattern(String),
    Io { path: String, message: String },
}

impl fmt::Display for SearchTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyQuery => write!(f, "query must not be empty"),
            Self::InvalidPath(path) => write!(f, "invalid path: {path}"),
            Self::OutsideWorkspace(path) => write!(f, "path is outside workspace: {path}"),
            Self::NotFound(path) => write!(f, "path not found: {path}"),
            Self::InvalidPattern(pattern) => write!(f, "invalid file pattern: {pattern}"),
            Self::Io { path, message } => write!(f, "failed to search {path}: {message}"),
        }
    }
}

impl std::error::Error for SearchTextError {}

#[derive(Clone, Debug)]
pub struct ProjectSearchCapability {
    workspace_root: PathBuf,
}

impl ProjectSearchCapability {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let workspace_root = fs::canonicalize(workspace_root)?;
        if !workspace_root.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "workspace root is not a directory"));
        }
        Ok(Self { workspace_root })
    }

    pub fn search_text(&self, request: SearchTextRequest) -> Result<SearchTextOutput, SearchTextError> {
        self.search_text_with_cancel(request, || false)
    }

    pub fn search_text_with_cancel<F>(
        &self,
        request: SearchTextRequest,
        cancelled: F,
    ) -> Result<SearchTextOutput, SearchTextError>
    where
        F: Fn() -> bool,
    {
        if request.query.is_empty() {
            return Err(SearchTextError::EmptyQuery);
        }
        if let Some(pattern) = request.file_pattern.as_deref() {
            if pattern.is_empty() || pattern.contains('\0') {
                return Err(SearchTextError::InvalidPattern(pattern.to_owned()));
            }
        }
        let scope = request.path.as_deref().unwrap_or("");
        let root = self.resolve_scope(scope)?;
        let mut matches = Vec::new();
        let mut truncated = false;
        self.visit(
            &root,
            &request,
            &cancelled,
            &mut matches,
            &mut truncated,
        )?;
        Ok(SearchTextOutput { matches, truncated })
    }

    fn resolve_scope(&self, input: &str) -> Result<PathBuf, SearchTextError> {
        let path = Path::new(input);
        if path.is_absolute()
            || path.components().any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        {
            return Err(SearchTextError::InvalidPath(input.to_owned()));
        }
        let mut lexical = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir if !lexical.pop() => {
                    return Err(SearchTextError::OutsideWorkspace(input.to_owned()));
                }
                Component::ParentDir => {}
                Component::Normal(value) => lexical.push(value),
                Component::Prefix(_) | Component::RootDir => unreachable!(),
            }
        }
        let candidate = self.workspace_root.join(lexical);
        let resolved = fs::canonicalize(&candidate).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SearchTextError::NotFound(input.to_owned())
            } else {
                SearchTextError::Io { path: input.to_owned(), message: error.to_string() }
            }
        })?;
        if !resolved.starts_with(&self.workspace_root) {
            return Err(SearchTextError::OutsideWorkspace(input.to_owned()));
        }
        Ok(resolved)
    }

    fn visit<F>(
        &self,
        path: &Path,
        request: &SearchTextRequest,
        cancelled: &F,
        matches: &mut Vec<SearchTextMatch>,
        truncated: &mut bool,
    ) -> Result<(), SearchTextError>
    where
        F: Fn() -> bool,
    {
        if cancelled() || *truncated {
            return Ok(());
        }
        let metadata = fs::symlink_metadata(path).map_err(|error| SearchTextError::Io {
            path: self.display_path(path), message: error.to_string(),
        })?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if metadata.is_dir() {
            let mut entries = fs::read_dir(path)
                .map_err(|error| SearchTextError::Io { path: self.display_path(path), message: error.to_string() })?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                if is_ignored_directory(&entry.file_name()) {
                    continue;
                }
                self.visit(&entry.path(), request, cancelled, matches, truncated)?;
                if *truncated || cancelled() {
                    break;
                }
            }
            return Ok(());
        }
        if !metadata.is_file() || metadata.len() > MAX_READ_FILE_BYTES {
            return Ok(());
        }
        let relative = path.strip_prefix(&self.workspace_root).unwrap_or(path);
        let relative_display = relative.to_string_lossy().replace('\\', "/");
        if let Some(pattern) = request.file_pattern.as_deref() {
            let candidate = if pattern.contains('/') {
                relative_display.clone()
            } else {
                path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned()
            };
            if !glob_matches(pattern, &candidate) {
                return Ok(());
            }
        }
        let bytes = fs::read(path).map_err(|error| SearchTextError::Io {
            path: relative_display.clone(), message: error.to_string(),
        })?;
        let Ok(content) = String::from_utf8(bytes) else {
            return Ok(());
        };
        for (offset, _) in content.match_indices(&request.query) {
            if matches.len() >= MAX_SEARCH_MATCHES {
                *truncated = true;
                break;
            }
            let line_start = content[..offset].rfind('\n').map_or(0, |index| index + 1);
            let line = content[line_start..].split('\n').next().unwrap_or("").trim_end_matches('\r');
            let column = line[..offset - line_start].chars().count() + 1;
            matches.push(SearchTextMatch {
                path: relative_display.clone(),
                line: content[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1,
                column,
                preview: bounded_preview(line),
            });
        }
        Ok(())
    }

    fn display_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.workspace_root).unwrap_or(path).to_string_lossy().replace('\\', "/")
    }
}

pub(crate) fn is_ignored_directory(name: &std::ffi::OsStr) -> bool {
    matches!(name.to_str(), Some(".git" | "target" | "node_modules" | "vendor"))
}

fn bounded_preview(line: &str) -> String {
    let mut end = line.len().min(MAX_SEARCH_PREVIEW_BYTES);
    while end > 0 && !line.is_char_boundary(end) { end -= 1; }
    if end < line.len() { format!("{}…", &line[..end]) } else { line.to_owned() }
}

pub(crate) fn glob_matches(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        if pattern.starts_with(b"**/") {
            return matches(&pattern[3..], value)
                || (!value.is_empty() && matches(pattern, &value[1..]));
        }
        match pattern.split_first() {
            None => value.is_empty(),
            Some((b'*', rest)) if rest.first() == Some(&b'*') => {
                matches(&rest[1..], value) || (!value.is_empty() && matches(pattern, &value[1..]))
            }
            Some((b'*', rest)) => {
                matches(rest, value) || (!value.is_empty() && value[0] != b'/' && matches(pattern, &value[1..]))
            }
            Some((b'?', rest)) => !value.is_empty() && value[0] != b'/' && matches(rest, &value[1..]),
            Some((character, rest)) => !value.is_empty() && *character == value[0] && matches(rest, &value[1..]),
        }
    }
    if !pattern.contains('/') { matches(pattern.as_bytes(), value.as_bytes()) }
    else { matches(pattern.as_bytes(), value.as_bytes()) || (pattern.starts_with("**/") && matches(&pattern.as_bytes()[3..], value.as_bytes())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(windows)]
    use std::os::windows::fs::symlink_file;
    use tempfile::tempdir;

    fn capability() -> (tempfile::TempDir, ProjectSearchCapability) {
        let dir = tempdir().unwrap();
        let capability = ProjectSearchCapability::new(dir.path()).unwrap();
        (dir, capability)
    }

    #[test]
    fn finds_literal_matches_with_line_and_utf8_column() {
        let (dir, capability) = capability();
        fs::write(dir.path().join("main.txt"), "α ProductService\nuso ProductService\n").unwrap();
        let output = capability.search_text(SearchTextRequest { query: "ProductService".into(), path: None, file_pattern: None }).unwrap();
        assert_eq!(output.matches[0].line, 1);
        assert_eq!(output.matches[0].column, 3);
        assert_eq!(output.matches[1].line, 2);
    }

    #[test]
    fn scopes_patterns_sort_and_truncate() {
        let (dir, capability) = capability();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("z.rs"), "hit\n").unwrap();
        fs::write(dir.path().join("src/a.rs"), "hit\nhit\n").unwrap();
        fs::write(dir.path().join("src/a.php"), "hit\n").unwrap();
        let output = capability.search_text(SearchTextRequest { query: "hit".into(), path: Some("src".into()), file_pattern: Some("**/*.rs".into()) }).unwrap();
        assert_eq!(output.matches.len(), 2);
        assert!(!output.truncated);
        assert_eq!(output.matches[0].path, "src/a.rs");
    }

    #[test]
    fn rejects_empty_and_escape_and_skips_binary_ignored_and_symlink() {
        let (dir, capability) = capability();
        fs::create_dir(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/ignored.txt"), "hit").unwrap();
        fs::write(dir.path().join("binary"), [0, 159, 146, 150]).unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("outside.txt"), "hit").unwrap();
        #[cfg(unix)]
        symlink(outside.path().join("outside.txt"), dir.path().join("link.txt")).unwrap();
        #[cfg(windows)]
        symlink_file(outside.path().join("outside.txt"), dir.path().join("link.txt")).unwrap();
        assert_eq!(capability.search_text(SearchTextRequest { query: String::new(), path: None, file_pattern: None }), Err(SearchTextError::EmptyQuery));
        assert!(matches!(capability.search_text(SearchTextRequest { query: "hit".into(), path: Some("../".into()), file_pattern: None }), Err(SearchTextError::OutsideWorkspace(_))));
        assert!(capability.search_text(SearchTextRequest { query: "hit".into(), path: None, file_pattern: None }).unwrap().matches.is_empty());
    }

    #[test]
    fn caps_results_and_reports_truncation() {
        let (dir, capability) = capability();
        fs::write(dir.path().join("many.txt"), "hit\n".repeat(MAX_SEARCH_MATCHES + 1)).unwrap();
        let output = capability.search_text(SearchTextRequest { query: "hit".into(), path: None, file_pattern: None }).unwrap();
        assert_eq!(output.matches.len(), MAX_SEARCH_MATCHES);
        assert!(output.truncated);
    }

    #[test]
    fn no_match_and_cancellation_are_bounded() {
        let (dir, capability) = capability();
        fs::write(dir.path().join("file.txt"), "present").unwrap();
        let none = capability.search_text(SearchTextRequest { query: "missing".into(), path: None, file_pattern: None }).unwrap();
        assert!(none.matches.is_empty());
        let cancelled = capability.search_text_with_cancel(
            SearchTextRequest { query: "present".into(), path: None, file_pattern: None },
            || true,
        ).unwrap();
        assert!(cancelled.matches.is_empty());
    }
}
