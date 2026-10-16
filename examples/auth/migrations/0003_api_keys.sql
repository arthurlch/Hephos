-- Machine-to-machine API keys: hashed, scoped (roles), and revocable. Only the
-- SHA-256 hash of the key is stored; the raw `hephos_sk_…` value is shown once at
-- creation. A key resolves to the same `Identity` as a user JWT, with `id` = the key
-- id and `roles` = the key's scopes.
create table api_keys (
    id          uuid primary key default gen_random_uuid(),
    name        text not null,
    key_hash    text not null unique,
    roles       text[] not null default '{}',
    expires_at  timestamptz,
    revoked_at  timestamptz,
    created_at  timestamptz not null default now()
);
