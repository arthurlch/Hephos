# example-rest

The canonical Hephos application shape. Every other example mirrors this.

Demonstrates: application startup, routing, path/query/JSON extraction, response
types, the one error type, and the service layer — read-only, no database (that is
`example-database`). Mutable shared state deliberately does not appear here; the
doctrine pushes it to the database or a dedicated task.

```
src/
  main.rs            build AppState, routes; run.
  state.rs           AppState, `type Ctx`.
  domain/product.rs  data types.
  services/catalog.rs business logic over an immutable catalog.
  api/products.rs    HTTP handlers + routes().
```

Run: `HEPHOS_ADDR=0.0.0.0:8080 cargo run -p example-rest`
