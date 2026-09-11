use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Uniform error type for API handlers: maps Kubernetes/Postgres failures to a
/// sensible HTTP status and a `{"error": "..."}` JSON body.
pub enum ApiError {
    Kube(kube::Error),
    Sqlx(sqlx::Error),
    BadRequest(String),
    Unauthorized,
    Forbidden(String),
    NotFound(String),
    /// The proxied app couldn't be reached (connection timeout/failure,
    /// handshake failure, ...) — distinct from `BadRequest` since it's not
    /// the caller's fault.
    ProxyUnavailable(String),
    /// Too many failed logins from one source address — see
    /// `AppState::login_blocked`.
    TooManyRequests(String),
}

impl From<kube::Error> for ApiError {
    fn from(err: kube::Error) -> Self {
        ApiError::Kube(err)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::Sqlx(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            // The API server's own validation messages ("already exists",
            // "must be no more than ...") are the most useful thing we can
            // show, and say nothing the caller couldn't learn by asking it.
            ApiError::Kube(kube::Error::Api(status)) => (
                StatusCode::from_u16(status.code).unwrap_or(StatusCode::BAD_GATEWAY),
                status.message,
            ),
            // Everything below is an internal fault. The detail goes to the
            // logs, not to the caller: a database error in particular quotes
            // SQL and column names straight back at whoever triggered it.
            ApiError::Kube(err) => {
                tracing::error!(error = %err, "kubernetes request failed");
                (StatusCode::BAD_GATEWAY, "the cluster could not be reached".to_string())
            }
            ApiError::Sqlx(err) => {
                tracing::error!(error = %err, "database request failed");
                (StatusCode::SERVICE_UNAVAILABLE, "the database could not be reached".to_string())
            }
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "not logged in".to_string()),
            ApiError::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::ProxyUnavailable(message) => (StatusCode::BAD_GATEWAY, message),
            ApiError::TooManyRequests(message) => (StatusCode::TOO_MANY_REQUESTS, message),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
