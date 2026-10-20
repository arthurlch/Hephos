# example-workflow

A deterministic, durable workflow: a struct with `run(ctx, flow, input) -> Result<Output>`
whose body sequences service calls, approvals, and agent executions with Postgres-checkpointed
progress (DBOS-style).

```
src/
  domain/account.rs       NewAccount, Account, Onboarded.
  services/accounts.rs    AccountService::register.
  agents/model.rs         ScriptedModel (offline Model, as in example-agent).
  agents/welcome.rs       builds the welcome agent.
  workflows/onboarding.rs the durable workflow: register → compensation → agent welcome.
  tests.rs                unit & integration tests for crash-recovery, approvals, saga rollback.
```

Workflows in Hephos are ordinary Rust, read top to bottom:
- Each step is wrapped with `flow.step("step_name", || async { ... })`, committing output checkpoints to the database.
- Interrupted or killed workflows resume from their last committed step, skipping already-executed side effects.
- Sagas register compensations with `flow.register_compensation(...)` and execute them in reverse order (LIFO) on downstream failure.
- Human-in-the-loop decisions block with `flow.await_approval(...)` and support timeout bounds.
- Agents execute with turn durability via `flow.agent_checkpoint("welcome")` and `agent.run_durable(...)`.

Run: `cargo run -p example-workflow`
Test: `cargo test -p example-workflow`
