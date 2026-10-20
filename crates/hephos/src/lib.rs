//! Hephos — the backend framework built only for coding agents.
//!
//! This meta crate is the single import surface. Use the prelude:
//!
//! ```ignore
//! use hephos::prelude::*;
//! ```
//!
//! Capabilities are feature-gated (`db`, `agent`, `mcp`) so an app only compiles
//! and only exposes what it uses.
//!
//! # Stability
//!
//! At `0.1.0` the **HTTP + database + auth** surface (`hephos_core`, `hephos_db`) is the
//! stable, reviewed API. The **`agent` and `mcp` features are pre-1.0 and unstable** —
//! their model-provider and server seams still have placeholder implementations and
//! their signatures may change in a later minor release (they are scheduled for
//! `0.5.0`/`0.6.0`). Do not treat them as frozen.

#![forbid(unsafe_code)]

pub use hephos_core::{
    App, AuditEvent, AuditLogger, AuditOutcome, CancellationToken, Config, ConnectionCap,
    ConnectionGuard, Ctx, Error, Event, Events, Identity, InMemoryAuditLogger, Json, LogFormat,
    Path, Principal, ProbeResponse, Query, RateLimitConfig, RedMetrics, Result, RouteEntry, Router,
    Secret, SecretProvider, SecretString, Secrets, StatusCode, Task, TracingAuditLogger, conn,
    http, security, sse, telemetry, test, transport, ws,
};

#[cfg(feature = "db")]
pub use hephos_db as db;

#[cfg(feature = "agent")]
pub use hephos_agent as agent;

#[cfg(feature = "mcp")]
pub use hephos_mcp as mcp;

/// The one import every file starts from.
pub mod prelude {
    pub use hephos_core::{
        App, AuditEvent, AuditOutcome, ConnectionCap, ConnectionGuard, Ctx, Error, Event, Events,
        Identity, Json, Path, Principal, Query, Result, Router, Secret, SecretString, Secrets,
        StatusCode, Task,
    };

    #[cfg(feature = "db")]
    pub use hephos_db::{
        ApprovalDecision, ApprovalState, Db, Job, JobOptions, JobRegistry, JobState, JobWorker,
        Jobs, Outbox, OutboxDrain, OutboxEntry, OutboxState, Tx, WorkflowApprovalRecord,
        WorkflowCompensationRecord, WorkflowContext, WorkflowRecord, WorkflowState,
        WorkflowStepRecord, Workflows,
    };

    #[cfg(feature = "agent")]
    pub use hephos_agent::{
        Agent, AgentCheckpoint, InMemoryAgentCheckpoint, Limits, Model, Tool, ToolCall,
        ToolDefinition, ToolRegistry, ToolResult, ToolSchema, Tools,
    };

    #[cfg(all(feature = "db", feature = "agent"))]
    pub use crate::workflow_agent::{WorkflowAgentCheckpoint, WorkflowAgentExt};

    #[cfg(feature = "mcp")]
    pub use hephos_mcp::{McpClient, McpServer, RegisteredMcpTool};
}

#[cfg(all(feature = "db", feature = "agent"))]
pub mod workflow_agent {
    use futures::future::BoxFuture;
    use hephos_agent::{AgentCheckpoint, Message};
    use hephos_core::Result;
    use hephos_db::WorkflowContext;

    /// Bridge enabling a [`WorkflowContext`] to provide turn checkpoint persistence for [`hephos_agent::Agent::run_durable`].
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
}

#[cfg(all(feature = "db", feature = "agent"))]
pub use workflow_agent::{WorkflowAgentCheckpoint, WorkflowAgentExt};
