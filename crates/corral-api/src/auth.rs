use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::cookie::PrivateCookieJar;
use chrono::Utc;
use corral_core::domain::{Admin, User};

use crate::error::ApiError;
use crate::session::SessionCodec;
use crate::state::AppState;

pub(crate) struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let jar = PrivateCookieJar::from_headers(&parts.headers, state.sessions.key());
        let session =
            SessionCodec::decode_session(&jar, Utc::now()).ok_or_else(ApiError::unauthenticated)?;
        Ok(Self(state.accounts.require_active(session.user_id).await?))
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
