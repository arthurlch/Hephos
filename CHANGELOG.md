# Changelog

All notable changes to Rivet are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow SemVer.

## [Unreleased]

## [0.0.2] — The real HTTP core

### Added
- **Standard middleware stack** (`rivet-core::stack`), installed by `App::run`:
  request-id + `x-request-id` response header, a per-request tracing span, request
  timeout (`408`, bounds response generation — streaming-safe), body-size limit
  (`413`), a seeded default `Identity`, and a per-request `CancellationToken` that is a
  child of the shutdown token.
- `Config` finalized with `RIVET_TIMEOUT_SECS` and `RIVET_BODY_LIMIT` (plus existing
  `RIVET_ADDR`/`RIVET_LOG`), fully unit-tested via a pure parser.
- Graceful-shutdown hardening: bounded task-drain deadline; background-task panics
  (`JoinError`) are logged, not swallowed.
- `TestClient` now wraps the real stack, with `with_stack(timeout, body_limit)`,
  `post_json_with_token`, and `TestResponse::{header, error_kind}`.
- `rivet-core` test suites: config, error, ctx, identity, event, router unit tests; an
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
- Framework crates: `rivet-core` (App, Ctx, Error, Router, extractors, Config, Task,
  Events), `rivet-db`, `rivet-agent`, `rivet-mcp`, and the `rivet` meta crate.
- `rivet::test::TestClient` — first-class in-memory harness that drives the real
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
