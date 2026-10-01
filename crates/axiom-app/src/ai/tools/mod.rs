//! Minimal, provider-independent Agent tool layer.

mod fetch_url;
mod list_directory;
mod read_file;
mod update_file;
mod write_file;

use axiom_project::project_directory::ProjectDirectoryCapability;
use axiom_project::project_read::{ProjectReadCapability, ReadFileRange};
use axiom_project::project_update::{ProjectUpdateCapability, TextFileFingerprint};
use axiom_project::project_write::ProjectWriteCapability;

pub(crate) use fetch_url::FetchUrlTool;

/// Provider-facing budget for fetched text. This is separate from axiom-web's
/// network and capability output limits because model context is smaller than
/// a safe HTTP response buffer.
pub(crate) const MAX_TOOL_CONTENT_BYTES: usize = 24 * 1024;
pub(crate) use list_directory::ListDirectoryTool;
pub(crate) use read_file::ReadFileTool;
pub(crate) use update_file::UpdateFileTool;
pub(crate) use write_file::WriteFileTool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolName {
    ReadFile,
    ListDirectory,
    FetchUrl,
    WriteFile,
    UpdateFile,
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    WriteFile {
        path: String,
        content: String,
    },
    UpdateFile {
        path: String,
        expected_fingerprint: String,
        content: String,
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
    pub(crate) fingerprint: Option<TextFileFingerprint>,
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
    AlreadyExists(String),
    SymlinkNotAllowed(String),
    InvalidUrl,
    UnsupportedScheme(String),
    BlockedAddress(String),
    Timeout,
    UnsupportedContentType(String),
    HttpStatus(u16),
    Network(String),
    Cancelled,
    RedirectLimit,
    InvalidFingerprint,
    FingerprintMismatch,
    NotRegularFile(String),
    CurrentFileTooLarge { path: String, limit: usize, actual: usize },
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
    write_file: Option<WriteFileTool>,
    update_file: Option<UpdateFileTool>,
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
            write_file: None,
            update_file: None,
        }
    }

    pub(crate) fn new_with_write_file(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
        fetch_url_capability: axiom_web::FetchUrlCapability,
        write_capability: ProjectWriteCapability,
    ) -> Self {
        Self {
            read_file: ReadFileTool::new(read_capability),
            list_directory: ListDirectoryTool::new(directory_capability),
            fetch_url: FetchUrlTool::new(fetch_url_capability),
            write_file: Some(WriteFileTool::new(write_capability)),
            update_file: None,
        }
    }

    pub(crate) fn new_with_mutations(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
        fetch_url_capability: axiom_web::FetchUrlCapability,
        write_capability: ProjectWriteCapability,
        update_capability: ProjectUpdateCapability,
    ) -> Self {
        Self {
            read_file: ReadFileTool::new(read_capability),
            list_directory: ListDirectoryTool::new(directory_capability),
            fetch_url: FetchUrlTool::new(fetch_url_capability),
            write_file: Some(WriteFileTool::new(write_capability)),
            update_file: Some(UpdateFileTool::new(update_capability)),
        }
    }

    pub(crate) fn kind(&self, name: &ToolName) -> Option<ToolKind> {
        match name {
            ToolName::ReadFile | ToolName::ListDirectory | ToolName::FetchUrl => {
                Some(ToolKind::ReadOnly)
            }
            ToolName::WriteFile => self.write_file.is_some().then_some(ToolKind::Mutating),
            ToolName::UpdateFile => self.update_file.is_some().then_some(ToolKind::Mutating),
            ToolName::Unknown(_) => None,
        }
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
                name: ToolName::WriteFile,
                arguments: ToolArguments::WriteFile { path, content },
            } => {
                if cancelled() {
                    return ToolResult {
                        tool: ToolName::WriteFile,
                        result: Err(ToolError::Cancelled),
                    };
                }
                match self.write_file.as_ref() {
                    Some(write_file) => write_file.execute(path, content),
                    None => ToolResult {
                        tool: ToolName::WriteFile,
                        result: Err(ToolError::UnknownTool("write_file".into())),
                    },
                }
            }
            ToolRequest {
                name: ToolName::UpdateFile,
                arguments:
                    ToolArguments::UpdateFile {
                        path,
                        expected_fingerprint,
                        content,
                    },
            } => {
                if cancelled() {
                    return ToolResult {
                        tool: ToolName::UpdateFile,
                        result: Err(ToolError::Cancelled),
                    };
                }
                match self.update_file.as_ref() {
                    Some(update_file) => {
                        update_file.execute(path, expected_fingerprint, content)
                    }
                    None => ToolResult {
                        tool: ToolName::UpdateFile,
                        result: Err(ToolError::UnknownTool("update_file".into())),
                    },
                }
            }
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
    use axiom_project::project_update::ProjectUpdateCapability;
    use axiom_project::project_write::ProjectWriteCapability;
    use std::fs;
    use tempfile::tempdir;

    fn registry() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempdir().unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        (dir, ToolRegistry::new(read, directory))
    }

    fn mutation_registry() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempdir().unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        let write = ProjectWriteCapability::new(dir.path()).unwrap();
        (
            dir,
            ToolRegistry::new_with_write_file(
                read,
                directory,
                axiom_web::FetchUrlCapability::new(),
                write,
            ),
        )
    }

    fn update_registry() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempdir().unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        let write = ProjectWriteCapability::new(dir.path()).unwrap();
        let update = ProjectUpdateCapability::new(dir.path()).unwrap();
        (
            dir,
            ToolRegistry::new_with_mutations(
                read,
                directory,
                axiom_web::FetchUrlCapability::new(),
                write,
                update,
            ),
        )
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
        assert_eq!(registry.kind(&ToolName::WriteFile), None);
    }

    #[test]
    fn write_file_is_mutating_and_creates_utf8_content() {
        let (dir, registry) = mutation_registry();
        fs::create_dir(dir.path().join("src")).unwrap();
        assert_eq!(
            registry.kind(&ToolName::WriteFile),
            Some(ToolKind::Mutating)
        );
        let result = registry.execute(ToolRequest {
            name: ToolName::WriteFile,
            arguments: ToolArguments::WriteFile {
                path: "src/new.txt".into(),
                content: "Olá, Axiom! 🚀".into(),
            },
        });
        assert!(matches!(result.result, Ok(ToolOutput { .. })));
        assert_eq!(
            fs::read_to_string(dir.path().join("src/new.txt")).unwrap(),
            "Olá, Axiom! 🚀"
        );
    }

    #[test]
    fn write_file_rejects_existing_destination_without_overwriting() {
        let (dir, registry) = mutation_registry();
        fs::write(dir.path().join("existing.txt"), "original").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::WriteFile,
            arguments: ToolArguments::WriteFile {
                path: "existing.txt".into(),
                content: "replacement".into(),
            },
        });
        assert!(matches!(result.result, Err(ToolError::AlreadyExists(_))));
        assert_eq!(
            fs::read_to_string(dir.path().join("existing.txt")).unwrap(),
            "original"
        );
    }

    #[test]
    fn update_file_requires_matching_fingerprint_and_preserves_utf8() {
        let (dir, registry) = update_registry();
        fs::write(dir.path().join("file.txt"), "old").unwrap();
        let expected = ProjectUpdateCapability::new(dir.path())
            .unwrap()
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        assert_eq!(
            registry.kind(&ToolName::UpdateFile),
            Some(ToolKind::Mutating)
        );
        let result = registry.execute(ToolRequest {
            name: ToolName::UpdateFile,
            arguments: ToolArguments::UpdateFile {
                path: "file.txt".into(),
                expected_fingerprint: expected,
                content: "Olá, Axiom! 🚀".into(),
            },
        });
        assert!(matches!(result.result, Ok(ToolOutput { .. })));
        assert_eq!(
            fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "Olá, Axiom! 🚀"
        );
    }

    #[test]
    fn update_file_rejects_stale_and_malformed_fingerprints_without_mutation() {
        let (dir, registry) = update_registry();
        fs::write(dir.path().join("file.txt"), "current").unwrap();
        for expected_fingerprint in [
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            "malformed".into(),
        ] {
            let result = registry.execute(ToolRequest {
                name: ToolName::UpdateFile,
                arguments: ToolArguments::UpdateFile {
                    path: "file.txt".into(),
                    expected_fingerprint,
                    content: "replacement".into(),
                },
            });
            assert!(matches!(
                result.result,
                Err(ToolError::FingerprintMismatch | ToolError::InvalidFingerprint)
            ));
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "current"
        );
    }

    #[test]
    fn update_file_conflict_preserves_newer_content_and_missing_file_is_not_created() {
        let (dir, registry) = update_registry();
        fs::write(dir.path().join("file.txt"), "old").unwrap();
        let expected = ProjectUpdateCapability::new(dir.path())
            .unwrap()
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        fs::write(dir.path().join("file.txt"), "newer").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::UpdateFile,
            arguments: ToolArguments::UpdateFile {
                path: "file.txt".into(),
                expected_fingerprint: expected,
                content: "stale replacement".into(),
            },
        });
        assert!(matches!(result.result, Err(ToolError::FingerprintMismatch)));
        assert_eq!(
            fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "newer"
        );

        let missing = registry.execute(ToolRequest {
            name: ToolName::UpdateFile,
            arguments: ToolArguments::UpdateFile {
                path: "missing.txt".into(),
                expected_fingerprint:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .into(),
                content: "must not be created".into(),
            },
        });
        assert!(matches!(missing.result, Err(ToolError::NotFound(_))));
        assert!(!dir.path().join("missing.txt").exists());
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
