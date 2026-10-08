<h1 align="center">Hephos</h1>

<p align="center"><strong>A framework only for agents to build quickly & deliver high performance application.</strong></p>

<p align="center">
  <a href="https://github.com/arthurlch/hephos/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/arthurlch/hephos/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
  <img alt="License" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue">
  <img alt="Status" src="https://img.shields.io/badge/status-pre--alpha-red">
</p>

> ** Writing code by hand is no longer an economically productive enterprise. **

---

## The bet

The cost of writing code is collapsing. Coding agents — Claude Code, Codex, Gemini
CLI, and their successors — now produce the majority of the code in many codebases,
and they get better every quarter. When a machine writes the code, the constraint
stops being *how fast a human can type* and becomes *how reliably the result is
correct, safe, and maintainable*.

That inversion changes which language you should reach for.

Rust was always the safest, fastest way to build a backend — memory-safe, no GC
pauses, fearless concurrency, errors you can't ignore. Its only real cost was human:
it's verbose and it has a learning curve. **But that cost was a human cost.** Hand
the typing to an agent and the downside largely disappears, while every upside
remains. In an agent-written world, Rust stops being the hard choice and becomes the
*obvious* one: you get a compiler that catches whole classes of bugs before they ship,
and an agent that doesn't mind the ceremony.

Hephos exists for exactly that world. It is not a general-purpose web framework with
nice docs for agents. It is a framework whose **architecture, naming, and conventions
are designed around how agents read a repository and write code** — so the agent's
output is correct, predictable, and production-grade by construction.

## It is aggressively opinionated — on purpose

Hephos gives you **one canonical way** to do each thing. One error type. One handler
shape. One place each kind of code lives. One way to access state, run a query, add
auth, spawn a background task, define a tool. There is no second valid style.

That is a deliberate trade: **coherence over flexibility, predictability over
preference.** Agents (and humans) are fastest and least error-prone when a codebase
is *regular* — when the same kind of thing always looks the same. Every choice Hephos
removes is a bug it prevents and a decision an agent never has to guess at.

If you want a framework that bends to your personal style, this isn't it, and that's
fine. If you want a backend where every file looks like the same careful engineer
wrote it — whether a human or an agent did — that's the whole point.

## What it looks like

A complete, production-shaped endpoint. `ctx` is always first; the body is always
`Json<T>`; the return is always `Result<Json<T>>`; the error type is always
`hephos::Error`. Learn it once, and every handler in every Hephos app is this shape.

```rust
use hephos::prelude::*;

use crate::domain::user::{CreateUser, User};
use crate::state::Ctx;

pub async fn create(ctx: Ctx, Json(input): Json<CreateUser>) -> Result<Json<User>> {
    let user = ctx.state().users.create(&ctx, input).await?;
    Ok(Json(user))
}
```

```rust
#[tokio::main]
async fn main() -> hephos::Result<()> {
    let state = AppState::init().await?;
    App::new(state).routes(api::routes()).run().await
}
```

## Built above proven infrastructure — not reinvented

Hephos does not ship a new runtime, HTTP stack, or serialization format. It is a thin,
coherent application layer **above** the best of the Rust ecosystem — and its value is
exactly the layer it adds: the opinions, guard-rails, and wiring that turn great
primitives into a backend an agent gets right the first time.

