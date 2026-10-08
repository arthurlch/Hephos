use hephos::agent::{Completion, Message, Model, TextStream};
use hephos::prelude::*;

/// Offline `Model` for a deterministic workflow run (as in example-agent).
pub struct ScriptedModel;

impl ScriptedModel {
    pub fn canned() -> Self {
        ScriptedModel
    }
}

impl Model for ScriptedModel {
    async fn complete(&self, _request: Completion) -> Result<Message> {
        Ok(Message::assistant(
            "Welcome aboard — your account is ready.",
        ))
    }

    async fn stream(&self, _request: Completion) -> Result<TextStream> {
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            "Welcome".to_string()
        )])))
    }
}
