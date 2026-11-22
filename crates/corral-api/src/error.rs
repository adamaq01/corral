use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use corral_core::Error;
use serde::Serialize;
use ts_rs::TS;

/// A `{"error": message}` response with its status. Core errors keep their message; internal
/// ones are logged and answered with a generic 500.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

/// What every error answers with.
#[derive(Serialize, TS)]
pub(crate) struct ErrorBody {
    error: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub(crate) fn internal(cause: &dyn std::fmt::Display) -> Self {
        tracing::error!(error = %cause, "request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }

    pub(crate) fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "not signed in")
    }

    fn rejected(status: StatusCode, body_text: String) -> Self {
        if status.is_server_error() {
            Self::internal(&body_text)
        } else {
            Self::new(status, body_text)
        }
    }
}

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        let status = match &error {
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::Forbidden | Error::Inactive(_) => StatusCode::FORBIDDEN,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::LimitReached(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Error::Internal { .. } => return Self::internal(&error),
        };
        Self::new(status, error.to_string())
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        let status = match &rejection {
            JsonRejection::JsonDataError(_) => StatusCode::BAD_REQUEST,
            other => other.status(),
        };
        Self::rejected(status, rejection.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::rejected(rejection.status(), rejection.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::rejected(rejection.status(), rejection.body_text())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}