| Concern | Reused | What Hephos adds on top |
| --- | --- | --- |
| async runtime | Tokio | structured lifecycle: graceful drain, and cancellation that reaches every handler, task, and agent — no detached, unowned work |
| HTTP / routing | Hyper + Axum | one verb-first router and one handler shape; a typed `Ctx` extractor; nesting at `/` is a clear error, not a late panic |
| middleware | Tower | one fixed, correct stack (request-id → trace → timeout → body-limit → cancel); middleware-ordering bugs are designed out, auth uses `route_layer` so a 404 stays a 404 |
| serialization | Serde | `Json`/`Path`/`Query` that turn bad input into a clean `400` in the one error type — no rejection zoo to learn |
| database | SQLx (Postgres) | `Db`/`Tx`, a service-owned transaction boundary, executor-generic repos, and rolled-back-transaction test fixtures |
| observability | tracing | one span tree following a request from route → service → repo → agent → tool; OTLP logs/metrics on the roadmap |
| agents / MCP | rmcp | one strongly-typed `Tool` (derived schema + validation + authz) reachable by an in-process agent **and** external MCP clients; every run bounded by turns/tokens/timeout |
| errors | thiserror / anyhow | **one** `Error` for every layer; variants map to HTTP status; internals are logged in full but returned opaque — secure by default |
| testing | — (pure Hephos) | `hephos::test::TestClient` drives the **real** router in-memory, plus a mandatory `routes_build` canary — the wiring is tested, not hoped |

### What that actually buys you

The product isn't the wrapper — it's **predictability**. Because there is exactly one
way to write each thing, an agent (or a human) pattern-matches the nearest example and
is correct the first time, and a reviewer reads any file already knowing its shape. The
compiler catches a whole class of mistakes before they ship; the one error type and the
test harness catch the rest. You could assemble Tokio + Axum + SQLx + rmcp yourself —
Hephos is the opinionated, tested, agent-legible way they fit together, so you don't
re-litigate those decisions on every endpoint.

## Scope

REST · typed errors · services & repositories · auth (argon2 + JWT) · background &
scheduled tasks · events · WebSockets · SSE/streaming · LLM agents · strongly-typed
tools · structured output · durable workflows · MCP — each with **one** canonical
pattern and a copy-ready example.

## Status — pre-alpha scaffold

Honest about where this is: the **doctrine, the public API contract, the reference
examples, and the project tooling** are in place. As of `0.0.2` the **HTTP core is
real** — the standard middleware stack (request-id + `x-request-id`, tracing span,
request timeout, body limit, shutdown-linked cancellation), graceful shutdown, a
finalized `Config`, and the `TestClient` harness — all exercised by tests and by
running the servers. The remaining runtime subsystems (DB migrations, model provider
clients, agent tool dispatch, MCP serving) are still marked `// BOUNDARY:` — designed,
contracted, not yet implemented.

This is **not yet a functional `0.1.0`**; it is a consolidated base being built toward
one. The full, versioned path to a production-ready `1.0` — including a
production-readiness definition-of-done and the risk register — lives in
[`MILESTONES.md`](./MILESTONES.md).

## Quickstart

```sh
git clone https://github.com/arthurlch/hephos && cd hephos
make ci            # fmt-check + clippy -D warnings + tests (offline)
make run-rest      # launch the REST example, then: curl localhost:8080/products
cargo test         # drives the real router via hephos::test::TestClient
```

## Documentation

| File | Purpose |
| --- | --- |
| [`CLAUDE.md`](./CLAUDE.md) | Operating manual for coding agents. Actionable, normative rules. |
| [`ARCHITECTURE.md`](./ARCHITECTURE.md) | Why the system is shaped the way it is. |
| [`CONVENTIONS.md`](./CONVENTIONS.md) | The single canonical way to write every kind of code. |
| [`MILESTONES.md`](./MILESTONES.md) | The versioned roadmap from `0.0.1` to a production `1.0`. |
| [`CONTRIBUTING.md`](./CONTRIBUTING.md) | How to extend Hephos (same bar for humans and agents). |

## Layout

```
crates/
  hephos/        meta crate: prelude + feature-gated re-exports
  hephos-core/   App, Ctx, Error, Router, extractors, config, tasks, events, test harness
  hephos-db/     Db, Tx — SQLx/Postgres helpers
  hephos-agent/  Model, Agent, Tool, structured output, streaming
  hephos-mcp/    rmcp integration
examples/
  rest/ auth/ database/ websocket/ streaming/ agent/ tools/ workflow/ mcp/
```

## License

Dual-licensed under either [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE), at
your option.
