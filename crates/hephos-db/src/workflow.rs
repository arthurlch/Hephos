//! Postgres-checkpointed durable workflows and saga orchestrations.
//!
//! Provides durable workflow execution aligned with Hephos's philosophy:
//! workflows are plain functions without a proprietary DSL. Each step commits its
//! progress in a database transaction, allowing interrupted workflows to resume from
//! their last checkpoint with exactly-once-effective execution.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hephos_core::{CancellationToken, Error, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{Db, into_internal};

/// Lifecycle state of a durable workflow execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    /// Actively executing steps.
    Running,
    /// Blocked awaiting a human-in-the-loop decision or approval.
    WaitingApproval,
    /// Successfully completed all steps.
    Completed,
    /// Execution failed without compensation.
    Failed,
    /// Cancelled via token or administrative action.
    Cancelled,
    /// Currently executing registered compensation actions (saga rollback).
    Compensating,
    /// Completed compensation after downstream failure.
    Compensated,
}

impl WorkflowState {
    /// Convert state to the database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowState::Running => "running",
            WorkflowState::WaitingApproval => "waiting_approval",
            WorkflowState::Completed => "completed",
            WorkflowState::Failed => "failed",
            WorkflowState::Cancelled => "cancelled",
            WorkflowState::Compensating => "compensating",
            WorkflowState::Compensated => "compensated",
        }
    }
}

impl std::str::FromStr for WorkflowState {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "running" => Ok(WorkflowState::Running),
            "waiting_approval" => Ok(WorkflowState::WaitingApproval),
            "completed" => Ok(WorkflowState::Completed),
            "failed" => Ok(WorkflowState::Failed),
            "cancelled" => Ok(WorkflowState::Cancelled),
            "compensating" => Ok(WorkflowState::Compensating),
            "compensated" => Ok(WorkflowState::Compensated),
            _ => Err(()),
        }
    }
}

/// A persisted workflow record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRecord {
    /// Unique workflow execution ID.
    pub id: Uuid,
    /// Workflow name identifier.
    pub name: String,
    /// Current execution state.
    pub state: WorkflowState,
    /// Workflow input payload.
    pub input: serde_json::Value,
    /// Final output payload (if completed).
    pub output: Option<serde_json::Value>,
    /// Last error message (if failed).
    pub last_error: Option<String>,
    /// Workflow creation timestamp.
    pub created_at: OffsetDateTime,
    /// Last status update timestamp.
    pub updated_at: OffsetDateTime,
}

/// A persisted checkpoint record for an individual workflow step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStepRecord {
    /// Workflow instance ID.
    pub workflow_id: Uuid,
    /// Step name within the workflow.
    pub step_name: String,
    /// Step state (e.g. "completed", "failed").
    pub state: String,
    /// Serialized output of the step.
    pub output: Option<serde_json::Value>,
    /// Error message if the step failed.
    pub error: Option<String>,
    /// Timestamp when this step checkpoint was created.
    pub created_at: OffsetDateTime,
}

/// Status of an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    /// Pending decision.
    Pending,
    /// Approved by user/system.
    Approved,
    /// Rejected by user/system.
    Rejected,
    /// Timed out before a decision was made.
    TimedOut,
}

impl ApprovalState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalState::Pending => "pending",
            ApprovalState::Approved => "approved",
            ApprovalState::Rejected => "rejected",
            ApprovalState::TimedOut => "timed_out",
        }
    }
}

impl std::str::FromStr for ApprovalState {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(ApprovalState::Pending),
            "approved" => Ok(ApprovalState::Approved),
            "rejected" => Ok(ApprovalState::Rejected),
            "timed_out" => Ok(ApprovalState::TimedOut),
            _ => Err(()),
        }
    }
}

/// Decision result of an approval step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    /// Approved with optional decision data.
    Approved(Option<serde_json::Value>),
    /// Rejected with optional reason.
    Rejected(Option<String>),
    /// The approval timed out.
    TimedOut,
}

/// Persisted approval record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowApprovalRecord {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub approval_name: String,
    pub state: ApprovalState,
    pub decision_payload: Option<serde_json::Value>,
    pub created_at: OffsetDateTime,
    pub decided_at: Option<OffsetDateTime>,
}

/// Persisted saga compensation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCompensationRecord {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub step_name: String,
    pub compensation_name: String,
    pub payload: serde_json::Value,
    pub sequence_order: i32,
    pub executed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

