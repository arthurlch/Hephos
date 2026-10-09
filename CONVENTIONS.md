# CONVENTIONS.md

The definitive, mechanical convention guide. Where `CLAUDE.md` gives rules and
`ARCHITECTURE.md` gives reasons, this gives the exact shape of every artifact. When
you write code, find the relevant section here and copy the shape.

The single meta-rule: **one canonical way.** If this document specifies a shape,
there is no other acceptable shape.

---

## 1. Naming

### 1.1 Crates & packages
- Framework crates: `hephos`, `hephos-core`, `hephos-<capability>` (kebab-case).
- Example/app crates: the app name, kebab-case.

### 1.2 Modules & files
- Modules are `snake_case`, singular when they model one concept (`user`), plural
  when they are a collection of peers (`services`, `repos`, `tools`).
- One resource/concept per file. `api/users.rs`, `services/users.rs`,
  `repos/users.rs`, `domain/user.rs`. The file name is the plural resource in
  `api`/`services`/`repos`, and the singular type in `domain`.

### 1.3 Types
- Structs & enums: `UpperCamelCase`.
- Domain record: the noun — `User`, `Order`.
- Creation input: `Create<Noun>` — `CreateUser`. Update input: `Update<Noun>`.
- Summary/projection: `<Noun>Summary` — `UserSummary`.
- Request/response for RPC-style endpoints: `<Verb><Noun>Request` /
  `<Verb><Noun>Response`.
- Enums are nouns; variants are `UpperCamelCase` and exhaustive.

### 1.4 Traits
- A capability the thing *is*: adjective or `-able` (`Clone`, not used for app
  code much). A capability the thing *provides*: the noun of the capability
  (`Model`, `Tool`, `Task`, `Event`). Prefer short, single-word trait names.

### 1.5 Functions & methods
- `snake_case`, verb-first: `create`, `find`, `list`, `update`, `delete`,
  `search`, `emit`, `run`.
- CRUD service/repo methods use this fixed vocabulary:
  - `create(&self, ctx, input) -> Result<T>`
  - `find(exec, id) -> Result<Option<T>>` (repo) / `get(&self, ctx, id) -> Result<T>` (service; maps `None`→`NotFound`)
  - `list(&self, ctx, filter) -> Result<Vec<T>>`
  - `update(&self, ctx, id, input) -> Result<T>`
  - `delete(&self, ctx, id) -> Result<()>`
- Fallible constructors: `connect`, `init`, `from_env`, `load`. Infallible: `new`.
- Boolean queries: `is_`, `has_`, `can_` (`has_role`).

### 1.6 Variables & constants
- Variables: `snake_case`, meaningful nouns; avoid abbreviations except the
  conventional `ctx`, `db`, `tx`, `id`.
- Constants & statics: `SCREAMING_SNAKE_CASE`. Tool constants: `NAME`,
  `DESCRIPTION`.

### 1.7 Errors
- There is one error type, `hephos::Error`. No app-defined error types. No `Error`
  suffix proliferation.

### 1.8 Endpoints
- Paths are lowercase, plural nouns, hyphenated if multi-word:
  `/users`, `/user-invites`. Resource id as a brace param: `/users/{id}`.
- Sub-resources nest: `/users/{id}/orders`.
- RPC-style: `/rpc/{verb_noun}` with a `Request`/`Response` body.

### 1.9 Services, agents, tools, workflows, events
- Service: `<Noun>Service` (`UserService`), file `services/users.rs`.
- Repo: `<Noun>Repo` (`UserRepo`), file `repos/users.rs`.
- Agent: `<Purpose>Agent` or a function `agents::<purpose>()` returning an
  `Agent<M>`; file `agents/<purpose>.rs`.
- Tool: `<Verb><Noun>` (`SearchUsers`), `NAME` = `snake_case` verb_noun
  (`"search_users"`), file `tools/<verb_noun>.rs`.
