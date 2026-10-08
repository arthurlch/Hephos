use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts};
use axum::http::Request;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Error;

/// Typed JSON body. The only way to read a request body in Hephos.
///
/// On deserialization failure it produces [`Error::Invalid`] with a message, so
/// bad input is a clean `400` that flows through the one error type — never an
/// axum-shaped rejection an agent has to learn separately.
pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request(
        req: Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Json(value)),
            Err(rejection) => Err(map_json(rejection)),
        }
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// Typed path parameters, e.g. `Path(id): Path<Uuid>`.
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Path(value)),
            Err(rejection) => Err(map_path(rejection)),
        }
    }
}

/// Typed query string, e.g. `Query(filter): Query<ListFilter>`.
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Query(value)),
            Err(rejection) => Err(map_query(rejection)),
        }
    }
}

// Bad input is a client error, but the raw deserializer text (field names, expected
// tokens, positions) is internal detail. Log it for debugging; return a fixed,
// safe-to-expose message so the response leaks nothing about the schema.
fn map_json(rejection: JsonRejection) -> Error {
    tracing::debug!(detail = %rejection.body_text(), "rejected JSON body");
    Error::invalid("request body is not valid JSON for this endpoint")
}

fn map_path(rejection: PathRejection) -> Error {
    tracing::debug!(detail = %rejection.body_text(), "rejected path parameter");
    Error::invalid("invalid path parameter")
}

fn map_query(rejection: QueryRejection) -> Error {
    tracing::debug!(detail = %rejection.body_text(), "rejected query string");
    Error::invalid("invalid query string")
}
