use hephos::agent::{Completion, Message, Model, TextStream};
use hephos::prelude::*;

/// A deterministic `Model` for offline runs and tests. It inspects the request:
/// when a response schema is set (structured output), it returns canned JSON;
/// otherwise it returns canned prose.
pub struct ScriptedModel {
    prose: String,
    json: String,
}

impl ScriptedModel {
    pub fn canned() -> Self {
        ScriptedModel {
            prose: "Ada's account is active with recent sign-ins.".into(),
            json: r#"{"headline":"Ada: active","bullet_points":["recent sign-ins","no open tickets"]}"#.into(),
        }
    }
}

impl Model for ScriptedModel {
    async fn complete(&self, request: Completion) -> Result<Message> {
        let content = if request.response_schema.is_some() {
            self.json.clone()
        } else {
            self.prose.clone()
        };
        Ok(Message::assistant(content))
    }

    async fn stream(&self, _request: Completion) -> Result<TextStream> {
        let chunks: Vec<Result<String>> = self
            .prose
            .split_inclusive(' ')
            .map(|word| Ok(word.to_string()))
            .collect();
        Ok(Box::pin(futures::stream::iter(chunks)))
    }
}
