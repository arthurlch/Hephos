use hephos::prelude::*;

use crate::agents::ScriptedModel;
use crate::agents::welcome;
use crate::domain::account::{Account, NewAccount, Onboarded};
use crate::state::Ctx;

/// Deterministic, checkpointed onboarding workflow.
///
/// Each step is executed through [`WorkflowContext::step`], committing progress in a
/// database transaction. If the process is killed or interrupted, resuming with the
/// same workflow context skips previously committed steps and proceeds directly
/// to the unexecuted steps.
pub struct OnboardingWorkflow;

impl OnboardingWorkflow {
    pub async fn run(
        &self,
        ctx: &Ctx,
        flow: &mut WorkflowContext,
        input: NewAccount,
    ) -> Result<Onboarded> {
        // Step 1: Register account with checkpointing
        let account: Account = flow
            .step("register_account", || async {
                self.register(ctx, input).await
            })
            .await?;

        // Register saga compensation for step 1 in case downstream steps fail
        flow.register_compensation(
            "register_account",
            "delete_account",
            &serde_json::json!({ "account_id": account.id }),
        )
        .await?;

        // Step 2: Checkpoint-durable agent welcome turn
        let checkpoint = flow.agent_checkpoint("welcome");
        let welcome: String = flow
            .step("generate_welcome", || async {
                self.welcome(&account, &checkpoint).await
            })
            .await?;

        let result = Onboarded { account, welcome };
        flow.complete(&result).await?;
        Ok(result)
    }

    async fn register(&self, ctx: &Ctx, input: NewAccount) -> Result<Account> {
        ctx.state().accounts.register(ctx, input).await
    }

    async fn welcome(
        &self,
        account: &Account,
        checkpoint: &impl AgentCheckpoint,
    ) -> Result<String> {
        let agent = welcome::build(ScriptedModel::canned());
        agent
            .run_durable(
                &format!("Welcome the new user {}.", account.email),
                checkpoint,
            )
            .await
    }
}
