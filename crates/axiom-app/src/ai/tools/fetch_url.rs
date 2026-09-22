use axiom_web::{FetchUrlCapability, FetchUrlError, FetchUrlRequest};
use serde_json::json;

use super::{MAX_TOOL_CONTENT_BYTES, ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

#[derive(Clone, Debug)]
pub(crate) struct FetchUrlTool {
    capability: FetchUrlCapability,
}

impl FetchUrlTool {
    pub(crate) fn new(capability: FetchUrlCapability) -> Self {
        Self { capability }
    }

    pub(crate) fn execute<F>(&self, url: String, cancelled: F) -> ToolResult
    where
        F: Fn() -> bool,
    {
        let tool = ToolName::FetchUrl;
        let request_url = url.clone();
        let result = self
            .capability
            .fetch_url_with_cancel(FetchUrlRequest { url }, cancelled)
            .map(|output| {
                let source_bytes = output.content.len();
                let (content, adapter_truncated) =
                    truncate_utf8_prefix(&output.content, MAX_TOOL_CONTENT_BYTES);
                let truncated = output.truncated || adapter_truncated;
                let bytes = content.len();
                ToolOutput {
                    content: json!({
                    "tool": "fetch_url",
                    "url": output.url,
                    "final_url": output.final_url,
                    "content_type": output.content_type,
                    "content": content,
                    "truncated": truncated,
                    })
                    .to_string(),
                    metadata: ToolMetadata {
                        path: request_url,
                        bytes,
                        range: None,
                        source_bytes: Some(source_bytes),
                        truncated,
                    },
                }
            })
            .map_err(map_error);
        ToolResult { tool, result }
    }
}

fn truncate_utf8_prefix(content: &str, max_bytes: usize) -> (&str, bool) {
    if content.len() <= max_bytes {
        return (content, false);
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    (&content[..end], true)
}

fn map_error(error: FetchUrlError) -> ToolError {
    match error {
        FetchUrlError::InvalidUrl => ToolError::InvalidUrl,
        FetchUrlError::UnsupportedScheme(scheme) => ToolError::UnsupportedScheme(scheme),
        FetchUrlError::BlockedAddress(address) => ToolError::BlockedAddress(address),
        FetchUrlError::Timeout => ToolError::Timeout,
        FetchUrlError::TooLarge { limit } => ToolError::TooLarge {
            path: String::new(),
            bytes: limit as u64 + 1,
            limit: limit as u64,
        },
        FetchUrlError::UnsupportedContentType(content_type) => {
            ToolError::UnsupportedContentType(content_type)
        }
        FetchUrlError::UnsupportedEncoding(encoding) => ToolError::UnsupportedEncoding(encoding),
        FetchUrlError::HttpStatus(status) => ToolError::HttpStatus(status),
        FetchUrlError::Network(message) => ToolError::Network(message),
        FetchUrlError::Cancelled => ToolError::Cancelled,
        FetchUrlError::RedirectLimit => ToolError::RedirectLimit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn small_and_exact_content_are_unchanged() {
        let small = "hello";
        assert_eq!(
            truncate_utf8_prefix(small, MAX_TOOL_CONTENT_BYTES),
            (small, false)
        );
        let exact = "x".repeat(MAX_TOOL_CONTENT_BYTES);
        assert_eq!(
            truncate_utf8_prefix(&exact, MAX_TOOL_CONTENT_BYTES),
            (exact.as_str(), false)
        );
    }

    #[test]
    fn oversized_content_is_utf8_safe_and_marked_truncated() {
        let content = "😀".repeat(MAX_TOOL_CONTENT_BYTES);
        let (bounded, truncated) = truncate_utf8_prefix(&content, MAX_TOOL_CONTENT_BYTES);
        assert!(truncated);
        assert!(bounded.len() <= MAX_TOOL_CONTENT_BYTES);
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
    }

    #[test]
    fn fetch_result_schema_preserves_urls_type_and_truncation_flag() {
        let source = "x".repeat(MAX_TOOL_CONTENT_BYTES + 1);
        let (content, adapter_truncated) = truncate_utf8_prefix(&source, MAX_TOOL_CONTENT_BYTES);
        let value: Value = serde_json::json!({
            "tool": "fetch_url",
            "url": "https://example.test",
            "final_url": "https://example.test/final",
            "content_type": "text/html",
            "content": content,
            "truncated": adapter_truncated,
        });
        assert_eq!(value["url"], "https://example.test");
        assert_eq!(value["final_url"], "https://example.test/final");
        assert_eq!(value["content_type"], "text/html");
        assert_eq!(value["truncated"], true);
        assert!(value["content"].as_str().unwrap().len() <= MAX_TOOL_CONTENT_BYTES);
    }
}
