# example-database

Canonical persistence: the `api → service → repo → database` layering, a
transaction owned by the service, an event emitted after commit, and a background
task consuming that event.

```
src/
  main.rs            build AppState, routes, tasks; run.
  state.rs           AppState (Db, Events, UserService), AppEvent, type Ctx.
  domain/user.rs     User, CreateUser.
  repos/users.rs     SQL only, compile-checked, executor-generic.
  services/users.rs  validation, transaction boundary, event emission.
  api/users.rs       handlers + routes().
  tasks/audit.rs     subscribes to AppEvent.
migrations/0001_init.sql
```

Requires Postgres. Set `DATABASE_URL`, then `cargo sqlx migrate run` (or let
`AppState::init` run migrations). Query macros are checked against the schema at
build time, so set `DATABASE_URL` for compilation or commit the `.sqlx` offline
cache (`cargo sqlx prepare`).

Run: `DATABASE_URL=postgres://… cargo run -p example-database`
