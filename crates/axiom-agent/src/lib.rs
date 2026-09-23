//! Provider-neutral, headless lifecycle contracts for one logical agent run.
//!
//! This crate deliberately does not execute providers or tools. It contains
//! only identity, state, budget, cancellation, and event semantics that a
//! future execution adapter can use.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
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
            arguments: bounded_arguments(&call.arguments.to_string()),
            reason: bounded_arguments(reason),
        }
    }
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOnlyToolPolicy;

impl ToolPolicy for ReadOnlyToolPolicy {
    fn decide(
        &mut self,
        _context: ToolPolicyContext,
        call: &axiom_ai_provider::ProviderToolCall,
    ) -> ToolPolicyDecision {
        match call.name.as_str() {
            "read_file" | "list_directory" | "fetch_url" => ToolPolicyDecision::Allow,
            _ => ToolPolicyDecision::Deny {
                reason: format!("tool '{}' is not allowed", call.name),
            },
        }
    }
}

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
                message: error.user_message().into(),
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
    let mut messages = request
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
    let mut synthesis = String::from(
        "Final response mode: tools are disabled. Do not emit tool calls or tool-call markup. \
         Produce only the final user-facing answer using the read-only tool results below.",
    );
    if !tool_results.is_empty() {
        synthesis.push_str("\n\n");
        synthesis.push_str(&tool_results.join("\n\n"));
    }
    messages.push(axiom_ai_provider::ProviderChatMessage {
        role: axiom_ai_provider::ChatRole::System,
        content: synthesis,
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
    pending: Option<ExecutionContinuation>,
}

impl<'a, P, T> AgentExecutor<'a, P, T, ReadOnlyToolPolicy>
where
    P: ProviderExecutor,
    T: ToolExecutor,
{
    pub fn new(provider: &'a mut P, tools: &'a mut T) -> Self {
        Self::with_policy(provider, tools, ReadOnlyToolPolicy)
    }
}

impl<'a, P, T, Policy> AgentExecutor<'a, P, T, Policy>
where
    P: ProviderExecutor,
    T: ToolExecutor,
    Policy: ToolPolicy,
{
    pub fn with_policy(provider: &'a mut P, tools: &'a mut T, policy: Policy) -> Self {
        Self {
            provider,
            tools,
            policy,
            pending: None,
        }
    }

    pub fn execute(
        &mut self,
        run: &mut AgentRun,
        request: axiom_ai_provider::ProviderChatRequest,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        emit(AgentEvent::RunStarted { run_id: run.id() });
        run.transition(AgentState::Preparing)
            .map_err(AgentExecutionError::Transition)?;
        run.transition(AgentState::RunningModel)
            .map_err(AgentExecutionError::Transition)?;
        self.drive(
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
        )
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
        self.drive(run, continuation, true, emit)
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
        self.drive(run, continuation, false, emit)
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
                return self.failed(run, emit, AgentExecutionError::BudgetExceeded(error));
            }
            let finalization_turn = run.usage().provider_turns == run.budget().max_provider_turns;
            let request_for_turn = if finalization_turn {
                finalization_request(&continuation.request)
            } else {
                continuation.request.clone()
            };
            emit(AgentEvent::ModelStarted { run_id: run.id() });
            let mut calls = Vec::new();
            let mut final_content = String::new();
            let mut provider_reasoning = String::new();
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
                    axiom_ai_provider::ProviderChatStreamEvent::ToolCall(call) => calls.push(call),
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
                reasoning: (!provider_reasoning.is_empty()).then_some(provider_reasoning),
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
            let decision = if approved_call {
                ToolPolicyDecision::Allow
            } else {
                self.policy
                    .decide(ToolPolicyContext { run_id: run.id() }, &call)
            };
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
            match outcome {
                ToolOutcome::Success(result) => {
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
    use std::sync::{Arc, Mutex};

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
            tools: Some(Vec::new()),
        }
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
        assert_eq!(result.messages().len(), 3);
        assert_eq!(result.messages()[1].role, ChatRole::Assistant);
        assert_eq!(result.messages()[2].role, ChatRole::Tool);
        assert_eq!(result.messages()[2].content, "contents");
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
            provider.requests[1].messages[1].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            provider.requests[1].messages[1].tool_calls[0].id.as_deref(),
            Some("call-1")
        );
        assert_eq!(
            result.messages()[1].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            result.messages()[3].reasoning.as_deref(),
            Some("reasoning-b")
        );
        assert_eq!(result.messages()[1].content, "");
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
            provider.requests[1].messages[1].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            provider.requests[2].messages[1].reasoning.as_deref(),
            Some("reasoning-a")
        );
        assert_eq!(
            provider.requests[2].messages[3].reasoning.as_deref(),
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
            result_a.messages()[1].reasoning.as_deref(),
            Some("run-a reasoning")
        );
        assert_eq!(
            result_b.messages()[1].reasoning.as_deref(),
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
        assert_eq!(result.messages()[2].content, "A");
        assert_eq!(result.messages()[3].content, "B");
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
            provider.requests[1].messages[1].reasoning.as_deref(),
            Some("approval reasoning")
        );
        assert_eq!(
            provider.requests[1].messages[1].tool_calls[0].id.as_deref(),
            Some("approval-1")
        );
        assert_eq!(
            result.messages()[3].reasoning.as_deref(),
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
            provider.requests[1].messages[1].reasoning.as_deref(),
            Some("deny reasoning")
        );
        assert_eq!(
            result.messages()[3].reasoning.as_deref(),
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
        assert!(provider.requests[4].messages.iter().any(|message| {
            matches!(message.role, ChatRole::System)
                && message.content.contains("one")
                && message.content.contains("four")
        }));
        assert_eq!(
            provider.requests[4]
                .messages
                .last()
                .map(|message| &message.role),
            Some(&ChatRole::System)
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
        assert!(provider.requests[1].messages.iter().any(|message| {
            matches!(message.role, ChatRole::System) && message.content.contains("not found")
        }));
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
}
