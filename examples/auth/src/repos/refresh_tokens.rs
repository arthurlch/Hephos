use hephos::prelude::*;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// A stored refresh token, identified by the hash of the opaque value the client
/// holds. The raw token never lives in the database.
pub struct StoredRefreshToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub expires_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}

pub struct RefreshTokenRepo;

impl RefreshTokenRepo {
    pub async fn insert<'e, E: PgExecutor<'e>>(
        exec: E,
        user_id: Uuid,
        token_hash: &str,
        expires_at: OffsetDateTime,
    ) -> Result<()> {
        sqlx::query!(
            "insert into refresh_tokens (user_id, token_hash, expires_at) values ($1, $2, $3)",
            user_id,
            token_hash,
            expires_at
        )
        .execute(exec)
        .await?;
        Ok(())
    }

    pub async fn find_by_hash<'e, E: PgExecutor<'e>>(
        exec: E,
        token_hash: &str,
    ) -> Result<Option<StoredRefreshToken>> {
        let row = sqlx::query_as!(
            StoredRefreshToken,
            "select id, user_id, expires_at, revoked_at from refresh_tokens \
             where token_hash = $1",
            token_hash
        )
        .fetch_optional(exec)
        .await?;
        Ok(row)
    }

    pub async fn revoke<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<()> {
        sqlx::query!(
            "update refresh_tokens set revoked_at = now() where id = $1 and revoked_at is null",
            id
        )
        .execute(exec)
        .await?;
        Ok(())
    }

    /// Idempotent: revoking an unknown or already-revoked token is a no-op, so logout
    /// does not reveal whether a token existed.
    pub async fn revoke_by_hash<'e, E: PgExecutor<'e>>(exec: E, token_hash: &str) -> Result<()> {
        sqlx::query!(
            "update refresh_tokens set revoked_at = now() \
             where token_hash = $1 and revoked_at is null",
            token_hash
        )
        .execute(exec)
        .await?;
        Ok(())
    }

    /// Revoke every live token for a user — used on reuse (theft) detection.
    pub async fn revoke_all_for_user<'e, E: PgExecutor<'e>>(exec: E, user_id: Uuid) -> Result<()> {
        sqlx::query!(
            "update refresh_tokens set revoked_at = now() \
             where user_id = $1 and revoked_at is null",
            user_id
        )
        .execute(exec)
        .await?;
        Ok(())
    }
}
