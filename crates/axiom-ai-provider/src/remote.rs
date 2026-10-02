use crate::{
    InvalidResponseCategory, OllamaProvider, ProviderChatRequest, ProviderChatStreamEvent,
    ProviderConnectionRequest, ProviderConnectivity, ProviderError, ProviderKind, ProviderModel,
};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::time::Duration;
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
    log_terminal("test_connection", protocol, result.is_ok(), result.as_ref().err());
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
        OllamaProvider::default().chat_stream_with_cancel(
            base_url,
            request,
            is_cancelled,
            on_event,
        )
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
        ProviderError::Unavailable(_) => "unavailable",
    }
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

    fn auth_headers(
        protocol: ProviderProtocol,
        api_key: &str,
    ) -> Result<HeaderMap, ProviderError> {
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
            _ => ProviderError::invalid_response(InvalidResponseCategory::HttpStatus),
        }
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
        let response = Self::client()?
            .post(url)
            .headers(Self::auth_headers(protocol, api_key)?)
            .json(&body)
            .send()
            .map_err(|error| {
                tracing::info!(
                    target: "axiom.ai_diag",
                    operation = "chat",
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
            operation = "chat",
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
        if is_cancelled() {
            return Ok(());
        }
        let content = parse_chat_content(protocol, &value)?;
        if !content.is_empty() {
            on_event(ProviderChatStreamEvent::ContentDelta(content))?;
        }
        on_event(ProviderChatStreamEvent::Done)
    }
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

fn chat_request(
    protocol: ProviderProtocol,
    base_url: &str,
    request: &ProviderChatRequest,
) -> Result<(Url, serde_json::Value), ProviderError> {
    match protocol {
        ProviderProtocol::Google => {
            let model = request.model.strip_prefix("models/").unwrap_or(&request.model);
            let url = RemoteProvider::google_endpoint(
                base_url,
                &format!("models/{model}:generateContent"),
            )?;
            let contents = request
                .messages
                .iter()
                .filter(|message| !matches!(message.role, crate::ChatRole::System))
                .map(|message| {
                    serde_json::json!({
                        "role": if matches!(message.role, crate::ChatRole::Assistant) {
                            "model"
                        } else {
                            "user"
                        },
                        "parts": [{"text": message.content}],
                    })
                })
                .collect::<Vec<_>>();
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
                    serde_json::json!({
                        "role": message_role(&message.role),
                        "content": message.content,
                    })
                })
                .collect::<Vec<_>>();
            Ok((
                url,
                serde_json::json!({
                    "model": request.model,
                    "messages": messages,
                    "stream": false,
                }),
            ))
        }
    }
}

fn parse_chat_content(
    protocol: ProviderProtocol,
    value: &serde_json::Value,
) -> Result<String, ProviderError> {
    let invalid = || ProviderError::invalid_response(InvalidResponseCategory::Other);
    match protocol {
        ProviderProtocol::Google => value
            .get("candidates")
            .and_then(|candidates| candidates.get(0))
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
                    .collect::<String>()
            })
            .ok_or_else(invalid),
        ProviderProtocol::Anthropic => value
            .get("content")
            .and_then(|content| content.as_array())
            .map(|content| {
                content
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
                    .collect::<String>()
            })
            .ok_or_else(invalid),
        _ => value
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_str())
            .map(str::to_owned)
            .ok_or_else(invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_provider_kinds_to_supported_protocols() {
        assert_eq!(provider_protocol(&ProviderKind::Ollama), ProviderProtocol::Ollama);
        assert_eq!(provider_protocol(&ProviderKind::OpenAi), ProviderProtocol::OpenAi);
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
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:generateContent"
        );
        assert!(!chat.as_str().contains("/chat/completions"));
        assert!(!chat.as_str().contains("/api/chat"));
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
            RemoteProvider::auth_headers(ProviderProtocol::Google, "test-key-not-sensitive").unwrap();
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
    fn unavailable_provider_errors_do_not_claim_ollama() {
        let error = ProviderError::Unavailable("model unavailable".into());
        assert_eq!(error.user_message(), "Provider unavailable");
        assert!(!error.user_message().contains("Ollama"));
    }
}
