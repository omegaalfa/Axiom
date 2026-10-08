//! Provider-neutral, headless lifecycle contracts for one logical agent run.
//!
//! This crate deliberately does not execute providers or tools. It contains
//! only identity, state, budget, cancellation, and event semantics that a
//! future execution adapter can use.

mod memory;
mod memory_backend;
mod memory_local;
mod memory_manager;
mod system_prompt;

pub use memory::{
    InMemoryMemoryService, MemoryCategory, MemoryHandoff, MemoryObservation, MemoryResult,
    MemoryScope, MemoryService, MemorySessionId, NoMemory,
};
pub use memory_backend::{
    AiMemoryBackend, AiMemoryReadTransport, MAX_BACKEND_RESULTS, MemoryReadError,
    MemoryReadOperation, MemoryReadRequest,
};
pub use memory_local::{
    AxiomMemoryBackend, AxiomMemoryError, AxiomMemoryScopeIds, MAX_LOCAL_MEMORY_RESULTS,
    MAX_RECORD_CONTENT_CHARS,
};
pub use memory_manager::{
    MemoryBackendConfig, MemoryBackendLauncher, MemoryHealthCheck, MemoryLaunchRequest,
    MemoryManager, MemoryManagerError, MemoryStatus, MemoryUnavailableReason,
    default_data_directory,
};
pub use system_prompt::agent_system_instruction;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentRunId(u64);

impl AgentRunId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApprovalId(u64);

impl ApprovalId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Preparing,
    RunningModel,
    WaitingForTool,
    WaitingForApproval,
    RunningTool,
    Finalizing,
    Completed,
    Cancelled,
    Failed,
}

impl AgentState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: AgentState,
    pub to: AgentState,
}

