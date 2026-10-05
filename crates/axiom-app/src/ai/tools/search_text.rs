use axiom_project::project_search::{ProjectSearchCapability, SearchTextError, SearchTextRequest};
use serde_json::json;

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct SearchTextTool {
    capability: ProjectSearchCapability,
}

impl SearchTextTool {
    pub(crate) fn new(capability: ProjectSearchCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute<F>(
        &self,
        query: String,
        path: Option<String>,
        file_pattern: Option<String>,
        cancelled: F,
    ) -> ToolResult
    where
        F: Fn() -> bool,
    {
        let display_path = path.clone().unwrap_or_else(|| ".".into());
        let result = self
            .capability
            .search_text_with_cancel(SearchTextRequest { query, path, file_pattern }, cancelled)
            .map(|output| ToolOutput {
                content: json!({
                    "matches": output.matches.iter().map(|item| json!({
                        "path": item.path,
                        "line": item.line,
                        "column": item.column,
                        "preview": item.preview,
                    })).collect::<Vec<_>>(),
                    "truncated": output.truncated,
                }).to_string(),
                metadata: ToolMetadata {
                    path: display_path,
                    bytes: output.matches.len(),
                    range: None,
                    source_bytes: None,
                    truncated: output.truncated,
                    fingerprint: None,
                },
            })
            .map_err(map_error);
        ToolResult { tool: ToolName::SearchText, result }
    }
}

fn map_error(error: SearchTextError) -> ToolError {
    match error {
        SearchTextError::EmptyQuery => ToolError::InvalidArguments(error.to_string()),
        SearchTextError::InvalidPattern(pattern) => {
            ToolError::InvalidArguments(format!("invalid file pattern: {pattern}"))
        }
        SearchTextError::InvalidPath(path) => ToolError::InvalidPath(path),
        SearchTextError::OutsideWorkspace(path) => ToolError::OutsideWorkspace(path),
        SearchTextError::NotFound(path) => ToolError::NotFound(path),
        SearchTextError::Io { path, message } => ToolError::Io { path, message },
    }
}
