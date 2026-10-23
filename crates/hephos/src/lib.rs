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
//! Hephos `1.0.0` is the production-ready, stable release. All public APIs across
//! HTTP routing, database/outbox/jobs/workflows, authentication, telemetry, agents,
//! and MCP follow Semantic Versioning 2.0 with a backwards-compatibility guarantee
//! and deprecation cycle policy.

#![forbid(unsafe_code)]

pub use hephos_core::{
    App, AuditEvent, AuditLogger, AuditOutcome, CancellationToken, Config, ConnectionCap,
    ConnectionGuard, Ctx, Error, Event, Events, Identity, InMemoryAuditLogger, Json, LogFormat,
    Path, Principal, ProbeResponse, Query, RateLimitConfig, RedMetrics, Result, RouteEntry, Router,
    Secret, SecretProvider, SecretString, Secrets, StatusCode, Task, TracingAuditLogger, Uuid,
    conn, http, security, sse, telemetry, test, transport, uuid, ws,
};

#[cfg(feature = "db")]
pub use hephos_db as db;

#[cfg(feature = "agent")]
pub use hephos_agent as agent;

#[cfg(feature = "mcp")]
pub use hephos_mcp as mcp;

#[cfg(all(feature = "db", feature = "agent"))]
pub mod workflow_agent;

#[cfg(all(feature = "db", feature = "agent"))]
pub use workflow_agent::{WorkflowAgentCheckpoint, WorkflowAgentExt};

/// The one import every file starts from.
pub mod prelude {
    pub use hephos_core::{
        App, AuditEvent, AuditOutcome, ConnectionCap, ConnectionGuard, Ctx, Error, Event, Events,
        Identity, Json, Path, Principal, Query, Result, Router, Secret, SecretString, Secrets,
        StatusCode, Task, Uuid,
    };

    #[cfg(feature = "db")]
    pub use hephos_db::{
        ApprovalDecision, ApprovalState, Db, Job, JobOptions, JobRegistry, JobState, JobWorker,
        Jobs, Outbox, OutboxDrain, OutboxEntry, OutboxState, Tx, WorkflowApprovalRecord,
        WorkflowCompensationRecord, WorkflowContext, WorkflowRecord, WorkflowState,
        WorkflowStepRecord, Workflows, is_foreign_key_violation, is_unique_violation, sqlx,
    };

    #[cfg(feature = "agent")]
    pub use hephos_agent::{
        Agent, AgentCheckpoint, InMemoryAgentCheckpoint, JsonSchema, Limits, Message, Model, Role,
        TextStream, Tool, ToolCall, ToolDefinition, ToolRegistry, ToolResult, ToolSchema, Tools,
    };

    #[cfg(all(feature = "db", feature = "agent"))]
    pub use crate::workflow_agent::{WorkflowAgentCheckpoint, WorkflowAgentExt};

    #[cfg(feature = "mcp")]
    pub use hephos_mcp::{McpClient, McpServer, RegisteredMcpTool};
}
