use hephos::agent::{Completion, Message, Model, TextStream};
use hephos::prelude::*;

/// Offline Model implementation for deterministic runs and tests.
#[derive(Debug, Clone, Default)]
pub struct ScriptedModel;

impl ScriptedModel {
    pub fn canned() -> Self {
        ScriptedModel
    }
}

impl Model for ScriptedModel {
    async fn complete(&self, _request: Completion) -> Result<Message> {
        Ok(Message::assistant(
            "Calculation result processed successfully.",
        ))
    }

    async fn stream(&self, _request: Completion) -> Result<TextStream> {
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            "Calculation result processed successfully.".to_string(),
        )])))
    }
}
