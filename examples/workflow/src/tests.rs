use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hephos::prelude::*;
use tokio::sync::RwLock;

use crate::domain::account::NewAccount;
use crate::state::{AppState, Ctx};
use crate::workflows::onboarding::OnboardingWorkflow;

#[tokio::test]
async fn workflow_checkpointing_resumes_from_crash_without_re_executing_steps() {
    let state = AppState::init().await.unwrap();
    let ctx = Ctx::detached(state.clone());
    let workflows = ctx.state().workflows.clone();

    let input = NewAccount {
        email: "grace@example.com".into(),
    };

    let wf_id = uuid::Uuid::new_v4();

    // 1. Initial run: execute step 1 only and simulate crash
    let account_id = {
        let mut flow = workflows
            .start_with_id(wf_id, "onboarding", &input)
            .await
            .unwrap();

        let account: crate::domain::account::Account = flow
            .step("register_account", || async {
                ctx.state().accounts.register(&ctx, input.clone()).await
            })
            .await
            .unwrap();

        account.id
        // Process interrupted before "generate_welcome"
    };

    // 2. Resume run: OnboardingWorkflow.run will skip "register_account" and execute "generate_welcome"
    {
        let mut flow = workflows.resume(wf_id).await.unwrap();
        let onboarded = OnboardingWorkflow
            .run(&ctx, &mut flow, input.clone())
            .await
            .unwrap();

        // The resumed execution reused the exact checkpointed account ID
        assert_eq!(onboarded.account.id, account_id);
        assert_eq!(onboarded.account.email, "grace@example.com");
        assert!(onboarded.welcome.contains("Welcome"));
    }

    let record = workflows.get(wf_id).await.unwrap().unwrap();
    assert_eq!(record.state, WorkflowState::Completed);

    let steps = workflows.steps(wf_id).await.unwrap();
    assert_eq!(steps.len(), 3);
    assert!(steps.iter().any(|s| s.step_name == "register_account"));
    assert!(steps.iter().any(|s| s.step_name == "generate_welcome"));
    assert!(steps.iter().any(|s| s.step_name == "welcome_turn_0"));
}

#[tokio::test]
async fn workflow_approval_blocks_until_approved() {
    let state = AppState::init().await.unwrap();
    let workflows = state.workflows.clone();

    let mut flow = workflows.start("approval_demo", &()).await.unwrap();
    let wf_id = flow.workflow_id();

    let workflows_clone = workflows.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        workflows_clone
            .approve(
                wf_id,
                "finance_approval",
                Some(serde_json::json!({ "limit": 100_000 })),
            )
            .await
            .unwrap();
    });

    let decision = flow
        .await_approval("finance_approval", Duration::from_secs(2))
        .await
        .unwrap();

    match decision {
        ApprovalDecision::Approved(Some(payload)) => {
            assert_eq!(payload["limit"], 100_000);
        }
        _ => panic!("expected approved decision with payload"),
    }
}

#[tokio::test]
async fn workflow_saga_compensation_runs_in_reverse_on_failure() {
    let state = AppState::init().await.unwrap();
    let workflows = state.workflows.clone();

    let mut flow = workflows.start("saga_demo", &()).await.unwrap();
    let wf_id = flow.workflow_id();

    // Step 1: Provision resource
    let res1: String = flow
        .step("step1_provision", || async { Ok("res_100".to_string()) })
        .await
        .unwrap();

    flow.register_compensation("step1_provision", "deprovision", &res1)
        .await
        .unwrap();

    // Step 2: Reserve seat
    let res2: String = flow
        .step("step2_reserve", || async { Ok("seat_200".to_string()) })
        .await
        .unwrap();

    flow.register_compensation("step2_reserve", "release_seat", &res2)
        .await
        .unwrap();

    // Step 3: Fails
    let step3_err = flow
        .step::<(), _, _>("step3_charge", || async {
            Err(Error::invalid("insufficient funds"))
        })
        .await;
    assert!(step3_err.is_err());

    // Execute saga compensations (rollback)
    let undone = Arc::new(RwLock::new(Vec::new()));
    let undone_clone = undone.clone();

    flow.compensate(|comp| {
        let list = undone_clone.clone();
        async move {
            let mut guard = list.write().await;
            guard.push((comp.step_name, comp.compensation_name, comp.payload));
            Ok(())
        }
    })
    .await
    .unwrap();

    let executed = undone.read().await;
    assert_eq!(executed.len(), 2);
    // LIFO order: step2 rolled back before step1
    assert_eq!(executed[0].1, "release_seat");
    assert_eq!(executed[1].1, "deprovision");

    let record = workflows.get(wf_id).await.unwrap().unwrap();
    assert_eq!(record.state, WorkflowState::Compensated);
}

#[tokio::test]
async fn workflow_durable_agent_turn_checkpointing() {
    let state = AppState::init().await.unwrap();
    let workflows = state.workflows.clone();

    let mut flow = workflows.start("agent_durable_flow", &()).await.unwrap();
    let call_counter = Arc::new(AtomicUsize::new(0));

    let c = call_counter.clone();
    let checkpoint = flow.agent_checkpoint("welcome");
    let welcome: String = flow
        .step("welcome_step", || async move {
            c.fetch_add(1, Ordering::SeqCst);
            let agent = crate::agents::welcome::build(crate::agents::ScriptedModel::canned());
            agent.run_durable("Welcome developer", &checkpoint).await
        })
        .await
        .unwrap();

    assert!(welcome.contains("welcome") || welcome.contains("Welcome"));
    assert_eq!(call_counter.load(Ordering::SeqCst), 1);

    // Second execution with same step name hits checkpoint cache:
    let c2 = call_counter.clone();
    let cached_welcome: String = flow
        .step("welcome_step", || async move {
            c2.fetch_add(100, Ordering::SeqCst);
            Ok("unreachable".to_string())
        })
        .await
        .unwrap();

    assert_eq!(cached_welcome, welcome);
    assert_eq!(call_counter.load(Ordering::SeqCst), 1);
}
