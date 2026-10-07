# REVIEW.md — Final architecture review

This closes the design. It records the consistency review, the build order, and the
answers to the eight questions the brief asks.

---

## Consistency review (performed)

Checked, and holding:

- **Every example follows `CONVENTIONS.md`.** Same `main.rs` → `state.rs` →
  `domain`/`services`/`repos`/`api` shape; same `type Ctx` alias; same handler
  signature (`ctx: Ctx` first, `Json<T>` in, `Result<Json<T>>` out).
- **One error type.** No example defines an error enum. Every fallible function
  returns `rivet::Result<T>`. Not-found is constructed at the service boundary;
  infrastructure errors propagate to `Internal` via `?`.
- **State is uniform.** One `AppState`, `#[derive(Clone)]`, cheap handles. No
  `Arc<Mutex<_>>` anywhere. Immutable shared data uses `Arc<[T]>`; mutable state
  lives in the database.
- **Async is uniform.** Background work is a `Task` tied to the shutdown token;
  every loop selects against `ctx.cancel_token()`. The one `tokio::spawn` (the
  WebSocket writer) is joined before return, not detached.
- **Services never import HTTP; handlers never touch repos.** Verified across
  `rest`, `database`, `auth`.
- **Tools are defined once.** `SearchUsers` is identical in shape in `tools`,
  `agent`, and `mcp` — callable by an agent and exposable over MCP with no second
  implementation.
- **`CLAUDE.md`, `ARCHITECTURE.md`, `CONVENTIONS.md` agree** on placement,
  layering, errors, async, and dependencies. No contradictions found; the two
  simulations in `ARCHITECTURE.md §24` resolve to a single implementation each.

### Verification status (actually run)

- `cargo check` / `cargo clippy -D warnings` pass for **all five framework crates**
  and the seven offline examples (`rest`, `websocket`, `streaming`, `agent`,
  `tools`, `workflow`, `mcp`). `cargo fmt` applied.
- **Runtime-tested, not just compiled**: `rest`, `streaming`, and `websocket` were
  launched and hit over HTTP — correct status codes, error bodies, and SSE output.
  `agent`, `tools`, and `workflow` binaries were run end to end.
- **`cargo test` drives the real router** via `rivet::test::TestClient`: `rest`
  has success/404/400 tests, all green; every HTTP app has a `routes_build` canary.
  That canary was confirmed to *fail* when `nest("/")` is reintroduced — the
  regression is proven caught.
- `database` and `auth` type-check up to SQLx's **compile-time query verification**,
  which requires a live Postgres (or a committed `.sqlx` cache). Their HTTP-surface
  tests are written and `#[ignore]`d until a database and wired migrations exist.
  This is expected and documented; it is not an API error.

> Note on the lockfile: the environment toolchain is 1.85.1, while several *latest*
> transitive crates now require 1.87–1.89. `Cargo.lock` pins a few of them
> (`uuid`, `time`, `url`/`idna`, `simple_asn1`) to 1.85-compatible releases. With a
> current stable toolchain these pins are unnecessary.

### Honest boundaries (designed, not yet implemented)

Marked `// BOUNDARY:` in `crates/`, never in app code:

