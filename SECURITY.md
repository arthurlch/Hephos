# Security Policy & Threat Model

This document outlines the security architecture, published threat model, secrets handling,
audit logging, and vulnerability reporting process for Hephos.

---

## 1. Security Architecture & Invariants

Hephos enforces defense-in-depth through architectural constraints that are statically or
dynamically guaranteed:

1. **`#![forbid(unsafe_code)]`**: Set crate-wide across all internal crates (`hephos`, `hephos-core`,
   `hephos-db`, `hephos-agent`, `hephos-mcp`, `hephos-cli`).
2. **One Fixed Tower Stack**: Ordering bugs in middleware are eliminated by assembly in `hephos_core::stack`.
   Every request lifecycle enforces request IDs, tracing spans, timeouts, body limits (2 MiB default),
   cancellation linkage, and panic catching.
3. **Layer Separation (CI Enforced)**: Handlers never run SQL; services never touch HTTP types;
   repos never make business decisions. Checked by `make arch-lint`.
4. **Opaque Error Reporting**: Internal failures (`sqlx::Error`, I/O errors) are converted to `Error::Internal`,
   logged server-side with full details and request IDs, and returned to clients as opaque `500` status codes.
   Database schema details, connection strings, or system paths are never leaked to callers.
5. **Compile-Checked SQL**: All SQL statements use `sqlx::query!` or `sqlx::query_as!` ensuring parameters
   are bound and types match the database schema. String concatenation in SQL queries is forbidden.
6. **Secrets Redaction & Zeroization**: All credentials (API keys, tokens, passwords) are wrapped in `Secret<T>` /
   `SecretString`, redacting them from `Debug`, `Display`, and serialization output, and **zeroizing the underlying
   memory on drop** (via `zeroize`) so plaintext does not linger in freed heap, swap, or a core dump.
7. **Identity Attribution & Audit Trail**: Security-sensitive actions record an `AuditEvent` with actor identity,
   action name, resource ID, outcome, and timestamp.
8. **Strict Input & Path Validation in Tooling**: Scaffolding and code generation in `hephos-cli` strictly
   validate all identifiers against ASCII alphanumeric rules and Rust reserved keywords, preventing path traversal,
   code injection, and SQL injection in generated migrations and resource files.
9. **Streaming Boundary & Memory Caps**: Agent SSE streaming parsers buffer chunks across multi-byte UTF-8
   boundaries and enforce hard capacity limits (2 MiB) to prevent memory exhaustion DoS attacks from rogue
   or compromised upstream model APIs.

---

## 2. Threat Model

### 2.1 Threat Actors

| Actor | Description | Capabilities |
| --- | --- | --- |
| **External Unauthenticated** | An anonymous caller on the public internet. | Can issue arbitrary HTTP/WebSocket requests, craft malformed payloads, attempt DoS/brute force. |
| **Authenticated User** | A caller with valid credentials but standard privileges. | Can access authenticated endpoints, attempt privilege escalation, or attempt IDOR against other tenants. |
| **Malicious / Compromised Admin** | A privileged caller or compromised operator. | Can invoke admin routes, trigger data changes, or attempt stealthy modification. |
| **Compromised LLM / Agent Output** | A model producing hallucinated, malicious, or prompt-injected tool calls. | Can request unexpected tool names, out-of-range arguments, or recursive execution. |
| **Supply Chain / Malicious Dep** | A compromised or vulnerable third-party crate. | Attempts code execution or data exfiltration at build or runtime. |

### 2.2 Assets

- **Secrets**: API keys (Anthropic, OpenAI), database credentials, JWT signing keys.
- **Tenant Data**: Domain database records, account data, private user content.
- **Agent Tools & Capabilities**: Outbound capabilities (sending emails, modifying accounts).
- **Audit Logs**: Immutable records of security actions.
- **Service Availability**: Compute, database connections, and memory.

### 2.3 Threat Matrix & Mitigations

