//! Model providers.
//!
//! Each provider is a thin `reqwest` client implementing [`crate::Model`]. The
//! API key is read from the environment at construction. Nothing here is clever:
//! build a request, send it, map the response into a [`crate::Message`] or a
//! [`crate::TextStream`].

pub mod anthropic;
pub(crate) mod common;
pub mod openai;

pub use anthropic::Anthropic;
pub use openai::OpenAI;

#[cfg(test)]
mod tests {
    use super::anthropic::{build_anthropic_payload, extract_anthropic_sse_delta};
    use super::openai::{build_openai_payload, extract_openai_sse_delta};
    use super::*;
    use crate::message::Message;
    use crate::model::Completion;
    use crate::tool::{ToolCall, ToolSchema};

    #[test]
    fn anthropic_sse_delta_extraction() {
        let line = r#"data: {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hello "}}"#;
        assert_eq!(extract_anthropic_sse_delta(line), Some("hello ".into()));

        let irrelevant = r#"data: {"type": "message_start"}"#;
        assert_eq!(extract_anthropic_sse_delta(irrelevant), None);
    }

    #[test]
    fn openai_sse_delta_extraction() {
        let line = r#"data: {"id":"1","choices":[{"index":0,"delta":{"content":"world"},"finish_reason":null}]}"#;
        assert_eq!(extract_openai_sse_delta(line), Some("world".into()));

        let done = "data: [DONE]";
        assert_eq!(extract_openai_sse_delta(done), None);
    }

    #[test]
    fn anthropic_payload_structure() {
        let completion = Completion {
            system: "helpful assistant".into(),
            messages: vec![
                Message::user("hi"),
                Message::assistant_with_tools(
                    "calling",
                    vec![ToolCall::with_id(
                        "t1",
                        "search",
                        serde_json::json!({"q": "test"}),
                    )],
                ),
                Message::tool_result("t1", "result"),
            ],
            tools: vec![ToolSchema::new(
                "search",
                "search",
                serde_json::json!({"type": "object"}),
            )],
            max_tokens: 1024,
            response_schema: None,
        };

        let payload = build_anthropic_payload("claude-3-5-sonnet", &completion, false).unwrap();
        assert_eq!(payload.model, "claude-3-5-sonnet");
        assert_eq!(payload.system, Some("helpful assistant"));
        assert_eq!(payload.messages.len(), 3);
        assert_eq!(payload.tools.len(), 1);
    }

    #[test]
    fn openai_payload_structure() {
        let completion = Completion {
            system: "system prompt".into(),
            messages: vec![Message::user("hello")],
            tools: vec![],
            max_tokens: 512,
            response_schema: Some(serde_json::json!({"type": "object"})),
        };

        let payload = build_openai_payload("gpt-4o", &completion, false).unwrap();
        assert_eq!(payload.model, "gpt-4o");
        assert!(payload.response_format.is_some());
        assert_eq!(payload.messages.len(), 2); // system + user
    }

    #[test]
    fn providers_api_key_redaction_in_debug() {
        let anthropic = Anthropic::new("sk-ant-sensitive-secret-token", "claude-3-5-sonnet");
        let debug_anthropic = format!("{anthropic:?}");
        assert!(!debug_anthropic.contains("sk-ant-sensitive-secret-token"));
        assert!(debug_anthropic.contains("[REDACTED]"));

        let openai = OpenAI::new("sk-proj-sensitive-secret-token", "gpt-4o");
        let debug_openai = format!("{openai:?}");
        assert!(!debug_openai.contains("sk-proj-sensitive-secret-token"));
        assert!(debug_openai.contains("[REDACTED]"));
    }
}