/// Storage interface for workflow state and step checkpoints.
pub trait WorkflowStore: Send + Sync {
    fn migrate(&self) -> BoxFuture<'_, Result<()>>;
    fn create_workflow(
        &self,
        id: Uuid,
        name: &str,
        input: serde_json::Value,
    ) -> BoxFuture<'_, Result<()>>;
    fn get_workflow(&self, id: Uuid) -> BoxFuture<'_, Result<Option<WorkflowRecord>>>;
    fn update_workflow_state(
        &self,
        id: Uuid,
        state: WorkflowState,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> BoxFuture<'_, Result<()>>;

    fn get_step(
        &self,
        id: Uuid,
        step_name: &str,
    ) -> BoxFuture<'_, Result<Option<WorkflowStepRecord>>>;
    fn list_steps(&self, id: Uuid) -> BoxFuture<'_, Result<Vec<WorkflowStepRecord>>>;
    fn save_step(
        &self,
        id: Uuid,
        step_name: &str,
        state: &str,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> BoxFuture<'_, Result<()>>;

    fn create_or_get_approval(
        &self,
        id: Uuid,
        approval_name: &str,
    ) -> BoxFuture<'_, Result<WorkflowApprovalRecord>>;
    fn decide_approval(
        &self,
        id: Uuid,
        approval_name: &str,
        state: ApprovalState,
        payload: Option<serde_json::Value>,
    ) -> BoxFuture<'_, Result<()>>;
    fn list_approvals(&self, id: Uuid) -> BoxFuture<'_, Result<Vec<WorkflowApprovalRecord>>>;

    fn register_compensation(
        &self,
        id: Uuid,
        step_name: &str,
        compensation_name: &str,
        payload: serde_json::Value,
    ) -> BoxFuture<'_, Result<()>>;
    fn list_pending_compensations(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowCompensationRecord>>>;
    fn mark_compensation_executed(
        &self,
        id: Uuid,
        compensation_id: Uuid,
    ) -> BoxFuture<'_, Result<()>>;
    fn list_compensations(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowCompensationRecord>>>;
}

/// Postgres-backed storage implementation for durable workflows.
pub struct PostgresWorkflowStore {
    db: Db,
}

impl PostgresWorkflowStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

impl WorkflowStore for PostgresWorkflowStore {
    fn migrate(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            sqlx::query("SELECT pg_advisory_lock(910248233)")
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;

            let statements = [
                r#"
                CREATE TABLE IF NOT EXISTS hephos_workflows (
                    id UUID PRIMARY KEY,
                    name TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'running',
                    input JSONB NOT NULL,
                    output JSONB,
                    last_error TEXT,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )
                "#,
                r#"
                CREATE INDEX IF NOT EXISTS idx_hephos_workflows_state
                    ON hephos_workflows (state, created_at DESC)
                "#,
                r#"
                CREATE TABLE IF NOT EXISTS hephos_workflow_steps (
                    workflow_id UUID NOT NULL REFERENCES hephos_workflows(id) ON DELETE CASCADE,
                    step_name TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'completed',
                    output JSONB,
                    error TEXT,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    PRIMARY KEY (workflow_id, step_name)
                )
                "#,
                r#"
                CREATE TABLE IF NOT EXISTS hephos_workflow_approvals (
                    id UUID PRIMARY KEY,
                    workflow_id UUID NOT NULL REFERENCES hephos_workflows(id) ON DELETE CASCADE,
                    approval_name TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'pending',
                    decision_payload JSONB,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    decided_at TIMESTAMPTZ,
                    UNIQUE (workflow_id, approval_name)
                )
                "#,
                r#"
                CREATE INDEX IF NOT EXISTS idx_hephos_workflow_approvals_pending
                    ON hephos_workflow_approvals (workflow_id, approval_name)
                    WHERE state = 'pending'
                "#,
                r#"
                CREATE TABLE IF NOT EXISTS hephos_workflow_compensations (
                    id UUID PRIMARY KEY,
                    workflow_id UUID NOT NULL REFERENCES hephos_workflows(id) ON DELETE CASCADE,
                    step_name TEXT NOT NULL,
                    compensation_name TEXT NOT NULL,
                    payload JSONB NOT NULL,
                    sequence_order INT NOT NULL,
                    executed_at TIMESTAMPTZ,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )
                "#,
                r#"
                CREATE INDEX IF NOT EXISTS idx_hephos_workflow_compensations_order
                    ON hephos_workflow_compensations (workflow_id, sequence_order DESC)
                "#,
                // Enforce one sequence number per workflow so a concurrent
                // `register_compensation` race fails loudly instead of corrupting LIFO
                // order. Idempotent: safe on fresh and existing databases.
                r#"
                CREATE UNIQUE INDEX IF NOT EXISTS uq_hephos_workflow_compensations_seq
                    ON hephos_workflow_compensations (workflow_id, sequence_order)
                "#,
            ];

            for stmt in statements {
                if let Err(e) = sqlx::query(stmt).execute(self.db.pool()).await {
                    let _ = sqlx::query("SELECT pg_advisory_unlock(910248233)")
                        .execute(self.db.pool())
                        .await;
                    return Err(into_internal(e));
                }
            }

            sqlx::query("SELECT pg_advisory_unlock(910248233)")
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;

            Ok(())
        })
    }

    fn create_workflow(
        &self,
        id: Uuid,
        name: &str,
        input: serde_json::Value,
    ) -> BoxFuture<'_, Result<()>> {
        let name = name.to_string();
        Box::pin(async move {
            let input_str = serde_json::to_string(&input)
                .map_err(|e| Error::internal(format!("failed to serialize input: {e}")))?;
            let query = r#"
                INSERT INTO hephos_workflows (id, name, state, input, created_at, updated_at)
                VALUES ($1, $2, 'running', $3::jsonb, NOW(), NOW())
                ON CONFLICT (id) DO NOTHING
            "#;
            sqlx::query(query)
                .bind(id)
                .bind(name)
                .bind(input_str)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
            Ok(())
        })
    }

    fn get_workflow(&self, id: Uuid) -> BoxFuture<'_, Result<Option<WorkflowRecord>>> {
        Box::pin(async move {
            let query = r#"
                SELECT id, name, state, input::text AS input, output::text AS output,
                       last_error, created_at, updated_at
                FROM hephos_workflows
                WHERE id = $1
            "#;
            let row = sqlx::query(query)
                .bind(id)
                .fetch_optional(self.db.pool())
                .await
                .map_err(into_internal)?;

            match row {
                Some(r) => {
                    let state_str: String = r.try_get("state").map_err(into_internal)?;
                    let state = state_str.parse::<WorkflowState>().map_err(|_| {
                        Error::internal(format!("invalid workflow state: {state_str}"))
                    })?;
                    let input_str: String = r.try_get("input").map_err(into_internal)?;
                    let input = serde_json::from_str(&input_str)
                        .map_err(|e| Error::internal(format!("corrupt input json: {e}")))?;
                    let output: Option<serde_json::Value> =
                        match r
                            .try_get::<Option<String>, _>("output")
                            .map_err(into_internal)?
                        {
                            Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                                Error::internal(format!("corrupt output json: {e}"))
                            })?),
                            None => None,
                        };
                    Ok(Some(WorkflowRecord {
                        id: r.try_get("id").map_err(into_internal)?,
                        name: r.try_get("name").map_err(into_internal)?,
                        state,
                        input,
                        output,
                        last_error: r.try_get("last_error").map_err(into_internal)?,
                        created_at: r.try_get("created_at").map_err(into_internal)?,
                        updated_at: r.try_get("updated_at").map_err(into_internal)?,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    fn update_workflow_state(
        &self,
        id: Uuid,
        state: WorkflowState,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let output_str = output
                .map(|v| serde_json::to_string(&v))
                .transpose()
                .map_err(|e| Error::internal(format!("failed to serialize output: {e}")))?;
            let query = r#"
                UPDATE hephos_workflows
                SET state = $2,
                    output = $3::jsonb,
                    last_error = $4,
                    updated_at = NOW()
                WHERE id = $1
            "#;
            sqlx::query(query)
                .bind(id)
                .bind(state.as_str())
                .bind(output_str)
                .bind(error)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
            Ok(())
        })
    }

    fn get_step(
        &self,
        id: Uuid,
        step_name: &str,
    ) -> BoxFuture<'_, Result<Option<WorkflowStepRecord>>> {
        let step_name = step_name.to_string();
        Box::pin(async move {
            let query = r#"
                SELECT workflow_id, step_name, state, output::text AS output, error, created_at
                FROM hephos_workflow_steps
                WHERE workflow_id = $1 AND step_name = $2
            "#;
            let row = sqlx::query(query)
                .bind(id)
                .bind(step_name)
                .fetch_optional(self.db.pool())
                .await
                .map_err(into_internal)?;

            match row {
                Some(r) => {
                    let output: Option<serde_json::Value> = match r
                        .try_get::<Option<String>, _>("output")
                        .map_err(into_internal)?
                    {
                        Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                            Error::internal(format!("corrupt step output json: {e}"))
                        })?),
                        None => None,
                    };
                    Ok(Some(WorkflowStepRecord {
                        workflow_id: r.try_get("workflow_id").map_err(into_internal)?,
                        step_name: r.try_get("step_name").map_err(into_internal)?,
                        state: r.try_get("state").map_err(into_internal)?,
                        output,
                        error: r.try_get("error").map_err(into_internal)?,
                        created_at: r.try_get("created_at").map_err(into_internal)?,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    fn list_steps(&self, id: Uuid) -> BoxFuture<'_, Result<Vec<WorkflowStepRecord>>> {
        Box::pin(async move {
            let query = r#"
                SELECT workflow_id, step_name, state, output::text AS output, error, created_at
                FROM hephos_workflow_steps
                WHERE workflow_id = $1
                ORDER BY created_at ASC
            "#;
            let rows = sqlx::query(query)
                .bind(id)
                .fetch_all(self.db.pool())
                .await
                .map_err(into_internal)?;

            let mut steps = Vec::new();
            for r in rows {
                let output: Option<serde_json::Value> =
                    match r
                        .try_get::<Option<String>, _>("output")
                        .map_err(into_internal)?
                    {
                        Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                            Error::internal(format!("corrupt step output json: {e}"))
                        })?),
                        None => None,
                    };
                steps.push(WorkflowStepRecord {
                    workflow_id: r.try_get("workflow_id").map_err(into_internal)?,
                    step_name: r.try_get("step_name").map_err(into_internal)?,
                    state: r.try_get("state").map_err(into_internal)?,
                    output,
                    error: r.try_get("error").map_err(into_internal)?,
                    created_at: r.try_get("created_at").map_err(into_internal)?,
                });
            }
            Ok(steps)
        })
    }

    fn save_step(
        &self,
        id: Uuid,
        step_name: &str,
        state: &str,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> BoxFuture<'_, Result<()>> {
        let step_name = step_name.to_string();
        let state = state.to_string();
        Box::pin(async move {
            let output_str = output
                .map(|v| serde_json::to_string(&v))
                .transpose()
                .map_err(|e| Error::internal(format!("failed to serialize step output: {e}")))?;
            let query = r#"
                INSERT INTO hephos_workflow_steps (workflow_id, step_name, state, output, error, created_at)
                VALUES ($1, $2, $3, $4::jsonb, $5, NOW())
                ON CONFLICT (workflow_id, step_name) DO UPDATE
                    SET state = EXCLUDED.state,
                        output = EXCLUDED.output,
                        error = EXCLUDED.error
            "#;
            sqlx::query(query)
                .bind(id)
                .bind(step_name)
                .bind(state)
                .bind(output_str)
                .bind(error)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
            Ok(())
        })
    }

    fn create_or_get_approval(
        &self,
        id: Uuid,
        approval_name: &str,
    ) -> BoxFuture<'_, Result<WorkflowApprovalRecord>> {
        let approval_name = approval_name.to_string();
        Box::pin(async move {
            let approval_id = Uuid::new_v4();
            let query = r#"
                INSERT INTO hephos_workflow_approvals (id, workflow_id, approval_name, state, created_at)
                VALUES ($1, $2, $3, 'pending', NOW())
                ON CONFLICT (workflow_id, approval_name) DO NOTHING
            "#;
            sqlx::query(query)
                .bind(approval_id)
                .bind(id)
                .bind(&approval_name)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;

            let select = r#"
                SELECT id, workflow_id, approval_name, state, decision_payload::text AS decision_payload,
                       created_at, decided_at
                FROM hephos_workflow_approvals
                WHERE workflow_id = $1 AND approval_name = $2
            "#;
            let r = sqlx::query(select)
                .bind(id)
                .bind(&approval_name)
                .fetch_one(self.db.pool())
                .await
                .map_err(into_internal)?;

            let state_str: String = r.try_get("state").map_err(into_internal)?;
            let state = state_str
                .parse::<ApprovalState>()
                .map_err(|_| Error::internal("invalid approval state"))?;
            let decision_payload: Option<serde_json::Value> =
                match r
                    .try_get::<Option<String>, _>("decision_payload")
                    .map_err(into_internal)?
                {
                    Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                        Error::internal(format!("corrupt decision payload json: {e}"))
                    })?),
                    None => None,
                };

            Ok(WorkflowApprovalRecord {
                id: r.try_get("id").map_err(into_internal)?,
                workflow_id: r.try_get("workflow_id").map_err(into_internal)?,
                approval_name: r.try_get("approval_name").map_err(into_internal)?,
                state,
                decision_payload,
                created_at: r.try_get("created_at").map_err(into_internal)?,
                decided_at: r.try_get("decided_at").map_err(into_internal)?,
            })
        })
    }

    fn decide_approval(
        &self,
        id: Uuid,
        approval_name: &str,
        state: ApprovalState,
        payload: Option<serde_json::Value>,
    ) -> BoxFuture<'_, Result<()>> {
        let approval_name = approval_name.to_string();
        Box::pin(async move {
            let payload_str = payload
                .map(|v| serde_json::to_string(&v))
                .transpose()
                .map_err(|e| {
                    Error::internal(format!("failed to serialize approval payload: {e}"))
                })?;
            let query = r#"
                UPDATE hephos_workflow_approvals
                SET state = $3,
                    decision_payload = $4::jsonb,
                    decided_at = NOW()
                WHERE workflow_id = $1 AND approval_name = $2 AND state = 'pending'
            "#;
            let res = sqlx::query(query)
                .bind(id)
                .bind(&approval_name)
                .bind(state.as_str())
                .bind(payload_str)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;

            if res.rows_affected() == 0 {
                let check_query = r#"
                    SELECT state FROM hephos_workflow_approvals
                    WHERE workflow_id = $1 AND approval_name = $2
                "#;
                let existing = sqlx::query(check_query)
                    .bind(id)
                    .bind(&approval_name)
                    .fetch_optional(self.db.pool())
                    .await
                    .map_err(into_internal)?;

                match existing {
                    Some(row) => {
                        let cur_state: String = row.try_get("state").map_err(into_internal)?;
                        return Err(Error::conflict(format!(
                            "approval '{approval_name}' has already been decided with state '{cur_state}'"
                        )));
                    }
                    None => {
                        return Err(Error::not_found(format!("approval '{approval_name}'")));
                    }
                }
            }

            Ok(())
        })
    }

    fn list_approvals(&self, id: Uuid) -> BoxFuture<'_, Result<Vec<WorkflowApprovalRecord>>> {
        Box::pin(async move {
            let query = r#"
                SELECT id, workflow_id, approval_name, state, decision_payload::text AS decision_payload,
                       created_at, decided_at
                FROM hephos_workflow_approvals
                WHERE workflow_id = $1
                ORDER BY created_at ASC
            "#;
            let rows = sqlx::query(query)
                .bind(id)
                .fetch_all(self.db.pool())
                .await
                .map_err(into_internal)?;

            let mut list = Vec::new();
            for r in rows {
                let state_str: String = r.try_get("state").map_err(into_internal)?;
                let state = state_str
                    .parse::<ApprovalState>()
                    .map_err(|_| Error::internal("invalid approval state"))?;
                let decision_payload: Option<serde_json::Value> = match r
                    .try_get::<Option<String>, _>("decision_payload")
                    .map_err(into_internal)?
                {
                    Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                        Error::internal(format!("corrupt decision payload json: {e}"))
                    })?),
                    None => None,
                };
                list.push(WorkflowApprovalRecord {
                    id: r.try_get("id").map_err(into_internal)?,
                    workflow_id: r.try_get("workflow_id").map_err(into_internal)?,
                    approval_name: r.try_get("approval_name").map_err(into_internal)?,
                    state,
                    decision_payload,
                    created_at: r.try_get("created_at").map_err(into_internal)?,
                    decided_at: r.try_get("decided_at").map_err(into_internal)?,
                });
            }
            Ok(list)
        })
    }

    fn register_compensation(
        &self,
        id: Uuid,
        step_name: &str,
        compensation_name: &str,
        payload: serde_json::Value,
    ) -> BoxFuture<'_, Result<()>> {
        let step_name = step_name.to_string();
        let compensation_name = compensation_name.to_string();
        Box::pin(async move {
            let comp_id = Uuid::new_v4();
            let payload_str = serde_json::to_string(&payload).map_err(|e| {
                Error::internal(format!("failed to serialize compensation payload: {e}"))
            })?;

            // Compute the sequence and insert in ONE statement so there is no
            // read-then-write gap. A UNIQUE index on (workflow_id, sequence_order)
            // (see `migrate`) turns any residual concurrent race into a hard error
            // rather than silently duplicating a sequence number and corrupting LIFO
            // order.
            let insert = r#"
                INSERT INTO hephos_workflow_compensations (
                    id, workflow_id, step_name, compensation_name, payload, sequence_order, created_at
                )
                VALUES (
                    $1, $2, $3, $4, $5::jsonb,
                    COALESCE(
                        (SELECT MAX(sequence_order) FROM hephos_workflow_compensations WHERE workflow_id = $2),
                        0
                    ) + 1,
                    NOW()
                )
            "#;
            sqlx::query(insert)
                .bind(comp_id)
                .bind(id)
                .bind(step_name)
                .bind(compensation_name)
                .bind(payload_str)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
            Ok(())
        })
    }

    fn list_pending_compensations(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowCompensationRecord>>> {
        Box::pin(async move {
            let query = r#"
                SELECT id, workflow_id, step_name, compensation_name, payload::text AS payload,
                       sequence_order, executed_at, created_at
                FROM hephos_workflow_compensations
                WHERE workflow_id = $1 AND executed_at IS NULL
                ORDER BY sequence_order DESC
            "#;
            let rows = sqlx::query(query)
                .bind(id)
                .fetch_all(self.db.pool())
                .await
                .map_err(into_internal)?;

            let mut list = Vec::new();
            for r in rows {
                let payload_str: String = r.try_get("payload").map_err(into_internal)?;
                let payload = serde_json::from_str(&payload_str).map_err(|e| {
                    Error::internal(format!("corrupt compensation payload json: {e}"))
                })?;
                list.push(WorkflowCompensationRecord {
                    id: r.try_get("id").map_err(into_internal)?,
                    workflow_id: r.try_get("workflow_id").map_err(into_internal)?,
                    step_name: r.try_get("step_name").map_err(into_internal)?,
                    compensation_name: r.try_get("compensation_name").map_err(into_internal)?,
                    payload,
                    sequence_order: r.try_get("sequence_order").map_err(into_internal)?,
                    executed_at: r.try_get("executed_at").map_err(into_internal)?,
                    created_at: r.try_get("created_at").map_err(into_internal)?,
                });
            }
            Ok(list)
        })
    }

    fn mark_compensation_executed(
        &self,
        id: Uuid,
        compensation_id: Uuid,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let query = r#"
                UPDATE hephos_workflow_compensations
                SET executed_at = NOW()
                WHERE workflow_id = $1 AND id = $2
            "#;
            sqlx::query(query)
                .bind(id)
                .bind(compensation_id)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
            Ok(())
        })
    }

    fn list_compensations(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowCompensationRecord>>> {
        Box::pin(async move {
            let query = r#"
                SELECT id, workflow_id, step_name, compensation_name, payload::text AS payload,
                       sequence_order, executed_at, created_at
                FROM hephos_workflow_compensations
                WHERE workflow_id = $1
                ORDER BY sequence_order ASC
            "#;
            let rows = sqlx::query(query)
                .bind(id)
                .fetch_all(self.db.pool())
                .await
                .map_err(into_internal)?;

            let mut list = Vec::new();
            for r in rows {
                let payload_str: String = r.try_get("payload").map_err(into_internal)?;
                let payload = serde_json::from_str(&payload_str).map_err(|e| {
                    Error::internal(format!("corrupt compensation payload json: {e}"))
                })?;
                list.push(WorkflowCompensationRecord {
                    id: r.try_get("id").map_err(into_internal)?,
                    workflow_id: r.try_get("workflow_id").map_err(into_internal)?,
                    step_name: r.try_get("step_name").map_err(into_internal)?,
                    compensation_name: r.try_get("compensation_name").map_err(into_internal)?,
                    payload,
                    sequence_order: r.try_get("sequence_order").map_err(into_internal)?,
                    executed_at: r.try_get("executed_at").map_err(into_internal)?,
                    created_at: r.try_get("created_at").map_err(into_internal)?,
                });
            }
            Ok(list)
        })
    }
}

