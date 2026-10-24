//! Postgres-backed durable background job queue.
//!
//! Provides durable background job processing aligned with the [`hephos_core::Task`]
//! API. Jobs survive worker restarts, support exponential backoff retries, dead-letter
//! queueing, idempotency keys, visibility timeouts, and immediate execution via
//! Postgres `LISTEN/NOTIFY`.

use std::collections::HashMap;
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

/// The lifecycle state of a durable job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// The job is queued and ready to be claimed.
    Ready,
    /// The job is currently claimed by a worker and actively running.
    Running,
    /// The job completed successfully.
    Completed,
    /// The job failed and will be retried after exponential backoff.
    Failed,
    /// The job exceeded its maximum attempts and was moved to the dead-letter queue.
    DeadLetter,
}

impl JobState {
    /// Convert state to the database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Ready => "ready",
            JobState::Running => "running",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::DeadLetter => "dead_letter",
        }
    }
}

impl std::str::FromStr for JobState {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "ready" => Ok(JobState::Ready),
            "running" => Ok(JobState::Running),
            "completed" => Ok(JobState::Completed),
            "failed" => Ok(JobState::Failed),
            "dead_letter" => Ok(JobState::DeadLetter),
            _ => Err(()),
        }
    }
}

/// A durable job record persisted in Postgres.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// Unique job identifier.
    pub id: Uuid,
    /// Named job identifier matching a registered handler.
    pub name: String,
    /// JSON payload passed to the job handler.
    pub payload: serde_json::Value,
    /// Current execution state.
    pub state: JobState,
    /// Number of genuine handler failures so far (incremented on failure, NOT on each
    /// delivery). A worker crash/reclaim does not count, so transient crashes do not
    /// burn the retry budget. Trade-off: a handler that crashes the worker process every
    /// time is redelivered without ever incrementing — bound such poison with a worker
    /// supervisor or a visibility-timeout tuned to surface it.
    pub attempts: i32,
    /// Maximum handler failures before the job is moved to the dead-letter queue.
    pub max_attempts: i32,
    /// Timestamp when this job is eligible to run.
    pub run_at: OffsetDateTime,
    /// Timestamp when the current worker claimed this job.
    pub locked_at: Option<OffsetDateTime>,
    /// Identifier of the worker holding the lock.
    pub locked_by: Option<String>,
    /// Last error message if a previous attempt failed.
    pub last_error: Option<String>,
    /// Unique idempotency key to prevent duplicate job creation.
    pub idempotency_key: Option<String>,
    /// Optional recurring interval in seconds for scheduled recurring jobs.
    pub recurring_interval_secs: Option<i64>,
    /// Creation timestamp.
    pub created_at: OffsetDateTime,
    /// Last updated timestamp.
    pub updated_at: OffsetDateTime,
}

/// Options configuring job execution, scheduling, and deduplication.
#[derive(Debug, Clone)]
pub struct JobOptions {
    /// Timestamp when this job should run (defaults to now).
    pub run_at: Option<OffsetDateTime>,
    /// Maximum attempts before moving to dead-letter (defaults to 5).
    pub max_attempts: i32,
    /// Unique key for deduplication. Enqueuing with the same key returns the existing job.
    pub idempotency_key: Option<String>,
    /// Optional recurring interval. If set, completion automatically schedules the next run.
    pub recurring_interval: Option<Duration>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            run_at: None,
            max_attempts: 5,
            idempotency_key: None,
            recurring_interval: None,
        }
    }
}

impl JobOptions {
    /// Create new default job options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Delay job execution by a relative duration from now.
    pub fn delay(mut self, duration: Duration) -> Self {
        let now = OffsetDateTime::now_utc();
        self.run_at = Some(now + time::Duration::seconds(duration.as_secs() as i64));
        self
    }

    /// Schedule job execution at a specific timestamp.
    pub fn run_at(mut self, timestamp: OffsetDateTime) -> Self {
        self.run_at = Some(timestamp);
        self
    }

    /// Set maximum attempts before dead-lettering.
    pub fn max_attempts(mut self, max: i32) -> Self {
        self.max_attempts = max.max(1);
        self
    }

