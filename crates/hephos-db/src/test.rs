//! Test fixtures for repositories and services.
//!
//! [`TestDb`] stands up a uniquely-named, migrated database per test and drops
//! it on `cleanup`, so tests leave no residue and run in isolation. For service/repo
//! tests that only need rollback, the canonical pattern is a plain `db.begin()` …
//! `tx.rollback()` around the test body.

use hephos_core::{Error, Result};
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::Db;
use crate::pool::into_internal;

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
        let base =
            std::env::var("DATABASE_URL").map_err(|_| Error::invalid("DATABASE_URL is not set"))?;
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
                let _ = sqlx::query(&format!("drop database if exists \"{name}\" with (force)"))
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
    if !server.contains("://") || database.is_empty() {
        return Err(Error::invalid(
            "DATABASE_URL must look like postgres://host/dbname",
        ));
    }
    Ok((server.to_string(), database.to_string()))
}
