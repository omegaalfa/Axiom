//! Bounded native read-only tool orchestration for chat.

use super::tools::{ToolArguments, ToolKind, ToolName, ToolRegistry, ToolRequest, ToolResult};
use axiom_ai_provider::{
    ProviderChatMessage, ProviderChatRequest, ProviderChatStreamEvent, ProviderError,
    ProviderToolCall, ProviderToolDefinition,
};
use serde_json::json;
use std::sync::atomic::AtomicBool;

/// Four rounds allow a short directory-navigation chain without turning chat
/// into an unbounded autonomous loop.
pub(crate) const MAX_TOOL_ROUNDS: usize = 4;
/// Four calls per round keeps a single request bounded even when a provider
/// emits several calls at once.
pub(crate) const MAX_TOOL_CALLS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolRoundTripError {
    Provider(ProviderError),
    Tool(String),
    Cancelled,
    ToolRoundLimit,
    ToolCallLimit,
}

pub(crate) fn user_message(error: &ToolRoundTripError) -> String {
    match error {
        ToolRoundTripError::Provider(error) => error.user_message().to_owned(),
        ToolRoundTripError::Cancelled => "Generation cancelled".into(),
        ToolRoundTripError::Tool(_) => "Tool request failed".into(),
        ToolRoundTripError::ToolRoundLimit => "Tool round limit reached".into(),
        ToolRoundTripError::ToolCallLimit => "Tool call limit reached".into(),
    }
}

pub(crate) fn read_file_definition() -> ProviderToolDefinition {
    ProviderToolDefinition {
        name: "read_file".into(),
        description: "Read a UTF-8 file inside the current workspace.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "start_line": { "type": "integer" },
                "end_line": { "type": "integer" }
            },
            "required": ["path"]
        }),
    }
}

pub(crate) fn list_directory_definition() -> ProviderToolDefinition {
    ProviderToolDefinition {
        name: "list_directory".into(),
        description:
            "List the immediate entries of a workspace-relative directory; this is non-recursive."
                .into(),
        parameters: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
    }
}

pub(crate) fn fetch_url_definition() -> ProviderToolDefinition {
    ProviderToolDefinition {
        name: "fetch_url".into(),
        description: "Fetch the textual content of one explicit public HTTP or HTTPS URL. Does not search the web, execute JavaScript, operate a browser, log in, follow links autonomously, or download binary files.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Public HTTP or HTTPS URL to fetch."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
    }
}

type RunProvider<'a> = dyn FnMut(
        &ProviderChatRequest,
        &mut dyn FnMut(ProviderChatStreamEvent) -> Result<(), ProviderError>,
    ) -> Result<(), ProviderError>
    + 'a;

