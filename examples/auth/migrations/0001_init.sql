create table users (
    id             uuid primary key default gen_random_uuid(),
    email          text not null unique,
    password_hash  text not null,
    roles          text[] not null default '{}',
    created_at     timestamptz not null default now()
);
