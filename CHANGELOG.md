# Changelog

All notable changes to Rivet are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow SemVer.

## [Unreleased]

### Status
Pre-alpha scaffold. The doctrine (`CLAUDE.md`, `ARCHITECTURE.md`, `CONVENTIONS.md`),
the public API contract (`crates/`), and the reference examples are in place. Core
runtime subsystems remain marked `// BOUNDARY:` — see `REVIEW.md` for what to build
first toward a functional `0.1.0`.

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

### Known boundaries (not yet implemented)
- Standard middleware stack composition (request id, trace, timeout, body limit,
  cancellation/identity seed).
- `Db::migrate` runner; model provider HTTP clients; agent tool-call dispatch;
  rmcp server wiring.
