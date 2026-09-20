//! Minimal, provider-independent read-only tool layer.

mod read_file;

use axiom_project::project_read::{ProjectReadCapability, ReadFileRange};

pub(crate) use read_file::ReadFileTool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolName {
    ReadFile,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolResult {
    pub(crate) tool: ToolName,
    pub(crate) result: Result<ToolOutput, ToolError>,
}

#[derive(Clone, Debug)]
pub(crate) struct ToolRegistry {
    read_file: ReadFileTool,
}

impl ToolRegistry {
    pub(crate) fn new(capability: ProjectReadCapability) -> Self {
        Self {
            read_file: ReadFileTool::new(capability),
        }
    }

    pub(crate) fn kind(&self, name: &ToolName) -> Option<ToolKind> {
        matches!(name, ToolName::ReadFile).then_some(ToolKind::ReadOnly)
    }

    pub(crate) fn execute(&self, request: ToolRequest) -> ToolResult {
        match request {
            ToolRequest {
                name: ToolName::ReadFile,
                arguments: ToolArguments::ReadFile { path, range },
            } => self.read_file.execute(path, range),
            ToolRequest {
                name: ToolName::Unknown(name),
                ..
            } => ToolResult {
                tool: ToolName::Unknown(name.clone()),
                result: Err(ToolError::UnknownTool(name)),
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
        let capability = ProjectReadCapability::new(dir.path()).unwrap();
        (dir, ToolRegistry::new(capability))
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
        let (_dir, registry) = registry();
        let result = registry.execute(request("missing.txt"));
        assert!(matches!(result.result, Err(ToolError::NotFound(_))));
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
}
