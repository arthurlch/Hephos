# example-tools

The `Tool` contract: strongly typed input (with a derived JSON Schema), strongly
typed output, validation, authorization, and error handling — all in the one
`Error` type. The same tool is callable by an `Agent` (see `example-agent`) and
exposable over MCP (see `example-mcp`) with no second implementation.

```
src/
  domain/user.rs        UserSummary (output), SearchQuery (input + JsonSchema).
  services/directory.rs immutable user directory the tool searches.
  tools/search_users.rs the Tool: captures its deps + the caller identity.
```

Authorization is done inside `call`: the tool captures the caller's `Identity`
when constructed and requires the `support` role. Validation (non-empty query) is
also inside `call`. Both surface `rivet::Error`.

Run: `cargo run -p example-tools`
