# example-mcp

Expose a Hephos `Tool` over the Model Context Protocol using `rmcp`, the official
Rust MCP SDK. The tool here is the **same** `SearchUsers` shape as `example-tools`
— one tool definition, reachable by an in-process `Agent` and by any external MCP
client, with no second implementation.

```
src/
  domain/user.rs        UserSummary, SearchQuery.
  services/directory.rs the immutable directory the tool searches.
  tools/search_users.rs the Tool (identical shape to example-tools).
  mcp/server.rs         McpServer wiring: register the tool, serve over stdio.
```

Hephos does not define a protocol. `hephos-mcp` wraps rmcp: it registers each Hephos
`Tool` as an MCP tool whose input schema is the tool's derived `schemars` schema,
and serves over stdio (local) or Streamable HTTP (networked — the current MCP
transport that replaced HTTP+SSE). External MCP servers can also be consumed as
typed in-process agent tools via `McpClient`.

Run (as a local MCP server over stdio): `cargo run -p example-mcp`
Run (as a networked Streamable HTTP server): `HEPHOS_MCP_HTTP=1 cargo run -p example-mcp`
Inspect OpenAPI 3.1.0 schema: `cargo run -p hephos-cli -- openapi --path examples/mcp`
