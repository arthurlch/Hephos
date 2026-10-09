//! Hephos database layer.
//!
//! One connection pool type ([`Db`]) and one transaction type ([`Tx`]), both over
//! SQLx/Postgres. Repositories are plain functions that take a SQLx executor and
//! run compile-time-checked `sqlx::query!` / `query_as!` queries. There is no
//! ORM and no query builder — the SQL is the SQL, which is exactly what an agent
//! can read and verify.

#![forbid(unsafe_code)]

use std::time::Duration;

use hephos_core::{Error, Result};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Connection-pool tuning. Read from the environment by [`Db::connect_from_env`];
/// bounded by default so a stuck query cannot exhaust the pool.
///
/// | Variable                           | Default | Meaning                            |
/// | ---------------------------------- | ------- | ---------------------------------- |
/// | `HEPHOS_DB_MAX_CONNECTIONS`         | `16`    | Max pooled connections.            |
/// | `HEPHOS_DB_ACQUIRE_TIMEOUT_SECS`    | `10`    | Max wait to acquire a connection.  |
/// | `HEPHOS_DB_STATEMENT_TIMEOUT_SECS`  | `30`    | Server-side per-statement timeout. |
#[derive(Debug, Clone, Copy)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub acquire_timeout: Duration,
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
    fn from_env() -> Result<Self> {
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
        let statement_timeout_ms = config.statement_timeout.as_millis() as u64;
        let pool = PgPoolOptions::new()
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
            .connect(url)
            .await
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
}

/// An open transaction. Drop without commit rolls back.
pub struct Tx {
    inner: sqlx::Transaction<'static, sqlx::Postgres>,
}

impl Tx {
    pub async fn commit(self) -> Result<()> {
        self.inner.commit().await.map_err(into_internal)
    }

    pub async fn rollback(self) -> Result<()> {
        self.inner.rollback().await.map_err(into_internal)
    }

    /// The underlying executor, for repo calls: `UserRepo::create(tx.exec(), &new)`.
    pub fn exec(&mut self) -> &mut sqlx::PgConnection {
        &mut self.inner
    }
}

/// Test fixtures for repositories and services.
///
/// [`test::TestDb`] stands up a uniquely-named, migrated database per test and drops
/// it on `cleanup`, so tests leave no residue and run in isolation. For service/repo
/// tests that only need rollback, the canonical pattern is a plain `db.begin()` …
/// `tx.rollback()` around the test body.
pub mod test {
    use super::{Db, Error, Migrator, PgPool, Result, into_internal};
    use uuid::Uuid;

    /// A disposable, migrated database for one test. Call [`TestDb::cleanup`] at the
    /// end to drop it.
    pub struct TestDb {
        admin: PgPool,
        name: String,
        db: Db,
    }

    impl TestDb {
        /// Create a fresh database named `hephos_test_<uuid>`, run `migrator`, and
        /// return a handle to it. Uses `DATABASE_URL` for the server location.
        pub async fn create(migrator: &Migrator) -> Result<Self> {
            let base = std::env::var("DATABASE_URL")
                .map_err(|_| Error::invalid("DATABASE_URL is not set"))?;
            let (server, _db) = split_server(&base)?;
            let name = format!("hephos_test_{}", Uuid::new_v4().simple());

            let admin = PgPool::connect(&format!("{server}/postgres"))
                .await
                .map_err(into_internal)?;
            sqlx::query(&format!("create database \"{name}\""))
                .execute(&admin)
                .await
                .map_err(into_internal)?;

            let db = Db::connect(&format!("{server}/{name}")).await?;
            db.migrate(migrator).await?;
            Ok(TestDb { admin, name, db })
        }

        pub fn db(&self) -> &Db {
            &self.db
        }

        /// Drop the database and close connections. Leaves no residue.
        pub async fn cleanup(self) -> Result<()> {
            self.db.pool().close().await;
            sqlx::query(&format!(
                "drop database if exists \"{}\" with (force)",
                self.name
            ))
            .execute(&self.admin)
            .await
            .map_err(into_internal)?;
            self.admin.close().await;
            Ok(())
        }
    }

    /// Split `postgres://user:pass@host:port/dbname[?params]` into the server prefix
    /// (everything up to the database name) and the database name.
    fn split_server(url: &str) -> Result<(String, String)> {
        let without_query = url.split('?').next().unwrap_or(url);
        let idx = without_query
            .rfind('/')
            .ok_or_else(|| Error::invalid("DATABASE_URL has no database path"))?;
        Ok((
            without_query[..idx].to_string(),
            without_query[idx + 1..].to_string(),
        ))
    }
}

/// Every SQLx error is an internal error. Not-found is expressed by the
/// repository (via `fetch_optional` returning `None`), never by leaking
/// `RowNotFound` — so "missing row" and "database is down" stay distinct.
fn into_internal(error: sqlx::Error) -> Error {
    Error::Internal(anyhow::Error::new(error))
}

fn into_internal_migrate(error: sqlx::migrate::MigrateError) -> Error {
    Error::Internal(anyhow::Error::new(error))
}

fn env_or<T>(key: &str, default: T) -> Result<T>
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

#[cfg(test)]
mod tests {
    use super::*;

    // Depends on `hephos-core`'s `sqlx` feature (enabled by this crate). Proves the
    // documented repository pattern — `sqlx::query!(...).await?` — actually compiles
    // and maps a SQLx error to an opaque `Internal`.
    #[test]
    fn sqlx_error_converts_to_internal() {
        let error: Error = sqlx::Error::RowNotFound.into();
        assert!(matches!(error, Error::Internal(_)));
    }

    #[test]
    fn pool_config_defaults_are_bounded() {
        let c = PoolConfig::default();
        assert_eq!(c.max_connections, 16);
        assert_eq!(c.acquire_timeout, Duration::from_secs(10));
        assert_eq!(c.statement_timeout, Duration::from_secs(30));
    }
}
