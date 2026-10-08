# ARCHITECTURE.md

This document explains **what** Rivet is and **why** it is shaped the way it is.
`CLAUDE.md` tells you how to write code; this tells you why those rules exist.
`CONVENTIONS.md` is the mechanical reference. When any two disagree, the
contradiction is a defect to be fixed, not a choice to be made.

> A note on status: the HTTP core is **implemented** as of `0.0.2` — the standard
> middleware stack (request id, span, timeout, body limit, cancellation), graceful
> shutdown, `Config`, and the `TestClient` harness are real and tested. The remaining
> non-trivial subsystems are still marked `// BOUNDARY:` with a precise description of
> what remains (`Db::migrate`, the model provider clients, agent tool dispatch, rmcp
> serving). The `examples/` directory is written against this contract. Nothing here is
> a fake API — every signature is one a correct implementation can satisfy, and the
> boundaries say exactly where work remains. See `MILESTONES.md` for the build order.

---

## 1. Goal

Rivet is a Rust backend framework whose primary user is a **coding agent**, not a
human. Humans read it too, and benefit, but every trade-off is resolved in favor
of what makes an autonomous agent correct and fast.

An agent builds software by pattern-matching a repository: it reads existing code,
infers the conventions, and produces more code in the same shape. An agent is at
its best when the repository is **regular** — when the same kind of thing always
looks the same — and at its worst when it must choose between several valid
approaches. Rivet is engineered to maximize regularity and minimize choice.

Concretely, the framework optimizes, in priority order: agent readability,
predictability, consistency, a small API surface, strong typing, Rust safety,
production performance, security, explicit behavior, and minimal configuration.

The design rule that follows from this: **coherence over flexibility, simplicity
over abstraction, agent predictability over human preference.** Where a more
flexible design would introduce a second valid way to do something, Rivet chooses
the single way.

---

## 2. What Rivet is not

- Not a general-purpose web framework. It is narrower and more opinionated than
  Axum or Actix by design.
- Not an AI SDK with HTTP bolted on, nor an LLM wrapper. The agent layer is one
  capability among several, built on the same primitives as the rest.
- Not a new runtime, HTTP stack, or serialization format. Rivet builds **above**
  Tokio, Hyper, Tower, Axum, and Serde; it adds value as an application layer, not
  as infrastructure.
- Not a plugin platform. There is no plugin system, no DI container, no
  macro-driven magic framework. Extensibility comes from writing ordinary code in
  the canonical shape.

---

## 3. Boundaries

Rivet draws a hard line between **infrastructure it reuses** and **the application
layer it owns**.

| Concern | Owned by | Rivet's role |
| --- | --- | --- |
| async runtime | Tokio | reuse directly |
| HTTP/1+2 | Hyper (via Axum) | reuse directly |
| routing/extraction | Axum | wrap in a smaller, fixed surface |
| middleware | Tower | assemble one fixed stack; hide from apps |
| serialization | Serde | reuse directly |
| observability | tracing | reuse; install one subscriber |
| SQL | SQLx | wrap `Db`/`Tx`; repos use `query!` directly |
| MCP | rmcp | wrap tool exposure; reuse the protocol impl |
| error ergonomics | thiserror/anyhow | build the one `Error` type on top |

Rivet owns: the application object (`App`), the request context (`Ctx`), the one
error type (`Error`), the constrained router, the typed extractors, the task and
event model, and the agent/tool/workflow/MCP application patterns.

**Why wrap Axum instead of exposing it?** Axum is excellent and flexible — it has
many extractors, several response conventions, and multiple routing styles. That
flexibility is exactly what hurts an agent: it multiplies the number of valid ways
to write a handler. Rivet exposes one extractor pattern (`Ctx` + `Json`/`Path`/
`Query`), one response convention (`Result<Json<T>>`), and one routing style
(`router.get(path, handler)`). The full power of Axum remains underneath for the
framework's own use; it is not part of the surface an agent must learn.

---

## 4. Crate structure

