use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::response::{IntoResponseParts, ResponseParts};
use axum_extra::extract::cookie::PrivateCookieJar;
use chrono::{DateTime, TimeDelta, Utc};
use cookie::time::{Duration as CookieDuration, OffsetDateTime};
use cookie::{Cookie, CookieBuilder, SameSite};
use corral_core::domain::UserId;
use corral_core::ports::PendingLogin;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::{Api, AppState};

const LOGIN_COOKIE: &str = "corral_login";
const SESSION_COOKIE: &str = "corral_session";
const LOGIN_TTL: CookieDuration = CookieDuration::minutes(10);
const SESSION_TTL: TimeDelta = TimeDelta::days(7);

/// The browser's signed-in user and its redirect login in progress, each in a private cookie.
/// Changes reach the browser when the session is part of the response.
pub(crate) struct Session {
    jar: PrivateCookieJar,
    secure: bool,
}

#[derive(Serialize, Deserialize)]
struct LoginPayload {
    provider: String,
    pending: String,
}

#[derive(Serialize, Deserialize)]
struct SessionPayload {
    user: Uuid,
    expires_at: DateTime<Utc>,
}

impl Session {
    pub(crate) fn from_headers(headers: &HeaderMap, api: &Api) -> Self {
        Self {
            jar: PrivateCookieJar::from_headers(headers, api.session_key.clone()),
            secure: api.secure_cookies,
        }
    }

    pub(crate) fn current(&self, now: DateTime<Utc>) -> Option<UserId> {
        let payload: SessionPayload = self.read(SESSION_COOKIE)?;
        (payload.expires_at > now).then_some(UserId(payload.user))
    }

    #[must_use]
    pub(crate) fn start(self, user: UserId, now: DateTime<Utc>) -> Self {
        let expires_at = now + SESSION_TTL;
        let payload = SessionPayload {
            user: user.0,
            expires_at,
        };
        let expires = OffsetDateTime::from_unix_timestamp(expires_at.timestamp()).ok();
        let cookie = self.cookie(SESSION_COOKIE, &payload).expires(expires);
        self.add(cookie)
    }

    #[must_use]
    pub(crate) fn end(self) -> Self {
        self.remove(SESSION_COOKIE)
    }

    #[must_use]
    pub(crate) fn start_login(self, provider: &str, pending: &PendingLogin) -> Self {
        let payload = LoginPayload {
            provider: provider.to_owned(),
            pending: pending.0.clone(),
        };
        let cookie = self.cookie(LOGIN_COOKIE, &payload).max_age(LOGIN_TTL);
        self.add(cookie)
    }

    /// The login in progress, only if it was started with `provider`.
    pub(crate) fn pending_login(&self, provider: &str) -> Option<PendingLogin> {
        let payload: LoginPayload = self.read(LOGIN_COOKIE)?;
        (payload.provider == provider).then_some(PendingLogin(payload.pending))
    }

    #[must_use]
    pub(crate) fn end_login(self) -> Self {
        self.remove(LOGIN_COOKIE)
    }

    fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Option<T> {
        serde_json::from_str(self.jar.get(name)?.value()).ok()
    }

    fn cookie(&self, name: &'static str, payload: &impl Serialize) -> CookieBuilder<'static> {
        Cookie::build((name, serde_json::json!(payload).to_string()))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(self.secure)
    }

    fn add(mut self, cookie: CookieBuilder<'static>) -> Self {
        self.jar = self.jar.add(cookie);
        self
    }

    fn remove(mut self, name: &'static str) -> Self {
        self.jar = self.jar.remove(Cookie::build(name).path("/"));
        self
    }
}

impl FromRequestParts<AppState> for Session {
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Infallible>> + Send {
        std::future::ready(Ok(Self::from_headers(&parts.headers, state)))
    }
}

impl IntoResponseParts for Session {
    type Error = Infallible;