impl std::fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid agent transition: {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for InvalidTransition {}

fn legal_transition(from: AgentState, to: AgentState) -> bool {
    matches!(
        (from, to),
        (AgentState::Idle, AgentState::Preparing)
            | (AgentState::Preparing, AgentState::RunningModel)
            | (AgentState::Preparing, AgentState::Failed)
            | (AgentState::Preparing, AgentState::Cancelled)
            | (AgentState::RunningModel, AgentState::WaitingForTool)
            | (AgentState::RunningModel, AgentState::Finalizing)
            | (AgentState::RunningModel, AgentState::Failed)
            | (AgentState::RunningModel, AgentState::Cancelled)
            | (AgentState::WaitingForTool, AgentState::RunningTool)
            | (AgentState::WaitingForTool, AgentState::RunningModel)
            | (AgentState::WaitingForTool, AgentState::WaitingForApproval)
            | (AgentState::WaitingForTool, AgentState::Failed)
            | (AgentState::WaitingForTool, AgentState::Cancelled)
            | (AgentState::WaitingForApproval, AgentState::RunningTool)
            | (AgentState::WaitingForApproval, AgentState::WaitingForTool)
            | (AgentState::WaitingForApproval, AgentState::RunningModel)
            | (AgentState::WaitingForApproval, AgentState::Failed)
            | (AgentState::WaitingForApproval, AgentState::Cancelled)
            | (AgentState::RunningTool, AgentState::RunningModel)
            | (AgentState::RunningTool, AgentState::WaitingForApproval)
            | (AgentState::RunningTool, AgentState::Failed)
            | (AgentState::RunningTool, AgentState::Cancelled)
            | (AgentState::Finalizing, AgentState::Completed)
            | (AgentState::Finalizing, AgentState::Failed)
            | (AgentState::Finalizing, AgentState::Cancelled)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentBudget {
    pub max_provider_turns: usize,
    pub max_tool_calls: usize,
}

impl AgentBudget {
    pub const fn new(max_provider_turns: usize, max_tool_calls: usize) -> Self {
        Self {
            max_provider_turns,
            max_tool_calls,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AgentUsage {
    pub provider_turns: usize,
    pub tool_calls: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetResource {
    ProviderTurns,
    ToolCalls,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetExceeded {
    pub resource: BudgetResource,
    pub limit: usize,
    pub attempted: usize,
}

impl std::fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent budget exceeded for {:?}", self.resource)
    }
}

impl std::error::Error for BudgetExceeded {}

#[derive(Clone, Debug, Default)]
pub struct Cancellation {
    cancelled: Arc<AtomicBool>,
}

impl Cancellation {
    pub fn cancel(&self) -> bool {
        !self.cancelled.swap(true, Ordering::AcqRel)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentFailureKind {
    Provider,
    Tool,
    BudgetExceeded,
    Transition,
    Infrastructure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentFailureDiagnostic {
    pub kind: AgentFailureKind,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub run_id: AgentRunId,
    pub approval_id: ApprovalId,
    pub tool_name: String,
    pub arguments: String,
    pub reason: String,
}

impl ApprovalRequest {
    fn from_call(
        run_id: AgentRunId,
        approval_id: ApprovalId,
        call: &axiom_ai_provider::ProviderToolCall,
        reason: &str,
    ) -> Self {
        Self {
            run_id,
            approval_id,
            tool_name: call.name.clone(),
            arguments: safe_approval_arguments(call),
            reason: bounded_arguments(reason),
        }
    }
}

fn safe_approval_arguments(call: &axiom_ai_provider::ProviderToolCall) -> String {
    if !matches!(
        call.name.as_str(),
        "write_file" | "update_file" | "delete_file"
    ) {
        return bounded_arguments(&call.arguments.to_string());
    }
    let arguments = call.arguments.as_object().cloned().unwrap_or_default();
    let path = arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(|path| serde_json::Value::String(path.to_owned()))
        .unwrap_or(serde_json::Value::Null);
    let content_bytes = arguments
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(|content| content.as_bytes().len())
        .unwrap_or(0);
    let mut preview = serde_json::json!({
        "tool": call.name,
        "path": path,
        "content_bytes": content_bytes,
    });
    if matches!(call.name.as_str(), "update_file" | "delete_file") {
        preview["expected_fingerprint"] = arguments
            .get("expected_fingerprint")
            .and_then(serde_json::Value::as_str)
            .map(|fingerprint| serde_json::Value::String(fingerprint.to_owned()))
            .unwrap_or_else(|| serde_json::Value::Null);
    }
    bounded_arguments(&preview.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalError {
    NotPending,
    UnknownApproval,
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPending => f.write_str("agent is not awaiting approval"),
            Self::UnknownApproval => f.write_str("unknown or stale approval"),
        }
    }
}

impl std::error::Error for ApprovalError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    RunStarted {
        run_id: AgentRunId,
    },
    ModelStarted {
        run_id: AgentRunId,
    },
    ThinkingDelta {
        run_id: AgentRunId,
        delta: String,
    },
    ContentDelta {
        run_id: AgentRunId,
        delta: String,
    },
    ToolRequested {
        run_id: AgentRunId,
    },
    ApprovalRequested {
        run_id: AgentRunId,
        approval_id: ApprovalId,
        tool_name: String,
        arguments: String,
        reason: String,
    },
    ToolStarted {
        run_id: AgentRunId,
    },
    ToolCompleted {
        run_id: AgentRunId,
        succeeded: bool,
    },
    Finalizing {
        run_id: AgentRunId,
    },
    Completed {
        run_id: AgentRunId,
    },
    Cancelled {
        run_id: AgentRunId,
    },
    Failed {
        run_id: AgentRunId,
        diagnostic: AgentFailureDiagnostic,
    },
}

impl AgentEvent {
    pub const fn run_id(&self) -> AgentRunId {
        match self {
            Self::RunStarted { run_id }
            | Self::ModelStarted { run_id }
            | Self::ThinkingDelta { run_id, .. }
            | Self::ContentDelta { run_id, .. }
            | Self::ToolRequested { run_id }
            | Self::ApprovalRequested { run_id, .. }
            | Self::ToolStarted { run_id }
            | Self::ToolCompleted { run_id, .. }
            | Self::Finalizing { run_id }
            | Self::Completed { run_id }
            | Self::Cancelled { run_id }
            | Self::Failed { run_id, .. } => *run_id,
        }
    }
}

pub fn is_stale_event(event: &AgentEvent, active_run_id: AgentRunId) -> bool {
    event.run_id() != active_run_id
}

#[derive(Clone, Debug)]
pub struct AgentRun {
    id: AgentRunId,
    state: AgentState,
    budget: AgentBudget,
    usage: AgentUsage,
    cancellation: Cancellation,
    next_approval_id: u64,
    pending_approval: Option<PendingApproval>,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingApproval {
    approval_id: ApprovalId,
    call: axiom_ai_provider::ProviderToolCall,
}

impl AgentRun {
    pub fn new(id: AgentRunId, budget: AgentBudget) -> Self {
        Self {
            id,
            state: AgentState::Idle,
            budget,
            usage: AgentUsage::default(),
            cancellation: Cancellation::default(),
            next_approval_id: 1,
            pending_approval: None,
        }
    }

    pub const fn id(&self) -> AgentRunId {
        self.id
    }

    pub const fn state(&self) -> AgentState {
        self.state
    }

    pub const fn budget(&self) -> AgentBudget {
        self.budget
    }

    pub const fn usage(&self) -> AgentUsage {
        self.usage
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }

    pub fn transition(&mut self, to: AgentState) -> Result<(), InvalidTransition> {
        if !legal_transition(self.state, to) {
            return Err(InvalidTransition {
                from: self.state,
                to,
            });
        }
        self.state = to;
        if to == AgentState::Cancelled {
            self.cancellation.cancel();
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> bool {
        if self.state.is_terminal() {
            return false;
        }
        self.cancellation.cancel();
        self.pending_approval = None;
        self.state = AgentState::Cancelled;
        true
    }

    fn issue_approval(
        &mut self,
        call: axiom_ai_provider::ProviderToolCall,
        reason: &str,
    ) -> ApprovalRequest {
        let approval_id = ApprovalId(self.next_approval_id);
        self.next_approval_id = self.next_approval_id.saturating_add(1);
        self.pending_approval = Some(PendingApproval {
            approval_id,
            call: call.clone(),
        });
        ApprovalRequest::from_call(self.id, approval_id, &call, reason)
    }

    fn resolve_approval(
        &mut self,
        approval_id: ApprovalId,
    ) -> Result<axiom_ai_provider::ProviderToolCall, ApprovalError> {
        if self.state != AgentState::WaitingForApproval {
            return Err(ApprovalError::NotPending);
        }
        let Some(pending) = self.pending_approval.as_ref() else {
            return Err(ApprovalError::NotPending);
        };
        if pending.approval_id != approval_id {
            return Err(ApprovalError::UnknownApproval);
        }
        Ok(self
            .pending_approval
            .take()
            .expect("pending approval exists")
            .call)
    }

    pub fn consume_provider_turn(&mut self) -> Result<(), BudgetExceeded> {
        consume(
            &mut self.usage.provider_turns,
            self.budget.max_provider_turns,
            BudgetResource::ProviderTurns,
        )
    }

    pub fn consume_tool_call(&mut self) -> Result<(), BudgetExceeded> {
        consume(
            &mut self.usage.tool_calls,
            self.budget.max_tool_calls,
            BudgetResource::ToolCalls,
        )
    }
}

fn consume(
    usage: &mut usize,
    limit: usize,
    resource: BudgetResource,
) -> Result<(), BudgetExceeded> {
    let attempted = if *usage == usize::MAX {
        usize::MAX
    } else {
        *usage + 1
    };
    if *usage >= limit {
        return Err(BudgetExceeded {
            resource,
            limit,
            attempted,
        });
    }
    *usage += 1;
    Ok(())
}

pub trait ProviderExecutor {
    fn execute(
        &mut self,
        request: &axiom_ai_provider::ProviderChatRequest,
        emit: &mut dyn FnMut(axiom_ai_provider::ProviderChatStreamEvent),
    ) -> Result<(), axiom_ai_provider::ProviderError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolOutcome {
    Success(String),
    ControlledError(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolInfrastructureError {
    pub message: String,
}

impl std::fmt::Display for ToolInfrastructureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ToolInfrastructureError {}

pub trait ToolExecutor {
    fn execute(
        &mut self,
        call: &axiom_ai_provider::ProviderToolCall,
    ) -> Result<ToolOutcome, ToolInfrastructureError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolPolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolPolicyContext {
    pub run_id: AgentRunId,
}

pub trait ToolPolicy {
    fn decide(
        &mut self,
        context: ToolPolicyContext,
        call: &axiom_ai_provider::ProviderToolCall,
    ) -> ToolPolicyDecision;

    fn is_run_scoped_allowed(
        &mut self,
        _context: ToolPolicyContext,
        _call: &axiom_ai_provider::ProviderToolCall,
    ) -> bool {
        false
    }

    fn grant_run_scoped(
        &mut self,
        _context: ToolPolicyContext,
        _call: &axiom_ai_provider::ProviderToolCall,
    ) {
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ApprovalScope {
    pub run_id: AgentRunId,
    pub tool_name: String,
    pub canonical_workspace_path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadOnlyToolPolicy {
    workspace_root: Option<PathBuf>,
    run_scoped_approvals: HashSet<ApprovalScope>,
}

impl ReadOnlyToolPolicy {
    pub fn with_workspace_root(root: PathBuf) -> Self {
        Self {
            workspace_root: Some(root),
            run_scoped_approvals: HashSet::new(),
        }
    }

    fn canonical_update_path(&self, call: &axiom_ai_provider::ProviderToolCall) -> Option<PathBuf> {
        if call.name != "update_file" {
            return None;
        }
        let path = call.arguments.get("path")?.as_str()?;
        let root = self.workspace_root.as_deref()?;
        let path = Path::new(path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let canonical = std::fs::canonicalize(path).ok()?;
        let canonical_root = std::fs::canonicalize(root).ok()?;
        canonical.starts_with(&canonical_root).then_some(canonical)
    }
}

impl ToolPolicy for ReadOnlyToolPolicy {
    fn is_run_scoped_allowed(
        &mut self,
        context: ToolPolicyContext,
        call: &axiom_ai_provider::ProviderToolCall,
    ) -> bool {
        let Some(path) = self.canonical_update_path(call) else {
            return false;
        };
        self.run_scoped_approvals.contains(&ApprovalScope {
            run_id: context.run_id,
            tool_name: "update_file".into(),
            canonical_workspace_path: path,
        })
    }

    fn grant_run_scoped(
        &mut self,
        context: ToolPolicyContext,
        call: &axiom_ai_provider::ProviderToolCall,
    ) {
        let Some(path) = self.canonical_update_path(call) else {
            return;
        };
        self.run_scoped_approvals.insert(ApprovalScope {
            run_id: context.run_id,
            tool_name: "update_file".into(),
            canonical_workspace_path: path,
        });
    }

    fn decide(
        &mut self,
        _context: ToolPolicyContext,
        call: &axiom_ai_provider::ProviderToolCall,
    ) -> ToolPolicyDecision {
        match call.name.as_str() {
            "read_file" | "list_directory" | "find_files" | "find_symbol" | "find_references"
            | "fetch_url" | "search_text" | "memory_search" | "memory_get" => {
                ToolPolicyDecision::Allow
            }
            "write_file" | "update_file" | "delete_file" => ToolPolicyDecision::RequireApproval {
                reason: "file mutation requires approval".into(),
            },
            _ => ToolPolicyDecision::Deny {
                reason: format!("tool '{}' is not allowed", call.name),
            },
        }
    }
}

pub type ProductionToolPolicy = ReadOnlyToolPolicy;

#[derive(Debug)]
pub enum AgentExecutionError {
    Provider(axiom_ai_provider::ProviderError),
    ProviderProtocol(&'static str),
    Tool(ToolInfrastructureError),
    BudgetExceeded(BudgetExceeded),
    ApprovalRequired(ApprovalRequest),
    Approval(ApprovalError),
    Cancelled,
    Transition(InvalidTransition),
}

impl std::fmt::Display for AgentExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(error) => write!(f, "provider execution failed: {error:?}"),
            Self::ProviderProtocol(message) => f.write_str(message),
            Self::Tool(error) => write!(f, "tool execution failed: {error}"),
            Self::BudgetExceeded(error) => error.fmt(f),
            Self::ApprovalRequired(request) => write!(
                f,
                "approval required for {} (approval_id={})",
                request.tool_name,
                request.approval_id.value()
            ),
            Self::Approval(error) => error.fmt(f),
            Self::Cancelled => f.write_str("agent run cancelled"),
            Self::Transition(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AgentExecutionError {}

impl AgentExecutionError {
    pub fn diagnostic(&self) -> AgentFailureDiagnostic {
        match self {
            Self::Provider(error) => AgentFailureDiagnostic {
                kind: AgentFailureKind::Provider,
                message: error.detailed_user_message(),
            },
            Self::ProviderProtocol(message) => AgentFailureDiagnostic {
                kind: AgentFailureKind::Provider,
                message: (*message).into(),
            },
            Self::Tool(error) => AgentFailureDiagnostic {
                kind: AgentFailureKind::Tool,
                message: concise_diagnostic(&error.to_string()),
            },
            Self::BudgetExceeded(error) => AgentFailureDiagnostic {
                kind: AgentFailureKind::BudgetExceeded,
                message: format!(
                    "{} (limit={}, attempted={})",
                    match error.resource {
                        BudgetResource::ProviderTurns => "provider turns",
                        BudgetResource::ToolCalls => "tool calls",
                    },
                    error.limit,
                    error.attempted
                ),
            },
            Self::ApprovalRequired(request) => AgentFailureDiagnostic {
                kind: AgentFailureKind::Infrastructure,
                message: format!(
                    "approval required for {} (approval_id={})",
                    request.tool_name,
                    request.approval_id.value()
                ),
            },
            Self::Approval(error) => AgentFailureDiagnostic {
                kind: AgentFailureKind::Infrastructure,
                message: error.to_string(),
            },
            Self::Cancelled => AgentFailureDiagnostic {
                kind: AgentFailureKind::Infrastructure,
                message: "agent run cancelled".into(),
            },
            Self::Transition(error) => AgentFailureDiagnostic {
                kind: AgentFailureKind::Transition,
                message: concise_diagnostic(&error.to_string()),
            },
        }
    }
}

fn concise_diagnostic(message: &str) -> String {
    let mut result = message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(240)
        .collect::<String>();
    if message.chars().count() > 240 {
        result.push('…');
    }
    result
}

fn bounded_arguments(arguments: &str) -> String {
    let mut result = arguments.chars().take(4096).collect::<String>();
    if arguments.chars().count() > 4096 {
        result.push('…');
    }
    result
}

fn contains_tool_protocol_markup(content: &str) -> bool {
    let content = content.to_ascii_lowercase();
    content.contains("<tool_call")
        || content.contains("</tool_call")
        || content.contains("<function=")
        || content.contains("</function>")
}

fn finalization_request(
    request: &axiom_ai_provider::ProviderChatRequest,
) -> axiom_ai_provider::ProviderChatRequest {
    let original = request
        .messages
        .iter()
        .filter(|message| {
            !matches!(
                message.role,
                axiom_ai_provider::ChatRole::Assistant | axiom_ai_provider::ChatRole::Tool
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let tool_results = request
        .messages
        .iter()
        .filter(|message| message.role == axiom_ai_provider::ChatRole::Tool)
        .map(|message| {
            let id = message.tool_call_id.as_deref().unwrap_or("unknown");
            format!("[tool result {id}]\n{}", message.content)
        })
        .collect::<Vec<_>>();
    let context = if tool_results.is_empty() {
        "Tool results/context for the final response:\n\nNo tool results are available.".into()
    } else {
        format!(
            "Tool results/context for the final response:\n\n{}",
            tool_results.join("\n\n")
        )
    };
    let mut messages = vec![axiom_ai_provider::ProviderChatMessage {
        role: axiom_ai_provider::ChatRole::System,
        content: context,
        reasoning: None,
        tool_call_id: None,
        tool_calls: Vec::new(),
    }];
    messages.extend(original);
    messages.push(axiom_ai_provider::ProviderChatMessage {
        role: axiom_ai_provider::ChatRole::User,
        content: "Produce the final user-facing answer for the original request using the tool context above. Do not call tools or emit tool-call markup.".into(),
        reasoning: None,
        tool_call_id: None,
        tool_calls: Vec::new(),
    });
    axiom_ai_provider::ProviderChatRequest {
        tools: None,
        messages,
        ..request.clone()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RequestMetadata {
    message_count: usize,
    user_count: usize,
    system_count: usize,
    assistant_count: usize,
    tool_count: usize,
    last_role: &'static str,
    tools_present: bool,
    think_enabled: bool,
    think_present: bool,
}

impl RequestMetadata {
    fn from_request(request: &axiom_ai_provider::ProviderChatRequest) -> Self {
        let mut metadata = Self {
            message_count: request.messages.len(),
            user_count: 0,
            system_count: 0,
            assistant_count: 0,
            tool_count: 0,
            last_role: "none",
            tools_present: request.tools.is_some(),
            think_enabled: request.think.unwrap_or(false),
            think_present: request.think.is_some(),
        };
        for message in &request.messages {
            match message.role {
                axiom_ai_provider::ChatRole::User => metadata.user_count += 1,
                axiom_ai_provider::ChatRole::System => metadata.system_count += 1,
                axiom_ai_provider::ChatRole::Assistant => metadata.assistant_count += 1,
                axiom_ai_provider::ChatRole::Tool => metadata.tool_count += 1,
            }
            metadata.last_role = match message.role {
                axiom_ai_provider::ChatRole::User => "user",
                axiom_ai_provider::ChatRole::System => "system",
                axiom_ai_provider::ChatRole::Assistant => "assistant",
                axiom_ai_provider::ChatRole::Tool => "tool",
            };
        }
        metadata
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentExecutionResult {
    pub content: String,
    pub thinking: String,
    messages: Vec<axiom_ai_provider::ProviderChatMessage>,
}

impl AgentExecutionResult {
    pub fn messages(&self) -> &[axiom_ai_provider::ProviderChatMessage] {
        &self.messages
    }
}

struct ExecutionContinuation {
    request: axiom_ai_provider::ProviderChatRequest,
    content: String,
    thinking: String,
    calls: Vec<axiom_ai_provider::ProviderToolCall>,
    tool_running: bool,
}

enum ToolProcessing {
    Complete(ExecutionContinuation),
    Pending {
        continuation: ExecutionContinuation,
        approval: ApprovalRequest,
    },
}

pub struct AgentExecutor<'a, P, T, Policy = ReadOnlyToolPolicy> {
    provider: &'a mut P,
    tools: &'a mut T,
    policy: Policy,
    memory: Box<dyn MemoryService>,
    memory_scope: MemoryScope,
    memory_session: Option<MemorySessionId>,
    memory_observations: usize,
    pending: Option<ExecutionContinuation>,
}

const MAX_MEMORY_OBSERVATIONS_PER_RUN: usize = 32;
const MAX_MEMORY_BRIEFING_ITEMS: usize = 6;
const MAX_MEMORY_BRIEFING_ITEM_CHARS: usize = 512;
const MAX_MEMORY_BRIEFING_CHARS: usize = 2048;

fn with_agent_system_instruction(
    mut request: axiom_ai_provider::ProviderChatRequest,
) -> axiom_ai_provider::ProviderChatRequest {
    // This is the sole injection point owned by AgentExecutor. Callers pass
    // dynamic messages only, so structural ownership guarantees one copy.
    request.messages.insert(
        0,
        axiom_ai_provider::ProviderChatMessage {
            role: axiom_ai_provider::ChatRole::System,
            content: agent_system_instruction().into(),
            reasoning: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    );
    request
}

impl<'a, P, T> AgentExecutor<'a, P, T, ReadOnlyToolPolicy>
where
    P: ProviderExecutor,
    T: ToolExecutor,
{
    pub fn new(provider: &'a mut P, tools: &'a mut T) -> Self {
        Self::with_policy(provider, tools, ReadOnlyToolPolicy::default())
    }
}

impl<'a, P, T, Policy> AgentExecutor<'a, P, T, Policy>
where
    P: ProviderExecutor,
    T: ToolExecutor,
    Policy: ToolPolicy,
{
    pub fn with_policy(provider: &'a mut P, tools: &'a mut T, policy: Policy) -> Self {
        Self::with_policy_and_memory(provider, tools, policy, Box::new(NoMemory))
    }

    pub fn with_policy_and_memory(
        provider: &'a mut P,
        tools: &'a mut T,
        policy: Policy,
        memory: Box<dyn MemoryService>,
    ) -> Self {
        Self::with_policy_and_memory_in_scope(
            provider,
            tools,
            policy,
            memory,
            MemoryScope::Workspace,
        )
    }

    pub fn with_policy_and_memory_in_scope(
        provider: &'a mut P,
        tools: &'a mut T,
        policy: Policy,
        memory: Box<dyn MemoryService>,
        memory_scope: MemoryScope,
    ) -> Self {
        Self {
            provider,
            tools,
            policy,
            memory,
            memory_scope,
            memory_session: None,
            memory_observations: 0,
            pending: None,
        }
    }

    pub fn memory_service(&self) -> &dyn MemoryService {
        self.memory.as_ref()
    }

    pub fn execute(
        &mut self,
        run: &mut AgentRun,
        request: axiom_ai_provider::ProviderChatRequest,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        self.start_memory_session();
        emit(AgentEvent::RunStarted { run_id: run.id() });
        run.transition(AgentState::Preparing)
            .map_err(AgentExecutionError::Transition)?;
        run.transition(AgentState::RunningModel)
            .map_err(AgentExecutionError::Transition)?;
        let request = self.inject_memory_briefing(with_agent_system_instruction(request));
        let result = self.drive(
            run,
            ExecutionContinuation {
                request,
                content: String::new(),
                thinking: String::new(),
                calls: Vec::new(),
                tool_running: false,
            },
            false,
            emit,
        );
        self.finish_memory_if_terminal(run, &result);
        result
    }

    pub fn approve(
        &mut self,
        run: &mut AgentRun,
        approval_id: ApprovalId,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        let pending = run
            .pending_approval
            .as_ref()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if pending.approval_id != approval_id {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        let stored_call = pending.call.clone();
        let continuation = self
            .pending
            .as_ref()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if continuation.calls.first() != Some(&stored_call) {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        let call = run
            .resolve_approval(approval_id)
            .map_err(AgentExecutionError::Approval)?;
        let continuation = self
            .pending
            .take()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if continuation.calls.first() != Some(&call) {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        let result = self.drive(run, continuation, true, emit);
        self.finish_memory_if_terminal(run, &result);
        result
    }

    pub fn allow_for_request(
        &mut self,
        run: &mut AgentRun,
        approval_id: ApprovalId,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        let pending = run
            .pending_approval
            .as_ref()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if pending.approval_id != approval_id {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        let stored_call = pending.call.clone();
        let continuation = self
            .pending
            .as_ref()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if continuation.calls.first() != Some(&stored_call) {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        self.policy
            .grant_run_scoped(ToolPolicyContext { run_id: run.id() }, &stored_call);
        self.approve(run, approval_id, emit)
    }

    pub fn deny(
        &mut self,
        run: &mut AgentRun,
        approval_id: ApprovalId,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        let pending = run
            .pending_approval
            .as_ref()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if pending.approval_id != approval_id {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        let stored_call = pending.call.clone();
        let continuation = self
            .pending
            .as_ref()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if continuation.calls.first() != Some(&stored_call) {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        let call = run
            .resolve_approval(approval_id)
            .map_err(AgentExecutionError::Approval)?;
        let mut continuation = self
            .pending
            .take()
            .ok_or(AgentExecutionError::Approval(ApprovalError::NotPending))?;
        if continuation.calls.first() != Some(&call) {
            return Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval,
            ));
        }
        continuation.calls.remove(0);
        continuation.request.messages.push(tool_message(
            call.id,
            "tool execution denied by approval decision".into(),
        ));
        emit(AgentEvent::ToolCompleted {
            run_id: run.id(),
            succeeded: false,
        });
        if !continuation.calls.is_empty() {
            run.transition(if continuation.tool_running {
                AgentState::RunningTool
            } else {
                AgentState::WaitingForTool
            })
            .map_err(AgentExecutionError::Transition)?;
        } else {
            run.transition(AgentState::RunningModel)
                .map_err(AgentExecutionError::Transition)?;
        }
        let result = self.drive(run, continuation, false, emit);
        self.finish_memory_if_terminal(run, &result);
        result
    }

    fn start_memory_session(&mut self) {
        if self.memory_session.is_none() {
            self.memory_observations = 0;
            self.memory_session = Some(self.memory.session_start(self.memory_scope));
        }
    }

    fn inject_memory_briefing(
        &mut self,
        mut request: axiom_ai_provider::ProviderChatRequest,
    ) -> axiom_ai_provider::ProviderChatRequest {
        let results = self
            .memory
            .briefing(self.memory_scope, MAX_MEMORY_BRIEFING_ITEMS);
        let mut content = String::from(
            "Memory briefing (historical, possibly stale or incomplete). Verify current project state; current deterministic state wins over memory.\n",
        );
        let mut included = 0;
        for result in results {
            if result.scope != self.memory_scope || included >= MAX_MEMORY_BRIEFING_ITEMS {
                continue;
            }
            let item = result
                .content
                .chars()
                .take(MAX_MEMORY_BRIEFING_ITEM_CHARS)
                .collect::<String>();
            if item.is_empty() {
                continue;
            }
            let line = format!("- {item}\n");
            if content.chars().count() + line.chars().count() > MAX_MEMORY_BRIEFING_CHARS {
                break;
            }
            content.push_str(&line);
            included += 1;
        }
        if included == 0 {
            return request;
        }
        let position = request
            .messages
            .iter()
            .position(|message| message.role != axiom_ai_provider::ChatRole::System)
            .unwrap_or(request.messages.len());
        request.messages.insert(
            position,
            axiom_ai_provider::ProviderChatMessage {
                role: axiom_ai_provider::ChatRole::System,
                content,
                reasoning: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
        );
        request
    }

    fn finish_memory_if_terminal(
        &mut self,
        run: &AgentRun,
        result: &Result<AgentExecutionResult, AgentExecutionError>,
    ) {
        if !run.state().is_terminal() {
            return;
        }
        let Some(session) = self.memory_session else {
            return;
        };
        let outcome = match result {
            Ok(_) => "completed successfully",
            Err(AgentExecutionError::Cancelled) => "cancelled",
            Err(AgentExecutionError::Provider(_))
            | Err(AgentExecutionError::ProviderProtocol(_)) => "failed due to provider error",
            Err(AgentExecutionError::Tool(_)) => "failed due to tool error",
            Err(AgentExecutionError::BudgetExceeded(_)) => "tool budget exhausted",
            Err(_) => "failed",
        };
        self.observe_memory(MemoryObservation::TaskOutcome(outcome.into()));
        self.memory.session_end(session);
        self.memory_session = None;
    }

    fn observe_memory(&mut self, observation: MemoryObservation) {
        let Some(session) = self.memory_session else {
            return;
        };
        if self.memory_observations >= MAX_MEMORY_OBSERVATIONS_PER_RUN {
            return;
        }
        self.memory_observations += 1;
        self.memory.observe(session, observation);
    }

    fn observe_tool_event(&mut self, call: &axiom_ai_provider::ProviderToolCall, succeeded: bool) {
        if !succeeded {
            return;
        }
        let observation = if matches!(
            call.name.as_str(),
            "write_file" | "update_file" | "delete_file"
        ) {
            Some(MemoryObservation::MutationSummary(format!(
                "{} completed",
                call.name
            )))
        } else if matches!(
            call.name.as_str(),
            "run_tests" | "run_test" | "validate" | "check" | "cargo_check"
        ) {
            Some(MemoryObservation::Validation(format!(
                "{} completed successfully",
                call.name
            )))
        } else if matches!(
            call.name.as_str(),
            "list_directory" | "find_files" | "find_symbol" | "find_references"
        ) {
            Some(MemoryObservation::Discovery(format!(
                "{} completed",
                call.name
            )))
        } else {
            None
        };
        if let Some(observation) = observation {
            self.observe_memory(observation);
        }
    }

    fn drive(
        &mut self,
        run: &mut AgentRun,
        mut continuation: ExecutionContinuation,
        mut approved_first: bool,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        loop {
            if run.cancellation().is_cancelled() {
                return self.cancelled(run, emit);
            }
            if !continuation.calls.is_empty() {
                match self.process_tool_calls(run, continuation, approved_first, emit)? {
                    ToolProcessing::Complete(next) => {
                        continuation = next;
                        approved_first = false;
                        continue;
                    }
                    ToolProcessing::Pending {
                        continuation: next,
                        approval,
                    } => {
                        self.pending = Some(next);
                        return Err(AgentExecutionError::ApprovalRequired(approval));
                    }
                }
            }

            if let Err(error) = run.consume_provider_turn() {
                if error.resource == BudgetResource::ProviderTurns {
                    tracing::warn!(
                        target: "axiom.ai_diag",
                        event = "agent_tool_budget_exhausted",
                        tool_rounds_used = run.usage().provider_turns,
                        tool_round_limit = run.budget().max_provider_turns,
                        "provider turn budget exhausted before another tool round",
                    );
                }
                return self.failed(run, emit, AgentExecutionError::BudgetExceeded(error));
            }
            let finalization_turn = run.usage().provider_turns == run.budget().max_provider_turns;
            if finalization_turn {
                tracing::warn!(
                    target: "axiom.ai_diag",
                    event = "agent_tool_budget_exhausted",
                    tool_rounds_used = run.usage().provider_turns.saturating_sub(1),
                    tool_round_limit = run.budget().max_provider_turns.saturating_sub(1),
                    "tool round budget exhausted; entering bounded finalization",
                );
            }
            let request_for_turn = if finalization_turn {
                finalization_request(&continuation.request)
            } else {
                continuation.request.clone()
            };
            let request_metadata = RequestMetadata::from_request(&request_for_turn);
            tracing::info!(
                target: "axiom.ai_diag",
                event = "agent_request_metadata",
                run_id = run.id().value(),
                provider_turn = run.usage().provider_turns,
                finalization = finalization_turn,
                tools_present = request_metadata.tools_present,
                message_count = request_metadata.message_count,
                user_count = request_metadata.user_count,
                system_count = request_metadata.system_count,
                assistant_count = request_metadata.assistant_count,
                tool_count = request_metadata.tool_count,
                last_role = request_metadata.last_role,
                think_enabled = request_metadata.think_enabled,
                think_present = request_metadata.think_present,
                "[AI-DIAG]"
            );
            emit(AgentEvent::ModelStarted { run_id: run.id() });
            let mut calls = Vec::new();
            let mut final_content = String::new();
            let mut provider_reasoning = String::new();
            let mut provider_thought_signature = None;
            self.provider
                .execute(&request_for_turn, &mut |event| match event {
                    axiom_ai_provider::ProviderChatStreamEvent::ThinkingDelta(delta) => {
                        continuation.thinking.push_str(&delta);
                        emit(AgentEvent::ThinkingDelta {
                            run_id: run.id(),
                            delta,
                        });
                    }
                    axiom_ai_provider::ProviderChatStreamEvent::ReasoningDelta(delta) => {
                        provider_reasoning.push_str(&delta);
                    }
                    axiom_ai_provider::ProviderChatStreamEvent::ContentDelta(delta) => {
                        if finalization_turn {
                            final_content.push_str(&delta);
                        } else {
                            continuation.content.push_str(&delta);
                            emit(AgentEvent::ContentDelta {
                                run_id: run.id(),
                                delta,
                            });
                        }
                    }
                    axiom_ai_provider::ProviderChatStreamEvent::ToolCall(call) => {
                        tracing::info!(
                            target: "axiom.ai_diag",
                            event = "agent_provider_tool_call",
                            round = run.usage().provider_turns,
                            name = call.name.as_str(),
                            call_id_present = call.id.is_some(),
                            thought_signature_present = provider_thought_signature.is_some(),
                            relative_path = call.arguments.get("path").and_then(|value| value.as_str()).unwrap_or(""),
                            classification = "native_function_call",
                            "[AI-DIAG]"
                        );
                        calls.push(call)
                    }
                    axiom_ai_provider::ProviderChatStreamEvent::ResponseMetadata(metadata) => {
                        let axiom_ai_provider::ProviderResponseMetadata::GoogleThoughtSignature(
                            signature,
                        ) = metadata;
                        provider_thought_signature = Some(signature);
                    }
                    axiom_ai_provider::ProviderChatStreamEvent::Done => {}
                })
                .map_err(|error| self.fail_provider(run, emit, error))?;
            if run.cancellation().is_cancelled() {
                return self.cancelled(run, emit);
            }
            if finalization_turn && !calls.is_empty() {
                return self.failed(
                    run,
                    emit,
                    AgentExecutionError::ProviderProtocol(
                        "provider emitted a tool call while tools were disabled",
                    ),
                );
            }
            if finalization_turn && contains_tool_protocol_markup(&final_content) {
                return self.failed(
                    run,
                    emit,
                    AgentExecutionError::ProviderProtocol(
                        "provider emitted tool-call markup while tools were disabled",
                    ),
                );
            }
            if finalization_turn && !final_content.is_empty() {
                continuation.content.push_str(&final_content);
                emit(AgentEvent::ContentDelta {
                    run_id: run.id(),
                    delta: final_content,
                });
            }
            if calls.is_empty() {
                if run.cancellation().is_cancelled() {
                    return self.cancelled(run, emit);
                }
                run.transition(AgentState::Finalizing)
                    .map_err(AgentExecutionError::Transition)?;
                emit(AgentEvent::Finalizing { run_id: run.id() });
                if run.cancellation().is_cancelled() {
                    return self.cancelled(run, emit);
                }
                run.transition(AgentState::Completed)
                    .map_err(AgentExecutionError::Transition)?;
                emit(AgentEvent::Completed { run_id: run.id() });
                return Ok(AgentExecutionResult {
                    content: continuation.content,
                    thinking: continuation.thinking,
                    messages: continuation.request.messages,
                });
            }

            run.transition(AgentState::WaitingForTool)
                .map_err(AgentExecutionError::Transition)?;
            let assistant = axiom_ai_provider::ProviderChatMessage {
                role: axiom_ai_provider::ChatRole::Assistant,
                content: String::new(),
                reasoning: provider_thought_signature
                    .or_else(|| (!provider_reasoning.is_empty()).then_some(provider_reasoning)),
                tool_call_id: None,
                tool_calls: calls.clone(),
            };
            continuation.request.messages.push(assistant);
            continuation.calls = calls;
            continuation.tool_running = false;
            approved_first = false;
        }
    }

    fn process_tool_calls(
        &mut self,
        run: &mut AgentRun,
        mut continuation: ExecutionContinuation,
        mut approved_first: bool,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<ToolProcessing, AgentExecutionError> {
        while !continuation.calls.is_empty() {
            if run.cancellation().is_cancelled() {
                run.cancel();
                emit(AgentEvent::Cancelled { run_id: run.id() });
                return Err(AgentExecutionError::Cancelled);
            }
            let call = continuation.calls.remove(0);
            let approved_call = approved_first;
            approved_first = false;
            if !approved_call {
                if let Err(error) = run.consume_tool_call() {
                    return match self.failed(run, emit, AgentExecutionError::BudgetExceeded(error))
                    {
                        Ok(_) => unreachable!(),
                        Err(error) => Err(error),
                    };
                }
                emit(AgentEvent::ToolRequested { run_id: run.id() });
            }
            let decision = if approved_call
                || self
                    .policy
                    .is_run_scoped_allowed(ToolPolicyContext { run_id: run.id() }, &call)
            {
                ToolPolicyDecision::Allow
            } else {
                self.policy
                    .decide(ToolPolicyContext { run_id: run.id() }, &call)
            };
            if approved_call && call.name == "update_file" {
                tracing::info!(
                    target: "axiom.ai_diag",
                    event = "update_file_approval_resumed",
                    run_id = run.id().value(),
                    call_id_present = call.id.is_some(),
                    "[AI-DIAG]"
                );
            }
            let outcome = match decision {
                ToolPolicyDecision::Allow => {
                    if !continuation.tool_running {
                        run.transition(AgentState::RunningTool)
                            .map_err(AgentExecutionError::Transition)?;
                        continuation.tool_running = true;
                    }
                    emit(AgentEvent::ToolStarted { run_id: run.id() });
                    match self.tools.execute(&call) {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            return match self.failed(run, emit, AgentExecutionError::Tool(error)) {
                                Ok(_) => unreachable!(),
                                Err(error) => Err(error),
                            };
                        }
                    }
                }
                ToolPolicyDecision::Deny { reason } => ToolOutcome::ControlledError(format!(
                    "tool execution denied by policy: {reason}"
                )),
                ToolPolicyDecision::RequireApproval { reason } => {
                    continuation.calls.insert(0, call.clone());
                    let approval = run.issue_approval(call, &reason);
                    run.transition(AgentState::WaitingForApproval)
                        .map_err(AgentExecutionError::Transition)?;
                    emit(AgentEvent::ApprovalRequested {
                        run_id: approval.run_id,
                        approval_id: approval.approval_id,
                        tool_name: approval.tool_name.clone(),
                        arguments: approval.arguments.clone(),
                        reason: approval.reason.clone(),
                    });
                    return Ok(ToolProcessing::Pending {
                        continuation,
                        approval,
                    });
                }
            };
            self.observe_tool_event(&call, matches!(&outcome, ToolOutcome::Success(_)));
            match outcome {
                ToolOutcome::Success(result) => {
                    tracing::info!(
                        target: "axiom.ai_diag",
                        event = "agent_function_response",
                        round = run.usage().provider_turns,
                        name = call.name.as_str(),
                        function_response_id_present = call.id.is_some(),
                        function_response_id_matches = true,
                        relative_path = call.arguments.get("path").and_then(|value| value.as_str()).unwrap_or(""),
                        classification = "native_function_response",
                        "[AI-DIAG]"
                    );
                    continuation
                        .request
                        .messages
                        .push(tool_message(call.id, result));
                    emit(AgentEvent::ToolCompleted {
                        run_id: run.id(),
                        succeeded: true,
                    });
                }
                ToolOutcome::ControlledError(error) => {
                    tracing::warn!(
                        target: "axiom.ai_diag",
                        event = "agent_function_response",
                        round = run.usage().provider_turns,
                        name = call.name.as_str(),
                        function_response_id_present = call.id.is_some(),
                        function_response_id_matches = true,
                        relative_path = call.arguments.get("path").and_then(|value| value.as_str()).unwrap_or(""),
                        classification = "controlled_tool_error",
                        "[AI-DIAG]"
                    );
                    continuation
                        .request
                        .messages
                        .push(tool_message(call.id, error));
                    emit(AgentEvent::ToolCompleted {
                        run_id: run.id(),
                        succeeded: false,
                    });
                }
            }
        }
        run.transition(AgentState::RunningModel)
            .map_err(AgentExecutionError::Transition)?;
        Ok(ToolProcessing::Complete(continuation))
    }

    fn cancelled(
        &mut self,
        run: &mut AgentRun,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        if !run.state().is_terminal() {
            run.cancel();
            emit(AgentEvent::Cancelled { run_id: run.id() });
        }
        Err(AgentExecutionError::Cancelled)
    }

    fn failed(
        &mut self,
        run: &mut AgentRun,
        emit: &mut dyn FnMut(AgentEvent),
        error: AgentExecutionError,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        run.transition(AgentState::Failed)
            .map_err(AgentExecutionError::Transition)?;
        emit(AgentEvent::Failed {
            run_id: run.id(),
            diagnostic: error.diagnostic(),
        });
        Err(error)
    }

    fn fail_provider(
        &mut self,
        run: &mut AgentRun,
        emit: &mut dyn FnMut(AgentEvent),
        error: axiom_ai_provider::ProviderError,
    ) -> AgentExecutionError {
        let _ = run.transition(AgentState::Failed);
        let error = AgentExecutionError::Provider(error);
        emit(AgentEvent::Failed {
            run_id: run.id(),
            diagnostic: error.diagnostic(),
        });
        error
    }
}

fn tool_message(id: Option<String>, content: String) -> axiom_ai_provider::ProviderChatMessage {
    axiom_ai_provider::ProviderChatMessage {
        role: axiom_ai_provider::ChatRole::Tool,
        content,
        reasoning: None,
        tool_call_id: id,
        tool_calls: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axiom_ai_provider::{
        ChatRole, ProviderChatMessage, ProviderChatRequest, ProviderChatStreamEvent, ProviderError,
        ProviderToolCall, ProviderToolDefinition,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };

    static APPROVAL_WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn budget() -> AgentBudget {
        AgentBudget::new(2, 2)
    }

    fn request() -> ProviderChatRequest {
        ProviderChatRequest {
            model: "fake".into(),
            messages: vec![ProviderChatMessage {
                role: ChatRole::User,
                content: "hello".into(),
                reasoning: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            think: None,
            thinking_level: None,
            tools: Some(Vec::new()),
        }
    }

    #[test]
    fn normal_agent_request_receives_one_central_system_instruction() {
        let request = with_agent_system_instruction(request());
        assert_eq!(
            request
                .messages
                .iter()
                .filter(|message| message.content == agent_system_instruction())
                .count(),
            1
        );
        assert_eq!(request.messages[0].role, ChatRole::System);
        assert_eq!(request.messages[1].role, ChatRole::User);
        assert_eq!(request.messages[1].content, "hello");
    }

    #[test]
    fn finalization_keeps_permanent_instruction_and_adds_temporary_context() {
        let request = with_agent_system_instruction(request());
        let projected = finalization_request(&request);
        assert!(projected.tools.is_none());
        assert_eq!(projected.messages[0].role, ChatRole::System);
        assert!(
            projected.messages[0]
                .content
                .contains("Tool results/context")
        );
        assert!(
            projected
                .messages
                .iter()
                .any(|message| message.content == agent_system_instruction())
        );
    }

    fn message(role: ChatRole, content: &str) -> ProviderChatMessage {
        ProviderChatMessage {
            role,
            content: content.into(),
            reasoning: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    fn tool_result(id: &str, content: &str) -> ProviderChatMessage {
        ProviderChatMessage {
            role: ChatRole::Tool,
            content: content.into(),
            reasoning: None,
            tool_call_id: Some(id.into()),
            tool_calls: Vec::new(),
        }
    }

    fn occurrence_count(haystack: &str, needle: &str) -> usize {
        haystack.match_indices(needle).count()
    }

    #[test]
    fn request_metadata_contains_counts_without_message_content() {
        let request = ProviderChatRequest {
            model: "fake".into(),
            messages: vec![
                ProviderChatMessage {
                    role: ChatRole::User,
                    content: "sensitive user content".into(),
                    reasoning: Some("sensitive reasoning".into()),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                ProviderChatMessage {
                    role: ChatRole::System,
                    content: "sensitive system content".into(),
                    reasoning: None,
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
            ],
            think: Some(true),
            thinking_level: None,
            tools: Some(Vec::new()),
        };
        let metadata = RequestMetadata::from_request(&request);
        assert_eq!(metadata.message_count, 2);
        assert_eq!(metadata.user_count, 1);
        assert_eq!(metadata.system_count, 1);
        assert_eq!(metadata.assistant_count, 0);
        assert_eq!(metadata.tool_count, 0);
        assert_eq!(metadata.last_role, "system");
        assert!(metadata.tools_present);
        assert!(metadata.think_enabled);
        assert!(metadata.think_present);
        let debug = format!("{metadata:?}");
        assert!(!debug.contains("sensitive"));
    }

    #[test]
    fn legacy_user_system_finalization_shape_is_replaced_by_user_termination() {
        let request = ProviderChatRequest {
            model: "fake".into(),
            messages: vec![
                message(ChatRole::User, "original request"),
                tool_result("call-1", "tool context"),
            ],
            think: None,
            thinking_level: None,
            tools: Some(Vec::new()),
        };

        let legacy_roles = vec![ChatRole::User, ChatRole::System];

        let projected = finalization_request(&request);
        assert!(projected.tools.is_none());
        let projected_roles = projected
            .messages
            .iter()
            .map(|message| message.role.clone())
            .collect::<Vec<_>>();
        assert_ne!(projected_roles, legacy_roles);
        assert_eq!(
            projected_roles,
            vec![ChatRole::System, ChatRole::User, ChatRole::User]
        );
        assert_eq!(
            projected.messages.last().map(|message| &message.role),
            Some(&ChatRole::User)
        );
    }

    #[test]
    fn tool_only_context_projects_system_then_user() {
        let request = ProviderChatRequest {
            model: "fake".into(),
            messages: vec![tool_result("result-1", "tool context")],
            think: None,
            thinking_level: None,
            tools: Some(Vec::new()),
        };

        let projected = finalization_request(&request);
        assert!(projected.tools.is_none());
        assert_eq!(
            projected
                .messages
                .iter()
                .map(|message| message.role.clone())
                .collect::<Vec<_>>(),
            vec![ChatRole::System, ChatRole::User]
        );
        assert!(projected.messages[0].content.contains("tool context"));
    }

    #[test]
    fn original_context_and_tool_results_project_user_system_user() {
        let mut assistant = message(ChatRole::Assistant, "");
        assistant.tool_calls = vec![ProviderToolCall {
            id: Some("call-1".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({}),
        }];
        let request = ProviderChatRequest {
            model: "fake".into(),
            messages: vec![
                message(ChatRole::User, "original request"),
                assistant,
                tool_result("call-1", "success marker"),
                tool_result("call-2", "controlled error marker"),
            ],
            think: None,
            thinking_level: None,
            tools: Some(Vec::new()),
        };

        let projected = finalization_request(&request);
        assert!(projected.tools.is_none());
        assert_eq!(
            projected
                .messages
                .iter()
                .map(|message| message.role.clone())
                .collect::<Vec<_>>(),
            vec![ChatRole::System, ChatRole::User, ChatRole::User]
        );
        assert_eq!(projected.messages[1].content, "original request");
        assert_eq!(
            occurrence_count(&projected.messages[0].content, "success marker"),
            1
        );
        assert_eq!(
            occurrence_count(&projected.messages[0].content, "controlled error marker"),
            1
        );
        assert!(
            projected
                .messages
                .iter()
                .all(|message| message.reasoning.is_none() && message.tool_calls.is_empty())
        );
    }

    #[test]
    fn finalization_leads_with_system_then_user_user() {
        let request = ProviderChatRequest {
            model: "fake".into(),
            messages: vec![
                message(ChatRole::User, "original request"),
                tool_result("call-1", "tool context"),
            ],
            think: None,
            thinking_level: None,
            tools: Some(Vec::new()),
        };
        let projected = finalization_request(&request);
        assert!(projected.tools.is_none());
        assert_eq!(
            projected
                .messages
                .iter()
                .map(|message| message.role.clone())
                .collect::<Vec<_>>(),
            vec![ChatRole::System, ChatRole::User, ChatRole::User]
        );
        assert_eq!(projected.messages[0].role, ChatRole::System);
        assert_eq!(projected.messages[1].role, ChatRole::User);
        assert_eq!(projected.messages[1].content, "original request");
        assert_eq!(projected.messages[2].role, ChatRole::User);
    }

    struct FakeProvider {
        scripts: Vec<Result<Vec<ProviderChatStreamEvent>, ProviderError>>,
        calls: usize,
        requests: Vec<ProviderChatRequest>,
    }

    impl ProviderExecutor for FakeProvider {
        fn execute(
            &mut self,
            request: &ProviderChatRequest,
            emit: &mut dyn FnMut(ProviderChatStreamEvent),
        ) -> Result<(), ProviderError> {
            self.requests.push(request.clone());
            let script = self.scripts.get(self.calls).cloned().unwrap_or_else(|| {
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ])
            });
            self.calls += 1;
            for event in script? {
                emit(event);
            }
            Ok(())
        }
    }

    struct FakeTools {
        outcomes: Vec<Result<ToolOutcome, ToolInfrastructureError>>,
        calls: Vec<String>,
    }

    struct ScriptedPolicy {
        decisions: Vec<ToolPolicyDecision>,
        calls: Arc<Mutex<Vec<(AgentRunId, String)>>>,
    }

    impl ToolPolicy for ScriptedPolicy {
        fn decide(
            &mut self,
            context: ToolPolicyContext,
            call: &ProviderToolCall,
        ) -> ToolPolicyDecision {
            self.calls
                .lock()
                .unwrap()
                .push((context.run_id, call.name.clone()));
            self.decisions
                .get(self.calls.lock().unwrap().len() - 1)
                .cloned()
                .unwrap_or(ToolPolicyDecision::Deny {
                    reason: "no scripted decision".into(),
                })
        }
    }

    impl ToolExecutor for FakeTools {
        fn execute(
            &mut self,
            call: &ProviderToolCall,
        ) -> Result<ToolOutcome, ToolInfrastructureError> {
            self.calls.push(call.name.clone());
            self.outcomes
                .get(self.calls.len() - 1)
                .cloned()
                .unwrap_or_else(|| Ok(ToolOutcome::Success("tool-result".into())))
        }
    }

    #[derive(Clone, Default)]
    struct MemoryLog {
        events: Arc<Mutex<Vec<MemoryObservation>>>,
        sessions_started: Arc<Mutex<Vec<(MemorySessionId, MemoryScope)>>>,
        sessions_ended: Arc<Mutex<Vec<MemorySessionId>>>,
        briefings: Arc<Mutex<usize>>,
    }

    struct RecordingMemory {
        log: MemoryLog,
        next: u64,
        briefing_results: Vec<MemoryResult>,
    }

    impl RecordingMemory {
        fn new(log: MemoryLog) -> Self {
            Self {
                log,
                next: 0,
                briefing_results: Vec::new(),
            }
        }

        fn with_briefing(log: MemoryLog, briefing_results: Vec<MemoryResult>) -> Self {
            Self {
                log,
                next: 0,
                briefing_results,
            }
        }
    }

    impl MemoryService for RecordingMemory {
        fn session_start(&mut self, scope: MemoryScope) -> MemorySessionId {
            self.next += 1;
            let session = MemorySessionId::new(self.next);
            self.log
                .sessions_started
                .lock()
                .unwrap()
                .push((session, scope));
            session
        }

        fn observe(&mut self, _session: MemorySessionId, observation: MemoryObservation) {
            self.log.events.lock().unwrap().push(observation);
        }

        fn session_end(&mut self, session: MemorySessionId) {
            self.log.sessions_ended.lock().unwrap().push(session);
        }

        fn query(&self, _: MemoryScope, _: &str, _: usize) -> Vec<MemoryResult> {
            Vec::new()
        }

        fn recent(&self, _: MemoryScope, _: usize) -> Vec<MemoryResult> {
            Vec::new()
        }

        fn history(&self, _: MemoryScope, _: usize) -> Vec<MemoryResult> {
            Vec::new()
        }

        fn briefing(&self, _: MemoryScope, _: usize) -> Vec<MemoryResult> {
            *self.log.briefings.lock().unwrap() += 1;
            self.briefing_results.clone()
        }

        fn handoff(&self, _: MemorySessionId) -> Option<MemoryHandoff> {
            None
        }
    }

    struct CancellingProvider {
        cancellation: Cancellation,
        calls: usize,
    }

    impl ProviderExecutor for CancellingProvider {
        fn execute(
            &mut self,
            _: &ProviderChatRequest,
            emit: &mut dyn FnMut(ProviderChatStreamEvent),
        ) -> Result<(), ProviderError> {
            self.calls += 1;
            emit(tool_call("cancel", "read_file"));
            emit(ProviderChatStreamEvent::Done);
            self.cancellation.cancel();
            Ok(())
        }
    }

    struct CancellingTool {
        cancellation: Cancellation,
        calls: usize,
    }

    impl ToolExecutor for CancellingTool {
        fn execute(
            &mut self,
            _: &ProviderToolCall,
        ) -> Result<ToolOutcome, ToolInfrastructureError> {
            self.calls += 1;
            self.cancellation.cancel();
            Ok(ToolOutcome::Success("cancelled after tool".into()))
        }
    }

    fn tool_call(id: &str, name: &str) -> ProviderChatStreamEvent {
        ProviderChatStreamEvent::ToolCall(ProviderToolCall {
            id: Some(id.into()),
            name: name.into(),
            arguments: serde_json::json!({}),
        })
    }

    fn update_call(id: &str, path: &str) -> ProviderChatStreamEvent {
        ProviderChatStreamEvent::ToolCall(ProviderToolCall {
            id: Some(id.into()),
            name: "update_file".into(),
            arguments: serde_json::json!({
                "path": path,
                "expected_fingerprint": "sha256:old",
                "content": "new",
            }),
        })
    }

    fn approval_workspace() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "axiom-agent-approval-{}-{}",
            std::process::id(),
            APPROVAL_WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("jogo.html"), "old").unwrap();
        std::fs::write(root.join("config.php"), "old").unwrap();
        root
    }

    #[test]
    fn one_agent_run_starts_and_ends_one_memory_session() {
        let log = MemoryLog::default();
        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![
                ProviderChatStreamEvent::ContentDelta("done".into()),
                ProviderChatStreamEvent::Done,
            ])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: Vec::new(),
            calls: Vec::new(),
        };
        let mut executor = AgentExecutor::with_policy_and_memory_in_scope(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::default(),
            Box::new(RecordingMemory::new(log.clone())),
            MemoryScope::Project,
        );
        let mut run = AgentRun::new(AgentRunId::new(100), budget());
        let _ = executor.execute(&mut run, request(), &mut |_| {}).unwrap();
        assert_eq!(log.sessions_started.lock().unwrap().len(), 1);
        assert_eq!(
            log.sessions_started.lock().unwrap()[0].1,
            MemoryScope::Project
        );
        assert_eq!(log.sessions_ended.lock().unwrap().len(), 1);
        assert!(log.events.lock().unwrap().iter().any(|event| matches!(
            event,
            MemoryObservation::TaskOutcome(value) if value == "completed successfully"
        )));
    }

    #[test]
    fn briefing_is_bounded_and_nomemory_adds_no_context_message() {
        let log = MemoryLog::default();
        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![ProviderChatStreamEvent::Done])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: Vec::new(),
            calls: Vec::new(),
        };
        let mut executor = AgentExecutor::with_policy_and_memory_in_scope(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::default(),
            Box::new(RecordingMemory::with_briefing(
                log,
                (0..20)
                    .map(|index| MemoryResult {
                        id: Some(index.to_string()),
                        scope: MemoryScope::Workspace,
                        category: None,
                        content: "x".repeat(1000),
                    })
                    .collect(),
            )),
            MemoryScope::Workspace,
        );
        let mut run = AgentRun::new(AgentRunId::new(104), budget());
        executor.execute(&mut run, request(), &mut |_| {}).unwrap();
        let briefing = provider.requests[0]
            .messages
            .iter()
            .find(|message| message.content.contains("Memory briefing"))
            .unwrap();
        assert!(briefing.content.chars().count() <= MAX_MEMORY_BRIEFING_CHARS);
        assert!(briefing.content.matches("- ").count() <= MAX_MEMORY_BRIEFING_ITEMS);

        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![ProviderChatStreamEvent::Done])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut executor = AgentExecutor::new(&mut provider, &mut tools);
        let mut run = AgentRun::new(AgentRunId::new(105), budget());
        executor.execute(&mut run, request(), &mut |_| {}).unwrap();
        assert!(
            !provider.requests[0]
                .messages
                .iter()
                .any(|message| message.content.contains("Memory briefing"))
        );
    }

    #[test]
    fn multiple_provider_turns_keep_one_session_and_capture_semantic_events() {
        let log = MemoryLog::default();
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("read", "list_directory"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("raw listing with secrets".into()))],
            calls: Vec::new(),
        };
        let mut executor = AgentExecutor::with_policy_and_memory(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::default(),
            Box::new(RecordingMemory::with_briefing(
                log.clone(),
                vec![
                    MemoryResult {
                        id: Some("project-fact".into()),
                        scope: MemoryScope::Workspace,
                        category: None,
                        content: "historical workspace constraint".into(),
                    },
                    MemoryResult {
                        id: Some("other-scope".into()),
                        scope: MemoryScope::Project,
                        category: None,
                        content: "must not leak".into(),
                    },
                ],
            )),
        );
        let mut run = AgentRun::new(AgentRunId::new(101), AgentBudget::new(3, 2));
        executor.execute(&mut run, request(), &mut |_| {}).unwrap();
        assert_eq!(provider.calls, 2);
        assert_eq!(*log.briefings.lock().unwrap(), 1);
        assert_eq!(log.sessions_started.lock().unwrap().len(), 1);
        for request in &provider.requests {
            let briefings = request
                .messages
                .iter()
                .filter(|message| message.content.contains("Memory briefing"))
                .collect::<Vec<_>>();
            assert_eq!(briefings.len(), 1);
            assert!(
                briefings[0]
                    .content
                    .contains("historical workspace constraint")
            );
            assert!(!briefings[0].content.contains("must not leak"));
            assert_eq!(briefings[0].role, ChatRole::System);
        }
        assert!(log.events.lock().unwrap().iter().any(|event| matches!(
            event,
            MemoryObservation::Discovery(value) if value == "list_directory completed"
        )));
        assert!(!log.events.lock().unwrap().iter().any(|event| matches!(
            event,
            MemoryObservation::Discovery(value) if value.contains("raw listing")
        )));
    }

    #[test]
    fn finalization_reuses_one_memory_briefing_without_refetching() {
        let log = MemoryLog::default();
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("read", "list_directory"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("final answer".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("listing".into()))],
            calls: Vec::new(),
        };
        let mut executor = AgentExecutor::with_policy_and_memory(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::default(),
            Box::new(RecordingMemory::with_briefing(
                log.clone(),
                vec![MemoryResult {
                    id: Some("historical-fact".into()),
                    scope: MemoryScope::Workspace,
                    category: None,
                    content: "historical fact".into(),
                }],
            )),
        );
        let mut run = AgentRun::new(AgentRunId::new(106), AgentBudget::new(2, 2));
        executor.execute(&mut run, request(), &mut |_| {}).unwrap();

        assert_eq!(*log.briefings.lock().unwrap(), 1);
        assert_eq!(provider.requests.len(), 2);
        for provider_request in &provider.requests {
            assert_eq!(
                provider_request
                    .messages
                    .iter()
                    .filter(|message| message.content.contains("Memory briefing"))
                    .count(),
                1
            );
        }
        assert!(provider.requests[1].tools.is_none());
    }

    #[test]
    fn provider_failure_and_cancellation_end_memory_session_without_failing_memory() {
        let log = MemoryLog::default();
        let mut provider = FakeProvider {
            scripts: vec![Err(ProviderError::Timeout)],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: Vec::new(),
            calls: Vec::new(),
        };
        let mut executor = AgentExecutor::with_policy_and_memory(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::default(),
            Box::new(RecordingMemory::new(log.clone())),
        );
        let mut run = AgentRun::new(AgentRunId::new(102), budget());
        assert!(executor.execute(&mut run, request(), &mut |_| {}).is_err());
        assert_eq!(log.sessions_ended.lock().unwrap().len(), 1);
        assert!(log.events.lock().unwrap().iter().any(|event| matches!(
            event,
            MemoryObservation::TaskOutcome(value) if value == "failed due to provider error"
        )));

        let cancellation = Cancellation::default();
        let mut provider = CancellingProvider {
            cancellation: cancellation.clone(),
            calls: 0,
        };
        let log = MemoryLog::default();
        let mut executor = AgentExecutor::with_policy_and_memory(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::default(),
            Box::new(RecordingMemory::new(log.clone())),
        );
        let mut run = AgentRun::new(AgentRunId::new(103), budget());
        assert!(executor.execute(&mut run, request(), &mut |_| {}).is_err());
        assert_eq!(log.sessions_ended.lock().unwrap().len(), 1);
    }

    #[test]
    fn new_run_starts_idle_with_zero_usage() {
        let run = AgentRun::new(AgentRunId::new(7), budget());
        assert_eq!(run.state(), AgentState::Idle);
        assert_eq!(run.usage(), AgentUsage::default());
    }

    #[test]
    fn happy_path_reaches_completed() {
        let mut run = AgentRun::new(AgentRunId::new(1), budget());
        for state in [
            AgentState::Preparing,
            AgentState::RunningModel,
            AgentState::Finalizing,
            AgentState::Completed,
        ] {
            run.transition(state).unwrap();
        }
        assert_eq!(run.state(), AgentState::Completed);
    }

    #[test]
    fn tool_path_returns_to_model() {
        let mut run = AgentRun::new(AgentRunId::new(1), budget());
        for state in [
            AgentState::Preparing,
            AgentState::RunningModel,
            AgentState::WaitingForTool,
            AgentState::RunningTool,
            AgentState::RunningModel,
        ] {
            run.transition(state).unwrap();
        }
        assert_eq!(run.state(), AgentState::RunningModel);
    }

    #[test]
    fn invalid_transition_is_typed_and_terminal_states_are_terminal() {
        let mut run = AgentRun::new(AgentRunId::new(1), budget());
        assert_eq!(
            run.transition(AgentState::Completed),
            Err(InvalidTransition {
                from: AgentState::Idle,
                to: AgentState::Completed
            })
        );
        run.transition(AgentState::Preparing).unwrap();
        run.transition(AgentState::Failed).unwrap();
        assert!(run.state().is_terminal());
        assert!(run.transition(AgentState::RunningModel).is_err());
        assert!(!run.cancel());
    }

    #[test]
    fn cancellation_is_idempotent_and_cannot_reactivate() {
        let mut run = AgentRun::new(AgentRunId::new(2), budget());
        assert!(run.cancel());
        assert!(!run.cancel());
        assert_eq!(run.state(), AgentState::Cancelled);
        assert!(run.cancellation().is_cancelled());
        assert!(run.transition(AgentState::Preparing).is_err());
    }

    #[test]
    fn budget_allows_exact_limit_and_rejects_exhaustion() {
        let mut run = AgentRun::new(AgentRunId::new(3), budget());
        assert!(run.consume_provider_turn().is_ok());
        assert!(run.consume_provider_turn().is_ok());
        assert_eq!(
            run.consume_provider_turn(),
            Err(BudgetExceeded {
                resource: BudgetResource::ProviderTurns,
                limit: 2,
                attempted: 3
            })
        );
        assert_eq!(run.usage().provider_turns, 2);
        assert!(run.consume_tool_call().is_ok());
        assert!(run.consume_tool_call().is_ok());
        assert!(run.consume_tool_call().is_err());
        assert_eq!(run.usage().tool_calls, 2);
    }

    #[test]
    fn cancellation_primitive_is_shared_and_one_way() {
        let first = Cancellation::default();
        let second = first.clone();
        assert!(!first.is_cancelled());
        assert!(second.cancel());
        assert!(!first.cancel());
        assert!(first.is_cancelled());
        assert!(second.is_cancelled());
    }

    #[test]
    fn ids_are_deterministic_distinct_and_events_are_attributable() {
        let first = AgentRunId::new(10);
        let second = AgentRunId::new(11);
        assert_ne!(first, second);
        assert_eq!(AgentRunId::new(10), first);
        let event = AgentEvent::ContentDelta {
            run_id: first,
            delta: "x".into(),
        };
        assert_eq!(event.run_id(), first);
        assert!(is_stale_event(&event, second));
        assert!(!is_stale_event(&event, first));
    }

    #[test]
    fn simple_completion_emits_content_and_completes() {
        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![
                ProviderChatStreamEvent::ContentDelta("hello".into()),
                ProviderChatStreamEvent::Done,
            ])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: Vec::new(),
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(20), AgentBudget::new(1, 1));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |event| events.push(event))
            .unwrap();
        assert_eq!(result.content, "hello");
        assert_eq!(provider.calls, 1);
        assert_eq!(run.usage().provider_turns, 1);
        assert_eq!(run.state(), AgentState::Completed);
        assert!(matches!(events.last(), Some(AgentEvent::Completed { .. })));
    }

    #[test]
    fn one_tool_round_preserves_provider_local_messages() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    ProviderChatStreamEvent::ResponseMetadata(
                        axiom_ai_provider::ProviderResponseMetadata::GoogleThoughtSignature(
                            "opaque-signature".into(),
                        ),
                    ),
                    tool_call("call-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("contents".into()))],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(21), AgentBudget::new(2, 1));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |event| events.push(event))
            .unwrap();
        assert_eq!(result.content, "done");
        assert_eq!(provider.calls, 2);
        assert_eq!(tools.calls, vec!["read_file"]);
        assert_eq!(result.messages().len(), 4);
        assert_eq!(result.messages()[2].role, ChatRole::Assistant);
        assert_eq!(
            result.messages()[2].reasoning.as_deref(),
            Some("opaque-signature")
        );
        assert_eq!(result.messages()[3].role, ChatRole::Tool);
        assert_eq!(result.messages()[3].content, "contents");
        assert!(matches!(events[2], AgentEvent::ToolRequested { .. }));
        assert!(matches!(events[3], AgentEvent::ToolStarted { .. }));
        assert!(matches!(
            events[4],
            AgentEvent::ToolCompleted {
                succeeded: true,
                ..
            }
        ));
    }

    #[test]
    fn reasoning_replay_is_preserved_separately_from_visible_thinking() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    ProviderChatStreamEvent::ThinkingDelta("visible-a".into()),
                    ProviderChatStreamEvent::ReasoningDelta("reasoning-a".into()),
                    tool_call("call-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ThinkingDelta("visible-b".into()),
                    ProviderChatStreamEvent::ReasoningDelta("reasoning-b".into()),
                    tool_call("call-2", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("contents-a".into())),
                Ok(ToolOutcome::Success("contents-b".into())),
            ],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(45), AgentBudget::new(3, 2));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |event| events.push(event))
            .unwrap();

        assert_eq!(result.thinking, "visible-avisible-b");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::ThinkingDelta { .. }))
                .count(),
            2
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentEvent::ThinkingDelta { delta, .. }
                if delta.contains("reasoning")
        )));
        assert_eq!(
            provider.requests[1].messages[2].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            provider.requests[1].messages[2].tool_calls[0].id.as_deref(),
            Some("call-1")
        );
        assert_eq!(
            result.messages()[2].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            result.messages()[4].reasoning.as_deref(),
            Some("reasoning-b")
        );
        assert_eq!(result.messages()[2].content, "");
        assert!(provider.requests[2].tools.is_none());
        assert!(
            provider.requests[2]
                .messages
                .iter()
                .all(|message| !matches!(message.role, ChatRole::Assistant | ChatRole::Tool))
        );
        assert!(
            provider.requests[2]
                .messages
                .iter()
                .all(|message| message.reasoning.is_none() && message.tool_calls.is_empty())
        );
    }

    #[test]
    fn multiple_tool_rounds_preserve_reasoning_by_turn() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    ProviderChatStreamEvent::ThinkingDelta("visible-a".into()),
                    ProviderChatStreamEvent::ReasoningDelta("reasoning-a".into()),
                    tool_call("call-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ThinkingDelta("visible-b".into()),
                    ProviderChatStreamEvent::ReasoningDelta("reasoning-b".into()),
                    tool_call("call-2", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ReasoningDelta("reasoning-c".into()),
                    tool_call("call-3", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("contents-a".into())),
                Ok(ToolOutcome::Success("contents-b".into())),
                Ok(ToolOutcome::Success("contents-c".into())),
            ],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(46), AgentBudget::new(4, 3));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |event| events.push(event))
            .unwrap();

        assert_eq!(result.thinking, "visible-avisible-b");
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentEvent::ThinkingDelta { delta, .. }
                if delta.contains("reasoning")
        )));
        assert_eq!(
            provider.requests[1].messages[2].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            provider.requests[2].messages[2].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            provider.requests[2].messages[4].reasoning.as_deref(),
            Some("reasoning-b")
        );
        assert_eq!(
            result
                .messages()
                .iter()
                .filter_map(|message| message.reasoning.as_deref())
                .collect::<Vec<_>>(),
            vec!["reasoning-a", "reasoning-b", "reasoning-c"]
        );
        assert_eq!(tools.calls, vec!["read_file", "read_file", "read_file"]);
        assert!(provider.requests[3].tools.is_none());
        assert!(
            provider.requests[3]
                .messages
                .iter()
                .all(|message| !matches!(message.role, ChatRole::Assistant | ChatRole::Tool))
        );
        assert!(
            provider.requests[3]
                .messages
                .iter()
                .all(|message| message.reasoning.is_none() && message.tool_calls.is_empty())
        );
    }

    #[test]
    fn provider_local_reasoning_is_isolated_between_runs() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    ProviderChatStreamEvent::ReasoningDelta("run-a reasoning".into()),
                    tool_call("run-a-call", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("run-a done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ReasoningDelta("run-b reasoning".into()),
                    tool_call("run-b-call", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("run-b done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("run-a result".into())),
                Ok(ToolOutcome::Success("run-b result".into())),
            ],
            calls: Vec::new(),
        };
        let mut run_a = AgentRun::new(AgentRunId::new(47), AgentBudget::new(2, 1));
        let mut run_b = AgentRun::new(AgentRunId::new(48), AgentBudget::new(2, 1));
        let mut events_a = Vec::new();
        let mut events_b = Vec::new();
        let mut executor = AgentExecutor::new(&mut provider, &mut tools);
        let result_a = executor
            .execute(&mut run_a, request(), &mut |event| events_a.push(event))
            .unwrap();
        let result_b = executor
            .execute(&mut run_b, request(), &mut |event| events_b.push(event))
            .unwrap();

        assert_eq!(
            result_a.messages()[2].reasoning.as_deref(),
            Some("run-a reasoning")
        );
        assert_eq!(
            result_b.messages()[2].reasoning.as_deref(),
            Some("run-b reasoning")
        );
        assert!(
            !result_b
                .messages()
                .iter()
                .any(|message| message.reasoning.as_deref() == Some("run-a reasoning"))
        );
        assert!(
            provider.requests[2..]
                .iter()
                .flat_map(|request| request.messages.iter())
                .all(|message| message.reasoning.as_deref() != Some("run-a reasoning"))
        );
        assert!(events_a.iter().all(|event| event.run_id() == run_a.id()));
        assert!(events_b.iter().all(|event| event.run_id() == run_b.id()));
        assert!(is_stale_event(&events_a[0], run_b.id()));
        assert!(!is_stale_event(&events_b[0], run_b.id()));
        assert!(!run_a.cancel());
        assert_eq!(run_b.state(), AgentState::Completed);
    }

    #[test]
    fn multiple_tools_are_sequential_and_ordered() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("a", "read_file"),
                    tool_call("b", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![ProviderChatStreamEvent::Done]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("A".into())),
                Ok(ToolOutcome::Success("B".into())),
            ],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(22), AgentBudget::new(2, 2));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |event| events.push(event))
            .unwrap();
        assert_eq!(tools.calls, vec!["read_file", "read_file"]);
        assert_eq!(result.messages()[3].content, "A");
        assert_eq!(result.messages()[4].content, "B");
        assert_eq!(run.usage().tool_calls, 2);
        assert!(matches!(events[2], AgentEvent::ToolRequested { .. }));
        assert!(matches!(events[7], AgentEvent::ToolCompleted { .. }));
    }

    #[test]
    fn policy_allow_is_consulted_and_executor_runs_once() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("allow-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("file".into()))],
            calls: Vec::new(),
        };
        let policy_calls = Arc::new(Mutex::new(Vec::new()));
        let policy = ScriptedPolicy {
            decisions: vec![ToolPolicyDecision::Allow],
            calls: policy_calls.clone(),
        };
        let mut run = AgentRun::new(AgentRunId::new(34), AgentBudget::new(2, 1));
        let result = AgentExecutor::with_policy(&mut provider, &mut tools, policy).execute(
            &mut run,
            request(),
            &mut |_| {},
        );

        assert_eq!(result.unwrap().content, "done");
        assert_eq!(tools.calls, vec!["read_file"]);
        assert_eq!(
            policy_calls.lock().unwrap().as_slice(),
            &[(AgentRunId::new(34), "read_file".into())]
        );
    }

    #[test]
    fn policy_deny_returns_controlled_result_without_execution_or_budget_failure() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("deny-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("continued".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("must not run".into()))],
            calls: Vec::new(),
        };
        let policy_calls = Arc::new(Mutex::new(Vec::new()));
        let policy = ScriptedPolicy {
            decisions: vec![ToolPolicyDecision::Deny {
                reason: "read-only scope".into(),
            }],
            calls: policy_calls.clone(),
        };
        let mut run = AgentRun::new(AgentRunId::new(35), AgentBudget::new(2, 1));
        let result = AgentExecutor::with_policy(&mut provider, &mut tools, policy)
            .execute(&mut run, request(), &mut |_| {})
            .unwrap();

        assert_eq!(result.content, "continued");
        assert!(tools.calls.is_empty());
        assert_eq!(policy_calls.lock().unwrap().len(), 1);
        assert_eq!(run.usage().tool_calls, 1);
        assert!(result.messages().iter().any(|message| {
            matches!(message.role, ChatRole::Tool) && message.content.contains("denied by policy")
        }));
    }

    #[test]
    fn require_approval_is_fail_safe_without_approval_flow() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("approval-1", "fetch_url"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("approval unavailable".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("must not run".into()))],
            calls: Vec::new(),
        };
        let policy_calls = Arc::new(Mutex::new(Vec::new()));
        let policy = ScriptedPolicy {
            decisions: vec![ToolPolicyDecision::RequireApproval {
                reason: "interactive approval unavailable".into(),
            }],
            calls: policy_calls.clone(),
        };
        let mut run = AgentRun::new(AgentRunId::new(36), AgentBudget::new(2, 1));
        let mut executor = AgentExecutor::with_policy(&mut provider, &mut tools, policy);
        let error = executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();

        assert!(matches!(error, AgentExecutionError::ApprovalRequired(_)));
        assert_eq!(run.state(), AgentState::WaitingForApproval);
        assert!(tools.calls.is_empty());
        assert_eq!(policy_calls.lock().unwrap().len(), 1);
        assert_eq!(provider.calls, 1);
    }

    #[test]
    fn approval_pause_emits_safe_request_and_approve_resumes_once() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    ProviderChatStreamEvent::ReasoningDelta("approval reasoning".into()),
                    tool_call("approval-1", "fetch_url"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ReasoningDelta("continuation reasoning".into()),
                    tool_call("followup-1", "write_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("approved".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("fetched".into())),
                Ok(ToolOutcome::Success("must not run".into())),
            ],
            calls: Vec::new(),
        };
        let policy = ScriptedPolicy {
            decisions: vec![
                ToolPolicyDecision::RequireApproval {
                    reason: "network access".into(),
                },
                ToolPolicyDecision::Deny {
                    reason: "write access".into(),
                },
            ],
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let mut run = AgentRun::new(AgentRunId::new(39), AgentBudget::new(3, 2));
        let mut events = Vec::new();
        let mut executor = AgentExecutor::with_policy(&mut provider, &mut tools, policy);
        let error = executor
            .execute(&mut run, request(), &mut |event| events.push(event))
            .unwrap_err();
        let approval = match error {
            AgentExecutionError::ApprovalRequired(request) => request,
            other => panic!("unexpected error: {other:?}"),
        };

        assert_eq!(run.state(), AgentState::WaitingForApproval);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ApprovalRequested {
                run_id,
                approval_id,
                tool_name,
                arguments,
                ..
            } if *run_id == run.id()
                && *approval_id == approval.approval_id
                && tool_name == "fetch_url"
                && arguments == "{}"
        )));

        let result = executor
            .approve(&mut run, approval.approval_id, &mut |event| {
                events.push(event)
            })
            .unwrap();

        assert_eq!(result.content, "approved");
        assert!(matches!(
            executor.approve(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(ApprovalError::NotPending))
        ));
        assert_eq!(tools.calls, vec!["fetch_url"]);
        assert_eq!(provider.calls, 3);
        assert_eq!(
            provider.requests[1].messages[2].reasoning.as_deref(),
            Some("approval reasoning")
        );
        assert_eq!(
            provider.requests[1].messages[2].tool_calls[0].id.as_deref(),
            Some("approval-1")
        );
        assert_eq!(
            result.messages()[4].reasoning.as_deref(),
            Some("continuation reasoning")
        );
        assert_eq!(run.usage().provider_turns, 3);
        assert_eq!(run.usage().tool_calls, 2);
        assert_eq!(run.state(), AgentState::Completed);
        assert!(provider.requests[2].tools.is_none());
        assert!(
            provider.requests[2]
                .messages
                .iter()
                .all(|message| !matches!(message.role, ChatRole::Assistant | ChatRole::Tool))
        );
        assert!(
            provider.requests[2]
                .messages
                .iter()
                .all(|message| message.reasoning.is_none() && message.tool_calls.is_empty())
        );
        assert!(provider.requests[2].messages.iter().any(|message| {
            matches!(message.role, ChatRole::System)
                && message.content.contains("fetched")
                && message.content.contains("denied by policy")
        }));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::ApprovalRequested { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn approval_deny_never_executes_and_returns_controlled_result() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    ProviderChatStreamEvent::ReasoningDelta("deny reasoning".into()),
                    tool_call("approval-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ReasoningDelta("continuation reasoning".into()),
                    tool_call("followup-1", "write_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("denied".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("must not run".into())),
                Ok(ToolOutcome::Success("must not run".into())),
            ],
            calls: Vec::new(),
        };
        let policy = ScriptedPolicy {
            decisions: vec![
                ToolPolicyDecision::RequireApproval {
                    reason: "needs confirmation".into(),
                },
                ToolPolicyDecision::Deny {
                    reason: "write access".into(),
                },
            ],
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let mut run = AgentRun::new(AgentRunId::new(40), AgentBudget::new(3, 2));
        let mut executor = AgentExecutor::with_policy(&mut provider, &mut tools, policy);
        let approval = match executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err()
        {
            AgentExecutionError::ApprovalRequired(request) => request,
            other => panic!("unexpected error: {other:?}"),
        };
        let result = executor
            .deny(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();

        assert_eq!(result.content, "denied");
        assert!(tools.calls.is_empty());
        assert_eq!(provider.calls, 3);
        assert_eq!(run.usage().provider_turns, 3);
        assert_eq!(run.usage().tool_calls, 2);
        assert!(result.messages().iter().any(|message| {
            matches!(message.role, ChatRole::Tool)
                && message.content.contains("denied by approval decision")
        }));
        assert_eq!(
            provider.requests[1].messages[2].reasoning.as_deref(),
            Some("deny reasoning")
        );
        assert_eq!(
            result.messages()[4].reasoning.as_deref(),
            Some("continuation reasoning")
        );
        assert!(provider.requests[2].tools.is_none());
        assert!(
            provider.requests[2]
                .messages
                .iter()
                .all(|message| !matches!(message.role, ChatRole::Assistant | ChatRole::Tool))
        );
        assert!(
            provider.requests[2]
                .messages
                .iter()
                .all(|message| message.reasoning.is_none() && message.tool_calls.is_empty())
        );
        assert!(provider.requests[2].messages.iter().any(|message| {
            matches!(message.role, ChatRole::System)
                && message.content.contains("denied by approval decision")
                && message.content.contains("denied by policy")
        }));
    }

    #[test]
    fn approval_identity_is_single_use_and_rejects_wrong_ids_and_runs() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("approval-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![ProviderChatStreamEvent::Done]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("read".into()))],
            calls: Vec::new(),
        };
        let policy = ScriptedPolicy {
            decisions: vec![ToolPolicyDecision::RequireApproval {
                reason: "confirm".into(),
            }],
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let mut run = AgentRun::new(AgentRunId::new(41), AgentBudget::new(2, 1));
        let mut other_run = AgentRun::new(AgentRunId::new(42), AgentBudget::new(2, 1));
        let mut executor = AgentExecutor::with_policy(&mut provider, &mut tools, policy);
        let approval = match executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err()
        {
            AgentExecutionError::ApprovalRequired(request) => request,
            other => panic!("unexpected error: {other:?}"),
        };

        assert!(matches!(
            executor.approve(&mut other_run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(ApprovalError::NotPending))
        ));
        assert!(matches!(
            executor.approve(&mut run, ApprovalId(999), &mut |_| {}),
            Err(AgentExecutionError::Approval(
                ApprovalError::UnknownApproval
            ))
        ));
        executor
            .deny(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();
        assert!(matches!(
            executor.deny(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(ApprovalError::NotPending))
        ));
        assert!(matches!(
            executor.approve(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(ApprovalError::NotPending))
        ));
        assert!(tools.calls.is_empty());
    }

    #[test]
    fn cancel_while_waiting_for_approval_prevents_later_execution() {
        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![
                tool_call("approval-1", "read_file"),
                ProviderChatStreamEvent::Done,
            ])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("must not run".into()))],
            calls: Vec::new(),
        };
        let policy = ScriptedPolicy {
            decisions: vec![ToolPolicyDecision::RequireApproval {
                reason: "confirm".into(),
            }],
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let mut run = AgentRun::new(AgentRunId::new(43), AgentBudget::new(2, 1));
        let mut executor = AgentExecutor::with_policy(&mut provider, &mut tools, policy);
        let approval = match executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err()
        {
            AgentExecutionError::ApprovalRequired(request) => request,
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(run.cancel());
        assert!(matches!(
            executor.approve(&mut run, approval.approval_id, &mut |_| {}),
            Err(AgentExecutionError::Approval(ApprovalError::NotPending))
        ));
        assert!(tools.calls.is_empty());
        assert_eq!(run.state(), AgentState::Cancelled);
    }

    #[test]
    fn multiple_tool_calls_pause_in_order_and_resume_without_repeating_provider() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("allow-1", "read_file"),
                    tool_call("approval-1", "fetch_url"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("finished".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("read".into())),
                Ok(ToolOutcome::Success("fetch".into())),
            ],
            calls: Vec::new(),
        };
        let policy = ScriptedPolicy {
            decisions: vec![
                ToolPolicyDecision::Allow,
                ToolPolicyDecision::RequireApproval {
                    reason: "network".into(),
                },
            ],
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let mut run = AgentRun::new(AgentRunId::new(44), AgentBudget::new(2, 2));
        let mut executor = AgentExecutor::with_policy(&mut provider, &mut tools, policy);
        let approval = match executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err()
        {
            AgentExecutionError::ApprovalRequired(request) => request,
            other => panic!("unexpected error: {other:?}"),
        };
        let result = executor
            .approve(&mut run, approval.approval_id, &mut |_| {})
            .unwrap();

        assert_eq!(result.content, "finished");
        assert_eq!(tools.calls, vec!["read_file", "fetch_url"]);
        assert_eq!(provider.calls, 2);
        assert_eq!(run.usage().tool_calls, 2);
    }

    #[test]
    fn policy_is_applied_to_each_call_in_one_provider_turn() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("unknown-1", "shell"),
                    tool_call("unknown-2", "write_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("safe".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("must not run".into())),
                Ok(ToolOutcome::Success("must not run".into())),
            ],
            calls: Vec::new(),
        };
        let policy_calls = Arc::new(Mutex::new(Vec::new()));
        let policy = ScriptedPolicy {
            decisions: vec![
                ToolPolicyDecision::Deny {
                    reason: "unknown".into(),
                },
                ToolPolicyDecision::Deny {
                    reason: "unknown".into(),
                },
            ],
            calls: policy_calls.clone(),
        };
        let mut run = AgentRun::new(AgentRunId::new(37), AgentBudget::new(2, 2));
        let result = AgentExecutor::with_policy(&mut provider, &mut tools, policy)
            .execute(&mut run, request(), &mut |_| {})
            .unwrap();

        assert_eq!(result.content, "safe");
        assert!(tools.calls.is_empty());
        assert_eq!(policy_calls.lock().unwrap().len(), 2);
        assert_eq!(run.usage().tool_calls, 2);
    }

    #[test]
    fn default_policy_denies_unknown_tool_fail_closed() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("unknown-1", "shell"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("safe".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("must not run".into()))],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(38), AgentBudget::new(2, 1));
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |_| {})
            .unwrap();

        assert_eq!(result.content, "safe");
        assert!(tools.calls.is_empty());
        assert!(result.messages().iter().any(|message| {
            matches!(message.role, ChatRole::Tool)
                && message.content.contains("tool 'shell' is not allowed")
        }));
    }

