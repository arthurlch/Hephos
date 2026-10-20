# AGENT_PLAYBOOK.md — The Canonical Operating Playbook for Coding Agents

> **Audience:** Autonomous Coding Agents (Claude, Gemini, OpenAI, Cursor, or Human Engineers).  
> **Purpose:** Enable zero-human-disambiguation development. Every prompt, code shape, directory decision, and validation step required to extend a Hephos 1.0 application in a single turn.

---

## 1. System Prompt for Coding Agents

When delegating tasks to a coding agent inside a Hephos workspace, prepend this system prompt into the agent's context:

```markdown
You are an autonomous senior backend engineer modifying a Hephos 1.0 application.
This codebase strictly enforces the Hephos doctrine (AGENTS.md, CONVENTIONS.md, ARCHITECTURE.md).

CRITICAL DIRECTIVES:
1. THE PRIME DIRECTIVE: There is exactly ONE canonical way to do each thing. Locate the nearest example, mirror its structure, and do not invent secondary patterns.
2. STRICT LAYERING (CI-ENFORCED by `hephos check`):
   - api/ (Handlers): Extract input, call ONE service method, return Result<Json<T>> or Result<StatusCode>. NEVER import repos, run SQL, or touch pool/tx.
   - services/ (Business Logic): Pure domain logic, validation, and orchestration. NEVER import axum, StatusCode, or HTTP types. Return `hephos::Result<T>`.
   - repos/ (Data Access): SQL only. Run parameterized `sqlx::query!` or `query_as!`. Return domain types or Option<T>. NEVER make business decisions or import services.
   - domain/ (Plain Data): Structs and enums only. No I/O, no logic.
3. ONE ERROR TYPE: Every fallible function returns `hephos::Result<T>` (`Result<T, hephos::Error>`).
   - Use `Error::not_found("entity")`, `Error::invalid("reason")`, `Error::conflict("reason")`, `Error::unauthorized()`, `Error::forbidden()`, `Error::rate_limited("msg")`.
   - Never define custom error enums in application crates.
4. UNIFORM CONTEXT: `ctx: Ctx` is ALWAYS the first argument of handlers, tools, and background tasks. Access state solely through `ctx.state()`. Never use global state or Arc<Mutex<AppState>>.
5. ROUTER CANARY & TESTS: Every app must pass `api::routes()` composition without panic. Every endpoint must have at least one success test and one failure test driven via `hephos::test::TestClient`.
6. ZERO UNWRAP: Never `.unwrap()` or `.expect()` in application runtime code. Use `?` with `From` conversions.
```

---

## 2. Directory Placement & Architectural Boundaries

```
src/
  main.rs         composition root: build AppState, routes, tasks; run.
  state.rs        AppState struct, type Ctx = hephos::Ctx<AppState>, AppEvent.
  api/            HTTP handlers (one per resource: items.rs, users.rs).
  services/       Business logic (one per domain concept: items.rs, auth.rs).
  repos/          Database queries (one per table: items.rs, users.rs).
  domain/         Plain data types and requests (items.rs, users.rs).
  tasks/          Background loops and scheduled maintenance.
  events/         In-process event subscribers.
  agents/         LLM agent definitions and model runners.
  tools/          Agent capabilities implementing `hephos::agent::Tool`.
  workflows/      Multi-step durable orchestrations (`Workflows`).
  mcp/            MCP server wiring (`McpServer`).
  tests.rs        Router-driving HTTP integration tests (`TestClient`).
```

### CI Boundary Matrix

| From | To `domain/` | To `repos/` | To `services/` | To `api/` | To SQL / `sqlx` | To `axum` / HTTP |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`api/`** | Allowed | **FORBIDDEN** | Allowed | Allowed | **FORBIDDEN** | Allowed |
| **`services/`**| Allowed | Allowed | Allowed | **FORBIDDEN** | Allowed (tx) | **FORBIDDEN** |
| **`repos/`** | Allowed | Allowed | **FORBIDDEN** | **FORBIDDEN** | Allowed | **FORBIDDEN** |
| **`domain/`**| Allowed | **FORBIDDEN** | **FORBIDDEN** | **FORBIDDEN** | **FORBIDDEN** | **FORBIDDEN** |

---

## 3. Canonical Task Recipes

### Recipe 1: Add a New REST Endpoint

#### 1. Domain (`src/domain/widget.rs`)
```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Widget {
    pub id: Uuid,
    pub name: String,
    pub quantity: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateWidget {
    pub name: String,
    pub quantity: i32,
}
```

#### 2. Repo (`src/repos/widgets.rs`)
```rust
use hephos::prelude::*;
use crate::domain::widget::{CreateWidget, Widget};

pub struct WidgetRepo;

impl WidgetRepo {
    pub async fn find(pool: &sqlx::PgPool, id: uuid::Uuid) -> Result<Option<Widget>> {
        let row = sqlx::query_as!(
            Widget,
            "SELECT id, name, quantity FROM widgets WHERE id = $1",
            id
        )
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }

    pub async fn create(pool: &sqlx::PgPool, input: CreateWidget) -> Result<Widget> {
        let row = sqlx::query_as!(
            Widget,
            "INSERT INTO widgets (id, name, quantity) VALUES ($1, $2, $3) RETURNING id, name, quantity",
            uuid::Uuid::new_v4(),
            input.name,
            input.quantity
        )
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if hephos::db::is_unique_violation(&e) {
                Error::conflict("widget with this name already exists")
            } else {
                e.into()
            }
        })?;
        Ok(row)
    }
}
```

