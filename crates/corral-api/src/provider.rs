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

/// The web UI's page that a failed redirect login comes back to.
pub const SIGN_IN_PAGE: &str = "/sign-in";

const MAX_RETURN_PATH: usize = 2048;

/// Where a redirect login that failed sends the browser: the sign-in page with the error.
#[must_use]
pub fn sign_in_failed(error: &ApiError) -> String {
    let message: String = form_urlencoded::byte_serialize(error.message().as_bytes()).collect();
    format!("{SIGN_IN_PAGE}?error={message}")
}

/// Where a redirect login comes back to: the `next` path asked for when it is a path on this
/// origin, else `/`.
#[must_use]
pub fn return_path(next: Option<&str>) -> String {
    match next {
        Some(path)
            if path.len() <= MAX_RETURN_PATH
                && path.starts_with('/')
                && !path.starts_with("//")
                && path.bytes().all(|b| b.is_ascii_graphic() && b != b'\\') =>
        {
            path.to_owned()
        }
        _ => "/".to_owned(),
    }
}

/// A provider's own routes, served under `provider_path(id)`, and what `GET /auth/providers`
/// shows about it next to its id.
pub struct ProviderRoutes {
    pub id: String,
    pub description: ProviderDescription,
    pub routes: Router,
}

/// What the sign-in page draws a provider from.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderDescription {
    pub label: String,
    pub kind: LoginKind,
    /// Where the login starts, as `kind` says.
    pub login: String,
    /// Where admins manage the provider's accounts, for a provider that keeps them.
    pub accounts: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LoginKind {
    /// The browser opens `login`, optionally with `?next=<path>`, and comes back to that path
    /// (else `/`) signed in, or to `SIGN_IN_PAGE` with `?error=<message>`.
    Redirect,
    /// `login` takes a JSON `{"username", "password"}` and answers with the user.
    Password,
}

#[derive(Clone, Serialize)]
pub(crate) struct ProviderListing {
    id: String,
    #[serde(flatten)]
    description: ProviderDescription,
}

#[derive(Clone)]
struct Mounted(Arc<str>);

impl ProviderRoutes {
    pub(crate) fn listing(&self) -> ProviderListing {
        ProviderListing {
            id: self.id.clone(),
            description: self.description.clone(),
        }
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

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::{return_path, sign_in_failed};
    use crate::error::ApiError;

    #[test]
    fn only_paths_on_this_origin_are_returned_to() {
        for path in ["/", "/sandboxes/main", "/admin/users?page=2#top", "/a%20b"] {
            assert_eq!(return_path(Some(path)), path);
        }
        for elsewhere in [
            None,
            Some(""),
            Some("sandboxes"),
            Some("//evil.example"),
            Some("/\\evil.example"),
            Some("https://evil.example/"),
            Some("/a b"),
            Some("/a\nb"),
            Some(&format!("/{}", "a".repeat(2048))),
        ] {
            assert_eq!(return_path(elsewhere), "/", "{elsewhere:?}");
        }
    }

    #[test]
    fn a_failed_sign_in_shows_its_error_on_the_sign_in_page() {
        let error = ApiError::new(StatusCode::FORBIDDEN, "no & such: user");
        assert_eq!(sign_in_failed(&error), "/sign-in?error=no+%26+such%3A+user");
    }
}
