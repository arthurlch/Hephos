# Getting started with Hephos

This guide takes you from a clean checkout to a running, tested backend, and shows the
one canonical way to add each kind of code. Hephos is **aggressively opinionated** so
that a coding agent (or you) never has to guess where something goes — every decision
below is determined by the doctrine in [`AGENTS.md`](../AGENTS.md) and
[`CONVENTIONS.md`](../CONVENTIONS.md).

There is no project generator and no boilerplate to scaffold. **You start a new app by
copying the example that is closest to what you need** — the examples are the templates,
and they are each a complete, tested app.

---

## 1. Prerequisites

- **Rust 1.85** (the repo pins the toolchain in `rust-toolchain.toml`; `rustup` will
  install it automatically).
- **Postgres 16+** — only for the database-backed examples (`database`, `auth`). The
  `rest`, `streaming`, and `websocket` examples run with no external services.

Clone and prove the baseline is green:

```sh
git clone https://github.com/arthurlch/hephos && cd hephos
make ci            # fmt-check + clippy -D warnings + tests (offline)
```

Run an example and hit it:

```sh
make run-rest      # serves on 127.0.0.1:8080
curl localhost:8080/products
curl localhost:8080/products/00000000-0000-0000-0000-000000000001
```

---

## 2. Which example do I copy?

| You want… | Copy | Needs Postgres |
| --- | --- | --- |
| A REST API over in-memory / computed data | `examples/rest` | no |
| A REST API backed by a database | `examples/database` | yes |
| Users, login, JWT + API keys, roles, ownership | `examples/auth` | yes |
| Server-sent events / streaming responses | `examples/streaming` | no |
| WebSockets | `examples/websocket` | no |
| A background task + an event subscriber | `examples/database` | yes |
| An agent with a tool, structured output | `examples/agent`, `examples/tools` | no¹ |

¹ The `agent`/`mcp` layers are **pre-1.0 / unstable** at `0.1.0` — fine to explore, not
yet API-frozen.

---

## 3. The anatomy of a Hephos app

Every app has the same shape. Responsibility — not feature — decides where code lives:

```
src/
  main.rs      compose AppState + routes + tasks, then run
  state.rs     AppState, `type Ctx`, AppEvent
  api/         HTTP handlers — one file per resource
  services/    business logic + orchestration — one file per domain concept
  repos/       SQL only — one file per table/aggregate
  domain/      plain data types, no logic, no I/O
  tasks/       background / scheduled work
  events/      AppEvent subscribers
```

The layering rule is strict and never skipped:

```
api  →  services  →  repos  →  database
         services  →  events, tasks
```

- Handlers extract input, call **one** service method, and return. No SQL, no business
  rules, under ~15 lines.
- Services make decisions and own transactions. They speak domain types and
  `hephos::Error` — never HTTP types.
- Repos run one query and return domain types. No `if` on business rules.

Every request flows through the standard middleware stack `App::run` installs:
request-id → security headers → timeout (`408`) → body limit (`413`) → a per-request
`CancellationToken` → per-group auth. You don't wire any of that; you get it.

---

## 4. Start a new app (copy → rename → strip)

Say you want a database-backed service. Copy the `database` example out of the repo:

```sh
cp -r examples/database ../my-app && cd ../my-app
```

Then:

1. **Rename the package** in `Cargo.toml` (`name = "my-app"`), and point the `hephos`
   dependency at a version or path instead of the workspace:
   ```toml
   [dependencies]
   hephos = { version = "0.1", features = ["db"] }
   # …plus tokio, serde, sqlx, uuid, time (see the example's Cargo.toml)
   ```
2. **Strip it to one resource.** Delete the files for concepts you don't need; keep one
   vertical slice (`domain/x.rs`, `repos/x.rs`, `services/x.rs`, `api/x.rs`) as your
   pattern to copy.
3. **Keep `state.rs`, `main.rs`, and `tests.rs`** — they are the spine of every app.
4. `cargo test` — the `routes_build` canary and the surface tests should pass
   immediately.

From here you extend by copying the slice you kept. The next three sections are exactly
that.

---

## 5. Add your first endpoint

The handler shape is fixed — memorize it once:

```rust
use hephos::prelude::*;
use crate::state::Ctx;
use crate::domain::product::Product;

pub async fn get(ctx: Ctx, Path(id): Path<uuid::Uuid>) -> Result<Json<Product>> {
    let product = ctx.state().catalog.get(&ctx, id).await?;
    Ok(Json(product))
}
```

Rules: `ctx: Ctx` is always first; the body is `Json<T>` of a `domain/` type; the return
is `Result<Json<T>>` (or `Result<StatusCode>` for `204`). Mount it in the resource's
`routes()`:

