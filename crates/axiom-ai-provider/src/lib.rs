use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
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
    ContentDelta(String),
    ToolCall(ProviderToolCall),
    Done,
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
    let calls = calls.as_array().ok_or(ProviderError::InvalidResponse)?;
    calls
        .iter()
        .map(|call| {
            let function = call
                .get("function")
                .and_then(|value| value.as_object())
                .ok_or(ProviderError::InvalidResponse)?;
            let name = function
                .get("name")
                .and_then(|value| value.as_str())
                .ok_or(ProviderError::InvalidResponse)?
                .to_owned();
            let arguments = function
                .get("arguments")
                .cloned()
                .ok_or(ProviderError::InvalidResponse)?;
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

fn emit_chat_value_events<F>(
    value: &serde_json::Value,
    done: &mut bool,
    on_event: &mut F,
) -> Result<(), ProviderError>
where
    F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
{
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
    for call in parse_tool_calls(message)? {
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
    InvalidResponse,
    Authentication,
    Unavailable(String),
}

fn parse_http_response(raw: &[u8]) -> Result<(u16, Vec<u8>), ProviderError> {
    let marker = b"\r\n\r\n";
    let split = raw
        .windows(marker.len())
        .position(|w| w == marker)
        .ok_or(ProviderError::InvalidResponse)?;
    let (header_bytes, body_bytes) = raw.split_at(split);
    let body_bytes = &body_bytes[4..];
    let headers = std::str::from_utf8(header_bytes).map_err(|_| ProviderError::InvalidResponse)?;
    let status = headers
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or(ProviderError::InvalidResponse)?;
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = Vec::new();
        let mut rest = body_bytes;
        loop {
            let end = rest
                .windows(2)
                .position(|w| w == b"\r\n")
                .ok_or(ProviderError::InvalidResponse)?;
            let size = usize::from_str_radix(
                std::str::from_utf8(&rest[..end])
                    .map_err(|_| ProviderError::InvalidResponse)?
                    .trim(),
                16,
            )
            .map_err(|_| ProviderError::InvalidResponse)?;
            rest = &rest[end + 2..];
            if size == 0 {
                break;
            }
            if rest.len() < size + 2 {
                return Err(ProviderError::InvalidResponse);
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
            return Err(ProviderError::InvalidResponse);
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
            Self::InvalidResponse => "Invalid Ollama response",
            Self::Authentication => "Authentication failed",
            Self::Unavailable(_) => "Ollama unavailable",
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
        mut is_cancelled: C,
        mut on_event: F,
    ) -> Result<(), ProviderError>
    where
        F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
        C: FnMut() -> bool,
    {
        let parsed = url::Url::parse(base_url).map_err(|_| ProviderError::InvalidResponse)?;
        let host = parsed.host_str().ok_or(ProviderError::InvalidResponse)?;
        let port = parsed
            .port_or_known_default()
            .ok_or(ProviderError::InvalidResponse)?;
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
                    .ok_or(ProviderError::InvalidResponse)?;
                if !(200..300).contains(&status) {
                    return Err(ProviderError::Unavailable("http error".into()));
                }
                chunked = headers
                    .to_ascii_lowercase()
                    .contains("transfer-encoding: chunked");
                chunk_pos = split + 4;
                headers_done = true;
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
                    .map_err(|_| ProviderError::InvalidResponse)?;
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
                let value: serde_json::Value =
                    serde_json::from_slice(&line).map_err(|_| ProviderError::InvalidResponse)?;
                emit_chat_value_events(&value, &mut done_emitted, &mut on_event)?;
            }
        }
        if is_cancelled() {
            return Ok(());
        }
        if !decoded.is_empty() {
            let value: serde_json::Value =
                serde_json::from_slice(&decoded).map_err(|_| ProviderError::InvalidResponse)?;
            emit_chat_value_events(&value, &mut done_emitted, &mut on_event)?;
        }
        Ok(())
    }

    fn parse_models(body: &str) -> Result<Vec<ProviderModel>, ProviderError> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|_| ProviderError::InvalidResponse)?;
        let a = v
            .get("models")
            .and_then(|x| x.as_array())
            .ok_or(ProviderError::InvalidResponse)?;
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
        let parsed =
            url::Url::parse(&request.base_url).map_err(|_| ProviderError::InvalidResponse)?;
        let host = parsed.host_str().ok_or(ProviderError::InvalidResponse)?;
        let port = parsed
            .port_or_known_default()
            .ok_or(ProviderError::InvalidResponse)?;
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
        let response = String::from_utf8(bytes).map_err(|_| ProviderError::InvalidResponse)?;
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .ok_or(ProviderError::InvalidResponse)?;
        if !headers.starts_with("HTTP/1.1 200") && !headers.starts_with("HTTP/1.0 200") {
            return Err(ProviderError::Unavailable("http error".into()));
        }
        if headers
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            let mut out = String::new();
            let mut rest = body;
            loop {
                let (size, tail) = rest
                    .split_once("\r\n")
                    .ok_or(ProviderError::InvalidResponse)?;
                let n = usize::from_str_radix(size.trim(), 16)
                    .map_err(|_| ProviderError::InvalidResponse)?;
                if n == 0 {
                    break;
                }
                if tail.len() < n + 2 {
                    return Err(ProviderError::InvalidResponse);
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
        let parsed = url::Url::parse(base_url).map_err(|_| ProviderError::InvalidResponse)?;
        let host = parsed.host_str().ok_or(ProviderError::InvalidResponse)?;
        let port = parsed
            .port_or_known_default()
            .ok_or(ProviderError::InvalidResponse)?;
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
        if !(200..300).contains(&status) {
            return Err(ProviderError::Unavailable(
                String::from_utf8_lossy(&body).into(),
            ));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| ProviderError::InvalidResponse)?;
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
                .map_err(|_| ProviderError::InvalidResponse)
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
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            think: Some(true),
            tools: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["model"], "demo");
        assert_eq!(value["think"], true);
        assert_eq!(value["messages"][0]["content"], "hi");
        assert!(value.get("tools").is_none());
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
            Err(ProviderError::InvalidResponse)
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
        emit_chat_value_events(&value, &mut done, &mut |event| {
            events.push(event);
            Ok(())
        })
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
                tool_call_id: None,
                tool_calls: vec![call.clone()],
            }],
            think: None,
            tools: Some(vec![definition.clone()]),
        };
        assert_eq!(request.tools.as_ref().unwrap()[0], definition);
        assert_eq!(request.messages[0].tool_calls[0], call);
        let result_message = ProviderChatMessage {
            role: ChatRole::Tool,
            content: "README contents".into(),
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
