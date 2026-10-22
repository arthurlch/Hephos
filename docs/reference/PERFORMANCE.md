# Hephos Performance & Latency Targets

Hephos provides an opinionated standard Tower middleware stack (request tracing, cancellation propagation, timeout enforcement, request size limits, panic isolation, and RED metrics) while keeping request overhead negligible.

---

## 1. Overhead vs. Raw Axum (Microbenchmarks)

Measured on an Apple Silicon M-series core using Criterion (`cargo bench -p hephos-core --bench overhead`):

| Pipeline Stage | Raw Axum | Hephos Standard Stack | Delta Overhead | % of Real-World Network RTT (10ms) |
| --- | --- | --- | --- | --- |
| **GET Request + JSON Serialize** | ~0.84 µs | ~5.69 µs | +4.85 µs | **0.048%** |
| **POST Request + JSON Deserialize & Return** | ~1.72 µs | ~5.82 µs | +4.10 µs | **0.041%** |

### Key Takeaway
The entire production-grade safety harness — spanning UUID request ID generation, panic isolation, request body bounding, global timeout tracking, and OTel RED metric counters — adds **less than 5 microseconds** of CPU overhead per request. In real-world network services where network latency is 5–50 ms, Hephos adds less than 0.1% overhead.

---

## 2. CPU Ceiling and Latency Design Budgets

The first row below is **measured** (CPU time in the microbenchmark above). The
throughput figure is the *inverse* of that CPU cost on a single core with **no network
and no database in the loop** — i.e. a CPU ceiling, **not** a sustained end-to-end
throughput measurement. Real throughput is dominated by network and database latency
and must be load-tested per deployment. The latency rows are **design budgets** backed
by a mechanism, not measured production percentiles.

| Metric | Budget | Status |
| --- | --- | --- |
| **In-Memory Request Processing Overhead** | < 10 µs | **measured ~5.7 µs** |
| **Single-Core CPU Ceiling** (1 / per-request CPU, no network/DB) | — | **~10⁵ req/s/core** (microbench ceiling, not sustained throughput) |
| **p50 Latency (REST CRUD + Postgres)** | < 3 ms | design budget — mechanism: pooled connections |
| **p95 Latency (REST CRUD + Postgres)** | < 10 ms | design budget — mechanism: statement timeouts |
| **p99 Latency (REST CRUD + Postgres)** | < 25 ms | design budget — mechanism: concurrency limits + load shedding |

> These latency budgets are targets the design is built to meet, not measured
> percentiles from a production workload. Validate them with a load test against your
> own deployment before quoting them.

---

## 3. Allocation and Clone Audit

- **`AppState` and handles**: Internally backed by `Arc`. Cloning `AppState` or `Ctx` on incoming requests clones only atomic reference pointers, with zero deep copies.
- **Buffer Recycling**: HTTP body extractors allocate only once during header decoding and stream payloads directly to Serde.
- **Agent Loops**: Shared `Tools` definitions are constructed once at startup and referenced via pointer borrowing during turn evaluations.
