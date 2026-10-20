mod agents;
mod domain;
mod services;
mod state;
mod workflows;

#[cfg(test)]
mod tests;

use hephos::prelude::*;

use crate::domain::account::NewAccount;
use crate::state::{AppState, Ctx};
use crate::workflows::onboarding::OnboardingWorkflow;

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState::init().await?;
    let ctx = Ctx::detached(state);

    let input = NewAccount {
        email: "ada@example.com".into(),
    };
    let mut flow = ctx.state().workflows.start("onboarding", &input).await?;
    let onboarded = OnboardingWorkflow.run(&ctx, &mut flow, input).await?;

    println!("{onboarded:#?}");
    Ok(())
}
