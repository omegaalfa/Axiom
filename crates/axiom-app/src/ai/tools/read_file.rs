use axiom_project::project_read::{
    ProjectReadCapability, ReadFileError, ReadFileMetadata, ReadFileRange, ReadFileRequest,
};

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct ReadFileTool {
    capability: ProjectReadCapability,
}

impl ReadFileTool {
    pub(crate) fn new(capability: ProjectReadCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute(&self, path: String, range: Option<ReadFileRange>) -> ToolResult {
        let request = ReadFileRequest { path, range };
        let tool = ToolName::ReadFile;
        let result = self
            .capability
            .read_file(request)
            .map(|output| ToolOutput {
                content: output.content,
                metadata: metadata(output.metadata),
            })
            .map_err(map_error);
        ToolResult { tool, result }
    }
}

fn metadata(value: ReadFileMetadata) -> ToolMetadata {
    ToolMetadata {
        path: value.path,
        bytes: value.bytes,
        range: value.range,
    }
}

fn map_error(error: ReadFileError) -> ToolError {
    match error {
        ReadFileError::InvalidPath(path) => ToolError::InvalidPath(path),
        ReadFileError::OutsideWorkspace(path) => ToolError::OutsideWorkspace(path),
        ReadFileError::NotFound(path) => ToolError::NotFound(path),
        ReadFileError::TooLarge { path, bytes, limit } => {
            ToolError::TooLarge { path, bytes, limit }
        }
        ReadFileError::InvalidRange {
            start_line,
            end_line,
        } => ToolError::InvalidRange {
            start_line,
            end_line,
        },
        ReadFileError::Io { path, message } => ToolError::Io { path, message },
        ReadFileError::UnsupportedEncoding(path) => ToolError::UnsupportedEncoding(path),
    }
}