    /// Set an idempotency key for deduplication.
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Configure this job as a recurring job with a fixed interval.
    pub fn recurring(mut self, interval: Duration) -> Self {
        self.recurring_interval = Some(interval);
        self
    }
}

/// Postgres-backed job queue handle.
#[derive(Clone)]
pub struct Jobs {
    db: Db,
}

impl Jobs {
    /// Create a new `Jobs` handle using the provided database connection pool.
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Idempotently create the `hephos_jobs` table and supporting indexes.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::query("SELECT pg_advisory_lock(910248231)")
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        let statements = [
            r#"
            CREATE TABLE IF NOT EXISTS hephos_jobs (
                id UUID PRIMARY KEY,
                name TEXT NOT NULL,
                payload JSONB NOT NULL,
                state TEXT NOT NULL DEFAULT 'ready',
                attempts INT NOT NULL DEFAULT 0,
                max_attempts INT NOT NULL DEFAULT 5,
                run_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                locked_at TIMESTAMPTZ,
                locked_by TEXT,
                last_error TEXT,
                idempotency_key TEXT UNIQUE,
                recurring_interval_secs BIGINT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_hephos_jobs_ready
                ON hephos_jobs (run_at ASC)
                WHERE state = 'ready'
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_hephos_jobs_locked
                ON hephos_jobs (locked_at)
                WHERE state = 'running'
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_hephos_jobs_dead_letter
                ON hephos_jobs (created_at DESC)
                WHERE state = 'dead_letter'
            "#,
        ];

        for stmt in statements {
            if let Err(e) = sqlx::query(stmt).execute(self.db.pool()).await {
                let _ = sqlx::query("SELECT pg_advisory_unlock(910248231)")
                    .execute(self.db.pool())
                    .await;
                return Err(into_internal(e));
            }
        }

        sqlx::query("SELECT pg_advisory_unlock(910248231)")
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        Ok(())
    }

    /// Enqueue a new job outside a transaction.
    ///
    /// If an `idempotency_key` is supplied and a job with that key already exists,
    /// the existing job is returned without re-enqueuing.
    pub async fn enqueue<T: Serialize>(
        &self,
        name: &str,
        payload: &T,
        options: JobOptions,
    ) -> Result<Job> {
        let id = Uuid::new_v4();
        let payload_json = serde_json::to_string(payload)
            .map_err(|e| Error::invalid(format!("failed to serialize job payload: {e}")))?;
        let run_at = options.run_at.unwrap_or_else(OffsetDateTime::now_utc);
        let recurring_secs = options.recurring_interval.map(|d| d.as_secs() as i64);

        let query = r#"
            INSERT INTO hephos_jobs (
                id, name, payload, state, attempts, max_attempts,
                run_at, idempotency_key, recurring_interval_secs, created_at, updated_at
            )
            VALUES (
                $1, $2, $3::jsonb, 'ready', 0, $4,
                $5, $6, $7, NOW(), NOW()
            )
            ON CONFLICT (idempotency_key) DO UPDATE
                SET updated_at = hephos_jobs.updated_at
            RETURNING
                id, name, payload::text AS payload, state, attempts, max_attempts,
                run_at, locked_at, locked_by, last_error, idempotency_key,
                recurring_interval_secs, created_at, updated_at;
        "#;

        let row = sqlx::query(query)
            .bind(id)
            .bind(name)
            .bind(&payload_json)
            .bind(options.max_attempts)
            .bind(run_at)
            .bind(&options.idempotency_key)
            .bind(recurring_secs)
            .fetch_one(self.db.pool())
            .await
            .map_err(into_internal)?;

        let job = parse_job_row(&row)?;

        // Notify workers that a new job is available (best-effort notification)
        let _ = sqlx::query("NOTIFY hephos_jobs, 'new_job'")
            .execute(self.db.pool())
            .await;

        Ok(job)
    }