/// Runs bounded native tool rounds and returns the events that should be
/// applied to the existing assistant placeholder.
pub(crate) fn run_read_file_round_trip(
    request: ProviderChatRequest,
    registry: Option<&ToolRegistry>,
    cancelled: &AtomicBool,
    run_provider: &mut RunProvider<'_>,
) -> Result<Vec<ProviderChatStreamEvent>, ToolRoundTripError> {
    let started = std::time::Instant::now();
    tracing::info!(
        target: "axiom.ai_diag",
        event = "orchestration_started",
        model = %request.model,
        tools_supplied = request.tools.as_ref().is_some_and(|tools| !tools.is_empty()),
        "[AI-DIAG]"
    );
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err(ToolRoundTripError::Cancelled);
    }
    let mut request = request;
    let mut total_calls = 0;
    for round in 0..MAX_TOOL_ROUNDS {
        tracing::info!(
            target: "axiom.ai_diag",
            event = "round_start",
            round = round + 1,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "[AI-DIAG]"
        );
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ToolRoundTripError::Cancelled);
        }
        let mut round_events = Vec::new();
        let round_span = tracing::info_span!(
            target: "axiom.ai_diag",
            "tool_round",
            round = round + 1
        );
        round_span
            .in_scope(|| {
                run_provider(&request, &mut |event| {
                    round_events.push(event);
                    Ok(())
                })
            })
            .map_err(ToolRoundTripError::Provider)?;
        let thinking_deltas = round_events
            .iter()
            .filter(|event| matches!(event, ProviderChatStreamEvent::ThinkingDelta(_)))
            .count();
        let content_deltas = round_events
            .iter()
            .filter(|event| matches!(event, ProviderChatStreamEvent::ContentDelta(_)))
            .count();
        let observed_done = round_events
            .iter()
            .any(|event| matches!(event, ProviderChatStreamEvent::Done));
        tracing::info!(
            target: "axiom.ai_diag",
            event = "provider_complete",
            round = round + 1,
            elapsed_ms = started.elapsed().as_millis() as u64,
            thinking_deltas,
            content_deltas,
            tool_calls = round_events
                .iter()
                .filter(|event| matches!(event, ProviderChatStreamEvent::ToolCall(_)))
                .count(),
            done = observed_done,
            "[AI-DIAG]"
        );
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ToolRoundTripError::Cancelled);
        }

        let calls: Vec<_> = round_events
            .iter()
            .filter_map(|event| match event {
                ProviderChatStreamEvent::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect();
        if calls.is_empty() {
            tracing::info!(
                target: "axiom.ai_diag",
                event = "orchestration_finished",
                outcome = "final_response",
                round = round + 1,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "[AI-DIAG]"
            );
            return Ok(round_events);
        }
        if total_calls + calls.len() > MAX_TOOL_CALLS {
            return Err(ToolRoundTripError::ToolCallLimit);
        }
        let registry = registry
            .ok_or_else(|| ToolRoundTripError::Tool("read-only tools unavailable".into()))?;
        total_calls += calls.len();

        let assistant = ProviderChatMessage {
            role: axiom_ai_provider::ChatRole::Assistant,
            content: String::new(),
            tool_call_id: None,
            tool_calls: calls.clone(),
        };
        let mut messages = request.messages;
        messages.push(assistant);
        for call in calls {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(ToolRoundTripError::Cancelled);
            }
            let tool_started = std::time::Instant::now();
            tracing::info!(
                target: "axiom.ai_diag",
                event = "tool_start",
                round = round + 1,
                tool = %call.name,
                "[AI-DIAG]"
            );
            let result = execute_call(registry, &call, cancelled);
            match &result.result {
                Ok(output) => tracing::info!(
                    target: "axiom.ai_diag",
                    event = "tool_complete",
                    round = round + 1,
                    tool = %call.name,
                    status = "success",
                    elapsed_ms = tool_started.elapsed().as_millis() as u64,
                    bytes = output.metadata.bytes,
                    source_bytes = output.metadata.source_bytes.unwrap_or(output.metadata.bytes),
                    tool_content_bytes = output.metadata.bytes,
                    tool_truncated = output.metadata.truncated,
                    truncated = output.metadata.truncated,
                    "[AI-DIAG]"
                ),
                Err(_) => tracing::warn!(
                    target: "axiom.ai_diag",
                    event = "tool_complete",
                    round = round + 1,
                    tool = %call.name,
                    status = "error",
                    elapsed_ms = tool_started.elapsed().as_millis() as u64,
                    "[AI-DIAG]"
                ),
            }
            messages.push(ProviderChatMessage {
                role: axiom_ai_provider::ChatRole::Tool,
                content: tool_result_content(&result, &call.name),
                tool_call_id: call.id,
                tool_calls: Vec::new(),
            });
        }
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ToolRoundTripError::Cancelled);
        }
        let next_request = ProviderChatRequest {
            messages,
            ..request
        };
        if round + 1 == MAX_TOOL_ROUNDS {
            tracing::warn!(
                target: "axiom.ai_diag",
                event = "tool_budget_exhausted",
                round = round + 1,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "[AI-DIAG]"
            );
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(ToolRoundTripError::Cancelled);
            }
            let synthesis_request = ProviderChatRequest {
                tools: None,
                ..next_request
            };
            tracing::info!(
                target: "axiom.ai_diag",
                event = "final_synthesis_started",
                tools_supplied = false,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "[AI-DIAG]"
            );
            let synthesis_span = tracing::info_span!(
                target: "axiom.ai_diag",
                "final_synthesis",
                tools_supplied = false
            );
            let mut synthesis_events = Vec::new();
            synthesis_span
                .in_scope(|| {
                    run_provider(&synthesis_request, &mut |event| {
                        synthesis_events.push(event);
                        Ok(())
                    })
                })
                .map_err(ToolRoundTripError::Provider)?;
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(ToolRoundTripError::Cancelled);
            }
            let synthesis_tool_call = synthesis_events
                .iter()
                .any(|event| matches!(event, ProviderChatStreamEvent::ToolCall(_)));
            tracing::info!(
                target: "axiom.ai_diag",
                event = "final_synthesis_finished",
                tools_supplied = false,
                tool_calls = synthesis_tool_call,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "[AI-DIAG]"
            );
            if synthesis_tool_call {
                return Err(ToolRoundTripError::ToolRoundLimit);
            }
            return Ok(synthesis_events);
        }
        tracing::info!(
            target: "axiom.ai_diag",
            event = "continue",
            next_round = round + 2,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "[AI-DIAG]"
        );
        request = next_request;
    }
    Err(ToolRoundTripError::ToolRoundLimit)
}

