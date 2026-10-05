use axiom_project::project_find_files::{FindFilesError, FindFilesRequest, ProjectFindFilesCapability};
use serde_json::json;

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct FindFilesTool {
    capability: ProjectFindFilesCapability,
}

impl FindFilesTool {
    pub(crate) fn new(capability: ProjectFindFilesCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute<F>(&self, pattern: String, cancelled: F) -> ToolResult
    where
        F: Fn() -> bool,
    {
        let result = self
            .capability
            .find_files_with_cancel(FindFilesRequest { pattern }, cancelled)
            .map(|output| ToolOutput {
                content: json!({
                    "matches": output.matches,
                    "truncated": output.truncated,
                })
                .to_string(),
                metadata: ToolMetadata {
                    path: ".".into(),
                    bytes: 0,
                    range: None,
                    source_bytes: None,
                    truncated: output.truncated,
                    fingerprint: None,
                },
            })
            .map_err(map_error);
        ToolResult { tool: ToolName::FindFiles, result }
    }
}

fn map_error(error: FindFilesError) -> ToolError {
    match error {
        FindFilesError::EmptyPattern | FindFilesError::InvalidPattern(_) => {
            ToolError::InvalidArguments(error.to_string())
        }
        FindFilesError::OutsideWorkspace(pattern) => ToolError::OutsideWorkspace(pattern),
        FindFilesError::Io { path, message } => ToolError::Io { path, message },
    }
}