#### 3. Service (`src/services/widgets.rs`)
```rust
use hephos::prelude::*;
use crate::state::Ctx;
use crate::domain::widget::{CreateWidget, Widget};
use crate::repos::widgets::WidgetRepo;

#[derive(Clone)]
pub struct WidgetService {
    db: Db,
}

impl WidgetService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn get(&self, _ctx: &Ctx, id: uuid::Uuid) -> Result<Widget> {
        match WidgetRepo::find(self.db.pool(), id).await? {
            Some(widget) => Ok(widget),
            None => Err(Error::not_found("widget")),
        }
    }

    pub async fn create(&self, _ctx: &Ctx, input: CreateWidget) -> Result<Widget> {
        if input.name.trim().is_empty() {
            return Err(Error::invalid("widget name is required"));
        }
        if input.quantity < 0 {
            return Err(Error::invalid("quantity must be non-negative"));
        }
        WidgetRepo::create(self.db.pool(), input).await
    }
}
```

#### 4. Handler (`src/api/widgets.rs`)
```rust
use hephos::prelude::*;
use crate::state::{AppState, Ctx};
use crate::domain::widget::{CreateWidget, Widget};

pub fn routes() -> Router<AppState> {
    Router::new()
        .get("/widgets/{id}", get_widget)
        .post("/widgets", create_widget)
}

pub async fn get_widget(ctx: Ctx, Path(id): Path<uuid::Uuid>) -> Result<Json<Widget>> {
    let widget = ctx.state().widgets.get(&ctx, id).await?;
    Ok(Json(widget))
}

pub async fn create_widget(ctx: Ctx, Json(input): Json<CreateWidget>) -> Result<Json<Widget>> {
    let widget = ctx.state().widgets.create(&ctx, input).await?;
    Ok(Json(widget))
}
```

---

### Recipe 2: Protect Routes with Authentication & Authorization

#### Guarding a Route Group in `src/api/mod.rs`
```rust
pub fn routes(state: &AppState) -> Router<AppState> {
    let auth_service = state.auth.clone();

    let protected = Router::new()
        .merge(items::routes())
        .authenticated(move |token| {
            let auth = auth_service.clone();
            async move { auth.verify_token(&token).await }
        });

    Router::new()
        .merge(public_routes())
        .merge(protected)
}
```

#### Enforcing Role or Owner Checks in Service (`src/services/items.rs`)
```rust
pub async fn update(&self, ctx: &Ctx, id: uuid::Uuid, input: UpdateItem) -> Result<Item> {
    let existing = ItemRepo::find(self.db.pool(), id).await?
        .ok_or_else(|| Error::not_found("item"))?;

    // IDOR protection: Verify current caller is the resource owner
    ctx.require_owner(existing.owner_id)?;

    // Optional admin override:
    // if ctx.require_role("admin").is_err() {
    //     ctx.require_owner(existing.owner_id)?;
    // }

    ItemRepo::update(self.db.pool(), id, input).await
}
```

---

### Recipe 3: Enqueue and Process Durable Background Jobs

#### Define a Job and Worker (`src/tasks/email.rs`)
```rust
use hephos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SendWelcomeEmail {
    pub user_id: uuid::Uuid,
    pub email: String,
}

pub fn email_worker(db: Db) -> JobWorker {
    JobWorker::builder(db)
        .concurrency(4)
        .poll_interval(std::time::Duration::from_millis(500))
        .register_async("send_welcome_email", |payload: serde_json::Value| async move {
            let job: SendWelcomeEmail = serde_json::from_value(payload)
                .map_err(|e| Error::invalid(e.to_string()))?;
            // Process outbound email delivery
            tracing::info!(user_id = %job.user_id, email = %job.email, "Delivering welcome email");
            Ok(())
        })
        .build()
}
```

#### Enqueueing Job Inside a Transaction (`src/services/users.rs`)
```rust
pub async fn register(&self, ctx: &Ctx, input: RegisterUser) -> Result<User> {
    let mut tx = self.db.begin().await?;
    let user = UserRepo::create(tx.exec(), &input).await?;

    // Enqueue transactional job
    self.db.jobs().enqueue_tx(
        &mut tx,
        "send_welcome_email",
        &SendWelcomeEmail {
            user_id: user.id,
            email: user.email.clone(),
        },
        JobOptions::default(),
    ).await?;

    tx.commit().await?;
    Ok(user)
}
```

---

### Recipe 4: Durable Workflows with Saga Compensations

