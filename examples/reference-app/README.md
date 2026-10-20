# Canonical Hephos Reference Application

The reference application is the production proof that every Hephos capability composes cleanly into a coherent backend:
- **REST APIs**: Strongly typed JSON requests and responses (`api/items.rs`, `domain/item.rs`)
- **Database & Repositories**: Pure SQL access via `sqlx` without leaky ORM abstractions (`repos/items.rs`, `repos/users.rs`)
- **Authentication & Security**: Argon2 password hashing, short-lived JWT access tokens, and IDOR protection via `ctx.require_owner` (`services/auth.rs`, `api/auth.rs`)
- **Durable Workflows**: Postgres-checkpointed multi-step workflows with saga compensations surviving process crashes (`workflows/provisioning.rs`)
- **Background Tasks & Outbox**: Long-running scheduled tasks selecting against `cancel_token` (`tasks/reaper.rs`)
- **In-Process Events**: Type-safe decoupled domain event bus (`state::AppEvent`, `events/subscribers.rs`)
- **Agent Layer**: Bounded agent loops with tool calling (`agents/assistant.rs`, `services/assistant.rs`)
- **Model Context Protocol (MCP)**: Shared tool definitions callable by both in-process agents and external clients over Streamable HTTP and stdio (`tools/calculator.rs`, `mcp/mod.rs`)

## Structure

Following `AGENTS.md §1`:

```
src/
  main.rs         composition root: build AppState, routes, tasks; run
  state.rs        AppState, type Ctx, AppEvent enum
  api/            HTTP handlers. One file per resource (items.rs, auth.rs, agent.rs, workflows.rs)
  services/       business logic and orchestration (items.rs, auth.rs, assistant.rs)
  repos/          database access. SQL only (items.rs, users.rs)
  domain/         plain data types with derives. No logic, no I/O (item.rs, user.rs, assistant.rs)
  tasks/          background and scheduled work (reaper.rs)
  events/         AppEvent subscribers (subscribers.rs)
  agents/         agent definitions (assistant.rs, model.rs)
  tools/          agent and MCP tools (calculator.rs)
  workflows/      durable checkpointed workflows (provisioning.rs)
  mcp/            MCP server and router wiring
  tests.rs        routes_build canary and TestClient tests
```

## Running

```sh
# Set required secrets (in development)
export JWT_SECRET="your-32-byte-long-production-jwt-secret-key"
export DATABASE_URL="postgres://postgres:postgres@localhost:5432/hephos_prod"

# Run migrations
cargo sqlx migrate run --source examples/reference-app/migrations

# Run the reference app
cargo run -p example-reference-app
```
