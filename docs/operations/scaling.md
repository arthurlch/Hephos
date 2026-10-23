# Postgres at Scale: The Everything-on-Postgres Playbook

Hephos makes an explicit architectural bet: **everything rides on PostgreSQL**.
Relational data, authentication credentials, revoked tokens, transactional outbox events, durable background jobs, workflow checkpoints, and vector embeddings all share one battle-tested database.

This document validates that bet under load, details operational tuning, and provides concrete thresholds for when specific concerns should be offloaded.

---

## 1. Why "Everything on Postgres"?

Distributed systems fail at the seams. Introducing separate operational infrastructure for queues (RabbitMQ/Kafka), caches (Redis), and state machines (Temporal) introduces:
- Dual-write failure modes (event emitted but database commit failed)
- Operational overhead (backup procedures, monitoring, security patching for multiple stateful clusters)
- Cognitive load and non-deterministic behavior during network partitions

With Postgres:
- **Transactional outbox** guarantees atomic event writes in the exact same transaction as domain entities.
- **`LISTEN/NOTIFY`** provides sub-millisecond job worker wakeups with zero polling latency under active load.
- **`FOR UPDATE SKIP LOCKED`** provides race-free, parallel job claiming across worker pools without lock contention.
- **Serializable and Repeatable Read isolation** protects workflow steps and saga compensations.

---

## 2. Operational Tuning for High Concurrency

### 2.1 Connection Pool Sizing
Never allow application pools to overwhelm Postgres with unbounded connections.
- **Application Pool (`HEPHOS_DB_MAX_CONNECTIONS`)**: Default to `16`–`20` connections per instance.
- **External Pooler (PgBouncer or Supavisor)**:
  - For deployments with > 10 application replicas, install PgBouncer in **transaction pooling** mode in front of Postgres.
  - Set `default_pool_size = 25` to `50` to match hardware core count (`2 × CPU cores + disk spindles`).

### 2.2 Timeouts & DoS Guards
A stuck query must never pin a connection or starve the pool:
- `HEPHOS_DB_ACQUIRE_TIMEOUT_SECS=5`: Rejects requests quickly if pool acquisition is saturated.
- `HEPHOS_DB_STATEMENT_TIMEOUT_SECS=15`: Server-side timeout enforced by Postgres on every connection.

### 2.3 Autovacuum Tuning for High-Churn Tables
Hephos features several high-churn tables where rows are frequently inserted, claimed, and deleted/updated (`hephos_jobs`, `refresh_tokens`, `hephos_workflows`):

```sql
-- Aggressive autovacuum for job queues to prevent table bloat:
ALTER TABLE hephos_jobs SET (
    autovacuum_vacuum_scale_factor = 0.05,
    autovacuum_vacuum_cost_limit = 1000,
    autovacuum_vacuum_cost_delay = 2
);

-- Aggressive autovacuum for workflow step checkpoints:
ALTER TABLE hephos_workflow_steps SET (
    autovacuum_vacuum_scale_factor = 0.05
);
```

---

## 3. Thresholds: When to Offload Concerns

The table below defines the empirical boundaries where a concern outgrows a single PostgreSQL cluster and should be migrated to dedicated infrastructure:

| Concern | Primary Postgres Capacity | Threshold for Offloading | Recommended Alternative |
| --- | --- | --- | --- |
| **Transactional Outbox / Events** | Up to 10,000 events/sec | > 10,000 events/sec sustained or multi-region fan-out | Apache Kafka, AWS Kinesis, Redpanda |
| **Durable Jobs (`Jobs`)** | Up to 5,000 jobs claimed/sec | > 5,000 jobs/sec or < 2ms latency SLA | NATS JetStream, Redis Streams |
| **Full-Text Search** | Up to 10M rows with GIN indexes | > 10M documents or advanced NLP/stemming | Meilisearch, Elasticsearch |
| **Vector Embeddings (pgvector)** | Up to 5M vectors (768–1536 dim) | > 5M vectors or vector index exceeds RAM | Qdrant, Milvus |
| **Rate Limiting (GCRA)** | Local per-instance up to 100k RPS | > 20 instances requiring strict cluster-wide rate limits | Redis (atomic Lua scripts) |
| **File / Object Storage** | Bytea up to 100 KB payloads | > 1 MB assets or high streaming bandwidth | S3, Cloudflare R2 |

---

## 4. Multi-Instance Rolling Restarts

Hephos applications are completely stateless in process:
1. When Kubernetes issues a `SIGTERM`, Hephos enters graceful drain:
   - Probes (`/health`, `/ready`) immediately return `503 Service Unavailable`, dropping the instance from load balancers.
   - In-flight HTTP requests have a 30-second grace window to complete.
   - Background tasks and job workers finish active turns before cancelling.
2. In-flight jobs whose worker abruptly dies are automatically reclaimed by survivor instances after the visibility timeout (`locked_at + 5 minutes`).
