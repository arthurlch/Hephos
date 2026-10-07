//! Model providers.
//!
//! Each provider is a thin `reqwest` client implementing [`crate::Model`]. The
//! API key is read from the environment at construction. Nothing here is clever:
//! build a request, send it, map the response into a [`crate::Message`] or a
//! [`crate::TextStream`].

use rivet_core::{Error, Result};

use crate::message::{Message, TextStream};
use crate::model::{Completion, Model};

/// Anthropic Messages API client.
///
/// BOUNDARY: implement `complete`/`stream` against
/// `POST https://api.anthropic.com/v1/messages`. Map `tools` into the Anthropic
/// tool format, decode `tool_use` blocks into tool calls, and for `response_schema`
/// force a single output tool whose schema is the response schema.
pub struct Anthropic {
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl Anthropic {
    /// Construct from `ANTHROPIC_API_KEY`.
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| Error::invalid("ANTHROPIC_API_KEY is not set"))?;
        Ok(Anthropic {
            api_key,
            model: model.into(),
            http: reqwest::Client::new(),
        })
    }
}

impl Model for Anthropic {
    async fn complete(&self, _request: Completion) -> Result<Message> {
        // BOUNDARY: POST to the Messages API, parse the response into a Message
        // (text) or tool calls. See module docs.
        let _ = (&self.api_key, &self.model, &self.http);
        Err(Error::internal("Anthropic::complete not yet implemented"))
    }

    async fn stream(&self, _request: Completion) -> Result<TextStream> {
        // BOUNDARY: open the SSE stream and yield text deltas as `Ok(String)`.
        Err(Error::internal("Anthropic::stream not yet implemented"))
    }
}
