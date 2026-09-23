//! Production adapters and the small UI-facing bridge for one Agent run.

use super::tool_orchestration::{
    fetch_url_definition, list_directory_definition, read_file_definition,
};
use super::tools::{ToolArguments, ToolError, ToolName, ToolRegistry, ToolRequest};
use axiom_agent::{
    AgentEvent, AgentExecutionError, AgentExecutor, AgentRun, AgentRunId, ApprovalId,
    ApprovalRequest, Cancellation, ProviderExecutor, ReadOnlyToolPolicy, ToolExecutor,
    ToolInfrastructureError, ToolOutcome, ToolPolicy, ToolPolicyContext, ToolPolicyDecision,
};
use axiom_ai_provider::{
    OllamaProvider, ProviderChatRequest, ProviderChatStreamEvent, ProviderError, ProviderToolCall,
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
    bridge: &ApprovalBridge,
    queue: &AgentEventQueue,
) -> Result<axiom_agent::AgentExecutionResult, axiom_agent::AgentExecutionError> {
    let cancellation = run.cancellation();
    let mut provider = AgentProviderAdapter::new(base_url, cancellation.clone());
    let mut tools = match registry {
        Some(registry) => AgentToolAdapter::new(registry, cancellation),
        None => AgentToolAdapter::without_tools(cancellation),
    };
    let result = execute_with_approval(
        run,
        request,
        &mut provider,
        &mut tools,
        ReadOnlyToolPolicy::default(),
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
            diagnostic = %diagnostic.message,
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

#[cfg(test)]
mod approval_bridge_tests {
    use super::*;
    use axiom_ai_provider::{ChatRole, ProviderChatMessage, ProviderChatStreamEvent};
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