A workspace of small crates, so capabilities evolve independently and an app only
compiles what it uses. This directly serves the "minimal surface" goal: an app
without a database never sees `Db`.

```
crates/
  rivet         meta: prelude + feature-gated re-exports (db, agent, mcp, full)
  rivet-core    App, Ctx, Error, Router, extractors, Config, Task, Events
  rivet-db      Db, Tx over SQLx/Postgres
  rivet-agent   Model, Agent, Tool, structured output, streaming
  rivet-mcp     rmcp integration
```

`rivet-core` has no knowledge of databases, models, or MCP. `rivet-agent` is
state-agnostic — it does not know your `AppState`. These are deliberate
dependency cuts: each crate is small enough to understand completely, and the
dependency graph is a DAG with `rivet-core` at the root.

**Why not one crate?** A monolith would force every app to compile the agent and
MCP machinery even for a plain REST service, and would blur the boundaries that
keep each capability independently evolvable. **Why not many micro-crates?** Below
this granularity the seams add import overhead without buying independence.

---

## 5. Application model

### 5.1 The composition root

There is exactly one `App`, built in `main` and run:

```rust
#[tokio::main]
async fn main() -> rivet::Result<()> {
    let state = AppState::init().await?;
    App::new(state)
        .routes(api::routes())
        .task(OutboxDrain)
        .run()
        .await
}
```

`App` is the composition root and the only place the whole system is wired
together. `AppState::init` constructs every handle (DB pool, services, event bus);
`api::routes()` returns the one `Router`; tasks are registered explicitly. There
is no hidden auto-registration, no component scanning, no inversion-of-control
container. An agent can read `main.rs` and `state.rs` and know the entire shape of
the application.

### 5.2 Application lifecycle

`App::run`:

1. Reads `Config` from the environment (addr, log format) — nothing more.
2. Initializes tracing (one subscriber, `pretty` or `json`).
3. Creates a root `CancellationToken` for graceful shutdown.
4. Spawns each registered `Task`, each receiving a detached `Ctx` whose
   cancellation token is the shutdown token.
5. Builds the Axum router, wraps it in the **standard middleware stack**, attaches
   state, binds the listener, and serves with graceful shutdown.
6. On SIGINT/SIGTERM: cancels the token, stops accepting connections, drains
   in-flight requests, then awaits task completion.

**Why explicit lifecycle?** Startup and shutdown are where production systems
break. Making them a single, readable function — rather than a web of
`#[on_startup]` hooks — means an agent debugging a shutdown hang has one place to
look.

### 5.3 State and ownership

`AppState` is a `Clone` struct of cheap handles. Axum clones it per request; each
handle is `Arc`-backed internally so cloning is cheap by construction. The rules
that follow (no `Arc<Mutex<AppState>>`, no globals) are not stylistic — they
encode where mutable state is *allowed* to live: in the database, or in a single
owning task, never smeared across request handlers behind a lock. This eliminates
an entire class of deadlock and contention bugs that an agent would otherwise have
to reason about.

---

## 6. Request lifecycle

```
TCP → Hyper → standard stack → Router → extractors → handler
                                                        │
                                              ctx.state().service.method()
                                                        │
                                                   repo → SQLx → Postgres
                                                        │
                                              Result<Json<T>>  or  Error
                                                        │
                                   IntoResponse → status + JSON body → client
```

The **standard middleware stack** is fixed and identical in every app, implemented
once in `rivet-core::stack` and installed by `App::run`. In order, outermost to
innermost:

1. **Observe** — generate a request-id UUID, insert it and a default
   `Identity::Anonymous` into extensions, open a request span (method, path, id), and
   echo the id back as the `x-request-id` response header for client/log correlation.
2. **Timeout** — a ceiling on *response generation* (`tower_http::timeout`, `408` on
   overrun). It bounds how long a handler may take to return its response, **not** body
   streaming — so SSE/WebSocket handlers, which return immediately, are unaffected by
   construction.
3. **Body limit** — a maximum request body size (`tower_http::limit`, `413`).
4. **Cancel** — insert a per-request `CancellationToken` that is a **child of the
   shutdown token**, so a graceful shutdown cancels in-flight work. On client
   disconnect the handler future and any streaming body are dropped by the runtime,
   which stops the work regardless.

