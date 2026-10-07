# CLAUDE.md — Operating manual for coding agents

This file governs how code is written in this repository. It is normative. When
this file and your own judgment disagree, this file wins. When this file and
`ARCHITECTURE.md` or `CONVENTIONS.md` disagree, stop and reconcile them before
writing code — that is a bug in the doctrine, not a license to improvise.

Rivet exists so that **you** — a coding agent — can build production backends with
no ambiguity. Every rule below removes a decision you would otherwise have to
guess at. Follow them exactly and your code will be indistinguishable from the
rest of the repository. That is the goal.

---

## 0. The prime directive

> There is **one canonical way** to do each thing. Find it, copy it, adapt it.

Before writing anything, locate the nearest existing example of the same kind of
code (an endpoint, a service, a repo, a tool). Mirror its structure. Do not invent
a second pattern for something that already has one. If you believe a genuinely
new pattern is required, say so explicitly and explain why the existing one does
not fit — do not add it silently.

---

## 1. Where code goes (the placement rule)

Every file has one home, decided by **responsibility**, not by feature. An app's
`src/` is always this shape:

```
src/
  main.rs         composition root: build AppState, routes, tasks; run.
  state.rs        AppState, `type Ctx`, AppEvent enum.
  api/            HTTP handlers. One file per resource (users.rs, orders.rs).
  services/       business logic + orchestration. One file per domain concept.
  repos/          database access. SQL only. One file per table/aggregate.
  domain/         plain data types (structs, enums). No logic, no I/O.
  tasks/          background + scheduled work.
  events/         AppEvent subscribers.
  agents/         agent definitions.
  tools/          agent tools.
  workflows/      multi-step orchestrations.
  mcp/            MCP server wiring.
```

Decision procedure when adding code:

| The code… | goes in |
| --- | --- |
| parses a request / shapes a response / maps a route | `api/` |
| makes a business decision, orchestrates, emits events, owns a transaction | `services/` |
| runs SQL | `repos/` |
| is a data shape with no behavior | `domain/` |
| runs in the background on a timer or loop | `tasks/` |
| reacts to an `AppEvent` | `events/` |
| calls a model | `agents/` |
| is a capability a model can invoke | `tools/` |
| sequences services/agents into a deterministic process | `workflows/` |

If code seems to belong in two places, it is doing two things. Split it.

---

## 2. The layering rule (never skip a layer)

```
api  →  services  →  repos  →  database
         services  →  events, tasks, agents, workflows
```

- Handlers (`api/`) **never** touch `repos/` or run SQL. They extract input, call
  one service method, and return. Keep them under ~15 lines.
- Services (`services/`) **never** touch HTTP types (no `StatusCode`, no
  `axum::`). They speak in domain types and `rivet::Error`.
- Repos (`repos/`) **never** make business decisions. They run one query and
  return domain types. No `if` on business rules.
- `domain/` depends on nothing in the app. Everything may depend on `domain/`.

A handler that calls a repo directly is a bug. A service that returns a
`StatusCode` is a bug. A repo with business logic is a bug.

---

## 3. Errors: one type, always

- Every fallible function returns `rivet::Result<T>` (= `Result<T, rivet::Error>`).
- Never define a new error enum in an app. Use `rivet::Error`.
- Construct domain errors with the constructors: `Error::not_found("user")`,
  `Error::invalid("email is required")`, `Error::conflict("email taken")`.
- For unexpected failures, use `?` — `From` conversions turn `sqlx::Error` and
  friends into `Error::Internal`, which is logged in full and returned as an
  opaque `500`. Never hand-build a 500 message that leaks internals.
- Map "row not found" explicitly: repos return `Option<T>`; the service turns
  `None` into `Error::not_found(...)`. Never let a bare `RowNotFound` escape.
- Never `.unwrap()` or `.expect()` outside tests and `main` startup. Never panic
  in a handler, service, repo, task, or tool.

See `CONVENTIONS.md § Errors` for the full table of variant → status.

---

## 4. The handler shape (memorize this)

```rust
use rivet::prelude::*;
use crate::state::Ctx;
use crate::domain::user::{CreateUser, User};

pub async fn create(ctx: Ctx, Json(input): Json<CreateUser>) -> Result<Json<User>> {
    let user = ctx.state().users.create(&ctx, input).await?;
    Ok(Json(user))
}
```

Rules:
- `ctx: Ctx` is **always** the first parameter.
- Body is **always** `Json<T>` where `T` is a `domain/` type.
- Return is **always** `Result<Json<T>>` (or `Result<StatusCode>` for 204).
- The body is: extract → call one service method → wrap → return. Nothing else.

---

## 5. State and ownership

- There is one `AppState`, defined in `state.rs`, `#[derive(Clone)]`. It holds
  cheap-to-clone handles: `Db`, services, `Events<AppEvent>`. Each of those is
  itself `Clone` and internally `Arc`-backed. Cloning `AppState` is cheap by
  construction.
- **Never** wrap state in `Arc<Mutex<_>>`. If you reach for `Arc<Mutex<T>>`, you
  are modeling shared mutable state that belongs in the database or in a dedicated
  actor task. Stop and reconsider.
- Access state only through `ctx.state()`. Never use `static`/`lazy_static`/
  `OnceCell` globals for application state.
- Services hold exactly the handles they need (often just `Db` and `Events`),
  injected when `AppState` is built in `state.rs`.