    /// Enqueue a job within an active SQL transaction.
    ///
    /// The job will become visible to workers only after the transaction commits.
    pub async fn enqueue_tx<T: Serialize>(
        &self,
        tx: &mut Tx,
        name: &str,
        payload: &T,
        options: JobOptions,
    ) -> Result<Job> {
        let id = Uuid::new_v4();
        let payload_json = serde_json::to_string(payload)
            .map_err(|e| Error::invalid(format!("failed to serialize job payload: {e}")))?;
        let run_at = options.run_at.unwrap_or_else(OffsetDateTime::now_utc);
        let recurring_secs = options.recurring_interval.map(|d| d.as_secs() as i64);

        let query = r#"
            INSERT INTO hephos_jobs (
                id, name, payload, state, attempts, max_attempts,
                run_at, idempotency_key, recurring_interval_secs, created_at, updated_at
            )
            VALUES (
                $1, $2, $3::jsonb, 'ready', 0, $4,
                $5, $6, $7, NOW(), NOW()
            )
            ON CONFLICT (idempotency_key) DO UPDATE
                SET updated_at = hephos_jobs.updated_at
            RETURNING
                id, name, payload::text AS payload, state, attempts, max_attempts,
                run_at, locked_at, locked_by, last_error, idempotency_key,
                recurring_interval_secs, created_at, updated_at;
        "#;

        let row = sqlx::query(query)
            .bind(id)
            .bind(name)
            .bind(&payload_json)
            .bind(options.max_attempts)
            .bind(run_at)
            .bind(&options.idempotency_key)
            .bind(recurring_secs)
            .fetch_one(tx.exec())
            .await
            .map_err(into_internal)?;

        let job = parse_job_row(&row)?;
        Ok(job)
    }

    /// Find a job by its unique ID.
    pub async fn find(&self, id: Uuid) -> Result<Option<Job>> {
        let query = r#"
            SELECT
                id, name, payload::text AS payload, state, attempts, max_attempts,
                run_at, locked_at, locked_by, last_error, idempotency_key,
                recurring_interval_secs, created_at, updated_at
            FROM hephos_jobs
            WHERE id = $1;
        "#;

        let maybe_row = sqlx::query(query)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(into_internal)?;

        maybe_row.map(|row| parse_job_row(&row)).transpose()
    }

    /// Find a job by its idempotency key.
    pub async fn find_by_idempotency_key(&self, key: &str) -> Result<Option<Job>> {
        let query = r#"
            SELECT
                id, name, payload::text AS payload, state, attempts, max_attempts,
                run_at, locked_at, locked_by, last_error, idempotency_key,
                recurring_interval_secs, created_at, updated_at
            FROM hephos_jobs
            WHERE idempotency_key = $1;
        "#;

        let maybe_row = sqlx::query(query)
            .bind(key)
            .fetch_optional(self.db.pool())
            .await
            .map_err(into_internal)?;

        maybe_row.map(|row| parse_job_row(&row)).transpose()
    }

    /// Atomically claim up to `limit` available jobs using `FOR UPDATE SKIP LOCKED`.
    ///
    /// Also reclaims orphaned jobs that have been locked longer than `visibility_timeout`.
    pub async fn claim(
        &self,
        worker_id: &str,
        limit: usize,
        visibility_timeout: Duration,
    ) -> Result<Vec<Job>> {
        let now = OffsetDateTime::now_utc();
        let timeout_secs = visibility_timeout.as_secs() as i64;
        let stale_before = now - time::Duration::seconds(timeout_secs);

        let query = r#"
            WITH available AS (
                SELECT id
                FROM hephos_jobs
                WHERE (state = 'ready' AND run_at <= $1)
                   OR (state = 'running' AND locked_at < $2)
                ORDER BY run_at ASC
                LIMIT $3
                FOR UPDATE SKIP LOCKED
            )
            -- NOTE: `attempts` is NOT incremented here. It counts genuine handler
            -- failures (incremented in `fail`), not deliveries, so a worker that crashes
            -- after claiming does not burn a retry and cause premature dead-lettering.
            UPDATE hephos_jobs j
            SET state = 'running',
                locked_at = $1,
                locked_by = $4,
                updated_at = $1
            FROM available
            WHERE j.id = available.id
            RETURNING
                j.id, j.name, j.payload::text AS payload, j.state, j.attempts, j.max_attempts,
                j.run_at, j.locked_at, j.locked_by, j.last_error, j.idempotency_key,
                j.recurring_interval_secs, j.created_at, j.updated_at;
        "#;

        let rows = sqlx::query(query)
            .bind(now)
            .bind(stale_before)
            .bind(limit as i64)
            .bind(worker_id)
            .fetch_all(self.db.pool())
            .await
            .map_err(into_internal)?;

        rows.iter().map(parse_job_row).collect()
    }