    #[test]
    fn production_policy_allows_reads_and_requires_mutation_approval() {
        let mut policy = ProductionToolPolicy::default();
        let call = |name: &str| ProviderToolCall {
            id: Some("call".into()),
            name: name.into(),
            arguments: serde_json::json!({}),
        };

        for name in ["read_file", "list_directory", "find_symbol", "fetch_url"] {
            assert_eq!(
                policy.decide(
                    ToolPolicyContext {
                        run_id: AgentRunId::new(1)
                    },
                    &call(name)
                ),
                ToolPolicyDecision::Allow
            );
        }
        assert!(matches!(
            policy.decide(
                ToolPolicyContext {
                    run_id: AgentRunId::new(1)
                },
                &call("write_file")
            ),
            ToolPolicyDecision::RequireApproval { .. }
        ));
        assert!(matches!(
            policy.decide(
                ToolPolicyContext {
                    run_id: AgentRunId::new(1)
                },
                &call("update_file")
            ),
            ToolPolicyDecision::RequireApproval { .. }
        ));
        assert!(matches!(
            policy.decide(
                ToolPolicyContext {
                    run_id: AgentRunId::new(1)
                },
                &call("delete_file")
            ),
            ToolPolicyDecision::RequireApproval { .. }
        ));
        assert!(matches!(
            policy.decide(
                ToolPolicyContext {
                    run_id: AgentRunId::new(1)
                },
                &call("shell")
            ),
            ToolPolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn update_file_approval_preview_excludes_content() {
        let call = ProviderToolCall {
            id: Some("update-1".into()),
            name: "update_file".into(),
            arguments: serde_json::json!({
                "path": "src/file.txt",
                "expected_fingerprint": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "content": "sensitive replacement content"
            }),
        };
        let request = ApprovalRequest::from_call(
            AgentRunId::new(1),
            ApprovalId::new(1),
            &call,
            "file mutation requires approval",
        );

        assert!(!request.arguments.contains("sensitive replacement content"));
        let preview: serde_json::Value = serde_json::from_str(&request.arguments).unwrap();
        assert_eq!(preview["tool"], "update_file");
        assert_eq!(preview["path"], "src/file.txt");
        assert_eq!(preview["content_bytes"], 29);
        assert_eq!(
            preview["expected_fingerprint"],
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn delete_file_approval_preview_contains_only_safe_identity_fields() {
        let call = ProviderToolCall {
            id: Some("delete-1".into()),
            name: "delete_file".into(),
            arguments: serde_json::json!({
                "path": "src/file.txt",
                "expected_fingerprint": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            }),
        };
        let request = ApprovalRequest::from_call(
            AgentRunId::new(1),
            ApprovalId::new(1),
            &call,
            "file mutation requires approval",
        );
        let preview: serde_json::Value = serde_json::from_str(&request.arguments).unwrap();
        assert_eq!(preview["tool"], "delete_file");
        assert_eq!(preview["path"], "src/file.txt");
        assert_eq!(
            preview["expected_fingerprint"],
            call.arguments["expected_fingerprint"]
        );
        assert!(preview.get("content").is_none());
    }

    #[test]
    fn read_workflow_can_use_more_than_four_tool_rounds_before_finalization() {
        let mut scripts = (0..6)
            .map(|index| {
                Ok(vec![
                    tool_call(&format!("read-{index}"), "read_file"),
                    ProviderChatStreamEvent::Done,
                ])
            })
            .collect::<Vec<_>>();
        scripts.push(Ok(vec![
            ProviderChatStreamEvent::ContentDelta("final answer".into()),
            ProviderChatStreamEvent::Done,
        ]));
        let mut provider = FakeProvider {
            scripts,
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: (0..6)
                .map(|index| Ok(ToolOutcome::Success(format!("file-{index}"))))
                .collect(),
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(51), AgentBudget::new(7, 16));

        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |_| {})
            .unwrap();

        assert_eq!(result.content, "final answer");
        assert_eq!(provider.calls, 7);
        assert_eq!(run.usage().tool_calls, 6);
        assert!(
            provider.requests[..6]
                .iter()
                .all(|request| request.tools.is_some())
        );
        assert!(provider.requests[6].tools.is_none());
    }

    #[test]
    fn endless_tool_loop_stays_bounded_and_finalization_cannot_reenable_tools() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("call-1", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    tool_call("call-2", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    tool_call("call-3", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    tool_call("call-final", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("one".into())),
                Ok(ToolOutcome::Success("two".into())),
                Ok(ToolOutcome::Success("three".into())),
            ],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(52), AgentBudget::new(4, 16));

        let result =
            AgentExecutor::new(&mut provider, &mut tools).execute(&mut run, request(), &mut |_| {});

        assert!(matches!(
            result,
            Err(AgentExecutionError::ProviderProtocol(
                "provider emitted a tool call while tools were disabled"
            ))
        ));
        assert_eq!(provider.calls, 4);
        assert_eq!(tools.calls, vec!["read_file", "read_file", "read_file"]);
        assert!(provider.requests[3].tools.is_none());
    }

    #[test]
    fn reserves_last_provider_turn_for_finalization() {
        let mut scripts = Vec::new();
        for index in 0..4 {
            scripts.push(Ok(vec![
                tool_call(&format!("call-{index}"), "read_file"),
                ProviderChatStreamEvent::Done,
            ]));
        }
        scripts.push(Ok(vec![
            ProviderChatStreamEvent::ContentDelta("final answer".into()),
            ProviderChatStreamEvent::Done,
        ]));
        let mut provider = FakeProvider {
            scripts,
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("one".into())),
                Ok(ToolOutcome::Success("two".into())),
                Ok(ToolOutcome::Success("three".into())),
                Ok(ToolOutcome::Success("four".into())),
            ],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(31), AgentBudget::new(5, 16));
        let mut request = request();
        request.tools = Some(vec![ProviderToolDefinition {
            name: "read_file".into(),
            description: "read a file".into(),
            parameters: serde_json::json!({"type": "object"}),
        }]);

        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request, &mut |_| {})
            .unwrap();

        assert_eq!(result.content, "final answer");
        assert_eq!(provider.calls, 5);
        assert_eq!(run.usage().provider_turns, 5);
        assert_eq!(run.usage().tool_calls, 4);
        assert!(
            provider.requests[..4]
                .iter()
                .all(|request| request.tools.is_some())
        );
        assert!(provider.requests[4].tools.is_none());
        assert!(
            provider.requests[4]
                .messages
                .iter()
                .all(|message| !matches!(message.role, ChatRole::Assistant | ChatRole::Tool))
        );
        assert_eq!(
            provider.requests[4]
                .messages
                .iter()
                .map(|message| message.role.clone())
                .collect::<Vec<_>>(),
            vec![
                ChatRole::System,
                ChatRole::System,
                ChatRole::User,
                ChatRole::User
            ]
        );
        let context = &provider.requests[4].messages[0].content;
        assert_eq!(occurrence_count(context, "[tool result call-0]"), 1);
        assert_eq!(occurrence_count(context, "[tool result call-3]"), 1);
        assert_eq!(
            provider.requests[4]
                .messages
                .last()
                .map(|message| &message.role),
            Some(&ChatRole::User)
        );
        assert_eq!(run.state(), AgentState::Completed);
    }

    #[test]
    fn finalization_rejects_tool_call_without_executing_it() {
        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![
                tool_call("call-1", "read_file"),
                ProviderChatStreamEvent::Done,
            ])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("must not run".into()))],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(32), AgentBudget::new(1, 16));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools).execute(
            &mut run,
            request(),
            &mut |event| events.push(event),
        );

        assert!(matches!(
            result,
            Err(AgentExecutionError::ProviderProtocol(
                "provider emitted a tool call while tools were disabled"
            ))
        ));
        assert_eq!(provider.calls, 1);
        assert!(provider.requests[0].tools.is_none());
        assert!(tools.calls.is_empty());
        assert_eq!(run.usage().provider_turns, 1);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Failed {
                diagnostic: AgentFailureDiagnostic {
                    kind: AgentFailureKind::Provider,
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn finalization_rejects_textual_tool_protocol_without_emitting_content() {
        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![
                ProviderChatStreamEvent::ContentDelta(
                    "<tool_call>\n<function=>\n</function>\n</tool_call>".into(),
                ),
                ProviderChatStreamEvent::Done,
            ])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("must not run".into()))],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(33), AgentBudget::new(1, 16));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools).execute(
            &mut run,
            request(),
            &mut |event| events.push(event),
        );

        assert!(matches!(
            result,
            Err(AgentExecutionError::ProviderProtocol(
                "provider emitted tool-call markup while tools were disabled"
            ))
        ));
        assert_eq!(provider.calls, 1);
        assert!(provider.requests[0].tools.is_none());
        assert!(tools.calls.is_empty());
        assert_eq!(run.usage().provider_turns, 1);
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentEvent::ContentDelta { delta, .. } if delta.contains("<tool_call")
        )));
    }

    #[test]
    fn provider_and_tool_budgets_are_checked_before_execution() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("a", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![ProviderChatStreamEvent::Done]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("A".into()))],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(23), AgentBudget::new(2, 0));
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools).execute(
            &mut run,
            request(),
            &mut |event| events.push(event),
        );
        assert!(matches!(
            result,
            Err(AgentExecutionError::BudgetExceeded(_))
        ));
        assert_eq!(provider.calls, 1);
        assert!(tools.calls.is_empty());
        assert_eq!(run.state(), AgentState::Failed);
    }

    #[test]
    fn controlled_tool_error_is_sent_back_to_provider() {
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    tool_call("a", "read_file"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("recovered".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::ControlledError("not found".into()))],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(24), AgentBudget::new(2, 1));
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |_| {})
            .unwrap();
        assert_eq!(result.content, "recovered");
        let context = &provider.requests[1].messages[0].content;
        assert_eq!(occurrence_count(context, "not found"), 1);
        assert_eq!(
            provider.requests[1]
                .messages
                .iter()
                .map(|message| message.role.clone())
                .collect::<Vec<_>>(),
            vec![
                ChatRole::System,
                ChatRole::System,
                ChatRole::User,
                ChatRole::User
            ]
        );
        assert_eq!(
            provider.requests[1]
                .messages
                .last()
                .map(|message| &message.role),
            Some(&ChatRole::User)
        );
    }

    #[test]
    fn failed_event_preserves_diagnostic_and_stale_failed_is_rejected() {
        let first = AgentRunId::new(28);
        let second = AgentRunId::new(29);
        let diagnostic = AgentFailureDiagnostic {
            kind: AgentFailureKind::BudgetExceeded,
            message: "provider turns (limit=5, attempted=6)".into(),
        };
        let event = AgentEvent::Failed {
            run_id: first,
            diagnostic: diagnostic.clone(),
        };
        assert_eq!(event.run_id(), first);
        assert_eq!(
            event,
            AgentEvent::Failed {
                run_id: first,
                diagnostic,
            }
        );
        assert!(is_stale_event(&event, second));
        assert!(!is_stale_event(&event, first));
    }

    #[test]
    fn cancellation_emits_cancelled_without_failed_event() {
        let mut provider = FakeProvider {
            scripts: Vec::new(),
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: Vec::new(),
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(30), budget());
        run.cancellation().cancel();
        let mut events = Vec::new();
        let result = AgentExecutor::new(&mut provider, &mut tools).execute(
            &mut run,
            request(),
            &mut |event| events.push(event),
        );
        assert!(matches!(result, Err(AgentExecutionError::Cancelled)));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::Cancelled { .. }))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::Failed { .. }))
        );
    }

    #[test]
    fn provider_failure_and_infrastructure_tool_failure_are_distinct() {
        let mut provider = FakeProvider {
            scripts: vec![Err(ProviderError::Timeout)],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: Vec::new(),
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(25), budget());
        let mut events = Vec::new();
        assert!(matches!(
            AgentExecutor::new(&mut provider, &mut tools).execute(
                &mut run,
                request(),
                &mut |event| events.push(event),
            ),
            Err(AgentExecutionError::Provider(ProviderError::Timeout))
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Failed {
                run_id,
                diagnostic: AgentFailureDiagnostic {
                    kind: AgentFailureKind::Provider,
                    message,
                },
            } if *run_id == AgentRunId::new(25) && message == "Connection timed out"
        )));

        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![
                tool_call("a", "read_file"),
                ProviderChatStreamEvent::Done,
            ])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![Err(ToolInfrastructureError {
                message: "broken executor".into(),
            })],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(26), budget());
        assert!(matches!(
            AgentExecutor::new(&mut provider, &mut tools).execute(&mut run, request(), &mut |_| {}),
            Err(AgentExecutionError::Tool(_))
        ));
    }

    #[test]
    fn cancellation_prevents_provider_and_tool_work() {
        let mut provider = FakeProvider {
            scripts: Vec::new(),
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: Vec::new(),
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(27), budget());
        run.cancellation().cancel();
        let result = AgentExecutor::new(&mut provider, &mut tools)
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err();
        assert!(matches!(result, AgentExecutionError::Cancelled));
        assert_eq!(provider.calls, 0);
        assert!(tools.calls.is_empty());
        assert_eq!(run.state(), AgentState::Cancelled);
    }

    #[test]
    fn cancellation_between_provider_and_tool_skips_tool() {
        let mut run = AgentRun::new(AgentRunId::new(28), budget());
        let cancellation = run.cancellation();
        let mut provider = CancellingProvider {
            cancellation: cancellation.clone(),
            calls: 0,
        };
        let mut tools = FakeTools {
            outcomes: vec![Ok(ToolOutcome::Success("should not run".into()))],
            calls: Vec::new(),
        };
        let result =
            AgentExecutor::new(&mut provider, &mut tools).execute(&mut run, request(), &mut |_| {});
        assert!(matches!(result, Err(AgentExecutionError::Cancelled)));
        assert!(tools.calls.is_empty());
    }

    #[test]
    fn cancellation_between_tool_and_next_provider_stops_next_turn() {
        let mut run = AgentRun::new(AgentRunId::new(29), budget());
        let cancellation = run.cancellation();
        let mut provider = FakeProvider {
            scripts: vec![Ok(vec![
                tool_call("a", "read_file"),
                ProviderChatStreamEvent::Done,
            ])],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = CancellingTool {
            cancellation: cancellation.clone(),
            calls: 0,
        };
        let result =
            AgentExecutor::new(&mut provider, &mut tools).execute(&mut run, request(), &mut |_| {});
        assert!(matches!(result, Err(AgentExecutionError::Cancelled)));
        assert_eq!(provider.calls, 1);
        assert_eq!(tools.calls, 1);
    }

    #[test]
    fn run_scoped_update_approval_allows_same_path_but_not_second_approve_once() {
        let root = approval_workspace();
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    update_call("one", "jogo.html"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    update_call("two", "jogo.html"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("updated".into())),
                Ok(ToolOutcome::Success("updated".into())),
            ],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(101), AgentBudget::new(3, 3));
        let mut executor = AgentExecutor::with_policy(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::with_workspace_root(root.clone()),
        );
        let first = match executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err()
        {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("unexpected error: {other:?}"),
        };
        let second = match executor
            .approve(&mut run, first.approval_id, &mut |_| {})
            .unwrap_err()
        {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(
            executor
                .approve(&mut run, second.approval_id, &mut |_| {})
                .is_ok()
        );
        drop(executor);
        assert_eq!(tools.calls, vec!["update_file", "update_file"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn allow_for_request_covers_same_canonical_path_only() {
        let root = approval_workspace();
        let mut provider = FakeProvider {
            scripts: vec![
                Ok(vec![
                    update_call("one", "sub/../jogo.html"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    update_call("two", "jogo.html"),
                    ProviderChatStreamEvent::Done,
                ]),
                Ok(vec![
                    ProviderChatStreamEvent::ContentDelta("done".into()),
                    ProviderChatStreamEvent::Done,
                ]),
            ],
            calls: 0,
            requests: Vec::new(),
        };
        let mut tools = FakeTools {
            outcomes: vec![
                Ok(ToolOutcome::Success("updated".into())),
                Ok(ToolOutcome::Success("updated".into())),
            ],
            calls: Vec::new(),
        };
        let mut run = AgentRun::new(AgentRunId::new(102), AgentBudget::new(3, 3));
        let mut executor = AgentExecutor::with_policy(
            &mut provider,
            &mut tools,
            ReadOnlyToolPolicy::with_workspace_root(root.clone()),
        );
        let approval = match executor
            .execute(&mut run, request(), &mut |_| {})
            .unwrap_err()
        {
            AgentExecutionError::ApprovalRequired(approval) => approval,
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(
            executor
                .allow_for_request(&mut run, approval.approval_id, &mut |_| {})
                .is_ok()
        );
        drop(executor);
        assert_eq!(tools.calls, vec!["update_file", "update_file"]);

        let mut policy = ReadOnlyToolPolicy::with_workspace_root(root.clone());
        let run_id = AgentRunId::new(102);
        let same_path = ProviderToolCall {
            id: Some("x".into()),
            name: "update_file".into(),
            arguments: serde_json::json!({"path": "jogo.html"}),
        };
        let other_path = ProviderToolCall {
            arguments: serde_json::json!({"path": "config.php"}),
            ..same_path.clone()
        };
        let alias_path = ProviderToolCall {
            arguments: serde_json::json!({"path": "sub/../jogo.html"}),
            ..same_path.clone()
        };
        policy.grant_run_scoped(ToolPolicyContext { run_id }, &alias_path);
        assert!(policy.is_run_scoped_allowed(ToolPolicyContext { run_id }, &same_path));
        assert!(!policy.is_run_scoped_allowed(ToolPolicyContext { run_id }, &other_path));
        assert!(!policy.is_run_scoped_allowed(
            ToolPolicyContext {
                run_id: AgentRunId::new(103)
            },
            &same_path
        ));

        for name in ["write_file", "delete_file"] {
            let other_tool = ProviderToolCall {
                name: name.into(),
                ..same_path.clone()
            };
            assert!(!policy.is_run_scoped_allowed(ToolPolicyContext { run_id }, &other_tool));
            assert!(matches!(
                policy.decide(ToolPolicyContext { run_id }, &other_tool),
                ToolPolicyDecision::RequireApproval { .. }
            ));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancelled_or_denied_run_has_no_scoped_authorization() {
        let root = approval_workspace();
        let call = ProviderToolCall {
            id: Some("x".into()),
            name: "update_file".into(),
            arguments: serde_json::json!({"path": "jogo.html"}),
        };
        let mut policy = ReadOnlyToolPolicy::with_workspace_root(root.clone());
        let run_id = AgentRunId::new(104);
        let context = ToolPolicyContext { run_id };
        assert!(matches!(
            policy.decide(context, &call),
            ToolPolicyDecision::RequireApproval { .. }
        ));
        assert!(!policy.is_run_scoped_allowed(context, &call));
        policy.grant_run_scoped(context, &call);
        assert!(policy.is_run_scoped_allowed(context, &call));
        let mut run = AgentRun::new(run_id, AgentBudget::new(1, 1));
        run.cancel();
        assert!(run.cancellation().is_cancelled());
        assert!(!policy.is_run_scoped_allowed(
            ToolPolicyContext {
                run_id: AgentRunId::new(105)
            },
            &call
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
