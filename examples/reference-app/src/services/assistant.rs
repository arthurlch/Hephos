use hephos::prelude::*;

use crate::agents::{ScriptedModel, assistant};
use crate::state::Ctx;

#[derive(Clone, Default)]
pub struct AssistantService;

impl AssistantService {
    pub fn new() -> Self {
        Self
    }

    pub async fn ask(&self, ctx: &Ctx, prompt: &str) -> Result<String> {
        let _user = ctx.require_user()?;
        if prompt.trim().is_empty() {
            return Err(Error::invalid("prompt must not be empty"));
        }
        let agent = assistant::build(ScriptedModel::canned());
        agent.run(prompt).await
    }
}
