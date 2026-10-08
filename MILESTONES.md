# MILESTONES.md — the road to Rivet 1.0

This is the long-term plan: every version from today's `0.0.1` scaffold to a
production-ready, API-stable `1.0.0`, roughly a year of work. It replaces the old
one-off architecture review — the forward-looking parts of that review (the
invariants, the testing lesson, what-to-build-first) live here now, as the spine of
the roadmap.

Read this with the doctrine: `CLAUDE.md` (rules), `ARCHITECTURE.md` (why),
`CONVENTIONS.md` (shapes). Nothing here overrides those; it schedules them.

---

## The thesis we are building toward

> **The backend framework built only for coding agents.** If a coding agent can
> understand the framework, it can build the application.

Production-ready, for Rivet, means a coding agent can build, test, observe, secure,
and operate a real backend — REST + DB + auth + background work + agents — by
copying one canonical pattern per task, and a human can run it at scale with
confidence. Every milestone below is judged against that, not against feature count.

## What Rivet adds above the ecosystem (and must keep adding)

Rivet does not ship a new runtime, HTTP stack, or serialization format. It is a thin,
coherent application layer **above** the best of the Rust ecosystem, and its value
lives there. "Thin" is not "small value" — the layer is the product: the opinions,
guard-rails, wiring, and tests that turn excellent primitives into a backend an agent
gets right the first time and a human can operate.

| Concern | Reused | What Rivet adds on top |
| --- | --- | --- |
| async runtime | Tokio | structured lifecycle: graceful drain; cancellation that reaches every handler, task, and agent; no detached, unowned work |
| HTTP / routing | Hyper + Axum | one verb-first router, one handler shape, a typed `Ctx` extractor; nesting at `/` is a clear error, not a late panic |
| middleware | Tower | one fixed, correct stack; ordering bugs designed out; auth via `route_layer` so a 404 stays a 404 |
| serialization | Serde | `Json`/`Path`/`Query` that turn bad input into a clean `400` in the one error type — no rejection zoo |
| database | SQLx (Postgres) | `Db`/`Tx`, a service-owned transaction boundary, executor-generic repos, rolled-back-tx test fixtures |
| observability | tracing | one span tree from route → service → repo → agent → tool; OTLP logs/metrics (roadmap) |
| agents / MCP | rmcp | one strongly-typed `Tool` (derived schema + validation + authz) reachable by an in-process agent **and** external MCP clients; every run bounded |
| errors | thiserror / anyhow | **one** `Error` for every layer; variants map to status; internals logged in full but returned opaque — secure by default |
| testing | — (pure Rivet) | `rivet::test::TestClient` drives the **real** router in-memory; a mandatory `routes_build` canary |

**This is a roadmap commitment, not just a description.** The reason to choose Rivet
over assembling Tokio + Axum + SQLx + rmcp yourself is the value in that right-hand
column — so every milestone must *deepen* it, not merely add surface. A feature that
adds capability without adding a canonical shape, a guard-rail, a test seam, or an
operability win has not earned its place. Concretely, each version grows the column:
`0.0.2` makes the lifecycle/middleware value real; `0.2.0` the observability and
resilience value; `0.3`+ the CLI/codegen value (the biggest agent-productivity
multiplier); `0.5`–`0.7` the agent value (cost, guardrails, durability, memory);
`0.9`–`1.0` the performance, operability, and API-stability value.

## Invariants that hold at every version (never broken)

These are the load-bearing walls. A change that violates one is wrong, regardless
of what it adds.

1. **One error type** (`rivet::Error`) across every layer.
2. **`ctx: Ctx` first** on every handler/task/tool; state only via `ctx.state()`.
3. **Strict layering**: `api → services → repos → db`; handlers never touch SQL,
   services never import HTTP.
4. **No `unsafe`** (`#![forbid(unsafe_code)]`). No `Arc<Mutex<_>>` app state, no
   global mutable state.
5. **Bounded everything**: timeouts, body limits, bounded channels, agent limits;
   background work is structured and owned by the app lifecycle.
6. **One canonical way per task.** A new pattern is a documented change to
   `CONVENTIONS.md` with a matching example — never a silent second style.
7. **`cargo check` ≠ correct.** Every capability ships with a `TestClient`/harness
   test that actually exercises it. "It compiles" is never a completion claim.
8. **Reuse mature infra; add value above it.** No new runtime, HTTP stack, or
   serialization format. A new dependency is justified in writing or not added.

## The process lesson that shaped our bar (keep it in view)

An early revision passed `cargo check`, `cargo build`, and `cargo clippy` while
shipping a startup panic (nesting a router at `"/"`). None of those commands runs a
request. The fix was structural: a framework-owned `TestClient`, a mandatory
`routes_build` canary, and a doctrine rule that the verification bar is
*observed-to-run*, not *compiled*. Every milestone's acceptance criteria below are
written as **observable behavior**, for that reason.

---

## How to read a milestone

Each version lists: **Theme**, **Ship** (what becomes real), **Explicitly not yet**
(to protect coherence), **Acceptance** (observable criteria), and **Deps** (any new
crate, with the one-line justification the doctrine requires). Versions are ordered
by dependency, not by date; indicative sequencing is at the end.

SemVer during `0.x`: minor bumps may change APIs (we say where); `1.0.0` freezes the
public surface under SemVer.

---

## `0.0.1` — Scaffold ✅ (shipped)

**Theme.** Doctrine, API contract, reference examples, project tooling.

