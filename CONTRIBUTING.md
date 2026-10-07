# Contributing to Rivet

Rivet is built for coding agents, so the contribution rules are the same whether a
human or an agent writes the change: follow the doctrine, copy the nearest example,
and verify by running — not just compiling.

## Read these first

- [`CLAUDE.md`](./CLAUDE.md) — the operating manual. Actionable rules, the forbidden
  / preferred lists, and the validation commands. This is normative.
- [`CONVENTIONS.md`](./CONVENTIONS.md) — the canonical shape of every artifact.
- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — why the system is shaped the way it is.
- [`REVIEW.md`](./REVIEW.md) — build order and the invariants that must never break.

## The one rule

There is **one canonical way** to do each thing. Before writing, find the nearest
existing example of the same kind of code and mirror it. A new pattern is a
documented change to `CONVENTIONS.md` in the same PR — never a silent second style.

## Before you open a PR

Run the validation (CLAUDE.md §11), or `make ci` for the offline subset:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test                 # includes the routes_build canary + TestClient tests
cargo build --all-features
```

And — this is the lesson that shaped the testing doctrine — **compilation is not
correctness.** If your change touches a route, handler, extractor, middleware, or
anything the type checker cannot exercise, add a `rivet::test::TestClient` test for
it, or run the app and hit it. Do not infer success from a green build.

The PR template checklist encodes the rest.

## Dependencies

All dependencies live once in the root `[workspace.dependencies]`. Adding one needs
a one-line justification and a check that the standard stack (tokio, axum, serde,
tracing, sqlx, rmcp, thiserror/anyhow) genuinely cannot do the job.

## License

By contributing you agree your work is dual-licensed under
[MIT](./LICENSE-MIT), matching the project.
