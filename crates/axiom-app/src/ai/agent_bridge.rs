//! Production adapters and the small UI-facing bridge for one Agent run.

use super::tool_orchestration::{
    delete_file_definition, fetch_url_definition, list_directory_definition, read_file_definition,
    update_file_definition, write_file_definition,
};
use super::tools::{ToolArguments, ToolError, ToolName, ToolRegistry, ToolRequest};
use axiom_agent::{
    AgentEvent, AgentExecutionError, AgentExecutor, AgentRun, AgentRunId, ApprovalId,
    ApprovalRequest, Cancellation, ProductionToolPolicy, ProviderExecutor, ToolExecutor,
    ToolInfrastructureError, ToolOutcome, ToolPolicy,
};
use axiom_ai_provider::{
    ProviderChatRequest, ProviderChatStreamEvent, ProviderError, ProviderKind, ProviderToolCall,
    provider_chat_stream_with_cancel,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
};

static NEXT_AGENT_RUN_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_APPROVAL_RUN_TOKEN: AtomicU64 = AtomicU64::new(1);

pub(crate) type AgentEventQueue = Arc<Mutex<Vec<AgentEvent>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalBridgeError {
    UnknownRun,
    NotPending,
    UnknownApproval,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApprovalCommand {
    Approve,
    AllowForRequest,
    Deny,
    Cancel,
}

struct ApprovalRunEntry {
    generation: u64,
    commands: Sender<ApprovalCommand>,
    cancellation: Cancellation,
    pending: Option<ApprovalRequest>,
}

#[derive(Default)]
struct ApprovalBridgeState {
    runs: HashMap<AgentRunId, ApprovalRunEntry>,
}

#[derive(Clone, Default)]
pub(crate) struct ApprovalBridge {
    state: Arc<Mutex<ApprovalBridgeState>>,
}

impl ApprovalBridge {
    fn register(&self, run_id: AgentRunId, cancellation: Cancellation) -> ApprovalRunHandle {
        let (commands, receiver) = mpsc::channel();
        let generation = NEXT_APPROVAL_RUN_TOKEN.fetch_add(1, Ordering::Relaxed);
        if let Some(previous) = self.state.lock().unwrap().runs.insert(
            run_id,
            ApprovalRunEntry {
                generation,
                commands,
                cancellation,
                pending: None,
            },
        ) {
            previous.cancellation.cancel();
            let _ = previous.commands.send(ApprovalCommand::Cancel);
        }
        ApprovalRunHandle {
            bridge: self.clone(),
            run_id,
            generation,
            receiver,
        }
    }

    pub(crate) fn pending(&self, run_id: AgentRunId) -> Option<ApprovalRequest> {
        self.state
            .lock()
            .unwrap()
            .runs
            .get(&run_id)
            .and_then(|entry| entry.pending.clone())
    }

    pub(crate) fn approve(
        &self,
        run_id: AgentRunId,
        approval_id: ApprovalId,
    ) -> Result<(), ApprovalBridgeError> {
        self.decide(run_id, approval_id, ApprovalCommand::Approve)
    }

    pub(crate) fn allow_for_request(
        &self,
        run_id: AgentRunId,
        approval_id: ApprovalId,
    ) -> Result<(), ApprovalBridgeError> {
        self.decide(run_id, approval_id, ApprovalCommand::AllowForRequest)
    }

    pub(crate) fn deny(
        &self,
        run_id: AgentRunId,
        approval_id: ApprovalId,
    ) -> Result<(), ApprovalBridgeError> {
        self.decide(run_id, approval_id, ApprovalCommand::Deny)
    }

    pub(crate) fn cancel(&self, run_id: AgentRunId) -> bool {
        let Some(entry) = self.state.lock().unwrap().runs.remove(&run_id) else {
            return false;
        };
        entry.cancellation.cancel();
        let _ = entry.commands.send(ApprovalCommand::Cancel);
        true
    }

    fn decide(
        &self,
        run_id: AgentRunId,
        approval_id: ApprovalId,
        command: ApprovalCommand,
    ) -> Result<(), ApprovalBridgeError> {
        let mut state = self.state.lock().unwrap();
        let entry = state
            .runs
            .get_mut(&run_id)
            .ok_or(ApprovalBridgeError::UnknownRun)?;
        let pending = entry
            .pending
            .as_ref()
            .ok_or(ApprovalBridgeError::NotPending)?;
        if pending.approval_id != approval_id {
            return Err(ApprovalBridgeError::UnknownApproval);
        }
        entry.pending = None;
        entry
            .commands
            .send(command)
            .map_err(|_| ApprovalBridgeError::Disconnected)
    }

    fn record_event(&self, event: &AgentEvent, generation: u64) {
        let mut state = self.state.lock().unwrap();
        match event {
            AgentEvent::ApprovalRequested {
                run_id,
                approval_id,
                tool_name,
                arguments,
                reason,
            } => {
                if let Some(entry) = state
                    .runs
                    .get_mut(run_id)
                    .filter(|entry| entry.generation == generation)
                {
                    entry.pending = Some(ApprovalRequest {
                        run_id: *run_id,
                        approval_id: *approval_id,
                        tool_name: tool_name.clone(),
                        arguments: arguments.clone(),
                        reason: reason.clone(),
                    });
                }
            }
            AgentEvent::Completed { run_id }
            | AgentEvent::Cancelled { run_id }
            | AgentEvent::Failed { run_id, .. } => {
                if state
                    .runs
                    .get(run_id)
                    .is_some_and(|entry| entry.generation == generation)
                {
                    state.runs.remove(run_id);
                }
            }
            _ => {}
        }
    }

    fn remove(&self, run_id: AgentRunId, generation: u64) {
        let mut state = self.state.lock().unwrap();
        if state
            .runs
            .get(&run_id)
            .is_some_and(|entry| entry.generation == generation)
        {
            state.runs.remove(&run_id);
        }
    }
}

struct ApprovalRunHandle {
    bridge: ApprovalBridge,
    run_id: AgentRunId,
    generation: u64,
    receiver: Receiver<ApprovalCommand>,
}

impl ApprovalRunHandle {
    fn record_event(&self, event: &AgentEvent) {
        self.bridge.record_event(event, self.generation);
    }

    fn wait_for_decision(
        &self,
        _request: &ApprovalRequest,
    ) -> Result<ApprovalCommand, ApprovalBridgeError> {
        self.receiver
            .recv()
            .map_err(|_| ApprovalBridgeError::Disconnected)
    }
}

impl Drop for ApprovalRunHandle {
    fn drop(&mut self) {
        self.bridge.remove(self.run_id, self.generation);
    }
}

pub(crate) fn next_agent_run_id() -> AgentRunId {
    AgentRunId::new(NEXT_AGENT_RUN_ID.fetch_add(1, Ordering::Relaxed))
}

pub(crate) fn production_tool_definitions() -> Vec<axiom_ai_provider::ProviderToolDefinition> {
    vec![
        read_file_definition(),
        list_directory_definition(),
        fetch_url_definition(),
        write_file_definition(),
        update_file_definition(),
        delete_file_definition(),
    ]
}

pub(crate) fn gemini_agent_tool_definitions() -> Vec<axiom_ai_provider::ProviderToolDefinition> {
    vec![
        read_file_definition(),
        list_directory_definition(),
        write_file_definition(),
        update_file_definition(),
        delete_file_definition(),
    ]
}

pub(crate) struct AgentProviderAdapter {
    kind: ProviderKind,
    api_key: String,
    base_url: String,
    cancellation: Cancellation,
}

impl AgentProviderAdapter {
    pub(crate) fn new(
        kind: ProviderKind,
        api_key: String,
        base_url: String,
        cancellation: Cancellation,
    ) -> Self {
        Self {
            kind,
            api_key,
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
        provider_chat_stream_with_cancel(
            &self.kind,
            &self.api_key,
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
        if call.name == "update_file" {
            tracing::info!(
                target: "axiom.ai_diag",
                event = "update_file_adapter_entered",
                call_id_present = call.id.is_some(),
                arguments_object = call.arguments.is_object(),
                expected_fingerprint_present = call
                    .arguments
                    .get("expected_fingerprint")
                    .and_then(|value| value.as_str())
                    .is_some(),
                "[AI-DIAG]"
            );
        }
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
        let tool = result.tool;
        Ok(match result.result {
            Ok(output) => ToolOutcome::Success(agent_tool_result(tool, output)),
            Err(error) => ToolOutcome::ControlledError(tool_error_message(&error)),
        })
    }
}

fn agent_tool_result(tool: ToolName, output: super::tools::ToolOutput) -> String {
    if tool != ToolName::ReadFile {
        return output.content;
    }
    let mut metadata = serde_json::json!({
        "bytes": output.metadata.bytes,
        "range": output.metadata.range.as_ref().map(|range| serde_json::json!({
            "start_line": range.start_line,
            "end_line": range.end_line,
        })),
    });
    if let Some(fingerprint) = output.metadata.fingerprint {
        metadata["fingerprint"] = Value::String(fingerprint.to_wire_string());
    }
    serde_json::json!({
        "tool": "read_file",
        "path": output.metadata.path,
        "content": output.content,
        "metadata": metadata,
    })
    .to_string()
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
        "write_file" => {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "path must be a string".to_owned())?;
            let content = object
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| "content must be a string".to_owned())?;
            Ok(ToolRequest {
                name: ToolName::WriteFile,
                arguments: ToolArguments::WriteFile {
                    path: path.into(),
                    content: content.into(),
                },
            })
        }
        "update_file" => {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "path must be a string".to_owned())?;
            let expected_fingerprint =
                object
                    .get("expected_fingerprint")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "expected_fingerprint must be a string".to_owned())?;
            let content = object
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| "content must be a string".to_owned())?;
            Ok(ToolRequest {
                name: ToolName::UpdateFile,
                arguments: ToolArguments::UpdateFile {
                    path: path.into(),
                    expected_fingerprint: expected_fingerprint.into(),
                    content: content.into(),
                },
            })
        }
        "delete_file" => {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "path must be a string".to_owned())?;
            let expected_fingerprint =
                object
                    .get("expected_fingerprint")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "expected_fingerprint must be a string".to_owned())?;
            Ok(ToolRequest {
                name: ToolName::DeleteFile,
                arguments: ToolArguments::DeleteFile {
                    path: path.into(),
                    expected_fingerprint: expected_fingerprint.into(),
                },
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
        ToolError::AlreadyExists(path) => format!("file already exists: {path}"),
        ToolError::SymlinkNotAllowed(path) => {
            format!("symbolic links are not allowed in write paths: {path}")
        }
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
        ToolError::InvalidFingerprint => {
            "expected_fingerprint must be sha256:<64 lowercase hex chars>".into()
        }
        ToolError::FingerprintMismatch => {
            "file changed since it was read; refresh it and use the current fingerprint".into()
        }
        ToolError::NotRegularFile(path) => format!("path is not a regular file: {path}"),
        ToolError::CurrentFileTooLarge { path, .. } => {
            format!("current file is too large: {path}")
        }
    }
}

