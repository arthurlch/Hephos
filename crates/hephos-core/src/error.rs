use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The one error type for every Hephos application.
///
/// Every fallible function in an app returns [`Result<T>`]. Handlers, services,
/// repositories, tasks, tools, and workflows all share this type. There is no
/// per-module error enum, because a single error type is the thing a coding
/// agent can reason about completely.
///
/// Each variant maps to exactly one HTTP status. [`Error::Internal`] is the
/// escape hatch for anything unexpected: its full cause chain is logged, but the
/// response body never leaks internal detail.
// `#[non_exhaustive]`: new variants (a future auth or rate-limit status, say) must not
// break a downstream `match` after 1.0. App code constructs errors via the `Error::*`
// helpers and maps them through `IntoResponse`, so this costs apps nothing; external
// code that matches must add a `_` arm.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid: {0}")]
    Invalid(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("payload too large")]
    PayloadTooLarge,

    #[error("request timeout")]
    Timeout,

    #[error("method not allowed")]
    MethodNotAllowed,

    #[error("too many requests: {0}")]
    RateLimited(String),

    #[error("service unavailable: {0}")]
    Unavailable(String),

    /// Anything unexpected. Logged in full, returned as an opaque 500.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// The application-wide result alias. Prefer `hephos::Result<T>` everywhere.
pub type Result<T, E = Error> = std::result::Result<T, E>;

// Let repositories use `?` on SQLx results directly: any `sqlx::Error` becomes an
// opaque `Internal` (logged in full, returned as a generic 500). "Row not found" is
// still expressed explicitly by repos via `fetch_optional` → `None`, never a leaked
// `RowNotFound`. Gated behind the `sqlx` feature so core stays sqlx-free by default.
#[cfg(feature = "sqlx")]
impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Error::Internal(anyhow::Error::new(error))
    }
}

impl Error {
    /// A `404`: the named resource does not exist, e.g. `Error::not_found("user")`.
    pub fn not_found(what: impl Into<String>) -> Self {
        Error::NotFound(what.into())
    }

    /// A `400`: the request broke a business rule. The message is shown to the client,
    /// e.g. `Error::invalid("email is required")`.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }

    /// A `401`: the client is unauthenticated or credentials are missing/invalid.
    pub fn unauthorized() -> Self {
        Error::Unauthorized
    }

    /// A `403`: the client is authenticated but lacks required permissions/roles.
    pub fn forbidden() -> Self {
        Error::Forbidden
    }

    /// A `409`: the request conflicts with current state, e.g. a duplicate unique key.
    pub fn conflict(msg: impl Into<String>) -> Self {
        Error::Conflict(msg.into())
    }

    /// A `413`: the request payload exceeds the allowed body limit.
    pub fn payload_too_large() -> Self {
        Error::PayloadTooLarge
    }

    /// A `408`: the request timed out before finishing.
    pub fn timeout() -> Self {
        Error::Timeout
    }

    /// A `405`: the HTTP method is not allowed for this route.
    pub fn method_not_allowed() -> Self {
        Error::MethodNotAllowed
    }

    /// A `429`: the client has exceeded its rate limit quota.
    pub fn rate_limited(msg: impl Into<String>) -> Self {
        Error::RateLimited(msg.into())
    }

    /// A `503`: a downstream dependency is down or the service is temporarily unavailable.
    pub fn unavailable(msg: impl Into<String>) -> Self {
        Error::Unavailable(msg.into())
    }

    /// Wrap any error as an internal error. Use `?` with `From` where possible;
    /// use this only to add a message to a non-`std::error::Error` value.
    pub fn internal(msg: impl Into<String>) -> Self {
        Error::Internal(anyhow::anyhow!(msg.into()))
    }