    /// Mark a job as completed. If configured with a recurring interval, automatically
    /// schedules the next run.
    pub async fn complete(&self, id: Uuid) -> Result<()> {
        let job = self
            .find(id)
            .await?
            .ok_or_else(|| Error::not_found("job"))?;

        if let Some(interval_secs) = job.recurring_interval_secs {
            let next_run = OffsetDateTime::now_utc() + time::Duration::seconds(interval_secs);
            let query = r#"
                UPDATE hephos_jobs
                SET state = 'ready',
                    run_at = $2,
                    locked_at = NULL,
                    locked_by = NULL,
                    last_error = NULL,
                    updated_at = NOW()
                WHERE id = $1;
            "#;
            sqlx::query(query)
                .bind(id)
                .bind(next_run)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
        } else {
            let query = r#"
                UPDATE hephos_jobs
                SET state = 'completed',
                    locked_at = NULL,
                    locked_by = NULL,
                    updated_at = NOW()
                WHERE id = $1;
            "#;
            sqlx::query(query)
                .bind(id)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
        }

        Ok(())
    }

    /// Mark a job as failed.
    ///
    /// If `is_dead_letter` is true, transitions state directly to `dead_letter`.
    /// Otherwise, schedules a retry at `now + backoff` in state `ready`.
    pub async fn fail(
        &self,
        id: Uuid,
        error: &str,
        backoff: Duration,
        is_dead_letter: bool,
    ) -> Result<()> {
        if is_dead_letter {
            // Count this terminal failure too, so a dead-lettered job's `attempts`
            // reflects the true number of handler failures.
            let query = r#"
                UPDATE hephos_jobs
                SET state = 'dead_letter',
                    attempts = attempts + 1,
                    last_error = $2,
                    locked_at = NULL,
                    locked_by = NULL,
                    updated_at = NOW()
                WHERE id = $1;
            "#;
            sqlx::query(query)
                .bind(id)
                .bind(error)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
        } else {
            let retry_at = OffsetDateTime::now_utc()
                + time::Duration::milliseconds(backoff.as_millis().max(1) as i64);
            // Increment on each real failure — this is what bounds retries.
            let query = r#"
                UPDATE hephos_jobs
                SET state = 'ready',
                    attempts = attempts + 1,
                    run_at = $2,
                    last_error = $3,
                    locked_at = NULL,
                    locked_by = NULL,
                    updated_at = NOW()
                WHERE id = $1;
            "#;
            sqlx::query(query)
                .bind(id)
                .bind(retry_at)
                .bind(error)
                .execute(self.db.pool())
                .await
                .map_err(into_internal)?;
        }

        Ok(())
    }

    /// Retrieve dead-letter jobs for inspection or debugging.
    pub async fn dead_letter_jobs(&self, limit: usize) -> Result<Vec<Job>> {
        let query = r#"
            SELECT
                id, name, payload::text AS payload, state, attempts, max_attempts,
                run_at, locked_at, locked_by, last_error, idempotency_key,
                recurring_interval_secs, created_at, updated_at
            FROM hephos_jobs
            WHERE state = 'dead_letter'
            ORDER BY updated_at DESC
            LIMIT $1;
        "#;

        let rows = sqlx::query(query)
            .bind(limit as i64)
            .fetch_all(self.db.pool())
            .await
            .map_err(into_internal)?;

        rows.iter().map(parse_job_row).collect()
    }

    /// Reset a dead-letter job back to `ready` to re-attempt processing.
    pub async fn retry_dead_letter(&self, id: Uuid) -> Result<()> {
        let query = r#"
            UPDATE hephos_jobs
            SET state = 'ready',
                run_at = NOW(),
                attempts = 0,
                last_error = NULL,
                locked_at = NULL,
                locked_by = NULL,
                updated_at = NOW()
            WHERE id = $1 AND state = 'dead_letter';
        "#;

        let result = sqlx::query(query)
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        if result.rows_affected() == 0 {
            return Err(Error::not_found("dead_letter_job"));
        }

        let _ = sqlx::query("NOTIFY hephos_jobs, 'retry'")
            .execute(self.db.pool())
            .await;

        Ok(())
    }

