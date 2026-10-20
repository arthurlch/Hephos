use hephos::test::TestClient;
use uuid::Uuid;

use crate::api;
use crate::state::AppState;
use crate::tools::CalculatorTool;
use crate::tools::calculator::CalcInput;
use crate::workflows::provisioning::{ProvisionInput, ProvisioningWorkflow};

#[tokio::test]
async fn routes_build() {
    let state = AppState::init().await.expect("state init");
    let router = api::routes(state.auth.clone());
    let _client = TestClient::new(router, state);
}

#[tokio::test]
async fn probes_respond() {
    let state = AppState::init().await.expect("state init");
    let router = api::routes(state.auth.clone());
    let client = TestClient::new(router, state);

    let res = client.get("/health").await;
    assert_eq!(res.status(), 200);

    let res = client.get("/ready").await;
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn unauthenticated_request_rejected() {
    let state = AppState::init().await.expect("state init");
    let router = api::routes(state.auth.clone());
    let client = TestClient::new(router, state);

    let res = client.get("/items").await;
    assert_eq!(res.status(), 401);
    assert_eq!(res.error_kind().as_deref(), Some("unauthorized"));
}

#[tokio::test]
async fn calculator_tool_evaluates_expression() {
    use hephos::prelude::Tool;

    let tool = CalculatorTool;
    let out = tool
        .call(CalcInput {
            expression: "25 * 4".into(),
        })
        .await
        .expect("calculation success");

    assert_eq!(out.result, 100.0);
}

#[tokio::test]
async fn workflow_execution_completes() {
    let state = AppState::init().await.expect("state init");
    let ctx = hephos::Ctx::detached(state.clone());
    let input = ProvisionInput {
        tenant_id: Uuid::new_v4(),
        name: "acme-corp".into(),
    };

    let mut flow = state
        .workflows
        .start("provisioning", &input)
        .await
        .expect("workflow start");

    let result = ProvisioningWorkflow
        .run(&ctx, &mut flow, input)
        .await
        .expect("workflow run");

    assert!(result.status.contains("acme-corp"));
}
