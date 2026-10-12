use crate::{
    InvalidResponseCategory, OllamaProvider, ProviderChatRequest, ProviderChatStreamEvent,
    ProviderConnectionRequest, ProviderConnectivity, ProviderError, ProviderKind, ProviderModel,
    ProviderResponseMetadata, ProviderToolCall, ProviderToolDefinition, ThinkingLevel,
    parse_tool_calls,
};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::{
    collections::BTreeMap,
    io::Read,
    thread,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderProtocol {
    Ollama,
    OpenAi,
    Anthropic,
    Google,
}

impl ProviderProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
        }
    }
}

pub fn provider_protocol(kind: &ProviderKind) -> ProviderProtocol {
    match kind {
        ProviderKind::Ollama => ProviderProtocol::Ollama,
        ProviderKind::OpenAi => ProviderProtocol::OpenAi,
        ProviderKind::Anthropic => ProviderProtocol::Anthropic,
        ProviderKind::Other(name) if matches!(name.as_str(), "Google" | "Gemini") => {
            ProviderProtocol::Google
        }
        ProviderKind::Other(_) => ProviderProtocol::OpenAi,
    }
}

/// Whether this adapter currently transports provider-neutral tools as native
/// declarations and parses native calls. This is protocol capability, not a
/// model-name allowlist.
pub const fn supports_native_tools(protocol: ProviderProtocol) -> bool {
    matches!(
        protocol,
        ProviderProtocol::Ollama | ProviderProtocol::Google | ProviderProtocol::OpenAi
    )
}

pub fn supports_native_tools_for_model(protocol: ProviderProtocol, model: &str) -> bool {
    supports_native_tools(protocol)
        && !(protocol == ProviderProtocol::OpenAi
            && model.to_ascii_lowercase().contains("mimo"))
}

pub fn test_provider_connection(
    request: &ProviderConnectionRequest,
) -> Result<Vec<ProviderModel>, ProviderError> {
    let protocol = provider_protocol(&request.kind);
    log_dispatch_start(
        "test_connection",
        protocol,
        !request.model.is_empty(),
        !request.base_url.is_empty(),
    );
    let result = if protocol == ProviderProtocol::Ollama {
        let provider = OllamaProvider::default();
        let models = match provider.test_connection(request) {
            crate::ProviderConnectionStatus::Connected => provider.list_models(request),
            crate::ProviderConnectionStatus::Failed(error) => Err(error),
        };
        models.and_then(|models| {
            ensure_model_available(&models, &request.model)?;
            Ok(models)
        })
    } else {
        RemoteProvider::test_connection(request)
    };
    log_terminal(
        "test_connection",
        protocol,
        result.is_ok(),
        result.as_ref().err(),
    );
    result
}

fn ensure_model_available(
    models: &[ProviderModel],
    requested_model: &str,
) -> Result<(), ProviderError> {
    if requested_model.is_empty()
        || models.iter().any(|model| {
            model_id_matches(&model.id, requested_model)
                || model.label == requested_model
                || model_id_matches(&model.label, requested_model)
        })
    {
        Ok(())
    } else {
        Err(ProviderError::Unavailable("model unavailable".into()))
    }
}

pub fn provider_chat_stream_with_cancel<F, C>(
    kind: &ProviderKind,
    api_key: &str,
    base_url: &str,
    request: &ProviderChatRequest,
    is_cancelled: C,
    on_event: F,
) -> Result<(), ProviderError>
where
    F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
    C: FnMut() -> bool,
{
    let protocol = provider_protocol(kind);
    log_dispatch_start(
        "chat",
        protocol,
        !request.model.is_empty(),
        !base_url.is_empty(),
    );
    let result = if protocol == ProviderProtocol::Ollama {
        OllamaProvider::default().chat_stream_with_cancel(base_url, request, is_cancelled, on_event)
    } else {
        RemoteProvider::chat_stream_with_cancel(
            protocol,
            api_key,
            base_url,
            request,
            is_cancelled,
            on_event,
        )
    };
    log_terminal("chat", protocol, result.is_ok(), result.as_ref().err());
    result
}

fn log_dispatch_start(
    operation: &'static str,
    protocol: ProviderProtocol,
    model_present: bool,
    base_url_present: bool,
) {
    tracing::info!(
        target: "axiom.ai_diag",
        operation,
        provider = protocol.as_str(),
        model_present,
        base_url_present,
        adapter = protocol.as_str(),
        parser = protocol.as_str(),
        request_started = false,
        "[AI-DIAG]"
    );
}

fn log_terminal(
    operation: &'static str,
    protocol: ProviderProtocol,
    success: bool,
    error: Option<&ProviderError>,
) {
    let terminal = if success {
        "success"
    } else {
        error_category(error.expect("failed result includes error"))
    };
    tracing::info!(
        target: "axiom.ai_diag",
        operation,
        provider = protocol.as_str(),
        parser = protocol.as_str(),
        terminal,
        "[AI-DIAG]"
    );
}

fn error_category(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::ConnectionRefused => "connection_refused",
        ProviderError::Timeout => "timeout",
        ProviderError::InvalidResponse(category) => category.as_str(),
        ProviderError::Authentication => "authentication",
        ProviderError::RateLimited => "rate_limited",
        ProviderError::TemporarilyUnavailable => "temporarily_unavailable",
        ProviderError::Unavailable(_) => "unavailable",
        ProviderError::RequestRejected(_) => "request_rejected",
    }
}

fn sanitize_google_error_text(value: &serde_json::Value) -> Option<String> {
    let raw = value.as_str()?;
    let sanitized = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric()
                || matches!(character, '_' | '-' | '.' | ':' | '/' | ' ')
            {
                character
            } else {
                '_'
            }
        })
        .take(160)
        .collect::<String>();
    (!sanitized.is_empty()).then_some(sanitized)
}

fn log_google_http_error(
    http_status: u16,
    response: &mut reqwest::blocking::Response,
) -> Option<String> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let mut body = Vec::new();
    let _ = response.take(8192).read_to_end(&mut body);
    let value = serde_json::from_slice::<serde_json::Value>(&body).ok();
    let error = value.as_ref().and_then(|value| value.get("error"));
    let provider_status = error
        .and_then(|error| error.get("status"))
        .and_then(sanitize_google_error_text);
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(sanitize_google_error_text);
    let reason = error
        .and_then(|error| error.get("details"))
        .and_then(serde_json::Value::as_array)
        .and_then(|details| {
            details
                .iter()
                .find_map(|detail| detail.get("reason").and_then(sanitize_google_error_text))
        })
        .or_else(|| {
            value
                .as_ref()
                .and_then(|value| value.get("reason"))
                .and_then(sanitize_google_error_text)
        });
    tracing::warn!(
        target: "axiom.ai_diag",
        event = "google_http_error",
        http_status,
        content_type = content_type.as_str(),
        provider_error_status = provider_status.as_deref().unwrap_or("<unknown>"),
        provider_error_category = provider_status.as_deref().unwrap_or("<unknown>"),
        provider_error_message = message.as_deref().unwrap_or("<unknown>"),
        provider_error_reason = reason.as_deref().unwrap_or("<unknown>"),
        "Google provider request failed",
    );
    message
}

fn sanitize_openai_error_text(value: &serde_json::Value) -> Option<String> {
    let raw = value.as_str()?;
    let sanitized = raw.chars().map(|character| {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':' | '/' | ' ') {
            character
        } else {
            '_'
        }
    }).take(160).collect::<String>();
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

fn openai_error_message(value: &serde_json::Value) -> Option<String> {
    let error = value.get("error").unwrap_or(value);
    error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_owned)
}

fn log_openai_http_error(http_status: u16, response: &mut reqwest::blocking::Response) -> Option<String> {
    let mut body = Vec::new();
    tracing::debug!(
        target: "axiom.ai_diag",
        event = "openai_compatible_http_error_response",
        http_status,
        response_headers = ?response.headers(),
        "captured OpenAI-compatible error response headers before parsing",
    );
    let _ = response.read_to_end(&mut body);
    tracing::debug!(
        target: "axiom.ai_diag",
        event = "openai_compatible_http_error_body",
        http_status,
        raw_body = ?body,
        "captured raw OpenAI-compatible error response body",
    );
    let value = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(value) => value,
        Err(_) => {
            let text = String::from_utf8_lossy(&body).trim().to_owned();
            return (!text.is_empty()).then_some(text);
        }
    };
    let error = value.get("error").unwrap_or(&value);
    let code = error.get("code").and_then(sanitize_openai_error_text);
    let error_type = error.get("type").and_then(sanitize_openai_error_text);
    let message = error.get("message").and_then(sanitize_openai_error_text);
    tracing::warn!(
        target: "axiom.ai_diag",
        event = "openai_compatible_http_error",
        http_status,
        provider_error_code = code.as_deref().unwrap_or("<unknown>"),
        provider_error_type = error_type.as_deref().unwrap_or("<unknown>"),
        provider_error_message = message.as_deref().unwrap_or("<unknown>"),
        "OpenAI-compatible provider request failed",
    );
    openai_error_message(&value)
}

struct RemoteProvider;