- Doctrine: `CLAUDE.md`, `ARCHITECTURE.md`, `CONVENTIONS.md`.
- API contract crates: `rivet-core`, `rivet-db`, `rivet-agent`, `rivet-mcp`, `rivet`.
- Nine reference examples; the REST path runs and is tested over HTTP.
- `rivet::test::TestClient`; the `routes_build` canary.
- Tooling: CI (offline + Postgres + cargo-deny), `.gitignore`, dual license,
  `rustfmt.toml`, `deny.toml`, `Makefile`, Dependabot, PR template, CONTRIBUTING.

**Reality.** Core runtime pieces are still `// BOUNDARY:`. This is a base to build
from, not a working release.

---

## `0.0.2` — The real HTTP core ✅ (shipped)

**Theme.** Make `Ctx` honest. This was the single most important next step.

**Shipped.**
- The standard middleware stack is implemented (`rivet-core::stack`) and installed by
  `App::run` — fixed order, outermost→innermost: **observe** (request id + request span
  + default `Identity` seed + `x-request-id` response header) → **timeout** → **body
  limit** → **cancel** (per-request token) → per-route auth → handler.
- `Ctx::request_id` is real and equals the span's id and the `x-request-id` header.
- The per-request token is a **child of the shutdown token**: it fires on graceful
  shutdown. On client disconnect the handler future / streaming body is dropped by the
  runtime, which stops the work regardless. (Per-request token-on-disconnect wiring is
  a `0.3` item — a naive drop-guard would wrongly kill SSE the instant the handler
  returns.)
- The request **timeout bounds response generation, not body streaming**, so SSE and
  WebSocket handlers (which return their response immediately) are unaffected by
  construction — verified: the streaming example streams for seconds under the default
  30s timeout. Oversized bodies get `413`; slow handlers get `408`.
- Graceful-shutdown hardening: tasks are cancelled, then drained with a bounded
  deadline; task panics (`JoinError`) are logged, not swallowed.
- `Config` finalized: `RIVET_ADDR`, `RIVET_LOG`, `RIVET_TIMEOUT_SECS`,
  `RIVET_BODY_LIMIT` — env-driven, fully unit-tested.
- Extractor rejections no longer leak raw deserializer text — fixed client message,
  detail logged (security).
- `TestClient` now wraps the **real** stack, with `with_stack(timeout, body_limit)` for
  limit assertions and `post_json_with_token` / `TestResponse::{header, error_kind}`.

**Explicitly not yet.** Per-route timeout/body **overrides** and idle timeouts for
long-lived connections (→ `0.3`, when file uploads and real-time maturity need them);
config files (env only).

**Acceptance (met).** `rivet-core` stack tests prove: the response carries an
`x-request-id` matching `ctx.request_id()`; identity is seeded `Anonymous`; a slow
handler returns `408`; an oversized body returns `413`; in-limit requests succeed. The
`streaming` example streams live under the default timeout (not killed).

**Deferred from this milestone.** **Un-pin the toolchain** — the `Cargo.lock` pins
(uuid/time/url/…) can't be dropped while the build toolchain is 1.85 (current latest
transitive crates need rustc ≥1.87). Moves to a follow-up once the toolchain is bumped;
MSRV stays 1.85 until then.

**Deps.** none new (tower-http `timeout`/`limit` already in tree).

---

## `0.0.3` — Database, for real

**Theme.** The persistence path end-to-end, runnable in CI.

**Ship.**
- Wire `Db::migrate` to `sqlx::migrate!` with the app-owned `migrations/` dir;
  migrations run on startup and in tests.
- Pool configuration (size, acquire timeout, statement timeout) via env.
- Transaction ergonomics finalized; repository pattern battle-tested.
- Test fixtures in `rivet::test`: a rolled-back-transaction helper and a
  per-test disposable-database helper.
- `database` + `auth` examples run green in the CI Postgres job (not `#[ignore]`d).

**Explicitly not yet.** Second database backend (Postgres only); read replicas;
query builder/ORM.

**Acceptance.** CI Postgres job builds and runs `database`/`auth` flow tests
(create→read roundtrip, unique-violation→409) against a live schema; fixtures leave
no residue.

**Deps.** none new.

---

## `0.0.4` — Authentication & authorization, hardened

**Theme.** Security-grade identity, not a demo.

**Ship.**
- Token lifecycle decision and implementation: short-lived access tokens + a
  documented refresh/revocation strategy (opinionated: stateless access + a
  revocation list table for logout/compromise), with support for **signing-key
  rotation** (multiple valid keys during a rotation window).
- **Machine-to-machine auth:** Postgres-backed API keys (hashed, scoped, revocable)
  resolved to the same `Identity`, alongside user JWTs — service callers are a
  first-class production need, not an afterthought.
- `argon2` parameters tuned and documented; password policy hooks.
- Role/permission checks consolidated (`require_role`, resource-ownership pattern).
- Security-headers middleware (HSTS, nosniff, frame-deny) and an opinionated,
  minimal CORS story.
- Canonical request-validation pattern (shape at the edge, rules in the service),
  decided once.

**Explicitly not yet.** OAuth/OIDC providers, multi-tenant RBAC, policy engines.

**Acceptance.** `auth` example demonstrates register→login→refresh→logout and
role-gated routes; tests cover 401/403 paths and token expiry/revocation; security
headers asserted.

**Deps.** none expected beyond `jsonwebtoken`/`argon2`.

---

## `0.1.0` — First coherent backend release

**Theme.** "You can build a real REST + DB + auth backend with Rivet."

**Ship.** Consolidation of `0.0.2–0.0.4` into a reviewed, documented release:
REST, DB, auth, in-process tasks, and in-process events all real and tested.
First public API review pass. Complete getting-started docs and a project template.

**Explicitly not yet.** Observability exporters, durable jobs, the agent layer
(all scheduled next).