pub(crate) fn execute_agent_run(
    run: &mut AgentRun,
    request: ProviderChatRequest,
    provider_kind: ProviderKind,
    api_key: String,
    base_url: String,
    registry: Option<ToolRegistry>,
    workspace_root: Option<std::path::PathBuf>,
    bridge: &ApprovalBridge,
    queue: &AgentEventQueue,
) -> Result<axiom_agent::AgentExecutionResult, axiom_agent::AgentExecutionError> {
    let cancellation = run.cancellation();
    let mut provider =
        AgentProviderAdapter::new(provider_kind, api_key, base_url, cancellation.clone());
    let mut tools = match registry {
        Some(registry) => AgentToolAdapter::new(registry, cancellation),
        None => AgentToolAdapter::without_tools(cancellation),
    };
    let result = execute_with_approval(
        run,
        request,
        &mut provider,
        &mut tools,
        workspace_root
            .map(ProductionToolPolicy::with_workspace_root)
            .unwrap_or_default(),
        bridge,
        queue,
    );
    if let Err(error) = &result {
        let diagnostic = error.diagnostic();
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "agent_terminal_failure",
            run_id = run.id().value(),
            category = ?diagnostic.kind,
            "[AI-DIAG]"
        );
    }
    result
}

pub(crate) fn execute_with_approval<P, T, Policy>(
    run: &mut AgentRun,
    request: ProviderChatRequest,
    provider: &mut P,
    tools: &mut T,
    policy: Policy,
    bridge: &ApprovalBridge,
    queue: &AgentEventQueue,
) -> Result<axiom_agent::AgentExecutionResult, AgentExecutionError>
where
    P: ProviderExecutor,
    T: ToolExecutor,
    Policy: ToolPolicy,
{
    let handle = bridge.register(run.id(), run.cancellation());
    let mut executor = AgentExecutor::with_policy(provider, tools, policy);
    let mut result = executor.execute(run, request, &mut |event| {
        publish_event(&handle, queue, event);
    });

    loop {
        let approval = match result {
            Ok(result) => return Ok(result),
            Err(AgentExecutionError::ApprovalRequired(approval)) => approval,
            Err(error) => return Err(error),
        };
        let command = match handle.wait_for_decision(&approval) {
            Ok(command) => command,
            Err(_) => {
                if run.cancel() {
                    publish_event(&handle, queue, AgentEvent::Cancelled { run_id: run.id() });
                }
                return Err(AgentExecutionError::Cancelled);
            }
        };
        result = match command {
            ApprovalCommand::Approve => executor.approve(run, approval.approval_id, &mut |event| {
                publish_event(&handle, queue, event);
            }),
            ApprovalCommand::AllowForRequest => {
                executor.allow_for_request(run, approval.approval_id, &mut |event| {
                    publish_event(&handle, queue, event)
                })
            }
            ApprovalCommand::Deny => executor.deny(run, approval.approval_id, &mut |event| {
                publish_event(&handle, queue, event);
            }),
            ApprovalCommand::Cancel => {
                if run.cancel() {
                    publish_event(&handle, queue, AgentEvent::Cancelled { run_id: run.id() });
                }
                return Err(AgentExecutionError::Cancelled);
            }
        };
    }
}

