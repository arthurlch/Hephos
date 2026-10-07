use rivet::test::TestClient;

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