- Workflow: `<Noun>Workflow` (`OnboardingWorkflow`), file `workflows/<noun>.rs`.
- Events: one enum `AppEvent`; variants are past-tense facts
  (`UserCreated`, `OrderShipped`).

---

## 2. Canonical code structures

### 2.1 `state.rs` (every app has exactly one)

```rust
use hephos::prelude::*;
use hephos::db::Db;
use uuid::Uuid;

use crate::services::users::UserService;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Debug, Clone)]
pub enum AppEvent {
    UserCreated { id: Uuid },
}
impl Event for AppEvent {}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub events: Events<AppEvent>,
    pub users: UserService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        let db = Db::connect_from_env().await?;
        db.migrate(&sqlx::migrate!("./migrations")).await?;
        let events = Events::new(1024);
        let users = UserService::new(db.clone(), events.clone());
        Ok(AppState { db, events, users })
    }
}
```

`db.migrate(&sqlx::migrate!("./migrations"))` embeds the app's `migrations/` dir at
compile time and runs any unapplied migrations (idempotent). Hold `db` in `AppState`
only if a handler/task reads it directly; otherwise pass it into the services that own
it (the examples do the latter).

### 2.2 REST endpoint (`api/users.rs`)

```rust
use hephos::prelude::*;

use crate::domain::user::{CreateUser, User};
use crate::state::Ctx;

pub fn routes() -> Router<crate::state::AppState> {
    Router::new()
        .get("/users/{id}", get)
        .post("/users", create)
}

pub async fn get(ctx: Ctx, Path(id): Path<uuid::Uuid>) -> Result<Json<User>> {
    let user = ctx.state().users.get(&ctx, id).await?;
    Ok(Json(user))
}

pub async fn create(ctx: Ctx, Json(input): Json<CreateUser>) -> Result<Json<User>> {
    let user = ctx.state().users.create(&ctx, input).await?;
    Ok(Json(user))
}
```

The top-level `api/mod.rs` composes resource routers with `merge` (each resource
router already declares full paths, so they join at the same level). `nest` is
reserved for real sub-paths such as orders under `/users/{id}` — never `nest("/")`.

```rust
pub fn routes() -> Router<crate::state::AppState> {
    Router::new().merge(users::routes())
}
```

### 2.3 Service (`services/users.rs`)

```rust
use hephos::prelude::*;
use hephos::db::Db;
use uuid::Uuid;

use crate::domain::user::{CreateUser, User};
use crate::repos::users::UserRepo;
use crate::state::{AppEvent, Ctx};

#[derive(Clone)]
pub struct UserService {
    db: Db,
    events: Events<AppEvent>,
}

impl UserService {
    pub fn new(db: Db, events: Events<AppEvent>) -> Self {
        UserService { db, events }
    }

    pub async fn get(&self, _ctx: &Ctx, id: Uuid) -> Result<User> {
        UserRepo::find(self.db.pool(), id)
            .await?
            .ok_or_else(|| Error::not_found("user"))
    }

    pub async fn create(&self, _ctx: &Ctx, input: CreateUser) -> Result<User> {
        if input.email.trim().is_empty() {
            return Err(Error::invalid("email is required"));
        }
        let mut tx = self.db.begin().await?;
        let user = UserRepo::create(tx.exec(), &input).await?;
        tx.commit().await?;
        self.events.emit(AppEvent::UserCreated { id: user.id });
        Ok(user)
    }
}
```

### 2.4 Repository (`repos/users.rs`)

```rust
use hephos::prelude::*;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::user::{CreateUser, User};

pub struct UserRepo;

impl UserRepo {
    pub async fn find<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<Option<User>> {
        let row = sqlx::query_as!(
            User,
            "select id, email, created_at from users where id = $1",
            id
        )
        .fetch_optional(exec)
        .await?;
        Ok(row)
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
```

Repo rules: one query per method; take an executor (`PgExecutor`) so pool and tx
both work; return domain types; `fetch_optional` for "maybe", never let
`RowNotFound` escape; no business logic.

### 2.5 Domain type (`domain/user.rs`)

