use rivet::prelude::*;

use crate::agents::welcome;
use crate::domain::account::{Account, NewAccount, Onboarded};
use crate::model::ScriptedModel;
use crate::state::Ctx;

/// Deterministic onboarding: register the account, then generate a welcome. Each
/// step is a private method; failures propagate with `?`; a cancellation
/// checkpoint sits between steps so a cancelled request stops before the
/// expensive agent call.
pub struct OnboardingWorkflow;

impl OnboardingWorkflow {
    pub async fn run(&self, ctx: &Ctx, input: NewAccount) -> Result<Onboarded> {
        let account = self.register(ctx, input).await?;
        self.checkpoint(ctx)?;
        let welcome = self.welcome(&account).await?;
        Ok(Onboarded { account, welcome })
    }

    async fn register(&self, ctx: &Ctx, input: NewAccount) -> Result<Account> {
        ctx.state().accounts.register(ctx, input).await
    }

    fn checkpoint(&self, ctx: &Ctx) -> Result<()> {
        if ctx.cancel_token().is_cancelled() {
            return Err(Error::internal("workflow cancelled"));
        }
        Ok(())
    }

    async fn welcome(&self, account: &Account) -> Result<String> {
        let agent = welcome::build(ScriptedModel::canned());
        agent
            .run(&format!("Welcome the new user {}.", account.email))
            .await
    }
}
