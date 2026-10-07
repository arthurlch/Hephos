# example-websocket

Canonical WebSocket shape: an upgrade handler that takes `Ctx`, then a connection
loop that `select!`s over incoming messages and the request cancellation token.

Demonstrates:
- connection lifecycle (open → messages → close/error),
- cancellation: the loop ends on client disconnect **or** server shutdown,
- backpressure: outbound messages go through a bounded channel, so a slow client
  slows the producer instead of growing an unbounded queue,
- error handling: a send/receive error ends the connection cleanly.

The handler is a simple echo with a server-pushed heartbeat, which is enough to
show both directions and the backpressure seam.

Run: `cargo run -p example-websocket`, then connect to `ws://localhost:8080/ws`.
