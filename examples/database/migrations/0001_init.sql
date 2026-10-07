create table users (
    id          uuid primary key default gen_random_uuid(),
    email       text not null unique,
    created_at  timestamptz not null default now()
);
