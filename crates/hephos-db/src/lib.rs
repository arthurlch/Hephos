//! Hephos database layer.
//!
//! One connection pool type ([`Db`]) and one transaction type ([`Tx`]), both over
//! SQLx/Postgres. Repositories are plain functions that take a SQLx executor and
//! run compile-time-checked `sqlx::query!` / `query_as!` queries. There is no
//! ORM and no query builder — the SQL is the SQL, which is exactly what an agent
//! can read and verify.

#![forbid(unsafe_code)]

pub mod jobs;
pub mod outbox;
pub mod pool;
pub mod test;
pub mod workflow;

pub use jobs::{Job, JobOptions, JobRegistry, JobState, JobWorker, Jobs};
pub use outbox::{Outbox, OutboxDrain, OutboxEntry, OutboxState};
pub(crate) use pool::into_internal;
pub use pool::{Db, PoolConfig, Tx, is_foreign_key_violation, is_unique_violation};
pub use sqlx;
pub use workflow::{
    ApprovalDecision, ApprovalState, WorkflowApprovalRecord, WorkflowCompensationRecord,
    WorkflowContext, WorkflowRecord, WorkflowState, WorkflowStepRecord, Workflows,
};

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hephos_core::Error;

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
        assert!(!is_foreign_key_violation(&sqlx::Error::RowNotFound));
        assert!(!is_foreign_key_violation(&sqlx::Error::PoolClosed));
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