```rust
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateUser {
    pub email: String,
}
```

Domain types: `Serialize` for outputs, `Deserialize` for inputs, `Debug + Clone`
always. No methods with logic; no I/O; no dependency on other app modules.

### 2.6 Authentication middleware (`api/auth.rs`) — shape

```rust
// Validates a bearer token and replaces the request Identity. Applied to a route
// group via the framework's layer seam (see examples/auth for the full wiring).
async fn authenticate(/* headers, next */) {
    // 1. read `Authorization: Bearer <token>`; missing → leave Identity::Anonymous
    // 2. verify with jsonwebtoken against the secret from env
    // 3. on success, insert Identity::User(Principal { id, roles }) into extensions
    // 4. on invalid token, respond 401 (Error::Unauthorized)
}
```

### 2.7 Background task (`tasks/outbox.rs`)

```rust
use std::time::Duration;
use hephos::prelude::*;
use crate::state::AppState;

pub struct OutboxDrain;

impl Task<AppState> for OutboxDrain {
    fn name(&self) -> &'static str { "outbox_drain" }

    async fn run(&self, ctx: Ctx<AppState>) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => break,
                _ = tick.tick() => {
                    if let Err(error) = ctx.state().users.drain_outbox().await {
                        tracing::error!(%error, "outbox drain failed");
                    }
                }
            }
        }
        Ok(())
    }
}
```

### 2.8 Event subscriber (`events/audit.rs`)

```rust
// Registered as a Task that owns a broadcast receiver and matches AppEvent.
use hephos::prelude::*;
use crate::state::{AppEvent, AppState};

pub struct AuditLog;

impl Task<AppState> for AuditLog {
    fn name(&self) -> &'static str { "audit_log" }

    async fn run(&self, ctx: Ctx<AppState>) -> Result<()> {
        let mut rx = ctx.state().events.subscribe();
        loop {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => break,
                event = rx.recv() => match event {
                    Ok(AppEvent::UserCreated { id }) => tracing::info!(%id, "user created"),
                    Err(_) => continue, // lagged: resync from source of truth
                }
            }
        }
        Ok(())
    }
}
```

### 2.9 Tool (`tools/search_users.rs`)

```rust
use hephos::prelude::*;
use hephos::db::Db;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::user::UserSummary;
use crate::repos::users::UserRepo;

pub struct SearchUsers { db: Db }

impl SearchUsers {
    pub fn new(db: Db) -> Self { SearchUsers { db } }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchQuery {
    /// Free-text match on name or email.
    pub query: String,
}

impl Tool for SearchUsers {
    type Input = SearchQuery;
    type Output = Vec<UserSummary>;
    const NAME: &'static str = "search_users";
    const DESCRIPTION: &'static str = "Search users by name or email.";

    async fn call(&self, input: SearchQuery) -> Result<Vec<UserSummary>> {
        if input.query.trim().is_empty() {
            return Err(Error::invalid("query must not be empty"));
        }
        UserRepo::search(self.db.pool(), &input.query).await
    }
}
```

### 2.10 Agent (`agents/support.rs`)

```rust
use hephos::prelude::*;
use hephos::agent::{Agent, Limits};
use hephos::agent::providers::Anthropic;

use crate::tools::search_users::SearchUsers;

pub fn support(db: hephos::db::Db) -> Result<Agent<Anthropic>> {
    let model = Anthropic::from_env("claude-opus-4-8")?;
    Ok(Agent::new(model)
        .system("You help support staff find and summarize user accounts.")
        .tool(SearchUsers::new(db))
        .limits(Limits::default()))
}
```

### 2.11 Workflow (`workflows/onboarding.rs`)

