use axiom_project::project_directory::{
    DirectoryEntryKind, ListDirectoryError, ProjectDirectoryCapability,
};
use serde_json::json;

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct ListDirectoryTool {
    capability: ProjectDirectoryCapability,
}

impl ListDirectoryTool {
    pub(crate) fn new(capability: ProjectDirectoryCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute(&self, path: String) -> ToolResult {
        let tool = ToolName::ListDirectory;
        let result = self
            .capability
            .list_directory(&path)
            .map(|output| ToolOutput {
                content: json!({
                    "path": output.path,
                    "entries": output.entries.iter().map(|entry| json!({
                        "name": entry.name,
                        "kind": match entry.kind {
                            DirectoryEntryKind::File => "file",
                            DirectoryEntryKind::Directory => "directory",
                            DirectoryEntryKind::Symlink => "symlink",
                            DirectoryEntryKind::Other => "other",
                        }
                    })).collect::<Vec<_>>()
                })
                .to_string(),
                metadata: ToolMetadata {
                    path,
                    bytes: output.entries.len(),
                    range: None,
                    source_bytes: None,
                    truncated: false,
                },
            })
            .map_err(map_error);
        ToolResult { tool, result }
    }
}

fn map_error(error: ListDirectoryError) -> ToolError {
    match error {
        ListDirectoryError::InvalidPath(path) => ToolError::InvalidPath(path),
        ListDirectoryError::NotFound(path) => ToolError::NotFound(path),
        ListDirectoryError::NotDirectory(path) => ToolError::NotDirectory(path),
        ListDirectoryError::TooManyEntries { path, limit } => {
            ToolError::TooManyEntries { path, limit }
        }
        ListDirectoryError::Io { path, message } => ToolError::Io { path, message },
    }
}
