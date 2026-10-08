# example-agent

An `Agent`: a `Model`, a system prompt, and execution `Limits`. Shows the three
run shapes — `run` (text), `run_typed` (structured output), and `stream` (text
deltas) — and how limits bound every run.

To keep the example runnable offline and deterministic, it uses a `ScriptedModel`
that implements the `Model` trait with canned responses. **In production** you
would swap it for `hephos::agent::providers::Anthropic::from_env(...)` — nothing
else about the agent changes, which is the point of the `Model` abstraction.

```
src/
  model.rs           ScriptedModel: a Model implementation for offline runs.
  domain/report.rs   Report: the structured-output type (JsonSchema).
  agents/report.rs   builds the Agent with system prompt + limits.
```

Run: `cargo run -p example-agent`
