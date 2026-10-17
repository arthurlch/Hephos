# Changelog

All notable changes to Hephos are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow SemVer.

## [Unreleased]

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
  `204` handler shape (CLAUDE.md §4) works without importing `axum`.
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
- CLAUDE.md §11 "Compiler feedback" protocol (check-before-test, scope commands) —
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
Pre-alpha scaffold. The doctrine (`CLAUDE.md`, `ARCHITECTURE.md`, `CONVENTIONS.md`),
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
