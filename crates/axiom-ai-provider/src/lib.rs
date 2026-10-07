use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

mod remote;

pub use remote::{
    ProviderProtocol, provider_chat_stream_with_cancel, provider_protocol,
    supports_native_tools, test_provider_connection,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Ollama,
    OpenAi,
    Anthropic,
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProviderRequestId(pub u64);

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
pub fn next_request_id() -> ProviderRequestId {
    ProviderRequestId(NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed))
}
pub fn is_stale(request: ProviderRequestId, latest: ProviderRequestId) -> bool {
    request != latest
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderUiState {
    Idle,
    Testing,
    Connected { models: Vec<ProviderModel> },
    Error(ProviderError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderUiResponse {
    pub request_id: ProviderRequestId,
    pub provider: ProviderKind,
    pub result: Result<Vec<ProviderModel>, ProviderError>,
}

#[derive(Clone, Debug)]
pub struct RequestTracker {
    current: Option<ProviderRequestId>,
}
impl Default for RequestTracker {
    fn default() -> Self {
        Self { current: None }
    }
}
impl RequestTracker {
    pub fn begin(&mut self) -> ProviderRequestId {
        let id = next_request_id();
        self.current = Some(id);
        id
    }
    pub fn current(&self) -> Option<ProviderRequestId> {
        self.current
    }
    pub fn accepts(&self, response: &ProviderUiResponse) -> bool {
        self.current == Some(response.request_id)
    }
    pub fn invalidate(&mut self) {
        self.current = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub metadata: Option<ModelMetadata>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
    pub context_length: Option<u64>,
    pub embedding_length: Option<u64>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingLevel {
    #[default]
    Minimal,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    pub const fn as_google_str(self) -> &'static str {
        match self {
            Self::Minimal => "MINIMAL",
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::High => "HIGH",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Minimal => "Minimal",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinkingCapability {
    pub supported: bool,
    pub levels: Vec<ThinkingLevel>,
    pub default_level: Option<ThinkingLevel>,
}

impl ThinkingCapability {
    pub fn supports_level(&self, level: ThinkingLevel) -> bool {
        self.levels.contains(&level)
    }
}

pub fn thinking_capability(
    provider: &ProviderKind,
    model: &str,
    metadata: Option<&ModelMetadata>,
) -> ThinkingCapability {
    if matches!(provider, ProviderKind::Ollama) {
        return ThinkingCapability {
            supported: metadata.is_some_and(ModelMetadata::supports_thinking),
            levels: Vec::new(),
            default_level: None,
        };
    }
    let is_google =
        matches!(provider, ProviderKind::Other(name) if name == "Google" || name == "Gemini");
    let normalized_model = model.strip_prefix("models/").unwrap_or(model);
    if is_google && normalized_model.starts_with("gemini-3.5-flash-lite") {
        return ThinkingCapability {
            supported: true,
            levels: vec![
                ThinkingLevel::Minimal,
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
            ],
            default_level: Some(ThinkingLevel::Minimal),
        };
    }
    if is_google && normalized_model.starts_with("gemini-3.7-flash") {
        return ThinkingCapability {
            supported: true,
            levels: vec![
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
            ],
            default_level: Some(ThinkingLevel::Medium),
        };
    }
    ThinkingCapability {
        supported: is_google && metadata.is_some_and(ModelMetadata::supports_thinking),
        levels: Vec::new(),
        default_level: None,
    }
}

impl ModelMetadata {
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|value| value == capability)
    }
    pub fn supports_thinking(&self) -> bool {
        self.supports("thinking")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderConnectionRequest {
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderToolCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub arguments: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Provider-local reasoning needed to replay an assistant turn.
    /// This is never user-visible and is optional for providers that do not
    /// require reasoning history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderChatRequest {
    pub model: String,
    pub messages: Vec<ProviderChatMessage>,
    pub think: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ProviderToolDefinition>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderChatResponse {
    pub content: String,
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderChatStreamEvent {
    ThinkingDelta(String),
    /// Reasoning intended for provider-local history replay. Adapters may
    /// also emit ThinkingDelta when the same reasoning should be presented.
    ReasoningDelta(String),
    ContentDelta(String),
    ToolCall(ProviderToolCall),
    /// Opaque provider-local metadata. It is never user-visible.
    ResponseMetadata(ProviderResponseMetadata),
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderResponseMetadata {
    GoogleThoughtSignature(String),
}

fn emit_stream_event<F>(
    done: &mut bool,
    event: ProviderChatStreamEvent,
    on_event: &mut F,
) -> Result<(), ProviderError>
where
    F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
{
    if *done {
        return Ok(());
    }
    if matches!(event, ProviderChatStreamEvent::Done) {
        *done = true;
    }
    on_event(event)
}

fn ollama_tool_definitions(
    tools: Option<&Vec<ProviderToolDefinition>>,
) -> Option<serde_json::Value> {
    let tools = tools.filter(|tools| !tools.is_empty())?;
    Some(serde_json::Value::Array(
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect(),
    ))
}

fn parse_tool_calls(
    message: Option<&serde_json::Value>,
) -> Result<Vec<ProviderToolCall>, ProviderError> {
    let Some(calls) = message.and_then(|message| message.get("tool_calls")) else {
        return Ok(Vec::new());
    };
    let calls = calls
        .as_array()
        .ok_or_else(|| diagnostic_invalid_response(InvalidResponseCategory::ToolCallsNotArray))?;
    calls
        .iter()
        .map(|call| {
            let function = call
                .get("function")
                .and_then(|value| value.as_object())
                .ok_or_else(|| {
                    diagnostic_invalid_response(InvalidResponseCategory::ToolMissingFunction)
                })?;
            let name = function
                .get("name")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    diagnostic_invalid_response(InvalidResponseCategory::ToolMissingName)
                })?
                .to_owned();
            let arguments = function.get("arguments").cloned().ok_or_else(|| {
                diagnostic_invalid_response(InvalidResponseCategory::ToolMissingArguments)
            })?;
            let id = call
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            Ok(ProviderToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidResponseCategory {
    InvalidUrl,
    HttpStatus,
    ChunkFraming,
    NdjsonParse,
    TruncatedFinalFrame,
    ToolCallsNotArray,
    ToolMissingFunction,
    ToolMissingName,
    ToolMissingArguments,
    ToolSchemaInvalid,
    ResponseDecode,
    NoUsableContent,
    Other,
}

impl InvalidResponseCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalid_url",
            Self::HttpStatus => "http_status",
            Self::ChunkFraming => "chunk_framing",
            Self::NdjsonParse => "ndjson_parse",
            Self::TruncatedFinalFrame => "truncated_final_frame",
            Self::ToolCallsNotArray => "tool_calls_not_array",
            Self::ToolMissingFunction => "tool_missing_function",
            Self::ToolMissingName => "tool_missing_name",
            Self::ToolMissingArguments => "tool_missing_arguments",
            Self::ToolSchemaInvalid => "tool_schema_invalid",
            Self::ResponseDecode => "response_decode",
            Self::NoUsableContent => "no_usable_content",
            Self::Other => "other",
        }
    }
}

fn diagnostic_invalid_response(category: InvalidResponseCategory) -> ProviderError {
    tracing::warn!(
        target: "axiom.ai_diag",
        event = "invalid_response",
        invalid_response_category = category.as_str(),
        "[AI-DIAG]"
    );
    ProviderError::InvalidResponse(category)
}

fn parse_ndjson_frame(
    frame: &[u8],
    truncated_final_frame: bool,
) -> Result<serde_json::Value, ProviderError> {
    serde_json::from_slice(frame).map_err(|_| {
        diagnostic_invalid_response(if truncated_final_frame {
            InvalidResponseCategory::TruncatedFinalFrame
        } else {
            InvalidResponseCategory::NdjsonParse
        })
    })
}

fn ensure_http_status(status: u16) -> Result<(), ProviderError> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(diagnostic_invalid_response(
            InvalidResponseCategory::HttpStatus,
        ))
    }
}

#[derive(Debug, Default)]
struct StreamDiagnostics {
    http_status: Option<u16>,
    content_type: Option<String>,
    response_byte_count: usize,
    ndjson_frame_count: usize,
    done_received: bool,
    content_present: bool,
    thinking_present: bool,
    reasoning_content_present: bool,
    tool_calls_present: bool,
    tool_call_count: usize,
    tool_calls_with_id: usize,
    server_error_code_present: bool,
    server_error_code: Option<String>,
    server_error_type_present: bool,
    server_error_type: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct HttpErrorClassification {
    code_present: bool,
    code: Option<String>,
    type_present: bool,
    error_type: Option<String>,
}

fn content_type_is_json(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .is_some_and(|value| value == "application/json" || value.ends_with("+json"))
}

fn sanitize_server_error_scalar(value: &serde_json::Value) -> Option<String> {
    let raw = match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        _ => return None,
    };
    let mut sanitized = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':' | '/')
            {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect::<String>();
    if sanitized.is_empty() {
        sanitized.push_str("<empty>");
    }
    Some(sanitized)
}

fn classify_http_error_body(content_type: Option<&str>, body: &[u8]) -> HttpErrorClassification {
    if !content_type_is_json(content_type) {
        return HttpErrorClassification::default();
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return HttpErrorClassification::default();
    };
    let code = value
        .get("code")
        .or_else(|| value.get("error").and_then(|error| error.get("code")))
        .and_then(sanitize_server_error_scalar);
    let error_type = value
        .get("type")
        .or_else(|| value.get("error").and_then(|error| error.get("type")))
        .and_then(sanitize_server_error_scalar);
    HttpErrorClassification {
        code_present: code.is_some(),
        code,
        type_present: error_type.is_some(),
        error_type,
    }
}

fn observe_http_error_body(
    diagnostics: &mut StreamDiagnostics,
    content_type: Option<&str>,
    body: &[u8],
) {
    let classification = classify_http_error_body(content_type, body);
    diagnostics.server_error_code_present = classification.code_present;
    diagnostics.server_error_code = classification.code;
    diagnostics.server_error_type_present = classification.type_present;
    diagnostics.server_error_type = classification.error_type;
}

fn decode_complete_chunked_body(input: &[u8]) -> Option<Vec<u8>> {
    let mut rest = input;
    let mut output = Vec::new();
    loop {
        let end = rest.windows(2).position(|window| window == b"\r\n")?;
        let size =
            usize::from_str_radix(std::str::from_utf8(&rest[..end]).ok()?.trim(), 16).ok()?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Some(output);
        }
        if rest.len() < size + 2 {
            return None;
        }
        output.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

fn observe_chat_value(value: &serde_json::Value, diagnostics: &mut StreamDiagnostics) {
    let message = value.get("message");
    diagnostics.content_present |= message
        .and_then(|message| message.get("content"))
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.is_empty());
    diagnostics.thinking_present |= message
        .and_then(|message| message.get("thinking"))
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.is_empty());
    diagnostics.reasoning_content_present |= message
        .and_then(|message| message.get("reasoning_content"))
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.is_empty());
    if let Some(tool_calls) = message.and_then(|message| message.get("tool_calls")) {
        diagnostics.tool_calls_present = true;
        diagnostics.tool_call_count += tool_calls.as_array().map_or(0, Vec::len);
    }
    diagnostics.done_received |= value
        .get("done")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
}

fn emit_chat_value_events<F>(
    value: &serde_json::Value,
    done: &mut bool,
    diagnostics: &mut StreamDiagnostics,
    on_event: &mut F,
) -> Result<(), ProviderError>
where
    F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
{
    diagnostics.ndjson_frame_count += 1;
    observe_chat_value(value, diagnostics);
    let message = value.get("message");
    if let Some(delta) = message
        .and_then(|message| message.get("thinking"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
    {
        emit_stream_event(
            done,
            ProviderChatStreamEvent::ThinkingDelta(delta.into()),
            on_event,
        )?;
    }
    if let Some(delta) = message
        .and_then(|message| message.get("content"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
    {
        emit_stream_event(
            done,
            ProviderChatStreamEvent::ContentDelta(delta.into()),
            on_event,
        )?;
    }
    let calls = parse_tool_calls(message)?;
    diagnostics.tool_calls_with_id += calls.iter().filter(|call| call.id.is_some()).count();
    for call in calls {
        tracing::info!(
            target: "axiom.ai_diag",
            event = "tool_call_parsed",
            id_present = call.id.is_some(),
            "[AI-DIAG]"
        );
        emit_stream_event(done, ProviderChatStreamEvent::ToolCall(call), on_event)?;
    }
    if value
        .get("done")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        emit_stream_event(done, ProviderChatStreamEvent::Done, on_event)?;
    }
    Ok(())
}

pub trait ProviderChat {
    fn chat(
        &self,
        base_url: &str,
        request: &ProviderChatRequest,
    ) -> Result<ProviderChatResponse, ProviderError>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    ConnectionRefused,
    Timeout,
    InvalidResponse(InvalidResponseCategory),
    Authentication,
    RateLimited,
    TemporarilyUnavailable,
    Unavailable(String),
    RequestRejected(String),
}

impl ProviderError {
    pub fn invalid_response(category: InvalidResponseCategory) -> Self {
        Self::InvalidResponse(category)
    }

    pub const fn invalid_response_category(&self) -> Option<InvalidResponseCategory> {
        match self {
            Self::InvalidResponse(category) => Some(*category),
            _ => None,
        }
    }
}

fn parse_http_response(raw: &[u8]) -> Result<(u16, Vec<u8>), ProviderError> {
    let marker = b"\r\n\r\n";
    let split = raw
        .windows(marker.len())
        .position(|w| w == marker)
        .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::HttpStatus))?;
    let (header_bytes, body_bytes) = raw.split_at(split);
    let body_bytes = &body_bytes[4..];
    let headers = std::str::from_utf8(header_bytes)
        .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::HttpStatus))?;
    let status = headers
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::HttpStatus))?;
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = Vec::new();
        let mut rest = body_bytes;
        loop {
            let end = rest.windows(2).position(|w| w == b"\r\n").ok_or_else(|| {
                ProviderError::invalid_response(InvalidResponseCategory::ChunkFraming)
            })?;
            let size = usize::from_str_radix(
                std::str::from_utf8(&rest[..end])
                    .map_err(|_| {
                        ProviderError::invalid_response(InvalidResponseCategory::ChunkFraming)
                    })?
                    .trim(),
                16,
            )
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::ChunkFraming))?;
            rest = &rest[end + 2..];
            if size == 0 {
                break;
            }
            if rest.len() < size + 2 {
                return Err(ProviderError::invalid_response(
                    InvalidResponseCategory::ChunkFraming,
                ));
            }
            out.extend_from_slice(&rest[..size]);
            rest = &rest[size + 2..];
        }
        return Ok((status, out));
    }
    if let Some(length) = headers.lines().find_map(|line| {
        line.strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok())
    }) {
        if body_bytes.len() < length {
            return Err(ProviderError::invalid_response(
                InvalidResponseCategory::Other,
            ));
        }
        return Ok((status, body_bytes[..length].to_vec()));
    }
    Ok((status, body_bytes.to_vec()))
}
impl ProviderError {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::ConnectionRefused => "Connection refused",
            Self::Timeout => "Connection timed out",
            Self::InvalidResponse(_) => "Invalid provider response",
            Self::Authentication => "Authentication failed",
            Self::RateLimited => "Provider rate limited",
            Self::TemporarilyUnavailable => "Provider temporarily unavailable",
            Self::Unavailable(_) => "Provider unavailable",
            Self::RequestRejected(_) => "Provider rejected the request",
        }
    }

    pub fn detailed_user_message(&self) -> String {
        match self {
            Self::RequestRejected(message) => format!("Provider rejected the request: {message}"),
            _ => self.user_message().to_owned(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderConnectionStatus {
    Connected,
    Failed(ProviderError),
}

pub trait ProviderConnectivity {
    fn test_connection(&self, request: &ProviderConnectionRequest) -> ProviderConnectionStatus;
    fn list_models(
        &self,
        request: &ProviderConnectionRequest,
    ) -> Result<Vec<ProviderModel>, ProviderError>;
}

#[derive(Clone, Debug)]
pub struct FakeProvider {
    pub status: ProviderConnectionStatus,
    pub models: Vec<ProviderModel>,
}
impl FakeProvider {
    pub fn connected(models: Vec<ProviderModel>) -> Self {
        Self {
            status: ProviderConnectionStatus::Connected,
            models,
        }
    }
    pub fn failed(error: ProviderError) -> Self {
        Self {
            status: ProviderConnectionStatus::Failed(error),
            models: Vec::new(),
        }
    }
}
impl ProviderConnectivity for FakeProvider {
    fn test_connection(&self, _: &ProviderConnectionRequest) -> ProviderConnectionStatus {
        self.status.clone()
    }
    fn list_models(
        &self,
        _: &ProviderConnectionRequest,
    ) -> Result<Vec<ProviderModel>, ProviderError> {
        match &self.status {
            ProviderConnectionStatus::Connected => Ok(self.models.clone()),
            ProviderConnectionStatus::Failed(e) => Err(e.clone()),
        }
    }
}

pub struct OllamaProvider {
    pub timeout: Duration,
    pub chat_timeout: Duration,
}
impl Default for OllamaProvider {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            chat_timeout: Duration::from_secs(300),
        }
    }
}
impl OllamaProvider {
    pub fn chat_stream<F>(
        &self,
        base_url: &str,
        request: &ProviderChatRequest,
        on_event: F,
    ) -> Result<(), ProviderError>
    where
        F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
    {
        self.chat_stream_with_cancel(base_url, request, || false, on_event)
    }

    pub fn chat_stream_with_cancel<F, C>(
        &self,
        base_url: &str,
        request: &ProviderChatRequest,
        is_cancelled: C,
        on_event: F,
    ) -> Result<(), ProviderError>
    where
        F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
        C: FnMut() -> bool,
    {
        let request_id = next_request_id();
        let started = std::time::Instant::now();
        let mut diagnostics = StreamDiagnostics::default();
        let result = self.chat_stream_with_cancel_inner(
            base_url,
            request,
            request_id,
            is_cancelled,
            on_event,
            &mut diagnostics,
        );
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result.as_ref() {
            Ok(()) => tracing::info!(
                target: "axiom.ai_diag",
                event = "provider_request_finished",
                provider_request_id = request_id.0,
                elapsed_ms,
                http_status = diagnostics.http_status.unwrap_or(0),
                content_type = diagnostics.content_type.as_deref().unwrap_or("<absent>"),
                response_byte_count = diagnostics.response_byte_count,
                ndjson_frame_count = diagnostics.ndjson_frame_count,
                done_received = diagnostics.done_received,
                content_present = diagnostics.content_present,
                thinking_present = diagnostics.thinking_present,
                reasoning_content_present = diagnostics.reasoning_content_present,
                tool_calls_present = diagnostics.tool_calls_present,
                tool_call_count = diagnostics.tool_call_count,
                tool_calls_with_id = diagnostics.tool_calls_with_id,
                server_error_code_present = diagnostics.server_error_code_present,
                server_error_code = diagnostics.server_error_code.as_deref().unwrap_or("<absent>"),
                server_error_type_present = diagnostics.server_error_type_present,
                server_error_type = diagnostics.server_error_type.as_deref().unwrap_or("<absent>"),
                invalid_response_category = "none",
                terminal_error_category = "none",
                "[AI-DIAG]"
            ),
            Err(error) => match error.invalid_response_category() {
                Some(category) => tracing::warn!(
                    target: "axiom.ai_diag",
                    event = "provider_request_finished",
                    provider_request_id = request_id.0,
                    elapsed_ms,
                    http_status = diagnostics.http_status.unwrap_or(0),
                    content_type = diagnostics.content_type.as_deref().unwrap_or("<absent>"),
                    response_byte_count = diagnostics.response_byte_count,
                    ndjson_frame_count = diagnostics.ndjson_frame_count,
                    done_received = diagnostics.done_received,
                    content_present = diagnostics.content_present,
                    thinking_present = diagnostics.thinking_present,
                    reasoning_content_present = diagnostics.reasoning_content_present,
                    tool_calls_present = diagnostics.tool_calls_present,
                    tool_call_count = diagnostics.tool_call_count,
                    tool_calls_with_id = diagnostics.tool_calls_with_id,
                    server_error_code_present = diagnostics.server_error_code_present,
                    server_error_code = diagnostics.server_error_code.as_deref().unwrap_or("<absent>"),
                    server_error_type_present = diagnostics.server_error_type_present,
                    server_error_type = diagnostics.server_error_type.as_deref().unwrap_or("<absent>"),
                    invalid_response_category = category.as_str(),
                    terminal_error_category = "invalid_response",
                    "[AI-DIAG]"
                ),
                None => tracing::warn!(
                    target: "axiom.ai_diag",
                    event = "provider_request_finished",
                    provider_request_id = request_id.0,
                    elapsed_ms,
                    http_status = diagnostics.http_status.unwrap_or(0),
                    content_type = diagnostics.content_type.as_deref().unwrap_or("<absent>"),
                    response_byte_count = diagnostics.response_byte_count,
                    ndjson_frame_count = diagnostics.ndjson_frame_count,
                    done_received = diagnostics.done_received,
                    content_present = diagnostics.content_present,
                    thinking_present = diagnostics.thinking_present,
                    reasoning_content_present = diagnostics.reasoning_content_present,
                    tool_calls_present = diagnostics.tool_calls_present,
                    tool_call_count = diagnostics.tool_call_count,
                    tool_calls_with_id = diagnostics.tool_calls_with_id,
                    server_error_code_present = diagnostics.server_error_code_present,
                    server_error_code = diagnostics.server_error_code.as_deref().unwrap_or("<absent>"),
                    server_error_type_present = diagnostics.server_error_type_present,
                    server_error_type = diagnostics.server_error_type.as_deref().unwrap_or("<absent>"),
                    invalid_response_category = "none",
                    terminal_error_category = match error {
                        ProviderError::ConnectionRefused => "connection_refused",
                        ProviderError::Timeout => "timeout",
                        ProviderError::Authentication => "authentication",
                        ProviderError::RateLimited => "rate_limited",
                        ProviderError::TemporarilyUnavailable => "temporarily_unavailable",
                        ProviderError::Unavailable(_) => "unavailable",
                        ProviderError::RequestRejected(_) => "request_rejected",
                        ProviderError::InvalidResponse(_) => "invalid_response",
                    },
                    "[AI-DIAG]"
                ),
            },
        }
        result
    }

    fn chat_stream_with_cancel_inner<F, C>(
        &self,
        base_url: &str,
        request: &ProviderChatRequest,
        request_id: ProviderRequestId,
        mut is_cancelled: C,
        mut on_event: F,
        diagnostics: &mut StreamDiagnostics,
    ) -> Result<(), ProviderError>
    where
        F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
        C: FnMut() -> bool,
    {
        tracing::info!(
            target: "axiom.ai_diag",
            event = "provider_request_started",
            provider_request_id = request_id.0,
            "[AI-DIAG]"
        );
        let parsed = url::Url::parse(base_url)
            .map_err(|_| diagnostic_invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| diagnostic_invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| diagnostic_invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let mut stream =
            TcpStream::connect((host, port)).map_err(|_| ProviderError::ConnectionRefused)?;
        stream.set_read_timeout(Some(self.chat_timeout)).ok();
        let mut body = serde_json::json!({"model": request.model, "messages": request.messages, "stream": true, "think": request.think});
        if let Some(tools) = ollama_tool_definitions(request.tools.as_ref()) {
            body["tools"] = tools;
        }
        let body = body.to_string();
        write!(stream, "POST /api/chat HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
            .map_err(|_| ProviderError::Unavailable("write failed".into()))?;
        let mut raw = Vec::new();
        let mut decoded = Vec::new();
        let mut headers_done = false;
        let mut chunked = false;
        let mut http_error = false;
        let mut chunk_pos = 0usize;
        let mut done_emitted = false;
        let mut buf = [0u8; 8192];
        loop {
            if is_cancelled() {
                return Ok(());
            }
            let n = stream.read(&mut buf).map_err(|_| ProviderError::Timeout)?;
            if n == 0 {
                break;
            }
            if is_cancelled() {
                return Ok(());
            }
            raw.extend_from_slice(&buf[..n]);
            diagnostics.response_byte_count += n;
            if !headers_done {
                let marker = b"\r\n\r\n";
                let Some(split) = raw.windows(marker.len()).position(|w| w == marker) else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&raw[..split]);
                let status = headers
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or_else(|| {
                        diagnostic_invalid_response(InvalidResponseCategory::HttpStatus)
                    })?;
                let content_type = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Type:")
                            .or_else(|| line.strip_prefix("content-type:"))
                            .map(str::trim)
                    })
                    .map(str::to_owned);
                diagnostics.http_status = Some(status);
                diagnostics.content_type = content_type;
                chunked = headers
                    .to_ascii_lowercase()
                    .contains("transfer-encoding: chunked");
                chunk_pos = split + 4;
                http_error = !(200..300).contains(&status);
                headers_done = true;
            }
            if http_error {
                continue;
            }
            if chunked {
                loop {
                    let Some(end) = raw[chunk_pos..].windows(2).position(|w| w == b"\r\n") else {
                        break;
                    };
                    let end = chunk_pos + end;
                    let size = usize::from_str_radix(
                        String::from_utf8_lossy(&raw[chunk_pos..end]).trim(),
                        16,
                    )
                    .map_err(|_| {
                        diagnostic_invalid_response(InvalidResponseCategory::ChunkFraming)
                    })?;
                    if raw.len() < end + 2 + size + 2 {
                        break;
                    }
                    if size == 0 {
                        chunk_pos = end + 2;
                        break;
                    }
                    decoded.extend_from_slice(&raw[end + 2..end + 2 + size]);
                    chunk_pos = end + 2 + size + 2;
                }
            } else {
                decoded.extend_from_slice(&raw[chunk_pos..]);
                chunk_pos = raw.len();
            }
            while let Some(pos) = decoded.iter().position(|b| *b == b'\n') {
                if is_cancelled() {
                    return Ok(());
                }
                let line = decoded.drain(..=pos).collect::<Vec<_>>();
                let value = parse_ndjson_frame(&line, false)?;
                emit_chat_value_events(&value, &mut done_emitted, diagnostics, &mut on_event)?;
            }
        }
        if is_cancelled() {
            return Ok(());
        }
        if !http_error && !decoded.is_empty() {
            let value = parse_ndjson_frame(&decoded, true)?;
            emit_chat_value_events(&value, &mut done_emitted, diagnostics, &mut on_event)?;
        }
        if http_error {
            let body = if chunked {
                decode_complete_chunked_body(&raw[chunk_pos..]).unwrap_or_default()
            } else {
                raw[chunk_pos..].to_vec()
            };
            let content_type = diagnostics.content_type.clone();
            observe_http_error_body(diagnostics, content_type.as_deref(), &body);
            ensure_http_status(diagnostics.http_status.unwrap_or(0))?;
        }
        Ok(())
    }

    fn parse_models(body: &str) -> Result<Vec<ProviderModel>, ProviderError> {
        let v: serde_json::Value = serde_json::from_str(body)
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::Other))?;
        let a = v
            .get("models")
            .and_then(|x| x.as_array())
            .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::Other))?;
        Ok(a.iter()
            .filter_map(|m| {
                let name = m.get("name")?.as_str()?.to_owned();
                let details = m.get("details");
                let metadata = ModelMetadata {
                    parameter_size: details
                        .and_then(|d| d.get("parameter_size"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                    quantization_level: details
                        .and_then(|d| d.get("quantization_level"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                    context_length: m.get("context_length").and_then(|v| v.as_u64()),
                    embedding_length: m.get("embedding_length").and_then(|v| v.as_u64()),
                    capabilities: m
                        .get("capabilities")
                        .and_then(|v| v.as_array())
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                Some(ProviderModel {
                    id: name.clone(),
                    label: name,
                    metadata: Some(metadata),
                })
            })
            .collect())
    }
    fn fetch(&self, request: &ProviderConnectionRequest) -> Result<String, ProviderError> {
        let parsed = url::Url::parse(&request.base_url)
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let mut stream =
            TcpStream::connect((host, port)).map_err(|_| ProviderError::ConnectionRefused)?;
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();
        write!(
            stream,
            "GET /api/tags HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .map_err(|_| ProviderError::Unavailable("write failed".into()))?;
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .map_err(|_| ProviderError::Timeout)?;
        let response = String::from_utf8(bytes)
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::Other))?;
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::HttpStatus))?;
        if !headers.starts_with("HTTP/1.1 200") && !headers.starts_with("HTTP/1.0 200") {
            return Err(ProviderError::invalid_response(
                InvalidResponseCategory::HttpStatus,
            ));
        }
        if headers
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            let mut out = String::new();
            let mut rest = body;
            loop {
                let (size, tail) = rest.split_once("\r\n").ok_or_else(|| {
                    ProviderError::invalid_response(InvalidResponseCategory::ChunkFraming)
                })?;
                let n = usize::from_str_radix(size.trim(), 16).map_err(|_| {
                    ProviderError::invalid_response(InvalidResponseCategory::ChunkFraming)
                })?;
                if n == 0 {
                    break;
                }
                if tail.len() < n + 2 {
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ChunkFraming,
                    ));
                }
                out.push_str(&tail[..n]);
                rest = &tail[n + 2..];
            }
            Ok(out)
        } else {
            Ok(body.into())
        }
    }

    pub fn chat(
        &self,
        base_url: &str,
        request: &ProviderChatRequest,
    ) -> Result<ProviderChatResponse, ProviderError> {
        let parsed = url::Url::parse(base_url)
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?;
        let mut stream =
            TcpStream::connect((host, port)).map_err(|_| ProviderError::ConnectionRefused)?;
        stream.set_read_timeout(Some(self.chat_timeout)).ok();
        let mut body = serde_json::json!({"model": request.model, "messages": request.messages, "stream": false, "think": request.think});
        if let Some(tools) = ollama_tool_definitions(request.tools.as_ref()) {
            body["tools"] = tools;
        }
        let bytes = body.to_string();
        write!(stream, "POST /api/chat HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", bytes.len(), bytes).map_err(|_| ProviderError::Unavailable("write failed".into()))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|_| ProviderError::Timeout)?;
        let (status, body) = parse_http_response(&raw)?;
        ensure_http_status(status)?;
        let value: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::Other))?;
        let message = value.get("message").unwrap_or(&value);
        Ok(ProviderChatResponse {
            content: message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into(),
            thinking: message
                .get("thinking")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            tool_calls: parse_tool_calls(Some(message))?,
        })
    }
}