    /// Reclaim orphaned jobs whose visibility timeout has expired.
    pub async fn reap_orphans(&self, visibility_timeout: Duration) -> Result<u64> {
        let timeout_secs = visibility_timeout.as_secs() as i64;
        let stale_before = OffsetDateTime::now_utc() - time::Duration::seconds(timeout_secs);

        let query = r#"
            UPDATE hephos_jobs
            SET state = 'ready',
                locked_at = NULL,
                locked_by = NULL,
                updated_at = NOW()
            WHERE state = 'running' AND locked_at < $1;
        "#;

        let result = sqlx::query(query)
            .bind(stale_before)
            .execute(self.db.pool())
            .await
            .map_err(into_internal)?;

        Ok(result.rows_affected())
    }
}

/// Helper to decode a SQL row into a [`Job`].
fn parse_job_row(row: &sqlx::postgres::PgRow) -> Result<Job> {
    let id: Uuid = row.try_get("id").map_err(into_internal)?;
    let name: String = row.try_get("name").map_err(into_internal)?;
    let payload_str: String = row.try_get("payload").map_err(into_internal)?;
    let payload: serde_json::Value = serde_json::from_str(&payload_str)
        .map_err(|e| Error::invalid(format!("invalid JSON in job payload: {e}")))?;

    let state_str: String = row.try_get("state").map_err(into_internal)?;
    let state: JobState = state_str
        .parse()
        .map_err(|()| Error::internal(format!("unknown job state in database: {state_str}")))?;

    let attempts: i32 = row.try_get("attempts").map_err(into_internal)?;
    let max_attempts: i32 = row.try_get("max_attempts").map_err(into_internal)?;
    let run_at: OffsetDateTime = row.try_get("run_at").map_err(into_internal)?;
    let locked_at: Option<OffsetDateTime> = row.try_get("locked_at").map_err(into_internal)?;
    let locked_by: Option<String> = row.try_get("locked_by").map_err(into_internal)?;
    let last_error: Option<String> = row.try_get("last_error").map_err(into_internal)?;
    let idempotency_key: Option<String> = row.try_get("idempotency_key").map_err(into_internal)?;
    let recurring_interval_secs: Option<i64> = row
        .try_get("recurring_interval_secs")
        .map_err(into_internal)?;
    let created_at: OffsetDateTime = row.try_get("created_at").map_err(into_internal)?;
    let updated_at: OffsetDateTime = row.try_get("updated_at").map_err(into_internal)?;

    Ok(Job {
        id,
        name,
        payload,
        state,
        attempts,
        max_attempts,
        run_at,
        locked_at,
        locked_by,
        last_error,
        idempotency_key,
        recurring_interval_secs,
        created_at,
        updated_at,
    })
}

/// Handler function signature for job execution.
pub type JobHandlerFn<S> = Arc<dyn Fn(Ctx<S>, Job) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Registry of named job handlers.
pub struct JobRegistry<S> {
    handlers: HashMap<String, JobHandlerFn<S>>,
}

impl<S> Default for JobRegistry<S> {
    fn default() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
}

impl<S: Clone + Send + Sync + 'static> JobRegistry<S> {
    /// Create a new empty job registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for jobs matching `name`.
    pub fn register<F, Fut>(mut self, name: &str, handler: F) -> Self
    where
        F: Fn(Ctx<S>, Job) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let wrapped: JobHandlerFn<S> = Arc::new(move |ctx, job| Box::pin(handler(ctx, job)));
        self.handlers.insert(name.to_string(), wrapped);
        self
    }

    /// Look up a handler by name.
    pub fn get(&self, name: &str) -> Option<JobHandlerFn<S>> {
        self.handlers.get(name).cloned()
    }
}

/// Background worker task that drives durable job execution.
///
/// Implements [`Task`], so it integrates directly with [`hephos_core::App`].
pub struct JobWorker<S> {
    jobs: Jobs,
    worker_id: String,
    registry: Arc<JobRegistry<S>>,
    poll_interval: Duration,
    visibility_timeout: Duration,
    base_backoff: Duration,
    max_backoff: Duration,
    batch_size: usize,
}