| Threat | Impact | Hephos Mitigation | Verification Test |
| --- | --- | --- | --- |
| **SQL Injection** | Data breach, DB takeover | Parameterized queries via `sqlx::query!`/`query_as!`. String formatting in SQL is strictly forbidden. | Unit + integration DB tests |
| **Credential Leakage in Logs** | API key compromise | `SecretString` wrapper redacting `Debug`, `Display`, and JSON serialization. Model providers use `SecretString`. | `tests/security.rs`, `providers_api_key_redaction_in_debug` |
| **IDOR (Horizontal Privilege Escalation)** | Unauthorized access to another user's resource | Mandatory `ctx.require_owner(row.owner_id)?` check in service layer after loading records. | `examples/auth`, `tests/http.rs` |
| **Denial of Service (DoS) via Oversized Body** | Memory exhaustion / crash | `RequestBodyLimitLayer` in fixed Tower stack (default 2 MiB) returning `413 Payload Too Large`. | `tests/stack.rs:oversized_body_is_rejected` |
| **DoS via Oversized Auth Headers** | CPU exhaustion in crypto/parsing | Strict `MAX_TOKEN_LEN = 4096` cap on bearer headers and query tokens, rejecting before verification. | `tests/fuzz_extractors.rs:fuzz_auth_tokens_and_enforce_size_caps` |
| **Brute Force & Credential Stuffing** | Account compromise | `RateLimitConfig::sensitive()` (5 RPS, burst 10) on sensitive authentication endpoints via `tower_governor`. | `tests/rate_limit.rs`, `tests/security.rs` |
| **Slowloris / Hanging Handlers** | Connection pool starvation | `TimeoutLayer` (default 30s) bounding response generation time. | `tests/stack.rs:slow_handler_is_cut_by_the_timeout` |
| **Connection Flooding** | File descriptor exhaustion | `ConnectionCap` rejecting excess concurrent connections with `503 Service Unavailable`. | `tests/conn.rs`, `tests/load_shed.rs` |
| **DNS Rebinding in MCP HTTP Transport** | Local network access from web | Streamable HTTP endpoints mountable behind `into_protected_router` with bearer token auth. | `hephos-mcp::tests::into_protected_router_mounts_auth_guarded_endpoint` |
| **Tool Execution Beyond Boundary** | Unbounded model actions | `Limits` enforce `max_turns`, `max_tokens`, `timeout`, and `max_tool_calls_per_turn`. Tools validate input schemas via `schemars`. | `hephos-agent::tests::agent` |
| **Path Traversal / Injection in Scaffolding** | Overwrite host files / inject SQL | Strict resource and migration name validation rejecting path delimiters (`/`, `\`, `..`), null bytes, invalid identifiers, and SQL injection syntax in `hephos-cli`. | `hephos-cli::commands::generate::tests` |
| **Stream Memory DoS in Agent SSE** | Memory exhaustion via endless stream | SSE lines parser enforces a 2 MiB buffer ceiling (`MAX_SSE_BUFFER_BYTES`) and handles chunk-split UTF-8 graphemes safely. | `hephos-agent::providers::common::tests` |
| **Secret File Path Injection** | Arbitrary file read via null bytes | `FileSecretProvider` rejects paths containing null characters (`\0`). | `hephos-core::security::secret::tests` |
| **CORS Misconfiguration** | Client correlation failure or leaked headers | Tower CORS layer securely exposes `x-request-id` and permits standard telemetry headers (`traceparent`, `tracestate`, `last-event-id`). | `hephos-core::stack::tests::cors_preflight_allows_listed_origin` |
| **Supply Chain Vulnerability** | Remote code execution / backdoors | `cargo-deny` blocking in CI on advisories, banned crates, licenses, and unknown registries. | CI job `deny` |

---

## 3. Secrets Handling

All secrets must be handled through `hephos_core::security`:

```rust
use hephos::prelude::*;

// Wrap sensitive values
let key = SecretString::from("sensitive-token");
assert_eq!(format!("{key:?}"), "[REDACTED]");

// Retrieve via pluggable provider (file/Docker secrets first, env fallback)
let secrets = Secrets::standard();
let db_pass = secrets.require("DB_PASSWORD")?;
```

In model clients, API keys are held as `SecretString`, ensuring they are never logged even if
the provider client is debug-printed or traced.

---

## 4. Audit Logging

Applications emit structured audit events for compliance, security monitoring, and forensic review:

```rust
ctx.audit(
    "account.transfer",
    "account",
    Some(account_id.to_string()),
    AuditOutcome::Success,
    serde_json::json!({ "amount": 100 }),
);
```

By default, audit events are logged to the `hephos::audit` tracing target at `INFO` level, containing
structured attributes (`audit_id`, `audit_timestamp_secs`, `audit_request_id`, `audit_actor_type`,
`audit_actor_id`, `audit_action`, `audit_resource_type`, `audit_resource_id`, `audit_outcome`).
Custom audit sinks can be registered via `hephos_core::security::set_audit_logger`.

---

## 5. Operational Security & Deployment Best Practices

### 5.1 Reverse Proxies & IP Spoofing Protection
Hephos utilizes `tower_governor::key_extractor::SmartIpKeyExtractor` for per-client IP rate limiting. When deployed behind a reverse proxy (e.g., NGINX, Envoy, AWS ALB, Cloudflare):
- Ensure the edge proxy strips untrusted client-supplied `X-Forwarded-For` and `Forwarded` headers before appending the true client IP.
- Failure to sanitize inbound proxy headers allows untrusted clients to cycle spoofed IP addresses, evading rate limits.

### 5.2 Transport Layer Security (TLS) & HSTS
- Always serve over TLS in production environments or terminate TLS at a secure edge gateway.
- Enable `HEPHOS_HSTS=true` (or set the header at your ingress) to enforce Strict-Transport-Security (1 year duration, including subdomains).

### 5.3 Secret Ingestion & Principle of Least Privilege
- Prefer filesystem secrets mounted into `/run/secrets/` or Docker secrets (`FileSecretProvider`) over environment variables (`EnvSecretProvider`) to prevent accidental leaks in environment dumps or child process tables.
- Restrict file permissions on secrets to `0400` or `0600` readable only by the application service user.

---

## 6. Reporting a Vulnerability

We take the security of Hephos seriously. If you discover a security vulnerability in Hephos:

1. **Do not open a public issue.**
2. Send an email to `security@hephos.dev` (or create a private GitHub Security Advisory).
3. Include detailed reproduction steps, proof of concept, and affected versions.
4. We acknowledge reports within 48 hours and provide regular progress updates toward a patch.
5. Coordinated disclosure occurs after a patch and advisory have been released.
