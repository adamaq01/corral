use std::convert::Infallible;

use axum::http::HeaderMap;
use axum::response::{IntoResponseParts, ResponseParts};
use axum_extra::extract::cookie::PrivateCookieJar;
use cookie::{Cookie, CookieBuilder, Key, SameSite};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The request's encrypted cookies. Changes reach the browser when the jar is part of the
/// response.
pub(crate) struct Jar {
    jar: PrivateCookieJar,
    secure: bool,
}

impl Jar {
    pub(crate) fn from_headers(headers: &HeaderMap, key: &Key, secure: bool) -> Self {
        Self {
            jar: PrivateCookieJar::from_headers(headers, key.clone()),
            secure,
        }
    }

    pub(crate) fn read<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        serde_json::from_str(self.jar.get(name)?.value()).ok()
    }

    pub(crate) fn cookie(
        &self,
        name: String,
        path: String,
        payload: &impl Serialize,
    ) -> CookieBuilder<'static> {
        Cookie::build((name, serde_json::json!(payload).to_string()))
            .path(path)
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(self.secure)
    }

    #[must_use]
    pub(crate) fn add(mut self, cookie: CookieBuilder<'static>) -> Self {
        self.jar = self.jar.add(cookie);
        self
    }

    #[must_use]
    pub(crate) fn remove(mut self, name: String, path: String) -> Self {
        self.jar = self.jar.remove(Cookie::build(name).path(path));
        self
    }
}

impl IntoResponseParts for Jar {
    type Error = Infallible;

    fn into_response_parts(self, parts: ResponseParts) -> Result<ResponseParts, Infallible> {
        self.jar.into_response_parts(parts)
    }
}
