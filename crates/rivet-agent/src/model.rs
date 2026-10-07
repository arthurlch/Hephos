use rivet_core::Result;

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