    fn into_response_parts(self, parts: ResponseParts) -> Result<ResponseParts, Infallible> {
        self.jar.into_response_parts(parts)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::http::header::{COOKIE, SET_COOKIE};
    use axum::http::{HeaderMap, HeaderValue};
    use axum::response::IntoResponse;
    use axum_extra::extract::cookie::PrivateCookieJar;
    use chrono::{DateTime, TimeDelta, TimeZone, Utc};
    use cookie::{Cookie, Key};
    use corral_core::domain::UserId;
    use corral_core::ports::PendingLogin;
    use uuid::Uuid;

    use super::{SESSION_TTL, Session};

    type Wire = BTreeMap<String, String>;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
    }

    fn received(key: &Key, cookies: &Wire) -> Session {
        let line = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        let mut headers = HeaderMap::new();
        if !line.is_empty() {
            headers.insert(COOKIE, HeaderValue::from_str(&line).unwrap());
        }
        Session {
            jar: PrivateCookieJar::from_headers(&headers, key.clone()),
            secure: true,
        }
    }

    fn sent(session: Session) -> Wire {
        session
            .jar
            .into_response()
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|header| {
                let cookie = Cookie::parse(header.to_str().unwrap().to_owned()).unwrap();
                (cookie.name().to_owned(), cookie.value().to_owned())
            })
            .collect()
    }

    fn signed_in(key: &Key, user: UserId) -> Wire {
        let session = received(key, &Wire::new())
            .start(user, now())
            .start_login("oidc", &PendingLogin("pkce-verifier-and-nonce".to_owned()));
        sent(session)
    }

    #[test]
    fn sessions_and_logins_survive_the_round_trip_unreadable() {
        let key = Key::generate();
        let user = UserId(Uuid::new_v4());
        let wire = signed_in(&key, user);
        assert_eq!(
            wire.keys().collect::<Vec<_>>(),
            ["corral_login", "corral_session"]
        );
        assert!(
            wire.values()
                .all(|value| !value.contains("pkce") && !value.contains("expires")),
            "{wire:?}"
        );
        let session = received(&key, &wire);
        assert_eq!(session.current(now()), Some(user));
        assert_eq!(
            session.pending_login("oidc"),
            Some(PendingLogin("pkce-verifier-and-nonce".to_owned()))
        );
        assert_eq!(session.pending_login("other"), None);
    }

    #[test]
    fn a_session_expires() {
        let key = Key::generate();
        let session = received(&key, &signed_in(&key, UserId(Uuid::new_v4())));
        let expiry = now() + SESSION_TTL;
        assert!(session.current(expiry - TimeDelta::seconds(1)).is_some());
        assert_eq!(session.current(expiry), None);
        assert_eq!(session.current(expiry + TimeDelta::days(30)), None);
    }

    #[test]
    fn ending_removes_each_cookie() {
        let key = Key::generate();
        let wire = signed_in(&key, UserId(Uuid::new_v4()));
        let ended = sent(received(&key, &wire).end().end_login());
        assert_eq!(
            ended,
            Wire::from([
                ("corral_login".to_owned(), String::new()),
                ("corral_session".to_owned(), String::new()),
            ])
        );
    }

    #[test]
    fn tampered_cookies_are_rejected() {
        let key = Key::generate();
        let user = UserId(Uuid::new_v4());
        let wire = signed_in(&key, user);

        let mut flipped = wire.clone();
        let value = flipped.get_mut("corral_session").unwrap();
        let middle = value.len() / 2;
        let replacement = if &value[middle..=middle] == "A" {
            "B"
        } else {
            "A"
        };
        value.replace_range(middle..=middle, replacement);
        let swapped = Wire::from([
            ("corral_session".to_owned(), wire["corral_login"].clone()),
            ("corral_login".to_owned(), wire["corral_session"].clone()),
        ]);
        let plain = Wire::from([(
            "corral_session".to_owned(),
            serde_json::json!({"user": user.0, "expires_at": now() + SESSION_TTL}).to_string(),
        )]);
        for forged in [flipped, swapped, plain] {
            assert_eq!(received(&key, &forged).current(now()), None, "{forged:?}");
        }
        let other_key = received(&Key::generate(), &wire);
        assert_eq!(other_key.current(now()), None);
        assert_eq!(other_key.pending_login("oidc"), None);
    }
}
