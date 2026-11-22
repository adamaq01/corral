use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, IntoResponseParts, Response, ResponseParts};
use axum::{Extension, Json, Router};
use chrono::Utc;
use corral_core::domain::{Admin, User};
use corral_core::ports::Identity;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::cookies::Jar;
use crate::dto::Me;
use crate::error::ApiError;
use crate::session::Session;
use crate::state::{AppState, shared};

/// Where a provider's routes are served.
#[must_use]
pub fn provider_path(id: &str) -> String {
    format!("/auth/{id}")
}

/// A provider's own routes, served under `provider_path(id)`, and what `GET /auth/providers`
/// shows about it, next to its id, as given.
pub struct ProviderRoutes {
    pub id: String,
    pub description: Map<String, Value>,
    pub routes: Router,
}

#[derive(Clone)]
struct Mounted(Arc<str>);

impl ProviderRoutes {
    pub(crate) fn listing(&self) -> Value {
        let mut listed = self.description.clone();
        listed.insert("id".to_owned(), Value::from(self.id.as_str()));
        Value::Object(listed)
    }

    pub(crate) fn mounted(self) -> Router {
        self.routes
            .layer(Extension(Mounted(Arc::from(self.id.as_str()))))
    }
}

fn mounted(parts: &Parts) -> Result<Arc<str>, ApiError> {
    parts
        .extensions
        .get::<Mounted>()
        .map(|Mounted(id)| id.clone())
        .ok_or_else(|| ApiError::internal(&"used outside a provider's routes"))
}

/// What a provider's routes do with the identities they establish.
pub struct Identities {
    api: AppState,
    provider: Arc<str>,
    session: Session,
}

impl Identities {
    fn extract(parts: &Parts) -> Result<Self, ApiError> {
        let api = shared(parts)?;
        Ok(Self {
            provider: mounted(parts)?,
            session: Session::from_parts(parts, &api),
            api,
        })
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Identities {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(Self::extract(parts))
    }
}

impl Identities {
    /// Logs the person in as their user, made at their first login, and starts their session.
    pub async fn sign_in(self, identity: Identity) -> Result<SignedIn, ApiError> {
        let user = self.api.accounts.login(&self.provider, identity).await?;
        Ok(SignedIn {
            session: self.session.start(user.id, Utc::now()),
            user,
        })
    }

    /// Deletes the user the subject logged in as with everything they own, or, if it never
    /// did, only has the provider forget it. Whether there was such a user.
    pub async fn delete(&self, admin: &Admin, subject: &str) -> Result<bool, ApiError> {
        Ok(self
            .api
            .accounts
            .delete_identity(admin, &self.provider, subject)
            .await?)
    }
}

/// A started session. As a response, it also tells who signed in; as response parts, it only
/// sets the session cookie.
pub struct SignedIn {
    session: Session,
    user: User,
}

impl IntoResponseParts for SignedIn {
    type Error = Infallible;

    fn into_response_parts(self, parts: ResponseParts) -> Result<ResponseParts, Infallible> {
        self.session.into_response_parts(parts)
    }
}

impl IntoResponse for SignedIn {
    fn into_response(self) -> Response {
        (self.session, Json(Me::from(&self.user))).into_response()
    }
}

/// Encrypted cookies a provider keeps between the steps of its login, seen only by its own
/// routes. Changes reach the browser when the cookies are part of the response.
pub struct ProviderCookies {
    jar: Jar,
    provider: Arc<str>,
}

impl ProviderCookies {
    fn extract(parts: &Parts) -> Result<Self, ApiError> {
        let api = shared(parts)?;
        Ok(Self {
            jar: Jar::from_headers(&parts.headers, &api.session_key, api.secure_cookies),
            provider: mounted(parts)?,
        })
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ProviderCookies {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(Self::extract(parts))
    }
}

impl ProviderCookies {
    /// `None` when absent, tampered with, or not a `T`.
    #[must_use]
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        self.jar.read(&self.name(name))
    }

    #[must_use]
    pub fn set(self, name: &str, value: &impl Serialize, max_age: Duration) -> Self {
        let max_age =
            cookie::time::Duration::try_from(max_age).unwrap_or(cookie::time::Duration::MAX);
        let cookie = self
            .jar
            .cookie(self.name(name), self.path(), value)
            .max_age(max_age);
        Self {
            jar: self.jar.add(cookie),
            provider: self.provider,
        }
    }

    #[must_use]
    pub fn remove(self, name: &str) -> Self {
        let (name, path) = (self.name(name), self.path());
        Self {
            jar: self.jar.remove(name, path),
            provider: self.provider,
        }
    }

    fn name(&self, name: &str) -> String {
        format!("corral_{}_{name}", self.provider)
    }

    fn path(&self) -> String {
        provider_path(&self.provider)
    }
}

impl IntoResponseParts for ProviderCookies {
    type Error = Infallible;

    fn into_response_parts(self, parts: ResponseParts) -> Result<ResponseParts, Infallible> {
        self.jar.into_response_parts(parts)
    }
}
