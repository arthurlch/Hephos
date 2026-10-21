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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Json<T>(pub T);

impl<T> Json<T> {
    /// Consume the extractor and return the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::ops::Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Path<T>(pub T);

impl<T> Path<T> {
    /// Consume the extractor and return the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::ops::Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Path<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Query<T>(pub T);

impl<T> Query<T> {
    /// Consume the extractor and return the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::ops::Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

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
//
// A body that exceeds the stack's limit surfaces here (as a length-limit rejection)
// when there is no `Content-Length` for the body-limit layer to short-circuit on. Map
// it to `413` so an oversized body reports the same status however it arrives, rather
// than being silently downgraded to `400`.
fn map_json(rejection: JsonRejection) -> Error {
    if rejection.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        return Error::PayloadTooLarge;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::FromRequest;
    use axum::http::Request;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct UserPayload {
        name: String,
        age: u32,
    }

    #[tokio::test]
    async fn json_extractor_valid_body() {
        let req = Request::builder()
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"name":"Alice","age":30}"#))
            .unwrap();
        let extracted = Json::<UserPayload>::from_request(req, &()).await.unwrap();
        assert_eq!(
            extracted.0,
            UserPayload {
                name: "Alice".into(),
                age: 30
            }
        );
    }

    #[tokio::test]
    async fn json_extractor_invalid_syntax_maps_to_safe_opaque_error() {
        let req = Request::builder()
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"name": broken"#))
            .unwrap();
        let err = Json::<UserPayload>::from_request(req, &())
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            Error::invalid("request body is not valid JSON for this endpoint").to_string()
        );
    }

    #[tokio::test]
    async fn json_extractor_wrong_schema_maps_to_safe_opaque_error() {
        let req = Request::builder()
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"name":"Alice","age":"not-a-number"}"#,
            ))
            .unwrap();
        let err = Json::<UserPayload>::from_request(req, &())
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            Error::invalid("request body is not valid JSON for this endpoint").to_string()
        );
    }

    #[derive(Debug, PartialEq, Eq, Deserialize)]
    struct QueryParams {
        search: String,
        limit: u32,
    }

    #[tokio::test]
    async fn query_extractor_valid_and_invalid() {
        use axum::extract::FromRequestParts;

        // Valid query
        let req = Request::builder()
            .uri("/items?search=rust&limit=10")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts, _) = req.into_parts();
        let query = Query::<QueryParams>::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(
            query.0,
            QueryParams {
                search: "rust".into(),
                limit: 10
            }
        );

        // Invalid query parameter (wrong type)
        let req_invalid = Request::builder()
            .uri("/items?search=rust&limit=abc")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts_inv, _) = req_invalid.into_parts();
        let err = Query::<QueryParams>::from_request_parts(&mut parts_inv, &())
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            Error::invalid("invalid query string").to_string()
        );
    }

    #[test]
    fn extractor_deref_and_into_inner() {
        let mut json = Json(UserPayload {
            name: "Bob".into(),
            age: 25,
        });
        assert_eq!(json.name, "Bob");
        assert_eq!(json.age, 25);
        json.age = 26;
        assert_eq!(json.age, 26);
        let inner = json.into_inner();
        assert_eq!(inner.name, "Bob");

        let mut path = Path(42u32);
        assert_eq!(*path, 42);
        *path = 43;
        assert_eq!(*path, 43);
        assert_eq!(path.into_inner(), 43);

        let mut query = Query(QueryParams {
            search: "test".into(),
            limit: 5,
        });
        assert_eq!(query.search, "test");
        query.limit = 10;
        assert_eq!(query.limit, 10);
        assert_eq!(query.into_inner().limit, 10);
    }
}
