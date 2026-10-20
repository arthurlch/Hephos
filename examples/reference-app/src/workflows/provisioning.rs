use hephos::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::Ctx;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionInput {
    pub tenant_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionResult {
    pub tenant_id: Uuid,
    pub status: String,
}

pub struct ProvisioningWorkflow;

impl ProvisioningWorkflow {
    pub async fn run(
        &self,
        _ctx: &Ctx,
        flow: &mut WorkflowContext,
        input: ProvisionInput,
    ) -> Result<ProvisionResult> {
        // Step 1: Initialize tenant resources
        let stage: String = flow
            .step("initialize_resources", || async {
                Ok("allocated".to_string())
            })
            .await?;

        // Register saga compensation in case subsequent steps fail
        flow.register_compensation(
            "initialize_resources",
            "release_resources",
            &serde_json::json!({ "tenant_id": input.tenant_id }),
        )
        .await?;

        let status = format!("{}:{}", stage, input.name);
        let result = ProvisionResult {
            tenant_id: input.tenant_id,
            status,
        };

        flow.complete(&result).await?;
        Ok(result)
    }
}
