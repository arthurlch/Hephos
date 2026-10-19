//! WebSocket integration tests.

use std::time::Duration;

use hephos_core::test::TestClient;
use hephos_core::ws::{
    Response, WebSocket, WebSocketUpgrade, WsConfig, close_away, close_normal, close_policy,
    close_size,
};
use hephos_core::{Ctx, Json, Result, Router};

#[derive(Clone)]
struct S;

async fn ws_handler(_ctx: Ctx<S>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|mut socket: WebSocket| async move {
        while let Some(Ok(msg)) = socket.recv().await {
            if socket.send(msg).await.is_err() {
                break;
            }
        }
    })
}

async fn config_info(_ctx: Ctx<S>) -> Result<Json<usize>> {
    let config = WsConfig::default()
        .max_message_size(128 * 1024)
        .max_frame_size(64 * 1024)
        .heartbeat_interval(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(30));
    Ok(Json(config.max_message_size))
}

fn routes() -> Router<S> {
    Router::new()
        .get("/ws", ws_handler)
        .get("/config", config_info)
}

#[tokio::test]
async fn ws_rejects_non_websocket_http_request() {
    let client = TestClient::new(routes(), S);
    let res = client.get("/ws").await;
    // Without WebSocket upgrade headers, Axum returns 400 or 426 Upgrade Required
    assert!(res.status() == 400 || res.status() == 426);
}

#[tokio::test]
async fn ws_config_and_close_frame_helpers() {
    let client = TestClient::new(routes(), S);
    let res = client.get("/config").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<usize>(), 128 * 1024);

    let norm = close_normal("bye");
    assert_eq!(norm.code, 1000);
    assert_eq!(norm.reason.as_str(), "bye");

    let away = close_away("going away");
    assert_eq!(away.code, 1001);
    assert_eq!(away.reason.as_str(), "going away");

    let policy = close_policy("policy violation");
    assert_eq!(policy.code, 1008);
    assert_eq!(policy.reason.as_str(), "policy violation");

    let size = close_size("too large");
    assert_eq!(size.code, 1009);
    assert_eq!(size.reason.as_str(), "too large");
}
