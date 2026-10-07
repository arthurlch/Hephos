# example-websocket

Canonical WebSocket shape: an upgrade handler that takes `Ctx`, then a connection
loop that `select!`s over incoming messages and the request cancellation token.

Demonstrates:
- connection lifecycle (open → messages → close/error),
- cancellation: the loop ends on client disconnect **or** server shutdown,
- backpressure: the awaited `sink.send` blocks the loop when the client cannot
  keep up, so a slow client throttles reads instead of growing an unbounded queue
  — flow control with no spawned writer task and no detached work,
- error handling: a send/receive error ends the connection cleanly.

The handler is a simple echo with a server-pushed heartbeat, which is enough to
show both directions and the backpressure seam.

Run: `cargo run -p example-websocket`, then connect to `ws://localhost:8080/ws`.