- The standard middleware stack composition (`rivet-core::app::standard_stack`).
- `Db::migrate` call site (must invoke `sqlx::migrate!` from the app crate).
- Provider HTTP clients and vendor tool-call encoding (`rivet-agent::providers`,
  `agent::drive`'s `parse_tool_calls`).
- rmcp server wiring (`rivet-mcp`).

Each boundary has a fixed contract and a precise description of the remaining work.
None is a fake API: every signature is one a correct implementation satisfies.

---

## Testing retrospective — a bug the first pass shipped

**What happened.** The first revision composed routers with `nest("/")`. That
compiles, lints clean, and type-checks — but axum panics on it at startup
("Nesting at the root is no longer supported"). Every HTTP example would have
crashed on launch. It was certified "done" because verification stopped at
`cargo check` / `cargo build` / `cargo clippy`, plus unit tests that only touched
the *service* layer. A stray `timeout cargo run` was killed during compilation and
misread as "it ran."

**Root cause.** Two gaps, one structural:
1. **Compilation was mistaken for correctness.** None of `check`/`build`/`clippy`
   runs a request, so none can see a routing/middleware/wiring fault.
2. **There was no canonical way to test an app's HTTP surface**, so no example
   did — the entire wiring layer was untested by construction.

**Measures taken (this revision):**
1. **Framework guard.** `Router::nest` now rejects `"/"` early with a Rivet-level
   message that names the fix (`merge`), turning a cryptic deep panic into an
   obvious one.
2. **A first-class test harness.** `rivet::test::TestClient` drives the real
   router in memory. Shipped with the framework so testing the HTTP surface is the
   one easy, canonical thing.
3. **Mandatory `routes_build` canary** in every app — the one-line test that
   catches this exact class. Verified: it fails when `nest("/")` is reintroduced.
4. **Doctrine updated.** `CLAUDE.md §10–11`, `CONVENTIONS.md §5`, and
   `ARCHITECTURE.md §21` now state plainly that `cargo check ≠ correct`, require
   router-driving tests per endpoint, and require launching-or-testing before any
   "done" claim.

**The generalized lesson.** For an agent-first framework the verification bar is
*"observed to run,"* not *"compiled."* Prefer defects the compiler catches; for the
ones it cannot (runtime panics, wiring), provide a canonical, mandatory test so the
check is uniform and unskippable. "It type-checks" is never a completion claim.

**Remaining follow-up.** The `routes_build` + `TestClient` pattern is applied to
all HTTP examples; `database`/`auth` endpoint tests are written but `#[ignore]`d
pending a database and wired migrations (`Db::migrate` boundary).

---

## Is this a consolidated 0.0.1?

Two different questions, two different answers.

**As a repository baseline: yes.** It now carries what a serious project needs on
day one — `.gitignore` (and `target/`, mistakenly committed in the init commit, has
been untracked), dual `LICENSE-MIT`/`LICENSE-APACHE` matching the manifest,
`rustfmt.toml`, GitHub Actions CI (offline + Postgres + cargo-deny), Dependabot, a
PR template that encodes the invariants, `deny.toml`, a `Makefile`, `CONTRIBUTING.md`,
and a `CHANGELOG.md`. The offline CI job is verified green by running its exact
commands locally.

**As a functional `0.1.0` of the framework: not yet, and the version should say so.**
Core runtime pieces are still `// BOUNDARY:` — the middleware stack (so `Ctx`
request-id, timeout, and client-disconnect cancellation are not actually wired),
`Db::migrate`, the provider HTTP clients, agent tool dispatch, and rmcp serving. The
REST path is real; the agent/db/mcp paths are partial. Tag this **`0.0.1` / pre-alpha**,
not a working release.

The single most valuable next step toward a real `0.1.0` is implementing the
standard middleware stack (§6 build-first item): it is the one boundary the docs
already describe as working, and wiring it makes `Ctx` honest. Do that behind the
test harness that now exists.

---

## 1. Why this is optimized for coding agents

- **One canonical way.** Placement, naming, error type, handler shape, state,
  async — each has exactly one form. An agent pattern-matches and reproduces it.
- **Placement is mechanical.** Responsibility → directory is a lookup table
  (`CLAUDE.md §1`). "Where does this go?" never requires judgment.
- **The compiler is the feedback loop agents exploit best.** One error type,
  strong types everywhere, compile-time-checked SQL, derived tool schemas: whole
  classes of mistakes become compile errors, not runtime surprises.
- **Small surface.** A handful of types (`App`, `Ctx`, `Error`, `Router`, `Json`,
  `Tool`, `Agent`) cover everything. The whole framework fits in an agent's head.
- **The repository is self-describing.** The examples *are* the spec. An agent
  reads the nearest example and copies it.

## 2. What makes it different from Axum / Actix / Rocket

Those are general-purpose frameworks optimized for human flexibility: many
extractors, several response conventions, multiple routing and error styles.
Rivet is the opposite trade — it *removes* choices. It wraps Axum to expose **one**
extractor pattern, **one** response convention, **one** router style, and **one**
error type, and it fixes the middleware order so an app never assembles a stack.
The difference is not features; it is the deliberate elimination of valid
alternatives so that generated code is uniform.

## 3. What to build first

1. **`rivet-core` for real** — finish the standard middleware stack (request id,
   trace, timeout, body limit, cancellation + identity seed). Everything depends
   on this.
2. **`rivet-db`** — wire `migrate!`, ship the repo pattern, add transaction tests.
3. **REST + auth + database end to end** — these three examples are the templates
   99% of generated code will copy. Make them run and test against a real Postgres.
4. **`rivet-agent` core** — the turn loop plus **one** provider (Anthropic),
   `run`/`run_typed`/`stream`.
5. **`rivet-mcp`** — expose tools over rmcp stdio, then Streamable HTTP.

## 4. What NOT to build yet

- A second database backend (stay Postgres).
- A durable workflow engine (event sourcing, sagas, replay).
- A cron/scheduler subsystem (the interval-loop `Task` suffices).
- An external event broker or an outbox (keep the in-process broadcast bus).
- RPC code generation / a schema language (the `POST /rpc/{method}` convention
  suffices).
- A plugin system, a DI container, or config-struct machinery.
- More than one or two model providers.

Each is a stateful subsystem with its own failure modes and its own second mental
model. Adding one before it is genuinely needed trades away the coherence that is
the entire point.

## 5. Which Rust ecosystem components to reuse

Tokio (runtime), Hyper + Axum (HTTP/routing), Tower + tower-http (middleware),
Serde (serialization), tracing + tracing-subscriber (observability), SQLx
(compile-checked Postgres), rmcp (MCP), thiserror + anyhow (error ergonomics),
schemars (tool/output schemas), jsonwebtoken + argon2 (auth), uuid, time. Rivet
adds the application layer **above** these and reimplements none of them.

## 6. Where this could become over-engineered

- **The agent/tool/workflow/MCP surface** is the largest risk. If the turn loop
  grows planner/critic/memory subsystems, or tools gain a registry and lifecycle,
  the "small surface" promise breaks. Keep the agent a model + tools + limits.
- **The event bus** must not grow into a message queue. The moment it needs
  durability, that is an outbox `Task` writing to the database — a new, explicit
  pattern, not a heavier bus.
- **`Router::authenticated` and the middleware seam** should not become a general
  middleware framework. One auth seam, one fixed stack.
- **Feature flags** could proliferate. Keep them to `db`, `agent`, `mcp`.

The guiding check: if a capability adds a *second way* to do an existing thing, or
a second mental model, it is probably over-engineering for this framework.

## 7. The invariants that must never be broken

1. **One error type** (`rivet::Error`) across every layer.
2. **`ctx: Ctx` first** on every handler/task/tool entry; state only via
   `ctx.state()`.
3. **Strict layering**: `api → services → repos → db`; handlers never touch SQL,
   services never import HTTP.
4. **No `unsafe`** (`#![forbid(unsafe_code)]`), ever.
5. **No `Arc<Mutex<_>>` for app state; no global mutable state.**
6. **Parameterized SQL only**, verified at compile time.
7. **Bounded everything**: timeouts, body limits, bounded channels, agent limits;
   structured concurrency owned by the app lifecycle.
8. **One canonical way per task.** A new pattern is a documented change to
   `CONVENTIONS.md`, never a silent second style.

Break any of these and the property that makes Rivet work for agents — uniformity —
degrades.

## 8. How future agents should extend the framework

- **Adding a feature to an app:** find the responsibility, open that directory,
  copy the nearest example, follow `CONVENTIONS.md`. Run the §11 validation
  commands. Done.
- **Adding a capability to the framework:** add a new crate behind a feature flag
  (never a runtime plugin), define its one canonical shape, add that shape to
  `CONVENTIONS.md` and a matching `examples/` entry **in the same change**, and
  confirm no document now contradicts the code.
- **Filling a boundary:** implement against the fixed contract at the
  `// BOUNDARY:` marker; do not change the public signature. If the signature must
  change, update `ARCHITECTURE.md` and every example that uses it, together.

The repository is designed so that the correct extension is the easiest one: the
pattern to copy is always already present.
