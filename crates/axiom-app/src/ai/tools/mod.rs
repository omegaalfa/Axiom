//! Minimal, provider-independent read-only tool layer.

mod fetch_url;
mod list_directory;
mod read_file;

use axiom_project::project_directory::ProjectDirectoryCapability;
use axiom_project::project_read::{ProjectReadCapability, ReadFileRange};

pub(crate) use fetch_url::FetchUrlTool;

/// Provider-facing budget for fetched text. This is separate from axiom-web's
/// network and capability output limits because model context is smaller than
/// a safe HTTP response buffer.
pub(crate) const MAX_TOOL_CONTENT_BYTES: usize = 24 * 1024;
pub(crate) use list_directory::ListDirectoryTool;
pub(crate) use read_file::ReadFileTool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolName {
    ReadFile,
    ListDirectory,
    FetchUrl,
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Mutating tools are intentionally not implemented yet, but the category is
// part of the registry contract for the future agent runtime.
#[allow(dead_code)]
pub(crate) enum ToolKind {
    ReadOnly,
    Mutating,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolArguments {
    ReadFile {
        path: String,
        range: Option<ReadFileRange>,
    },
    ListDirectory {
        path: String,
    },
    FetchUrl {
        url: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolRequest {
    pub(crate) name: ToolName,
    pub(crate) arguments: ToolArguments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolMetadata {
    pub(crate) path: String,
    pub(crate) bytes: usize,
    pub(crate) range: Option<ReadFileRange>,
    pub(crate) source_bytes: Option<usize>,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolOutput {
    pub(crate) content: String,
    pub(crate) metadata: ToolMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolError {
    UnknownTool(String),
    InvalidArguments(String),
    InvalidPath(String),
    OutsideWorkspace(String),
    NotFound(String),
    Directory(String),
    NotDirectory(String),
    TooManyEntries {
        path: String,
        limit: usize,
    },
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
    InvalidUrl,
    UnsupportedScheme(String),
    BlockedAddress(String),
    Timeout,
    UnsupportedContentType(String),
    HttpStatus(u16),
    Network(String),
    Cancelled,
    RedirectLimit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolResult {
    pub(crate) tool: ToolName,
    pub(crate) result: Result<ToolOutput, ToolError>,
}

#[derive(Clone, Debug)]
pub(crate) struct ToolRegistry {
    read_file: ReadFileTool,
    list_directory: ListDirectoryTool,
    fetch_url: FetchUrlTool,
}

impl ToolRegistry {
    #[cfg(test)]
    pub(crate) fn new(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
    ) -> Self {
        Self::new_with_fetch_url(
            read_capability,
            directory_capability,
            axiom_web::FetchUrlCapability::new(),
        )
    }

    pub(crate) fn new_with_fetch_url(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
        fetch_url_capability: axiom_web::FetchUrlCapability,
    ) -> Self {
        Self {
            read_file: ReadFileTool::new(read_capability),
            list_directory: ListDirectoryTool::new(directory_capability),
            fetch_url: FetchUrlTool::new(fetch_url_capability),
        }
    }

    pub(crate) fn kind(&self, name: &ToolName) -> Option<ToolKind> {
        matches!(
            name,
            ToolName::ReadFile | ToolName::ListDirectory | ToolName::FetchUrl
        )
        .then_some(ToolKind::ReadOnly)
    }

    pub(crate) fn execute(&self, request: ToolRequest) -> ToolResult {
        self.execute_with_cancel(request, || false)
    }

    pub(crate) fn execute_with_cancel<F>(&self, request: ToolRequest, cancelled: F) -> ToolResult
    where
        F: Fn() -> bool,
    {
        match request {
            ToolRequest {
                name: ToolName::ReadFile,
                arguments: ToolArguments::ReadFile { path, range },
            } => self.read_file.execute(path, range),
            ToolRequest {
                name: ToolName::ListDirectory,
                arguments: ToolArguments::ListDirectory { path },
            } => self.list_directory.execute(path),
            ToolRequest {
                name: ToolName::FetchUrl,
                arguments: ToolArguments::FetchUrl { url },
            } => self.fetch_url.execute(url, cancelled),
            ToolRequest {
                name: ToolName::Unknown(name),
                ..
            } => ToolResult {
                tool: ToolName::Unknown(name.clone()),
                result: Err(ToolError::UnknownTool(name)),
            },
            ToolRequest { name, .. } => ToolResult {
                tool: name,
                result: Err(ToolError::InvalidArguments(
                    "tool arguments do not match the tool name".into(),
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_project::project_read::ProjectReadCapability;
    use std::fs;
    use tempfile::tempdir;

    fn registry() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempdir().unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        (dir, ToolRegistry::new(read, directory))
    }

    fn request(path: &str) -> ToolRequest {
        ToolRequest {
            name: ToolName::ReadFile,
            arguments: ToolArguments::ReadFile {
                path: path.into(),
                range: None,
            },
        }
    }

    #[test]
    fn registry_knows_read_file_as_read_only() {
        let (_dir, registry) = registry();
        assert_eq!(registry.kind(&ToolName::ReadFile), Some(ToolKind::ReadOnly));
        assert_eq!(
            registry.kind(&ToolName::ListDirectory),
            Some(ToolKind::ReadOnly)
        );
        assert_eq!(registry.kind(&ToolName::FetchUrl), Some(ToolKind::ReadOnly));
    }

    #[test]
    fn read_file_adapter_delegates_content_range_and_metadata() {
        let (dir, registry) = registry();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/Test.php"), "one\ntwo\nthree\n").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::ReadFile,
            arguments: ToolArguments::ReadFile {
                path: "src/Test.php".into(),
                range: Some(ReadFileRange {
                    start_line: 2,
                    end_line: 3,
                }),
            },
        });
        let output = result.result.unwrap();
        assert_eq!(output.content, "two\nthree\n");
        assert_eq!(output.metadata.path, "src/Test.php");
        assert_eq!(
            output.metadata.range,
            Some(ReadFileRange {
                start_line: 2,
                end_line: 3
            })
        );
    }

    #[test]
    fn capability_errors_are_mapped_without_collapsing_categories() {
        let (dir, registry) = registry();
        let result = registry.execute(request("missing.txt"));
        assert!(matches!(result.result, Err(ToolError::NotFound(_))));
        fs::create_dir(dir.path().join("App")).unwrap();
        let result = registry.execute(request("App"));
        assert!(matches!(result.result, Err(ToolError::Directory(path)) if path == "App"));
    }

    #[test]
    fn list_directory_adapter_returns_deterministic_structured_content() {
        let (dir, registry) = registry();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::write(dir.path().join("App/Z.php"), "z").unwrap();
        fs::write(dir.path().join("App/A.php"), "a").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::ListDirectory,
            arguments: ToolArguments::ListDirectory { path: "App".into() },
        });
        let output = result.result.unwrap();
        assert!(output.content.contains("A.php"));
        assert!(output.content.contains("Z.php"));
        assert_eq!(output.metadata.path, "App");
    }

    #[test]
    fn list_directory_adapter_preserves_nested_path_and_root_distinction() {
        let (dir, registry) = registry();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::create_dir_all(dir.path().join("src/App")).unwrap();
        fs::write(dir.path().join("App/Root.php"), "root").unwrap();
        fs::write(dir.path().join("src/App/FileStone.php"), "nested").unwrap();

        let src = registry.execute(ToolRequest {
            name: ToolName::ListDirectory,
            arguments: ToolArguments::ListDirectory { path: "src".into() },
        });
        let src_output = src.result.unwrap();
        assert_eq!(src_output.metadata.path, "src");
        assert!(src_output.content.contains("\"name\":\"App\""));
        assert!(!src_output.content.contains("FileStone.php"));

        let nested = registry.execute(ToolRequest {
            name: ToolName::ListDirectory,
            arguments: ToolArguments::ListDirectory {
                path: "src/App".into(),
            },
        });
        let nested_output = nested.result.unwrap();
        assert_eq!(nested_output.metadata.path, "src/App");
        assert!(nested_output.content.contains("FileStone.php"));
        assert!(!nested_output.content.contains("Root.php"));
    }

    #[test]
    fn unknown_tool_is_a_typed_error() {
        let (_dir, registry) = registry();
        let result = registry.execute(ToolRequest {
            name: ToolName::Unknown("list_files".into()),
            arguments: ToolArguments::ReadFile {
                path: "x".into(),
                range: None,
            },
        });
        assert_eq!(result.tool, ToolName::Unknown("list_files".into()));
        assert!(matches!(result.result, Err(ToolError::UnknownTool(name)) if name == "list_files"));
    }

    #[test]
    fn fetch_url_blocked_address_is_a_controlled_tool_error() {
        let (_dir, registry) = registry();
        let result = registry.execute(ToolRequest {
            name: ToolName::FetchUrl,
            arguments: ToolArguments::FetchUrl {
                url: "http://127.0.0.1/".into(),
            },
        });
        assert!(matches!(result.result, Err(ToolError::BlockedAddress(_))));
    }
}
