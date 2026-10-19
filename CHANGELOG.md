# Changelog

All notable changes to Hephos are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow SemVer.

## [0.6.0] - 2026-10-20

### Added
- **Unified Tools & MCP Architecture (`hephos-agent::tool`, `hephos-mcp`)**:
  - One tool definition (`hephos_agent::Tool`) exposed to both in-process `Agent` and external MCP clients via stdio or Streamable HTTP.
  - Shared `Tools` collection and `ToolRegistry` with `ToolDefinition` erased wrappers supporting both statically typed tools and dynamic closures.
  - Tool schemas generated via `schemars` (`ToolSchema`) with owned name, description, and JSON schema strings.
  - `McpServer` implementing `rmcp::ServerHandler` over stdio (`serve_stdio`), arbitrary async read/write pairs (`serve_io`), and Streamable HTTP (`into_router`).
  - `McpClient` providing `to_tools` and `to_tool` enabling in-process agents to seamlessly consume external MCP servers as native tools.
- **Router Services & OpenAPI 3.1 (`hephos-core`, `hephos-cli`)**:
  - `Router::nest_service` mounting generic Tower / Axum services (such as `rmcp::transport::StreamableHttpService`) into the Hephos routing tree.
  - Runtime OpenAPI 3.1.0 generator via `Router::openapi(title, version)` reflecting routes, HTTP methods, and parameter paths.
  - Static OpenAPI generator via `hephos openapi` CLI command inspecting route definitions without executing server binaries.
- **MCP Example & Integration Tests (`examples/mcp`)**:
  - `examples/mcp` demonstrating dual-consumption of the same `SearchUsers` tool by both in-process `Agent` and external MCP clients.
  - Comprehensive end-to-end integration tests in `examples/mcp/src/tests.rs` verifying route composition, OpenAPI validation, in-process agent execution, duplex-channel MCP client execution, and agent consumption of client tools.

## [0.5.0] - 2026-10-19

### Added
- **Real Model Providers (`hephos-agent::providers`)**:
  - `Anthropic`: Complete and SSE streaming client against `POST /v1/messages` using `reqwest`. Supports text generations, tool calling (`tool_use`), tool result feedback (`tool_result`), and native structured output via constrained tool choices.
  - `OpenAI`: Complete and SSE streaming client against `POST /v1/chat/completions`. Supports text responses, tool calling (`type: function`), tool result replies (`role: tool`), and structured output enforcement via `response_format` with `json_schema`.
  - Configurable `with_base_url` on both providers for offline mock testing, local model gateways, or custom proxy deployments.
- **Turn Loop & Tool Dispatch (`Agent`)**:
  - Full turn-based tool execution loop with `parse_tool_calls` model seam, extracting requested tools, dispatching to registered tools, and feeding `ToolResult` messages back to the model.
  - Runtime bounds via `Limits`: turn limits (`max_turns`), token ceilings (`max_tokens`), wall-clock timeouts (`timeout`), and per-turn tool-call fan-out bounds (`max_tool_calls_per_turn`).
  - Cancellation propagation via `cancel_token(CancellationToken)` integrating with `Ctx::cancel_token`.
  - Tracing spans attached per turn (`agent.turn`) and per tool execution (`agent.tool`).
  - `Agent::stream` handles tool-free agents via direct model token streaming and tool-using agents by executing tool turns and streaming the final text deltas.
  - Dropping the stream immediately aborts generation and closes underlying network streams.
- **Structured Output (`Agent::run_typed`)**:
  - Native JSON schema generation via `schemars` and validation for type-safe structured agent outputs.
- **Example Agent Integration**:
  - Enhanced `examples/agent` to dynamically use `Anthropic` or `OpenAI` when `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` is present in the environment, falling back smoothly to `ScriptedModel` for offline and CI runs.

## [0.4.0] - 2026-10-19

