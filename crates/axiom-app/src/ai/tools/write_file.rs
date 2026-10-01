use axiom_project::project_write::{ProjectWriteCapability, ProjectWriteError};

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct WriteFileTool {
    capability: ProjectWriteCapability,
}

impl WriteFileTool {
    pub(crate) fn new(capability: ProjectWriteCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute(&self, path: String, content: String) -> ToolResult {
        let tool = ToolName::WriteFile;
        let bytes = content.len();
        let requested_path = path.clone();
        let result = self
            .capability
            .create_text_file(&path, content)
            .map(|_| ToolOutput {
                content: "created successfully".into(),
                metadata: ToolMetadata {
                    path,
                    bytes,
                    range: None,
                    source_bytes: None,
                    truncated: false,
                    fingerprint: None,
                },
            })
            .map_err(|error| map_error(error, requested_path));
        ToolResult { tool, result }
    }
}

fn map_error(error: ProjectWriteError, requested_path: String) -> ToolError {
    match error {
        ProjectWriteError::InvalidPath(path) => ToolError::InvalidPath(path),
        ProjectWriteError::OutsideWorkspace(path) => ToolError::OutsideWorkspace(path),
        ProjectWriteError::ParentNotFound(path) => ToolError::NotFound(path),
        ProjectWriteError::ParentNotDirectory(path) => ToolError::NotDirectory(path),
        ProjectWriteError::SymlinkNotAllowed(path) => ToolError::SymlinkNotAllowed(path),
        ProjectWriteError::AlreadyExists(path) => ToolError::AlreadyExists(path),
        ProjectWriteError::ContentTooLarge { limit, actual } => ToolError::TooLarge {
            path: requested_path,
            bytes: actual as u64,
            limit: limit as u64,
        },
        ProjectWriteError::Io { path, .. } => ToolError::Io {
            path,
            message: "write failed".into(),
        },
    }
}
