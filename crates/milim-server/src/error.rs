//! HTTP error mapping: `milim_core::Error` → status + OpenAI error envelope.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use milim_core::api::openai::ErrorEnvelope;
use milim_core::Error;

/// Newtype so we can implement `IntoResponse` for the core error.
pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self.0 {
            Error::InvalidRequest(_) | Error::Json(_) => StatusCode::BAD_REQUEST,
            Error::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Error::ModelNotFound(_) | Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let env = ErrorEnvelope::new(self.0.to_string(), self.0.code());
        (status, Json(env)).into_response()
    }
}
