use futures::stream::BoxStream;
use rivet_core::Result;
use serde::{Deserialize, Serialize};

/// A conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
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
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// A stream of text deltas from a model. Backed by the provider's SSE stream and
/// bounded by the agent's limits. Dropping the stream cancels generation.
pub type TextStream = BoxStream<'static, Result<String>>;