```rust
use hephos::prelude::*;

pub async fn execute_order_workflow(
    workflows: &Workflows,
    order_id: uuid::Uuid,
    amount_cents: i64,
) -> Result<()> {
    let mut flow = workflows.start("order_fulfillment", &order_id).await?;

    // Step 1: Reserve inventory with compensation
    flow.step("reserve_inventory", || async {
        // Reserve stock
        Ok("reservation_123".to_string())
    })
    .compensate(|_step_res| async move {
        // Release reserved stock on downstream failure
        Ok(())
    })
    .await?;

    // Step 2: Charge credit card
    flow.step("charge_card", || async {
        if amount_cents > 100_000 {
            return Err(Error::invalid("card limit exceeded"));
        }
        Ok("ch_999".to_string())
    })
    .await?;

    flow.complete(&"order_completed").await?;
    Ok(())
}
```

---

### Recipe 5: Defining Strongly Typed Agent Tools & MCP Export

#### Define a Tool (`src/tools/calculator.rs`)
```rust
use hephos::prelude::*;

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
pub struct CalcInput {
    #[schemars(description = "First integer operand")]
    pub a: i64,
    #[schemars(description = "Second integer operand")]
    pub b: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CalcOutput {
    pub sum: i64,
}

pub struct CalculatorTool;

impl Tool for CalculatorTool {
    type Input = CalcInput;
    type Output = CalcOutput;

    const NAME: &'static str = "calculate_sum";
    const DESCRIPTION: &'static str = "Calculate the sum of two integers.";

    async fn call(&self, input: Self::Input) -> Result<Self::Output> {
        Ok(CalcOutput {
            sum: input.a + input.b,
        })
    }
}
```

#### Expose to MCP Server over Streamable HTTP (`src/mcp/mod.rs`)
```rust
use hephos::prelude::*;
use crate::tools::calculator::CalculatorTool;

pub fn server() -> McpServer {
    McpServer::new("reference-mcp", "1.0.0")
        .tool(CalculatorTool)
}
```

---

### Recipe 6: Integration Testing with `TestClient`

Every application endpoint must have tests in `src/tests.rs`:

```rust
use hephos::prelude::*;
use hephos::test::TestClient;
use crate::state::AppState;
use crate::api;

#[tokio::test]
async fn routes_build() {
    let state = AppState::init_test().await;
    let _ = api::routes(&state);
}

#[tokio::test]
async fn create_and_get_item_succeeds() {
    let state = AppState::init_test().await;
    let client = TestClient::new(api::routes(&state), state);

    let res = client
        .post_json("/widgets", &serde_json::json!({ "name": "Gear", "quantity": 10 }))
        .await;
    assert_eq!(res.status(), 200);

    let created: serde_json::Value = res.json();
    let id = created["id"].as_str().unwrap();

    let get_res = client.get(&format!("/widgets/{id}")).await;
    assert_eq!(get_res.status(), 200);
    assert_eq!(get_res.json::<serde_json::Value>()["name"], "Gear");
}

#[tokio::test]
async fn unauthenticated_access_rejected() {
    let state = AppState::init_test().await;
    let client = TestClient::new(api::routes(&state), state);

    let res = client.get("/widgets/00000000-0000-0000-0000-000000000000").await;
    assert_eq!(res.status(), 401);
    assert_eq!(res.error_kind(), Some("unauthorized".to_string()));
}
```

---

## 4. The Anti-Pattern Codex (What Fails CI)

| Anti-Pattern | Why It Is Forbidden | Correct Replacement |
| :--- | :--- | :--- |
| `sqlx::query!` in `api/` | Violates separation of concerns. Handlers are HTTP adapters. | Move query to `repos/` and call from `services/`. |
| `use crate::repos;` in `api/` | Layer skip: handlers must not know database details. | Call service method in `services/`. |
| `axum::http::StatusCode` in `services/` | Services must be transport-agnostic (usable by MCP/CLI). | Return `hephos::Error` variants (`not_found`, `invalid`). |
| `.unwrap()` in handlers / services | Panics take down requests without structured telemetry. | Use `?` with `hephos::Result`. |
| `Arc<Mutex<AppState>>` | Shared mutable in-memory state is an anti-pattern. | Store mutable state in Postgres or an actor task. |
| Handcrafted 500 error messages | Leaks stack traces and database internals to callers. | Return `Error::internal(...)` or use `?` on sqlx errors. |
| Detached `tokio::spawn` in handlers | Unowned futures leak across shutdown and drain signals. | Register a `Task` on `App::task` or use `db.jobs()`. |
| String-interpolated SQL | SQL injection risk and breaks compile-time checking. | Use parameterized `sqlx::query!` / `query_as!`. |

---

## 5. Turn-to-Green-Build Validation Protocol

Execute these commands in sequence before declaring any task complete:

```sh
# 1. Fast compile & type check (~1s)
cargo check --all-targets --all-features

# 2. Architecture Boundary Lint
hephos check .
# or ./scripts/arch-lint.sh

# 3. Format & Clippy (Zero Warnings)
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings

# 4. Router & Integration Test Suite
cargo test --locked --all-features
```

If all 4 commands pass with zero warnings and zero failures, the backend is production-ready.