fn publish_event(handle: &ApprovalRunHandle, queue: &AgentEventQueue, event: AgentEvent) {
    match &event {
        AgentEvent::ModelStarted { run_id } => tracing::info!(
            target: "axiom.ai_diag",
            event = "agent_provider_turn_started",
            run_id = run_id.value(),
            "[AI-DIAG]"
        ),
        AgentEvent::ToolRequested { run_id } => tracing::info!(
            target: "axiom.ai_diag",
            event = "agent_tool_requested",
            run_id = run_id.value(),
            "[AI-DIAG]"
        ),
        AgentEvent::ToolStarted { run_id } => tracing::info!(
            target: "axiom.ai_diag",
            event = "agent_tool_started",
            run_id = run_id.value(),
            "[AI-DIAG]"
        ),
        AgentEvent::ToolCompleted { run_id, succeeded } => tracing::info!(
            target: "axiom.ai_diag",
            event = "agent_tool_completed",
            run_id = run_id.value(),
            succeeded = *succeeded,
            "[AI-DIAG]"
        ),
        _ => {}
    }
    handle.record_event(&event);
    if let Ok(mut events) = queue.lock() {
        events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_project::{
        project_directory::ProjectDirectoryCapability, project_read::ProjectReadCapability,
        project_update::ProjectUpdateCapability,
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
        let (dir, mut adapter) = adapter();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::write(
            dir.path().join("App/ProductService.php"),
            "<?php final class ProductService {}",
        )
        .unwrap();
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
        let nested = ProviderToolCall {
            id: Some("nested-1".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "App/ProductService.php"}),
        };
        assert!(matches!(
            adapter.execute(&nested),
            Ok(ToolOutcome::Success(content)) if content.contains("ProductService")
        ));
        let traversal = ProviderToolCall {
            id: Some("traversal-1".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "../outside.php"}),
        };
        assert!(matches!(
            adapter.execute(&traversal),
            Ok(ToolOutcome::ControlledError(_))
        ));
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

    #[test]
    fn full_read_exposes_fingerprint_and_ranged_read_does_not() {
        let (dir, mut adapter) = adapter();
        std::fs::write(dir.path().join("file.txt"), "exact bytes").unwrap();
        let full = ProviderToolCall {
            id: Some("read-full".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "file.txt"}),
        };
        let ToolOutcome::Success(content) = adapter.execute(&full).unwrap() else {
            panic!("read failed");
        };
        let value: Value = serde_json::from_str(&content).unwrap();
        let expected = ProjectUpdateCapability::new(dir.path())
            .unwrap()
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        assert_eq!(value["metadata"]["fingerprint"], expected);

        let ranged = ProviderToolCall {
            id: Some("read-range".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "file.txt", "start_line": 1, "end_line": 1}),
        };
        let ToolOutcome::Success(content) = adapter.execute(&ranged).unwrap() else {
            panic!("ranged read failed");
        };
        let value: Value = serde_json::from_str(&content).unwrap();
        assert!(value["metadata"].get("fingerprint").is_none());
    }

    #[test]
    fn production_tool_set_registers_write_and_update_files() {
        let names: Vec<_> = production_tool_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "read_file",
                "list_directory",
                "fetch_url",
                "write_file",
                "update_file",
                "delete_file"
            ]
        );
    }

    #[test]
    fn gemini_agent_tool_set_contains_both_mutation_tools() {
        let names: Vec<_> = gemini_agent_tool_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "read_file",
                "list_directory",
                "write_file",
                "update_file",
                "delete_file"
            ]
        );
    }
}

