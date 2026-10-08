use std::time::Duration;

use futures::{SinkExt, StreamExt};
use hephos::prelude::*;
use hephos::ws::{Message, Response, WebSocket, WebSocketUpgrade};

use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new().get("/ws", connect)
}

/// Upgrade handler: takes `Ctx` like every handler, then hands the socket to the
/// connection loop. The `Ctx` carries the cancellation token, so the connection
/// is tied to request and server lifecycle.
pub async fn connect(ctx: Ctx, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| handle(ctx, socket))
}

/// One connection, one loop, no spawned tasks. The socket is split so reads and
/// writes own separate halves; the loop selects over incoming frames, a heartbeat,
/// and cancellation. Backpressure is the awaited `sink.send`: when the client
/// cannot keep up, the send blocks the loop, which stops reading — flow control
/// without an unbounded queue.
async fn handle(ctx: Ctx, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            _ = ctx.cancel_token().cancelled() => break,
            _ = heartbeat.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if sink.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    tracing::warn!(%error, "websocket receive error");
                    break;
                }
            }
        }
    }
}
