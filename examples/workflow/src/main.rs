mod agents;
mod domain;
mod model;
mod services;
mod state;
mod workflows;

use rivet::prelude::*;

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
    let onboarded = OnboardingWorkflow.run(&ctx, input).await?;

    println!("{onboarded:#?}");
    Ok(())
}
