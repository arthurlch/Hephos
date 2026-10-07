use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The one error type for every Rivet application.
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

/// The application-wide result alias. Prefer `rivet::Result<T>` everywhere.
pub type Result<T, E = Error> = std::result::Result<T, E>;

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

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        // Internal errors are logged with their full cause chain and returned
        // opaque, so no implementation detail reaches the client. Every other
        // variant is a deliberate, safe-to-expose message.
        let message = match &self {
            Error::Internal(cause) => {
                tracing::error!(error = ?cause, "internal error");
                "internal error".to_string()
            }
            other => other.to_string(),
        };

        let body = Body {
            error: Payload {
                kind: self.kind(),
                message: &message,
            },
        };

        (self.status(), axum::Json(body)).into_response()
    }
}