**Acceptance.** A new app built only by copying examples passes `make ci`, serves,
and handles the two canonical simulations from `ARCHITECTURE.md §24` end-to-end.

**Deps.** none new.

---

## `0.2.0` — Observability & operability

**Theme.** You can see it and run it in production.

**Ship.**
- OpenTelemetry integration via `tracing-opentelemetry` + `opentelemetry-appender-tracing`:
  **logs and metrics on by default** (both are stable upstream), **traces behind a
  flag** (upstream traces are still beta — we adopt, but gate). OTLP export configurable.
- RED metrics (request rate, errors, duration), labeled by **route template**
  (`/users/{id}`), never raw path — unbounded label cardinality would take down the
  metrics backend, so this is a correctness requirement, not a nicety.
- `/health` (liveness) and `/ready` (readiness incl. DB check) endpoints, canonical.
- Panic isolation: a handler panic becomes a logged `500`, never a crash.
- Rate limiting via `tower-governor` (GCRA), one canonical configuration surface.
- Load-shedding / concurrency-limit (Tower) with a documented backpressure story.

**Explicitly not yet.** Custom metrics DSL; distributed tracing propagation across
services beyond the standard OTLP context. **Distributed (cross-instance) rate
limiting** — `tower-governor` is per-instance, so at N replicas the effective limit is
N× the configured value; a shared-store limiter is a later, additive seam (documented
so operators aren't surprised).

**Acceptance.** Metrics and logs export to an OTLP collector in a CI integration
test; a rate-limited route returns `429` under load; a panicking handler returns
`500` and the server stays up; `/ready` flips when the DB is down.

**Deps.** `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry`,
`opentelemetry-appender-tracing` (observability standard, logs/metrics stable);
`tower-governor` (GCRA rate limiting — reuse, don't build).

---

## `0.3.0` — Real-time & streaming maturity

**Theme.** Long-lived connections that behave under load and failure.

**Ship.**
- WebSocket hardening: auth-on-upgrade, heartbeat/idle timeout, max message size,
  backpressure (the no-spawn split-loop pattern as canon), clean close semantics.
- SSE maturity: `Last-Event-ID` resume, retry hints, bounded buffering.
- Per-connection and global connection caps; graceful shutdown drains sockets.
- A `rivet` CLI (first cut): `rivet new` (scaffold an app from the template),
  `rivet routes` (list the route table) — agent-run-ready commands.

**Explicitly not yet.** WebSocket pub/sub fan-out across instances; a CLI plugin
system.

**Acceptance.** Tests prove heartbeat/idle-timeout, oversized-message rejection,
and disconnect cleanup; `rivet new` produces an app that passes `make ci`.

**Deps.** a small CLI arg parser (`clap`) for the `rivet` binary — justified: the
CLI is a distinct, agent-facing surface.

---

## `0.4.0` — Durable background jobs & scheduling

**Theme.** Work that survives a restart, on the one database we already have.

**Ship.**
- Postgres-backed job queue **owned by Rivet** (a `jobs` table; polling + `LISTEN/NOTIFY`),
  aligned to the existing `Task` API — no new infrastructure. (Prior art: `apalis`;
  we stay Postgres-native and opinionated rather than taking a dependency.)
- Retries with exponential backoff, dead-letter, idempotency keys, visibility
  timeouts, orphan re-enqueue.
- Scheduled/cron jobs on the same substrate.
- **Transactional outbox** for events: durable, at-least-once delivery, drained by a
  task — the additive upgrade the in-process bus always pointed at.

**Explicitly not yet.** External brokers (Kafka/NATS); cross-language workers.

**Acceptance.** A job survives a worker restart and runs exactly-once-effectively
(idempotency key); a failing job lands in dead-letter after N backed-off retries;
an emitted event is delivered after a simulated crash between commit and dispatch.

**Deps.** none new (sqlx `LISTEN/NOTIFY`).

---

## `0.5.0` — The agent layer, real

**Theme.** The agent-native promise becomes executable.

**Ship.**
- `Model` providers implemented: Anthropic first, then OpenAI — thin `reqwest`
  clients behind the two-method trait.
- The turn loop + tool dispatch (`parse_tool_calls`) fully implemented and bounded
  by `Limits` (turns/tokens/timeout), with an explicit error on overrun.
- Structured output (`run_typed`) enforced via provider-native constrained output.
- Token streaming through the unified stream primitive (same as HTTP streaming).
- Per-turn / per-tool tracing spans and cancellation inherited from `Ctx`.

**Explicitly not yet.** Durable/long-running agents (needs `0.7`); agent memory
stores; multi-agent orchestration frameworks.

**Acceptance.** The `agent` example runs against a real provider (and a scripted
one offline); a tool-using agent dispatches a tool and returns structured output; a
run that exceeds `max_turns` errors; dropping the stream cancels generation.

**Deps.** none beyond `reqwest`/`schemars` already present.

---

## `0.6.0` — Tools & MCP

**Theme.** One tool definition, reachable by an in-process agent and the world.

**Ship.**
- Tool authorization model and error taxonomy finalized; tool registry.
- MCP **server** via `rmcp`: stdio + Streamable HTTP (current transport), exposing
  Rivet tools, resources, and prompts — one definition, two consumers.
- MCP **client**: consume external MCP servers as agent tools.
- OpenAPI generation from the route table (agent- and client-friendly).

**Explicitly not yet.** A proprietary tool protocol (we use MCP); GraphQL.

**Acceptance.** The `mcp` example serves a Rivet tool over stdio and Streamable
HTTP and is driven by an MCP client in a test; the same tool is callable by an
in-process agent with no second implementation; generated OpenAPI validates.

**Deps.** an OpenAPI generator (or hand-rolled from our constrained router) —
decided during the milestone; MCP via `rmcp` (already in tree).

---

## `0.7.0` — Durable workflows & agent orchestration

**Theme.** Long-running processes that resume after a crash — plain functions, one DB.

**Ship.**
- **Postgres-checkpointed workflows** (DBOS-style): each step ends in a DB
  transaction recording progress; resume reads the last checkpoint. No replay DSL,
  no determinism constraints, no new infra — consistent with Rivet's philosophy.
- Durable agent runs (a long agent survives process restarts).
- Human-in-the-loop approval steps; saga/compensation pattern as canon.
- Cancellation and timeouts at the workflow and step level.

**Explicitly not yet.** Journal/replay engines (Temporal/Restate-style); distributed
workflow scheduling across clusters.

**Acceptance.** A workflow killed mid-run resumes from its last committed step and
completes exactly-once-effectively; an approval step blocks until a decision and
times out safely; a compensation runs on downstream failure.

**Deps.** none new (built on `0.4` jobs + Postgres).

---

## `0.8.0` — Security & compliance hardening

**Theme.** Safe to put on the internet with real data.

**Ship.**
- Full security review and a published threat model.
- Secrets handling: env + file + pluggable provider seam (no secret ever logged).
- Audit logging (who did what) as a canonical capability.
- `cargo-deny` becomes **blocking** in CI; add `cargo-audit`; publish an SBOM;
  evaluate `cargo-vet`/`cargo-vet`-style supply-chain attestations.
- Fuzz the parsers/extractors; abuse protections (auth throttling, request caps).

**Explicitly not yet.** Formal compliance certifications (SOC2 etc. are operator
concerns, not framework ones) — but we provide the hooks they need.

**Acceptance.** CI fails on a known-vulnerable dep or disallowed license; a fuzz
suite runs in CI; the threat model enumerates mitigations with tests; no secret
appears in any log in an integration test.

**Deps.** `cargo-audit` (advisories), fuzz tooling (`cargo-fuzz`), dev/CI only.

---

## `0.9.0` — Performance, scale & API-freeze candidate

**Theme.** Fast, horizontally scalable, and ready to commit to the API.

**Ship.**
- Benchmarks (`criterion` micro + a load harness) with published latency/throughput
  and p99 targets; regression gate in CI.
- **Overhead vs. raw Axum + SQLx**, measured, published, and gated (target: negligible,
  single-digit-% overhead). This is the number that answers "why not just use Axum?" —
  the layer must prove it is nearly free.
- A **reference application** (`examples/reference-app`) combining REST + DB + auth +
  jobs + an agent + MCP — the production proof and the richest copy-target, and what
  the `1.0` acceptance test is run against.
- Validate the **everything-on-Postgres** bet under load; document the thresholds at
  which a concern (jobs, cache, vector search) should move off the primary DB.
- Pool/connection tuning; allocation and copy audit on the hot path.
- Multi-instance readiness: stateless by construction; documented external
  session/cache seam where state is unavoidable.
- Deployment: distroless Docker image, Kubernetes manifests, 12-factor config guide.
- **API-freeze candidate**: deprecation policy, SemVer commitments, a `0.x → 1.0`
  migration guide.

**Explicitly not yet.** Breaking changes after freeze without a deprecation cycle.

**Acceptance.** Load test meets published p99 under target concurrency; a two-instance
deployment handles rolling restarts with zero dropped in-flight requests; the
benchmark gate blocks a regression PR.

**Deps.** `criterion` (dev only).

---

## `1.0.0` — Production-ready, stable API

**Theme.** Commit, and mean it.

**Ship.**
- Public API frozen under SemVer; LTS commitment and support policy.
- Complete documentation site, tutorials, and per-capability guides; every example
  production-grade and security-reviewed.
- `rivet` CLI matured: `new`, `migrate`, `routes`, `openapi`, `check`.
- Independent security audit completed; performance targets validated.
- A published, versioned "agent playbook": the exact prompts/patterns a coding agent
  uses to extend a Rivet app — the capstone of the agent-first thesis.

**Acceptance.** A coding agent, given only the repo and the playbook, builds a
non-trivial production backend (REST + DB + auth + jobs + an agent + MCP) that
passes `make ci`, deploys, and meets the p99 target — with no human disambiguation.

---

## Per-crate improvement backlog (from the crate audits)

Produced by auditing each crate in parallel across five dimensions — security,
speed, LLM-friendliness/token-efficiency, testing, correctness. Every item is
mapped to the milestone that will carry it. Severities: **H** blocks a milestone's
acceptance, **M** should land in its window, **L** opportunistic.

**Pull-forward (these undermine *currently documented* contracts — fix as the
relevant milestone opens, not later):** ~~`From<sqlx::Error>` missing~~ (fixed in
`0.0.2`), `Db::migrate` no-op, ~~extractor error leakage~~ (fixed in `0.0.2`), the
agent `stream`/`timeout` doc claims, and `rivet-mcp` discarding the tool instance.

### `rivet-core`

| Sev | Finding (location) | Fix | Target |
| --- | --- | --- | --- |
| H | `standard_stack` was an empty `Identity` — no timeout, body limit, request-id, or cancellation seed | implemented the fixed Tower stack (`stack.rs`) | ✅ `0.0.2` |
| M | Extractor rejections echoed raw `rejection.body_text()` to clients | fixed message + logged detail | ✅ `0.0.2` |
| M | Background task panic/early-exit unobserved until drain | log task exit incl. `JoinError` + bounded drain deadline | ✅ `0.0.2` |
| H | No unit tests (Config parsing, Error status/kind/opacity, `nest("/")` panic) | added unit + integration + stack tests | ✅ `0.0.2` |
| M | `TestClient` lacked `post_json_with_token` / `error_kind()` | added, plus `with_stack` + `header()` | ✅ `0.0.2` |
| L | `Ctx` keeps `unwrap_or` fallbacks for extensions (`ctx.rs:from_request_parts`) | intentionally retained — `Ctx::detached` (tasks/tests) has no extensions; the stack now always seeds the HTTP path | won't fix |
| M | `authenticated`: strict `Bearer ` only, no token-length cap, `verify` non-401 errors pass through (`router.rs:authenticated`) | normalize scheme, cap length, force-map to 401 | `0.0.4` |
| M | Two ways to build `Internal` (`Error::internal` vs `From`), no context-wrapping (`error.rs`) | document/`.context()` to remove ambiguity | `0.1.0` |

### `rivet-db`

| Sev | Finding (location) | Fix | Target |
| --- | --- | --- | --- |
| H | `rivet-core::Error` had no `From<sqlx::Error>`, so the repo `…await?` snippet would not compile — surfaced as the `database` CI job's `E0277` | added a feature-gated `From<sqlx::Error> → Internal` in `rivet-core` (behind a `sqlx` feature that `rivet-db` enables) | ✅ pulled forward to `0.0.2` |
| H | `Db::migrate` is a silent no-op returning `Ok(())` while its doc promises to run migrations (`lib.rs:migrate`) | take a `&Migrator` (app passes `migrate!()`) or make it a doc-only marker | `0.0.3` |
| H | No `acquire_timeout`/`statement_timeout`; a stuck query pins a connection → pool exhaustion/DoS (`lib.rs:connect`) | set acquire + statement timeouts at connect | `0.0.3` → tune `0.2.0` |
| H | No test fixtures (rolled-back `Tx`, disposable DB) despite CONVENTIONS §5 promising them | add `test_pool()` + `with_rolled_back_tx(..)` | `0.0.3` |
| H | Executor-generic repo signature is the most token-heavy shape to copy; `Tx::exec()` returns a connection, not an `E` | pin one verbatim copy-paste template in docs | `0.0.3` |
| M | Hardcoded `max_connections(16)`, no min/lifetime/idle config (`lib.rs`) | accept pool config via env/struct | `0.0.3` |
| M | No `SQLX_OFFLINE`/`.sqlx` cache doc; compile needs a live DB | document `cargo sqlx prepare`, commit `.sqlx/` | `0.0.3` |

### `rivet-agent`

| Sev | Finding (location) | Fix | Target |
| --- | --- | --- | --- |
| H | `Limits.timeout` is never enforced — `drive` has no wall-clock bound (`agent.rs:drive`) | wrap the loop in `tokio::time::timeout` | `0.5.0` |
| H | `max_tokens` is per-turn, so total ≈ `max_turns × max_tokens` — the "token ceiling" is violated (`agent.rs` / `Limits`) | track a running budget or document as per-turn | `0.5.0` |
| H | Tool-call parsing seam is in `agent.rs:parse_tool_calls` (always `None`); a provider author can't enable tools by implementing `Model` only — contract bug | have `Model::complete` return the tool calls | `0.5.0` |
| H | `stream` does not share the turn/tool loop despite the `Agent` doc claiming it does (`agent.rs:stream` vs doc) | fix the doc now; unify behavior in `0.5` | doc `0.0.x`, impl `0.5.0` |
| M | `tool_result_message` uses `unwrap_or_default()` → silent corruption fed back to the model (`agent.rs`) | propagate the error | `0.5.0` |
| M | Per-turn `messages.clone()` + `tool_schemas()` clone in the hot loop (`agent.rs`) | build tools once; borrow/`Arc` messages | `0.5.0` |
| M | Duplicate tool `NAME`s allowed; dispatch takes first match (`agent.rs:tool/dispatch`) | reject/debug-assert duplicates | `0.5.0` |
| M | No per-turn cap on tool-call fan-out (`agent.rs`) | cap calls per turn | `0.5.0` |
| M | API key held as plain `String`, no redaction (`providers.rs`) | wrap in a zeroizing/secret type | `0.5.0` → `0.8.0` |
| H | No tests; turn loop untestable offline | add a `MockModel`; test limits, dispatch, `run_typed` success/failure | `0.5.0` |

### `rivet-mcp`

| Sev | Finding (location) | Fix | Target |
| --- | --- | --- | --- |
| H | `RegisteredMcpTool::new` discards the tool instance, keeping only the schema — the "deserialize→call→serialize" BOUNDARY is unimplementable from stored state (`lib.rs`) | store an erased invoke closure, mirroring the agent's `RegisteredTool` | `0.6.0` |
| H | `into_route(self)` returns `()` though it claims to mount a nestable endpoint (`lib.rs`) | return a `Router`/service to nest | `0.6.0` |
| H | No identity/authz model at the MCP boundary — external clients could inherit a tool's captured identity (`lib.rs`) | define how an MCP-invoked tool gets `Identity` (default `Anonymous`) | `0.6.0` → `0.8.0` |
| H | No transport auth for Streamable HTTP; default posture unspecified (`lib.rs:into_route`) | mandate mountable-behind-auth + default-deny | `0.6.0` → `0.8.0` |
| M | No resource limits (body, concurrency, timeout) for an external endpoint; internal-error redaction unspecified | specify both in the contract | `0.6.0` |
| M | Tool list written twice (once for `Agent`, once for `McpServer`) — "one definition" not literal | share a `Tools` collection consumed by both | `0.6.0` |
| M | Schema-parity between agent and MCP producers is untested | add a unit test comparing `tool_schema::<T>()` outputs | `0.6.0` |
| L | Unused `anyhow`/`tracing` deps (`Cargo.toml`) | drop until needed | quick |

### `rivet` (meta)

| Sev | Finding (location) | Fix | Target |
| --- | --- | --- | --- |
| H | `rivet::test` is re-exported unconditionally — the internal harness ships in release builds and pulls test plumbing into every binary (`lib.rs` / `rivet-core/lib.rs:pub mod test`) | feature-gate behind `cfg(test)`/`test-util` | `0.1.0` |
| H | Nothing proves the feature matrix compiles | add a `cargo hack --feature-powerset check` CI job | `0.1.0` |
| M | Prelude omits `ws`/`sse`, forcing a second import path in the websocket example (`lib.rs:prelude`) | re-export `ws`/`sse` from the prelude | `0.1.0` |
| M | Agent apps re-import `Completion`/`Message`/`TextStream` after `prelude::*` | add them to the `agent`-gated prelude block | `0.1.0` |
| M | Top-level glob and prelude diverge (`Model`/`ws`/`sse`/`test`) | make the prelude a superset of example needs | `0.1.0` |
| L | Crate `version = "0.1.0"` vs project `0.0.1` metadata drift (several crates) | align versions | quick |

---

## Cross-cutting tracks (advance every version)

These never "complete"; they deepen each release. Each has a standing bar.

- **Testing & quality.** `TestClient`/harness test per capability (non-negotiable);
  unit + integration; property-based (`proptest`) for parsers/serializers (`0.2`);
  snapshot/golden tests for API responses (`0.2`); a mock `Model` (`0.5`); load +
  `criterion` benches with a regression gate (`0.9`); fuzzing of
  extractors/parsers (`0.8`); coverage reported (gate at `0.9`); flaky-test quarantine.
- **Documentation.** Rustdoc on every public item; an mdBook doc site (tutorials,
  how-tos, cookbook, reference) from `0.1`; Architecture Decision Records (ADRs) for
  every canonical-pattern decision; runbooks and an operations guide by `0.9`;
  versioned docs at `1.0`. No feature merges without its `CONVENTIONS.md` shape and
  an example in the same change.
- **Developer experience & the `rivet` CLI.** The CLI is a first-class, agent-facing
  surface and a force multiplier for the agent-first thesis: `new` (`0.3`),
  `routes`/`doctor` (`0.3`), `migrate` (`0.3`), `generate` (scaffold an
  endpoint/service/repo/tool/agent in the canonical shape — the single biggest
  token-saver for a coding agent, `0.4`), `openapi` (`0.6`), `check` (`0.9`). Plus
  high-quality compiler diagnostics and a watch/dev mode.
- **Security.** Standing: `cargo-deny`/`cargo-audit`, SBOM, secret scanning,
  Dependabot, dependency-update review; threat-model upkeep; `SECURITY.md` +
  `security.txt` + responsible-disclosure process. Hardens to blocking at `0.8`.
- **Performance.** A published latency/throughput/p99 budget per capability; an
  allocation/clone audit on hot paths each release; a benchmark regression gate at
  `0.9`.
- **API conventions.** One canonical shape for each of: pagination, filtering,
  sorting, the error body (keep `{error:{kind,message}}` or adopt RFC 9457 — decided at
  `0.1.0`, see Risks), idempotency keys, conditional requests (ETag/If-Match), content
  negotiation + compression (gzip/brotli), and URL API versioning. Landed across
  `0.1`–`0.2`, frozen at `0.9`.
- **Resilience.** Outbound calls (DB, HTTP, providers) get timeout + bounded
  retry/backoff + circuit-breaker + bulkhead as canonical Tower middleware (`0.2`);
  idempotency and graceful degradation patterns documented.
- **Dependency budget.** The standard stack is the default answer; every addition is
  justified in the PR and checked by `cargo-deny`.

## Capability areas mapped onto the ladder

Breadth a production framework needs, each with Rivet's **opinionated** answer and
its target. Where the honest answer is "an operator/vendor concern," Rivet ships a
**seam, not an implementation** (see the principle at the end).

| Area | Rivet's opinionated provision | Target |
| --- | --- | --- |
| Error bodies | `{error:{kind,message}}` today (shipped); whether to adopt RFC 9457 `application/problem+json` is an **open decision** — see Risks | decide `0.1.0` |
| Pagination / filter / sort | one cursor-based convention + query extractors | `0.1.0` |
| API versioning | URL-prefixed (`/v1`), documented deprecation of old versions | `0.1.0` |
| Idempotency | `Idempotency-Key` on unsafe methods, Postgres-backed dedupe | `0.2.0` |
| Content negotiation / compression | JSON default; gzip/brotli response compression | `0.2.0` |
| Conditional requests | ETag / `If-None-Match` / `If-Match` helpers | `0.2.0` |
| Health & probes | `/health` (live), `/ready` (DB-aware), startup gate | `0.2.0` |
| Rate limiting / abuse | `tower-governor` (GCRA), per-key quotas | `0.2.0` |
| CORS / CSRF / security headers | one opinionated, minimal, documented config | `0.0.4`/`0.2.0` |
| Caching | response + data cache seam (in-proc default; external seam) | `0.2.0` |
| Config & feature flags | env-first typed config; a simple flag seam | `0.0.2`/`0.3.0` |
| Secrets management | env/file + pluggable provider seam; never logged | `0.8.0` |
| File uploads & object storage | streaming multipart + an object-store trait (local + S3 seam) | `0.3.0` |
| Email / notifications | a `Notifier` trait + templating; SMTP/provider seam | `0.4.0` |
| Outbound webhooks | signed, retried, dead-lettered — built on the job queue | `0.4.0` |
| Migrations | up/down, CI-gated, zero-downtime guidance | `0.0.3` |
| Search | Postgres full-text first; pgvector for semantic | `0.9.0`/agent track |
| Audit logging | canonical, append-only, PII-aware | `0.8.0` |
| Data retention / PII / GDPR | deletion + retention hooks (framework provides the seam) | `0.8.0` |
| i18n / time | UTC everywhere; locale/timezone handling documented | `0.1.0` |
| Deployment artifacts | distroless Docker, k8s manifests, Helm chart, 12-factor guide | `0.9.0` |
| Zero-downtime | connection draining, rolling-restart safety, readiness gating | `0.2.0`/`0.9.0` |

## Agent-native capability track (the differentiator)

This is why Rivet exists; it is planned deepest. All of it is built on the same
primitives (Postgres, tasks, workflows, the `Model`/`Tool`/`Agent` core) — no new
infrastructure, consistent with the philosophy.

| Capability | What Rivet provides | Target |
| --- | --- | --- |
| Core loop | bounded turn loop, tool dispatch, structured output, streaming | `0.5.0` |
| Cost & quota | token accounting, per-tenant/agent budgets, cost tracking | `0.5.0` |
| Provider reliability | retries, rate-limit handling, fallback/failover models, routing | `0.5.0` |
| Prompt caching | provider prompt-cache support surfaced in the `Model` API | `0.5.0` |
| Tools & MCP | one `Tool` def → agent + MCP; tool authz; registry | `0.6.0` |
| Guardrails | input/output moderation hooks, PII redaction, output-schema validation | `0.6.0` |
| Sandboxing & HITL | capability-scoped tools, execution isolation, human approval gates | `0.6.0`/`0.7.0` |
| Durable & long-running agents | agent state survives restarts (on `0.7` workflows) | `0.7.0` |
| Memory | short-term context mgmt + Postgres-backed long-term memory + summarization | `0.7.0` |
| Observability & replay | per-run traces (reasoning + tool calls), replayable runs | `0.5.0`/`0.6.0` |
| Evaluations | offline eval harness + regression evals in CI (agent quality gate) | `0.8.0` |
| Prompt-injection defense | injection/jailbreak detection as a guardrail + threat-model item | `0.8.0` |
| RAG / embeddings | embeddings via `Model`, **pgvector** vector search (Postgres-native) | `0.9.0` |
| Multi-agent | orchestration via workflows (simple handoffs); complex frameworks deferred | `0.7.0` / post-1.0 |

Guiding opinion: an agent capability ships only when it has **one canonical shape**,
a `TestClient`/eval test, and a canonical example — the same bar as everything else.
We do not chase every agent-framework feature; we provide the few that compose.

## Production-readiness checklist (the 1.0 definition of done)

`1.0.0` ships only when every box is true and tested. This is the contract for
"production ready."

**Correctness & API**
- [ ] Public API frozen; `cargo-semver-checks` guards it in CI.
- [ ] One canonical shape per artifact; no `// BOUNDARY:` remains in shipped crates.
- [ ] Error-body shape finalized (keep `{error:{kind,message}}` or adopt RFC 9457 — decided at `0.1.0`); pagination/filter/sort/versioning/idempotency conventions stable.

**Data**
- [ ] Migrations up/down, CI-gated, zero-downtime guidance; pool + statement timeouts configurable.
- [ ] Test fixtures (rolled-back tx, disposable DB); `.sqlx` offline cache committed.

**Security**
- [ ] `#![forbid(unsafe_code)]` everywhere; independent security audit passed.
- [ ] Verified auth (tokens + refresh/revocation), RBAC, security headers, CORS, rate limiting.
- [ ] Secrets never logged; `cargo-deny`/`audit` blocking; SBOM published; supply-chain attested.
- [ ] Input validation canonical; fuzzed parsers; threat model with tested mitigations.

**Observability & ops**
- [ ] OTel logs + metrics (traces gated); RED metrics; `/health` + `/ready`.
- [ ] Structured logs with request-id correlation and redaction; audit logging.
- [ ] Graceful shutdown with connection draining; zero-downtime rolling restarts proven.

**Reliability**
- [ ] Outbound timeout/retry/circuit-breaker/bulkhead; durable jobs + outbox; durable workflows resume after crash.

**Performance**
- [ ] Published p99 targets met under load; benchmark regression gate; hot-path allocation audit.

**Testing**
- [ ] Every endpoint: success + failure `TestClient` tests; `routes_build` canary.
- [ ] Property, fuzz, load, and (for agents) eval suites in CI; coverage gate.

**Docs & DX**
- [ ] Doc site, tutorials, cookbook, ADRs, runbooks, migration guides; every example production-grade.
- [ ] `rivet` CLI: `new`, `generate`, `migrate`, `routes`, `openapi`, `check`, `doctor`.
- [ ] Deployment artifacts (Docker/k8s/Helm) and a 12-factor guide.

**Agent**
- [ ] Bounded runs (turns/tokens/timeout enforced); cost accounting; guardrails; HITL.
- [ ] Durable agents; memory; evals in CI; one `Tool` def reachable by agent + MCP.

**Release & governance**
- [ ] SemVer + deprecation + MSRV policies published; LTS + security-backport policy.
- [ ] Published, versioned **agent playbook** (the capstone of the thesis).

## Release engineering, versioning & governance

- **Versioning.** SemVer from `1.0`. A written **public-API-surface** definition
  (what is covered). **Deprecation policy**: deprecate for ≥1 minor before removal,
  with a migration note. `cargo-semver-checks` in CI blocks accidental breaks.
- **MSRV & edition.** A stated Minimum Supported Rust Version with a CI job pinned to
  it; a policy for bumping MSRV and Rust edition (currently 2024 / 1.85).
- **Release process.** Automated (e.g. `release-plz`): changelog from commits, tagged
  releases, publish order `core → db/agent/mcp → meta`, docs.rs, signed artifacts.
- **Compatibility CI.** Feature-powerset check (`cargo hack`), MSRV job, OS matrix
  (Linux primary; macOS best-effort), `cargo-semver-checks`.
- **Support.** An LTS line with a defined window and security backports; a published
  EOL policy.
- **Governance.** A lightweight **RFC process** for any new canonical pattern (a new
  pattern is never added silently — invariant #6); ADRs committed; `CODEOWNERS`; a
  Code of Conduct; a maintainer/decision model; `SECURITY.md` + disclosure process.

## Risks, trade-offs & open decisions

A roadmap that doesn't own its risks isn't a plan. These are the strategic bets and
the unsettled technical decisions — surfaced so they are chosen deliberately, and
revisited, rather than discovered in production.

### Strategic bets

- **Everything-on-Postgres.** Jobs, the event outbox, workflows, idempotency,
  rate-limit dedupe, token revocation, agent memory, and `pgvector` all ride one
  Postgres. This is the "no new infrastructure" bet that keeps the system legible — and
  it concentrates load and blast radius on a single node. *Mitigation:* `0.9` load-tests
  Postgres as the backbone and documents the thresholds at which a concern should move
  off the primary DB; the seams already exist. Revisit if the single-DB ceiling is hit.
- **Agent-layer timing.** The agent value (`0.5`+) is the differentiator but lands
  mid-roadmap. *Open decision:* ship a minimal agent MVP right after `0.2` to validate
  the thesis and attract adoption, trading some REST/ops polish for an earlier proof of
  the thing that makes Rivet distinctive?
- **Ecosystem & provider churn.** Rivet rides Axum/Tower/SQLx/rmcp and vendor LLM APIs;
  an upstream breaking change or a provider API shift is a maintenance tax. *Mitigation:*
  the wrapper surface is deliberately small and version-pinned; providers sit behind the
  two-method `Model` trait, so a vendor change is contained to one file.
- **The meta-risk.** If agents get good enough to wield un-opinionated frameworks
  directly, Rivet's edge narrows. *The bet:* regularity still cuts tokens, errors, and
  review cost even for strong agents — and that advantage grows with codebase size, not
  shrinks.

### Technical decisions to settle before the API freeze

- **Error body — keep or adopt RFC 9457?** Today it is `{"error":{"kind","message"}}`
  (shipped, and documented in `CONVENTIONS`/`ARCHITECTURE`). RFC 9457
  `application/problem+json` is the interoperable standard and can still carry our
  `kind`. Adopting it is a **pre-0.1 breaking change** that must update the code and
  both doctrine docs together. **Decide at `0.1.0`.** Until then, every "Problem
  Details" mention in this file is aspirational, not the current contract.
- **Timeout vs. streaming** (`0.0.2`): the global request timeout must exempt
  streaming/SSE/WebSocket routes (idle timeout instead) or it kills every stream.
- **Body limit vs. uploads** (`0.0.2`/`0.3`): the global body limit must be
  per-route-overridable or it blocks file uploads.
- **Rate limiting is per-instance** (`0.2`): `tower-governor` holds state in-process, so
  at N replicas the effective limit is N×; distributed limiting is a deferred, additive
  seam — documented, not silently wrong.
- **Cross-instance real-time** (`0.3`+): WebSocket fan-out across instances is deferred;
  single-instance or sticky-session only until a shared fan-out seam lands.

### The adoption bar (why an engineer actually switches)

- Overhead vs. raw Axum+SQLx is negligible **and published** (`0.9`).
- A **reference application** proves the whole stack composes (`0.9`).
- The **agent playbook** makes "an agent builds a production backend with no human
  disambiguation" reproducible, not a slogan (`1.0`).

## Explicitly out of scope until after 1.0

Committing to these now would trade away the coherence that is the whole point.
The governing principle for breadth: **Rivet ships seams, not vendor
implementations** — an object-store trait, a `Notifier` trait, a secrets-provider
trait, a cache seam — so an operator plugs in S3 / SES / Vault / Redis without Rivet
taking a dependency on any of them.

- A second database backend; a query builder or ORM.
- gRPC/`tonic` as a first-class transport (the `POST /rpc/{method}` convention holds).
- GraphQL.
- A plugin system, a DI container, or a config-struct framework.
- Journal/replay durable execution (we chose Postgres checkpointing).
- External message brokers as the default (outbox-on-Postgres is the default).
- A proprietary agent/tool protocol (we use MCP).
- More than two or three model providers in-tree; a general multi-agent framework.
- Bundled cloud/vendor integrations (S3, SES, Vault, Redis, Kafka) — seams only.
- A web UI / admin panel beyond an optional read-only jobs/agents inspector.

## Indicative sequencing (~13–14 months)

| Window | Versions |
| --- | --- |
| Months 1–2 | `0.0.2`, `0.0.3`, `0.0.4` |
| Month 3 | `0.1.0` |
| Month 4 | `0.2.0` |
| Month 5 | `0.3.0` |
| Month 6 | `0.4.0` |
| Months 7–8 | `0.5.0` |
| Month 9 | `0.6.0` |
| Month 10 | `0.7.0` |
| Month 11 | `0.8.0` |
| Month 12 | `0.9.0` |
| Months 13–14 | `1.0.0` |

Dates are indicative and dependency-ordered, not commitments. The ordering is the
contract; the calendar flexes. Ship a version only when its **Acceptance** criteria
are observably met — not when it compiles.
