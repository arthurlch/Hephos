use futures::stream::BoxStream;
use hephos_core::Result;
use serde::{Deserialize, Serialize};

use crate::tool::ToolCall;

/// A conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Message {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// A stream of text deltas from a model. Backed by the provider's SSE stream and
/// bounded by the agent's limits. Dropping the stream cancels generation.
pub type TextStream = BoxStream<'static, Result<String>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constructors_and_serialization() {
        let user = Message::user("hello");
        assert_eq!(user.role, Role::User);
        assert_eq!(user.content, "hello");
        assert!(user.tool_calls.is_empty());
        assert!(user.tool_call_id.is_none());

        let assistant = Message::assistant("world");
        assert_eq!(assistant.role, Role::Assistant);
        assert_eq!(assistant.content, "world");

        let json = serde_json::to_string(&user).unwrap();
        assert!(json.contains(r#""role":"user""#));
        assert!(json.contains(r#""content":"hello""#));
        assert!(!json.contains("tool_calls"));

        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.role, Role::User);
        assert_eq!(deserialized.content, "hello");
        assert!(deserialized.tool_calls.is_empty());

        let tool_call = ToolCall::with_id("call_1", "calc", serde_json::json!({"x": 1}));
        let assistant_tools =
            Message::assistant_with_tools("calling calc", vec![tool_call.clone()]);
        assert_eq!(assistant_tools.tool_calls.len(), 1);
        assert_eq!(assistant_tools.tool_calls[0].name, "calc");

        let tool_msg = Message::tool_result("call_1", "2");
        assert_eq!(tool_msg.role, Role::Tool);
        assert_eq!(tool_msg.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(tool_msg.content, "2");
    }
}