/// In-memory storage implementation for offline unit tests and mock workflows.
#[derive(Default)]
pub struct InMemoryWorkflowStore {
    workflows: RwLock<HashMap<Uuid, WorkflowRecord>>,
    steps: RwLock<HashMap<(Uuid, String), WorkflowStepRecord>>,
    approvals: RwLock<HashMap<(Uuid, String), WorkflowApprovalRecord>>,
    compensations: RwLock<HashMap<Uuid, Vec<WorkflowCompensationRecord>>>,
}

impl InMemoryWorkflowStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WorkflowStore for InMemoryWorkflowStore {
    fn migrate(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn create_workflow(
        &self,
        id: Uuid,
        name: &str,
        input: serde_json::Value,
    ) -> BoxFuture<'_, Result<()>> {
        let name = name.to_string();
        Box::pin(async move {
            let mut guard = self.workflows.write().await;
            guard.entry(id).or_insert_with(|| {
                let now = OffsetDateTime::now_utc();
                WorkflowRecord {
                    id,
                    name,
                    state: WorkflowState::Running,
                    input,
                    output: None,
                    last_error: None,
                    created_at: now,
                    updated_at: now,
                }
            });
            Ok(())
        })
    }

    fn get_workflow(&self, id: Uuid) -> BoxFuture<'_, Result<Option<WorkflowRecord>>> {
        Box::pin(async move {
            let guard = self.workflows.read().await;
            Ok(guard.get(&id).cloned())
        })
    }

    fn update_workflow_state(
        &self,
        id: Uuid,
        state: WorkflowState,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let mut guard = self.workflows.write().await;
            if let Some(wf) = guard.get_mut(&id) {
                wf.state = state;
                if output.is_some() {
                    wf.output = output;
                }
                if error.is_some() {
                    wf.last_error = error;
                }
                wf.updated_at = OffsetDateTime::now_utc();
            }
            Ok(())
        })
    }

    fn get_step(
        &self,
        id: Uuid,
        step_name: &str,
    ) -> BoxFuture<'_, Result<Option<WorkflowStepRecord>>> {
        let key = (id, step_name.to_string());
        Box::pin(async move {
            let guard = self.steps.read().await;
            Ok(guard.get(&key).cloned())
        })
    }

    fn list_steps(&self, id: Uuid) -> BoxFuture<'_, Result<Vec<WorkflowStepRecord>>> {
        Box::pin(async move {
            let guard = self.steps.read().await;
            let mut steps: Vec<_> = guard
                .values()
                .filter(|s| s.workflow_id == id)
                .cloned()
                .collect();
            steps.sort_by_key(|s| s.created_at);
            Ok(steps)
        })
    }

    fn save_step(
        &self,
        id: Uuid,
        step_name: &str,
        state: &str,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> BoxFuture<'_, Result<()>> {
        let key = (id, step_name.to_string());
        let state = state.to_string();
        Box::pin(async move {
            let mut guard = self.steps.write().await;
            guard.insert(
                key.clone(),
                WorkflowStepRecord {
                    workflow_id: id,
                    step_name: key.1,
                    state,
                    output,
                    error,
                    created_at: OffsetDateTime::now_utc(),
                },
            );
            Ok(())
        })
    }

    fn create_or_get_approval(
        &self,
        id: Uuid,
        approval_name: &str,
    ) -> BoxFuture<'_, Result<WorkflowApprovalRecord>> {
        let key = (id, approval_name.to_string());
        Box::pin(async move {
            let mut guard = self.approvals.write().await;
            let record = guard
                .entry(key.clone())
                .or_insert_with(|| WorkflowApprovalRecord {
                    id: Uuid::new_v4(),
                    workflow_id: id,
                    approval_name: key.1,
                    state: ApprovalState::Pending,
                    decision_payload: None,
                    created_at: OffsetDateTime::now_utc(),
                    decided_at: None,
                });
            Ok(record.clone())
        })
    }

    fn decide_approval(
        &self,
        id: Uuid,
        approval_name: &str,
        state: ApprovalState,
        payload: Option<serde_json::Value>,
    ) -> BoxFuture<'_, Result<()>> {
        let key = (id, approval_name.to_string());
        let approval_name = approval_name.to_string();
        Box::pin(async move {
            let mut guard = self.approvals.write().await;
            if let Some(record) = guard.get_mut(&key) {
                if record.state != ApprovalState::Pending {
                    return Err(Error::conflict(format!(
                        "approval '{approval_name}' has already been decided with state {:?}",
                        record.state
                    )));
                }
                record.state = state;
                record.decision_payload = payload;
                record.decided_at = Some(OffsetDateTime::now_utc());
                Ok(())
            } else {
                Err(Error::not_found(format!("approval '{approval_name}'")))
            }
        })
    }

    fn list_approvals(&self, id: Uuid) -> BoxFuture<'_, Result<Vec<WorkflowApprovalRecord>>> {
        Box::pin(async move {
            let guard = self.approvals.read().await;
            let mut list: Vec<_> = guard
                .values()
                .filter(|a| a.workflow_id == id)
                .cloned()
                .collect();
            list.sort_by_key(|a| a.created_at);
            Ok(list)
        })
    }

    fn register_compensation(
        &self,
        id: Uuid,
        step_name: &str,
        compensation_name: &str,
        payload: serde_json::Value,
    ) -> BoxFuture<'_, Result<()>> {
        let step_name = step_name.to_string();
        let compensation_name = compensation_name.to_string();
        Box::pin(async move {
            let mut guard = self.compensations.write().await;
            let list = guard.entry(id).or_default();
            let next_seq = (list.len() + 1) as i32;
            list.push(WorkflowCompensationRecord {
                id: Uuid::new_v4(),
                workflow_id: id,
                step_name,
                compensation_name,
                payload,
                sequence_order: next_seq,
                executed_at: None,
                created_at: OffsetDateTime::now_utc(),
            });
            Ok(())
        })
    }

    fn list_pending_compensations(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowCompensationRecord>>> {
        Box::pin(async move {
            let guard = self.compensations.read().await;
            let mut list: Vec<_> = guard
                .get(&id)
                .map(|l| {
                    l.iter()
                        .filter(|c| c.executed_at.is_none())
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            list.sort_by_key(|c| std::cmp::Reverse(c.sequence_order));
            Ok(list)
        })
    }

    fn mark_compensation_executed(
        &self,
        id: Uuid,
        compensation_id: Uuid,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let mut guard = self.compensations.write().await;
            if let Some(list) = guard.get_mut(&id) {
                if let Some(item) = list.iter_mut().find(|c| c.id == compensation_id) {
                    item.executed_at = Some(OffsetDateTime::now_utc());
                }
            }
            Ok(())
        })
    }

    fn list_compensations(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowCompensationRecord>>> {
        Box::pin(async move {
            let guard = self.compensations.read().await;
            let mut list = guard.get(&id).cloned().unwrap_or_default();
            list.sort_by_key(|c| c.sequence_order);
            Ok(list)
        })
    }
}

/// Context handle provided to workflow executions.
///
/// Steps are wrapped via [`WorkflowContext::step`], which memoizes successful executions
/// in Postgres. If the workflow process crashes or restarts, subsequent execution runs
/// skip already completed steps without re-executing side effects.
#[derive(Clone)]
pub struct WorkflowContext {
    id: Uuid,
    name: String,
    store: Arc<dyn WorkflowStore>,
    cancel_token: CancellationToken,
}

impl std::fmt::Debug for WorkflowContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowContext")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish()
    }
}

