//! Minimal, single-round native tool orchestration for chat.

use super::tools::{ToolArguments, ToolKind, ToolName, ToolRegistry, ToolRequest, ToolResult};
use axiom_ai_provider::{
    ProviderChatMessage, ProviderChatRequest, ProviderChatStreamEvent, ProviderToolCall,
    ProviderToolDefinition,
};
use serde_json::json;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolRoundTripError {
    Provider(String),
    Tool(String),
    Cancelled,
    SecondToolCall,
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

type RunProvider<'a> = dyn FnMut(
        &ProviderChatRequest,
        &mut dyn FnMut(ProviderChatStreamEvent) -> Result<(), String>,
    ) -> Result<(), String>
    + 'a;

/// Runs at most one native tool round and returns the events that should be
/// applied to the existing assistant placeholder.
pub(crate) fn run_read_file_round_trip(
    request: ProviderChatRequest,
    registry: Option<&ToolRegistry>,
    cancelled: &AtomicBool,
    run_provider: &mut RunProvider<'_>,
) -> Result<Vec<ProviderChatStreamEvent>, ToolRoundTripError> {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err(ToolRoundTripError::Cancelled);
    }
    let mut first_events = Vec::new();
    run_provider(&request, &mut |event| {
        first_events.push(event);
        Ok(())
    })
    .map_err(ToolRoundTripError::Provider)?;

    let calls: Vec<_> = first_events
        .iter()
        .filter_map(|event| match event {
            ProviderChatStreamEvent::ToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .collect();
    if calls.is_empty() {
        return Ok(first_events);
    }
    let registry =
        registry.ok_or_else(|| ToolRoundTripError::Tool("read_file unavailable".into()))?;
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err(ToolRoundTripError::Cancelled);
    }

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
        let result = execute_call(registry, &call);
        messages.push(ProviderChatMessage {
            role: axiom_ai_provider::ChatRole::Tool,
            content: tool_result_content(&result),
            tool_call_id: call.id,
            tool_calls: Vec::new(),
        });
    }

    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err(ToolRoundTripError::Cancelled);
    }
    let second_request = ProviderChatRequest {
        messages,
        ..request
    };
    let mut final_events = Vec::new();
    let mut second_tool_call = false;
    run_provider(&second_request, &mut |event| {
        if matches!(event, ProviderChatStreamEvent::ToolCall(_)) {
            second_tool_call = true;
            return Ok(());
        }
        final_events.push(event);
        Ok(())
    })
    .map_err(ToolRoundTripError::Provider)?;
    if second_tool_call {
        return Err(ToolRoundTripError::SecondToolCall);
    }
    Ok(final_events)
}

fn execute_call(registry: &ToolRegistry, call: &ProviderToolCall) -> ToolResult {
    if call.name != "read_file" {
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
    registry.execute(ToolRequest {
        name: ToolName::ReadFile,
        arguments: ToolArguments::ReadFile {
            path: path.into(),
            range,
        },
    })
}

fn tool_result_content(result: &ToolResult) -> String {
    match &result.result {
        Ok(output) => json!({
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
            "tool": "read_file"
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
    fn sentinel_round_trip_puts_file_in_second_request() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "AXIOM_TOOL_SENTINEL_C3_91F4").unwrap();
        let registry = ToolRegistry::new(ProjectReadCapability::new(dir.path()).unwrap());
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run =
            |request: &ProviderChatRequest,
             events: &mut dyn FnMut(ProviderChatStreamEvent) -> Result<(), String>| {
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
    fn cancellation_before_second_request_does_not_call_provider_again() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "sentinel").unwrap();
        let registry = ToolRegistry::new(ProjectReadCapability::new(dir.path()).unwrap());
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let mut run =
            |_: &ProviderChatRequest,
             events: &mut dyn FnMut(ProviderChatStreamEvent) -> Result<(), String>| {
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
        let registry = ToolRegistry::new(ProjectReadCapability::new(dir.path()).unwrap());
        let cancelled = AtomicBool::new(false);
        let mut requests = Vec::new();
        let mut run =
            |request: &ProviderChatRequest,
             events: &mut dyn FnMut(ProviderChatStreamEvent) -> Result<(), String>| {
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
    fn second_tool_call_is_rejected_without_looping() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::new(ProjectReadCapability::new(dir.path()).unwrap());
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let mut run =
            |_: &ProviderChatRequest,
             events: &mut dyn FnMut(ProviderChatStreamEvent) -> Result<(), String>| {
                calls += 1;
                events(ProviderChatStreamEvent::ToolCall(ProviderToolCall {
                    id: None,
                    name: "read_file".into(),
                    arguments: json!({"path":"x"}),
                }))?;
                events(ProviderChatStreamEvent::Done)?;
                Ok(())
            };
        assert_eq!(
            run_read_file_round_trip(request(), Some(&registry), &cancelled, &mut run),
            Err(ToolRoundTripError::SecondToolCall)
        );
        assert_eq!(calls, 2);
    }
}
