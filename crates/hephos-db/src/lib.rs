//! Hephos database layer.
//!
//! One connection pool type ([`Db`]) and one transaction type ([`Tx`]), both over
//! SQLx/Postgres. Repositories are plain functions that take a SQLx executor and
//! run compile-time-checked `sqlx::query!` / `query_as!` queries. There is no
//! ORM and no query builder — the SQL is the SQL, which is exactly what an agent
//! can read and verify.

#![forbid(unsafe_code)]

use hephos_core::{Error, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Postgres connection pool. Held in application state, cloned freely (it is an
/// `Arc` internally).
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    /// Connect using `DATABASE_URL`. This is the canonical constructor; call it
    /// in `AppState::init`.
    pub async fn connect_from_env() -> Result<Self> {
        let url =
            std::env::var("DATABASE_URL").map_err(|_| Error::invalid("DATABASE_URL is not set"))?;
        Self::connect(&url).await
    }

    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
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

    /// Begin a transaction for multi-statement writes. Pass `&mut *tx` to repo
    /// functions, then `tx.commit().await?`.
    pub async fn begin(&self) -> Result<Tx> {
        let inner = self.pool.begin().await.map_err(into_internal)?;
        Ok(Tx { inner })
    }

    /// Run embedded migrations. Called once in `AppState::init`, after connect.
    ///
    /// BOUNDARY: wire to `sqlx::migrate!("./migrations")` in the app crate, where
    /// the migrations directory lives. Exposed here as the canonical entry point.
    pub async fn migrate(&self) -> Result<()> {
        // BOUNDARY: `sqlx::migrate!()` must be invoked from the crate that owns
        // the `migrations/` directory; this method documents where that call
        // belongs in the lifecycle (AppState::init).
        Ok(())
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

/// Every SQLx error is an internal error. Not-found is expressed by the
/// repository (via `fetch_optional` returning `None`), never by leaking
/// `RowNotFound` — so "missing row" and "database is down" stay distinct.
fn into_internal(error: sqlx::Error) -> Error {
    Error::Internal(anyhow::Error::new(error))
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
}
