use hephos::prelude::*;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::user::{CreateUser, User};

/// SQL only. Executor-generic so the same function works with a pool or a
/// transaction. One query per method. No business logic.
pub struct UserRepo;

impl UserRepo {
    pub async fn find<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<Option<User>> {
        let user = sqlx::query_as!(
            User,
            "select id, email, created_at from users where id = $1",
            id
        )
        .fetch_optional(exec)
        .await?;
        Ok(user)
    }

    pub async fn find_by_email<'e, E: PgExecutor<'e>>(
        exec: E,
        email: &str,
    ) -> Result<Option<User>> {
        let user = sqlx::query_as!(
            User,
            "select id, email, created_at from users where email = $1",
            email
        )
        .fetch_optional(exec)
        .await?;
        Ok(user)
    }

    pub async fn create<'e, E: PgExecutor<'e>>(exec: E, input: &CreateUser) -> Result<User> {
        let user = sqlx::query_as!(
            User,
            "insert into users (email) values ($1) returning id, email, created_at",
            input.email
        )
        .fetch_one(exec)
        .await?;
        Ok(user)
    }
}