    /// The HTTP status code corresponding to this error.
    pub fn status(&self) -> StatusCode {
        match self {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Error::Timeout => StatusCode::REQUEST_TIMEOUT,
            Error::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Error::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Error::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The machine-readable error kind identifier (e.g. `"not_found"`, `"unauthorized"`).
    pub fn kind(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::Invalid(_) => "invalid",
            Error::Unauthorized => "unauthorized",
            Error::Forbidden => "forbidden",
            Error::Conflict(_) => "conflict",
            Error::PayloadTooLarge => "payload_too_large",
            Error::Timeout => "timeout",
            Error::MethodNotAllowed => "method_not_allowed",
            Error::RateLimited(_) => "rate_limited",
            Error::Unavailable(_) => "unavailable",
            Error::Internal(_) => "internal",
        }
    }

    /// Returns `true` if this error is [`Error::NotFound`].
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::NotFound(_))
    }

    /// Returns `true` if this error is [`Error::Invalid`].
    pub fn is_invalid(&self) -> bool {
        matches!(self, Error::Invalid(_))
    }

    /// Returns `true` if this error is [`Error::Unauthorized`].
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Error::Unauthorized)
    }

    /// Returns `true` if this error is [`Error::Forbidden`].
    pub fn is_forbidden(&self) -> bool {
        matches!(self, Error::Forbidden)
    }

    /// Returns `true` if this error is [`Error::Conflict`].
    pub fn is_conflict(&self) -> bool {
        matches!(self, Error::Conflict(_))
    }

    /// Returns `true` if this error is [`Error::PayloadTooLarge`].
    pub fn is_payload_too_large(&self) -> bool {
        matches!(self, Error::PayloadTooLarge)
    }

    /// Returns `true` if this error is [`Error::Timeout`].
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Timeout)
    }

    /// Returns `true` if this error is [`Error::MethodNotAllowed`].
    pub fn is_method_not_allowed(&self) -> bool {
        matches!(self, Error::MethodNotAllowed)
    }

    /// Returns `true` if this error is [`Error::RateLimited`].
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Error::RateLimited(_))
    }

    /// Returns `true` if this error is [`Error::Unavailable`].
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Error::Unavailable(_))
    }

    /// Returns `true` if this error is [`Error::Internal`].
    pub fn is_internal(&self) -> bool {
        matches!(self, Error::Internal(_))
    }
}

#[derive(Serialize)]
struct Body<'a> {
    error: Payload<'a>,
}

#[derive(Serialize)]
struct Payload<'a> {
    kind: &'a str,
    message: &'a str,
}