impl RemoteProvider {
    fn client() -> Result<Client, ProviderError> {
        Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|_| ProviderError::Unavailable("provider client unavailable".into()))
    }

    fn endpoint(base_url: &str, path: &str) -> Result<Url, ProviderError> {
        let mut url = Url::parse(base_url)
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?;
        url.path_segments_mut()
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::InvalidUrl))?
            .pop_if_empty()
            .extend(path.split('/').filter(|segment| !segment.is_empty()));
        Ok(url)
    }

    fn google_endpoint(base_url: &str, path: &str) -> Result<Url, ProviderError> {
        let base_url = base_url.trim_end_matches('/');
        let path = if base_url.ends_with("/v1beta") {
            path.to_owned()
        } else {
            format!("v1beta/{path}")
        };
        Self::endpoint(base_url, &path)
    }

    fn connection_endpoint(
        protocol: ProviderProtocol,
        base_url: &str,
    ) -> Result<Url, ProviderError> {
        match protocol {
            ProviderProtocol::Google => Self::google_endpoint(base_url, "models"),
            ProviderProtocol::Anthropic => Self::endpoint(base_url, "v1/models"),
            _ => Self::endpoint(base_url, "models"),
        }
    }

    fn auth_headers(protocol: ProviderProtocol, api_key: &str) -> Result<HeaderMap, ProviderError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if api_key.is_empty() {
            return Ok(headers);
        }
        let value = HeaderValue::from_str(api_key)
            .map_err(|_| ProviderError::Unavailable("invalid provider credentials".into()))?;
        match protocol {
            ProviderProtocol::Anthropic => {
                headers.insert("x-api-key", value);
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
            ProviderProtocol::Google => {
                headers.insert("x-goog-api-key", value);
            }
            _ => {
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| {
                        ProviderError::Unavailable("invalid provider credentials".into())
                    })?,
                );
            }
        }
        Ok(headers)
    }

    fn map_status(status: reqwest::StatusCode) -> ProviderError {
        match status.as_u16() {
            401 | 403 => ProviderError::Authentication,
            404 => ProviderError::Unavailable("model unavailable".into()),
            408 => ProviderError::Timeout,
            429 => ProviderError::RateLimited,
            500 | 502 | 503 | 504 => ProviderError::TemporarilyUnavailable,
            _ => ProviderError::invalid_response(InvalidResponseCategory::HttpStatus),
        }
    }

    fn is_retryable_status(status: u16) -> bool {
        matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
    }

    fn retry_delay(attempt: usize) -> Duration {
        Duration::from_millis(match attempt {
            1 => 50,
            _ => 100,
        })
    }

    fn wait_for_retry<C>(delay: Duration, mut is_cancelled: C) -> bool
    where
        C: FnMut() -> bool,
    {
        let deadline = Instant::now() + delay;
        loop {
            if is_cancelled() {
                return false;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return true;
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }

    #[cfg(test)]
    fn with_http_retry<T, F, C>(
        protocol: ProviderProtocol,
        send: F,
        is_cancelled: C,
    ) -> Result<Option<T>, ProviderError>
    where
        F: FnMut(usize) -> Result<(u16, T), ProviderError>,
        C: FnMut() -> bool,
    {
        Self::with_http_retry_diagnostics(protocol, send, is_cancelled, |_, _| None)
    }

    fn with_http_retry_diagnostics<T, F, C, D>(
        protocol: ProviderProtocol,
        mut send: F,
        mut is_cancelled: C,
        mut on_error: D,
    ) -> Result<Option<T>, ProviderError>
    where
        F: FnMut(usize) -> Result<(u16, T), ProviderError>,
        C: FnMut() -> bool,
        D: FnMut(u16, &mut T) -> Option<ProviderError>,
    {
        for attempt in 1..=3 {
            if is_cancelled() {
                return Ok(None);
            }
            let (status, mut response) = send(attempt)?;
            let retry =
                !((200..300).contains(&status)) && Self::is_retryable_status(status) && attempt < 3;
            tracing::info!(
                target: "axiom.ai_diag",
                operation = "provider_http",
                provider = protocol.as_str(),
                request_started = true,
                attempt,
                http_status = status,
                retry,
                "[AI-DIAG]"
            );
            if (200..300).contains(&status) {
                return Ok(Some(response));
            }
            if let Some(error) = on_error(status, &mut response) {
                return Err(error);
            }
            if retry && !Self::wait_for_retry(Self::retry_delay(attempt), &mut is_cancelled) {
                return Ok(None);
            }
            if !retry {
                return Err(Self::map_status(
                    reqwest::StatusCode::from_u16(status)
                        .unwrap_or(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
                ));
            }
        }
        Err(ProviderError::TemporarilyUnavailable)
    }

    fn test_connection(
        request: &ProviderConnectionRequest,
    ) -> Result<Vec<ProviderModel>, ProviderError> {
        let protocol = provider_protocol(&request.kind);
        let url = Self::connection_endpoint(protocol, &request.base_url)?;
        let response = Self::client()?
            .get(url)
            .headers(Self::auth_headers(protocol, &request.api_key)?)
            .send()
            .map_err(|error| {
                tracing::info!(
                    target: "axiom.ai_diag",
                    operation = "test_connection",
                    provider = protocol.as_str(),
                    adapter = protocol.as_str(),
                    parser = protocol.as_str(),
                    request_started = true,
                    "[AI-DIAG]"
                );
                if error.is_timeout() {
                    ProviderError::Timeout
                } else {
                    ProviderError::ConnectionRefused
                }
            })?;
        tracing::info!(
            target: "axiom.ai_diag",
            operation = "test_connection",
            provider = protocol.as_str(),
            adapter = protocol.as_str(),
            parser = protocol.as_str(),
            request_started = true,
            http_status = response.status().as_u16(),
            "[AI-DIAG]"
        );
        if !response.status().is_success() {
            return Err(Self::map_status(response.status()));
        }
        let value: serde_json::Value = response
            .json()
            .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::Other))?;
        let models = parse_models(protocol, &value)?;
        ensure_model_available(&models, &request.model)?;
        Ok(models)
    }

    fn chat_stream_with_cancel<F, C>(
        protocol: ProviderProtocol,
        api_key: &str,
        base_url: &str,
        request: &ProviderChatRequest,
        mut is_cancelled: C,
        mut on_event: F,
    ) -> Result<(), ProviderError>
    where
        F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
        C: FnMut() -> bool,
    {
        if is_cancelled() {
            return Ok(());
        }
        let (url, body) = chat_request(protocol, base_url, request)?;
        let gemini_round = request
            .messages
            .iter()
            .filter(|message| {
                matches!(message.role, crate::ChatRole::Assistant) && !message.tool_calls.is_empty()
            })
            .count()
            + 1;
        let client = Self::client()?;
        let headers = Self::auth_headers(protocol, api_key)?;
        if protocol == ProviderProtocol::Google {
            let thinking_level = match request.thinking_level {
                Some(ThinkingLevel::Minimal) => "minimal",
                Some(ThinkingLevel::Low) => "low",
                Some(ThinkingLevel::Medium) => "medium",
                Some(ThinkingLevel::High) => "high",
                None => "none",
            };
            tracing::info!(
                target: "axiom.ai_diag",
                provider = "google",
                thinking_enabled = request.think.unwrap_or(false),
                thinking_level,
                include_thoughts = request.think == Some(true),
                "google thinking request configuration"
            );
        }
        let mut last_openai_error = None;
        let response_result = Self::with_http_retry_diagnostics(
            protocol,
            |attempt| {
                tracing::info!(
                    target: "axiom.ai_diag",
                    operation = "chat",
                    provider = protocol.as_str(),
                    request_started = true,
                    attempt,
                    round = if protocol == ProviderProtocol::Google {
                        gemini_round
                    } else {
                        0
                    },
                    "[AI-DIAG]"
                );
                let response = client
                    .post(url.clone())
                    .headers(headers.clone())
                    .json(&body)
                    .send()
                    .map_err(|error| {
                        if error.is_timeout() {
                            ProviderError::Timeout
                        } else {
                            ProviderError::ConnectionRefused
                        }
                    })?;
                Ok((response.status().as_u16(), response))
            },
            &mut is_cancelled,
            |status, response| {
                if protocol == ProviderProtocol::Google {
                    let message = log_google_http_error(status, response);
                    if status == 400 {
                        return message.map(ProviderError::RequestRejected);
                    }
                } else if protocol == ProviderProtocol::OpenAi {
                    let message = log_openai_http_error(status, response);
                    if let Some(message) = message {
                        last_openai_error = Some(message.clone());
                        if !Self::is_retryable_status(status) {
                            return Some(ProviderError::RequestRejected(message));
                        }
                    }
                }
                None
            },
        );
        let Some(response) = (match response_result {
            Err(error) if protocol == ProviderProtocol::OpenAi => {
                if let Some(message) = last_openai_error {
                    return Err(ProviderError::RequestRejected(message));
                }
                return Err(error);
            }
            result => result?,
        })
        else {
            return Ok(());
        };
        if protocol == ProviderProtocol::Google {
            tracing::info!(
                target: "axiom.ai_diag",
                event = "gemini_round_http",
                round = gemini_round,
                http_status = response.status().as_u16(),
                "[AI-DIAG]"
            );
        }
        if protocol == ProviderProtocol::OpenAi {
            tracing::debug!(
                target: "axiom.ai_diag",
                event = "openai_compatible_response_headers",
                http_status = response.status().as_u16(),
                response_headers = ?response.headers(),
                model = request.model.as_str(),
                "captured OpenAI-compatible response headers before body parsing",
            );
        }
        if protocol == ProviderProtocol::Google {
            return Self::stream_google_response(response, &mut is_cancelled, &mut on_event);
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let response_status = response.status().as_u16();
        let body = response.bytes().map_err(|error| {
            tracing::warn!(
                target: "axiom.ai_diag",
                event = "openai_compatible_response_read_failed",
                provider = protocol.as_str(),
                error = %error,
                "OpenAI-compatible response body could not be read",
            );
            ProviderError::invalid_response(InvalidResponseCategory::ResponseDecode)
        })?;
        if protocol == ProviderProtocol::OpenAi {
            tracing::debug!(
                target: "axiom.ai_diag",
                event = "openai_compatible_raw_response_body",
                http_status = response_status,
                raw_body = ?body.as_ref(),
                "captured raw OpenAI-compatible response body before parsing",
            );
        }
        if protocol == ProviderProtocol::OpenAi
            && (content_type.contains("text/event-stream")
                || body.trim_ascii_start().starts_with(b"data:"))
        {
            return stream_openai_sse_response(&body, &mut on_event);
        }
        let value: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(error) if protocol == ProviderProtocol::OpenAi => {
                tracing::warn!(
                    target: "axiom.ai_diag",
                    event = "openai_compatible_json_decode_fallback",
                    provider = protocol.as_str(),
                    error = %error,
                    body_prefix = %String::from_utf8_lossy(&body).chars().take(80).collect::<String>(),
                    "trying tolerant SSE parsing for OpenAI-compatible response",
                );
                return stream_openai_sse_response(&body, &mut on_event);
            }
            Err(_) => {
                tracing::info!(
                    target: "axiom.ai_diag",
                    operation = "chat",
                    provider = protocol.as_str(),
                    json_decode = "failure",
                    terminal = "invalid_response",
                    "[AI-DIAG]"
                );
                return Err(ProviderError::invalid_response(InvalidResponseCategory::ResponseDecode));
            }
        };
        tracing::info!(
            target: "axiom.ai_diag",
            operation = "chat",
            provider = protocol.as_str(),
            model_family_present = !request.model.is_empty(),
            json_decode = "success",
            "[AI-DIAG]"
        );
        if is_cancelled() {
            return Ok(());
        }
        let parsed = parse_chat_content_with_diagnostics(protocol, &value)?;
        if let Some(diagnostics) = parsed.diagnostics.as_ref() {
            tracing::info!(
                target: "axiom.ai_diag",
                operation = "chat",
                provider = protocol.as_str(),
                candidate_count = diagnostics.candidate_count,
                part_count = diagnostics.part_count,
                visible_text_parts = diagnostics.visible_text_parts,
                thought_parts = diagnostics.thought_parts,
                structured_parts = diagnostics.structured_parts,
                finish_reason_present = diagnostics.finish_reason_present,
                thought_signature_present = diagnostics.thought_signature_present,
                terminal = "success",
                "[AI-DIAG]"
            );
        }
        let content = parsed.content;
        if !parsed.thinking.is_empty() {
            on_event(ProviderChatStreamEvent::ThinkingDelta(parsed.thinking))?;
        }
        if !content.is_empty() {
            on_event(ProviderChatStreamEvent::ContentDelta(content))?;
        }
        for call in parsed.tool_calls {
            on_event(ProviderChatStreamEvent::ToolCall(call))?;
        }
        for metadata in parsed.metadata {
            on_event(ProviderChatStreamEvent::ResponseMetadata(metadata))?;
        }
        on_event(ProviderChatStreamEvent::Done)
    }

    fn stream_google_response<F, C>(
        mut response: reqwest::blocking::Response,
        is_cancelled: &mut C,
        on_event: &mut F,
    ) -> Result<(), ProviderError>
    where
        F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
        C: FnMut() -> bool,
    {
        let mut parser = GeminiSseParser::default();
        let mut buffer = [0_u8; 8192];
        let mut visible_output = false;

        loop {
            if is_cancelled() {
                return Ok(());
            }
            let count = response.read(&mut buffer).map_err(|error| {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    ProviderError::Timeout
                } else {
                    ProviderError::ConnectionRefused
                }
            })?;
            if count == 0 {
                break;
            }
            for payload in parser.push(&buffer[..count])? {
                if is_cancelled() {
                    return Ok(());
                }
                for event in parse_google_stream_payload(&payload)? {
                    if is_cancelled() {
                        return Ok(());
                    }
                    if matches!(
                        event,
                        ProviderChatStreamEvent::ContentDelta(_)
                            | ProviderChatStreamEvent::ToolCall(_)
                    ) {
                        visible_output = true;
                    }
                    on_event(event)?;
                }
            }
        }

        for payload in parser.finish()? {
            if is_cancelled() {
                return Ok(());
            }
            for event in parse_google_stream_payload(&payload)? {
                if matches!(
                    event,
                    ProviderChatStreamEvent::ContentDelta(_) | ProviderChatStreamEvent::ToolCall(_)
                ) {
                    visible_output = true;
                }
                on_event(event)?;
            }
        }
        if !visible_output {
            return Err(ProviderError::invalid_response(
                InvalidResponseCategory::NoUsableContent,
            ));
        }
        on_event(ProviderChatStreamEvent::Done)
    }
}

