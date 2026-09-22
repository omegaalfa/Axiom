//! Safe, headless reads of UTF-8 files inside a project workspace.
//!
//! This capability reads the filesystem only. It does not represent unsaved
//! editor state and has no dependency on GPUI, providers, or agent runtime
//! concepts.

use std::{
    fmt, fs, io,
    io::Read,
    path::{Component, Path, PathBuf},
};

use crate::{FileContent, read_file_content};

pub const MAX_READ_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadFileRange {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadFileRequest {
    pub path: String,
    pub range: Option<ReadFileRange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadFileMetadata {
    pub path: String,
    pub bytes: usize,
    pub truncated: bool,
    pub range: Option<ReadFileRange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadFileOutput {
    pub content: String,
    pub metadata: ReadFileMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadFileError {
    InvalidPath(String),
    OutsideWorkspace(String),
    NotFound(String),
    Directory(String),
    TooLarge {
        path: String,
        bytes: u64,
        limit: u64,
    },
    InvalidRange {
        start_line: usize,
        end_line: usize,
    },
    Io {
        path: String,
        message: String,
    },
    UnsupportedEncoding(String),
}

impl fmt::Display for ReadFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid path: {path}"),
            Self::OutsideWorkspace(path) => write!(f, "path is outside workspace: {path}"),
            Self::NotFound(path) => write!(f, "file not found: {path}"),
            Self::Directory(path) => write!(f, "path is a directory: {path}"),
            Self::TooLarge { path, bytes, limit } => {
                write!(
                    f,
                    "file is too large: {path} ({bytes} bytes, limit {limit})"
                )
            }
            Self::InvalidRange {
                start_line,
                end_line,
            } => {
                write!(f, "invalid line range: {start_line}..={end_line}")
            }
            Self::Io { path, message } => write!(f, "failed to read {path}: {message}"),
            Self::UnsupportedEncoding(path) => write!(f, "unsupported text encoding: {path}"),
        }
    }
}

impl std::error::Error for ReadFileError {}

#[derive(Clone, Debug)]
pub struct ProjectReadCapability {
    workspace_root: PathBuf,
}

impl ProjectReadCapability {
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

    pub fn read_file(&self, request: ReadFileRequest) -> Result<ReadFileOutput, ReadFileError> {
        let resolved = self.resolve_inside_workspace(&request.path)?;
        let metadata = match fs::metadata(&resolved) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ReadFileError::NotFound(request.path));
            }
            Err(error) => {
                return Err(ReadFileError::Io {
                    path: request.path,
                    message: error.to_string(),
                });
            }
        };
        if metadata.is_dir() {
            return Err(ReadFileError::Directory(request.path));
        }
        let bytes = metadata.len();
        if request.range.is_none() && bytes > MAX_READ_FILE_BYTES {
            return Err(ReadFileError::TooLarge {
                path: request.path,
                bytes,
                limit: MAX_READ_FILE_BYTES,
            });
        }
        let content = if let Some(range) = request.range.as_ref() {
            read_bounded_range(&resolved, range, bytes, &request.path)?
        } else {
            match read_file_content(&resolved) {
                Ok(FileContent::Text(content)) => content,
                Ok(FileContent::Binary | FileContent::UnsupportedEncoding) => {
                    return Err(ReadFileError::UnsupportedEncoding(request.path));
                }
                Err(error) => {
                    return Err(ReadFileError::Io {
                        path: request.path,
                        message: error.to_string(),
                    });
                }
            }
        };
        let content_bytes = content.len();
        Ok(ReadFileOutput {
            content,
            metadata: ReadFileMetadata {
                path: request.path,
                bytes: content_bytes,
                truncated: false,
                range: request.range,
            },
        })
    }

    fn resolve_inside_workspace(&self, input: &str) -> Result<PathBuf, ReadFileError> {
        let path = Path::new(input);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        {
            return Err(ReadFileError::InvalidPath(input.to_owned()));
        }
        let mut lexical = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir if !lexical.pop() => {
                    return Err(ReadFileError::OutsideWorkspace(input.to_owned()));
                }
                Component::ParentDir => {}
                Component::Normal(value) => lexical.push(value),
                Component::Prefix(_) | Component::RootDir => unreachable!(),
            }
        }
        let candidate = self.workspace_root.join(lexical);
        let resolved = fs::canonicalize(&candidate).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ReadFileError::NotFound(input.to_owned())
            } else {
                ReadFileError::Io {
                    path: input.to_owned(),
                    message: error.to_string(),
                }
            }
        })?;
        if !resolved.starts_with(&self.workspace_root) {
            return Err(ReadFileError::OutsideWorkspace(input.to_owned()));
        }
        Ok(resolved)
    }
}

