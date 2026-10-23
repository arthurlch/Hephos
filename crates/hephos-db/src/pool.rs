//! Postgres connection pool and transaction management.

use std::time::Duration;

use hephos_core::{Error, Result};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::jobs::Jobs;
use crate::outbox::Outbox;
use crate::workflow::Workflows;

/// Connection-pool tuning. Read from the environment by [`Db::connect_from_env`];
/// bounded by default so a stuck query cannot exhaust the pool.
///
/// | Variable                           | Default | Meaning                            |
/// | ---------------------------------- | ------- | ---------------------------------- |
/// | `HEPHOS_DB_MAX_CONNECTIONS`         | `16`    | Max pooled connections.            |
/// | `HEPHOS_DB_ACQUIRE_TIMEOUT_SECS`    | `10`    | Max wait to acquire a connection.  |
/// | `HEPHOS_DB_STATEMENT_TIMEOUT_SECS`  | `30`    | Server-side per-statement timeout. |
// `#[non_exhaustive]`: pool tuning grows (idle timeout, min connections, …); a struct
// literal in app code must not break. Start from `PoolConfig::default()` and adjust.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct PoolConfig {
    /// Maximum number of pooled connections.
    pub max_connections: u32,
    /// Maximum time to wait for a free connection before erroring.
    pub acquire_timeout: Duration,
    /// Server-side `statement_timeout` installed on every connection, so one stuck
    /// query cannot pin a connection indefinitely.
    pub statement_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_connections: 16,
            acquire_timeout: Duration::from_secs(10),
            statement_timeout: Duration::from_secs(30),
        }
    }
}

impl PoolConfig {
    pub(crate) fn from_env() -> Result<Self> {
        let d = PoolConfig::default();
        Ok(PoolConfig {
            max_connections: env_or("HEPHOS_DB_MAX_CONNECTIONS", d.max_connections)?,
            acquire_timeout: Duration::from_secs(env_or(
                "HEPHOS_DB_ACQUIRE_TIMEOUT_SECS",
                d.acquire_timeout.as_secs(),
            )?),
            statement_timeout: Duration::from_secs(env_or(
                "HEPHOS_DB_STATEMENT_TIMEOUT_SECS",
                d.statement_timeout.as_secs(),
            )?),
        })
    }
}

