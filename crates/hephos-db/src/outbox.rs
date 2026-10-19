//! Transactional outbox for durable at-least-once event delivery.
//!
//! Enables atomic database operations and domain event publishing within the same
//! SQL transaction. A background [`OutboxDrain`] task continuously drains pending
//! events, guaranteeing events survive application crashes between database commit
//! and event dispatch.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hephos_core::{Ctx, Error, Result, Task};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::postgres::PgListener;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{Db, Tx, into_internal};

/// Delivery state of an outbox event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    /// Pending dispatch.
    Pending,
    /// Successfully dispatched.
    Dispatched,
    /// Failed delivery after maximum attempts.
    Failed,
}

impl OutboxState {
    /// Convert state to the database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            OutboxState::Pending => "pending",
            OutboxState::Dispatched => "dispatched",
            OutboxState::Failed => "failed",
        }
    }
}

impl std::str::FromStr for OutboxState {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(OutboxState::Pending),
            "dispatched" => Ok(OutboxState::Dispatched),
            "failed" => Ok(OutboxState::Failed),
            _ => Err(()),
        }
    }
}

/// A persisted outbox entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEntry {
    /// Unique event identifier.
    pub id: Uuid,
    /// Type name of the event (e.g. `user_created`, `order_placed`).
    pub event_type: String,
    /// Event payload as structured JSON.
    pub payload: serde_json::Value,
    /// Current delivery state.
    pub state: OutboxState,
    /// Number of delivery attempts made.
    pub attempts: i32,
    /// Last error encountered during dispatch.
    pub last_error: Option<String>,
    /// Creation timestamp.
    pub created_at: OffsetDateTime,
    /// Dispatched timestamp (if dispatched).
    pub dispatched_at: Option<OffsetDateTime>,
}

/// Transactional outbox handle for inserting and querying events.
#[derive(Clone)]
pub struct Outbox {
    db: Db,
}

impl Outbox {
    /// Create a new `Outbox` handle.
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Idempotently create the `hephos_outbox` table and indexes.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::query("SELECT pg_advisory_lock(910248232)")
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        let statements = [
            r#"
            CREATE TABLE IF NOT EXISTS hephos_outbox (
                id UUID PRIMARY KEY,
                event_type TEXT NOT NULL,
                payload JSONB NOT NULL,
                state TEXT NOT NULL DEFAULT 'pending',
                attempts INT NOT NULL DEFAULT 0,
                last_error TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                dispatched_at TIMESTAMPTZ
            )
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_hephos_outbox_pending
                ON hephos_outbox (created_at ASC)
                WHERE state = 'pending'
            "#,
        ];

        for stmt in statements {
            if let Err(e) = sqlx::query(stmt).execute(self.db.pool()).await {
                let _ = sqlx::query("SELECT pg_advisory_unlock(910248232)")
                    .execute(self.db.pool())
                    .await;
                return Err(into_internal(e));
            }
        }