---

## 6. Async and concurrency

- `async fn` only when the body actually awaits I/O. Do not make CPU-only
  functions async.
- Spawn long-lived work as a `Task` registered on `App`. Do not `tokio::spawn`
  detached futures from inside handlers — spawned work must be owned by the app
  lifecycle so it is cancelled on shutdown.
- Every loop in a task selects against `ctx.cancel_token().cancelled()`.
- Bound concurrency: use `futures::stream::...buffered(N)` or a
  `tokio::sync::Semaphore`. Never fan out an unbounded number of tasks over
  user-controlled input.
- Put a timeout on every outbound network call. For agents, the timeout is part
  of `Limits`.
- Never block the runtime: no `std::thread::sleep`, no blocking file/DB calls on
  the async threads. Use `tokio::time::sleep` and async drivers.

---

## 7. Security (non-negotiable)

- `#![forbid(unsafe_code)]` is set crate-wide. Do not remove it. Do not add
  `unsafe`.
- Authented routes get the identity from `ctx.require_user()?` /
  `ctx.require_role("admin")?`. Never parse auth headers in a handler.
- Never log secrets, tokens, passwords, or full request bodies. `Error::Internal`
  is logged server-side and returned opaque — keep it that way.
- All SQL goes through `sqlx::query!`/`query_as!` (parameterized, compile-checked).
  Never format user input into a SQL string.
- Validate input in the **service** layer (shape is checked by `serde` at the
  edge; business rules are checked in the service). Return `Error::invalid`.
- Hash passwords with `argon2`. Verify tokens with `jsonwebtoken`. Never roll
  your own crypto.

---

## 8. Dependencies

- All dependencies are declared **once** in the root `Cargo.toml`
  `[workspace.dependencies]` and inherited with `{ workspace = true }`.
- Do not add a dependency to solve something the standard stack already does
  (HTTP → axum, async → tokio, serialize → serde, logs → tracing, SQL → sqlx,
  MCP → rmcp, errors → thiserror/anyhow).
- Adding a new dependency requires a one-line justification in the PR description
  and a check that it is maintained and widely used. When in doubt, don't.

---

## 9. Comments and documentation

- Comment-light by default. Do not explain what the code already says.
- A comment is allowed only for: a safety/security invariant, an external protocol
  requirement, a non-obvious compiler constraint, or a deliberate `// BOUNDARY:`
  marker (framework-internal, see below).
- Public items in `crates/` get a doc comment describing the contract. App code
  (`examples/`, real apps) is self-documenting through names and types.
- `// BOUNDARY:` marks a place where the framework's shape is defined but the
  implementation is deferred. Never leave a `// BOUNDARY:` in application code —
  they exist only inside `crates/`.

---

## 10. Testing

- Unit tests live in a `#[cfg(test)] mod tests` block at the bottom of the file
  they test. Test services and repos here.
- Integration tests live in `tests/` at the crate root and exercise the app
  through its HTTP surface.
- Every new endpoint gets at least: one success test and one failure test (the
  error path that returns a non-2xx).
- Tests build a `Ctx` with `Ctx::detached(state)` and call services directly, or
  drive the router with a test client.
- Tests must not reach the network. Database tests use a transaction rolled back
  at the end, or a disposable test database.

---

## 11. Validation commands (run before you claim done)

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --all-features
```

All four must pass. `-D warnings` means clippy warnings are errors — fix them,
do not `#[allow]` them without a written reason. If `sqlx` offline mode is in use,
run `cargo sqlx prepare` after changing any query.

Never report a task complete if any command above fails. Report the failure and
the output instead.

---

## 12. Workflow

- Work on a branch, never commit directly to `main`.
- One logical change per commit. Commit messages: imperative mood, present tense.
- Do not commit generated files, secrets, or `.env`.
- Before opening a PR: run §11, update the relevant example if you changed a
  framework contract, and confirm no document in the repo now contradicts your
  change.

---

## 13. The simulation test (apply it to yourself)

Before finishing, confirm you can answer these about your own change without
guessing. If you cannot, the change is not done:

1. Where does this code belong? (→ §1)
2. Which existing abstraction did I reuse? (→ §0)
3. What is this type/function called, and why that name? (→ `CONVENTIONS.md`)
4. What error type does it return? (→ §3, always `rivet::Error`)
5. How is it exposed? (→ §4)
6. How is state owned and accessed? (→ §5)
7. How does the async behave under cancellation? (→ §6)
8. Where are its tests? (→ §10)
9. Does it match the nearest existing example? (→ §0)

---

## 14. Forbidden / preferred (quick reference)

**Forbidden**
- `unsafe`
- new error enums in app code
- `Arc<Mutex<T>>` for app state
- global mutable state
- handlers touching repos or SQL
- services importing `axum`/HTTP types
- `.unwrap()`/`.expect()`/`panic!` outside tests & startup
- detached `tokio::spawn` in handlers
- string-formatted SQL
- adding a dependency that duplicates the standard stack
- a second way to do something that already has a canonical way

**Preferred**
- `rivet::Result<T>` everywhere
- `ctx: Ctx` first parameter everywhere
- `Json<T>` in, `Result<Json<T>>` out
- compile-checked `sqlx::query_as!`
- `?` with `From` conversions
- small functions, explicit types, exhaustive `match`
- copying the nearest example
