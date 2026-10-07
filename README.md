# Rivet

> **The backend framework built only for coding agents.**

Rivet is a Rust backend framework whose architecture, naming, and conventions are
designed around how autonomous coding agents (Claude Code, Codex, Gemini CLI, and
peers) read repositories, place new code, and extend applications.

The governing principle:

> **If a coding agent can understand the framework, it can build the application.**

Rivet is built **above** proven Rust infrastructure — Tokio, Tower, Hyper, Axum,
Serde, tracing, SQLx, rmcp — and adds a single, opinionated, predictable
application layer on top. It does not reimplement a runtime, an HTTP stack, or a
serialization format. 

## Documents

| File | Purpose |
| --- | --- |
| [`CLAUDE.md`](./CLAUDE.md) | Operating manual for coding agents working in this repo. Actionable rules. |
| [`ARCHITECTURE.md`](./ARCHITECTURE.md) | Why the system is shaped the way it is. |
| [`CONVENTIONS.md`](./CONVENTIONS.md) | The single canonical way to write every kind of code. |
| [`REVIEW.md`](./REVIEW.md) | Final architecture review and build order. |

## Status — pre-alpha scaffold

This repository is the **design doctrine plus the canonical API surface, reference
examples, and project tooling**. The `crates/` directory defines the public API
contract (types, traits, signatures) with trivial parts implemented and non-trivial
subsystems marked with an explicit `// BOUNDARY:` comment describing what remains.
The `examples/` directory shows the one canonical way to use each capability.

What works end to end today: the REST path (routing, extraction, the error model,
services) — launched, served, and tested over HTTP. What is still `// BOUNDARY:`:
the standard middleware stack, DB migration runner, model provider clients, agent
tool dispatch, and MCP server wiring. So this is **not yet a functional `0.1.0`** —
it is a consolidated base to build one from. See [`REVIEW.md`](./REVIEW.md) for the
build order and the testing retrospective.

## Developing

```sh
make ci            # fmt-check + clippy -D warnings + test (offline subset)
make run-rest      # launch the REST example
cargo test         # includes the routes_build canary + TestClient HTTP tests
```

CI (`.github/workflows/ci.yml`) runs the offline baseline, a Postgres job for the
database/auth examples, and `cargo-deny`. See [`CONTRIBUTING.md`](./CONTRIBUTING.md).

## Layout

```
crates/
  rivet/        meta crate: prelude + feature-gated re-exports
  rivet-core/   App, Ctx, Error, Router, extractors, config, tasks, events
  rivet-db/     Db, Tx — SQLx/Postgres helpers
  rivet-agent/  Model, Agent, Tool, structured output, streaming
  rivet-mcp/    rmcp integration
examples/
  rest/ auth/ database/ websocket/ streaming/ agent/ tools/ workflow/ mcp/
```