```rust
use hephos::prelude::*;
use crate::domain::user::CreateUser;
use crate::state::{AppState, Ctx};

pub struct OnboardingWorkflow;

impl OnboardingWorkflow {
    pub async fn run(&self, ctx: &Ctx, input: CreateUser) -> Result<uuid::Uuid> {
        let user = ctx.state().users.create(ctx, input).await?;
        self.provision(ctx, user.id).await?;
        self.welcome(ctx, user.id).await?;
        Ok(user.id)
    }

    async fn provision(&self, ctx: &Ctx, id: uuid::Uuid) -> Result<()> {
        ctx.state().workspaces.create_default(ctx, id).await
    }

    async fn welcome(&self, ctx: &Ctx, id: uuid::Uuid) -> Result<()> {
        ctx.state().notifier.welcome(ctx, id).await
    }
}
```

Workflow rules: a struct with `run(ctx, input) -> Result<Output>`; steps are
private methods; each step calls services/agents; failure propagates with `?`;
cancellation is inherited from `ctx`.

### 2.12 MCP server (`mcp/server.rs`)

```rust
use hephos::prelude::*;
use hephos::mcp::McpServer;
use crate::tools::search_users::SearchUsers;

pub async fn serve(db: hephos::db::Db) -> Result<()> {
    McpServer::new()
        .tool(SearchUsers::new(db))
        .serve_stdio()
        .await
}
```

---

## 3. Errors

### 3.1 The type and mapping

| Variant | Constructor | Status | Client sees |
| --- | --- | --- | --- |
| `NotFound(String)` | `Error::not_found(what)` | 404 | `"<what> not found"` |
| `Invalid(String)` | `Error::invalid(msg)` | 400 | the message |
| `Unauthorized` | `Error::Unauthorized` | 401 | `"unauthorized"` |
| `Forbidden` | `Error::Forbidden` | 403 | `"forbidden"` |
| `Conflict(String)` | `Error::conflict(msg)` | 409 | the message |
| `Internal(anyhow::Error)` | `?` / `Error::internal(msg)` | 500 | `"internal error"` |

Response body is always `{ "error": { "kind": "...", "message": "..." } }`.

### 3.2 Creation & propagation
- Expected failures are **constructed** (`Error::not_found`, `Error::conflict`,
  `Error::invalid`) so they are visible in the code.
- Unexpected failures **propagate** with `?`; `From` turns infrastructure errors
  into `Internal`.
- Convert "no row" to a domain error at the service boundary:
  `repo.find(...).await?.ok_or_else(|| Error::not_found("user"))`.
- Never construct an `Internal` with a message that contains user input or
  secrets; never surface infrastructure detail to the client.

### 3.3 Where each layer errors
- `api/`: only extraction errors (automatic, via `Json`/`Path`/`Query`) and
  re-raising service errors with `?`.
- `services/`: the business errors — `Invalid`, `NotFound`, `Conflict`,
  `Forbidden`.
- `repos/`: only `Internal` (via `?`); never business errors.

---

## 4. Async

- `async fn` only when the body awaits I/O.
- Spawning: only via `App::task`. No detached `tokio::spawn` in app code.
- Cancellation: every task loop and every long-lived handler loop selects against
  `ctx.cancel_token().cancelled()`.
- Timeouts: the request stack has one; outbound calls carry their own; agents use
  `Limits.timeout`.
- Concurrency: bound every fan-out (`buffered(N)`, `Semaphore`). Channels bounded.
- Backpressure: prefer bounded channels that make a slow consumer slow the
  producer, over unbounded queues that grow.
- Never block the async runtime (`std::thread::sleep`, blocking I/O). Use async
  equivalents.

---

## 5. Testing

| Kind | Location | What |
| --- | --- | --- |
| Unit | `#[cfg(test)] mod tests` at file bottom | services, repos, tools |
| HTTP surface | `#[cfg(test)] mod tests;` → `src/tests.rs` | the real router, via `TestClient` |
| DB | inside a rolled-back `Tx` or a disposable DB | repo/service with data |

A binary crate has no lib target, so HTTP-surface tests live in a
`#[cfg(test)] mod tests;` module (`src/tests.rs`), not `tests/`. A library crate
uses `tests/` as usual.

### 5.1 The canonical HTTP-surface test

