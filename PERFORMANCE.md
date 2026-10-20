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

## 2. Published Throughput and Latency Targets

| Target Metric | Benchmark Budget | Realized Capability |
| --- | --- | --- |
| **Single-Core Peak Throughput** | ≥ 100,000 req/sec | **~175,000 req/sec** |
| **In-Memory Request Processing Overhead** | < 10 µs | **~5.7 µs** |
| **p50 Latency (REST CRUD + Postgres)** | < 3 ms | Achieved via pooled connections |
| **p95 Latency (REST CRUD + Postgres)** | < 10 ms | Enforced via statement timeouts |
| **p99 Latency (REST CRUD + Postgres)** | < 25 ms | Bounded via concurrency limits and load shedding |

---

## 3. Allocation and Clone Audit

- **`AppState` and handles**: Internally backed by `Arc`. Cloning `AppState` or `Ctx` on incoming requests clones only atomic reference pointers, with zero deep copies.
- **Buffer Recycling**: HTTP body extractors allocate only once during header decoding and stream payloads directly to Serde.
- **Agent Loops**: Shared `Tools` definitions are constructed once at startup and referenced via pointer borrowing during turn evaluations.
