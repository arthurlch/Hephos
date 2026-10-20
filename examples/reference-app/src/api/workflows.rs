use hephos::prelude::*;

use crate::state::{AppState, Ctx};
use crate::workflows::provisioning::{ProvisionInput, ProvisionResult, ProvisioningWorkflow};

pub fn routes() -> Router<AppState> {
    Router::new().post("/workflows/provision", provision)
}

pub async fn provision(
    ctx: Ctx,
    Json(input): Json<ProvisionInput>,
) -> Result<Json<ProvisionResult>> {
    let mut flow = ctx.state().workflows.start("provisioning", &input).await?;
    let result = ProvisioningWorkflow.run(&ctx, &mut flow, input).await?;
    Ok(Json(result))
}