        sqlx::query("SELECT pg_advisory_unlock(910248232)")
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        Ok(())
    }

    /// Enqueue an event into the outbox within an active SQL transaction.
    ///
    /// This guarantees that the event is committed if and only if the surrounding
    /// domain business transaction commits.
    pub async fn enqueue_tx<T: Serialize>(
        &self,
        tx: &mut Tx,
        event_type: &str,
        payload: &T,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let payload_json = serde_json::to_string(payload)
            .map_err(|e| Error::invalid(format!("failed to serialize outbox payload: {e}")))?;

        let query = r#"
            INSERT INTO hephos_outbox (id, event_type, payload, state, attempts, created_at)
            VALUES ($1, $2, $3::jsonb, 'pending', 0, NOW());
        "#;

        sqlx::query(query)
            .bind(id)
            .bind(event_type)
            .bind(&payload_json)
            .execute(tx.exec())
            .await
            .map_err(into_internal)?;

        Ok(id)
    }

    /// Find an outbox entry by ID.
    pub async fn find(&self, id: Uuid) -> Result<Option<OutboxEntry>> {
        let query = r#"
            SELECT id, event_type, payload::text AS payload, state, attempts, last_error, created_at, dispatched_at
            FROM hephos_outbox
            WHERE id = $1;
        "#;

        let maybe_row = sqlx::query(query)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(into_internal)?;

        maybe_row.map(|row| parse_outbox_row(&row)).transpose()
    }

    /// Atomically claim up to `limit` pending outbox entries.
    pub async fn claim_pending(&self, limit: usize) -> Result<Vec<OutboxEntry>> {
        let query = r#"
            WITH pending AS (
                SELECT id
                FROM hephos_outbox
                WHERE state = 'pending'
                ORDER BY created_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            UPDATE hephos_outbox o
            SET attempts = o.attempts + 1
            FROM pending
            WHERE o.id = pending.id
            RETURNING o.id, o.event_type, o.payload::text AS payload, o.state, o.attempts, o.last_error, o.created_at, o.dispatched_at;
        "#;

        let rows = sqlx::query(query)
            .bind(limit as i64)
            .fetch_all(self.db.pool())
            .await
            .map_err(into_internal)?;

        rows.iter().map(parse_outbox_row).collect()
    }

    /// Mark an outbox entry as dispatched.
    pub async fn mark_dispatched(&self, id: Uuid) -> Result<()> {
        let query = r#"
            UPDATE hephos_outbox
            SET state = 'dispatched',
                dispatched_at = NOW()
            WHERE id = $1;
        "#;

        sqlx::query(query)
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        Ok(())
    }

    /// Mark an entry as failed.
    pub async fn mark_failed(&self, id: Uuid, error: &str, is_terminal: bool) -> Result<()> {
        let state = if is_terminal { "failed" } else { "pending" };
        let query = r#"
            UPDATE hephos_outbox
            SET state = $2,
                last_error = $3
            WHERE id = $1;
        "#;

        sqlx::query(query)
            .bind(id)
            .bind(state)
            .bind(error)
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        Ok(())
    }
}

/// Helper to decode a SQL row into an [`OutboxEntry`].
fn parse_outbox_row(row: &sqlx::postgres::PgRow) -> Result<OutboxEntry> {
    let id: Uuid = row.try_get("id").map_err(into_internal)?;
    let event_type: String = row.try_get("event_type").map_err(into_internal)?;
    let payload_str: String = row.try_get("payload").map_err(into_internal)?;
    let payload: serde_json::Value = serde_json::from_str(&payload_str)
        .map_err(|e| Error::invalid(format!("invalid JSON in outbox payload: {e}")))?;

    let state_str: String = row.try_get("state").map_err(into_internal)?;
    let state: OutboxState = state_str
        .parse()
        .map_err(|()| Error::internal(format!("unknown outbox state in database: {state_str}")))?;

    let attempts: i32 = row.try_get("attempts").map_err(into_internal)?;
    let last_error: Option<String> = row.try_get("last_error").map_err(into_internal)?;
    let created_at: OffsetDateTime = row.try_get("created_at").map_err(into_internal)?;
    let dispatched_at: Option<OffsetDateTime> =
        row.try_get("dispatched_at").map_err(into_internal)?;

    Ok(OutboxEntry {
        id,
        event_type,
        payload,
        state,
        attempts,
        last_error,
        created_at,
        dispatched_at,
    })
}

/// Dispatcher callback signature for delivering outbox events.
pub type OutboxDispatcherFn<S> =
    Arc<dyn Fn(Ctx<S>, OutboxEntry) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Background task that continuously drains pending entries from [`Outbox`].
///
/// Implements [`Task`], so it can be passed directly to [`hephos_core::App::task`].
pub struct OutboxDrain<S> {
    outbox: Outbox,
    dispatcher: OutboxDispatcherFn<S>,
    poll_interval: Duration,
    batch_size: usize,
    max_attempts: i32,
}