fn read_bounded_range(
    path: &Path,
    range: &ReadFileRange,
    file_bytes: u64,
    display_path: &str,
) -> Result<String, ReadFileError> {
    if range.start_line == 0 || range.end_line < range.start_line {
        return Err(ReadFileError::InvalidRange {
            start_line: range.start_line,
            end_line: range.end_line,
        });
    }
    let mut file = fs::File::open(path).map_err(|error| ReadFileError::Io {
        path: display_path.to_owned(),
        message: error.to_string(),
    })?;
    let mut buffer = [0_u8; 8192];
    let mut line = Vec::new();
    let mut output = Vec::new();
    let mut current_line = 1_usize;
    let mut scanned = 0_u64;
    let scan_limit = MAX_READ_FILE_BYTES;

    loop {
        let read = file.read(&mut buffer).map_err(|error| ReadFileError::Io {
            path: display_path.to_owned(),
            message: error.to_string(),
        })?;
        if read == 0 {
            if !line.is_empty() || current_line == 1 {
                append_range_line(&line, current_line, range, &mut output, display_path)?;
            }
            let available_lines = if current_line == 1 {
                1
            } else if line.is_empty() {
                current_line - 1
            } else {
                current_line
            };
            if range.end_line > available_lines {
                return Err(ReadFileError::InvalidRange {
                    start_line: range.start_line,
                    end_line: range.end_line,
                });
            }
            return String::from_utf8(output)
                .map_err(|_| ReadFileError::UnsupportedEncoding(display_path.to_owned()));
        }

        scanned = scanned.saturating_add(read as u64);
        if scanned > scan_limit {
            return Err(ReadFileError::TooLarge {
                path: display_path.to_owned(),
                bytes: file_bytes,
                limit: scan_limit,
            });
        }
        for byte in &buffer[..read] {
            line.push(*byte);
            if *byte == b'\n' {
                append_range_line(&line, current_line, range, &mut output, display_path)?;
                if current_line == range.end_line {
                    return String::from_utf8(output)
                        .map_err(|_| ReadFileError::UnsupportedEncoding(display_path.to_owned()));
                }
                current_line += 1;
                line.clear();
            }
        }
    }
}