impl WorkflowContext {
    /// Construct a new workflow context.
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        store: Arc<dyn WorkflowStore>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            store,
            cancel_token,
        }
    }

    /// Access the unique workflow instance ID.
    pub fn workflow_id(&self) -> Uuid {
        self.id
    }

    /// Access the workflow name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Access the cancellation token.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }

    /// Check if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// Execute a checkpointed step.
    ///
    /// # Delivery semantics (at-least-once, idempotency required)
    ///
    /// A step that has **completed** in a prior execution is memoized: its output is
    /// returned immediately without invoking `f`. Any other prior state — never started,
    /// interrupted mid-execution (`running`), or previously `failed` — causes `f` to run
    /// (again). Because the side effect runs *before* the completion checkpoint commits,
    /// a crash in that window re-executes the effect on resume. **Step bodies must
    /// therefore be idempotent** (use an idempotency key, upsert, or a conditional
    /// write). This is at-least-once with exactly-once *memoization of completed steps* —
    /// not atomic exactly-once. True exactly-once would require the effect and the
    /// checkpoint to commit in a single transaction, which this closure-based API cannot
    /// do for arbitrary external effects.
    ///
    /// A write-ahead `running` marker is persisted *before* `f` runs, so an interrupted
    /// step leaves a detectable record (surfaced via a warning on resume) rather than
    /// vanishing. A previously `failed` step is retried on resume rather than permanently
    /// poisoning the workflow.
    pub async fn step<T, F, Fut>(&mut self, step_name: &str, f: F) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        if self.is_cancelled() {
            let _ = self
                .store
                .update_workflow_state(
                    self.id,
                    WorkflowState::Cancelled,
                    None,
                    Some("cancelled".into()),
                )
                .await;
            return Err(Error::internal("workflow execution cancelled"));
        }

        // 1. Memoized completion is the ONLY short-circuit. `running`/`failed`/absent all
        //    fall through to (re-)execution — see the delivery-semantics doc above.
        if let Some(step) = self.store.get_step(self.id, step_name).await? {
            match step.state.as_str() {
                "completed" => {
                    if let Some(output) = step.output {
                        return serde_json::from_value(output).map_err(|e| {
                            Error::internal(format!(
                                "failed to deserialize step {step_name} checkpoint: {e}"
                            ))
                        });
                    }
                }
                "running" => {
                    tracing::warn!(
                        workflow_id = %self.id,
                        step = %step_name,
                        "resuming a step interrupted mid-execution; re-executing (side effect must be idempotent)"
                    );
                }
                "failed" => {
                    tracing::warn!(
                        workflow_id = %self.id,
                        step = %step_name,
                        "retrying a previously failed step on resume"
                    );
                }
                _ => {}
            }
        }

        // 2. Write-ahead intent marker: persist `running` BEFORE the side effect so a
        //    crash leaves a detectable in-flight record instead of silent nothing.
        self.store
            .save_step(self.id, step_name, "running", None, None)
            .await?;

        // 3. Execute the step and record the terminal state.
        match f().await {
            Ok(result) => {
                let json = serde_json::to_value(&result).map_err(|e| {
                    Error::internal(format!("failed to serialize step {step_name} output: {e}"))
                })?;
                self.store
                    .save_step(self.id, step_name, "completed", Some(json), None)
                    .await?;
                Ok(result)
            }
            Err(err) => {
                let err_str = err.to_string();
                let _ = self
                    .store
                    .save_step(self.id, step_name, "failed", None, Some(err_str.clone()))
                    .await;
                let _ = self
                    .store
                    .update_workflow_state(self.id, WorkflowState::Failed, None, Some(err_str))
                    .await;
                Err(err)
            }
        }
    }

    /// Execute a step bounded by a wall-clock timeout.
    pub async fn step_with_timeout<T, F, Fut>(
        &mut self,
        step_name: &str,
        timeout: Duration,
        f: F,
    ) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        self.step(step_name, || async {
            match tokio::time::timeout(timeout, f()).await {
                Ok(res) => res,
                Err(_) => Err(Error::internal(format!(
                    "step {step_name} exceeded timeout of {timeout:?}"
                ))),
            }
        })
        .await
    }

    /// Register a saga compensation to be executed in reverse order if a downstream step fails.
    pub async fn register_compensation<P: Serialize>(
        &mut self,
        step_name: &str,
        compensation_name: &str,
        payload: &P,
    ) -> Result<()> {
        let val = serde_json::to_value(payload).map_err(|e| {
            Error::internal(format!("failed to serialize compensation payload: {e}"))
        })?;
        self.store
            .register_compensation(self.id, step_name, compensation_name, val)
            .await
    }

    /// Execute all pending registered compensations in reverse order (LIFO).
    ///
    /// Rollback is **best-effort**: a compensator that fails does NOT abort the remaining
    /// compensations (that would leave earlier side effects un-rolled-back and wedge the
    /// saga in `Compensating` forever). Every pending compensation is attempted; each
    /// success is marked executed. If all succeed the workflow ends `Compensated`; if any
    /// fail it ends `Failed` with the collected errors, never stuck mid-rollback.
    ///
    /// Compensations are themselves at-least-once (the executed marker is written after
    /// the compensator runs), so **compensators must be idempotent** — a crash between
    /// running and marking re-runs that compensation on resume.
    pub async fn compensate<F, Fut>(&mut self, mut compensator: F) -> Result<()>
    where
        F: FnMut(WorkflowCompensationRecord) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let pending = self.store.list_pending_compensations(self.id).await?;
        if pending.is_empty() {
            return Ok(());
        }

        self.store
            .update_workflow_state(self.id, WorkflowState::Compensating, None, None)
            .await?;

        let mut failures: Vec<String> = Vec::new();
        for comp in pending {
            let comp_id = comp.id;
            let comp_name = comp.compensation_name.clone();
            match compensator(comp).await {
                Ok(()) => {
                    if let Err(e) = self
                        .store
                        .mark_compensation_executed(self.id, comp_id)
                        .await
                    {
                        // The compensation ran but we could not record it; on resume it
                        // will run again (hence the idempotency requirement). Surface it.
                        failures.push(format!(
                            "compensation '{comp_name}' executed but could not be marked: {e}"
                        ));
                    }
                }
                Err(e) => {
                    // Record and continue — do NOT abort the remaining rollback.
                    failures.push(format!("compensation '{comp_name}' failed: {e}"));
                }
            }
        }

        if failures.is_empty() {
            self.store
                .update_workflow_state(self.id, WorkflowState::Compensated, None, None)
                .await?;
            Ok(())
        } else {
            let msg = format!("compensation incomplete: {}", failures.join("; "));
            // Terminal state so the saga is never stuck in `Compensating`.
            let _ = self
                .store
                .update_workflow_state(self.id, WorkflowState::Failed, None, Some(msg.clone()))
                .await;
            Err(Error::internal(msg))
        }
    }

    /// Block on a human-in-the-loop approval step until decided or timed out.
    pub async fn await_approval(
        &mut self,
        approval_name: &str,
        timeout: Duration,
    ) -> Result<ApprovalDecision> {
        let existing = self
            .store
            .create_or_get_approval(self.id, approval_name)
            .await?;
        match existing.state {
            ApprovalState::Approved => {
                return Ok(ApprovalDecision::Approved(existing.decision_payload));
            }
            ApprovalState::Rejected => {
                let reason = existing
                    .decision_payload
                    .and_then(|v| v.as_str().map(String::from));
                return Ok(ApprovalDecision::Rejected(reason));
            }
            ApprovalState::TimedOut => return Ok(ApprovalDecision::TimedOut),
            ApprovalState::Pending => {}
        }

        self.store
            .update_workflow_state(self.id, WorkflowState::WaitingApproval, None, None)
            .await?;

        let start = tokio::time::Instant::now();
        let mut poll_interval = Duration::from_millis(50);

        loop {
            if self.is_cancelled() {
                let _ = self
                    .store
                    .update_workflow_state(
                        self.id,
                        WorkflowState::Cancelled,
                        None,
                        Some("cancelled".into()),
                    )
                    .await;
                return Err(Error::internal("workflow cancelled during approval"));
            }

            if start.elapsed() >= timeout {
                match self
                    .store
                    .decide_approval(self.id, approval_name, ApprovalState::TimedOut, None)
                    .await
                {
                    Ok(()) => return Ok(ApprovalDecision::TimedOut),
                    Err(Error::Conflict(_)) => {
                        // Concurrently decided right before timeout; fall through to inspect state
                    }
                    Err(e) => return Err(e),
                }
            }

            let current = self
                .store
                .create_or_get_approval(self.id, approval_name)
                .await?;
            match current.state {
                ApprovalState::Approved => {
                    self.store
                        .update_workflow_state(self.id, WorkflowState::Running, None, None)
                        .await?;
                    return Ok(ApprovalDecision::Approved(current.decision_payload));
                }
                ApprovalState::Rejected => {
                    self.store
                        .update_workflow_state(self.id, WorkflowState::Running, None, None)
                        .await?;
                    let reason = current
                        .decision_payload
                        .and_then(|v| v.as_str().map(String::from));
                    return Ok(ApprovalDecision::Rejected(reason));
                }
                ApprovalState::TimedOut => {
                    return Ok(ApprovalDecision::TimedOut);
                }
                ApprovalState::Pending => {
                    tokio::time::sleep(poll_interval).await;
                    poll_interval =
                        std::cmp::min(poll_interval.saturating_mul(2), Duration::from_secs(1));
                }
            }
        }
    }

    /// Retrieve the memoized output of an earlier step.
    pub async fn get_step_output<T: DeserializeOwned>(&self, step_name: &str) -> Result<Option<T>> {
        match self.store.get_step(self.id, step_name).await? {
            Some(step) => match step.output {
                Some(val) => Ok(Some(serde_json::from_value(val).map_err(|e| {
                    Error::internal(format!(
                        "failed to deserialize step {step_name} output: {e}"
                    ))
                })?)),
                None => Ok(None),
            },
            None => Ok(None),
        }
    }

    /// Mark the workflow as successfully completed with final output.
    pub async fn complete<T: Serialize>(&mut self, output: &T) -> Result<()> {
        let json = serde_json::to_value(output)
            .map_err(|e| Error::internal(format!("failed to serialize workflow output: {e}")))?;
        self.store
            .update_workflow_state(self.id, WorkflowState::Completed, Some(json), None)
            .await
    }

    /// Mark the workflow as failed with an error message.
    pub async fn fail(&mut self, error: &str) -> Result<()> {
        self.store
            .update_workflow_state(
                self.id,
                WorkflowState::Failed,
                None,
                Some(error.to_string()),
            )
            .await
    }
}