impl Error {
    /// The client-facing message. It never repeats the `kind` and never leaks
    /// internals: `Internal` is logged in full and reported opaquely.
    fn message(&self) -> String {
        match self {
            Error::NotFound(what) => format!("{what} not found"),
            Error::Invalid(msg) => msg.clone(),
            Error::Unauthorized => "unauthorized".to_string(),
            Error::Forbidden => "forbidden".to_string(),
            Error::Conflict(msg) => msg.clone(),
            Error::PayloadTooLarge => "request body too large".to_string(),
            Error::Timeout => "request timed out".to_string(),
            Error::MethodNotAllowed => "method not allowed".to_string(),
            Error::RateLimited(msg) => msg.clone(),
            Error::Unavailable(msg) => msg.clone(),
            Error::Internal(cause) => {
                tracing::error!(error = ?cause, "internal error");
                "internal error".to_string()
            }
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let message = self.message();
        let body = Body {
            error: Payload {
                kind: self.kind(),
                message: &message,
            },
        };

        (self.status(), axum::Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn status_mapping_is_exhaustive_and_correct() {
        assert_eq!(Error::not_found("user").status(), StatusCode::NOT_FOUND);
        assert_eq!(Error::invalid("x").status(), StatusCode::BAD_REQUEST);
        assert_eq!(Error::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(Error::Forbidden.status(), StatusCode::FORBIDDEN);
        assert_eq!(Error::conflict("x").status(), StatusCode::CONFLICT);
        assert_eq!(
            Error::PayloadTooLarge.status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(Error::Timeout.status(), StatusCode::REQUEST_TIMEOUT);
        assert_eq!(
            Error::MethodNotAllowed.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            Error::rate_limited("quota").status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            Error::unavailable("down").status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            Error::internal("boom").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn kind_strings_are_stable() {
        assert_eq!(Error::not_found("u").kind(), "not_found");
        assert_eq!(Error::invalid("i").kind(), "invalid");
        assert_eq!(Error::Unauthorized.kind(), "unauthorized");
        assert_eq!(Error::Forbidden.kind(), "forbidden");
        assert_eq!(Error::conflict("c").kind(), "conflict");
        assert_eq!(Error::PayloadTooLarge.kind(), "payload_too_large");
        assert_eq!(Error::Timeout.kind(), "timeout");
        assert_eq!(Error::MethodNotAllowed.kind(), "method_not_allowed");
        assert_eq!(Error::rate_limited("q").kind(), "rate_limited");
        assert_eq!(Error::unavailable("d").kind(), "unavailable");
        assert_eq!(Error::internal("x").kind(), "internal");
    }

    #[test]
    fn not_found_message_reads_cleanly_without_repeating_kind() {
        assert_eq!(Error::not_found("user").message(), "user not found");
        assert_eq!(Error::invalid("email required").message(), "email required");
    }

    #[test]
    fn internal_message_is_opaque() {
        // The cause must never reach the client-facing message.
        let err = Error::internal("secret db dsn leaked here");
        assert_eq!(err.message(), "internal error");
    }

    #[test]
    fn anyhow_converts_to_internal() {
        let err: Error = anyhow::anyhow!("underlying").into();
        assert!(matches!(err, Error::Internal(_)));
    }

    #[tokio::test]
    async fn into_response_renders_canonical_json_payload() {
        use axum::body::to_bytes;
        use serde_json::Value;

        let err = Error::conflict("email taken");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["error"]["kind"], "conflict");
        assert_eq!(json["error"]["message"], "email taken");
    }

    #[tokio::test]
    async fn error_into_response_all_variants_exhaustive() {
        use axum::body::to_bytes;
        use serde_json::Value;

        let cases: Vec<(Error, StatusCode, &'static str, &'static str)> = vec![
            (
                Error::not_found("book"),
                StatusCode::NOT_FOUND,
                "not_found",
                "book not found",
            ),
            (
                Error::invalid("name required"),
                StatusCode::BAD_REQUEST,
                "invalid",
                "name required",
            ),
            (
                Error::Unauthorized,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "unauthorized",
            ),
            (
                Error::Forbidden,
                StatusCode::FORBIDDEN,
                "forbidden",
                "forbidden",
            ),
            (
                Error::conflict("slug in use"),
                StatusCode::CONFLICT,
                "conflict",
                "slug in use",
            ),
            (
                Error::PayloadTooLarge,
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "request body too large",
            ),
            (
                Error::Timeout,
                StatusCode::REQUEST_TIMEOUT,
                "timeout",
                "request timed out",
            ),
            (
                Error::MethodNotAllowed,
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "method not allowed",
            ),
            (
                Error::rate_limited("wait 5s"),
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "wait 5s",
            ),
            (
                Error::unavailable("service down"),
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "service down",
            ),
            (
                Error::internal("database connection error with secret password"),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error", // guaranteed opaque
            ),
        ];

        for (err, expected_status, expected_kind, expected_message) in cases {
            let res = err.into_response();
            assert_eq!(res.status(), expected_status);
            let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["error"]["kind"], expected_kind);
            assert_eq!(json["error"]["message"], expected_message);
        }
    }

    #[test]
    fn error_display_formats_cleanly() {
        assert_eq!(format!("{}", Error::not_found("item")), "not found: item");
        assert_eq!(
            format!("{}", Error::invalid("bad field")),
            "invalid: bad field"
        );
        assert_eq!(format!("{}", Error::Unauthorized), "unauthorized");
        assert_eq!(format!("{}", Error::Forbidden), "forbidden");
        assert_eq!(format!("{}", Error::conflict("dup")), "conflict: dup");
        assert_eq!(format!("{}", Error::PayloadTooLarge), "payload too large");
        assert_eq!(format!("{}", Error::Timeout), "request timeout");
        assert_eq!(format!("{}", Error::MethodNotAllowed), "method not allowed");
        assert_eq!(
            format!("{}", Error::rate_limited("slow down")),
            "too many requests: slow down"
        );
        assert_eq!(
            format!("{}", Error::unavailable("maintenance")),
            "service unavailable: maintenance"
        );
        assert_eq!(
            format!("{}", Error::internal("unexpected failure")),
            "unexpected failure"
        );
    }
}
