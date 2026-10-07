# example-streaming

Canonical server-sent-events streaming: a handler returns an `Sse` stream driven
by a bounded source and tied to the request cancellation token.

Demonstrates:
- HTTP streaming via SSE,
- cancellation: client disconnect fires `ctx.cancel_token()`, ending production,
- bounded resource usage: the stream emits a fixed number of events at a fixed
  rate, so one request cannot pin the server indefinitely.

This is the same streaming model the agent layer uses for token streaming — one
mechanism, not two.

Run: `cargo run -p example-streaming`, then `curl -N localhost:8080/events`.