/// Durable workflow orchestrator handle.
#[derive(Clone)]
pub struct Workflows {
    store: Arc<dyn WorkflowStore>,
}

impl Workflows {
    /// Create a Postgres-backed workflow orchestrator.
    pub fn new(db: Db) -> Self {
        Self {
            store: Arc::new(PostgresWorkflowStore::new(db)),
        }
    }

    /// Create an in-memory workflow orchestrator for offline unit testing.
    pub fn in_memory() -> Self {
        Self {
            store: Arc::new(InMemoryWorkflowStore::new()),
        }
    }

    /// Idempotently create workflow tables and indexes in Postgres.
    pub async fn migrate(&self) -> Result<()> {
        self.store.migrate().await
    }

    /// Start a new workflow execution with a generated UUID.
    pub async fn start(&self, name: &str, input: &impl Serialize) -> Result<WorkflowContext> {
        let id = Uuid::new_v4();
        self.start_with_id(id, name, input).await
    }

    /// Start a workflow with a deterministic or explicit execution ID.
    pub async fn start_with_id(
        &self,
        id: Uuid,
        name: &str,
        input: &impl Serialize,
    ) -> Result<WorkflowContext> {
        let val = serde_json::to_value(input)
            .map_err(|e| Error::internal(format!("failed to serialize input: {e}")))?;
        self.store.create_workflow(id, name, val).await?;
        Ok(WorkflowContext::new(
            id,
            name,
            self.store.clone(),
            CancellationToken::new(),
        ))
    }

