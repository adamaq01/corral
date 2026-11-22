use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::Utc;
use corral_core::domain::{Admin, User};

use crate::error::ApiError;
use crate::session::Session;
use crate::state::AppState;

pub(crate) struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let user = Session::from_headers(&parts.headers, state)
            .current(Utc::now())
            .ok_or_else(ApiError::unauthenticated)?;
        Ok(Self(state.accounts.require_active(user).await?))
    }
}

pub(crate) struct CurrentAdmin(pub Admin);

impl FromRequestParts<AppState> for CurrentAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        Ok(Self(Admin::check(&user)?))
    }
}
