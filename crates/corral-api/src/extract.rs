use axum::extract::{FromRequest, FromRequestParts};

use crate::error::ApiError;

/// `axum::Json` whose rejections are API errors.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct JsonBody<T>(pub T);

/// `axum::extract::Path` whose rejections are API errors.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct PathParam<T>(pub T);

/// `axum::extract::Query` whose rejections are API errors.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct QueryParams<T>(pub T);
