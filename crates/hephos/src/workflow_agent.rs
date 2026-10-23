//! Workflow-backed Agent checkpointing bridge.

use futures::future::BoxFuture;
use hephos_agent::{AgentCheckpoint, Message};
use hephos_core::Result;
use hephos_db::WorkflowContext;

/// Bridge enabling a [`WorkflowContext`] to provide turn checkpoint persistence for [`hephos_agent::Agent::run_durable`].
#[derive(Clone, Debug)]
pub struct WorkflowAgentCheckpoint {
    context: WorkflowContext,
    prefix: String,
}

impl WorkflowAgentCheckpoint {
    pub fn new(context: WorkflowContext, prefix: impl Into<String>) -> Self {
        Self {
            context,
            prefix: prefix.into(),
        }
    }
}

impl AgentCheckpoint for WorkflowAgentCheckpoint {
    fn load_turn(&self, turn: usize) -> BoxFuture<'_, Result<Option<Vec<Message>>>> {
        let step_name = format!("{}_turn_{turn}", self.prefix);
        Box::pin(async move { self.context.get_step_output(&step_name).await })
    }

    fn save_turn(&self, turn: usize, messages: Vec<Message>) -> BoxFuture<'_, Result<()>> {
        let step_name = format!("{}_turn_{turn}", self.prefix);
        let mut context = self.context.clone();
        Box::pin(async move {
            context.step(&step_name, || async { Ok(messages) }).await?;
            Ok(())
        })
    }
}

/// Extension trait adding agent checkpoint helpers directly to [`WorkflowContext`].
pub trait WorkflowAgentExt {
    /// Return an [`AgentCheckpoint`] scoped to a given step prefix within this workflow.
    fn agent_checkpoint(&self, prefix: impl Into<String>) -> WorkflowAgentCheckpoint;
}

impl WorkflowAgentExt for WorkflowContext {
    fn agent_checkpoint(&self, prefix: impl Into<String>) -> WorkflowAgentCheckpoint {
        WorkflowAgentCheckpoint::new(self.clone(), prefix)
    }
}