impl<S: Clone + Send + Sync + 'static> OutboxDrain<S> {
    /// Create a new `OutboxDrain` task with a dispatch function.
    pub fn new<F, Fut>(outbox: Outbox, dispatcher: F) -> Self
    where
        F: Fn(Ctx<S>, OutboxEntry) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let wrapped: OutboxDispatcherFn<S> =
            Arc::new(move |ctx, entry| Box::pin(dispatcher(ctx, entry)));
        Self {
            outbox,
            dispatcher: wrapped,
            poll_interval: Duration::from_secs(2),
            batch_size: 20,
            max_attempts: 5,
        }
    }

    /// Configure polling interval.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Configure batch size per claim.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    /// Configure maximum delivery attempts before marking as failed.
    pub fn max_attempts(mut self, max: i32) -> Self {
        self.max_attempts = max.max(1);
        self
    }

    /// Drain a batch of pending entries.
    async fn drain_batch(&self, ctx: &Ctx<S>) -> Result<usize> {
        let entries = self.outbox.claim_pending(self.batch_size).await?;
        let count = entries.len();

        for entry in entries {
            if ctx.cancel_token().is_cancelled() {
                break;
            }

            tracing::info!(
                entry_id = %entry.id,
                event_type = %entry.event_type,
                attempts = entry.attempts,
                "dispatching outbox entry"
            );

            match (self.dispatcher)(ctx.clone(), entry.clone()).await {
                Ok(()) => {
                    tracing::info!(entry_id = %entry.id, "outbox entry dispatched successfully");
                    if let Err(e) = self.outbox.mark_dispatched(entry.id).await {
                        tracing::error!(entry_id = %entry.id, error = %e, "failed to mark outbox entry dispatched");
                    }
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    let is_terminal = entry.attempts >= self.max_attempts;
                    tracing::error!(
                        entry_id = %entry.id,
                        event_type = %entry.event_type,
                        attempts = entry.attempts,
                        is_terminal,
                        error = %err_msg,
                        "failed to dispatch outbox entry"
                    );

                    if let Err(e) = self
                        .outbox
                        .mark_failed(entry.id, &err_msg, is_terminal)
                        .await
                    {
                        tracing::error!(entry_id = %entry.id, error = %e, "failed to record outbox failure");
                    }
                }
            }
        }

        Ok(count)
    }
}

impl<S: Clone + Send + Sync + 'static> Task<S> for OutboxDrain<S> {
    fn name(&self) -> &'static str {
        "hephos_outbox_drain"
    }

    async fn run(&self, ctx: Ctx<S>) -> Result<()> {
        tracing::info!("starting transactional outbox drain task");

        let mut listener = match PgListener::connect_with(self.outbox.db.pool()).await {
            Ok(mut l) => {
                if let Err(e) = l.listen("hephos_outbox").await {
                    tracing::warn!(error = %e, "failed to listen on hephos_outbox, using polling");
                }
                Some(l)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to connect PgListener for outbox, using polling");
                None
            }
        };

        let mut ticker = tokio::time::interval(self.poll_interval);

        loop {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => {
                    tracing::info!("outbox drain received cancellation token, shutting down");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(e) = self.drain_batch(&ctx).await {
                        tracing::error!(error = %e, "error draining outbox batch on tick");
                    }
                }
                _ = async {
                    if let Some(ref mut l) = listener {
                        let _ = l.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    if let Err(e) = self.drain_batch(&ctx).await {
                        tracing::error!(error = %e, "error draining outbox batch on notification");
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_state_roundtrip_and_serialization() {
        let states = [
            (OutboxState::Pending, "pending"),
            (OutboxState::Dispatched, "dispatched"),
            (OutboxState::Failed, "failed"),
        ];

        for (state, s) in states {
            assert_eq!(state.as_str(), s);
            assert_eq!(s.parse::<OutboxState>(), Ok(state));

            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{s}\""));
            let parsed: OutboxState = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, state);
        }

        assert_eq!("unknown".parse::<OutboxState>(), Err(()));
    }

    #[tokio::test]
    async fn outbox_drain_builder_and_task_name() {
        let db = Db::connect_lazy("postgres://localhost/test").unwrap();
        let outbox = Outbox::new(db);
        let drain = OutboxDrain::<()>::new(outbox, |_ctx, _entry| async { Ok(()) })
            .poll_interval(Duration::from_millis(500))
            .batch_size(50)
            .max_attempts(3);

        assert_eq!(drain.name(), "hephos_outbox_drain");
        assert_eq!(drain.poll_interval, Duration::from_millis(500));
        assert_eq!(drain.batch_size, 50);
        assert_eq!(drain.max_attempts, 3);
    }
}