Authentication is **not** in the standard stack. It is an app-level `route_layer`
(so it runs only on matched routes — an unmatched path stays `404`, never `401`) that
replaces the default `Identity` for the routes that need it (see §16). The observe
layer runs outermost and seeds `Anonymous`; auth, applied inner on matched routes,
overrides it.

`Ctx` is produced by reading these extensions. Because the stack always runs on the
HTTP path, extraction is infallible — `Ctx::from_request_parts` cannot reject.
(`Ctx::detached`, used by tasks and tests, constructs the context directly.)

**Why one fixed stack?** Middleware order is a classic source of subtle bugs
(auth after logging, timeout inside body-limit, cancellation that kills streams).
Fixing the order once, in the framework, removes a decision and a bug class from every
app — and is covered by the `rivet-core` stack acceptance tests.

---

## 7. Error architecture

One type, `rivet::Error`, used by every layer. It is a small enum whose variants
map one-to-one to HTTP statuses, plus an `Internal(anyhow::Error)` escape hatch.

```
NotFound     → 404      Invalid     → 400
Unauthorized → 401      Forbidden   → 403
Conflict     → 409      Internal    → 500 (logged in full, returned opaque)
```

Design decisions and their reasons:

- **One type, not per-module enums.** An agent reasoning about error handling has
  exactly one type to understand. Error conversion is never a design question.
- **Variants are HTTP-shaped, but services never import HTTP.** The mapping to
  status lives in `IntoResponse`, in `rivet-core`. A service returns
  `Error::not_found("user")` and stays ignorant of the number 404.
- **`Internal` is opaque to clients, verbose to operators.** Its full cause chain
  is logged via tracing; the response body is a generic message. This is a
  security property (no internal leakage) and an observability property (full
  detail server-side) in one.
- **`From` conversions feed `?`.** `sqlx::Error`, `serde` errors, etc. convert to
  `Internal`, so the happy path uses `?` and unexpected failures become 500s
  automatically. Expected failures (not-found, conflict) are constructed
  explicitly, so they are visible in the code.

**Why not `anyhow` everywhere, or a rich typed error per operation?** `anyhow`
alone loses the status mapping and the client contract. A rich per-operation error
type maximizes type information but multiplies types an agent must learn and
convert between. The single enum is the balance point: enough structure to drive
the HTTP contract, few enough variants to hold in mind.

---

## 8. Async & concurrency model

- **Runtime:** Tokio multi-threaded. Rivet does not expose runtime configuration
  beyond what `#[tokio::main]` gives; there is one runtime per process.
- **Structured concurrency:** background work is owned by `App` as `Task`s, tied
  to the shutdown token. There is no detached `spawn` in application code; work
  that outlives a request is a `Task`, so it is always cancellable and always
  drained on shutdown.
- **Cancellation:** `Ctx` carries a `CancellationToken` that is a child of the
  process shutdown token, so it fires on graceful shutdown; tasks receive the shutdown
  token directly. Loops select against it. On client disconnect the handler future and
  any streaming body are dropped by the runtime, which stops the work regardless — so
  disconnect is handled by Rust's drop semantics, and the token covers shutdown.
  (Making the per-request token *also* fire on disconnect is a `0.3` refinement; a naive
  drop-guard would wrongly cancel an SSE stream the instant the handler returned.)
- **Backpressure & bounded concurrency:** fan-out uses bounded primitives
  (`buffered(N)`, `Semaphore`). Channels are bounded. The event bus has a fixed
  per-subscriber buffer and surfaces lag rather than growing without limit.
- **Timeouts:** the request stack has a timeout; outbound calls (DB pool, HTTP
  clients, agent runs) each carry their own. No unbounded await on the network.