fn execute_call(
    registry: &ToolRegistry,
    call: &ProviderToolCall,
    cancelled: &AtomicBool,
) -> ToolResult {
    if call.name == "fetch_url" {
        let Some(object) = call.arguments.as_object() else {
            return ToolResult {
                tool: ToolName::FetchUrl,
                result: Err(super::tools::ToolError::InvalidArguments(
                    "arguments must be an object".into(),
                )),
            };
        };
        let Some(url) = object.get("url").and_then(|value| value.as_str()) else {
            return ToolResult {
                tool: ToolName::FetchUrl,
                result: Err(super::tools::ToolError::InvalidArguments(
                    "url must be a string".into(),
                )),
            };
        };
        if registry.kind(&ToolName::FetchUrl) != Some(ToolKind::ReadOnly) {
            return ToolResult {
                tool: ToolName::FetchUrl,
                result: Err(super::tools::ToolError::UnknownTool("fetch_url".into())),
            };
        }
        return registry.execute_with_cancel(
            ToolRequest {
                name: ToolName::FetchUrl,
                arguments: ToolArguments::FetchUrl { url: url.into() },
            },
            || cancelled.load(std::sync::atomic::Ordering::Acquire),
        );
    }
    if call.name != "read_file" {
        if call.name == "list_directory" {
            let Some(object) = call.arguments.as_object() else {
                return ToolResult {
                    tool: ToolName::ListDirectory,
                    result: Err(super::tools::ToolError::InvalidArguments(
                        "arguments must be an object".into(),
                    )),
                };
            };
            let Some(path) = object.get("path").and_then(|value| value.as_str()) else {
                return ToolResult {
                    tool: ToolName::ListDirectory,
                    result: Err(super::tools::ToolError::InvalidArguments(
                        "path must be a string".into(),
                    )),
                };
            };
            if registry.kind(&ToolName::ListDirectory) != Some(ToolKind::ReadOnly) {
                return ToolResult {
                    tool: ToolName::ListDirectory,
                    result: Err(super::tools::ToolError::UnknownTool(
                        "list_directory".into(),
                    )),
                };
            }
            return registry.execute_with_cancel(
                ToolRequest {
                    name: ToolName::ListDirectory,
                    arguments: ToolArguments::ListDirectory { path: path.into() },
                },
                || cancelled.load(std::sync::atomic::Ordering::Acquire),
            );
        }
        return registry.execute(ToolRequest {
            name: ToolName::Unknown(call.name.clone()),
            arguments: ToolArguments::ReadFile {
                path: String::new(),
                range: None,
            },
        });
    }
    if registry.kind(&ToolName::ReadFile) != Some(ToolKind::ReadOnly) {
        return ToolResult {
            tool: ToolName::ReadFile,
            result: Err(super::tools::ToolError::UnknownTool("read_file".into())),
        };
    }
    let Some(object) = call.arguments.as_object() else {
        return ToolResult {
            tool: ToolName::ReadFile,
            result: Err(super::tools::ToolError::InvalidArguments(
                "arguments must be an object".into(),
            )),
        };
    };
    let Some(path) = object.get("path").and_then(|value| value.as_str()) else {
        return ToolResult {
            tool: ToolName::ReadFile,
            result: Err(super::tools::ToolError::InvalidArguments(
                "path must be a string".into(),
            )),
        };
    };
    let range = match (object.get("start_line"), object.get("end_line")) {
        (None, None) => None,
        (Some(start), Some(end)) => match (start.as_u64(), end.as_u64()) {
            (Some(start_line), Some(end_line)) => {
                Some(axiom_project::project_read::ReadFileRange {
                    start_line: start_line as usize,
                    end_line: end_line as usize,
                })
            }
            _ => {
                return ToolResult {
                    tool: ToolName::ReadFile,
                    result: Err(super::tools::ToolError::InvalidArguments(
                        "line range must be integers".into(),
                    )),
                };
            }
        },
        _ => {
            return ToolResult {
                tool: ToolName::ReadFile,
                result: Err(super::tools::ToolError::InvalidArguments(
                    "start_line and end_line must be provided together".into(),
                )),
            };
        }
    };
    registry.execute_with_cancel(
        ToolRequest {
            name: ToolName::ReadFile,
            arguments: ToolArguments::ReadFile {
                path: path.into(),
                range,
            },
        },
        || cancelled.load(std::sync::atomic::Ordering::Acquire),
    )
}

