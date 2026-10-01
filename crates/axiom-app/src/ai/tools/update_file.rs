use std::str::FromStr;

use axiom_project::project_update::{
    ProjectUpdateCapability, ProjectUpdateError, TextFileFingerprint,
};

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct UpdateFileTool {
    capability: ProjectUpdateCapability,
}

impl UpdateFileTool {
    pub(crate) fn new(capability: ProjectUpdateCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute(
        &self,
        path: String,
        expected_fingerprint: String,
        content: String,
    ) -> ToolResult {
        let expected = match TextFileFingerprint::from_str(&expected_fingerprint) {
            Ok(expected) => expected,
            Err(_) => {
                return ToolResult {
                    tool: ToolName::UpdateFile,
                    result: Err(ToolError::InvalidFingerprint),
                };
            }
        };
        let bytes = content.len();
        let requested_path = path.clone();
        let result = self
            .capability
            .update_text_file(&path, expected, content)
            .map(|_| ToolOutput {
                content: "updated successfully".into(),
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
        ToolResult {
            tool: ToolName::UpdateFile,
            result,
        }
    }
}

fn map_error(error: ProjectUpdateError, requested_path: String) -> ToolError {
    match error {
        ProjectUpdateError::InvalidPath(path) => ToolError::InvalidPath(path),
        ProjectUpdateError::OutsideWorkspace(path) => ToolError::OutsideWorkspace(path),
        ProjectUpdateError::ParentNotFound(path) => ToolError::NotFound(path),
        ProjectUpdateError::ParentNotDirectory(path) => ToolError::NotDirectory(path),
        ProjectUpdateError::SymlinkNotAllowed(path) => ToolError::SymlinkNotAllowed(path),
        ProjectUpdateError::AlreadyExists(path) => ToolError::AlreadyExists(path),
        ProjectUpdateError::NotFound(path) => ToolError::NotFound(path),
        ProjectUpdateError::NotRegularFile(path) => ToolError::NotRegularFile(path),
        ProjectUpdateError::UnsupportedEncoding(path) => ToolError::UnsupportedEncoding(path),
        ProjectUpdateError::CurrentFileTooLarge { limit, actual } => {
            ToolError::CurrentFileTooLarge {
                path: requested_path,
                limit,
                actual,
            }
        }
        ProjectUpdateError::ContentTooLarge { limit, actual } => ToolError::TooLarge {
            path: requested_path,
            bytes: actual as u64,
            limit: limit as u64,
        },
        ProjectUpdateError::FingerprintMismatch { .. } => ToolError::FingerprintMismatch,
        ProjectUpdateError::Io { path, .. } => ToolError::Io {
            path,
            message: "update failed".into(),
        },
    }
}