#[derive(Default)]
struct GeminiSseParser {
    line: Vec<u8>,
    data_lines: Vec<Vec<u8>>,
}

impl GeminiSseParser {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, ProviderError> {
        self.line.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        while let Some(newline) = self.line.iter().position(|byte| *byte == b'\n') {
            let mut line = self.line.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.consume_line(&line, &mut payloads)?;
        }
        Ok(payloads)
    }

    fn finish(&mut self) -> Result<Vec<Vec<u8>>, ProviderError> {
        let mut payloads = Vec::new();
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.consume_line(&line, &mut payloads)?;
        }
        self.finish_event(&mut payloads)?;
        Ok(payloads)
    }

    fn consume_line(
        &mut self,
        line: &[u8],
        payloads: &mut Vec<Vec<u8>>,
    ) -> Result<(), ProviderError> {
        if line.is_empty() {
            return self.finish_event(payloads);
        }
        if let Some(data) = line.strip_prefix(b"data:") {
            self.data_lines
                .push(data.strip_prefix(b" ").unwrap_or(data).to_vec());
        }
        Ok(())
    }

    fn finish_event(&mut self, payloads: &mut Vec<Vec<u8>>) -> Result<(), ProviderError> {
        if self.data_lines.is_empty() {
            return Ok(());
        }
        let payload = self.data_lines.drain(..).collect::<Vec<_>>().join(&b'\n');
        if payload.iter().all(u8::is_ascii_whitespace) {
            return Ok(());
        }
        if serde_json::from_slice::<serde_json::Value>(&payload).is_err() {
            return Err(ProviderError::invalid_response(
                InvalidResponseCategory::ResponseDecode,
            ));
        }
        payloads.push(payload);
        Ok(())
    }
}

fn parse_google_stream_payload(
    payload: &[u8],
) -> Result<Vec<ProviderChatStreamEvent>, ProviderError> {
    let value: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|_| ProviderError::invalid_response(InvalidResponseCategory::ResponseDecode))?;
    let candidates = value
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::NoUsableContent))?;
    let mut events = Vec::new();
    for candidate in candidates {
        let Some(parts) = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for part in parts {
            let signature = part
                .get("thoughtSignature")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned);
            let thought = part
                .get("thought")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if let Some(signature) = signature {
                events.push(ProviderChatStreamEvent::ResponseMetadata(
                    ProviderResponseMetadata::GoogleThoughtSignature(signature.clone()),
                ));
            }
            if let Some(function) = part.get("functionCall") {
                let Some(name) = function.get("name").and_then(|value| value.as_str()) else {
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolMissingName,
                    ));
                };
                let Some(arguments) = function.get("args") else {
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolMissingArguments,
                    ));
                };
                tracing::info!(
                    target: "axiom.ai_diag",
                    event = "gemini_function_call",
                    name,
                    call_id_present = function.get("id").and_then(|value| value.as_str()).is_some(),
                    thought_signature_present = part.get("thoughtSignature").and_then(|value| value.as_str()).is_some(),
                    relative_path = arguments.get("path").and_then(|value| value.as_str()).unwrap_or(""),
                    classification = "native_function_call",
                    "[AI-DIAG]"
                );
                events.push(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: function
                        .get("id")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned),
                    name: name.to_owned(),
                    arguments: arguments.clone(),
                }));
                continue;
            }
            if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                if thought {
                    events.push(ProviderChatStreamEvent::ThinkingDelta(text.to_owned()));
                } else if !text.is_empty() {
                    events.push(ProviderChatStreamEvent::ContentDelta(text.to_owned()));
                }
            }
        }
    }
    Ok(events)
}

fn model_id_matches(candidate: &str, requested: &str) -> bool {
    fn normalize(value: &str) -> &str {
        value.strip_prefix("models/").unwrap_or(value)
    }
    normalize(candidate) == normalize(requested)
}

fn parse_models(
    protocol: ProviderProtocol,
    value: &serde_json::Value,
) -> Result<Vec<ProviderModel>, ProviderError> {
    let models = if protocol == ProviderProtocol::Google {
        value.get("models").and_then(|value| value.as_array())
    } else {
        value.get("data").and_then(|value| value.as_array())
    }
    .ok_or_else(|| ProviderError::invalid_response(InvalidResponseCategory::Other))?;
    Ok(models
        .iter()
        .filter_map(|model| {
            let id = model
                .get("id")
                .or_else(|| model.get("name"))
                .and_then(|value| value.as_str())?;
            Some(ProviderModel {
                id: id.to_owned(),
                label: id.strip_prefix("models/").unwrap_or(id).to_owned(),
                metadata: None,
            })
        })
        .collect())
}

fn message_role(role: &crate::ChatRole) -> &'static str {
    match role {
        crate::ChatRole::User => "user",
        crate::ChatRole::Assistant => "assistant",
        crate::ChatRole::System => "system",
        crate::ChatRole::Tool => "tool",
    }
}

fn gemini_schema(
    value: &serde_json::Value,
    schema_path: &str,
    tool_name: &str,
) -> Result<serde_json::Value, ProviderError> {
    let object = value.as_object().ok_or_else(|| {
        log_tool_schema_invalid(tool_name, schema_path, "schema must be an object");
        ProviderError::invalid_response(InvalidResponseCategory::ToolSchemaInvalid)
    })?;
    let mut normalized = serde_json::Map::new();
    for (key, value) in object {
        match key.as_str() {
            "type" => {
                let Some(schema_type) = value.as_str() else {
                    log_tool_schema_invalid(tool_name, schema_path, "type must be a string");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                };
                if !matches!(
                    schema_type,
                    "object" | "string" | "integer" | "number" | "boolean" | "array"
                ) {
                    log_tool_schema_invalid(tool_name, schema_path, "unsupported type");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                }
                normalized.insert(key.clone(), value.clone());
            }
            "description" | "format" => {
                if !value.is_string() {
                    log_tool_schema_invalid(tool_name, &format!("{schema_path}.{key}"), "value must be a string");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                }
                normalized.insert(key.clone(), value.clone());
            }
            "nullable" => {
                if !value.is_boolean() {
                    log_tool_schema_invalid(tool_name, &format!("{schema_path}.{key}"), "value must be a boolean");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                }
                normalized.insert(key.clone(), value.clone());
            }
            "properties" => {
                let Some(properties) = value.as_object() else {
                    log_tool_schema_invalid(tool_name, &format!("{schema_path}.properties"), "value must be an object");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                };
                let mut normalized_properties = serde_json::Map::new();
                for (name, schema) in properties {
                    normalized_properties.insert(
                        name.clone(),
                        gemini_schema(
                            schema,
                            &format!("{schema_path}.properties.{name}"),
                            tool_name,
                        )?,
                    );
                }
                normalized.insert(
                    key.clone(),
                    serde_json::Value::Object(normalized_properties),
                );
            }
            "required" => {
                let Some(required) = value.as_array() else {
                    log_tool_schema_invalid(tool_name, &format!("{schema_path}.required"), "value must be an array");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                };
                if !required.iter().all(serde_json::Value::is_string) {
                    log_tool_schema_invalid(tool_name, &format!("{schema_path}.required"), "items must be strings");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                }
                normalized.insert(key.clone(), value.clone());
            }
            "items" => {
                normalized.insert(
                    key.clone(),
                    gemini_schema(value, &format!("{schema_path}.items"), tool_name)?,
                );
            }
            "enum" => {
                if !value.is_array() {
                    log_tool_schema_invalid(tool_name, &format!("{schema_path}.enum"), "value must be an array");
                    return Err(ProviderError::invalid_response(
                        InvalidResponseCategory::ToolSchemaInvalid,
                    ));
                }
                normalized.insert(key.clone(), value.clone());
            }
            "additionalProperties" if value == &serde_json::Value::Bool(false) => {}
            // Gemini's Schema subset does not support numeric or string
            // length bounds. Runtime tool validation remains authoritative.
            "minimum" | "maximum" | "minLength" | "maxLength" => {}
            // OpenAPI-only fields such as additionalProperties are not part
            // of Gemini's function declaration Schema subset.
            _ => {
                log_tool_schema_invalid(
                    tool_name,
                    &format!("{schema_path}.{key}"),
                    "unsupported Gemini schema keyword",
                );
                return Err(ProviderError::invalid_response(
                    InvalidResponseCategory::ToolSchemaInvalid,
                ));
            }
        }
    }
    if !normalized.contains_key("type") {
        log_tool_schema_invalid(tool_name, schema_path, "missing type");
        return Err(ProviderError::invalid_response(
            InvalidResponseCategory::ToolSchemaInvalid,
        ));
    }
    Ok(serde_json::Value::Object(normalized))
}

