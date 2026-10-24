<h1 align="center">Hephos</h1>

<p align="center"><strong>The Rust backend framework built for the agents that now write the code.</strong></p>

<p align="center">
  <a href="https://github.com/arthurlch/hephos/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/arthurlch/hephos/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
  <img alt="License" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue">
  <img alt="Status" src="https://img.shields.io/badge/release-v1.0.0--stable-brightgreen">
</p>

> **When a machine writes the code, the constraint stops being how fast a human can type and becomes how reliably the result is correct.**

---

## The bet

The cost of writing code is collapsing. Coding agents — Claude Code, Codex, Gemini
CLI, and their successors — already produce the majority of the code in many
codebases, and they get better every quarter. That inversion changes which language
you should reach for.

Rust was always the safest, fastest way to build a backend: memory-safe, no GC
pauses, fearless concurrency, errors you can't ignore. Its only real cost was *human* —
it's verbose and it has a learning curve. Hand the typing to an agent and that cost
largely disappears while every upside remains. In an agent-written world, Rust stops
being the hard choice and becomes the obvious one.

Hephos exists for exactly that world. It is not a general-purpose web framework with
nice docs for agents. It is a framework whose **architecture, naming, and conventions
are shaped around how an agent reads a repository and writes code** — so the output is
correct, predictable, and production-grade by construction.

## Agent Experience (AX) — the lens, not a new science

UX designs the surface a *human* reads. DX designs the surface a *developer* works in.
**AX designs the surface a *model* reads** — your error messages, your API shapes, your
file layout — for the fact that its primary consumer is now a probabilistic
token-predictor with a finite context window.

AX is not a new theory and Hephos does not ask you to take one on faith. It's the
disciplined application of boring, proven ideas — strict layering, one canonical form,
closed error types, determinism, idempotency — with the *consumer* reframed from human
to model. The value is in enforcing them consistently, not in naming them. So rather
than argue the point, Hephos makes it concrete and testable:

## AX, demonstrated

Every principle below is a mechanism in the code, and every mechanism is enforced by a
lint or proven by a test — not a claim in a doc.

| AX principle | Mechanism in Hephos | Enforced / proven by |
| --- | --- | --- |
| **One canonical way to do each thing** | Fixed `api → services → repos → domain` layout; one handler shape; one place each kind of code lives | `scripts/arch-lint.sh` + `hephos check` fail CI if a handler runs SQL, a service names an HTTP type, or a repo calls a service |
| **Failures are dense and machine-readable** | A closed `Error` algebra; every fault becomes `{"error":{"kind","message"}}` with a stable `kind` | Variant → HTTP status is total and tested; internals are logged in full but returned opaque |
| **Context extraction can never fail** | `Ctx<S>` with `type Rejection = Infallible`; the standard stack always injects request-id, identity, and a cancel token | Every handler starts `ctx: Ctx` — no extractor-ordering class of compile errors to debug |
| **Write a capability once, serve it twice** | One `#[derive]`-schema'd `Tool` trait | The same tool is reachable by the in-process agent loop **and** an external MCP server from one definition |
| **Don't re-run completed work on resume** | Write-ahead `running`→`completed` checkpointing + best-effort LIFO saga compensation (at-least-once; steps must be idempotent) | Canary tests resume an interrupted agent/workflow and assert completed steps are memoized, failed steps retry (not poison), and a failing compensator doesn't wedge the saga |
| **The wiring is tested, not hoped** | `hephos::test::TestClient` drives the *real* router in-memory | A mandatory `routes_build` canary catches route-composition panics (e.g. nesting at `/`) before they reach startup |

The quantified trade-offs of these choices — where structured errors *lose* to backtraces
for source-code repair, where the 4-layer tax actually costs tokens, where static tool
schemas blow the context budget — are measured honestly in
[`HEPHOS_AX_PAPER.md`](./docs/reference/HEPHOS_AX_PAPER.md) and asserted in
`crates/hephos-agent/tests/unbiased_evaluation.rs`. AX is not free, and the paper says so.

## What it looks like