#[cfg(test)]
mod approval_bridge_tests {
    use super::*;
    use axiom_agent::{ToolPolicyContext, ToolPolicyDecision};
    use axiom_ai_provider::{ChatRole, ProviderChatMessage, ProviderChatStreamEvent};
    use axiom_project::{
        project_delete::ProjectDeleteCapability, project_update::ProjectUpdateCapability,
        project_write::ProjectWriteCapability,
    };
    use std::{
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    fn request() -> ProviderChatRequest {
        ProviderChatRequest {
            model: "fake".into(),
            messages: vec![ProviderChatMessage {
                role: ChatRole::User,
                content: "run".into(),
                reasoning: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            think: None,
            thinking_level: None,
            tools: Some(Vec::new()),
        }
    }

    struct ScriptedProvider {
        scripts: Vec<Vec<ProviderChatStreamEvent>>,
        requests: Arc<Mutex<Vec<ProviderChatRequest>>>,
        calls: usize,
    }

    impl ProviderExecutor for ScriptedProvider {
        fn execute(
            &mut self,
            request: &ProviderChatRequest,
            emit: &mut dyn FnMut(ProviderChatStreamEvent),
        ) -> Result<(), ProviderError> {
            self.requests.lock().unwrap().push(request.clone());
            let events = self
                .scripts
                .get(self.calls)
                .cloned()
                .unwrap_or_else(|| vec![ProviderChatStreamEvent::Done]);
            self.calls += 1;
            for event in events {
                emit(event);
            }
            Ok(())
        }
    }

    struct RecordingTool {
        calls: Arc<Mutex<Vec<String>>>,
        outcomes: Vec<ToolOutcome>,
    }

    impl ToolExecutor for RecordingTool {
        fn execute(
            &mut self,
            call: &ProviderToolCall,
        ) -> Result<ToolOutcome, ToolInfrastructureError> {
            self.calls
                .lock()
                .unwrap()
                .push(call.id.clone().unwrap_or_else(|| call.name.clone()));
            Ok(self
                .outcomes
                .get(self.calls.lock().unwrap().len() - 1)
                .cloned()
                .unwrap_or(ToolOutcome::Success("tool-result".into())))
        }
    }

    struct ScriptedPolicy {
        decisions: Vec<ToolPolicyDecision>,
        calls: usize,
    }

    impl ToolPolicy for ScriptedPolicy {
        fn decide(
            &mut self,
            _context: ToolPolicyContext,
            _call: &ProviderToolCall,
        ) -> ToolPolicyDecision {
            let decision =
                self.decisions
                    .get(self.calls)
                    .cloned()
                    .unwrap_or(ToolPolicyDecision::Deny {
                        reason: "no scripted decision".into(),
                    });
            self.calls += 1;
            decision
        }
    }

    fn tool_call(id: &str) -> ProviderChatStreamEvent {
        ProviderChatStreamEvent::ToolCall(ProviderToolCall {
            id: Some(id.into()),
            name: "fake_tool".into(),
            arguments: serde_json::json!({"id": id}),
        })
    }

    fn write_tool_call(id: &str, path: &str, content: &str) -> ProviderChatStreamEvent {
        ProviderChatStreamEvent::ToolCall(ProviderToolCall {
            id: Some(id.into()),
            name: "write_file".into(),
            arguments: serde_json::json!({"path": path, "content": content}),
        })
    }

    fn update_tool_call(
        id: &str,
        path: &str,
        expected_fingerprint: &str,
        content: &str,
    ) -> ProviderChatStreamEvent {
        ProviderChatStreamEvent::ToolCall(ProviderToolCall {
            id: Some(id.into()),
            name: "update_file".into(),
            arguments: serde_json::json!({
                "path": path,
                "expected_fingerprint": expected_fingerprint,
                "content": content,
            }),
        })
    }

    fn write_adapter() -> (tempfile::TempDir, AgentToolAdapter) {
        let dir = tempfile::tempdir().unwrap();
        let read = axiom_project::project_read::ProjectReadCapability::new(dir.path()).unwrap();
        let directory =
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap();
        let write = ProjectWriteCapability::new(dir.path()).unwrap();
        let registry = ToolRegistry::new_with_write_file(
            read,
            directory,
            axiom_web::FetchUrlCapability::new(),
            write,
        );
        (
            dir,
            AgentToolAdapter::new(registry, Cancellation::default()),
        )
    }

    fn update_adapter() -> (tempfile::TempDir, AgentToolAdapter) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "old").unwrap();
        let read = axiom_project::project_read::ProjectReadCapability::new(dir.path()).unwrap();
        let directory =
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap();
        let write = ProjectWriteCapability::new(dir.path()).unwrap();
        let update = ProjectUpdateCapability::new(dir.path()).unwrap();
        let registry = ToolRegistry::new_with_mutations(
            read,
            directory,
            axiom_web::FetchUrlCapability::new(),
            write,
            update,
        );
        (
            dir,
            AgentToolAdapter::new(registry, Cancellation::default()),
        )
    }

    fn delete_adapter() -> (tempfile::TempDir, AgentToolAdapter, String) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "remove").unwrap();
        let read = axiom_project::project_read::ProjectReadCapability::new(dir.path()).unwrap();
        let directory =
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap();
        let write = ProjectWriteCapability::new(dir.path()).unwrap();
        let update = ProjectUpdateCapability::new(dir.path()).unwrap();
        let expected = update
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        let delete = ProjectDeleteCapability::new(dir.path()).unwrap();
        let registry = ToolRegistry::new_with_all_mutations(
            read,
            directory,
            axiom_web::FetchUrlCapability::new(),
            write,
            update,
            delete,
        );
        (
            dir,
            AgentToolAdapter::new(registry, Cancellation::default()),
            expected,
        )
    }

    struct CountingUpdateToolAdapter {
        inner: AgentToolAdapter,
        update_calls: usize,
    }

    impl ToolExecutor for CountingUpdateToolAdapter {
        fn execute(
            &mut self,
            call: &ProviderToolCall,
        ) -> Result<ToolOutcome, ToolInfrastructureError> {
            if call.name == "update_file" {
                self.update_calls += 1;
            }
            self.inner.execute(call)
        }
    }

    fn approval(reason: &str) -> ToolPolicyDecision {
        ToolPolicyDecision::RequireApproval {
            reason: reason.into(),
        }
    }

    fn wait_for_pending(bridge: &ApprovalBridge, run_id: AgentRunId) -> ApprovalRequest {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(request) = bridge.pending(run_id) {
                return request;
            }
            assert!(Instant::now() < deadline, "approval did not become pending");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn approved_write_executes_once_and_preserves_utf8_content() {
        let (dir, mut tools) = write_adapter();
        let mut provider = ScriptedProvider {
            scripts: vec![
                vec![
                    write_tool_call("write-1", "new.txt", "Olá, Axiom! 🚀"),
                    ProviderChatStreamEvent::Done,
                ],
                vec![
                    ProviderChatStreamEvent::ContentDelta("created".into()),
                    ProviderChatStreamEvent::Done,
                ],
            ],
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: 0,
        };
        let mut run = AgentRun::new(AgentRunId::new(501), axiom_agent::AgentBudget::new(2, 1));
        let mut executor =
            AgentExecutor::with_policy(&mut provider, &mut tools, ProductionToolPolicy::default());
        let error = executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();
        let approval = match error {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("expected approval: {other:?}"),
        };
        assert!(!dir.path().join("new.txt").exists());

        let result = executor
            .approve(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();
        assert!(
            !result
                .messages()
                .iter()
                .any(|message| message.content.contains("Olá, Axiom! 🚀"))
        );
        assert!(
            result
                .messages()
                .iter()
                .all(|message| !message.content.contains("<tool_call"))
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "Olá, Axiom! 🚀"
        );
        assert!(matches!(
            executor.approve(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(
                axiom_agent::ApprovalError::NotPending
            ))
        ));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "Olá, Axiom! 🚀"
        );
    }

    #[test]
    fn approved_delete_revalidates_fingerprint_and_deletes_once() {
        let (dir, mut tools, expected) = delete_adapter();
        let mut provider = ScriptedProvider {
            scripts: vec![vec![
                ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some("delete-1".into()),
                    name: "delete_file".into(),
                    arguments: serde_json::json!({
                        "path": "file.txt",
                        "expected_fingerprint": expected
                    }),
                }),
                ProviderChatStreamEvent::Done,
            ]],
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: 0,
        };
        let mut run = AgentRun::new(AgentRunId::new(503), axiom_agent::AgentBudget::new(1, 1));
        let mut executor =
            AgentExecutor::with_policy(&mut provider, &mut tools, ProductionToolPolicy::default());
        let error = executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();
        let approval = match error {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("expected approval: {other:?}"),
        };
        assert!(dir.path().join("file.txt").exists());
        executor
            .approve(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();
        assert!(!dir.path().join("file.txt").exists());
    }

    #[test]
    fn approved_update_executes_once_and_approval_hides_content() {
        let (dir, mut tools) = update_adapter();
        let expected = ProjectUpdateCapability::new(dir.path())
            .unwrap()
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        let mut provider = ScriptedProvider {
            scripts: vec![
                vec![
                    update_tool_call("update-1", "file.txt", &expected, "Olá, atualização! 🚀"),
                    ProviderChatStreamEvent::Done,
                ],
                vec![
                    ProviderChatStreamEvent::ContentDelta("updated".into()),
                    ProviderChatStreamEvent::Done,
                ],
            ],
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: 0,
        };
        let mut run = AgentRun::new(AgentRunId::new(503), axiom_agent::AgentBudget::new(2, 1));
        let mut executor =
            AgentExecutor::with_policy(&mut provider, &mut tools, ProductionToolPolicy::default());
        let error = executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();
        let approval = match error {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("expected approval: {other:?}"),
        };
        assert!(!approval.arguments.contains("Olá, atualização! 🚀"));
        assert!(approval.arguments.contains("content_bytes"));
        assert!(approval.arguments.contains(&expected));

        let result = executor
            .approve(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "Olá, atualização! 🚀"
        );
        assert!(
            !result
                .messages()
                .iter()
                .any(|message| message.content.contains("Olá, atualização! 🚀"))
        );
        assert!(matches!(
            executor.approve(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(
                axiom_agent::ApprovalError::NotPending
            ))
        ));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "Olá, atualização! 🚀"
        );
    }

    #[test]
    fn approved_update_rejects_external_replacement_on_production_path() {
        let (dir, adapter) = update_adapter();
        std::fs::write(dir.path().join("file.txt"), "STALE_A").unwrap();
        let mut tools = CountingUpdateToolAdapter {
            inner: adapter,
            update_calls: 0,
        };

        let read = ProviderToolCall {
            id: Some("read-stale".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "file.txt"}),
        };
        let ToolOutcome::Success(read_result) = tools.inner.execute(&read).unwrap() else {
            panic!("full production read_file failed");
        };
        let read_result: Value = serde_json::from_str(&read_result).unwrap();
        let expected = read_result["metadata"]["fingerprint"]
            .as_str()
            .expect("full read exposes fingerprint")
            .to_owned();
        assert_eq!(
            expected,
            "sha256:510b84ef4f86f13670e89c549a5670aa30bbe51c0c8e6a3895960e1df4cf987d"
        );

        let update_call = ProviderToolCall {
            id: Some("update-stale".into()),
            name: "update_file".into(),
            arguments: serde_json::json!({
                "path": "file.txt",
                "expected_fingerprint": expected,
                "content": "STALE_AGENT",
            }),
        };
        let mut provider = ScriptedProvider {
            scripts: vec![
                vec![
                    ProviderChatStreamEvent::ToolCall(update_call.clone()),
                    ProviderChatStreamEvent::Done,
                ],
                vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ],
            ],
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: 0,
        };
        let mut run = AgentRun::new(AgentRunId::new(505), axiom_agent::AgentBudget::new(2, 1));
        assert_eq!(tools.update_calls, 0);
        let mut executor =
            AgentExecutor::with_policy(&mut provider, &mut tools, ProductionToolPolicy::default());
        let error = executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();
        let approval = match error {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("expected update_file approval: {other:?}"),
        };
        assert_eq!(
            update_call.arguments["expected_fingerprint"].as_str(),
            Some(expected.as_str())
        );
        assert!(approval.arguments.contains(&expected));

        std::fs::write(dir.path().join("file.txt"), "STALE_EXTERNAL").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "STALE_EXTERNAL"
        );
        assert_eq!(
            update_call.arguments["expected_fingerprint"].as_str(),
            Some(expected.as_str())
        );

        let result = executor
            .approve(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();
        assert_eq!(tools.update_calls, 1);
        assert!(result.messages().iter().any(|message| {
            message.content
                == "file changed since it was read; refresh it and use the current fingerprint"
        }));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "STALE_EXTERNAL"
        );
    }

    #[test]
    fn denied_wrong_and_duplicate_update_approvals_never_mutate() {
        let (dir, mut tools) = update_adapter();
        let expected = ProjectUpdateCapability::new(dir.path())
            .unwrap()
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        let mut provider = ScriptedProvider {
            scripts: vec![
                vec![
                    update_tool_call("update-1", "file.txt", &expected, "must not write"),
                    ProviderChatStreamEvent::Done,
                ],
                vec![
                    ProviderChatStreamEvent::ContentDelta("denied".into()),
                    ProviderChatStreamEvent::Done,
                ],
            ],
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: 0,
        };
        let mut run = AgentRun::new(AgentRunId::new(504), axiom_agent::AgentBudget::new(2, 1));
        let mut executor =
            AgentExecutor::with_policy(&mut provider, &mut tools, ProductionToolPolicy::default());
        let error = executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();
        let approval = match error {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("expected approval: {other:?}"),
        };
        assert!(matches!(
            executor.approve(&mut run, ApprovalId::new(999), &mut |_| {}),
            Err(AgentExecutionError::Approval(
                axiom_agent::ApprovalError::UnknownApproval
            ))
        ));
        executor
            .deny(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "old"
        );
        assert!(matches!(
            executor.approve(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(
                axiom_agent::ApprovalError::NotPending
            ))
        ));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "old"
        );
    }

    #[test]
    fn denied_or_stale_write_approval_never_creates_a_file() {
        let (dir, mut tools) = write_adapter();
        let mut provider = ScriptedProvider {
            scripts: vec![
                vec![
                    write_tool_call("write-1", "blocked.txt", "must not exist"),
                    ProviderChatStreamEvent::Done,
                ],
                vec![
                    ProviderChatStreamEvent::ContentDelta("denied".into()),
                    ProviderChatStreamEvent::Done,
                ],
            ],
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: 0,
        };
        let mut run = AgentRun::new(AgentRunId::new(502), axiom_agent::AgentBudget::new(2, 1));
        let mut executor =
            AgentExecutor::with_policy(&mut provider, &mut tools, ProductionToolPolicy::default());
        let error = executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();
        let approval = match error {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("expected approval: {other:?}"),
        };
        assert!(matches!(
            executor.approve(&mut run, ApprovalId::new(999), &mut |_| {}),
            Err(AgentExecutionError::Approval(
                axiom_agent::ApprovalError::UnknownApproval
            ))
        ));
        executor
            .deny(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();
        assert!(!dir.path().join("blocked.txt").exists());
        assert!(matches!(
            executor.approve(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(
                axiom_agent::ApprovalError::NotPending
            ))
        ));
        assert!(!dir.path().join("blocked.txt").exists());
    }

    fn spawn_run(
        run_id: AgentRunId,
        mut provider: ScriptedProvider,
        mut tools: RecordingTool,
        policy: ScriptedPolicy,
        bridge: ApprovalBridge,
        queue: AgentEventQueue,
    ) -> (
        Arc<Mutex<AgentRun>>,
        Arc<Mutex<Vec<ProviderChatRequest>>>,
        Arc<Mutex<Vec<String>>>,
        thread::JoinHandle<Result<axiom_agent::AgentExecutionResult, AgentExecutionError>>,
    ) {
        let run = Arc::new(Mutex::new(AgentRun::new(
            run_id,
            axiom_agent::AgentBudget::new(2, 4),
        )));
        let requests = provider.requests.clone();
        let calls = tools.calls.clone();
        let run_for_thread = run.clone();
        let handle = thread::spawn(move || {
            let mut run = run_for_thread.lock().unwrap();
            execute_with_approval(
                &mut run,
                request(),
                &mut provider,
                &mut tools,
                policy,
                &bridge,
                &queue,
            )
        });
        (run, requests, calls, handle)
    }

    #[test]
    fn approval_request_is_safe_and_approve_resumes_without_repeating_provider_turn() {
        let bridge = ApprovalBridge::default();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = ScriptedProvider {
            scripts: vec![
                vec![
                    ProviderChatStreamEvent::ReasoningDelta("provider-local".into()),
                    tool_call("call-1"),
                    ProviderChatStreamEvent::Done,
                ],
                vec![
                    ProviderChatStreamEvent::ContentDelta("answer".into()),
                    ProviderChatStreamEvent::Done,
                ],
            ],
            requests: requests.clone(),
            calls: 0,
        };
        let tools = RecordingTool {
            calls: calls.clone(),
            outcomes: vec![ToolOutcome::Success("tool-result".into())],
        };
        let policy = ScriptedPolicy {
            decisions: vec![approval("needs review")],
            calls: 0,
        };
        let (run, _requests, _calls, handle) = spawn_run(
            AgentRunId::new(100),
            provider,
            tools,
            policy,
            bridge.clone(),
            queue.clone(),
        );

        let pending = wait_for_pending(&bridge, AgentRunId::new(100));
        assert_eq!(pending.run_id, AgentRunId::new(100));
        assert_eq!(pending.tool_name, "fake_tool");
        assert_eq!(pending.reason, "needs review");
        assert!(pending.arguments.contains("call-1"));
        assert!(!pending.arguments.contains("provider-local"));
        bridge
            .approve(AgentRunId::new(100), pending.approval_id)
            .unwrap();
        let result = handle.join().unwrap().unwrap();

        assert_eq!(calls.lock().unwrap().as_slice(), ["call-1"]);
        assert_eq!(requests.lock().unwrap().len(), 2);
        assert_eq!(run.lock().unwrap().usage().provider_turns, 2);
        assert_eq!(run.lock().unwrap().usage().tool_calls, 1);
        assert_eq!(
            result.messages()[1].reasoning.as_deref(),
            Some("provider-local")
        );
        let final_request = requests.lock().unwrap()[1].clone();
        assert!(final_request.tools.is_none());
        assert!(final_request.messages.iter().all(|message| {
            message.reasoning.is_none()
                && message.tool_calls.is_empty()
                && message.tool_call_id.is_none()
                && !message.content.contains("<tool_call")
        }));
        let events = queue.lock().unwrap().clone();
        assert!(matches!(events.last(), Some(AgentEvent::Completed { .. })));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentEvent::Completed { .. }
                        | AgentEvent::Cancelled { .. }
                        | AgentEvent::Failed { .. }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn deny_resumes_with_controlled_result_without_executing_tools() {
        let bridge = ApprovalBridge::default();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = ScriptedProvider {
            scripts: vec![
                vec![
                    ProviderChatStreamEvent::ReasoningDelta("provider-local".into()),
                    tool_call("call-1"),
                    ProviderChatStreamEvent::Done,
                ],
                vec![
                    ProviderChatStreamEvent::ContentDelta("answer".into()),
                    ProviderChatStreamEvent::Done,
                ],
            ],
            requests: requests.clone(),
            calls: 0,
        };
        let tools = RecordingTool {
            calls: calls.clone(),
            outcomes: Vec::new(),
        };
        let policy = ScriptedPolicy {
            decisions: vec![approval("needs review")],
            calls: 0,
        };
        let (run, _requests, _calls, handle) = spawn_run(
            AgentRunId::new(101),
            provider,
            tools,
            policy,
            bridge.clone(),
            queue.clone(),
        );
        let pending = wait_for_pending(&bridge, AgentRunId::new(101));
        bridge
            .deny(AgentRunId::new(101), pending.approval_id)
            .unwrap();
        let result = handle.join().unwrap().unwrap();

        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(requests.lock().unwrap().len(), 2);
        assert_eq!(run.lock().unwrap().usage().provider_turns, 2);
        assert_eq!(run.lock().unwrap().usage().tool_calls, 1);
        assert_eq!(
            result.messages()[1].reasoning.as_deref(),
            Some("provider-local")
        );
        let denied = result
            .messages()
            .iter()
            .find(|message| message.role == ChatRole::Tool)
            .unwrap();
        assert_eq!(denied.content, "tool execution denied by approval decision");
        assert!(matches!(
            queue.lock().unwrap().last(),
            Some(AgentEvent::Completed { .. })
        ));
    }

    #[test]
    fn multiple_tool_calls_keep_order_and_issue_a_new_approval_id() {
        let bridge = ApprovalBridge::default();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = ScriptedProvider {
            scripts: vec![
                vec![
                    tool_call("call-1"),
                    tool_call("call-2"),
                    ProviderChatStreamEvent::Done,
                ],
                vec![ProviderChatStreamEvent::Done],
            ],
            requests: requests.clone(),
            calls: 0,
        };
        let tools = RecordingTool {
            calls: calls.clone(),
            outcomes: vec![
                ToolOutcome::Success("first".into()),
                ToolOutcome::Success("second".into()),
            ],
        };
        let policy = ScriptedPolicy {
            decisions: vec![approval("first"), approval("second")],
            calls: 0,
        };
        let (_run, _requests, _calls, handle) = spawn_run(
            AgentRunId::new(102),
            provider,
            tools,
            policy,
            bridge.clone(),
            queue.clone(),
        );

        let first = wait_for_pending(&bridge, AgentRunId::new(102));
        assert!(calls.lock().unwrap().is_empty());
        bridge
            .approve(AgentRunId::new(102), first.approval_id)
            .unwrap();
        let second = wait_for_pending(&bridge, AgentRunId::new(102));
        assert_ne!(first.approval_id, second.approval_id);
        assert_eq!(calls.lock().unwrap().as_slice(), ["call-1"]);
        bridge
            .approve(AgentRunId::new(102), second.approval_id)
            .unwrap();
        handle.join().unwrap().unwrap();
        assert_eq!(calls.lock().unwrap().as_slice(), ["call-1", "call-2"]);
    }

    fn direct_pending(
        _bridge: &ApprovalBridge,
        handle: &ApprovalRunHandle,
        run_id: AgentRunId,
        approval_id: ApprovalId,
    ) {
        handle.record_event(&AgentEvent::ApprovalRequested {
            run_id,
            approval_id,
            tool_name: "fake_tool".into(),
            arguments: "{}".into(),
            reason: "review".into(),
        });
    }

    #[test]
    fn wrong_run_wrong_approval_duplicate_and_opposite_decisions_fail_closed() {
        let bridge = ApprovalBridge::default();
        let run_a = AgentRunId::new(200);
        let run_b = AgentRunId::new(201);
        let handle_a = bridge.register(run_a, Cancellation::default());
        let handle_b = bridge.register(run_b, Cancellation::default());
        direct_pending(&bridge, &handle_a, run_a, ApprovalId::new(1));
        direct_pending(&bridge, &handle_b, run_b, ApprovalId::new(1));

        assert_eq!(
            bridge.approve(run_b, ApprovalId::new(2)),
            Err(ApprovalBridgeError::UnknownApproval)
        );
        assert_eq!(
            bridge.approve(AgentRunId::new(999), ApprovalId::new(1)),
            Err(ApprovalBridgeError::UnknownRun)
        );
        bridge.approve(run_a, ApprovalId::new(1)).unwrap();
        assert_eq!(
            bridge.approve(run_a, ApprovalId::new(1)),
            Err(ApprovalBridgeError::NotPending)
        );
        assert_eq!(
            bridge.deny(run_a, ApprovalId::new(1)),
            Err(ApprovalBridgeError::NotPending)
        );
        bridge.deny(run_b, ApprovalId::new(1)).unwrap();
        assert_eq!(
            bridge.approve(run_b, ApprovalId::new(1)),
            Err(ApprovalBridgeError::NotPending)
        );
    }

    #[test]
    fn cancellation_terminal_and_stale_runs_reject_late_decisions() {
        let bridge = ApprovalBridge::default();
        let run_a = AgentRunId::new(300);
        let run_b = AgentRunId::new(301);
        let handle_a = bridge.register(run_a, Cancellation::default());
        let handle_b = bridge.register(run_b, Cancellation::default());
        direct_pending(&bridge, &handle_a, run_a, ApprovalId::new(1));
        direct_pending(&bridge, &handle_b, run_b, ApprovalId::new(1));

        bridge.cancel(run_a);
        assert_eq!(
            bridge.approve(run_a, ApprovalId::new(1)),
            Err(ApprovalBridgeError::UnknownRun)
        );
        assert!(bridge.pending(run_b).is_some());

        handle_b.record_event(&AgentEvent::Completed { run_id: run_b });
        assert_eq!(
            bridge.approve(run_b, ApprovalId::new(1)),
            Err(ApprovalBridgeError::UnknownRun)
        );
    }

    #[test]
    fn stop_while_waiting_cancels_the_run_without_executing_tools() {
        let bridge = ApprovalBridge::default();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = ScriptedProvider {
            scripts: vec![vec![tool_call("call-1"), ProviderChatStreamEvent::Done]],
            requests: requests.clone(),
            calls: 0,
        };
        let tools = RecordingTool {
            calls: calls.clone(),
            outcomes: Vec::new(),
        };
        let policy = ScriptedPolicy {
            decisions: vec![approval("needs review")],
            calls: 0,
        };
        let (run, _requests, _calls, handle) = spawn_run(
            AgentRunId::new(103),
            provider,
            tools,
            policy,
            bridge.clone(),
            queue.clone(),
        );
        let pending = wait_for_pending(&bridge, AgentRunId::new(103));
        bridge.cancel(AgentRunId::new(103));
        let result = handle.join().unwrap();
        assert!(matches!(result, Err(AgentExecutionError::Cancelled)));
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            run.lock().unwrap().state(),
            axiom_agent::AgentState::Cancelled
        );
        assert_eq!(
            bridge.approve(AgentRunId::new(103), pending.approval_id),
            Err(ApprovalBridgeError::UnknownRun)
        );
        assert!(matches!(
            queue.lock().unwrap().last(),
            Some(AgentEvent::Cancelled { .. })
        ));
    }
}