impl ProviderChat for OllamaProvider {
    fn chat(
        &self,
        base_url: &str,
        request: &ProviderChatRequest,
    ) -> Result<ProviderChatResponse, ProviderError> {
        self.chat(base_url, request)
    }
}
impl ProviderConnectivity for OllamaProvider {
    fn test_connection(&self, r: &ProviderConnectionRequest) -> ProviderConnectionStatus {
        match self.fetch(r).and_then(|b| {
            serde_json::from_str::<serde_json::Value>(&b)
                .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::Other))
        }) {
            Ok(_) => ProviderConnectionStatus::Connected,
            Err(e) => ProviderConnectionStatus::Failed(e),
        }
    }
    fn list_models(
        &self,
        r: &ProviderConnectionRequest,
    ) -> Result<Vec<ProviderModel>, ProviderError> {
        Self::parse_models(&self.fetch(r)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> ProviderConnectionRequest {
        ProviderConnectionRequest {
            kind: ProviderKind::Ollama,
            base_url: "http://localhost:11434".into(),
            api_key: String::new(),
            model: String::new(),
        }
    }
    #[test]
    fn fake_connection_success_and_models() {
        let p = FakeProvider::connected(vec![ProviderModel {
            id: "llama3".into(),
            label: "llama3".into(),
            metadata: Some(ModelMetadata {
                context_length: Some(8192),
                ..Default::default()
            }),
        }]);
        assert_eq!(
            p.test_connection(&request()),
            ProviderConnectionStatus::Connected
        );
        assert_eq!(p.list_models(&request()).unwrap().len(), 1);
    }
    #[test]
    fn fake_failure_is_typed() {
        let p = FakeProvider::failed(ProviderError::ConnectionRefused);
        assert_eq!(
            p.test_connection(&request()),
            ProviderConnectionStatus::Failed(ProviderError::ConnectionRefused)
        );
        assert!(matches!(
            p.list_models(&request()),
            Err(ProviderError::ConnectionRefused)
        ));
    }
    #[test]
    fn request_ids_are_monotonic_and_stale_detectable() {
        let a = next_request_id();
        let b = next_request_id();
        assert!(b.0 > a.0);
        assert!(is_stale(a, b));
        assert!(!is_stale(b, b));
    }
    #[test]
    fn metadata_is_optional() {
        let m = ProviderModel {
            id: "x".into(),
            label: "X".into(),
            metadata: None,
        };
        assert!(m.metadata.is_none());
    }

    #[test]
    fn ollama_model_metadata_and_capabilities_are_parsed() {
        let body = r#"{"models":[{"name":"qwen","details":{"parameter_size":"27B","quantization_level":"Q4_K_M"},"context_length":32768,"embedding_length":4096,"capabilities":["completion","thinking","tools"],"future_field":true},{"name":"old"}]}"#;
        let models = OllamaProvider::parse_models(body).unwrap();
        let metadata = models[0].metadata.as_ref().unwrap();
        assert_eq!(metadata.parameter_size.as_deref(), Some("27B"));
        assert_eq!(metadata.quantization_level.as_deref(), Some("Q4_K_M"));
        assert_eq!(metadata.context_length, Some(32768));
        assert_eq!(metadata.embedding_length, Some(4096));
        assert!(metadata.supports_thinking());
        assert!(metadata.supports("tools"));
        assert!(models[1].metadata.as_ref().unwrap().capabilities.is_empty());
    }

    #[test]
    fn chat_request_serializes_non_streaming_and_think() {
        let request = ProviderChatRequest {
            model: "demo".into(),
            messages: vec![ProviderChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
                reasoning: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            think: Some(true),
            thinking_level: None,
            tools: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["model"], "demo");
        assert_eq!(value["think"], true);
        assert_eq!(value["messages"][0]["content"], "hi");
        assert!(value.get("tools").is_none());
    }

    #[test]
    fn gemini_thinking_capability_exposes_levels_and_minimal_default() {
        let capability = thinking_capability(
            &ProviderKind::Other("Google".into()),
            "gemini-3.5-flash-lite",
            None,
        );
        assert!(capability.supported);
        assert_eq!(capability.default_level, Some(ThinkingLevel::Minimal));
        assert_eq!(capability.levels.len(), 4);
        assert!(capability.supports_level(ThinkingLevel::High));
    }

    #[test]
    fn gemini_37_thinking_capability_excludes_minimal() {
        let capability = thinking_capability(
            &ProviderKind::Other("Google".into()),
            "gemini-3.7-flash",
            None,
        );
        assert!(capability.supported);
        assert_eq!(capability.levels, vec![ThinkingLevel::Low, ThinkingLevel::Medium, ThinkingLevel::High]);
        assert_eq!(capability.default_level, Some(ThinkingLevel::Medium));
        assert!(!capability.supports_level(ThinkingLevel::Minimal));
    }


    #[test]
    fn provider_reasoning_is_optional_and_stays_on_provider_message() {
        let message = ProviderChatMessage {
            role: ChatRole::Assistant,
            content: String::new(),
            reasoning: Some("provider-local plan".into()),
            tool_call_id: None,
            tool_calls: vec![ProviderToolCall {
                id: Some("call-1".into()),
                name: "read_file".into(),
                arguments: serde_json::json!({"path": "README.md"}),
            }],
        };
        let value = serde_json::to_value(message).unwrap();
        assert_eq!(value["reasoning"], "provider-local plan");
        assert_eq!(value["role"], "Assistant");
    }

    #[test]
    fn ollama_tool_definitions_use_native_function_shape() {
        let definition = ProviderToolDefinition {
            name: "read_file".into(),
            description: "Read a UTF-8 file".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "start_line": {"type": "integer"}
                },
                "required": ["path"]
            }),
        };
        let tools = ollama_tool_definitions(Some(&vec![definition])).unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "read_file");
        assert_eq!(tools[0]["function"]["parameters"]["required"][0], "path");
    }

    #[test]
    fn ollama_tool_calls_parse_with_structured_arguments_and_optional_ids() {
        let value = serde_json::json!({
            "message": {"tool_calls": [
                {"id": "call-1", "function": {"name": "read_file", "arguments": {"path": "README.md"}}},
                {"function": {"name": "read_file", "arguments": {"path": "src/Test.php"}}}
            ]}
        });
        let calls = parse_tool_calls(value.get("message")).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id.as_deref(), Some("call-1"));
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "README.md");
        assert_eq!(calls[1].id, None);
    }

    #[test]
    fn malformed_tool_calls_are_rejected_without_panic() {
        let value =
            serde_json::json!({"message": {"tool_calls": [{"function": {"arguments": {}}}]}});
        assert_eq!(
            parse_tool_calls(value.get("message")),
            Err(ProviderError::invalid_response(
                InvalidResponseCategory::ToolMissingName
            ))
        );
    }

    #[test]
    fn stream_chunk_preserves_thinking_content_multiple_calls_and_done() {
        let value = serde_json::json!({
            "message": {
                "thinking": "think",
                "content": "answer",
                "tool_calls": [
                    {"function": {"name": "read_file", "arguments": {"path": "a"}}},
                    {"function": {"name": "read_file", "arguments": {"path": "b"}}}
                ]
            },
            "done": true
        });
        let mut events = Vec::new();
        let mut done = false;
        emit_chat_value_events(
            &value,
            &mut done,
            &mut StreamDiagnostics::default(),
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        assert!(matches!(
            events[0],
            ProviderChatStreamEvent::ThinkingDelta(_)
        ));
        assert!(matches!(
            events[1],
            ProviderChatStreamEvent::ContentDelta(_)
        ));
        assert!(matches!(events[2], ProviderChatStreamEvent::ToolCall(_)));
        assert!(matches!(events[3], ProviderChatStreamEvent::ToolCall(_)));
        assert!(matches!(events[4], ProviderChatStreamEvent::Done));
    }

    #[test]
    fn textual_tool_markup_is_content_not_a_structured_tool_call() {
        let value = serde_json::json!({
            "message": {
                "content": "<tool_call><function=></function></tool_call>",
                "tool_calls": []
            },
            "done": true
        });
        let mut events = Vec::new();
        let mut done = false;
        emit_chat_value_events(
            &value,
            &mut done,
            &mut StreamDiagnostics::default(),
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        assert!(matches!(
            events.first(),
            Some(ProviderChatStreamEvent::ContentDelta(content))
                if content.contains("<tool_call>")
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ProviderChatStreamEvent::ToolCall(_)))
        );
        assert!(matches!(events.last(), Some(ProviderChatStreamEvent::Done)));
    }

    #[test]
    fn provider_neutral_tool_contracts_preserve_definition_call_and_result_message() {
        let definition = ProviderToolDefinition {
            name: "read_file".into(),
            description: "Read a UTF-8 project file".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}}
            }),
        };
        let call = ProviderToolCall {
            id: Some("call-1".into()),
            name: definition.name.clone(),
            arguments: serde_json::json!({"path": "README.md"}),
        };
        let request = ProviderChatRequest {
            model: "demo".into(),
            messages: vec![ProviderChatMessage {
                role: ChatRole::Assistant,
                content: String::new(),
                reasoning: None,
                tool_call_id: None,
                tool_calls: vec![call.clone()],
            }],
            think: None,
            thinking_level: None,
            tools: Some(vec![definition.clone()]),
        };
        assert_eq!(request.tools.as_ref().unwrap()[0], definition);
        assert_eq!(request.messages[0].tool_calls[0], call);
        let result_message = ProviderChatMessage {
            role: ChatRole::Tool,
            content: "README contents".into(),
            reasoning: None,
            tool_call_id: Some("call-1".into()),
            tool_calls: Vec::new(),
        };
        let value = serde_json::to_value(result_message).unwrap();
        assert_eq!(value["role"], "Tool");
        assert_eq!(value["tool_call_id"], "call-1");
    }

    #[test]
    fn tool_call_stream_event_is_representable_without_changing_existing_events() {
        let event = ProviderChatStreamEvent::ToolCall(ProviderToolCall {
            id: None,
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "README.md"}),
        });
        assert!(matches!(event, ProviderChatStreamEvent::ToolCall(_)));
        assert!(matches!(
            ProviderChatStreamEvent::ThinkingDelta("x".into()),
            ProviderChatStreamEvent::ThinkingDelta(_)
        ));
        assert!(matches!(
            ProviderChatStreamEvent::ReasoningDelta("x".into()),
            ProviderChatStreamEvent::ReasoningDelta(_)
        ));
        assert!(matches!(
            ProviderChatStreamEvent::ContentDelta("x".into()),
            ProviderChatStreamEvent::ContentDelta(_)
        ));
        assert!(matches!(
            ProviderChatStreamEvent::Done,
            ProviderChatStreamEvent::Done
        ));
    }

    #[test]
    fn stream_event_emitter_applies_each_delta_once_and_stops_after_done() {
        let mut done = false;
        let mut content = String::new();
        let mut done_count = 0;
        for event in [
            ProviderChatStreamEvent::ContentDelta("abc".into()),
            ProviderChatStreamEvent::ContentDelta("def".into()),
            ProviderChatStreamEvent::ContentDelta("ghi".into()),
            ProviderChatStreamEvent::Done,
            ProviderChatStreamEvent::Done,
            ProviderChatStreamEvent::ContentDelta("late".into()),
        ] {
            emit_stream_event(&mut done, event, &mut |event| {
                match event {
                    ProviderChatStreamEvent::ContentDelta(delta) => content.push_str(&delta),
                    ProviderChatStreamEvent::Done => done_count += 1,
                    ProviderChatStreamEvent::ToolCall(_) => {}
                    ProviderChatStreamEvent::ThinkingDelta(_) => {}
                    ProviderChatStreamEvent::ReasoningDelta(_) => {}
                    ProviderChatStreamEvent::ResponseMetadata(_) => {}
                }
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(content, "abcdefghi");
        assert_eq!(done_count, 1);
    }

    #[test]
    fn http_response_parser_handles_split_sized_and_chunked_bodies() {
        let sized = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello worldEXTRA";
        assert_eq!(parse_http_response(sized).unwrap().1, b"hello world");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(parse_http_response(chunked).unwrap().1, b"hello world");
    }

    #[test]
    fn invalid_response_preserves_only_safe_category() {
        let error = diagnostic_invalid_response(InvalidResponseCategory::ToolMissingArguments);
        assert_eq!(
            error.invalid_response_category(),
            Some(InvalidResponseCategory::ToolMissingArguments)
        );
        assert_eq!(
            format!("{error:?}"),
            "InvalidResponse(ToolMissingArguments)"
        );
        assert_eq!(error.user_message(), "Invalid provider response");
    }

    #[test]
    fn invalid_ndjson_and_truncated_final_frame_have_distinct_categories() {
        let invalid = parse_ndjson_frame(b"{not-json}\n", false).unwrap_err();
        assert_eq!(
            invalid.invalid_response_category(),
            Some(InvalidResponseCategory::NdjsonParse)
        );

        let truncated = parse_ndjson_frame(b"{\"message\":", true).unwrap_err();
        assert_eq!(
            truncated.invalid_response_category(),
            Some(InvalidResponseCategory::TruncatedFinalFrame)
        );
    }

    #[test]
    fn malformed_tool_call_categories_are_distinct() {
        let cases = [
            (
                serde_json::json!({"tool_calls": {}}),
                InvalidResponseCategory::ToolCallsNotArray,
            ),
            (
                serde_json::json!({"tool_calls": [{}]}),
                InvalidResponseCategory::ToolMissingFunction,
            ),
            (
                serde_json::json!({"tool_calls": [{"function": {}}]}),
                InvalidResponseCategory::ToolMissingName,
            ),
            (
                serde_json::json!({"tool_calls": [{"function": {"name": "read_file"}}]}),
                InvalidResponseCategory::ToolMissingArguments,
            ),
        ];

        for (message, expected) in cases {
            let error = parse_tool_calls(Some(&message)).unwrap_err();
            assert_eq!(error.invalid_response_category(), Some(expected));
        }
    }

    #[test]
    fn http_rejection_is_categorized_without_response_payload() {
        let error = ensure_http_status(500).unwrap_err();
        assert_eq!(
            error.invalid_response_category(),
            Some(InvalidResponseCategory::HttpStatus)
        );
        assert_eq!(error.user_message(), "Invalid provider response");
    }

    #[test]
    fn http_json_error_extracts_only_allowlisted_code() {
        let classification = classify_http_error_body(
            Some("application/json"),
            br#"{"code":"upstream_failed","message":"sensitive","detail":{"secret":"value"}}"#,
        );
        assert!(classification.code_present);
        assert_eq!(classification.code.as_deref(), Some("upstream_failed"));
        assert!(!classification.type_present);
        let debug = format!("{classification:?}");
        assert!(!debug.contains("sensitive"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn http_json_error_extracts_only_allowlisted_type() {
        let classification = classify_http_error_body(
            Some("application/problem+json; charset=utf-8"),
            br#"{"error":{"type":"model_backend_error","message":"sensitive"}}"#,
        );
        assert!(!classification.code_present);
        assert!(classification.type_present);
        assert_eq!(
            classification.error_type.as_deref(),
            Some("model_backend_error")
        );
        assert!(!format!("{classification:?}").contains("sensitive"));
    }

    #[test]
    fn unknown_or_non_json_http_errors_do_not_leak_payload() {
        let unknown = classify_http_error_body(
            Some("application/json"),
            br#"{"message":"sensitive","detail":"secret","response":{"body":"payload"}}"#,
        );
        assert_eq!(unknown, HttpErrorClassification::default());
        assert!(!format!("{unknown:?}").contains("sensitive"));
        assert!(!format!("{unknown:?}").contains("secret"));

        let non_json = classify_http_error_body(Some("text/plain"), b"sensitive body");
        assert_eq!(non_json, HttpErrorClassification::default());
        assert!(!format!("{non_json:?}").contains("sensitive"));
    }

    #[test]
    fn server_error_scalars_are_bounded_and_sanitized() {
        let oversized = "x".repeat(100);
        let value = serde_json::json!({"code": oversized, "type": ["not", "scalar"]});
        let classification = classify_http_error_body(
            Some("application/json"),
            &serde_json::to_vec(&value).unwrap(),
        );
        assert_eq!(classification.code.as_deref().map(str::len), Some(64));
        assert!(!classification.type_present);

        let unsafe_value = serde_json::json!({"code": "line one\nline two"});
        let classification = classify_http_error_body(
            Some("application/json"),
            &serde_json::to_vec(&unsafe_value).unwrap(),
        );
        assert_eq!(classification.code.as_deref(), Some("line_one_line_two"));
    }

    #[test]
    fn stream_diagnostics_store_only_safe_metadata() {
        let mut diagnostics = StreamDiagnostics::default();
        observe_chat_value(
            &serde_json::json!({
                "message": {
                    "content": "sensitive content",
                    "thinking": "sensitive reasoning",
                    "reasoning_content": "sensitive reasoning_content",
                    "tool_calls": [
                        {"id": "secret-id", "function": {"name": "secret", "arguments": {"secret": "value"}}},
                        {"function": {"name": "secret", "arguments": {"secret": "value"}}}
                    ]
                },
                "done": true
            }),
            &mut diagnostics,
        );

        assert!(diagnostics.content_present);
        assert!(diagnostics.thinking_present);
        assert!(diagnostics.reasoning_content_present);
        assert!(diagnostics.tool_calls_present);
        assert!(diagnostics.done_received);
        assert_eq!(diagnostics.tool_call_count, 2);
        let debug = format!("{diagnostics:?}");
        assert!(!debug.contains("sensitive"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn bridge_accepts_latest_and_rejects_stale_response() {
        let mut tracker = RequestTracker::default();
        let first = tracker.begin();
        let second = tracker.begin();
        let stale = ProviderUiResponse {
            request_id: first,
            provider: ProviderKind::Ollama,
            result: Ok(Vec::new()),
        };
        let current = ProviderUiResponse {
            request_id: second,
            provider: ProviderKind::Ollama,
            result: Ok(Vec::new()),
        };
        assert!(!tracker.accepts(&stale));
        assert!(tracker.accepts(&current));
        tracker.invalidate();
        assert!(!tracker.accepts(&current));
    }

    #[test]
    fn ui_state_models_success_and_error_without_booleans() {
        let success = ProviderUiState::Connected { models: Vec::new() };
        assert!(matches!(success, ProviderUiState::Connected { .. }));
        let error = ProviderUiState::Error(ProviderError::Timeout);
        assert!(matches!(
            error,
            ProviderUiState::Error(ProviderError::Timeout)
        ));
    }
}