Drive the real router with `hephos::test::TestClient` — same routes and middleware
the app serves. Every app includes a `routes_build` canary.

```rust
use hephos::test::TestClient;
use serde_json::{json, Value};

use crate::{api, state::AppState};

async fn client() -> TestClient {
    TestClient::new(api::routes(), AppState::init().await.unwrap())
}

// The canary: route composition must not panic. Catches bad nest/merge.
#[tokio::test]
async fn routes_build() {
    let _ = api::routes();
}

#[tokio::test]
async fn create_empty_email_is_400() {
    let res = client().await.post_json("/users", &json!({ "email": "" })).await;
    assert_eq!(res.status(), 400);
    assert_eq!(res.json::<Value>()["error"]["kind"], "invalid");
}
```

Rules:
- Every app has a `routes_build` test. Never optional — it is the one-line guard
  against wiring bugs that `cargo check` cannot see.
- Every endpoint: ≥1 success test and ≥1 failure test, driven through `TestClient`,
  asserting status and `error.kind`.
- Build service-level contexts with `Ctx::detached(state)`.
- No network in tests; exercise `Model` via a test implementation.
- Tests needing external infra (Postgres) are `#[ignore = "…"]` with the reason, so
  `cargo test` stays green offline; CI runs them with `--include-ignored`.
- Name tests `fn <subject>_<condition>_<expected>()`:
  `create_empty_email_is_400`, `get_missing_product_is_404`.

### 5.2 Database tests (leave no residue)

Two shapes, both `#[ignore]`d offline:

```rust
// Disposable database — a fresh, migrated DB per test, dropped afterward.
use hephos::db::test::TestDb;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn creates_and_reads() {
    let test_db = TestDb::create(&sqlx::migrate!("./migrations")).await.unwrap();
    let user = UserRepo::create(test_db.db().pool(), &new_user()).await.unwrap();
    assert!(UserRepo::find(test_db.db().pool(), user.id).await.unwrap().is_some());
    test_db.cleanup().await.unwrap(); // drops the database — no residue
}
```

```rust
// Rolled-back transaction — when you only need isolation, not a fresh DB.
let mut tx = db.begin().await?;
let user = UserRepo::create(tx.exec(), &new_user()).await?;
// ... assertions using tx.exec() ...
tx.rollback().await?; // nothing persists
```

---

## 6. Dependencies

- Declared once in root `[workspace.dependencies]`, inherited with
  `{ workspace = true }`.
- Add one only if: the standard stack cannot do it, it is maintained and widely
  used, and the PR says why in one line.
- Never add a crate that duplicates: axum (HTTP), tokio (async), serde (serde),
  tracing (logs), sqlx (SQL), rmcp (MCP), thiserror/anyhow (errors), uuid, time.

---

## 7. Comments

- Comment-light. Names and types carry intent.
- Allowed only for: safety/security invariants, external-protocol requirements,
  non-obvious compiler constraints, and `// BOUNDARY:` markers (framework crates
  only, never app code).
- Public items in `crates/` carry a doc comment stating the contract. App code is
  self-documenting.
- A `///` doc comment on a tool-input field becomes part of the schema the model
  sees — write those as if the model will read them (it will).

---

## 8. Preventing competing patterns

This is the convention that protects all the others.

1. **Before writing, read.** Find the nearest existing artifact of the same kind
   and copy its shape. The examples in `examples/` are the reference set.
2. **One shape per kind.** If this document names a shape, that is the only shape.
3. **No second abstraction for a solved problem.** If a helper, service, or type
   already does the job, reuse it. Do not introduce a parallel one "just for this
   case."
4. **A new pattern is a documented decision.** If a genuinely new kind of artifact
   is needed, add its canonical shape to this file in the same PR, with a reason.
   An undocumented new pattern is a defect.
5. **Abstractions earn their place by repetition.** Do not extract an abstraction
   that is used once. Wait for the third occurrence, then make it canonical here.
6. **Consistency review before merge.** Confirm the change matches every section
   above and that no document now contradicts the code.