fn log_tool_schema_invalid(tool_name: &str, schema_path: &str, reason: &str) {
    tracing::warn!(
        target: "axiom.ai_diag",
        event = "tool_schema_invalid",
        tool = tool_name,
        schema_path,
        reason,
        "Gemini tool schema rejected locally"
    );
}

fn gemini_function_declaration(
    tool: &ProviderToolDefinition,
) -> Result<serde_json::Value, ProviderError> {
    Ok(serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "parameters": gemini_schema(&tool.parameters, "parameters", &tool.name)?,
    }))
}

fn openai_tool_definition(tool: &ProviderToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

fn chat_request(
    protocol: ProviderProtocol,
    base_url: &str,
    request: &ProviderChatRequest,
) -> Result<(Url, serde_json::Value), ProviderError> {
    match protocol {
        ProviderProtocol::Google => {
            let model = request
                .model
                .strip_prefix("models/")
                .unwrap_or(&request.model);
            let google_kind = crate::ProviderKind::Other("Google".to_owned());
            let capability = crate::thinking_capability(&google_kind, model, None);
            let thinking_level = if request.think == Some(true) {
                if let Some(level) = request.thinking_level
                    && !capability.supports_level(level)
                {
                    return Err(ProviderError::RequestRejected(format!(
                        "thinking level {} is not supported for this model",
                        level.label()
                    )));
                }
                request.thinking_level
            } else {
                None
            };
            let mut url = RemoteProvider::google_endpoint(
                base_url,
                &format!("models/{model}:streamGenerateContent"),
            )?;
            url.query_pairs_mut().append_pair("alt", "sse");
            let mut contents = Vec::new();
            for message in request
                .messages
                .iter()
                .filter(|message| !matches!(message.role, crate::ChatRole::System))
            {
                let role = if matches!(message.role, crate::ChatRole::Assistant) {
                    "model"
                } else {
                    "user"
                };
                let mut parts = Vec::new();
                if !message.content.is_empty() && !matches!(message.role, crate::ChatRole::Tool) {
                    parts.push(serde_json::json!({"text": message.content}));
                }
                if matches!(message.role, crate::ChatRole::Assistant) {
                    for call in &message.tool_calls {
                        let mut function_call = serde_json::json!({
                            "name": call.name,
                            "args": call.arguments,
                        });
                        if let Some(id) = &call.id {
                            function_call["id"] = serde_json::Value::String(id.clone());
                        }
                        let mut part = serde_json::json!({"functionCall": function_call});
                        if let Some(signature) = &message.reasoning {
                            part["thoughtSignature"] = serde_json::Value::String(signature.clone());
                        }
                        parts.push(part);
                    }
                } else if matches!(message.role, crate::ChatRole::Tool) {
                    let call = request.messages.iter().rev().find_map(|candidate| {
                        (matches!(candidate.role, crate::ChatRole::Assistant))
                            .then(|| {
                                candidate.tool_calls.iter().find(|call| {
                                    call.id.as_deref() == message.tool_call_id.as_deref()
                                })
                            })
                            .flatten()
                    });
                    let Some(call) = call else {
                        tracing::warn!(
                            target: "axiom.ai_diag",
                            event = "gemini_function_response_correlation_failed",
                            function_response_id_present = message.tool_call_id.is_some(),
                            classification = "missing_function_call",
                            "[AI-DIAG]"
                        );
                        return Err(ProviderError::invalid_response(
                            InvalidResponseCategory::ToolMissingFunction,
                        ));
                    };
                    tracing::info!(
                        target: "axiom.ai_diag",
                        event = "gemini_function_response",
                        name = call.name.as_str(),
                        function_response_id_present = message.tool_call_id.is_some(),
                        function_response_id_matches = call.id.as_deref() == message.tool_call_id.as_deref(),
                        relative_path = call.arguments.get("path").and_then(|value| value.as_str()).unwrap_or(""),
                        classification = "native_function_response",
                        "[AI-DIAG]"
                    );
                    let response = serde_json::from_str(&message.content)
                        .unwrap_or_else(|_| serde_json::json!({"content": message.content}));
                    let mut function_response = serde_json::json!({
                        "name": call.name,
                        "response": response,
                    });
                    if let Some(id) = &message.tool_call_id {
                        function_response["id"] = serde_json::Value::String(id.clone());
                    }
                    parts.push(serde_json::json!({"functionResponse": function_response}));
                }
                contents.push(serde_json::json!({"role": role, "parts": parts}));
            }
            let system = request
                .messages
                .iter()
                .filter(|message| matches!(message.role, crate::ChatRole::System))
                .map(|message| serde_json::json!({"text": message.content}))
                .collect::<Vec<_>>();
            let mut body = serde_json::json!({ "contents": contents });
            if !system.is_empty() {
                body["systemInstruction"] = serde_json::json!({ "parts": system });
            }
            if let Some(tools) = request.tools.as_ref().filter(|tools| !tools.is_empty()) {
                let declarations = tools
                    .iter()
                    .map(gemini_function_declaration)
                    .collect::<Result<Vec<_>, _>>()?;
                body["tools"] = serde_json::json!([{
                    "functionDeclarations": declarations
                }]);
            }
            if let Some(level) = thinking_level {
                let mut thinking_config = serde_json::json!({
                    "thinkingLevel": level.as_google_str(),
                });
                if request.think == Some(true) {
                    thinking_config["includeThoughts"] = serde_json::Value::Bool(true);
                }
                body["generationConfig"] = serde_json::json!({
                    "thinkingConfig": thinking_config,
                });
            }
            Ok((url, body))
        }
        ProviderProtocol::Anthropic => {
            let url = RemoteProvider::endpoint(base_url, "v1/messages")?;
            let system = request
                .messages
                .iter()
                .filter(|message| matches!(message.role, crate::ChatRole::System))
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let messages = request
                .messages
                .iter()
                .filter(|message| !matches!(message.role, crate::ChatRole::System))
                .map(|message| {
                    serde_json::json!({
                        "role": message_role(&message.role),
                        "content": message.content,
                    })
                })
                .collect::<Vec<_>>();
            let mut body = serde_json::json!({
                "model": request.model,
                "max_tokens": 4096,
                "messages": messages,
            });
            if !system.is_empty() {
                body["system"] = system.into();
            }
            Ok((url, body))
        }
        _ => {
            let url = RemoteProvider::endpoint(base_url, "chat/completions")?;
            let messages = request
                .messages
                .iter()
                .map(|message| {
                    let mut value = serde_json::json!({
                        "role": message_role(&message.role),
                        "content": message.content,
                    });
                    if matches!(message.role, crate::ChatRole::Assistant) && !message.tool_calls.is_empty() {
                        value["tool_calls"] = serde_json::Value::Array(message.tool_calls.iter().map(|call| {
                            serde_json::json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments.to_string(),
                                }
                            })
                        }).collect());
                    }
                    if matches!(message.role, crate::ChatRole::Tool) {
                        if let Some(id) = &message.tool_call_id {
                            value["tool_call_id"] = serde_json::Value::String(id.clone());
                        }
                    }
                    value
                })
                .collect::<Vec<_>>();
            let mut body = serde_json::json!({
                "model": request.model,
                "messages": messages,
                "stream": false,
            });
            if let Some(tools) = request.tools.as_ref().filter(|tools| !tools.is_empty()) {
                body["tools"] = serde_json::Value::Array(
                    tools.iter().map(openai_tool_definition).collect(),
                );
            }
            Ok((url, body))
        }
    }
}

#[cfg(test)]
fn parse_chat_content(
    protocol: ProviderProtocol,
    value: &serde_json::Value,
) -> Result<String, ProviderError> {
    Ok(parse_chat_content_with_diagnostics(protocol, value)?.content)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GoogleResponseDiagnostics {
    candidate_count: usize,
    part_count: usize,
    visible_text_parts: usize,
    thought_parts: usize,
    structured_parts: usize,
    finish_reason_present: bool,
    thought_signature_present: bool,
}

#[derive(Debug, Default)]
struct ParsedChatContent {
    content: String,
    thinking: String,
    metadata: Vec<ProviderResponseMetadata>,
    tool_calls: Vec<ProviderToolCall>,
    diagnostics: Option<GoogleResponseDiagnostics>,
}

fn parse_chat_content_with_diagnostics(
    protocol: ProviderProtocol,
    value: &serde_json::Value,
) -> Result<ParsedChatContent, ProviderError> {
    let invalid = || ProviderError::invalid_response(InvalidResponseCategory::Other);
    match protocol {
        ProviderProtocol::Google => parse_google_chat_content(value),
        ProviderProtocol::Anthropic => value
            .get("content")
            .and_then(|content| content.as_array())
            .map(|content| {
                content
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
                    .collect::<String>()
            })
            .map(|content| ParsedChatContent {
                content,
                thinking: String::new(),
                metadata: Vec::new(),
                tool_calls: Vec::new(),
                diagnostics: None,
            })
            .ok_or_else(invalid),
        _ => parse_openai_chat_content(value, invalid),
    }
}

fn parse_openai_chat_content(
    value: &serde_json::Value,
    invalid: impl Fn() -> ProviderError,
) -> Result<ParsedChatContent, ProviderError> {
    let Some(choices) = value.get("choices") else {
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "openai_compatible_response_missing_choices",
            "OpenAI-compatible response did not contain choices",
        );
        return Err(invalid());
    };
    let Some(choices) = choices.as_array() else {
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "openai_compatible_response_choices_not_array",
            "OpenAI-compatible response choices was not an array",
        );
        return Err(invalid());
    };
    let Some(choice) = choices.first() else {
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "openai_compatible_response_empty_choices",
            "OpenAI-compatible response choices was empty",
        );
        return Err(invalid());
    };
    let Some(message) = choice.get("message") else {
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "openai_compatible_response_missing_message",
            "OpenAI-compatible response choice did not contain message",
        );
        return Err(invalid());
    };
    let content = message
        .get("content")
        .and_then(|content| content.as_str())
        .unwrap_or_default()
        .to_owned();
    let tool_calls = parse_tool_calls(Some(message))?;
    if content.is_empty() && tool_calls.is_empty() {
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "openai_compatible_response_without_usable_content",
            content_is_null = message.get("content").is_some_and(serde_json::Value::is_null),
            tool_calls_present = message.get("tool_calls").is_some(),
            "OpenAI-compatible response contained no visible content or tool calls",
        );
        return Err(invalid());
    }
    Ok(ParsedChatContent {
        content,
        thinking: String::new(),
        metadata: Vec::new(),
        tool_calls,
        diagnostics: None,
    })
}

