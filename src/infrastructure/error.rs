pub use crate::application::error::AppError;
use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde_json::json;
impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(kind="database", code=?e.as_database_error().and_then(|d|d.code()), "database operation failed");
        Self::Internal
    }
}
impl ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Busy => StatusCode::TOO_MANY_REQUESTS,
            Self::Provider => StatusCode::BAD_GATEWAY,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    fn error_response(&self) -> HttpResponse {
        let code = match self {
            Self::Invalid(_) => "invalid_input",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Busy => "busy",
            Self::Provider => "provider_unavailable",
            Self::Internal => "internal_error",
        };
        HttpResponse::build(self.status_code())
            .json(json!({"error":{"code":code,"message":self.to_string()}}))
    }
}
