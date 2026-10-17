-- A per-user resource, used to demonstrate resource-ownership authorization
-- (`ctx.require_owner`): a note belongs to the user who created it, and only that
-- user may read, update, or delete it.
create table notes (
    id          uuid primary key default gen_random_uuid(),
    owner_id    uuid not null references users (id) on delete cascade,
    body        text not null,
    created_at  timestamptz not null default now()
);

create index notes_owner_id_idx on notes (owner_id);
