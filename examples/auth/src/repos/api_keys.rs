use hephos::prelude::*;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// An active API key resolved from a presented key's hash. Revoked keys are filtered
/// out in SQL, so a row here is live (modulo the expiry check the caller applies).
pub struct ActiveApiKey {
    pub id: Uuid,
    pub roles: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
}

pub struct ApiKeyRepo;

impl ApiKeyRepo {
    pub async fn insert<'e, E: PgExecutor<'e>>(
        exec: E,
        name: &str,
        key_hash: &str,
        roles: &[String],
        expires_at: Option<OffsetDateTime>,
    ) -> Result<Uuid> {
        let row = sqlx::query!(
            "insert into api_keys (name, key_hash, roles, expires_at) \
             values ($1, $2, $3, $4) returning id",
            name,
            key_hash,
            roles,
            expires_at
        )
        .fetch_one(exec)
        .await?;
        Ok(row.id)
    }

    pub async fn find_active_by_hash<'e, E: PgExecutor<'e>>(
        exec: E,
        key_hash: &str,
    ) -> Result<Option<ActiveApiKey>> {
        let row = sqlx::query_as!(
            ActiveApiKey,
            "select id, roles, expires_at from api_keys \
             where key_hash = $1 and revoked_at is null",
            key_hash
        )
        .fetch_optional(exec)
        .await?;
        Ok(row)
    }

    /// Returns whether a live key was revoked, so the service can map an unknown or
    /// already-revoked id to `404` rather than silently succeeding.
    pub async fn revoke<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<bool> {
        let result = sqlx::query!(
            "update api_keys set revoked_at = now() where id = $1 and revoked_at is null",
            id
        )
        .execute(exec)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