```rust
pub fn routes() -> Router<AppState> {
    Router::new()
        .get("/products", list)
        .get("/products/{id}", get)   // axum 0.8 brace params
        .post("/products/search", search)
}
```

Then **test it through the real router** — this is the one load-bearing test (it is what
catches a bad route composition before it panics at startup):

```rust
#[tokio::test]
async fn routes_build() {
    let _ = api::routes(); // the canary: route composition must not panic
}

#[tokio::test]
async fn get_product_ok() {
    let client = TestClient::new(api::routes(), AppState::init().await.unwrap());
    let res = client.get("/products/00000000-0000-0000-0000-000000000001").await;
    assert_eq!(res.status(), 200);
}
```

For every endpoint, write at least one success test and one failure test (assert the
status and `error.kind`). `cargo test` is the only command that actually runs a request.

---

## 6. Add a database-backed resource

Four coordinated pieces — copy `examples/database`'s `users` slice:

**A migration** (`migrations/0001_init.sql`), applied at startup by `db.migrate(...)`:

```sql
create table products (
    id          uuid primary key default gen_random_uuid(),
    name        text not null,
    price_cents integer not null,
    created_at  timestamptz not null default now()
);
```

**A repo** — SQL only, compile-checked by `sqlx`:

```rust
pub async fn find<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<Option<Product>> {
    let product = sqlx::query_as!(
        Product,
        "select id, name, price_cents, created_at from products where id = $1",
        id
    )
    .fetch_optional(exec)
    .await?;
    Ok(product)
}
```

Map a `UNIQUE` violation to `409` with `hephos::db::is_unique_violation` rather than a
racy find-then-insert. Repos return `Option<T>`; the **service** turns `None` into
`Error::not_found(...)`.

**A service** — decisions and transactions, validation via `Error::invalid`:

```rust
pub async fn get(&self, _ctx: &Ctx, id: Uuid) -> Result<Product> {
    ProductRepo::find(self.db.pool(), id)
        .await?
        .ok_or_else(|| Error::not_found("product"))
}
```

**Wire it into `AppState`** (`state.rs`), where the `Db` is connected and migrated once.

Database tests are `#[ignore]`d so `cargo test` stays green offline; run them against a
real Postgres with `DATABASE_URL=… cargo test -- --ignored`.

---

## 7. Add authentication & authorization

Copy `examples/auth`. One seam guards a route group; handlers never parse auth headers:

```rust
let protected = products::routes().authenticated(move |token| {
    let auth = auth.clone();
    async move { auth.authenticate(&token).await } // resolves a JWT or an API key
});
Router::new().merge(public::routes()).merge(protected)
```

Inside a protected handler, authorize from `ctx` with the narrowest guard:

- `ctx.require_user()?` — any authenticated caller.
- `ctx.require_role("admin")?` — a coarse capability.
- `ctx.require_owner(resource.owner_id)?` — the IDOR guard: load the row first, then
  check the caller owns it. (See `examples/auth`'s `/notes`.)

All three return the narrowest-correct status automatically: `401` when anonymous, `403`
when authenticated but denied.

---

## 8. Proof: the two canonical tasks

Hephos is designed so these resolve to one obvious implementation (ARCHITECTURE §24):

1. **"An authenticated `POST /users` that writes to the DB and emits an event."** Route
   → `api/users.rs::create`, mounted in `api::routes()`. The group is already
   `authenticated`, so the handler calls `ctx.require_role("admin")?` then
   `ctx.state().users.create(&ctx, input)`. The service opens a `Tx`, calls
   `UserRepo::create`, emits `AppEvent::UserCreated`, commits. Realized in
   `examples/database` + `examples/auth`.

2. **"An agent with a tool that searches the DB and returns structured results."** Tool
   → `tools/`, implementing `Tool` with typed input/output and a captured `Db`; agent →
   `agents/`, `.system(...).tool(SearchUsers::new(db))`; results via
   `agent.run_typed::<Vec<UserSummary>>(...)`. Realized in `examples/tools` +
   `examples/agent`.

If you ever can't tell where code goes, that is a bug in the doctrine — open an issue;
don't improvise.

---

## 9. Before you ship

Run the full gate (the same commands CI runs):

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --all-features
```

A green type-check is **not** evidence the app runs — only a router-driving test (or
launching it and hitting it) proves wiring. If you changed a `sqlx` query, run
`cargo sqlx prepare`.

---

## Where to go next

- [`CONVENTIONS.md`](../CONVENTIONS.md) — the canonical code structure for every layer,
  with full examples.
- [`AGENTS.md`](../AGENTS.md) — the normative operating manual (what an agent must follow).
- [`ARCHITECTURE.md`](../ARCHITECTURE.md) — why the system is shaped this way.
- [`MILESTONES.md`](./project/MILESTONES.md) — the roadmap toward a production `1.0`.
