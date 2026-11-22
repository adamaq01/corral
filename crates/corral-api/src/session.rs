use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponseParts, ResponseParts};
use chrono::{DateTime, TimeDelta, Utc};
use cookie::time::OffsetDateTime;
use corral_core::domain::UserId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cookies::Jar;
use crate::error::ApiError;
use crate::state::{Api, shared};

const SESSION_COOKIE: &str = "corral_session";
const SESSION_TTL: TimeDelta = TimeDelta::days(7);

/// The browser's signed-in user, in a private cookie. Changes reach the browser when the
/// session is part of the response.
pub(crate) struct Session {
    jar: Jar,
}

#[derive(Serialize, Deserialize)]
struct SessionPayload {
    user: Uuid,
    expires_at: DateTime<Utc>,
}

impl Session {
    pub(crate) fn from_parts(parts: &Parts, api: &Api) -> Self {
        Self {
            jar: Jar::from_headers(&parts.headers, &api.session_key, api.secure_cookies),
        }
    }

    pub(crate) fn current(&self, now: DateTime<Utc>) -> Option<UserId> {
        let payload: SessionPayload = self.jar.read(SESSION_COOKIE)?;
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
        let cookie = self
            .jar
            .cookie(SESSION_COOKIE.to_owned(), "/".to_owned(), &payload)
            .expires(expires);
        Self {
            jar: self.jar.add(cookie),
        }
    }

    #[must_use]
    pub(crate) fn end(self) -> Self {
        Self {
            jar: self.jar.remove(SESSION_COOKIE.to_owned(), "/".to_owned()),
        }
    }
}

impl Session {
    fn extract(parts: &Parts) -> Result<Self, ApiError> {
        let api = shared(parts)?;
        Ok(Self::from_parts(parts, &api))
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Session {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(Self::extract(parts))
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
    use chrono::{DateTime, TimeDelta, TimeZone, Utc};
    use cookie::{Cookie, Key};
    use corral_core::domain::UserId;
    use uuid::Uuid;

    use super::{SESSION_TTL, Session};
    use crate::cookies::Jar;

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
            jar: Jar::from_headers(&headers, key, true),
        }
    }

    fn sent(session: Session) -> Wire {
        (session, ())
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
        sent(received(key, &Wire::new()).start(user, now()))
    }

    #[test]
    fn sessions_survive_the_round_trip_unreadable() {
        let key = Key::generate();
        let user = UserId(Uuid::new_v4());
        let wire = signed_in(&key, user);
        assert_eq!(wire.keys().collect::<Vec<_>>(), ["corral_session"]);
        assert!(
            wire.values()
                .all(|value| !value.contains(&user.0.to_string()) && !value.contains("expires")),
            "{wire:?}"
        );
        assert_eq!(received(&key, &wire).current(now()), Some(user));
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
    fn ending_removes_the_cookie() {
        let key = Key::generate();
        let wire = signed_in(&key, UserId(Uuid::new_v4()));
        let ended = sent(received(&key, &wire).end());
        assert_eq!(
            ended,
            Wire::from([("corral_session".to_owned(), String::new())])
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
        let plain = Wire::from([(
            "corral_session".to_owned(),
            serde_json::json!({"user": user.0, "expires_at": now() + SESSION_TTL}).to_string(),
        )]);
        for forged in [flipped, plain] {
            assert_eq!(received(&key, &forged).current(now()), None, "{forged:?}");
        }
        assert_eq!(received(&Key::generate(), &wire).current(now()), None);
    }
}