### Added
- **Postgres-Backed Durable Job Queue (`hephos-db::jobs`)**:
  - `Jobs` handle for enqueueing, claiming, completing, and retrying durable background jobs using Postgres as the single storage engine.
  - Dedicated `hephos_jobs` schema with indexing for fast claims, visibility timeouts, and dead-letter retrieval.
  - Exactly-once-effectively deduplication via `idempotency_key` (`ON CONFLICT (idempotency_key) DO UPDATE`).
  - Worker pool (`JobWorker<S>`) implementing `hephos_core::Task<S>` for seamless integration with `App::task`.
  - Immediate execution through Postgres `LISTEN/NOTIFY` on the `hephos_jobs` channel paired with fallback timer polling.
  - Retries with exponential backoff (`base_backoff * 2^(attempts-1)`), visibility timeouts, orphan re-enqueue, and a dead-letter queue (`JobState::DeadLetter`) after exhausting `max_attempts`.
  - Scheduled and recurring jobs (`JobOptions::delay`, `JobOptions::run_at`, `JobOptions::recurring`).
- **Transactional Outbox Pattern (`hephos-db::outbox`)**:
  - `Outbox` handle with `enqueue_tx` allowing atomic domain data modification and event publishing in the same SQL transaction.
  - Dedicated `hephos_outbox` schema with indexing on pending events.
  - `OutboxDrain<S>` implementing `hephos_core::Task<S>` for continuous, durable, at-least-once event delivery across application restarts and failures.
- **Top-Level Re-Exports (`hephos::prelude`)**:
  - Re-exported `Job`, `JobOptions`, `JobRegistry`, `JobState`, `JobWorker`, `Jobs`, `Outbox`, `OutboxDrain`, `OutboxEntry`, `OutboxState` in `hephos::prelude` when `feature = "db"` is active.

## [0.3.0] - 2026-10-19

### Added
- **`hephos` CLI (`crates/hephos-cli`)**:
  - `hephos new <name>` scaffolds a canonical AGENTS.md-compliant project structure (`Cargo.toml`, `src/main.rs`, `src/state.rs`, `src/api.rs`, `src/api/hello.rs`, `src/domain.rs`, `src/domain/hello.rs`, `src/services.rs`, `src/services/hello.rs`, `src/tests.rs`) with valid Rust crate name verification and offline local path override support (`--local-path`).
  - `hephos routes [path]` inspects route tables statically with tabular and `--json` formatting, listing methods, route paths, and handler function names.