/// Postgres connection pool. Held in application state, cloned freely (it is an
/// `Arc` internally).
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    /// Connect using `DATABASE_URL` and the pool config from the environment. This
    /// is the canonical constructor; call it in `AppState::init`.
    pub async fn connect_from_env() -> Result<Self> {
        let url =
            std::env::var("DATABASE_URL").map_err(|_| Error::invalid("DATABASE_URL is not set"))?;
        Self::connect_with(&url, PoolConfig::from_env()?).await
    }

    /// Connect with default pool config.
    pub async fn connect(url: &str) -> Result<Self> {
        Self::connect_with(url, PoolConfig::default()).await
    }

    /// Connect with explicit pool config. Installs the server-side statement timeout
    /// on every connection, so one pathological query cannot pin a connection.
    pub async fn connect_with(url: &str, config: PoolConfig) -> Result<Self> {
        let pool = pool_options(&config)
            .connect(url)
            .await
            .map_err(into_internal)?;
        Ok(Db { pool })
    }

    /// Build a pool **without** opening a connection, using the default [`PoolConfig`].
    /// SQLx connects lazily on first use, so startup never blocks on the database and
    /// offline tests (e.g. the `routes_build` canary) can construct application state
    /// without a live server. See [`Db::connect_lazy_with`] to pass explicit config.
    pub fn connect_lazy(url: &str) -> Result<Self> {
        Self::connect_lazy_with(url, PoolConfig::default())
    }

    /// Build a lazy pool with explicit config — the lazy counterpart of
    /// [`Db::connect_with`]. Shares its options (including the server-side statement
    /// timeout, installed on each connection when it is actually opened). No connection
    /// is attempted here, so this fails only on a URL SQLx cannot parse.
    pub fn connect_lazy_with(url: &str, config: PoolConfig) -> Result<Self> {
        let pool = pool_options(&config)
            .connect_lazy(url)
            .map_err(into_internal)?;
        Ok(Db { pool })
    }

    /// The pool, for read queries and single-statement writes:
    /// `UserRepo::find(db.pool(), id).await?`.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Begin a transaction for multi-statement writes. Pass `tx.exec()` to repo
    /// functions, then `tx.commit().await?`.
    pub async fn begin(&self) -> Result<Tx> {
        let inner = self.pool.begin().await.map_err(into_internal)?;
        Ok(Tx { inner })
    }

    /// Run embedded migrations to the latest version. The app owns the migrations,
    /// so it passes the `Migrator` from its own crate:
    ///
    /// ```ignore
    /// db.migrate(&sqlx::migrate!("./migrations")).await?;
    /// ```
    ///
    /// Idempotent: already-applied migrations are skipped.
    pub async fn migrate(&self, migrator: &Migrator) -> Result<()> {
        migrator
            .run(&self.pool)
            .await
            .map_err(into_internal_migrate)
    }

    /// Ping the database by executing `SELECT 1`.
    ///
    /// Returns `Ok(())` if the database is reachable and accepting queries,
    /// or an error (such as [`Error::Internal`]) if connection acquisition or execution fails.
    /// Wire into [`hephos_core::App::readiness`] to gate `/ready` on database connectivity:
    ///
    /// ```ignore
    /// let db = state.db.clone();
    /// App::new(state)
    ///     .routes(api::routes())
    ///     .readiness(move || {
    ///         let db = db.clone();
    ///         async move { db.ping().await }
    ///     })
    /// ```
    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(into_internal)?;
        Ok(())
    }

    /// Access the Postgres-backed durable job queue.
    pub fn jobs(&self) -> Jobs {
        Jobs::new(self.clone())
    }

    /// Access the transactional outbox handle.
    pub fn outbox(&self) -> Outbox {
        Outbox::new(self.clone())
    }

    /// Access the Postgres-checkpointed durable workflows orchestrator.
    pub fn workflows(&self) -> Workflows {
        Workflows::new(self.clone())
    }
}

/// An open transaction. Drop without commit rolls back.
pub struct Tx {
    inner: sqlx::Transaction<'static, sqlx::Postgres>,
}

impl Tx {
    /// Commit the transaction, persisting every statement run through [`Tx::exec`].
    pub async fn commit(self) -> Result<()> {
        self.inner.commit().await.map_err(into_internal)
    }

    /// Roll back the transaction explicitly. Dropping a `Tx` without committing also
    /// rolls back, so this is only needed to roll back early and surface any error.
    pub async fn rollback(self) -> Result<()> {
        self.inner.rollback().await.map_err(into_internal)
    }

    /// The underlying executor, for repo calls: `UserRepo::create(tx.exec(), &new)`.
    pub fn exec(&mut self) -> &mut sqlx::PgConnection {
        &mut self.inner
    }
}

/// Whether a SQLx error is a Postgres unique-constraint violation (SQLSTATE `23505`).
pub fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

/// Whether a SQLx error is a Postgres foreign-key-constraint violation (SQLSTATE `23503`).
pub fn is_foreign_key_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.code().as_deref() == Some("23503"))
}

pub(crate) fn pool_options(config: &PoolConfig) -> PgPoolOptions {
    let statement_timeout_ms = (config.statement_timeout.as_millis() as u64).max(1);
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                sqlx::query(&format!("SET statement_timeout = {statement_timeout_ms}"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
}

pub(crate) fn into_internal(error: sqlx::Error) -> Error {
    anyhow::Error::new(error).into()
}

pub(crate) fn into_internal_migrate(error: sqlx::migrate::MigrateError) -> Error {
    anyhow::Error::new(error).into()
}

pub(crate) fn env_or<T>(key: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
{
    match std::env::var(key) {
        Ok(value) => value
            .parse()
            .map_err(|_| Error::invalid(format!("{key} is not a valid value: {value}"))),
        Err(_) => Ok(default),
    }
}