    /// Resume an existing workflow execution from its last checkpoint.
    pub async fn resume(&self, id: Uuid) -> Result<WorkflowContext> {
        let record = self
            .store
            .get_workflow(id)
            .await?
            .ok_or_else(|| Error::not_found(format!("workflow {id} not found")))?;
        Ok(WorkflowContext::new(
            id,
            &record.name,
            self.store.clone(),
            CancellationToken::new(),
        ))
    }

    /// Approve a pending workflow approval.
    pub async fn approve(
        &self,
        workflow_id: Uuid,
        approval_name: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<()> {
        self.store
            .decide_approval(workflow_id, approval_name, ApprovalState::Approved, payload)
            .await
    }

    /// Reject a pending workflow approval with a reason.
    pub async fn reject(
        &self,
        workflow_id: Uuid,
        approval_name: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let payload = reason.map(|r| serde_json::json!(r));
        self.store
            .decide_approval(workflow_id, approval_name, ApprovalState::Rejected, payload)
            .await
    }

    /// Cancel an active workflow.
    pub async fn cancel(&self, workflow_id: Uuid) -> Result<()> {
        self.store
            .update_workflow_state(
                workflow_id,
                WorkflowState::Cancelled,
                None,
                Some("cancelled".into()),
            )
            .await
    }

    /// Fetch a workflow record.
    pub async fn get(&self, workflow_id: Uuid) -> Result<Option<WorkflowRecord>> {
        self.store.get_workflow(workflow_id).await
    }

    /// List all recorded step checkpoints for a workflow.
    pub async fn steps(&self, workflow_id: Uuid) -> Result<Vec<WorkflowStepRecord>> {
        self.store.list_steps(workflow_id).await
    }

    /// List all approvals for a workflow.
    pub async fn approvals(&self, workflow_id: Uuid) -> Result<Vec<WorkflowApprovalRecord>> {
        self.store.list_approvals(workflow_id).await
    }

    /// List all compensations for a workflow.
    pub async fn compensations(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowCompensationRecord>> {
        self.store.list_compensations(workflow_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn step_memoization_and_resumption() {
        let workflows = Workflows::in_memory();
        let wf_id = Uuid::new_v4();
        let step1_counter = Arc::new(AtomicUsize::new(0));

        // 1. First execution: step 1 completes, then process is interrupted before step 2
        {
            let mut flow = workflows
                .start_with_id(wf_id, "test_onboarding", &"input_data")
                .await
                .unwrap();

            let c = step1_counter.clone();
            let step1_out: i32 = flow
                .step("step1", || async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(42)
                })
                .await
                .unwrap();
            assert_eq!(step1_out, 42);
            assert_eq!(step1_counter.load(Ordering::SeqCst), 1);
            // Workflow terminates prematurely without calling complete()
        }

        // 2. Resuming execution: step 1 is skipped using checkpointed output, step 2 executes
        {
            let mut flow = workflows.resume(wf_id).await.unwrap();

            let c = step1_counter.clone();
            let step1_out: i32 = flow
                .step("step1", || async move {
                    // Must not be executed!
                    c.fetch_add(100, Ordering::SeqCst);
                    Ok(999)
                })
                .await
                .unwrap();
            // Returned cached output 42, counter was NOT incremented
            assert_eq!(step1_out, 42);
            assert_eq!(step1_counter.load(Ordering::SeqCst), 1);

            let step2_out: String = flow
                .step(
                    "step2",
                    || async move { Ok(format!("computed_{step1_out}")) },
                )
                .await
                .unwrap();
            assert_eq!(step2_out, "computed_42");

            flow.complete(&step2_out).await.unwrap();
        }

        let record = workflows.get(wf_id).await.unwrap().unwrap();
        assert_eq!(record.state, WorkflowState::Completed);
        assert_eq!(record.output, Some(serde_json::json!("computed_42")));

        let steps = workflows.steps(wf_id).await.unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_name, "step1");
        assert_eq!(steps[1].step_name, "step2");
    }

    #[tokio::test]
    async fn step_timeout_enforcement() {
        let workflows = Workflows::in_memory();
        let mut flow = workflows.start("slow_step_flow", &()).await.unwrap();

        let err = flow
            .step_with_timeout("slow", Duration::from_millis(10), || async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok("done".to_string())
            })
            .await
            .unwrap_err();

        assert!(matches!(err, Error::Internal(_)));
        assert!(err.to_string().contains("exceeded timeout"));

        let record = workflows.get(flow.workflow_id()).await.unwrap().unwrap();
        assert_eq!(record.state, WorkflowState::Failed);
    }

    #[tokio::test]
    async fn approval_decision_and_timeout() {
        let workflows = Workflows::in_memory();
        let mut flow = workflows.start("approval_flow", &()).await.unwrap();
        let wf_id = flow.workflow_id();

        let workflows_clone = workflows.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            workflows_clone
                .approve(
                    wf_id,
                    "manager_signoff",
                    Some(serde_json::json!({ "approved_by": "alice" })),
                )
                .await
                .unwrap();
        });

