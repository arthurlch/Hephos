<h1 align="center">Rivet</h1>

<p align="center"><strong>The backend framework built only for coding agents.</strong></p>

<p align="center">
  <a href="https://github.com/arthurlch/rivet/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/arthurlch/rivet/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
  <img alt="License" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue">
  <img alt="Status" src="https://img.shields.io/badge/status-pre--alpha-red">
</p>

> **If a coding agent can understand the framework, it can build the application.**

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

Rivet exists for exactly that world. It is not a general-purpose web framework with
nice docs for agents. It is a framework whose **architecture, naming, and conventions
are designed around how agents read a repository and write code** — so the agent's
output is correct, predictable, and production-grade by construction.

## It is aggressively opinionated — on purpose

Rivet gives you **one canonical way** to do each thing. One error type. One handler
shape. One place each kind of code lives. One way to access state, run a query, add
auth, spawn a background task, define a tool. There is no second valid style.

That is a deliberate trade: **coherence over flexibility, predictability over
preference.** Agents (and humans) are fastest and least error-prone when a codebase
is *regular* — when the same kind of thing always looks the same. Every choice Rivet
removes is a bug it prevents and a decision an agent never has to guess at.

If you want a framework that bends to your personal style, this isn't it, and that's
fine. If you want a backend where every file looks like the same careful engineer
wrote it — whether a human or an agent did — that's the whole point.

## What it looks like

A complete, production-shaped endpoint. `ctx` is always first; the body is always
`Json<T>`; the return is always `Result<Json<T>>`; the error type is always
`rivet::Error`. Learn it once, and every handler in every Rivet app is this shape.

```rust
use rivet::prelude::*;

use crate::domain::user::{CreateUser, User};
use crate::state::Ctx;

pub async fn create(ctx: Ctx, Json(input): Json<CreateUser>) -> Result<Json<User>> {
    let user = ctx.state().users.create(&ctx, input).await?;
    Ok(Json(user))
}
```

```rust
#[tokio::main]
async fn main() -> rivet::Result<()> {
    let state = AppState::init().await?;
    App::new(state).routes(api::routes()).run().await
}
```

## Built above proven infrastructure — not reinvented

Rivet does not ship a new runtime, HTTP stack, or serialization format. It is a thin,
coherent application layer **above** the best of the Rust ecosystem, and its value
lives there:

| Concern | Reused | Rivet adds |
| --- | --- | --- |
| async runtime | Tokio | structured lifecycle & cancellation |
| HTTP / routing | Hyper + Axum | one constrained router & extractor set |
| middleware | Tower | one fixed, correct stack |
| serialization | Serde | typed `Json`/`Path`/`Query` with one error path |
| database | SQLx (Postgres) | `Db`/`Tx` + the repository pattern |
| observability | tracing | one subscriber, one span tree |
| agents / MCP | rmcp | one `Tool` def → agent **and** MCP |
| errors | thiserror / anyhow | **one** `Error` type, HTTP-aware |

## Scope

REST · typed errors · services & repositories · auth (argon2 + JWT) · background &
scheduled tasks · events · WebSockets · SSE/streaming · LLM agents · strongly-typed
tools · structured output · durable workflows · MCP — each with **one** canonical
pattern and a copy-ready example.

## Status — pre-alpha scaffold

Honest about where this is: the **doctrine, the public API contract, the reference
examples, and the project tooling** are in place, and the REST path runs, serves, and
is tested over HTTP. Core runtime subsystems (the middleware stack, DB migrations,
model provider clients, agent tool dispatch, MCP serving) are still marked
`// BOUNDARY:` — designed, contracted, not yet implemented.

This is **not yet a functional `0.1.0`**; it is a consolidated base to build one from.
The full, versioned path to a production-ready `1.0` — including a production-readiness
definition-of-done — lives in [`MILESTONES.md`](./MILESTONES.md).

## Quickstart

```sh
git clone https://github.com/arthurlch/rivet && cd rivet
make ci            # fmt-check + clippy -D warnings + tests (offline)
make run-rest      # launch the REST example, then: curl localhost:8080/products
cargo test         # drives the real router via rivet::test::TestClient
```

## Documentation

| File | Purpose |
| --- | --- |
| [`CLAUDE.md`](./CLAUDE.md) | Operating manual for coding agents. Actionable, normative rules. |
| [`ARCHITECTURE.md`](./ARCHITECTURE.md) | Why the system is shaped the way it is. |
| [`CONVENTIONS.md`](./CONVENTIONS.md) | The single canonical way to write every kind of code. |
| [`MILESTONES.md`](./MILESTONES.md) | The versioned roadmap from `0.0.1` to a production `1.0`. |
| [`CONTRIBUTING.md`](./CONTRIBUTING.md) | How to extend Rivet (same bar for humans and agents). |

## Layout

```
crates/
  rivet/        meta crate: prelude + feature-gated re-exports
  rivet-core/   App, Ctx, Error, Router, extractors, config, tasks, events, test harness
  rivet-db/     Db, Tx — SQLx/Postgres helpers
  rivet-agent/  Model, Agent, Tool, structured output, streaming
  rivet-mcp/    rmcp integration
examples/
  rest/ auth/ database/ websocket/ streaming/ agent/ tools/ workflow/ mcp/
```

## License

Dual-licensed under either [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE), at
your option.
