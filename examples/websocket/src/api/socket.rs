use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rivet::prelude::*;
use rivet::ws::{Message, Response, WebSocket, WebSocketUpgrade};
use tokio::sync::mpsc;

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

async fn handle(ctx: Ctx, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();

    // Outbound queue is BOUNDED: when the client cannot keep up, `outbound.send`
    // awaits, which propagates backpressure to whatever produces messages.
    let (outbound, mut rx) = mpsc::channel::<Message>(32);

    // Writer drains the bounded queue to the socket. It is joined before this
    // function returns (see the end) — owned, not detached.
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });

    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            _ = ctx.cancel_token().cancelled() => break,
            _ = heartbeat.tick() => {
                if outbound.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if outbound.send(Message::Text(text)).await.is_err() {
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

    drop(outbound);
    let _ = writer.await;
}
