# API Freeze & SemVer Policy

As Hephos approaches `1.0.0`, this document formalizes the public API commitments, SemVer boundaries, deprecation cycle, and migration paths.

---

## 1. SemVer Guarantees

Starting with release `1.0.0`, all crates in the Hephos workspace (`hephos`, `hephos-core`, `hephos-db`, `hephos-agent`, `hephos-mcp`, `hephos-cli`) adhere strictly to [Semantic Versioning 2.0.0](https://semver.org/):

- **Major Releases (`X.0.0`)**: May introduce backward-incompatible changes only after a deprecation window of at least one minor release.
- **Minor Releases (`1.X.0`)**: Introduce backward-compatible new capabilities, optimizations, or non-breaking trait/struct extensions.
- **Patch Releases (`1.0.X`)**: Backward-compatible bug fixes and security patches.

---

## 2. Public API Surface Boundaries

The guaranteed public API consists of:

### 2.1 Covered by SemVer Stability
- **`hephos::prelude::*`**: The canonical import prelude containing `App`, `Ctx`, `Error`, `Result`, `Router`, `Json`, `Path`, `Query`, `StatusCode`, `Db`, `Tx`, `Jobs`, `Workflows`, `Agent`, `Tool`, `Model`.
- **`hephos_core` public types**: The lifecycle of `App`, `Ctx`, `Router`, `Error`, `Security` primitives (`Secret`, `SecretString`, `Secrets`), and Tower standard stack configuration (`Config`, `RateLimitConfig`).
- **`hephos_db` interfaces**: `Db`, `Tx`, `PoolConfig`, `Jobs`, `Workflows`, `Outbox`, and embedded migration helpers (`Db::migrate`).
- **`hephos_agent` core interfaces**: `Model`, `Tool`, `Tools`, `Agent`, `Limits`, `AgentCheckpoint`.
- **`hephos_mcp` interfaces**: `McpServer`, `McpClient`.

### 2.2 Explicitly Not Covered (Internal / Unstable)
- Private or internal implementation details not exported via `hephos` prelude or crate roots.
- Direct dependencies of upstream framework crates (`axum`, `tower`, `sqlx`) except where exposed as explicit public types.
- Generated SQL query macro internals.

---

## 3. Deprecation Policy

No public item will be removed or altered in a breaking manner without:
1. Being marked with `#[deprecated(since = "1.x.0", note = "use ... instead")]` for at least **one full minor release cycle**.
2. A clear compiler diagnostic and documented migration path in `CHANGELOG.md`.

---

## 4. Migration Guide: `0.x` to `1.0`

### 4.1 Secrets Management
In `0.8.0+`, raw sensitive strings (API keys, database passwords) must use `SecretString` or `Secret<T>` to prevent accidental logging.
```rust
// Before (0.7.x)
let key: String = std::env::var("API_KEY")?;

// After (0.8.0 - 1.0.0)
let key = SecretString::from(std::env::var("API_KEY")?);
```

### 4.2 Durable Workflows
Workflow checkpointing now uses `WorkflowContext::step` rather than ad-hoc persistence:
```rust
// In 1.0.0
let result: MyData = flow.step("step_name", || async {
    do_work().await
}).await?;
```

### 4.3 MCP Server Mounting
Streamable HTTP MCP services are mounted directly onto Hephos routers:
```rust
// In 1.0.0
let app_router = Router::new().merge(mcp_server.into_router("/mcp"));
```
