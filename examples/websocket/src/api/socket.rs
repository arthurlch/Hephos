use futures::{SinkExt, StreamExt};
use hephos::prelude::*;
use hephos::ws::{self, Message, Response, WebSocket, WebSocketUpgrade, WsConfig, WsHeartbeat};
use uuid::Uuid;

use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    let protected = Router::new()
        .get("/ws/protected", connect_protected)
        .authenticated(verify_token);

    Router::new().get("/ws", connect).merge(protected)
}

async fn verify_token(token: String) -> Result<Identity> {
    if token == "secret-token" {
        Ok(Identity::User(Principal {
            id: Uuid::nil(),
            roles: vec!["user".to_string()],
        }))
    } else {
        Err(Error::Unauthorized)
    }
}

/// Public WebSocket connection with default limits, heartbeats, and connection tracking.
pub async fn connect(ctx: Ctx, guard: ConnectionGuard, upgrade: WebSocketUpgrade) -> Response {
    let config = WsConfig::default();
    config
        .apply(upgrade)
        .on_upgrade(move |socket| handle(ctx, socket, config, guard))
}

/// Protected WebSocket connection requiring authentication on upgrade.
/// Accepts credentials via `Authorization: Bearer <token>` header or `?token=<token>` query param.
pub async fn connect_protected(
    ctx: Ctx,
    guard: ConnectionGuard,
    upgrade: WebSocketUpgrade,
) -> Result<Response> {
    ctx.require_user()?;
    let config = WsConfig::default();
    Ok(config
        .apply(upgrade)
        .on_upgrade(move |socket| handle(ctx, socket, config, guard)))
}

/// One connection, one loop, no spawned tasks. The socket is split so reads and
/// writes own separate halves; the loop selects over incoming frames, a heartbeat,
/// and shutdown. The connection ends on: shutdown (token), client close/disconnect
/// (`None`/`Err` from the stream, or a failed `sink.send`), idle timeout, or a receive error.
/// Backpressure is the awaited `sink.send`: when the client cannot keep up, the send
/// blocks the loop, which stops reading — flow control without an unbounded queue.
/// Holding `_guard` tracks this socket and ensures graceful draining on shutdown.
async fn handle(ctx: Ctx, socket: WebSocket, config: WsConfig, _guard: ConnectionGuard) {
    let (mut sink, mut stream) = socket.split();
    let mut heartbeat = WsHeartbeat::from_config(&config);

    loop {
        tokio::select! {
            _ = ctx.cancel_token().cancelled() => {
                let _ = sink.send(Message::Close(Some(ws::close_away("server shutting down")))).await;
                break;
            }
            _ = heartbeat.tick() => {
                if heartbeat.is_idle() {
                    let _ = sink.send(Message::Close(Some(ws::close_policy("idle timeout")))).await;
                    break;
                }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => match incoming {
                Some(Ok(msg)) => {
                    heartbeat.record_activity();
                    match msg {
                        Message::Text(text) => {
                            if sink.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                        Message::Binary(bin) => {
                            if sink.send(Message::Binary(bin)).await.is_err() {
                                break;
                            }
                        }
                        Message::Close(frame) => {
                            let _ = sink.send(Message::Close(frame)).await;
                            break;
                        }
                        Message::Ping(_) | Message::Pong(_) => {}
                    }
                }
                Some(Err(error)) => {
                    tracing::warn!(%error, "websocket receive error");
                    break;
                }
                None => break,
            }
        }
    }
}