impl<S: Clone + Send + Sync + 'static> JobWorker<S> {
    /// Create a new job worker with the given `Jobs` handle and handler registry.
    pub fn new(jobs: Jobs, registry: JobRegistry<S>) -> Self {
        let worker_id = format!("worker-{}", Uuid::new_v4().simple());
        Self {
            jobs,
            worker_id,
            registry: Arc::new(registry),
            poll_interval: Duration::from_secs(5),
            visibility_timeout: Duration::from_secs(60),
            base_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(3600),
            batch_size: 10,
        }
    }

    /// Configure the periodic polling interval (fallback when `LISTEN` is idle).
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Configure the visibility timeout before uncompleted jobs are re-queued.
    pub fn visibility_timeout(mut self, timeout: Duration) -> Self {
        self.visibility_timeout = timeout;
        self
    }

    /// Configure the initial exponential backoff duration.
    pub fn base_backoff(mut self, backoff: Duration) -> Self {
        self.base_backoff = backoff;
        self
    }

    /// Configure the maximum backoff ceiling.
    pub fn max_backoff(mut self, max: Duration) -> Self {
        self.max_backoff = max;
        self
    }

    /// Configure maximum jobs claimed per batch.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    /// Calculate the exponential backoff duration for a given attempt.
    fn calculate_backoff(&self, attempts: i32) -> Duration {
        let exponent = (attempts.saturating_sub(1)) as u32;
        let factor = 2u64.saturating_pow(exponent.min(16));
        let backoff_secs = self.base_backoff.as_secs().saturating_mul(factor);
        Duration::from_secs(backoff_secs.min(self.max_backoff.as_secs()))
    }

    /// Process a batch of available jobs.
    async fn process_batch(&self, ctx: &Ctx<S>) -> Result<usize> {
        let claimed = self
            .jobs
            .claim(&self.worker_id, self.batch_size, self.visibility_timeout)
            .await?;

        let count = claimed.len();
        for job in claimed {
            if ctx.cancel_token().is_cancelled() {
                break;
            }

            let handler = match self.registry.get(&job.name) {
                Some(h) => h,
                None => {
                    tracing::warn!(job_id = %job.id, job_name = %job.name, "no handler registered for job; dead-lettering");
                    self.jobs
                        .fail(
                            job.id,
                            &format!("no handler registered for job: {}", job.name),
                            Duration::ZERO,
                            true,
                        )
                        .await?;
                    continue;
                }
            };

            tracing::info!(job_id = %job.id, job_name = %job.name, attempt = job.attempts, "executing job");

            match handler(ctx.clone(), job.clone()).await {
                Ok(()) => {
                    tracing::info!(job_id = %job.id, job_name = %job.name, "job completed successfully");
                    if let Err(e) = self.jobs.complete(job.id).await {
                        tracing::error!(job_id = %job.id, error = %e, "failed to mark job completed");
                    }
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    // `job.attempts` is the count of PRIOR handler failures (claim no
                    // longer increments). This failure is the (attempts + 1)th, so
                    // dead-letter once it reaches the cap.
                    let is_dead = job.attempts + 1 >= job.max_attempts;
                    let backoff = self.calculate_backoff(job.attempts);

                    if is_dead {
                        tracing::error!(
                            job_id = %job.id,
                            job_name = %job.name,
                            attempts = job.attempts,
                            error = %err_msg,
                            "job exhausted max attempts, moving to dead letter"
                        );
                    } else {
                        tracing::warn!(
                            job_id = %job.id,
                            job_name = %job.name,
                            attempts = job.attempts,
                            backoff_secs = backoff.as_secs(),
                            error = %err_msg,
                            "job failed, scheduling retry"
                        );
                    }

                    if let Err(e) = self.jobs.fail(job.id, &err_msg, backoff, is_dead).await {
                        tracing::error!(job_id = %job.id, error = %e, "failed to update failed job state");
                    }
                }
            }
        }

        Ok(count)
    }
}