- **Dynamic Route Reflection (`Router::entries`)**:
  - Added `RouteEntry { method, path }` to `hephos-core::router` and re-exported in `hephos_core::*` and `hephos::*`.
  - `Router::entries(&self)` reflects registered routes across all HTTP verbs (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`) through `merge` and `nest` operations.
- **Connection Lifecycle & Caps (`ConnectionCap`, `ConnectionGuard`)**:
  - Global and per-route connection concurrency limits via `ConnectionCap`.
  - Graceful connection draining with configurable timeout (`drain()`) on shutdown.
  - Probe endpoints (`/health`, `/ready`) are unconditionally exempted from connection limits.
- **WebSocket Hardening (`hephos-core::ws`)**:
  - `WsUpgrade` extractor with handshake validation and `on_upgrade` callback.
  - Configurable message limits (`max_message_size`, `max_frame_size`).
  - Heartbeat / ping-pong tracking and idle timeouts.
  - Clean close semantics and close frame helpers (`CloseFrame::normal`, `CloseFrame::going_away`).
- **SSE Maturity (`hephos-core::sse`)**:
  - `Last-Event-ID` extractor supporting both `Last-Event-ID` HTTP header and `last_event_id` query parameter fallback.
  - Typed and untyped last event ID parsing (`parse_id::<T>()`).
  - `SseEvent` constructors supporting `event`, `data`, `id`, `retry`, and `comment`.

### Changed
- **Folder and Files Architecture Modularization**:
  - Organized `crates/hephos-core/src/` real-time, streaming, and connection-lifecycle primitives (`ws`, `sse`, `conn`) into a dedicated `transport/` module (`crates/hephos-core/src/transport/`), while preserving 100% backward compatibility for all crate-level re-exports (`hephos_core::{ws, sse, conn}`, `hephos::{ws, sse, conn}`, and `hephos::prelude`).
  - Added dedicated integration test suites `crates/hephos-core/tests/ws.rs` and `crates/hephos-core/tests/sse.rs` to accompany `crates/hephos-core/tests/conn.rs`.
  - Cleaned up `examples/` directory architecture to strictly conform to `AGENTS.md § 1`: relocated loose `token.rs` in `examples/auth` into `services/token.rs`, and relocated loose `model.rs` in `examples/agent` and `examples/workflow` into `agents/model.rs`, eliminating loose source files at the example `src/` roots.
- **Renamed `CLAUDE.md` → `AGENTS.md`** as the canonical, agent-agnostic operating
  manual (any coding agent — Claude, Gemini, Cursor, or a human — reads the same rules).
  `CLAUDE.md` is now a symlink to `AGENTS.md`, so tools that auto-load either name get the
  doctrine with no duplicated source of truth. All references updated across the repo.

## [0.2.0] - 2026-10-19
- **OpenTelemetry & OTLP export in the standard stack**: integrated `opentelemetry`, `opentelemetry-otlp`,
  `opentelemetry-appender-tracing`, and `tracing-opentelemetry`. Standard OTLP HTTP transport exports
  logs and metrics by default when configured via `HEPHOS_OTLP_ENDPOINT` (or `OTEL_EXPORTER_OTLP_ENDPOINT`)
  or programmatically via `App::otlp_endpoint`. Distributed tracing export is gated behind `HEPHOS_TRACES` /
  `HEPHOS_OTEL_TRACES` or `App::traces` (default false while upstream tracing is beta). Telemetry handles
  ensure all providers flush and shut down cleanly on graceful shutdown.
- **Canonical RED metrics (Rate, Errors, Duration) with bounded label cardinality**:
  automatic request measurement via `RedMetrics` (`http.server.requests`, `http.server.request.duration`,
  `http.server.errors`). Labels are strictly constrained to `http.request.method`, `http.route`, and
  `http.response.status_code`. Route labeling strictly uses the matched route template (`/users/{id}`)
  rather than raw request paths, with unmatched requests mapped to `"__unmatched__"` to prevent cardinality
  explosions in the metrics backend. Supported in `App` and `TestClient` with custom hook instrumentation.
- **Canonical data-retention reaper pattern** (`examples/auth`): scheduled background task
  (`TokenReaper` in `tasks/reaper.rs`) that periodically purges dead security rows (expired or
  revoked `refresh_tokens` and `api_keys`) on a configurable grace window, keeping security tables
  bounded in production.
- **Load shedding & concurrency limiting in the standard stack**: global in-flight request
  bounding via `tower::limit::GlobalConcurrencyLimitLayer` and `tower::load_shed::LoadShedLayer`.
  Configured via `HEPHOS_CONCURRENCY_LIMIT` or programmatically via `App::concurrency_limit(limit)` /
  `TestClient::with_concurrency_limit`. Saturated servers immediately shed excess load with `503 Service Unavailable`
  (`{"error":{"kind":"unavailable","message":"service overloaded, shedding load"}}`), preventing in-memory
  request buffering and cascading latency failures under overload. Health probes (`/health`, `/ready`)
  are cleanly excluded from load shedding and remain responsive under load.
- **Strict cargo-deny policy and CI enforcement**: configured `deny.toml` with documented advisory
  exemptions for transitive crates, permissive licenses including `CDLA-Permissive-2.0`, and allowed
  workspace wildcard paths. Removed `continue-on-error` from GitHub Actions CI so supply-chain security
  checks are strictly enforced.
- **Rate limiting via `tower-governor` GCRA in the standard stack**: per-IP rate limiting
  configured via `HEPHOS_RATE_LIMIT_RPS` and `HEPHOS_RATE_LIMIT_BURST` or programmatically via
  `App::rate_limit(rps, burst)` / `TestClient::with_rate_limit`. Uses `SmartIpKeyExtractor`
  (extracting client IP from `ConnectInfo<SocketAddr>` or `x-forwarded-for` / `x-real-ip` headers)
  and emits standard rate limit response headers (`x-ratelimit-after`, `x-ratelimit-limit`,
  `x-ratelimit-remaining`, `retry-after`). Rate-limited requests return status `429` with error
  contract `{"error":{"kind":"rate_limited","message":"too many requests"}}`. Probes (`/health`,
  `/ready`) are cleanly excluded from rate limiting and remain accessible even under heavy traffic.
  A background reaper runs against cancellation to periodically evict idle IP state.
- **Canonical `/health` and `/ready` probes** (`0.2.0`): `/health` (liveness) is
  auto-mounted by `App` and returns `200` (`{"status":"ok"}`); `/ready` (readiness)
  defaults to `200` (`{"status":"ready"}`) and supports custom async checks via
  `App::readiness(check)` (e.g. `move || { let db = db.clone(); async move { db.ping().await } }`),
  flipping to `503 Service Unavailable` with opaque error reporting when a dependency fails.
  Adds `Db::ping()` (`SELECT 1`), path reservation in `Router` to protect probe paths,
  and `TestClient::from_app` for end-to-end probe testing.
- **Build/test-speed accelerators** (opt-in, zero-install default preserved): `make
  nextest` (faster test runner) alongside the existing opt-in faster-linker/`sccache`
  guidance in `.cargo/config.toml`; AGENTS.md §11 lists them as the tighter agent loop.
  `cargo test` stays the CI/`make test` default so a clean checkout always works
  (addresses external review on compile-time/DX; forcing a linker or runner would break
  checkouts that lack the tool, so these stay opt-in by design).
- **Canonical handler escape hatches** (`CONVENTIONS.md §2.13`, AGENTS.md §4): one
  sanctioned shape each for the ~20% of endpoints that aren't `Result<Json<T>>` — custom
  status/body, response headers, raw body + request headers (webhooks), SSE/streaming,
  WebSocket upgrade, redirects/files. New `hephos::http` module re-exports the needed
  primitives (`Bytes`, `HeaderMap`, `HeaderName`, `HeaderValue`, `Response`,
  `IntoResponse`) so apps never reach into `axum` directly (addresses external review:
  "define canonical patterns for the 20%, don't let an agent guess how to break the rule").
- **Architecture-boundary lint** (`scripts/arch-lint.sh`, `make arch-lint`, CI): the
  load-bearing AGENTS.md §2 layering rules are now machine-enforced on application code —
  the build fails if a handler runs SQL or imports a repo, if a service names an
  `axum`/HTTP type, or if a repo calls up into a service. Doctrine that the compiler
  rejects, not just documents (addresses external review feedback).
- **Panic isolation**: the standard stack now catches a handler panic (innermost, via
  `tower-http`'s `CatchPanicLayer`) and turns it into the standard opaque `500` instead
  of letting the worker task unwind — the server stays up. The panic payload is logged
  in full (as an `Error::Internal` cause) and never leaks to the client; the response
  still flows out through `observe`, so it carries the request id and security headers.

## [0.1.0] - 2026-10-18

First coherent backend release: REST + database + authentication/authorization, with
in-process tasks and events — all real and tested. Consolidates `0.0.2`–`0.0.4`, with a
first public API review pass that freezes the stable surface. The `agent`/`mcp` features
are pre-1.0/unstable and not covered by the stability promise.

### Added (0.1.0)
- **End-to-end test for §24 simulation 1** (`examples/database`): asserts a successful
  `create` emits `AppEvent::UserCreated` to a subscriber *after* commit — closing the
  one untested link in the authenticated-write + DB + event chain. `ARCHITECTURE.md §24`
  now states the realization status honestly (simulation 2's full agent→tool dispatch is
  the agent layer, deferred to `0.5.0`).
- **`GETTING_STARTED.md`**: a hands-on guide from a clean checkout to a running, tested
  backend — which example to copy for what, the app anatomy and layering rule, and the
  canonical way to add an endpoint, a database-backed resource, and auth. Linked from the
  README. The examples are the project templates (copy → rename → strip).

### Changed (0.1.0 — API stabilization)
- **First public API review pass.** Froze the stable surface ahead of `0.1.0`:
  - `Error`, `Config`, and `PoolConfig` are now `#[non_exhaustive]`, so adding a
    variant/field later is not a breaking change. App code is unaffected (errors are
    built via `Error::*` helpers; config via `*_from_env`/`Default`).
  - The **`agent` and `mcp` features are documented as pre-1.0 / unstable** — their
    provider and server seams still have placeholders and are not covered by the 0.1.0
    stability promise (scheduled for `0.5.0`/`0.6.0`).
  - Added `Db::connect_lazy_with(url, config)` to close the asymmetric constructor
    family (`connect`/`connect_with`/`connect_lazy`/`connect_lazy_with`).
  - Doc-comment gaps filled on load-bearing public items: `Tx::commit`/`rollback`,
    `Error::not_found`/`invalid`/`conflict`, and the `Config`/`PoolConfig`/`LogFormat`
    fields and variants.

### Added (0.0.4 — Auth & authz hardening)
- **Baseline security headers** on every response, set by the standard stack:
  `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy:
  no-referrer`. `Strict-Transport-Security` is opt-in via `HEPHOS_HSTS` (TLS only).
- **CORS** via `HEPHOS_CORS_ORIGINS` (comma-separated allow-list, or `*`). Off by
  default (same-origin); an opinionated minimal layer (standard methods +
  `content-type`/`authorization`, no credentials), preflight handled outermost.
- **JWT signing-key rotation** (auth example): `token::verify` takes a keyring and
  accepts a token signed under any key. Set `JWT_SECRET_PREVIOUS` alongside
  `JWT_SECRET` during a rotation for zero-downtime key rollover.
- **Token lifecycle: refresh + revocation** (auth example). Login now returns a
  `{ access_token, refresh_token }` pair: a short-lived (15 min) access JWT and a
  long-lived (30 day) opaque refresh token. New endpoints `POST /auth/refresh`
  (rotates the refresh token and mints a new pair) and `POST /auth/logout` (revokes
  it, `204`, existence-opaque). Refresh tokens are stored only as SHA-256 hashes in a
  `refresh_tokens` table; presenting an already-rotated token triggers reuse/theft
  detection and revokes the user's whole token family.
- `StatusCode` is now in `hephos::prelude`, so the doctrine's `Result<StatusCode>`
  `204` handler shape (AGENTS.md §4) works without importing `axum`.
- **Auth-seam token-length cap** (`MAX_TOKEN_LEN`, 4 KiB): the bearer extractor
  rejects oversized credentials before `verify` runs, so a caller cannot force
  signature checks or credential-store lookups on pathologically large input.
- **Machine-to-machine API keys** (auth example): hashed, scoped, revocable keys
  resolved to the same `Identity` as a user JWT. `POST /api-keys` (admin-only) mints a
  one-time `hephos_sk_…` key with roles; `DELETE /api-keys/{id}` revokes it. Only the
  SHA-256 hash is stored; the prefix lets the seam route a key to the store vs. JWT
  verification. New table `api_keys` (migration `0003`).
- **The auth seam is now async** (framework contract): `Router::authenticated` takes
  `Fn(String) -> impl Future<Output = Result<Identity>>`, because resolving an API key
  or checking a revocation list is I/O. A stateless JWT check just returns ready. The
  verify closure captures its dependencies (keyring, credential store).
- `Db::connect_lazy(url)`: build a pool without opening a connection (SQLx connects on
  first use) — non-blocking startup, and lets offline tests like the `routes_build`
  canary assemble app state without a live database.
- `TestClient::delete_with_token` for driving authenticated `DELETE` routes.
- **Argon2id parameters made explicit** (auth example): OWASP-2024 cost (19 MiB, 2
  iterations, 1 lane) instead of relying on the library default, with a note that the
  parameters live in the PHC hash so they can be raised without breaking old hashes.
- **Canonical password policy** (auth example): a single `validate_password` in the
  service (NIST 800-63B — length over composition) with a `MAX_PASSWORD_LEN` bound so
  a huge input can't turn hashing into a DoS. Unit-tested.
- **`Ctx::require_owner(owner)`**: the canonical resource-ownership (IDOR) guard —
  `401` anonymous, `403` when the resource belongs to someone else. Rounds out the
  authorization trio with `require_user` / `require_role`.
- **Canonical patterns documented once** in `CONVENTIONS.md`: the async auth seam and
  authorization guards (§2.6/§2.6a), and request validation — shape at the edge, rules
  in the service (§3.4).
- **Owner-only `/notes` resource** (auth example): a per-user CRUD resource that
  exercises `ctx.require_owner` end-to-end — the owner gets `200`, another user `403`,
  anonymous `401` (migration `0004`). Makes the resource-ownership pattern runnable, not
  just documented. Adds `TestClient::put_json_with_token`.

### Fixed (0.0.4 audit remediation)
- **Refresh-token double-spend race closed** — `RefreshTokenRepo::revoke` now reports
  whether it revoked a *live* row (atomic `where revoked_at is null` test-and-set); two
  concurrent rotations of the same token can no longer both succeed. The loser is
  treated as reuse and the token family is revoked, restoring single-use + theft
  detection under concurrency.
- **Empty/short `JWT_SECRET` rejected at startup** — HMAC accepts a zero-length key, so
  an exported-but-empty secret would have become a predictable, forgeable signing key.
  `JWT_SECRET`/`JWT_SECRET_PREVIOUS` must now be ≥ 32 bytes.
- **`login` bounds password length before hashing** — the `MAX_PASSWORD_LEN` policy is
  now enforced on login too (as `401`), so an unauthenticated caller can't force argon2
  work on a multi-megabyte body.
- **Auth-seam `Internal`→`500` now covered by an offline test** — the standard-stack
  HTTP suite asserts a resolver I/O failure surfaces as `500`, not a masked `401`.
- **`Db::connect_lazy`/`connect_with` share one pool-options builder** — the
  security-relevant statement-timeout install can no longer diverge between them.

### Fixed (audit remediation)
- **Unique-violation now maps to `409`, not `500`** — the DB-example repos catch a
  Postgres `23505` via the new `hephos::db::is_unique_violation` and return
  `Error::conflict` (the `find`-then-`insert` pre-check was racy under concurrency).
- **Agent `Limits.timeout` is now enforced** — the turn loop is wrapped in
  `tokio::time::timeout`; a hung model/tool call ends the run with an error.
- **Oversized bodies report `413`** — a new `Error::PayloadTooLarge`; the `Json`
  extractor maps a length-limit rejection to `413` instead of downgrading it to `400`.
- **Audit-task busy-loop fixed** — the example task now `break`s on `Closed` and only
  `continue`s on `Lagged` (previously spun the CPU on a closed channel).
- Bearer scheme parsed case-insensitively (RFC 7235); shutdown drains tasks against a
  single shared deadline (not N× per task); `statement_timeout` clamped to ≥1 ms (0
  disables it in Postgres); `TestDb` drops its database if setup fails; `split_server`
  rejects malformed `DATABASE_URL`; auth `exp` widened to `u64`; WS/SSE docs corrected
  to not overstate disconnect-cancellation.

### Added (0.0.3 — Database, for real)
- `Db::migrate(&Migrator)` runs embedded migrations (idempotent); the DB examples
  self-migrate in `AppState::init` via `sqlx::migrate!("./migrations")`.
- Env-driven `PoolConfig` (`HEPHOS_DB_MAX_CONNECTIONS`, `HEPHOS_DB_ACQUIRE_TIMEOUT_SECS`,
  `HEPHOS_DB_STATEMENT_TIMEOUT_SECS`); the server-side statement timeout is installed
  per connection so a stuck query can't pin the pool.
- `hephos::db::test::TestDb` — a disposable, migrated database per test, dropped on
  `cleanup` (no residue); plus the documented rolled-back-tx pattern.
- CI DB job applies schema via `cargo sqlx migrate run` and runs the DB-example flow
  tests with `--include-ignored`.

### Changed
- **Renamed the project `rivet` → `hephos`** (crates `hephos`, `hephos-core`,
  `hephos-db`, `hephos-agent`, `hephos-mcp`; env vars `HEPHOS_*`; all imports, docs,
  and examples). "Rivet" collided with existing projects; "Hephos" is distinct.

### Added (DX)
- `.cargo/config.toml` with `cargo ck`/`cargo lint` aliases and opt-in faster-linker
  guidance; `[profile.dev] debug = "line-tables-only"` for faster edit→check links.
- AGENTS.md §11 "Compiler feedback" protocol (check-before-test, scope commands) —
  agent iteration speed is `cargo check` speed.
- MILESTONES: the agent-turns-to-green-build benchmark as the headline adoption proof.

### Fixed (CI)
- `hephos-core` gains a feature-gated `From<sqlx::Error> for Error` (behind a `sqlx`
  feature that `hephos-db` enables), so the documented `sqlx::query!(...).await?`
  repository pattern compiles — this was the `database` CI job's `E0277` failure.
- `features` CI job drops `--locked` (incompatible with `cargo hack --no-dev-deps`,
  which rewrites `Cargo.toml`).
- `cargo-deny` CI job uses the official `EmbarkStudios/cargo-deny-action@v2`, which
  resolves workspace-inherited dependencies correctly (the bare CLI tripped on them).

## [0.0.2] — The real HTTP core

### Added
- **Standard middleware stack** (`hephos-core::stack`), installed by `App::run`:
  request-id + `x-request-id` response header, a per-request tracing span, request
  timeout (`408`, bounds response generation — streaming-safe), body-size limit
  (`413`), a seeded default `Identity`, and a per-request `CancellationToken` that is a
  child of the shutdown token.
- `Config` finalized with `HEPHOS_TIMEOUT_SECS` and `HEPHOS_BODY_LIMIT` (plus existing
  `HEPHOS_ADDR`/`HEPHOS_LOG`), fully unit-tested via a pure parser.
- Graceful-shutdown hardening: bounded task-drain deadline; background-task panics
  (`JoinError`) are logged, not swallowed.
- `TestClient` now wraps the real stack, with `with_stack(timeout, body_limit)`,
  `post_json_with_token`, and `TestResponse::{header, error_kind}`.
- `hephos-core` test suites: config, error, ctx, identity, event, router unit tests; an
  HTTP integration test; and a standard-stack acceptance test (request-id, `408`,
  `413`, identity seed).

### Fixed
- `Router::authenticated` now uses `route_layer` (not `layer`), so an unmatched path
  stays `404` instead of being turned into `401` by the merged fallback.
- Extractor rejections no longer leak raw deserializer text to clients; a fixed message
  is returned and the detail is logged.

### Changed
- `TimeoutLayer::new` → `TimeoutLayer::with_status_code` (upstream deprecation).

### Deferred
- Un-pinning the `Cargo.lock` version pins is blocked by the 1.85 build toolchain
  (latest transitive crates need rustc ≥1.87); MSRV stays 1.85 for now.

## [0.0.1] — Scaffold

### Status
Pre-alpha scaffold. The doctrine (`AGENTS.md`, `ARCHITECTURE.md`, `CONVENTIONS.md`),
the public API contract (`crates/`), and the reference examples are in place.

### Added
- Framework crates: `hephos-core` (App, Ctx, Error, Router, extractors, Config, Task,
  Events), `hephos-db`, `hephos-agent`, `hephos-mcp`, and the `hephos` meta crate.
- `hephos::test::TestClient` — first-class in-memory harness that drives the real
  router.
- Nine reference examples: rest, auth, database, websocket, streaming, agent, tools,
  workflow, mcp.
- Project tooling: GitHub Actions CI (offline + Postgres jobs + cargo-deny),
  `.gitignore`, dual `LICENSE-MIT`/`LICENSE-APACHE`, `rustfmt.toml`, `deny.toml`,
  `Makefile`, Dependabot, PR template, `CONTRIBUTING.md`.

### Fixed
- `Router` composition panicked at startup when nesting at `"/"`; added
  `Router::merge`, a guard on `Router::nest`, and a mandatory `routes_build` test.
- Error responses no longer repeat the error kind in the message.
- Auth login uses a guaranteed-valid decoy hash to close a timing side channel.
- Declared tokio's `net` feature explicitly; removed misleading dead code.

### Known boundaries (not yet implemented, at 0.0.1)
- Standard middleware stack composition — **implemented in 0.0.2**.
- `Db::migrate` runner; model provider HTTP clients; agent tool-call dispatch;
  rmcp server wiring.
