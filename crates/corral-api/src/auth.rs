use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::Utc;
use corral_core::domain::{Admin, User};

use crate::error::ApiError;
use crate::session::Session;
use crate::state::shared;

/// The signed-in user, who must be active.
pub struct CurrentUser(pub User);

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        let api = shared(parts)?;
        let user = Session::from_parts(parts, &api)
            .current(Utc::now())
            .ok_or_else(ApiError::unauthenticated)?;
        Ok(Self(api.accounts.require_active(user).await?))
    }
}

/// Proof that the signed-in user is an active admin: 401 without a session, else 403.
pub struct CurrentAdmin(pub Admin);

impl<S: Send + Sync> FromRequestParts<S> for CurrentAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        Ok(Self(Admin::check(&user)?))
    }
}