**Why native async traits (not `async-trait`)?** On edition 2024 / Rust 1.85,
`async fn` in traits and RPITIT are stable. Rivet uses them directly
(`fn run(&self, ...) -> impl Future<...> + Send`). This avoids the `async-trait`
macro's boxing and keeps trait definitions readable — an agent sees a normal async
signature, not a macro expansion. The one place a boxed future is used
(`App`'s internal task list, the agent's heterogeneous tool list) is an explicit,
contained erasure, not a blanket policy.

---

## 9. Transport & API architecture

Rivet treats REST as the default and RPC as a thin discipline on top of the same
machinery, rather than a separate subsystem.

### 9.1 REST

The canonical transport. Handlers are Axum handlers constrained to the Rivet
shape. Routing is verb-first (`router.post("/users", users::create)`). Path params
use Axum 0.8 brace syntax (`/users/{id}`). This is the fully supported path today.

### 9.2 RPC

RPC in Rivet is **not a second framework** — it is a convention over REST:
`POST /rpc/{method}` with a `Json<Request>` body and a `Json<Response>` reply,
handlers living in `api/` like any other. Rivet does not ship code generation or a
schema language in v1; that is a deliberate deferral (see `MILESTONES.md`). Choosing
one HTTP-shaped RPC convention over a bespoke protocol keeps the surface an agent
must learn to zero beyond REST.

### 9.3 WebSockets

Built on Axum's WebSocket upgrade. The canonical shape (see `examples/websocket`):
an upgrade handler that takes `Ctx`, then a connection loop that `select!`s over
incoming messages and `ctx.cancel_token().cancelled()`. Outbound messages go
through a **bounded** channel so a slow client applies backpressure instead of
growing an unbounded queue. Connection lifecycle (open, message, close, error) is
explicit.

### 9.4 HTTP streaming & SSE

Streaming responses are an `axum::response::Sse` or a bounded body stream. The
canonical shape (see `examples/streaming`): a handler returns a stream that is
driven by a bounded source and tied to the request cancellation token, so client
disconnect stops production. The same streaming primitive backs agent token
streaming (§12), so there is one streaming model, not two.

**Why unify streaming?** An agent that learns how HTTP streaming works should not
have to learn a different mechanism for model streaming. Both are "a bounded,
cancellable stream of items rendered to the client."

---

## 10. Background tasks & scheduling

A `Task` is a long-lived unit of work owned by `App`. It implements one method,
`run(ctx)`, and runs until the shutdown token fires. Scheduled work is a task that
drives a `tokio::time::interval` inside its loop. Rivet deliberately ships **no
cron engine** — the interval-loop pattern covers the overwhelming majority of
needs and is fully visible in the code. Durable, distributed scheduling is a
future, additive concern, explicitly out of scope for v1 (see `MILESTONES.md`).

**Why no scheduler abstraction?** A scheduler is a stateful subsystem with its own
failure modes. For an agent, a `select!` loop over `interval.tick()` and
`cancelled()` is more legible than a declarative cron registry, and it composes
with the existing task lifecycle instead of adding a parallel one.

---

## 11. Events

An app defines **one** event enum (`AppEvent`) and implements the `Event` marker.
`Events<AppEvent>` is a cheap handle in `AppState`; services call
`events.emit(AppEvent::UserCreated { id })`. Subscribers are background tasks that
receive every event and `match` on it.

The bus is **in-process and best-effort** (a Tokio `broadcast`). This is a
deliberate floor, not a ceiling:

- One enum (not one type per event) lets a subscriber match exhaustively and lets
  an agent see every event in one place.
- Best-effort, in-process delivery is correct for cache invalidation, metrics, and
  fan-out to local subscribers.
- **BOUNDARY / future:** durable or cross-service delivery (a transactional outbox
  table drained by a `Task`, or a broker) is additive. The service still calls
  `emit`; only the transport behind it changes. v1 does not build this, and an
  agent must not grow the broadcast bus into a message queue.

**Why not an external broker in v1?** It would add operational dependencies and a
second delivery semantics to learn, for a capability most apps first need in its
simple form. The architecture leaves the door open without paying the cost now.

---

## 12. LLM & agent architecture

The agent layer is a thin, state-agnostic application of the same primitives.

