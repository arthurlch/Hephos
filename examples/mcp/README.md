# example-mcp

Expose a Rivet `Tool` over the Model Context Protocol using `rmcp`, the official
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

Rivet does not define a protocol. `rivet-mcp` wraps rmcp: it registers each Rivet
`Tool` as an MCP tool whose input schema is the tool's derived `schemars` schema,
and serves over stdio (local) or Streamable HTTP (networked — the current MCP
transport that replaced HTTP+SSE). The `serve_stdio` / Streamable HTTP wiring is
marked `// BOUNDARY:` in `crates/rivet-mcp`; this example shows the intended call
site.

Run (as a local MCP server over stdio): `cargo run -p example-mcp`
