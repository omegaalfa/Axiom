//! Production adapters and the small UI-facing bridge for one Agent run.

use super::tool_orchestration::{
    fetch_url_definition, list_directory_definition, read_file_definition,
};
use super::tools::{ToolArguments, ToolError, ToolName, ToolRegistry, ToolRequest};
use axiom_agent::{
    AgentEvent, AgentExecutor, AgentRun, AgentRunId, Cancellation, ProviderExecutor, ToolExecutor,
    ToolInfrastructureError, ToolOutcome,
};
use axiom_ai_provider::{
    OllamaProvider, ProviderChatRequest, ProviderChatStreamEvent, ProviderError, ProviderToolCall,
};
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static NEXT_AGENT_RUN_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) type AgentEventQueue = Arc<Mutex<Vec<AgentEvent>>>;

pub(crate) fn next_agent_run_id() -> AgentRunId {
    AgentRunId::new(NEXT_AGENT_RUN_ID.fetch_add(1, Ordering::Relaxed))
}

pub(crate) fn read_only_tool_definitions() -> Vec<axiom_ai_provider::ProviderToolDefinition> {
    vec![
        read_file_definition(),
        list_directory_definition(),
        fetch_url_definition(),
    ]
}

pub(crate) struct AgentProviderAdapter {
    base_url: String,
    cancellation: Cancellation,
}

impl AgentProviderAdapter {
    pub(crate) fn new(base_url: String, cancellation: Cancellation) -> Self {
        Self {
            base_url,
            cancellation,
        }
    }
}

impl ProviderExecutor for AgentProviderAdapter {
    fn execute(
        &mut self,
        request: &ProviderChatRequest,
        emit: &mut dyn FnMut(ProviderChatStreamEvent),
    ) -> Result<(), ProviderError> {
        OllamaProvider::default().chat_stream_with_cancel(
            &self.base_url,
            request,
            || self.cancellation.is_cancelled(),
            |event| {
                emit(event);
                Ok(())
            },
        )
    }
}

pub(crate) struct AgentToolAdapter {
    registry: Option<ToolRegistry>,
    cancellation: Cancellation,
}

impl AgentToolAdapter {
    pub(crate) fn new(registry: ToolRegistry, cancellation: Cancellation) -> Self {
        Self {
            registry: Some(registry),
            cancellation,
        }
    }

    pub(crate) fn without_tools(cancellation: Cancellation) -> Self {
        Self {
            registry: None,
            cancellation,
        }
    }
}

impl ToolExecutor for AgentToolAdapter {
    fn execute(&mut self, call: &ProviderToolCall) -> Result<ToolOutcome, ToolInfrastructureError> {
        let request = match provider_call_to_request(call) {
            Ok(request) => request,
            Err(error) => return Ok(ToolOutcome::ControlledError(error)),
        };
        let Some(registry) = self.registry.as_ref() else {
            return Err(ToolInfrastructureError {
                message: "read-only tools unavailable".into(),
            });
        };
        let result = registry.execute_with_cancel(request, || self.cancellation.is_cancelled());
        Ok(match result.result {
            Ok(output) => ToolOutcome::Success(output.content),
            Err(error) => ToolOutcome::ControlledError(tool_error_message(&error)),
        })
    }
}

fn provider_call_to_request(call: &ProviderToolCall) -> Result<ToolRequest, String> {
    let object = call
        .arguments
        .as_object()
        .ok_or_else(|| "arguments must be an object".to_owned())?;
    match call.name.as_str() {
        "read_file" => {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "path must be a string".to_owned())?;
            let range = match (object.get("start_line"), object.get("end_line")) {
                (None, None) => None,
                (Some(start), Some(end)) => Some(axiom_project::project_read::ReadFileRange {
                    start_line: start
                        .as_u64()
                        .ok_or_else(|| "start_line must be an integer".to_owned())?
                        as usize,
                    end_line: end
                        .as_u64()
                        .ok_or_else(|| "end_line must be an integer".to_owned())?
                        as usize,
                }),
                _ => return Err("start_line and end_line must be provided together".into()),
            };
            Ok(ToolRequest {
                name: ToolName::ReadFile,
                arguments: ToolArguments::ReadFile {
                    path: path.into(),
                    range,
                },
            })
        }
        "list_directory" => {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "path must be a string".to_owned())?;
            Ok(ToolRequest {
                name: ToolName::ListDirectory,
                arguments: ToolArguments::ListDirectory { path: path.into() },
            })
        }
        "fetch_url" => {
            let url = object
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| "url must be a string".to_owned())?;
            Ok(ToolRequest {
                name: ToolName::FetchUrl,
                arguments: ToolArguments::FetchUrl { url: url.into() },
            })
        }
        other => Ok(ToolRequest {
            name: ToolName::Unknown(other.into()),
            arguments: ToolArguments::ListDirectory {
                path: String::new(),
            },
        }),
    }
}

