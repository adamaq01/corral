use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

// Bodiless POSTs need the header too: a cross-site form or a no-cors fetch cannot send it
// without a CORS preflight, which this API never grants.
pub(crate) async fn require_json(request: Request, next: Next) -> Response {
    if matches!(*request.method(), Method::POST | Method::PUT) && !is_json(request.headers()) {
        return ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "expected Content-Type: application/json",
        )
        .into_response();
    }
    next.run(request).await
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"))
}