fn append_range_line(
    line: &[u8],
    line_number: usize,
    range: &ReadFileRange,
    output: &mut Vec<u8>,
    display_path: &str,
) -> Result<(), ReadFileError> {
    if line_number >= range.start_line && line_number <= range.end_line {
        if std::str::from_utf8(line).is_err() {
            return Err(ReadFileError::UnsupportedEncoding(display_path.to_owned()));
        }
        output.extend_from_slice(line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn capability() -> (tempfile::TempDir, ProjectReadCapability) {
        let dir = tempdir().unwrap();
        let capability = ProjectReadCapability::new(dir.path()).unwrap();
        (dir, capability)
    }

    fn request(path: &str) -> ReadFileRequest {
        ReadFileRequest {
            path: path.into(),
            range: None,
        }
    }

    #[test]
    fn reads_utf8_relative_file_and_preserves_content() {
        let (dir, reader) = capability();
        fs::write(dir.path().join("README.md"), "Olá\nAxiom\n").unwrap();
        let output = reader.read_file(request("README.md")).unwrap();
        assert_eq!(output.content, "Olá\nAxiom\n");
        assert_eq!(output.metadata.path, "README.md");
    }

    #[test]
    fn supports_normalized_inside_path_and_line_range() {
        let (dir, reader) = capability();
        fs::write(dir.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
        let output = reader
            .read_file(ReadFileRequest {
                path: "docs/../README.md".into(),
                range: Some(ReadFileRange {
                    start_line: 2,
                    end_line: 3,
                }),
            })
            .unwrap();
        assert_eq!(output.content, "two\nthree\n");
    }

    #[test]
    fn rejects_directory_with_explicit_typed_error() {
        let (dir, reader) = capability();
        fs::create_dir(dir.path().join("App")).unwrap();
        assert!(matches!(
            reader.read_file(request("App")),
            Err(ReadFileError::Directory(path)) if path == "App"
        ));
    }

    #[test]
    fn rejects_escape_absolute_missing_and_invalid_range() {
        let (dir, reader) = capability();
        assert!(matches!(
            reader.read_file(request("../outside")),
            Err(ReadFileError::OutsideWorkspace(_))
        ));
        assert!(matches!(
            reader.read_file(request(&dir.path().join("outside").display().to_string())),
            Err(ReadFileError::InvalidPath(_))
        ));
        let outside = tempdir().unwrap();
        fs::create_dir(outside.path().join("App")).unwrap();
        assert!(matches!(
            reader.read_file(request(&outside.path().join("App").display().to_string())),
            Err(ReadFileError::InvalidPath(_))
        ));
        assert!(matches!(
            reader.read_file(request("missing.txt")),
            Err(ReadFileError::NotFound(_))
        ));
        fs::write(dir.path().join("one.txt"), "one\n").unwrap();
        let result = reader.read_file(ReadFileRequest {
            path: "one.txt".into(),
            range: Some(ReadFileRange {
                start_line: 0,
                end_line: 1,
            }),
        });
        assert!(matches!(result, Err(ReadFileError::InvalidRange { .. })));
    }

    #[test]
    fn rejects_binary_and_too_large_files_without_silent_truncation() {
        let (dir, reader) = capability();
        fs::write(dir.path().join("binary"), [0, 1, 2]).unwrap();
        assert!(matches!(
            reader.read_file(request("binary")),
            Err(ReadFileError::UnsupportedEncoding(_))
        ));
        fs::write(
            dir.path().join("large"),
            vec![b'x'; MAX_READ_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(matches!(
            reader.read_file(request("large")),
            Err(ReadFileError::TooLarge { .. })
        ));
    }

    #[test]
    fn bounded_range_reads_prefix_of_large_file() {
        let (dir, reader) = capability();
        let mut content = String::from("first\nsecond\nthird\n");
        content.push_str(&"x".repeat(MAX_READ_FILE_BYTES as usize));
        fs::write(dir.path().join("large.log"), content).unwrap();

        let output = reader
            .read_file(ReadFileRequest {
                path: "large.log".into(),
                range: Some(ReadFileRange {
                    start_line: 1,
                    end_line: 2,
                }),
            })
            .unwrap();
        assert_eq!(output.content, "first\nsecond\n");
        assert_eq!(output.metadata.bytes, "first\nsecond\n".len());
        assert_eq!(
            output.metadata.range,
            Some(ReadFileRange {
                start_line: 1,
                end_line: 2,
            })
        );
    }

    #[test]
    fn bounded_range_keeps_scan_limit_for_deep_requests() {
        let (dir, reader) = capability();
        fs::write(
            dir.path().join("large.log"),
            format!("{}\nlast\n", "x".repeat(MAX_READ_FILE_BYTES as usize)),
        )
        .unwrap();

        assert!(matches!(
            reader.read_file(ReadFileRequest {
                path: "large.log".into(),
                range: Some(ReadFileRange {
                    start_line: 2,
                    end_line: 2,
                }),
            }),
            Err(ReadFileError::TooLarge { .. })
        ));
    }
}
