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

pub mod jobs;
pub mod outbox;

pub use jobs::{Job, JobOptions, JobRegistry, JobState, JobWorker, Jobs};
pub use outbox::{Outbox, OutboxDrain, OutboxEntry, OutboxState};

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

            // The database now exists; if connect/migrate fails, drop it so a failed
            // setup leaves no orphaned database or open pool behind.
            let setup = async {
                let db = Db::connect(&format!("{server}/{name}")).await?;
                db.migrate(migrator).await?;
                Ok::<Db, Error>(db)
            }
            .await;

            match setup {
                Ok(db) => Ok(TestDb { admin, name, db }),
                Err(error) => {
                    let _ =
                        sqlx::query(&format!("drop database if exists \"{name}\" with (force)"))
                            .execute(&admin)
                            .await;
                    admin.close().await;
                    Err(error)
                }
            }
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
    pub(crate) fn split_server(url: &str) -> Result<(String, String)> {
        let without_query = url.split('?').next().unwrap_or(url);
        let idx = without_query
            .rfind('/')
            .ok_or_else(|| Error::invalid("DATABASE_URL has no database path"))?;
        let server = &without_query[..idx];
        let database = &without_query[idx + 1..];
        // Guard the shapes `rfind` can't disambiguate: a missing `scheme://` authority
        // or an empty database name (e.g. a trailing slash). TestDb targets standard
        // `postgres://[user[:pass]@]host[:port]/db` URLs.
        if !server.contains("://") || database.is_empty() {
            return Err(Error::invalid(
                "DATABASE_URL must look like postgres://host/dbname",
            ));
        }
        Ok((server.to_string(), database.to_string()))
    }
}

/// Whether a SQLx error is a Postgres unique-constraint violation (SQLSTATE `23505`).
///
/// Repositories use this to map a lost insert race to `Error::conflict` (409) instead
/// of a generic 500 — the `find`-then-`insert` pre-check is a fast path, not the
/// correctness guarantee; the database constraint is:
///
/// ```ignore
/// let user = sqlx::query_as!(User, "insert into users ...")
///     .fetch_one(exec)
///     .await
///     .map_err(|e| if is_unique_violation(&e) {
///         Error::conflict("email already registered")
///     } else {
///         e.into()
///     })?;
/// ```
pub fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

/// Every SQLx error is an internal error. Not-found is expressed by the
/// repository (via `fetch_optional` returning `None`), never by leaking
/// `RowNotFound` — so "missing row" and "database is down" stay distinct.
// The one place pool options are built, shared by `connect_with` and `connect_lazy`
// so the security-relevant statement-timeout install never diverges between them.
fn pool_options(config: &PoolConfig) -> PgPoolOptions {
    // Clamp to ≥1ms: Postgres treats `statement_timeout = 0` as *disabled*, the
    // opposite of the intent, which a sub-millisecond `Duration` would trigger.
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

// Build via `From<anyhow::Error>` rather than the `Error::Internal` variant literal:
// `Error` is `#[non_exhaustive]`, so sibling crates construct it through its conversions.
fn into_internal(error: sqlx::Error) -> Error {
    anyhow::Error::new(error).into()
}

fn into_internal_migrate(error: sqlx::migrate::MigrateError) -> Error {
    anyhow::Error::new(error).into()
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

    // A Tokio context is required: the lazy pool spawns a background maintenance task.
    #[tokio::test]
    async fn connect_lazy_builds_without_a_server_and_rejects_bad_urls() {
        // No connection is opened, so a well-formed URL succeeds even offline.
        assert!(Db::connect_lazy("postgres://user@localhost/db").is_ok());
        // Only the URL is parsed here, so a malformed one is the one failure mode.
        assert!(Db::connect_lazy("not-a-url").is_err());
    }

    #[test]
    fn split_server_validation() {
        let (server, db) =
            crate::test::split_server("postgres://admin:pass@localhost:5432/my_app").unwrap();
        assert_eq!(server, "postgres://admin:pass@localhost:5432");
        assert_eq!(db, "my_app");

        assert!(crate::test::split_server("postgres://localhost:5432/").is_err());
        assert!(crate::test::split_server("postgres://localhost").is_err());
        assert!(crate::test::split_server("invalid-url").is_err());
    }

    #[test]
    fn is_unique_violation_returns_false_for_non_db_errors() {
        assert!(!is_unique_violation(&sqlx::Error::RowNotFound));
        assert!(!is_unique_violation(&sqlx::Error::PoolClosed));
    }

    #[tokio::test]
    async fn connect_lazy_with_custom_pool_config() {
        let config = PoolConfig {
            max_connections: 32,
            acquire_timeout: Duration::from_secs(5),
            statement_timeout: Duration::from_secs(15),
        };
        let db = Db::connect_lazy_with("postgres://user@localhost/db", config).unwrap();
        assert_eq!(db.pool().options().get_max_connections(), 32);

        assert!(Db::connect_lazy_with("invalid-dsn", config).is_err());
    }

    #[test]
    fn pool_config_debug_and_copy() {
        let config = PoolConfig {
            max_connections: 8,
            acquire_timeout: Duration::from_secs(2),
            statement_timeout: Duration::from_secs(4),
        };
        let copied = config;
        assert_eq!(copied.max_connections, 8);
        assert_eq!(copied.acquire_timeout, Duration::from_secs(2));
        assert_eq!(copied.statement_timeout, Duration::from_secs(4));

        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("max_connections: 8"));
    }
}