fn tool_error_message(error: &ToolError) -> String {
    match error {
        ToolError::UnknownTool(name) => format!("unknown tool: {name}"),
        ToolError::InvalidArguments(message) => message.clone(),
        ToolError::NotFound(path) => format!("file not found: {path}"),
        ToolError::Directory(path) => format!("path is a directory: {path}"),
        ToolError::NotDirectory(path) => format!("path is not a directory: {path}"),
        ToolError::OutsideWorkspace(path) => format!("path is outside the workspace: {path}"),
        ToolError::InvalidPath(path) => format!("invalid path: {path}"),
        ToolError::TooLarge { path, .. } => format!("file is too large: {path}"),
        ToolError::TooManyEntries { path, .. } => format!("too many directory entries: {path}"),
        ToolError::UnsupportedEncoding(path) => format!("unsupported encoding: {path}"),
        ToolError::InvalidRange { .. } => "invalid file range".into(),
        ToolError::Io { path, message } => format!("I/O error for {path}: {message}"),
        ToolError::InvalidUrl => "invalid URL".into(),
        ToolError::UnsupportedScheme(scheme) => format!("unsupported URL scheme: {scheme}"),
        ToolError::BlockedAddress(address) => format!("blocked address: {address}"),
        ToolError::Timeout => "request timed out".into(),
        ToolError::UnsupportedContentType(content_type) => {
            format!("unsupported content type: {content_type}")
        }
        ToolError::HttpStatus(status) => format!("HTTP status: {status}"),
        ToolError::Network(message) => format!("network error: {message}"),
        ToolError::Cancelled => "tool cancelled".into(),
        ToolError::RedirectLimit => "redirect limit reached".into(),
    }
}

pub(crate) fn execute_agent_run(
    run: &mut AgentRun,
    request: ProviderChatRequest,
    base_url: String,
    registry: Option<ToolRegistry>,
    queue: &AgentEventQueue,
) -> Result<axiom_agent::AgentExecutionResult, axiom_agent::AgentExecutionError> {
    let cancellation = run.cancellation();
    let mut provider = AgentProviderAdapter::new(base_url, cancellation.clone());
    let mut tools = match registry {
        Some(registry) => AgentToolAdapter::new(registry, cancellation),
        None => AgentToolAdapter::without_tools(cancellation),
    };
    let mut executor = AgentExecutor::new(&mut provider, &mut tools);
    let result = executor.execute(run, request, &mut |event| {
        if let Ok(mut events) = queue.lock() {
            events.push(event);
        }
    });
    if let Err(error) = &result {
        let diagnostic = error.diagnostic();
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "agent_terminal_failure",
            run_id = run.id().value(),
            category = ?diagnostic.kind,
            diagnostic = %diagnostic.message,
            "[AI-DIAG]"
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_project::{
        project_directory::ProjectDirectoryCapability, project_read::ProjectReadCapability,
    };
    use std::fs;

    fn adapter() -> (tempfile::TempDir, AgentToolAdapter) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        let registry =
            ToolRegistry::new_with_fetch_url(read, directory, axiom_web::FetchUrlCapability::new());
        (
            dir,
            AgentToolAdapter::new(registry, Cancellation::default()),
        )
    }

    #[test]
    fn read_file_and_list_directory_use_the_existing_registry() {
        let (_dir, mut adapter) = adapter();
        let read = ProviderToolCall {
            id: Some("read-1".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "Cargo.toml"}),
        };
        assert!(
            matches!(adapter.execute(&read), Ok(ToolOutcome::Success(content)) if content.contains("[package]"))
        );
        let list = ProviderToolCall {
            id: Some("list-1".into()),
            name: "list_directory".into(),
            arguments: serde_json::json!({"path": "."}),
        };
        assert!(
            matches!(adapter.execute(&list), Ok(ToolOutcome::Success(content)) if content.contains("Cargo.toml"))
        );
    }

    #[test]
    fn malformed_call_is_a_controlled_tool_error() {
        let (_dir, mut adapter) = adapter();
        let call = ProviderToolCall {
            id: None,
            name: "read_file".into(),
            arguments: serde_json::json!({}),
        };
        assert!(matches!(
            adapter.execute(&call),
            Ok(ToolOutcome::ControlledError(_))
        ));
    }
}
