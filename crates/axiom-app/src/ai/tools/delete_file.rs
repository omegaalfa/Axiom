use std::str::FromStr;

use axiom_project::project_delete::{ProjectDeleteCapability, ProjectDeleteError};
use axiom_project::project_update::TextFileFingerprint;

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct DeleteFileTool {
    capability: ProjectDeleteCapability,
}

impl DeleteFileTool {
    pub(crate) fn new(capability: ProjectDeleteCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute(&self, path: String, expected_fingerprint: String) -> ToolResult {
        let expected = match TextFileFingerprint::from_str(&expected_fingerprint) {
            Ok(value) => value,
            Err(_) => {
                return ToolResult {
                    tool: ToolName::DeleteFile,
                    result: Err(ToolError::InvalidFingerprint),
                };
            }
        };
        let result = self
            .capability
            .delete_file(&path, expected)
            .map(|_| ToolOutput {
                content: "deleted successfully".into(),
                metadata: ToolMetadata {
                    path,
                    bytes: 0,
                    range: None,
                    source_bytes: None,
                    truncated: false,
                    fingerprint: None,
                },
            })
            .map_err(map_error);
        ToolResult {
            tool: ToolName::DeleteFile,
            result,
        }
    }
}

fn map_error(error: ProjectDeleteError) -> ToolError {
    match error {
        ProjectDeleteError::InvalidPath(path) => ToolError::InvalidPath(path),
        ProjectDeleteError::OutsideWorkspace(path) => ToolError::OutsideWorkspace(path),
        ProjectDeleteError::NotFound(path) => ToolError::NotFound(path),
        ProjectDeleteError::NotRegularFile(path) => ToolError::NotRegularFile(path),
        ProjectDeleteError::SymlinkNotAllowed(path) => ToolError::SymlinkNotAllowed(path),
        ProjectDeleteError::FingerprintMismatch { .. } => ToolError::FingerprintMismatch,
        ProjectDeleteError::Io { path, message, .. } => ToolError::Io { path, message },
    }
}
