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

## Status

This repository is the **design doctrine plus the canonical API surface and
examples**. The `crates/` directory defines the public API contract (types,
traits, signatures) with trivial parts implemented and non-trivial subsystems
marked with an explicit `// BOUNDARY:` comment describing what remains. The
`examples/` directory shows the one canonical way to use each capability.

Read [`REVIEW.md`](./REVIEW.md) for what to build first and what to deliberately
leave unbuilt.

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
