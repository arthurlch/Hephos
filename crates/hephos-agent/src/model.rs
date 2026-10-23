use hephos_core::Result;

use crate::message::{Message, TextStream};
use crate::tool::ToolSchema;

/// A single request to a model.
#[derive(Debug, Clone)]
pub struct Completion {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub max_tokens: u32,
    /// When set, the model is constrained to emit a JSON value matching this
    /// schema. Used by [`crate::Agent::run_typed`].
    pub response_schema: Option<serde_json::Value>,
}

impl Completion {
    /// Construct a basic completion request with messages.
    pub fn new(messages: Vec<Message>) -> Self {
        Completion {
            system: String::new(),
            messages,
            tools: Vec::new(),
            max_tokens: 4096,
            response_schema: None,
        }
    }

    /// Set the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = system.into();
        self
    }

    /// Set the tool schemas.
    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    /// Set the token limit.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

/// A model provider.
///
/// The one abstraction over LLM vendors. Implementations are thin HTTP clients
/// (see [`crate::providers`]). Keep this trait minimal: complete and stream. Tool
/// calling, retries, and turn loops live in [`crate::Agent`], not here, so every
/// provider implementation stays small and obviously correct.
pub trait Model: Send + Sync + 'static {
    /// One round trip. Returns the model's message, which may be a final answer
    /// or a request to call tools.
    fn complete(
        &self,
        request: Completion,
    ) -> impl std::future::Future<Output = Result<Message>> + Send;

    /// One round trip, streamed as text deltas. Tool-calling turns do not stream;
    /// only the final textual answer does.
    fn stream(
        &self,
        request: Completion,
    ) -> impl std::future::Future<Output = Result<TextStream>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_construction_and_cloning() {
        let completion = Completion {
            system: "you are a helpful assistant".into(),
            messages: vec![Message::user("hello"), Message::assistant("world")],
            tools: vec![ToolSchema::new(
                "dummy",
                "dummy tool",
                serde_json::json!({ "type": "object" }),
            )],
            max_tokens: 1024,
            response_schema: Some(serde_json::json!({ "type": "string" })),
        };

        assert_eq!(completion.system, "you are a helpful assistant");
        assert_eq!(completion.messages.len(), 2);
        assert_eq!(completion.tools.len(), 1);
        assert_eq!(completion.max_tokens, 1024);
        assert!(completion.response_schema.is_some());

        let cloned = completion.clone();
        assert_eq!(cloned.system, completion.system);
        assert_eq!(cloned.max_tokens, completion.max_tokens);

        let debug_str = format!("{:?}", completion);
        assert!(debug_str.contains("you are a helpful assistant"));

        let basic = Completion::new(vec![Message::user("ping")])
            .with_system("sys")
            .with_max_tokens(2048);
        assert_eq!(basic.system, "sys");
        assert_eq!(basic.messages.len(), 1);
        assert_eq!(basic.max_tokens, 2048);
        assert!(basic.tools.is_empty());
    }
}