impl<S: Clone + Send + Sync + 'static> Task<S> for JobWorker<S> {
    fn name(&self) -> &'static str {
        "hephos_job_worker"
    }

    async fn run(&self, ctx: Ctx<S>) -> Result<()> {
        tracing::info!(worker_id = %self.worker_id, "starting durable job worker");

        // Try setting up PgListener for NOTIFY events; fall back to pure polling if not connected
        let mut listener = match PgListener::connect_with(self.jobs.db.pool()).await {
            Ok(mut l) => {
                if let Err(e) = l.listen("hephos_jobs").await {
                    tracing::warn!(error = %e, "failed to listen on hephos_jobs channel, using timer polling");
                }
                Some(l)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to establish PgListener, using timer polling");
                None
            }
        };

        let mut ticker = tokio::time::interval(self.poll_interval);

        loop {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => {
                    tracing::info!(worker_id = %self.worker_id, "job worker received cancellation, shutting down");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(e) = self.process_batch(&ctx).await {
                        tracing::error!(error = %e, "error processing job batch on tick");
                    }
                }
                _ = async {
                    if let Some(ref mut l) = listener {
                        let _ = l.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    if let Err(e) = self.process_batch(&ctx).await {
                        tracing::error!(error = %e, "error processing job batch on notification");
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
    fn job_state_roundtrip_and_serialization() {
        let states = [
            (JobState::Ready, "ready"),
            (JobState::Running, "running"),
            (JobState::Completed, "completed"),
            (JobState::Failed, "failed"),
            (JobState::DeadLetter, "dead_letter"),
        ];

        for (state, s) in states {
            assert_eq!(state.as_str(), s);
            assert_eq!(s.parse::<JobState>(), Ok(state));

            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{s}\""));
            let parsed: JobState = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, state);
        }

        assert_eq!("unknown".parse::<JobState>(), Err(()));
    }

    #[test]
    fn job_options_builder_defaults_and_customization() {
        let opts = JobOptions::new()
            .delay(Duration::from_secs(60))
            .max_attempts(10)
            .idempotency_key("order-123")
            .recurring(Duration::from_secs(300));

        assert!(opts.run_at.is_some());
        assert_eq!(opts.max_attempts, 10);
        assert_eq!(opts.idempotency_key.as_deref(), Some("order-123"));
        assert_eq!(opts.recurring_interval, Some(Duration::from_secs(300)));
    }

    #[tokio::test]
    async fn exponential_backoff_calculation() {
        let db = Db::connect_lazy("postgres://localhost/test").unwrap();
        let jobs = Jobs::new(db);
        let registry = JobRegistry::<()>::new();
        let worker = JobWorker::new(jobs, registry)
            .base_backoff(Duration::from_secs(2))
            .max_backoff(Duration::from_secs(30));

        assert_eq!(worker.calculate_backoff(1), Duration::from_secs(2));
        assert_eq!(worker.calculate_backoff(2), Duration::from_secs(4));
        assert_eq!(worker.calculate_backoff(3), Duration::from_secs(8));
        assert_eq!(worker.calculate_backoff(4), Duration::from_secs(16));
        // Capped at max_backoff (30s)
        assert_eq!(worker.calculate_backoff(5), Duration::from_secs(30));
        assert_eq!(worker.calculate_backoff(10), Duration::from_secs(30));
    }

    #[test]
    fn job_registry_registration_and_lookup() {
        let registry = JobRegistry::<()>::new()
            .register("send_email", |_ctx, _job| async { Ok(()) })
            .register("generate_report", |_ctx, _job| async { Ok(()) });

        assert!(registry.get("send_email").is_some());
        assert!(registry.get("generate_report").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn worker_task_name_and_tuning() {
        let db = Db::connect_lazy("postgres://localhost/test").unwrap();
        let jobs = Jobs::new(db);
        let registry = JobRegistry::<()>::new();
        let worker = JobWorker::new(jobs, registry)
            .poll_interval(Duration::from_secs(1))
            .visibility_timeout(Duration::from_secs(120))
            .batch_size(25);

        assert_eq!(worker.name(), "hephos_job_worker");
        assert_eq!(worker.poll_interval, Duration::from_secs(1));
        assert_eq!(worker.visibility_timeout, Duration::from_secs(120));
        assert_eq!(worker.batch_size, 25);
    }
}