- **`Model`** — the provider abstraction. Two methods: `complete` and `stream`.
  Providers are thin `reqwest` clients (`providers::Anthropic`, etc.). Keeping the
  trait to two methods means every provider implementation is small and obviously
  correct. (Prior art: Rig generalizes across 20+ providers; Rivet deliberately
  keeps the trait minimal and provider count small, favoring predictability over
  breadth.)
- **`Tool`** — a strongly typed capability. `type Input: DeserializeOwned +
  JsonSchema`, `type Output: Serialize`, plus `const NAME`/`DESCRIPTION` and one
  `call` method. The input schema is derived (`schemars`) and handed to the model,
  so the model cannot call a tool with a mis-shaped input. Validation beyond shape
  and authorization happen inside `call`, returning the one `Error`.
- **`Agent`** — `Model` + system prompt + tools + `Limits`. It owns the turn loop:
  call model, dispatch tool calls, feed results back, repeat until a final answer
  or a limit is hit. `run` returns text; `run_typed::<O>()` returns a deserialized
  value (structured output); `stream` returns text deltas.
- **`Limits`** — every run is bounded by max turns, max tokens, and a timeout.
  There is no unbounded agent loop; overrun is an explicit error.

**State-agnostic by design.** `rivet-agent` does not know `AppState`. A tool
captures its dependencies (`Db`, a service, the caller's identity) at construction,
inside a handler that already holds `Ctx`. So authorization is done where the
identity is known, and the agent layer stays free of application generics. This is
the key cut that keeps the agent crate small and reusable.

**Observability & cancellation** come for free: agent runs happen inside a request
or task, under the same tracing span and cancellation token as everything else.
Dropping a `stream` cancels generation.

**BOUNDARY:** provider HTTP clients, the vendor-specific encoding of tool calls,
and the structured-output enforcement are marked in `crates/rivet-agent`. The
trait contracts and the turn-loop algorithm are fixed; the vendor wiring is the
remaining work.

---

## 13. Tool architecture

Tools are the unit of capability shared between agents and MCP. One `Tool`
implementation is:

- callable by an `Agent` (registered with `.tool(...)`), and
- exposable over MCP (registered with `McpServer::tool(...)`),

with **no second implementation**. The strongly typed input/output plus derived
schema mean the same validation and authorization apply whether the caller is a
model inside the process or an external MCP client. This single-definition
property is a core reason tools are a first-class concept rather than ad-hoc
functions.

---

## 14. Workflow architecture

A workflow is a **deterministic** orchestration of services and agents, written as
ordinary Rust: a struct with a `run(ctx, input) -> Result<Output>` method whose
body calls services and agents in sequence, with explicit state, explicit failure
handling, and explicit cancellation points. Steps are private methods.

Rivet v1 does **not** ship a durable workflow engine (no event-sourced replay, no
distributed saga runtime). That is the correct deferral: durable execution is a
large stateful subsystem, and most workflows are a bounded sequence of steps that
a plain async function expresses more legibly than a DSL. The architecture marks
durable execution as a future, additive layer (see `MILESTONES.md`), reachable without
changing the `run(ctx, input)` shape.

**Why functions, not a DSL?** An agent reads a plain async function and knows
exactly what happens, in order. A workflow DSL introduces a second control-flow
language to learn and a runtime to debug. For deterministic, bounded orchestration
the function wins on legibility.

---

## 15. MCP integration

Rivet uses **rmcp**, the official Rust MCP SDK, rather than defining a protocol.
`rivet-mcp` turns a Rivet `Tool` into an MCP tool and runs an rmcp server over
stdio (local) or Streamable HTTP (networked — the current MCP transport, which
replaced the deprecated HTTP+SSE transport). Resources and prompts, where an app
needs them, are exposed through rmcp directly.

**Why reuse rmcp?** The protocol is a moving target maintained by the MCP project;
reimplementing it would be both wasted effort and a correctness risk. Rivet's value
is that a tool written once in the Rivet shape is reachable by local agents and
external MCP clients alike.

---

## 16. Authentication & authorization

- **Authentication** is app-level middleware, layered onto the route groups that
  need it. It validates a bearer token (`jsonwebtoken`, verified against a secret
  from the environment) and replaces the request's default `Identity::Anonymous`
  with `Identity::User(Principal { id, roles })`.
- **Authorization** is done in handlers and services via `ctx.require_user()?` and
  `ctx.require_role("admin")?`, which return the narrowest correct error (401 vs
  403). Fine-grained checks (ownership of a resource) are service-layer decisions,
  expressed as `Error::Forbidden`.
- **Passwords** are hashed with `argon2`. Login issues a signed token; Rivet does
  not store sessions server-side in v1 (stateless tokens), with a revocation list
  as a future addition if needed.

**Why a thin `Principal` (id + roles), not a full user?** Putting the whole user
record in the identity couples the auth layer to the user schema and tempts stale
data. Loading the full user by id in a service, when needed, keeps auth uniform
across apps and keeps the identity cheap to propagate.

See `examples/auth` for the canonical implementation. It is a real token flow, not
a toy.

---

## 17. Database integration

- **SQLx over Postgres**, chosen for compile-time-checked queries. `sqlx::query!`
  and `query_as!` verify SQL against the database schema at build time, so a
  malformed query or a column mismatch is a compile error, not a runtime 500. For
  an agent, this turns a whole class of data-layer bugs into immediate compiler
  feedback — which is exactly the feedback loop agents exploit best.
- **No ORM, no query builder.** The SQL is written as SQL. An agent can read a
  repo function and know precisely what hits the database. An ORM would add a
  second mental model (entities, relations, lazy loading) and hide the queries.
- **`Db` and `Tx`.** `Db` is the pool (reads, single-statement writes). `Tx` is a
  transaction for multi-statement writes; the service owns the transaction
  boundary, passing the executor to repo functions and committing explicitly.
- **Repos are plain functions** generic over the SQLx executor, so the same
  function works with a pool or a transaction. The generic is uniform boilerplate
  an agent copies verbatim.

**Why Postgres specifically?** Committing to one database removes a configuration
axis and lets the compile-time query checks be real. Supporting other databases is
a future, additive concern; v1 is Postgres.

---

## 18. Observability

One mechanism: `tracing`. One subscriber, installed by `App::run`, `pretty` or
`json` by env. Every request is a span carrying method, path, and request id;
`Error::Internal` logs its cause chain. The agent layer and tasks run inside the
same span tree, so a trace follows a request from HTTP through service, repo,
agent, and tool. There is no second logging facade and no metrics framework in v1
(metrics are an additive concern; `tracing` spans are the substrate they would be
derived from).

---

## 19. Configuration & secrets

Minimal and decentralized by ownership. `rivet-core::Config` reads only the two
things the framework needs (`RIVET_ADDR`, `RIVET_LOG`). Each component reads its
own config from the environment where it is constructed (`Db` reads
`DATABASE_URL`; a provider reads `ANTHROPIC_API_KEY`). There is no central config
object threaded through the app. Secrets live in the environment, are never logged,
and are read once at startup.

**Why no central config struct?** A central config becomes a dumping ground every
feature must touch, and a thing an agent must thread through constructors. Reading
each value where it is used keeps config local to its owner and keeps `main.rs`
small.

---

## 20. Security model

- `#![forbid(unsafe_code)]` crate-wide. No `unsafe` anywhere. If it were ever
  unavoidable it would be isolated, justified, tested, and documented — but v1
  needs none.
- Parameterized SQL only (SQLx enforces this).
- Input validated at the edge (serde shape) and in the service (business rules).
- Opaque internal errors; no leakage of internals to clients.
- Secrets never logged; `Internal` errors logged server-side only.
- Auth via verified tokens; passwords via `argon2`; no hand-rolled crypto.
- Bounded resource use (timeouts, body limits, bounded channels, agent limits) so
  a single caller cannot exhaust the process.

---

## 21. Testing architecture

Testing is a **first-class framework capability**, not an afterthought — because a
framework optimized for agents must make its correctness checkable the same
uniform way everywhere, and because compilation alone proves nothing about wiring.

- **`rivet::test::TestClient`** is the canonical harness. It drives the real
  [`Router`] — the same routes, extractors, and middleware the app serves — in
  memory, without a socket. It is shipped as part of the framework so there is one
  obvious way to test an app's HTTP surface, and so the test exercises the actual
  wiring rather than a reconstruction of it.
- **Two layers, both required.** *Unit tests* (`#[cfg(test)] mod tests` at a file's
  bottom) check a service/repo/tool via `Ctx::detached(state)`. *HTTP-surface
  tests* drive `TestClient` and assert status + `error.kind`. A unit test proves a
  service works; only an HTTP-surface test proves the app is wired.
- **The `routes_build` canary.** Every app has a one-line test that builds
  `api::routes()` and asserts no panic. Route composition, `nest`/`merge` misuse,
  and similar wiring faults are invisible to `cargo check` and only surface when the
  router is constructed — this test is where they surface, in CI, not in production.
- **Binary-crate placement.** An app is a binary with no lib target, so
  HTTP-surface tests live in a `#[cfg(test)] mod tests;` module (`src/tests.rs`)
  that can reach `crate::api::routes()`. Library crates use `tests/`.
- **Determinism.** Database tests run in a rolled-back transaction or a disposable
  database; tests never touch the network (providers are exercised behind the
  `Model` trait); tests requiring external infrastructure are `#[ignore]`d with a
  reason so the default suite stays green offline.

Why this is in the architecture at all: an earlier revision passed `cargo check`,
`cargo build`, and `cargo clippy` while shipping a startup panic (nesting a router
at `"/"`). None of those commands runs a request. The response was structural — a
framework-owned test client plus a mandatory `routes_build` canary — so the gap
cannot silently reopen. The single error type and uniform handler shape then make
the tests themselves regular: the same success/failure pair fits every endpoint.

---

## 22. Extensibility strategy

Extension is **writing ordinary code in the canonical shape**, not configuring a
plugin system. To add a capability, an agent adds a file in the right directory
(§1 of `CLAUDE.md`) and follows the nearest example. New transports or subsystems
are added to the framework as new crates behind feature flags, never as runtime
plugins. This keeps the system statically analyzable — an agent can always see the
whole program — and avoids the indirection a plugin registry would impose.

---

## 23. Dependency strategy

One pinned version per dependency, declared once in `[workspace.dependencies]`,
inherited everywhere. A dependency is added only when it provides something the
standard stack does not, is maintained and widely used, and is justified in the PR.
The standard stack (Tokio, Axum/Hyper/Tower, Serde, tracing, SQLx, rmcp,
thiserror/anyhow, uuid, time) covers the vast majority of needs; the default
answer to "should I add a crate?" is no.

---

## 24. The two simulations

The architecture is validated against the two tasks in the brief.

**"Add an authenticated `POST /users` that writes to the DB and emits an event."**
An agent, following the doctrine, knows: the route goes in `api/users.rs` as
`create`, mounted in `api::routes()`; auth is already applied to the route group,
so the handler calls `ctx.require_role("admin")?`; it calls
`ctx.state().users.create(&ctx, input)`; the service opens a `Tx`, calls
`UserRepo::create`, emits `AppEvent::UserCreated`, commits; the error type is
`rivet::Error` throughout; tests go in `services/users.rs` (unit) and `tests/`
(integration). Every decision is determined by the conventions — none is guessed.
This flow is realized in `examples/database` + `examples/auth`.

**"Create an agent with a tool that searches the DB and returns structured
results."** The agent knows: the tool goes in `tools/`, implementing `Tool` with
typed input/output, holding a `Db`, validating and authorizing in `call`; the
agent goes in `agents/`, built with `.system(...).tool(SearchUsers::new(db))`;
structured results come from `agent.run_typed::<Vec<UserSummary>>(...)`. Realized
in `examples/tools` + `examples/agent`.

Both simulations resolve to a single obvious implementation. Where they did not,
the convention was tightened until they did.
