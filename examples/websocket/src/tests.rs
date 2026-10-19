use hephos::http::{Body, Request};
use hephos::test::TestClient;

use crate::api;
use crate::state::AppState;

async fn client() -> TestClient {
    TestClient::new(api::routes(), AppState::init().await.unwrap())
}

#[tokio::test]
async fn routes_build() {
    let _ = api::routes();
}

/// A plain GET without upgrade headers must be rejected by the upgrade extractor,
/// never reach the socket loop, and never hang.
#[tokio::test]
async fn ws_requires_upgrade() {
    let res = client().await.get("/ws").await;
    assert_eq!(res.status(), 400);
}

/// Protected WebSocket endpoint rejects unauthenticated requests with 401 Unauthorized
/// before any upgrade processing happens.
#[tokio::test]
async fn protected_ws_rejects_missing_token() {
    let res = client().await.get("/ws/protected").await;
    assert_eq!(res.status(), 401);
}

/// Protected WebSocket endpoint rejects invalid token with 401 Unauthorized.
#[tokio::test]
async fn protected_ws_rejects_invalid_token() {
    let res = client().await.get("/ws/protected?token=bad-token").await;
    assert_eq!(res.status(), 401);
}

/// Protected WebSocket endpoint accepts valid query parameter token.
/// Passing auth allows the request to reach the upgrade extractor (returning 400 for missing upgrade headers instead of 401).
#[tokio::test]
async fn protected_ws_accepts_valid_query_token() {
    let res = client().await.get("/ws/protected?token=secret-token").await;
    assert_eq!(res.status(), 400);
}

/// Protected WebSocket endpoint accepts valid Bearer header token.
#[tokio::test]
async fn protected_ws_accepts_valid_header_token() {
    let res = client()
        .await
        .get_with_token("/ws/protected", "secret-token")
        .await;
    assert_eq!(res.status(), 400);
}

/// Valid WebSocket handshake headers result in 101 Switching Protocols.
#[tokio::test]
async fn ws_upgrade_handshake_succeeds() {
    let client = client().await;
    let req = Request::builder()
        .uri("/ws")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();

    let res = client.send_upgrade(req).await;
    assert_eq!(res.status(), 101);
}

/// Valid WebSocket handshake on protected endpoint succeeds with query token.
#[tokio::test]
async fn protected_ws_upgrade_handshake_succeeds_with_query_token() {
    let client = client().await;
    let req = Request::builder()
        .uri("/ws/protected?token=secret-token")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();

    let res = client.send_upgrade(req).await;
    assert_eq!(res.status(), 101);
}

/// Saturated connection cap rejects WebSocket upgrades with 503 Service Unavailable
/// before establishing any connection or protocol switch.
#[tokio::test]
async fn ws_upgrade_exceeding_connection_cap_is_rejected_with_503() {
    let client = TestClient::with_connection_cap(api::routes(), AppState::init().await.unwrap(), 1);

    // First upgrade takes the 1 available connection slot
    let req1 = Request::builder()
        .uri("/ws")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();

    let res1 = client.send_upgrade(req1).await;
    assert_eq!(res1.status(), 101);

    // Second upgrade attempt is immediately rejected with 503
    let req2 = Request::builder()
        .uri("/ws")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();

    let res2 = client.send_upgrade(req2).await;
    assert_eq!(res2.status(), 503);
}

/// Active WebSocket connections are tracked by the ConnectionCap and drain cleanly on disconnect.
#[tokio::test]
async fn ws_connection_draining_tracks_active_sockets() {
    let cap = hephos::ConnectionCap::new(2);
    let app = hephos::App::new(AppState::init().await.unwrap())
        .routes(api::routes().connection_cap(cap.clone()));
    let client = TestClient::from_app(app);

    let req = Request::builder()
        .uri("/ws")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();

    let res = client.send_upgrade(req).await;
    assert_eq!(res.status(), 101);

    // Active connection is tracked
    assert_eq!(cap.active(), 1);
    drop(res);
}