A complete, production-shaped endpoint. `ctx` is always first; the body is always
`Json<T>`; the return is always `Result<Json<T>>`; the error is always `hephos::Error`.
Learn it once, and every handler in every Hephos app is this shape.

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

## Aggressively opinionated — on purpose

Hephos gives you **one** error type, **one** handler shape, **one** place each kind of
code lives, **one** way to access state, run a query, add auth, spawn a task, define a
tool. There is no second valid style.

That is a deliberate trade — **coherence over flexibility, predictability over
preference.** Agents (and humans) are fastest and least error-prone when a codebase is
*regular*: when the same kind of thing always looks the same. Every choice Hephos
removes is a bug it prevents and a decision an agent never has to guess at. If you want
a framework that bends to your personal style, this isn't it. If you want a backend
where every file looks like the same careful engineer wrote it — human or agent — that's
the whole point.

## Built above proven infrastructure — not reinvented

Hephos ships no new runtime, HTTP stack, or serialization format. It is a thin, coherent
application layer **above** the best of the Rust ecosystem; its value is exactly the
opinions, guard-rails, and wiring it adds.

| Concern | Reused | What Hephos adds on top |
| --- | --- | --- |
| async runtime | Tokio | structured lifecycle: graceful drain, cancellation that reaches every handler, task, and agent — no detached, unowned work |
| HTTP / routing | Hyper + Axum | one verb-first router and one handler shape; a typed `Ctx` extractor; nesting at `/` is a clear error, not a late panic |
| middleware | Tower | one fixed, correct stack (request-id → trace → timeout → body-limit → cancel); ordering bugs are designed out; auth uses `route_layer` so a 404 stays a 404 |
| serialization | Serde | `Json`/`Path`/`Query` that turn bad input into a clean `400` in the one error type — no rejection zoo to learn |
| database | SQLx (Postgres) | `Db`/`Tx`, a service-owned transaction boundary, executor-generic repos, and rolled-back-transaction test fixtures |
| observability | tracing | one span tree following a request from route → service → repo → agent → tool; OTLP on the roadmap |
| agents / MCP | rmcp | one strongly-typed `Tool` (derived schema + validation + authz) reachable in-process **and** by external MCP clients; every run bounded by turns/tokens/timeout |
| errors | thiserror / anyhow | **one** `Error` for every layer; variants map to HTTP status; internals logged in full, returned opaque — secure by default |
| testing | — (pure Hephos) | `TestClient` drives the **real** router in-memory, plus a mandatory `routes_build` canary |

You could assemble Tokio + Axum + SQLx + rmcp yourself. Hephos is the opinionated,
tested, agent-legible way they fit together — so you don't re-litigate those decisions on
every endpoint.

## Scope

REST · typed errors · services & repositories · auth (argon2 + JWT) · background &
scheduled tasks · events · WebSockets · SSE/streaming · LLM agents · strongly-typed
tools · structured output · durable workflows · MCP — each with **one** canonical
pattern and a copy-ready example in [`examples/`](./examples).

## Performance — honestly

The full standard stack adds **+4.85 µs** (GET) / **+4.10 µs** (POST) of CPU per request
over raw Axum — measured with Criterion on a single core, no network in the loop.

That is a *per-request CPU cost* — a microbenchmark ceiling on the order of **10⁵
req/s/core** — not a sustained end-to-end throughput figure. In any real deployment it
disappears into the network and database latency that dominate a request (<0.05% of a
single 10 ms round-trip). See [`PERFORMANCE.md`](./docs/reference/PERFORMANCE.md) for the full
breakdown.

## Status

Hephos is tagged **v1.0.0** with SemVer 2.0 stability commitments on its public surface
(see [`API_FREEZE.md`](./docs/reference/API_FREEZE.md)). What's in the box:

- **HTTP & stack:** request-id tracking, tracing context, per-route & global timeouts, body limits, baseline security headers, GCRA rate limiting, load shedding, connection caps, graceful draining.
- **Database:** Postgres pooling, embedded migrations, transactional outbox, durable job queues with retry/backoff, durable saga workflows with automated compensation.
- **Security & identity:** Argon2id hashing, rotating JWT access/refresh tokens, revocable API keys, IDOR owner guards, `Secret<T>` redaction + zeroize-on-drop, and audit-logging hooks.
- **Agents & MCP:** `schemars`-derived tool definitions, Anthropic/OpenAI providers, streaming, durable agent execution, MCP server & client.
- **Tooling:** canonical layout, CI architecture enforcement (`hephos check`), environment diagnostics (`hephos doctor`), and code generators.

## Quickstart

```sh
git clone https://github.com/arthurlch/hephos && cd hephos
make ci            # arch-lint + fmt + clippy -D warnings + cargo test
```

Scaffold and check your own project with the CLI:

```sh
cargo install --path crates/hephos-cli
hephos new my-backend     # scaffold a canonical project
cd my-backend
hephos check              # enforce the layer boundaries
hephos doctor             # verify the toolchain/environment
cargo test                # drives the real router via TestClient
```

> The `rest`, `agent`, `tools`, `streaming`, `websocket`, and `mcp` examples run with no
> external services. The `database`, `auth`, and `workflow` examples use SQLx's
> compile-checked queries against Postgres — set `DATABASE_URL` (or run
> `cargo sqlx prepare` to commit an offline query cache) before building those. See
> [`GETTING_STARTED.md`](./docs/GETTING_STARTED.md).

## Documentation

**The doctrine** (root — the canon the agent and the compiler-enforced lints operate by):

| File | Purpose |
| --- | --- |
| [`AGENTS.md`](./AGENTS.md) | The operating manual for coding agents — normative, enforced rules. (`CLAUDE.md` symlinks here.) |
| [`AGENT_PLAYBOOK.md`](./AGENT_PLAYBOOK.md) | The canonical playbook / system prompt for driving an agent against this repo. |
| [`CONVENTIONS.md`](./CONVENTIONS.md) | The single canonical way to write every kind of code. |
| [`ARCHITECTURE.md`](./ARCHITECTURE.md) | Why the system is shaped the way it is. |
| [`CONTRIBUTING.md`](./CONTRIBUTING.md) · [`SECURITY.md`](./SECURITY.md) | How to extend Hephos; how to report vulnerabilities. |

**The manual** ([`docs/`](./docs) — human reference):

| File | Purpose |
| --- | --- |
| [`docs/GETTING_STARTED.md`](./docs/GETTING_STARTED.md) | From clone to a running, tested backend — the hands-on start. |
| [`docs/reference/HEPHOS_AX_PAPER.md`](./docs/reference/HEPHOS_AX_PAPER.md) | The AX thesis and the measured, honest trade-offs behind it. |
| [`docs/reference/PERFORMANCE.md`](./docs/reference/PERFORMANCE.md) | Microbenchmarks and middleware-overhead analysis. |
| [`docs/reference/API_FREEZE.md`](./docs/reference/API_FREEZE.md) | SemVer 2.0 stability commitments and public API surface. |
| [`docs/operations/deployment.md`](./docs/operations/deployment.md) · [`scaling.md`](./docs/operations/scaling.md) | Running and scaling Hephos in production. |
| [`docs/project/MILESTONES.md`](./docs/project/MILESTONES.md) · [`RELEASING.md`](./docs/project/RELEASING.md) | The versioned roadmap and release process. |

## Layout

```
crates/
  hephos/        meta crate: prelude + feature-gated re-exports
  hephos-core/   App, Ctx, Error, Router, extractors, config, tasks, events, test harness
  hephos-db/     Db, Tx — SQLx/Postgres helpers
  hephos-agent/  Model, Agent, Tool, structured output, streaming
  hephos-mcp/    rmcp integration
  hephos-cli/    new · check · doctor · generate · routes
examples/        rest/ auth/ database/ websocket/ streaming/ agent/ tools/ workflow/ mcp/ reference-app/
docs/
  GETTING_STARTED.md
  reference/     HEPHOS_AX_PAPER.md · PERFORMANCE.md · API_FREEZE.md
  operations/    deployment.md · scaling.md
  project/       MILESTONES.md · RELEASING.md
```

## License

Dual-licensed under either [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE), at
your option.