fn tool_result_content(result: &ToolResult, tool_name: &str) -> String {
    match &result.result {
        Ok(output) if tool_name == "fetch_url" => {
            if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&output.content) {
                value["tool"] = json!(tool_name);
                return value.to_string();
            }
            json!({
                "tool": tool_name,
                "path": output.metadata.path,
                "content": output.content,
            })
            .to_string()
        }
        Ok(output) => json!({
            "tool": tool_name,
            "path": output.metadata.path,
            "content": output.content,
            "metadata": {
                "bytes": output.metadata.bytes,
                "range": output.metadata.range.as_ref().map(|range| json!({
                    "start_line": range.start_line,
                    "end_line": range.end_line,
                }))
            }
        })
        .to_string(),
        Err(error) => json!({
            "error": format_tool_error(error),
            "tool": tool_name
        })
        .to_string(),
    }
}

fn format_tool_error(error: &super::tools::ToolError) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::{MAX_TOOL_CONTENT_BYTES, ToolMetadata, ToolOutput};
    use axiom_ai_provider::{ChatRole, ProviderToolCall};
    use axiom_project::project_read::ProjectReadCapability;
    use std::fs;
    use tempfile::tempdir;

    fn request() -> ProviderChatRequest {
        ProviderChatRequest {
            model: "fake".into(),
            messages: vec![ProviderChatMessage {
                role: ChatRole::User,
                content: "read README".into(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            think: None,
            tools: Some(vec![read_file_definition()]),
        }
    }

    #[test]
    fn fetch_url_definition_requires_only_public_url() {
        let definition = fetch_url_definition();
        assert_eq!(definition.name, "fetch_url");
        assert_eq!(definition.parameters["required"], json!(["url"]));
        assert_eq!(definition.parameters["additionalProperties"], json!(false));
        assert!(definition.description.contains("Does not search"));
    }

    #[test]
    fn sentinel_round_trip_puts_file_in_second_request() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "AXIOM_TOOL_SENTINEL_C3_91F4").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            requests.push(request.clone());
            if requests.len() == 1 {
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some("1".into()),
                    name: "read_file".into(),
                    arguments: json!({"path":"README.md"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
            } else {
                events(ProviderChatStreamEvent::ContentDelta(
                    "sentinel recebido".into(),
                ))?;
                events(ProviderChatStreamEvent::Done)?;
            }
            Ok(())
        };
        let events =
            run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run).unwrap();
        assert!(matches!(
            events[0],
            ProviderChatStreamEvent::ContentDelta(_)
        ));
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|message| message.role == ChatRole::Tool
                    && message.content.contains("AXIOM_TOOL_SENTINEL_C3_91F4"))
        );
        assert_eq!(requests[1].messages[1].tool_calls.len(), 1);
    }

    #[test]
    fn list_directory_round_trip_puts_entries_in_second_request() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::write(dir.path().join("App/Controller.php"), "<?php").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            requests.push(request.clone());
            if requests.len() == 1 {
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some("directory-1".into()),
                    name: "list_directory".into(),
                    arguments: json!({"path": "App"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
            } else {
                events(ProviderChatStreamEvent::ContentDelta(
                    "found Controller.php".into(),
                ))?;
                events(ProviderChatStreamEvent::Done)?;
            }
            Ok(())
        };
        run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run).unwrap();
        assert!(requests[1].messages.iter().any(|message| {
            message.role == ChatRole::Tool && message.content.contains("Controller.php")
        }));
    }

    #[test]
    fn multiple_tools_preserve_call_and_result_order() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::write(dir.path().join("App/Controller.php"), "controller sentinel").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            requests.push(request.clone());
            if requests.len() == 1 {
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some("list".into()),
                    name: "list_directory".into(),
                    arguments: json!({"path": "App"}),
                }))?;
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some("read".into()),
                    name: "read_file".into(),
                    arguments: json!({"path": "App/Controller.php"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
            } else {
                events(ProviderChatStreamEvent::ContentDelta("done".into()))?;
                events(ProviderChatStreamEvent::Done)?;
            }
            Ok(())
        };
        run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run).unwrap();
        let tool_messages: Vec<_> = requests[1]
            .messages
            .iter()
            .filter(|message| message.role == ChatRole::Tool)
            .collect();
        assert_eq!(tool_messages.len(), 2);
        assert!(tool_messages[0].content.contains("Controller.php"));
        assert!(tool_messages[1].content.contains("controller sentinel"));
    }

    #[test]
    fn nested_directory_navigation_uses_multiple_bounded_rounds() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/App")).unwrap();
        fs::write(dir.path().join("src/App/FileStone.php"), "stone").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            requests.push(request.clone());
            match requests.len() {
                1 => {
                    events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                        id: Some("src".into()),
                        name: "list_directory".into(),
                        arguments: json!({"path": "src"}),
                    }))?;
                    events(ProviderChatStreamEvent::Done)?;
                }
                2 => {
                    events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                        id: Some("src-app".into()),
                        name: "list_directory".into(),
                        arguments: json!({"path": "src/App"}),
                    }))?;
                    events(ProviderChatStreamEvent::Done)?;
                }
                _ => {
                    events(ProviderChatStreamEvent::ContentDelta(
                        "Found src/App/FileStone.php".into(),
                    ))?;
                    events(ProviderChatStreamEvent::Done)?;
                }
            }
            Ok(())
        };
        let events = run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run)
            .expect("nested navigation should complete");
        assert!(matches!(
            events.first(),
            Some(ProviderChatStreamEvent::ContentDelta(content))
                if content.contains("FileStone.php")
        ));
        assert_eq!(requests.len(), 3);
        assert!(requests[1].messages.iter().any(|message| {
            message.role == ChatRole::Tool
                && message.content.contains("\"tool\":\"list_directory\"")
                && message.content.contains("\\\"path\\\":\\\"src\\\"")
                && message.content.contains("App")
        }));
        assert!(requests[2].messages.iter().any(|message| {
            message.role == ChatRole::Tool
                && message.content.contains("\\\"path\\\":\\\"src/App\\\"")
                && message.content.contains("FileStone.php")
        }));
    }

    #[test]
    fn read_file_result_keeps_read_file_tool_label() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/App")).unwrap();
        fs::write(dir.path().join("src/App/FileStone.php"), "stone").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            requests.push(request.clone());
            if requests.len() == 1 {
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some("read".into()),
                    name: "read_file".into(),
                    arguments: json!({"path": "src/App/FileStone.php"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
            } else {
                events(ProviderChatStreamEvent::ContentDelta("done".into()))?;
                events(ProviderChatStreamEvent::Done)?;
            }
            Ok(())
        };
        run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run).unwrap();
        assert!(requests[1].messages.iter().any(|message| {
            message.role == ChatRole::Tool
                && message.content.contains("\"tool\":\"read_file\"")
                && message.content.contains("FileStone.php")
        }));
    }

    #[test]
    fn fetch_url_tool_call_returns_controlled_result_and_final_response() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            requests.push(request.clone());
            if requests.len() == 1 {
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some("fetch".into()),
                    name: "fetch_url".into(),
                    arguments: json!({"url": "http://127.0.0.1/"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
            } else {
                events(ProviderChatStreamEvent::ContentDelta(
                    "blocked safely".into(),
                ))?;
                events(ProviderChatStreamEvent::Done)?;
            }
            Ok(())
        };
        let events = run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run)
            .expect("tool failure should be returned to provider");
        assert!(matches!(
            events[0],
            ProviderChatStreamEvent::ContentDelta(_)
        ));
        assert!(requests[1].messages.iter().any(|message| {
            message.role == ChatRole::Tool
                && message.content.contains("\"tool\":\"fetch_url\"")
                && message.content.contains("BlockedAddress")
        }));
    }

    #[test]
    fn fetch_url_tool_result_wrapper_does_not_duplicate_content() {
        let content = "x".repeat(MAX_TOOL_CONTENT_BYTES);
        let result = ToolResult {
            tool: ToolName::FetchUrl,
            result: Ok(ToolOutput {
                content: json!({
                    "tool": "fetch_url",
                    "url": "https://example.test",
                    "final_url": "https://example.test",
                    "content_type": "text/html",
                    "content": content,
                    "truncated": false,
                })
                .to_string(),
                metadata: ToolMetadata {
                    path: "https://example.test".into(),
                    bytes: MAX_TOOL_CONTENT_BYTES,
                    range: None,
                    source_bytes: Some(MAX_TOOL_CONTENT_BYTES),
                    truncated: false,
                },
            }),
        };
        let serialized = tool_result_content(&result, "fetch_url");
        let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            value["content"].as_str().unwrap().len(),
            MAX_TOOL_CONTENT_BYTES
        );
        assert!(serialized.len() < MAX_TOOL_CONTENT_BYTES + 512);
    }

    #[test]
    fn cancellation_before_second_request_does_not_call_provider_again() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "sentinel").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let mut run = |_: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            calls += 1;
            events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                id: None,
                name: "read_file".into(),
                arguments: json!({"path":"README.md"}),
            }))?;
            events(ProviderChatStreamEvent::Done)?;
            cancelled.store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        };
        assert_eq!(
            run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run),
            Err(ToolRoundTripError::Cancelled)
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn unknown_tool_is_returned_as_controlled_tool_result() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            requests.push(request.clone());
            if requests.len() == 1 {
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: None,
                    name: "delete_file".into(),
                    arguments: json!({"path":"x"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
            } else {
                events(ProviderChatStreamEvent::ContentDelta("explained".into()))?;
                events(ProviderChatStreamEvent::Done)?;
            }
            Ok(())
        };
        run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run).unwrap();
        assert!(
            requests[1]
                .messages
                .last()
                .unwrap()
                .content
                .contains("UnknownTool")
        );
    }

    #[test]
    fn tool_round_budget_runs_final_synthesis() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("proof.txt"), "round-4-result").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let mut requests = Vec::new();
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            calls += 1;
            requests.push(request.clone());
            if request.tools.is_none() {
                events(ProviderChatStreamEvent::ContentDelta(
                    "synthesized from tool results".into(),
                ))?;
                events(ProviderChatStreamEvent::Done)?;
                return Ok(());
            }
            events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                id: Some(format!("call-{calls}")),
                name: "read_file".into(),
                arguments: json!({"path":"proof.txt"}),
            }))?;
            events(ProviderChatStreamEvent::Done)?;
            Ok(())
        };
        let events = run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run)
            .expect("final synthesis should consume the bounded tool budget");
        assert!(matches!(
            events.first(),
            Some(ProviderChatStreamEvent::ContentDelta(content))
                if content == "synthesized from tool results"
        ));
        assert_eq!(calls, MAX_TOOL_ROUNDS + 1);
        let synthesis = requests.last().expect("request 5 should be synthesis");
        assert!(synthesis.tools.is_none());
        assert_eq!(synthesis.messages.len(), 1 + MAX_TOOL_ROUNDS * 2);
        for (index, pair) in synthesis.messages[1..].chunks_exact(2).enumerate() {
            let expected_id = format!("call-{}", index + 1);
            assert_eq!(pair[0].role, ChatRole::Assistant);
            assert_eq!(
                pair[0].tool_calls[0].id.as_deref(),
                Some(expected_id.as_str())
            );
            assert_eq!(pair[1].role, ChatRole::Tool);
            assert_eq!(pair[1].tool_call_id.as_deref(), Some(expected_id.as_str()));
            assert!(pair[1].content.contains("round-4-result"));
        }
    }

    #[test]
    fn final_synthesis_cannot_execute_another_tool() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut synthesis_requests = 0;
        let mut run = |request: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            if request.tools.is_none() {
                synthesis_requests += 1;
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: None,
                    name: "read_file".into(),
                    arguments: json!({"path": "x"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
                return Ok(());
            }
            events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                id: None,
                name: "read_file".into(),
                arguments: json!({"path": "x"}),
            }))?;
            events(ProviderChatStreamEvent::Done)?;
            Ok(())
        };
        assert_eq!(
            run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run),
            Err(ToolRoundTripError::ToolRoundLimit)
        );
        assert_eq!(synthesis_requests, 1);
    }

    #[test]
    fn cancellation_after_last_tool_round_skips_final_synthesis() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let mut run = |_: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            calls += 1;
            events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                id: None,
                name: "read_file".into(),
                arguments: json!({"path":"x"}),
            }))?;
            events(ProviderChatStreamEvent::Done)?;
            if calls == MAX_TOOL_ROUNDS {
                cancelled.store(true, std::sync::atomic::Ordering::Release);
            }
            Ok(())
        };
        assert_eq!(
            run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run),
            Err(ToolRoundTripError::Cancelled)
        );
        assert_eq!(calls, MAX_TOOL_ROUNDS);
    }

    #[test]
    fn tool_call_limit_stops_a_round_with_too_many_calls() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(dir.path()).unwrap(),
            axiom_project::project_directory::ProjectDirectoryCapability::new(dir.path()).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let mut run = |_: &ProviderChatRequest,
                       events: &mut dyn FnMut(
            ProviderChatStreamEvent,
        ) -> Result<(), ProviderError>| {
            for index in 0..=MAX_TOOL_CALLS {
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: Some(index.to_string()),
                    name: "list_directory".into(),
                    arguments: json!({"path": "."}),
                }))?;
            }
            events(ProviderChatStreamEvent::Done)?;
            Ok(())
        };
        assert_eq!(
            run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run),
            Err(ToolRoundTripError::ToolCallLimit)
        );
    }

    #[test]
    fn request_specific_tool_errors_never_use_provider_unavailable_message() {
        assert_eq!(
            user_message(&ToolRoundTripError::Cancelled),
            "Generation cancelled"
        );
        assert_eq!(
            user_message(&ToolRoundTripError::Tool("bad arguments".into())),
            "Tool request failed"
        );
        assert_eq!(
            user_message(&ToolRoundTripError::ToolRoundLimit),
            "Tool round limit reached"
        );
        assert_eq!(
            user_message(&ToolRoundTripError::ToolCallLimit),
            "Tool call limit reached"
        );
        assert_eq!(
            user_message(&ToolRoundTripError::Provider(
                ProviderError::InvalidResponse
            )),
            "Invalid Ollama response"
        );
        assert_eq!(
            user_message(&ToolRoundTripError::Provider(
                ProviderError::ConnectionRefused
            )),
            "Connection refused"
        );
    }
}
