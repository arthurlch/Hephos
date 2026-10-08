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
#[derive(Debug, thiserror::Error)]
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
    pub fn not_found(what: impl Into<String>) -> Self {
        Error::NotFound(what.into())
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Error::Conflict(msg.into())
    }

    /// Wrap any error as an internal error. Use `?` with `From` where possible;
    /// use this only to add a message to a non-`std::error::Error` value.
    pub fn internal(msg: impl Into<String>) -> Self {
        Error::Internal(anyhow::anyhow!(msg.into()))
    }

    fn status(&self) -> StatusCode {
        match self {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::Invalid(_) => "invalid",
            Error::Unauthorized => "unauthorized",
            Error::Forbidden => "forbidden",
            Error::Conflict(_) => "conflict",
            Error::Internal(_) => "internal",
        }
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
}