fn stream_openai_sse_response<F>(
    body: &[u8],
    on_event: &mut F,
) -> Result<(), ProviderError>
where
    F: FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
{
    let text = String::from_utf8_lossy(body);
    let mut saw_sse_marker = false;
    let mut saw_payload = false;
    let mut tool_calls = BTreeMap::<usize, ProviderToolCall>::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
            continue;
        }
        let Some(data) = line.strip_prefix("data:") else {
            if line.starts_with("id:") || line.starts_with("retry:") || line == "ping" {
                saw_sse_marker = true;
                continue;
            }
            continue;
        };
        saw_sse_marker = true;
        let data = data.trim();
        tracing::debug!(
            target: "axiom.ai_diag",
            event = "openai_compatible_raw_sse_chunk",
            raw_chunk = data,
            "captured raw OpenAI-compatible SSE data chunk before parsing",
        );
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value = match serde_json::from_str::<serde_json::Value>(data) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    target: "axiom.ai_diag",
                    event = "openai_compatible_sse_chunk_ignored",
                    error = %error,
                    "ignored malformed OpenAI-compatible SSE data chunk",
                );
                continue;
            }
        };
        let Some(choices) = value.get("choices") else {
            tracing::debug!(
                target: "axiom.ai_diag",
                event = "openai_compatible_sse_chunk_without_choices",
                "ignored OpenAI-compatible SSE metadata chunk",
            );
            continue;
        };
        let Some(choices) = choices.as_array() else {
            tracing::warn!(
                target: "axiom.ai_diag",
                event = "openai_compatible_sse_choices_not_array",
                "ignored OpenAI-compatible SSE chunk with invalid choices",
            );
            continue;
        };
        saw_payload = true;
        for choice in choices {
            let Some(delta) = choice.get("delta").or_else(|| choice.get("message")) else {
                continue;
            };
            if let Some(content) = delta.get("content").and_then(|value| value.as_str()) {
                if !content.is_empty() {
                    on_event(ProviderChatStreamEvent::ContentDelta(content.to_owned()))?;
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(|value| value.as_array()) {
                for call in calls {
                    let index = call
                        .get("index")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(tool_calls.len() as u64) as usize;
                    let entry = tool_calls.entry(index).or_insert_with(|| ProviderToolCall {
                        id: None,
                        name: String::new(),
                        arguments: serde_json::Value::String(String::new()),
                    });
                    if let Some(id) = call.get("id").and_then(|value| value.as_str()) {
                        entry.id = Some(id.to_owned());
                    }
                    if let Some(function) = call.get("function") {
                        if let Some(name) = function.get("name").and_then(|value| value.as_str()) {
                            entry.name.push_str(name);
                        }
                        if let Some(arguments) = function
                            .get("arguments")
                            .and_then(|value| value.as_str())
                        {
                            if let Some(existing) = entry.arguments.as_str() {
                                entry.arguments = serde_json::Value::String(format!(
                                    "{existing}{arguments}"
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    if !saw_sse_marker {
        tracing::warn!(
            target: "axiom.ai_diag",
            event = "openai_compatible_sse_not_detected",
            "response was neither valid JSON nor recognizable SSE",
        );
        return Err(ProviderError::invalid_response(InvalidResponseCategory::ResponseDecode));
    }
    for (_, mut call) in tool_calls {
        let arguments = match call.arguments.as_str() {
            Some(arguments) if arguments.trim().is_empty() => serde_json::json!({}),
            Some(arguments) => match serde_json::from_str(arguments) {
                Ok(arguments) => arguments,
                Err(error) => {
                    tracing::warn!(
                        target: "axiom.ai_diag",
                        event = "openai_compatible_sse_tool_arguments_invalid",
                        error = %error,
                        tool = call.name.as_str(),
                        "ignored incomplete streamed tool call",
                    );
                    continue;
                }
            },
            None => serde_json::json!({}),
        };
        call.arguments = arguments;
        if !call.name.is_empty() {
            on_event(ProviderChatStreamEvent::ToolCall(call))?;
        }
    }
    if !saw_payload {
        tracing::debug!(
            target: "axiom.ai_diag",
            event = "openai_compatible_sse_without_usable_choices",
            "SSE response contained no usable choices",
        );
    }
    on_event(ProviderChatStreamEvent::Done)
}

fn parse_google_chat_content(
    value: &serde_json::Value,
) -> Result<ParsedChatContent, ProviderError> {
    let candidates = value
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            tracing::info!(
                target: "axiom.ai_diag",
                provider = "google",
                candidate_count = 0usize,
                part_count = 0usize,
                visible_text_parts = 0usize,
                thought_parts = 0usize,
                structured_parts = 0usize,
                finish_reason_present = false,
                terminal = "invalid_response",
                "[AI-DIAG]"
            );
            ProviderError::invalid_response(InvalidResponseCategory::NoUsableContent)
        })?;
    let mut diagnostics = GoogleResponseDiagnostics {
        candidate_count: candidates.len(),
        ..GoogleResponseDiagnostics::default()
    };
    let mut visible_content = String::new();
    let mut thinking = String::new();
    let mut metadata = Vec::new();

    for candidate in candidates {
        diagnostics.finish_reason_present |= candidate.get("finishReason").is_some();
        let Some(parts) = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        diagnostics.part_count += parts.len();
        let mut candidate_content = String::new();
        for part in parts {
            if let Some(signature) = part
                .get("thoughtSignature")
                .and_then(serde_json::Value::as_str)
            {
                diagnostics.thought_signature_present = true;
                metadata.push(ProviderResponseMetadata::GoogleThoughtSignature(
                    signature.to_owned(),
                ));
            }
            let thought = part
                .get("thought")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if thought {
                diagnostics.thought_parts += 1;
                if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                    thinking.push_str(text);
                }
                if part.get("text").is_none() {
                    diagnostics.structured_parts += 1;
                }
                continue;
            }
            if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                if !text.is_empty() {
                    diagnostics.visible_text_parts += 1;
                    candidate_content.push_str(text);
                }
            } else {
                diagnostics.structured_parts += 1;
            }
        }
        if visible_content.is_empty() && !candidate_content.is_empty() {
            visible_content = candidate_content;
        }
    }

    if visible_content.is_empty() {
        tracing::info!(
            target: "axiom.ai_diag",
            provider = "google",
            candidate_count = diagnostics.candidate_count,
            part_count = diagnostics.part_count,
            visible_text_parts = diagnostics.visible_text_parts,
            thought_parts = diagnostics.thought_parts,
            structured_parts = diagnostics.structured_parts,
            finish_reason_present = diagnostics.finish_reason_present,
            thought_signature_present = diagnostics.thought_signature_present,
            terminal = "invalid_response",
            "[AI-DIAG]"
        );
        return Err(ProviderError::invalid_response(
            InvalidResponseCategory::NoUsableContent,
        ));
    }
    Ok(ParsedChatContent {
        content: visible_content,
        thinking,
        metadata,
        tool_calls: Vec::new(),
        diagnostics: Some(diagnostics),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProviderChatMessage, ProviderToolDefinition};

    #[test]
    fn openai_sse_ignores_done_ping_null_content_and_metadata_chunks() {
        let body = br#": ping
data: {"id":"x","choices":[{"delta":{"content":null}}]}
data: {"choices":[]}
data: {"choices":[{"delta":{"content":"hello"}}]}
data: [DONE]
"#;
        let mut events = Vec::new();
        stream_openai_sse_response(body, &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            events,
            vec![
                ProviderChatStreamEvent::ContentDelta("hello".into()),
                ProviderChatStreamEvent::Done,
            ]
        );
    }

    #[test]
    fn openai_sse_reassembles_fragmented_tool_arguments() {
        let body = br#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"execute_command","arguments":"{\"command\":"}}]}}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"pwd\"}"}}]}}]}
data: [DONE]
"#;
        let mut events = Vec::new();
        stream_openai_sse_response(body, &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                id: Some("call-1".into()),
                name: "execute_command".into(),
                arguments: serde_json::json!({"command": "pwd"}),
            })
        );
        assert_eq!(events[1], ProviderChatStreamEvent::Done);
    }

    #[test]
    fn openai_sse_without_valid_marker_returns_decode_error() {
        let error = stream_openai_sse_response(br#"unexpected body"#, &mut |_| Ok(())).unwrap_err();
        assert_eq!(
            error.invalid_response_category(),
            Some(InvalidResponseCategory::ResponseDecode)
        );
    }

    #[test]
    fn openai_protocol_supports_native_tools_except_explicit_mimo_fallback() {
        assert!(supports_native_tools(ProviderProtocol::OpenAi));
        assert!(supports_native_tools_for_model(ProviderProtocol::OpenAi, "gpt-5"));
        assert!(!supports_native_tools_for_model(ProviderProtocol::OpenAi, "MiMo-V2-Flash"));
        assert!(!supports_native_tools_for_model(ProviderProtocol::Anthropic, "MiMo"));
    }

    #[test]
    fn openai_compatible_request_serializes_tool_definitions() {
        let request = ProviderChatRequest {
            model: "gpt-5".into(),
            messages: vec![ProviderChatMessage {
                role: crate::ChatRole::User,
                content: "run a command".into(),
                reasoning: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            think: None,
            thinking_level: None,
            tools: Some(vec![ProviderToolDefinition {
                name: "execute_command".into(),
                description: "Run a command".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"]
                }),
            }]),
        };
        let (_, body) = chat_request(ProviderProtocol::OpenAi, "https://example.test/v1", &request).unwrap();
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "execute_command");
        assert_eq!(body["tools"][0]["function"]["parameters"]["required"][0], "command");
    }

    #[test]
    fn openai_compatible_history_serializes_tool_calls_and_correlated_results() {
        let request = ProviderChatRequest {
            model: "test-model".into(),
            messages: vec![
                ProviderChatMessage {
                    role: crate::ChatRole::Assistant,
                    content: String::new(),
                    reasoning: None,
                    tool_call_id: None,
                    tool_calls: vec![ProviderToolCall {
                        id: Some("call_synth_123".into()),
                        name: "execute_command".into(),
                        arguments: serde_json::json!({"command": "pwd"}),
                    }],
                },
                ProviderChatMessage {
                    role: crate::ChatRole::Tool,
                    content: "{\"stdout\":\"/workspace\"}".into(),
                    reasoning: None,
                    tool_call_id: Some("call_synth_123".into()),
                    tool_calls: Vec::new(),
                },
            ],
            think: None,
            thinking_level: None,
            tools: None,
        };
        let (_, body) = chat_request(
            ProviderProtocol::OpenAi,
            "https://platform.xiaomimimo.com/v1",
            &request,
        ).unwrap();
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call_synth_123");
        assert_eq!(body["messages"][0]["tool_calls"][0]["type"], "function");
        assert_eq!(body["messages"][0]["tool_calls"][0]["function"]["name"], "execute_command");
        assert_eq!(body["messages"][0]["tool_calls"][0]["function"]["arguments"], "{\"command\":\"pwd\"}");
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["tool_call_id"], "call_synth_123");
        assert_eq!(body["messages"][1]["content"], "{\"stdout\":\"/workspace\"}");

        let mut native_request = request.clone();
        native_request.messages[0].tool_calls[0].id = Some("call_native_456".into());
        native_request.messages[1].tool_call_id = Some("call_native_456".into());
        let (_, native_body) = chat_request(
            ProviderProtocol::OpenAi,
            "https://platform.xiaomimimo.com/v1",
            &native_request,
        ).unwrap();
        assert_eq!(native_body["messages"][0]["tool_calls"][0]["id"], "call_native_456");
        assert_eq!(native_body["messages"][1]["tool_call_id"], "call_native_456");
    }

    #[test]
    fn openai_compatible_error_returns_provider_message_without_rewriting_it() {
        let value = serde_json::json!({
            "error": {
                "code": "invalid_request",
                "type": "bad_request",
                "message": "tool_call_id missing; unexpected <payload>"
            }
        });
        assert_eq!(
            sanitize_openai_error_text(&value["error"]["message"]).as_deref(),
            Some("tool_call_id missing_ unexpected _payload_")
        );
        assert_eq!(
            openai_error_message(&value).as_deref(),
            Some("tool_call_id missing; unexpected <payload>")
        );
    }

    #[test]
    fn openai_response_ignores_optional_and_unknown_top_level_fields() {
        let value = serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 123,
            "model": "mimo-v2.6-flash",
            "system_fingerprint": "fp-test",
            "usage": {"total_tokens": 5},
            "choices": [{
                "finish_reason": "stop",
                "unexpected_choice_field": true,
                "message": {"role": "assistant", "content": "ok", "unexpected": null}
            }]
        });
        let parsed = parse_chat_content_with_diagnostics(ProviderProtocol::OpenAi, &value).unwrap();
        assert_eq!(parsed.content, "ok");
    }

    #[test]
    fn openai_json_error_payload_extracts_message() {
        let value = serde_json::json!({"error": {"message": "invalid tool schema"}});
        assert_eq!(openai_error_message(&value).as_deref(), Some("invalid tool schema"));
    }

    fn agent_tool_definition_set(include_find_symbol: bool) -> Vec<ProviderToolDefinition> {
        let mut definitions = vec![
            ProviderToolDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "start_line": {"type": "integer"},
                        "end_line": {"type": "integer"}
                    },
                    "required": ["path"]
                }),
            },
            ProviderToolDefinition {
                name: "list_directory".into(),
                description: "List a directory".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            },
            ProviderToolDefinition {
                name: "find_files".into(),
                description: "Find files".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"pattern": {"type": "string"}},
                    "required": ["pattern"],
                    "additionalProperties": false
                }),
            },
            ProviderToolDefinition {
                name: "search_text".into(),
                description: "Search text".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "path": {"type": "string"},
                        "file_pattern": {"type": "string"}
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            ProviderToolDefinition {
                name: "write_file".into(),
                description: "Create a file".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            },
            ProviderToolDefinition {
                name: "update_file".into(),
                description: "Update a file".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "expected_fingerprint": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "expected_fingerprint", "content"],
                    "additionalProperties": false
                }),
            },
            ProviderToolDefinition {
                name: "delete_file".into(),
                description: "Delete a file".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "expected_fingerprint": {"type": "string"}
                    },
                    "required": ["path", "expected_fingerprint"],
                    "additionalProperties": false
                }),
            },
        ];
        if include_find_symbol {
            definitions.insert(
                4,
                ProviderToolDefinition {
                    name: "find_symbol".into(),
                    description: "Find symbols".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"},
                            "kind": {"type": "string"},
                            "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                        },
                        "required": ["query"],
                        "additionalProperties": false
                    }),
                },
            );
            definitions.insert(
                5,
                ProviderToolDefinition {
                    name: "find_references".into(),
                    description: "Find semantic references".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"},
                            "kind": {"type": "string"},
                            "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                        },
                        "required": ["query"],
                        "additionalProperties": false
                    }),
                },
            );
        }
        definitions
    }

    fn google_declarations(
        model: &str,
        definitions: Vec<ProviderToolDefinition>,
    ) -> Result<Vec<serde_json::Value>, ProviderError> {
        let request = ProviderChatRequest {
            model: model.into(),
            messages: Vec::new(),
            think: None,
            thinking_level: None,
            tools: Some(definitions),
        };
        let (_, body) = chat_request(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com",
            &request,
        )?;
        Ok(body["tools"][0]["functionDeclarations"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    #[test]
    fn google_agent_declarations_without_find_symbol_are_valid() {
        let declarations = google_declarations("gemini-3.5-flash-lite", agent_tool_definition_set(false)).unwrap();
        assert_eq!(declarations.len(), 7);
        assert!(!declarations.iter().any(|declaration| declaration["name"] == "find_symbol"));
    }

    #[test]
    fn google_find_symbol_declaration_is_valid_and_normalized() {
        let find_symbol = agent_tool_definition_set(true)
            .into_iter()
            .find(|definition| definition.name == "find_symbol")
            .unwrap();
        let declaration = gemini_function_declaration(&find_symbol).unwrap();
        assert_eq!(declaration["name"], "find_symbol");
        assert_eq!(
            declaration["parameters"]["properties"]["limit"]["type"],
            "integer"
        );
        assert!(declaration["parameters"]["properties"]["limit"]
            .get("minimum")
            .is_none());
        assert!(declaration["parameters"]["properties"]["limit"]
            .get("maximum")
            .is_none());
    }

    #[test]
    fn complete_nine_tool_gemini_agent_set_is_valid() {
        let declarations = google_declarations("gemini-3.5-flash-lite", agent_tool_definition_set(true)).unwrap();
        assert_eq!(declarations.len(), 9);
        let names = declarations
            .iter()
            .map(|declaration| declaration["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "read_file",
                "list_directory",
                "find_files",
                "search_text",
                "find_symbol",
                "find_references",
                "write_file",
                "update_file",
                "delete_file"
            ]
        );
        assert!(declarations.iter().all(|declaration| {
            declaration["parameters"].get("additionalProperties").is_none()
        }));
        assert!(declarations
            .iter()
            .find(|declaration| declaration["name"] == "find_symbol")
            .is_some_and(|declaration| {
                declaration["parameters"]["properties"]["limit"]
                    .get("minimum")
                    .is_none()
                    && declaration["parameters"]["properties"]["limit"]
                        .get("maximum")
                        .is_none()
            }));
    }

    #[test]
    fn switching_between_google_models_does_not_remove_agent_tools() {
        let definitions = agent_tool_definition_set(true);
        for model in ["gemini-3.5-flash-lite", "gemini-3.7-flash"] {
            let declarations = google_declarations(model, definitions.clone()).unwrap();
            assert_eq!(declarations.len(), 9, "tools disappeared for {model}");
        }
    }

    #[test]
    fn google_maps_read_tools_to_function_declarations_and_responses() {
        let request = ProviderChatRequest {
            model: "gemini-3.5-flash-lite".into(),
            messages: vec![ProviderChatMessage {
                role: crate::ChatRole::User,
                content: "read jogo.html".into(),
                reasoning: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            think: None,
            thinking_level: None,
            tools: Some(vec![
                ProviderToolDefinition {
                    name: "read_file".into(),
                    description: "Read a UTF-8 file".into(),
                    parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
                },
                ProviderToolDefinition {
                    name: "list_directory".into(),
                    description: "List a directory".into(),
                    parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
                },
            ]),
        };
        let (_, body) = chat_request(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com",
            &request,
        )
        .unwrap();
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "read_file"
        );
        assert_eq!(
            body["tools"][0]["functionDeclarations"][1]["name"],
            "list_directory"
        );

        let call = ProviderToolCall {
            id: Some("call-7".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({"path":"jogo.html"}),
        };
        let response_request = ProviderChatRequest {
            tools: None,
            messages: vec![
                ProviderChatMessage {
                    role: crate::ChatRole::Assistant,
                    content: String::new(),
                    reasoning: Some("opaque-signature".into()),
                    tool_call_id: None,
                    tool_calls: vec![call],
                },
                ProviderChatMessage {
                    role: crate::ChatRole::Tool,
                    content: "{\"content\":\"actual file\"}".into(),
                    reasoning: None,
                    tool_call_id: Some("call-7".into()),
                    tool_calls: Vec::new(),
                },
            ],
            ..request
        };
        let (_, body) = chat_request(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com",
            &response_request,
        )
        .unwrap();
        assert!(body.get("tools").is_none());
        assert_eq!(
            body["contents"][0]["parts"][0]["functionCall"]["id"],
            "call-7"
        );
        assert_eq!(
            body["contents"][0]["parts"][0]["thoughtSignature"],
            "opaque-signature"
        );
        assert_eq!(
            body["contents"][1]["parts"][0]["functionResponse"]["name"],
            "read_file"
        );
        assert_eq!(
            body["contents"][1]["parts"][0]["functionResponse"]["id"],
            "call-7"
        );
    }

    #[test]
    fn google_write_file_declaration_drops_openapi_only_fields() {
        let write = ProviderToolDefinition {
            name: "write_file".into(),
            description: "Create a new UTF-8 text file.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Workspace-relative path."},
                    "content": {"type": "string", "description": "UTF-8 content."}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        };
        let declaration = gemini_function_declaration(&write).unwrap();
        assert_eq!(
            declaration["parameters"],
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Workspace-relative path."},
                    "content": {"type": "string", "description": "UTF-8 content."}
                },
                "required": ["path", "content"]
            })
        );
        assert!(
            declaration["parameters"]
                .get("additionalProperties")
                .is_none()
        );
    }

    #[test]
    fn google_update_file_declaration_preserves_fingerprint_contract() {
        let update = ProviderToolDefinition {
            name: "update_file".into(),
            description: "Replace an existing UTF-8 text file only when its fingerprint matches."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "expected_fingerprint": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "expected_fingerprint", "content"],
                "additionalProperties": false
            }),
        };
        let declaration = gemini_function_declaration(&update).unwrap();
        assert_eq!(declaration["name"], "update_file");
        assert_eq!(
            declaration["parameters"]["required"],
            serde_json::json!(["path", "expected_fingerprint", "content"])
        );
        assert!(
            declaration["parameters"]
                .get("additionalProperties")
                .is_none()
        );
    }

    #[test]
    fn google_delete_file_declaration_is_native_and_compatible() {
        let delete = ProviderToolDefinition {
            name: "delete_file".into(),
            description: "Delete one existing regular file with a matching fingerprint.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "expected_fingerprint": {"type": "string"}
                },
                "required": ["path", "expected_fingerprint"],
                "additionalProperties": false
            }),
        };
        let declaration = gemini_function_declaration(&delete).unwrap();
        assert_eq!(declaration["name"], "delete_file");
        assert_eq!(
            declaration["parameters"]["required"],
            serde_json::json!(["path", "expected_fingerprint"])
        );
        assert!(
            declaration["parameters"]
                .get("additionalProperties")
                .is_none()
        );
    }

    #[test]
    fn google_read_and_write_declarations_have_no_leaked_unsupported_fields() {
        let definitions = vec![
            ProviderToolDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "start_line": {"type": "integer"},
                        "end_line": {"type": "integer"}
                    },
                    "required": ["path"]
                }),
            },
            ProviderToolDefinition {
                name: "list_directory".into(),
                description: "List a directory".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            },
            ProviderToolDefinition {
                name: "write_file".into(),
                description: "Create a file".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            },
        ];
        let request = ProviderChatRequest {
            model: "gemini-3.5-flash-lite".into(),
            messages: Vec::new(),
            think: None,
            thinking_level: None,
            tools: Some(definitions.clone()),
        };
        let (_, body) = chat_request(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com",
            &request,
        )
        .unwrap();
        let declarations = body["tools"][0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(declarations.len(), 3);
        assert!(declarations.iter().all(|declaration| {
            declaration["parameters"]
                .get("additionalProperties")
                .is_none()
        }));
        assert_eq!(declarations[0]["parameters"], definitions[0].parameters);
        assert_eq!(declarations[1]["parameters"], definitions[1].parameters);
    }

    #[test]
    fn malformed_gemini_provider_schema_fails_before_serialization() {
        let definition = ProviderToolDefinition {
            name: "write_file".into(),
            description: "Create a file".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
                "unsupportedProviderField": true
            }),
        };
        assert_eq!(
            gemini_function_declaration(&definition),
            Err(ProviderError::invalid_response(
                InvalidResponseCategory::ToolSchemaInvalid
            ))
        );
    }

    #[test]
    fn gemini_memory_tool_declarations_omit_unsupported_length_bounds() {
        let definitions = [
            ProviderToolDefinition {
                name: "memory_search".into(),
                description: "Search memory".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "maxLength": 256 },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            ProviderToolDefinition {
                name: "memory_get".into(),
                description: "Get memory".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "id": { "type": "string", "maxLength": 256 } },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        ];
        let request = ProviderChatRequest {
            model: "gemini-2.5-pro".into(),
            messages: Vec::new(),
            think: None,
            thinking_level: None,
            tools: Some(definitions.into()),
        };

        let (_, body) = chat_request(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com",
            &request,
        )
        .expect("Memory declarations must be serializable for Gemini");

        let declarations = body["tools"][0]["functionDeclarations"]
            .as_array()
            .unwrap();
        assert_eq!(declarations.len(), 2);
        assert_eq!(declarations[0]["name"], "memory_search");
        assert_eq!(declarations[1]["name"], "memory_get");
        assert!(declarations.iter().all(|declaration| {
            declaration["parameters"]["properties"]
                .as_object()
                .unwrap()
                .values()
                .all(|property| property.get("maxLength").is_none())
                && declaration["parameters"].get("additionalProperties").is_none()
        }));
    }

    #[test]
    fn google_function_call_parses_without_visible_thinking() {
        let events = parse_google_stream_payload(
            br#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_file","id":"call-7","args":{"path":"jogo.html"}},"thoughtSignature":"opaque-signature"}]}}]}"#,
        )
        .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderChatStreamEvent::ToolCall(call)
                if call.id.as_deref() == Some("call-7")
                    && call.name == "read_file"
                    && call.arguments["path"] == "jogo.html"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderChatStreamEvent::ResponseMetadata(
                ProviderResponseMetadata::GoogleThoughtSignature(signature)
            ) if signature == "opaque-signature"
        )));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ProviderChatStreamEvent::ThinkingDelta(_)))
        );
    }

    #[test]
    fn google_mixed_text_and_function_call_are_both_accepted() {
        let events = parse_google_stream_payload(
            br#"{"candidates":[{"content":{"parts":[{"text":"checking"},{"functionCall":{"name":"memory_search","args":{"query":"tests"}}}]}}]}"#,
        )
        .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderChatStreamEvent::ContentDelta(text) if text == "checking"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderChatStreamEvent::ToolCall(call)
                if call.name == "memory_search" && call.arguments["query"] == "tests"
        )));
    }

    #[test]
    fn maps_provider_kinds_to_supported_protocols() {
        assert_eq!(
            provider_protocol(&ProviderKind::Ollama),
            ProviderProtocol::Ollama
        );
        assert_eq!(
            provider_protocol(&ProviderKind::OpenAi),
            ProviderProtocol::OpenAi
        );
        assert_eq!(
            provider_protocol(&ProviderKind::Anthropic),
            ProviderProtocol::Anthropic
        );
        assert_eq!(
            provider_protocol(&ProviderKind::Other("Google".into())),
            ProviderProtocol::Google
        );
        assert_eq!(
            provider_protocol(&ProviderKind::Other("Gemini".into())),
            ProviderProtocol::Google
        );
        assert_eq!(
            provider_protocol(&ProviderKind::Other("LM Studio".into())),
            ProviderProtocol::OpenAi
        );
    }

    #[test]
    fn model_availability_normalizes_google_model_prefix() {
        let models = vec![ProviderModel {
            id: "models/gemini-2.5-pro".into(),
            label: "gemini-2.5-pro".into(),
            metadata: None,
        }];
        assert!(ensure_model_available(&models, "models/gemini-2.5-pro").is_ok());
        assert!(ensure_model_available(&models, "gemini-2.5-pro").is_ok());
        assert!(matches!(
            ensure_model_available(&models, "missing-model"),
            Err(ProviderError::Unavailable(message)) if message == "model unavailable"
        ));
    }

    #[test]
    fn google_connection_and_chat_endpoints_use_the_configured_v1beta_base() {
        let connection = RemoteProvider::connection_endpoint(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com/v1beta",
        )
        .unwrap();
        assert_eq!(
            connection.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );

        let request = ProviderChatRequest {
            model: "gemini-3.6-flash".into(),
            messages: Vec::new(),
            think: None,
            thinking_level: None,
            tools: None,
        };
        let (chat, _) = chat_request(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com/v1beta",
            &request,
        )
        .unwrap();
        assert_eq!(
            chat.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:streamGenerateContent?alt=sse"
        );
        assert!(chat.as_str().contains(":streamGenerateContent?alt=sse"));
        assert!(!chat.as_str().contains("/chat/completions"));
    }

    #[test]
    fn google_connection_endpoint_adds_v1beta_when_base_is_unversioned() {
        let connection = RemoteProvider::connection_endpoint(
            ProviderProtocol::Google,
            "https://generativelanguage.googleapis.com",
        )
        .unwrap();
        assert_eq!(
            connection.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn google_uses_google_auth_and_response_parser() {
        let headers =
            RemoteProvider::auth_headers(ProviderProtocol::Google, "test-key-not-sensitive")
                .unwrap();
        assert!(headers.contains_key("x-goog-api-key"));
        assert!(!headers.contains_key(reqwest::header::AUTHORIZATION));

        let response = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Google response"}]
                }
            }]
        });
        assert_eq!(
            parse_chat_content(ProviderProtocol::Google, &response).unwrap(),
            "Google response"
        );
    }

    #[test]
    fn google_gemini_36_and_38_text_responses_parse_without_model_allowlist() {
        for model in [
            "gemini-3.6-flash",
            "gemini-3.8-flash",
            "gemini-future-flash",
        ] {
            let request = ProviderChatRequest {
                model: model.into(),
                messages: vec![crate::ProviderChatMessage {
                    role: crate::ChatRole::User,
                    content: "Hello".into(),
                    reasoning: None,
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                }],
                think: None,
                thinking_level: None,
                tools: None,
            };
            let (_, body) = chat_request(
                ProviderProtocol::Google,
                "https://generativelanguage.googleapis.com/v1beta",
                &request,
            )
            .unwrap();
            assert_eq!(body["contents"][0]["parts"][0]["text"], "Hello");
            assert!(body.get("generationConfig").is_none());
            assert!(body.get("thinkingConfig").is_none());

            let response = serde_json::json!({
                "candidates": [{
                    "content": {"parts": [{"text": format!("{model} response")}]},
                    "finishReason": "STOP"
                }]
            });
            assert_eq!(
                parse_chat_content(ProviderProtocol::Google, &response).unwrap(),
                format!("{model} response")
            );
        }
    }

    #[test]
    fn google_collects_visible_text_parts_in_order() {
        let response = serde_json::json!({
            "candidates": [{
                "content": {"parts": [
                    {"text": "first"},
                    {"text": " second"}
                ]}
            }]
        });
        assert_eq!(
            parse_chat_content(ProviderProtocol::Google, &response).unwrap(),
            "first second"
        );
    }

    #[test]
    fn google_omits_thought_parts_but_keeps_visible_text() {
        let response = serde_json::json!({
            "candidates": [{
                "content": {"parts": [
                    {"text": "private thought", "thought": true},
                    {"text": "visible answer", "thoughtSignature": "opaque"}
                ]}
            }]
        });
        let parsed =
            parse_chat_content_with_diagnostics(ProviderProtocol::Google, &response).unwrap();
        assert_eq!(parsed.content, "visible answer");
        assert_eq!(parsed.diagnostics.unwrap().thought_parts, 1);
        assert_eq!(parsed.thinking, "private thought");
        assert_eq!(parsed.metadata.len(), 1);
        assert!(matches!(
            &parsed.metadata[0],
            ProviderResponseMetadata::GoogleThoughtSignature(signature)
                if signature == "opaque"
        ));

        let response = serde_json::json!({
            "candidates": [{
                "content": {"parts": [
                    {"text": "private thought", "thought": true},
                    {"text": "visible answer"}
                ]}
            }]
        });
        let parsed =
            parse_chat_content_with_diagnostics(ProviderProtocol::Google, &response).unwrap();
        assert_eq!(parsed.content, "visible answer");
        assert_eq!(parsed.diagnostics.unwrap().thought_parts, 1);
    }

    #[test]
    fn google_request_maps_level_and_only_requests_visible_thoughts_when_enabled() {
        for (level, expected) in [
            (crate::ThinkingLevel::Minimal, "MINIMAL"),
            (crate::ThinkingLevel::Low, "LOW"),
            (crate::ThinkingLevel::Medium, "MEDIUM"),
            (crate::ThinkingLevel::High, "HIGH"),
        ] {
            for (think, include_thoughts) in [(Some(true), true), (Some(false), false)] {
                let request = ProviderChatRequest {
                    model: "gemini-3.5-flash-lite".into(),
                    messages: Vec::new(),
                    think,
                    thinking_level: Some(level),
                    tools: None,
                };
                let (_, body) = chat_request(
                    ProviderProtocol::Google,
                    "https://generativelanguage.googleapis.com/v1beta",
                    &request,
                )
                .unwrap();
                if think == Some(true) {
                    assert_eq!(
                        body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
                        expected
                    );
                    assert_eq!(
                        body["generationConfig"]["thinkingConfig"]
                            .get("includeThoughts")
                            .is_some(),
                        include_thoughts
                    );
                } else {
                    assert!(body.get("generationConfig").is_none());
                    assert!(body.to_string().find("MINIMAL").is_none());
                }
                assert!(body.to_string().find("thinkingBudget").is_none());
            }
        }
    }

    #[test]
    fn gemini_37_serializes_low_medium_and_high_only() {
        for (level, expected) in [
            (crate::ThinkingLevel::Low, "LOW"),
            (crate::ThinkingLevel::Medium, "MEDIUM"),
            (crate::ThinkingLevel::High, "HIGH"),
        ] {
            let request = ProviderChatRequest {
                model: "gemini-3.7-flash".into(),
                messages: Vec::new(),
                think: Some(true),
                thinking_level: Some(level),
                tools: None,
            };
            let (_, body) = chat_request(
                ProviderProtocol::Google,
                "https://generativelanguage.googleapis.com/v1beta",
                &request,
            )
            .unwrap();
            assert_eq!(body["generationConfig"]["thinkingConfig"]["thinkingLevel"], expected);
            assert_eq!(body["generationConfig"]["thinkingConfig"]["includeThoughts"], true);
        }
    }

    #[test]
    fn gemini_37_minimal_is_rejected_before_http() {
        let request = ProviderChatRequest {
            model: "gemini-3.7-flash".into(),
            messages: Vec::new(),
            think: Some(true),
            thinking_level: Some(crate::ThinkingLevel::Minimal),
            tools: None,
        };
        assert_eq!(
            chat_request(
                ProviderProtocol::Google,
                "https://generativelanguage.googleapis.com/v1beta",
                &request,
            )
            .unwrap_err(),
            ProviderError::RequestRejected(
                "thinking level Minimal is not supported for this model".into()
            )
        );
    }

    #[test]
    fn gemini_sse_parser_handles_fragmented_frames_and_multiple_events() {
        let mut parser = GeminiSseParser::default();
        let first = parser
            .push(b": keepalive\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"thought\",\"thought\":true}]}}")
            .unwrap();
        assert!(first.is_empty());
        let second = parser
            .push(b"]}\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"answer\"}]}}]}\n\n")
            .unwrap();
        assert_eq!(second.len(), 2);

        let first_events = parse_google_stream_payload(&second[0]).unwrap();
        let second_events = parse_google_stream_payload(&second[1]).unwrap();
        assert!(matches!(
            first_events.as_slice(),
            [ProviderChatStreamEvent::ThinkingDelta(text)] if text == "thought"
        ));
        assert!(matches!(
            second_events.as_slice(),
            [ProviderChatStreamEvent::ContentDelta(text)] if text == "answer"
        ));
    }

    #[test]
    fn gemini_stream_payload_preserves_interleaved_order_and_hides_signatures() {
        let payload = br#"{"candidates":[{"content":{"parts":[
            {"text":"a","thought":true},
            {"text":"b","thoughtSignature":"opaque"},
            {"text":"c","thought":true},
            {"text":"d"}
        ]}}]}"#;
        let events = parse_google_stream_payload(payload).unwrap();
        assert!(matches!(
            events.as_slice(),
            [
                ProviderChatStreamEvent::ThinkingDelta(a),
                ProviderChatStreamEvent::ResponseMetadata(
                    ProviderResponseMetadata::GoogleThoughtSignature(signature)
                ),
                ProviderChatStreamEvent::ContentDelta(b),
                ProviderChatStreamEvent::ThinkingDelta(c),
                ProviderChatStreamEvent::ContentDelta(d),
            ] if a == "a" && signature == "opaque" && b == "b" && c == "c" && d == "d"
        ));
    }

    #[test]
    fn gemini_sse_parser_rejects_malformed_json_without_buffering_a_body() {
        let mut parser = GeminiSseParser::default();
        let result = parser.push(b"data: {not-json}\n\n");
        assert!(matches!(
            result,
            Err(ProviderError::InvalidResponse(
                InvalidResponseCategory::ResponseDecode
            ))
        ));
    }

    #[test]
    fn google_tolerates_supported_non_text_parts_when_text_exists() {
        let response = serde_json::json!({
            "candidates": [{
                "content": {"parts": [
                    {"inlineData": {"mimeType": "image/png", "data": "opaque"}},
                    {"functionCall": {"name": "future_tool", "args": {}}},
                    {"text": "answer"}
                ]}
            }]
        });
        let parsed =
            parse_chat_content_with_diagnostics(ProviderProtocol::Google, &response).unwrap();
        assert_eq!(parsed.content, "answer");
        assert_eq!(parsed.diagnostics.unwrap().structured_parts, 2);
    }

    #[test]
    fn google_empty_or_malformed_content_is_controlled() {
        let missing_candidates = serde_json::json!({"usageMetadata": {}});
        assert_eq!(
            parse_chat_content(ProviderProtocol::Google, &missing_candidates),
            Err(ProviderError::invalid_response(
                InvalidResponseCategory::NoUsableContent
            ))
        );
        let thought_only = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "thinking", "thought": true}]},
                "finishReason": "STOP"
            }]
        });
        assert_eq!(
            parse_chat_content(ProviderProtocol::Google, &thought_only),
            Err(ProviderError::invalid_response(
                InvalidResponseCategory::NoUsableContent
            ))
        );
    }

    #[test]
    fn google_http_errors_are_not_response_parse_errors() {
        assert_eq!(
            RemoteProvider::map_status(reqwest::StatusCode::BAD_REQUEST),
            ProviderError::InvalidResponse(InvalidResponseCategory::HttpStatus)
        );
        assert_eq!(
            RemoteProvider::map_status(reqwest::StatusCode::UNAUTHORIZED),
            ProviderError::Authentication
        );
        assert_eq!(
            RemoteProvider::map_status(reqwest::StatusCode::NOT_FOUND),
            ProviderError::Unavailable("model unavailable".into())
        );
        assert_eq!(
            RemoteProvider::map_status(reqwest::StatusCode::REQUEST_TIMEOUT),
            ProviderError::Timeout
        );
        assert_eq!(
            RemoteProvider::map_status(reqwest::StatusCode::TOO_MANY_REQUESTS),
            ProviderError::RateLimited
        );
        for status in [
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            reqwest::StatusCode::BAD_GATEWAY,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            reqwest::StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert_eq!(
                RemoteProvider::map_status(status),
                ProviderError::TemporarilyUnavailable
            );
        }
        assert_eq!(
            ProviderError::TemporarilyUnavailable.user_message(),
            "Provider temporarily unavailable"
        );
    }

    #[test]
    fn transient_http_retry_succeeds_on_second_attempt() {
        let mut attempts = 0;
        let result = RemoteProvider::with_http_retry(
            ProviderProtocol::Google,
            |attempt| {
                attempts += 1;
                Ok((if attempt == 1 { 503 } else { 200 }, "ok"))
            },
            || false,
        )
        .unwrap();
        assert_eq!(result, Some("ok"));
        assert_eq!(attempts, 2);
    }

    #[test]
    fn transient_http_retry_stops_after_three_attempts() {
        let mut attempts = 0;
        let result = RemoteProvider::with_http_retry(
            ProviderProtocol::Google,
            |_| {
                attempts += 1;
                Ok((503, ()))
            },
            || false,
        );
        assert_eq!(result, Err(ProviderError::TemporarilyUnavailable));
        assert_eq!(attempts, 3);
    }

    #[test]
    fn non_transient_http_errors_are_not_retried() {
        for status in [400, 401, 403] {
            let mut attempts = 0;
            let result = RemoteProvider::with_http_retry(
                ProviderProtocol::Google,
                |_| {
                    attempts += 1;
                    Ok((status, ()))
                },
                || false,
            );
            assert_eq!(attempts, 1);
            let expected = match status {
                401 | 403 => ProviderError::Authentication,
                _ => ProviderError::InvalidResponse(InvalidResponseCategory::HttpStatus),
            };
            assert_eq!(result, Err(expected));
        }
    }

    #[test]
    fn cancellation_during_transient_backoff_prevents_next_attempt() {
        let mut attempts = 0;
        let mut cancellation_checks = 0;
        let result = RemoteProvider::with_http_retry(
            ProviderProtocol::Google,
            |_| {
                attempts += 1;
                Ok((503, ()))
            },
            || {
                cancellation_checks += 1;
                cancellation_checks > 1
            },
        )
        .unwrap();
        assert_eq!(result, None);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn transient_http_failure_does_not_invoke_response_parser() {
        let mut parser_calls = 0;
        let result = RemoteProvider::with_http_retry(
            ProviderProtocol::Google,
            |_| Ok((503, serde_json::json!({"malformed": true}))),
            || false,
        );
        if result.is_ok() {
            parser_calls += 1;
        }
        assert_eq!(result, Err(ProviderError::TemporarilyUnavailable));
        assert_eq!(parser_calls, 0);
    }

    #[test]
    fn unavailable_provider_errors_do_not_claim_ollama() {
        let error = ProviderError::Unavailable("model unavailable".into());
        assert_eq!(error.user_message(), "Provider unavailable");
        assert!(!error.user_message().contains("Ollama"));
    }
}
