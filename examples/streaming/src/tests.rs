use crate::api;

/// Building the router must not panic. The SSE body itself is a timed 10-event
/// stream, so it is exercised with the live smoke test (see the README) rather
/// than drained synchronously here; this canary still guards route composition.
#[tokio::test]
async fn routes_build() {
    let _ = api::routes();
}
