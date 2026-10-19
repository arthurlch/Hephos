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
    }
}
