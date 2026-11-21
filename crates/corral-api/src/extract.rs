use axum::extract::{FromRequest, FromRequestParts};

use crate::error::ApiError;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub(crate) struct JsonBody<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub(crate) struct PathParam<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub(crate) struct QueryParams<T>(pub T);
