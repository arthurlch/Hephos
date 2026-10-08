# example-workflow

A deterministic workflow: a struct with `run(ctx, input) -> Result<Output>` whose
body sequences a service call and an agent call, with explicit state, failure
handling, and a cancellation checkpoint between steps.

```
src/
  domain/account.rs      NewAccount, Account, Onboarded.
  services/accounts.rs   AccountService::register.
  model.rs               ScriptedModel (offline Model, as in example-agent).
  agents/welcome.rs      builds the welcome agent.
  workflows/onboarding.rs the workflow: register → checkpoint → welcome.
```

Hephos ships no workflow engine. A workflow is ordinary Rust, read top to bottom:
each step is a private method, failures propagate with `?`, and cancellation is
inherited from `ctx`. Durable/replayable execution is a future, additive layer
(see `MILESTONES.md`) reachable without changing this shape.

Run: `cargo run -p example-workflow`
