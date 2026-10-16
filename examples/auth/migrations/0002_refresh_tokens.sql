-- Opaque refresh tokens. Only the SHA-256 hash is stored, so a database dump cannot
-- be replayed as a live session. Rotation revokes the presented token and issues a
-- new one; `revoked_at` also powers logout and reuse (theft) detection.
create table refresh_tokens (
    id          uuid primary key default gen_random_uuid(),
    user_id     uuid not null references users (id) on delete cascade,
    token_hash  text not null unique,
    expires_at  timestamptz not null,
    revoked_at  timestamptz,
    created_at  timestamptz not null default now()
);

create index refresh_tokens_user_id_idx on refresh_tokens (user_id);
