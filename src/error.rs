use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    #[error("bad gateway: {0}")]
    BadGateway(String),
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => AppError::NotFound,
            _ => AppError::Internal(e.to_string()),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, ""),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::TooManyRequests(_) => (StatusCode::TOO_MANY_REQUESTS, ""),
            AppError::BadGateway(_) => (StatusCode::BAD_GATEWAY, ""),
            AppError::ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, ""),
            AppError::Internal(detail) => {
                error!("internal error: {}", detail);
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
        };
        let body =
            json!({"error": if msg.is_empty() { self.to_string() } else { msg.to_string() }});
        (status, axum::Json(body)).into_response()
    }
}
