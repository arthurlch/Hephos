# 12-Factor Deployment & Zero-Downtime Operations

This guide provides operational standards for deploying Hephos services in cloud-native production environments (Docker, Kubernetes).

---

## 1. The Twelve Factors Applied to Hephos

1. **Codebase**: One codebase tracked in revision control, many deploys.
2. **Dependencies**: Explicitly declared in workspace `Cargo.toml` and verified by `cargo deny`.
3. **Config**: Strict environment-variable-first configuration (`HEPHOS_*`, `DATABASE_URL`, `JWT_SECRET`).
4. **Backing Services**: Databases, caches, and model providers treated as attached resources.
5. **Build, Release, Run**: Strict separation of build stage (multi-stage Dockerfile) from execution.
6. **Processes**: Execute the app as one or more stateless processes. Process crashes restart safely without data loss.
7. **Port Binding**: Export HTTP services via port binding (default `HEPHOS_ADDR=0.0.0.0:8080`).
8. **Concurrency**: Scale horizontally by adding process instances behind a reverse proxy or Kubernetes service.
9. **Disposability**: Fast startup (< 50ms) and graceful shutdown (`SIGTERM` triggers connection draining and task cancellation).
10. **Dev/Prod Parity**: Identical Tower stack and Postgres database substrate in tests and production.
11. **Logs**: Treat logs as event streams emitted to stdout in JSON format (`HEPHOS_LOG_FORMAT=json`).
12. **Admin Processes**: Run database migrations via embedded `Db::migrate` or `cargo sqlx migrate run` as one-off tasks.

---

## 2. Zero-Downtime Rolling Restarts

When deploying an update in Kubernetes:

```
[Load Balancer]
      |
      +-----> Pod 1 (Old: Serving traffic)
      |
      +-----> Pod 2 (New: Running /ready probe)
```

1. **Readiness Gating**: Kubernetes will not route traffic to Pod 2 until its `/ready` probe returns `200 OK`.
2. **Graceful Drain**: When Pod 1 receives `SIGTERM`:
   - `/ready` immediately returns `503`, causing endpoints to remove it from the active service endpoints list.
   - Hephos allows existing in-flight connections up to `terminationGracePeriodSeconds` (30s) to complete.
   - Long-running background loops exit cleanly when `ctx.cancel_token().cancelled()` fires.
3. **No Dropped Requests**: The PodDisruptionBudget (`deploy/k8s/pdb.yaml`) ensures `minAvailable: 1`, preventing cluster maintenance from evicting all instances simultaneously.