        let decision = flow
            .await_approval("manager_signoff", Duration::from_secs(1))
            .await
            .unwrap();

        match decision {
            ApprovalDecision::Approved(Some(val)) => {
                assert_eq!(val["approved_by"], "alice");
            }
            _ => panic!("expected approved decision"),
        }

        // Test timeout branch
        let mut flow2 = workflows.start("timeout_flow", &()).await.unwrap();
        let decision2 = flow2
            .await_approval("stale_signoff", Duration::from_millis(20))
            .await
            .unwrap();
        assert_eq!(decision2, ApprovalDecision::TimedOut);

        // A late approval on an already timed-out workflow must be rejected with conflict
        let late_res = workflows
            .approve(flow2.workflow_id(), "stale_signoff", None)
            .await;
        assert!(matches!(late_res.unwrap_err(), Error::Conflict(_)));
    }

    #[tokio::test]
    async fn saga_compensation_lifo_order() {
        let workflows = Workflows::in_memory();
        let mut flow = workflows.start("saga_flow", &()).await.unwrap();

        flow.register_compensation("step1", "undo_inventory", &"reserve_id_101")
            .await
            .unwrap();
        flow.register_compensation("step2", "undo_payment", &"charge_id_202")
            .await
            .unwrap();

        let executed_compensations = Arc::new(RwLock::new(Vec::new()));
        let exec_clone = executed_compensations.clone();

        flow.compensate(|comp| {
            let exec = exec_clone.clone();
            async move {
                let mut guard = exec.write().await;
                guard.push((comp.step_name, comp.compensation_name, comp.payload));
                Ok(())
            }
        })
        .await
        .unwrap();

        let executed = executed_compensations.read().await;
        assert_eq!(executed.len(), 2);
        // LIFO order: step2 undone before step1
        assert_eq!(executed[0].1, "undo_payment");
        assert_eq!(executed[1].1, "undo_inventory");

        let record = workflows.get(flow.workflow_id()).await.unwrap().unwrap();
        assert_eq!(record.state, WorkflowState::Compensated);
    }

    #[tokio::test]
    async fn failed_step_is_retried_on_resume_not_poisoned() {
        // Audit fix: a step that failed once must be RE-RUN on resume, not permanently
        // poison the workflow.
        let workflows = Workflows::in_memory();
        let attempts = Arc::new(AtomicUsize::new(0));

        let wf_id = {
            let mut flow = workflows.start("retry_flow", &()).await.unwrap();
            let id = flow.workflow_id();
            let a = attempts.clone();
            let res: Result<i32> = flow
                .step("maybe", || async move {
                    a.fetch_add(1, Ordering::SeqCst);
                    Err(Error::internal("transient failure"))
                })
                .await;
            assert!(res.is_err());
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            id
        };

        // Resume: the previously-failed step re-executes and now succeeds.
        let mut flow = workflows.resume(wf_id).await.unwrap();
        let a = attempts.clone();
        let out: i32 = flow
            .step("maybe", || async move {
                a.fetch_add(1, Ordering::SeqCst);
                Ok(7)
            })
            .await
            .unwrap();
        assert_eq!(out, 7);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "step must re-run on resume"
        );
        flow.complete(&out).await.unwrap();

        let rec = workflows.get(wf_id).await.unwrap().unwrap();
        assert_eq!(rec.state, WorkflowState::Completed);
    }

    #[tokio::test]
    async fn compensation_continues_when_one_compensator_fails() {
        // Audit fix: a failing compensator must NOT abort the rollback or wedge the saga
        // in `Compensating`. All compensations are attempted (LIFO); the saga ends in a
        // terminal state; the failed compensation stays pending for a later retry.
        let workflows = Workflows::in_memory();
        let mut flow = workflows.start("saga_fail", &()).await.unwrap();
        flow.register_compensation("step1", "undo_a", &"a")
            .await
            .unwrap();
        flow.register_compensation("step2", "undo_b", &"b")
            .await
            .unwrap();
        flow.register_compensation("step3", "undo_c", &"c")
            .await
            .unwrap();

        let seen = Arc::new(RwLock::new(Vec::new()));
        let seen2 = seen.clone();
        let res = flow
            .compensate(|comp| {
                let seen = seen2.clone();
                async move {
                    seen.write().await.push(comp.compensation_name.clone());
                    if comp.compensation_name == "undo_b" {
                        return Err(Error::internal("compensator failed"));
                    }
                    Ok(())
                }
            })
            .await;

        assert!(
            res.is_err(),
            "compensate reports failure when a compensator fails"
        );
        // All three attempted in LIFO order despite the middle failure.
        assert_eq!(
            *seen.read().await,
            vec![
                "undo_c".to_string(),
                "undo_b".to_string(),
                "undo_a".to_string()
            ]
        );
        // Terminal state — never stuck in `Compensating`.
        let rec = workflows.get(flow.workflow_id()).await.unwrap().unwrap();
        assert_eq!(rec.state, WorkflowState::Failed);
        // The two that succeeded are marked executed; the failed one remains pending.
        let pending: Vec<String> = workflows
            .compensations(flow.workflow_id())
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.executed_at.is_none())
            .map(|c| c.compensation_name)
            .collect();
        assert_eq!(pending, vec!["undo_b".to_string()]);
    }
}
