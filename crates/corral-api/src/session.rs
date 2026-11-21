use axum_extra::extract::cookie::PrivateCookieJar;
use chrono::{DateTime, TimeDelta, Utc};
use cookie::time::{Duration as CookieDuration, OffsetDateTime};
use cookie::{Cookie, CookieBuilder, Key, SameSite};
use corral_core::domain::UserId;
use corral_core::ports::LoginState;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const LOGIN_COOKIE: &str = "corral_login";
const SESSION_COOKIE: &str = "corral_session";
const LOGIN_TTL: CookieDuration = CookieDuration::minutes(10);
const SESSION_TTL: TimeDelta = TimeDelta::days(7);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub user_id: UserId,
    pub expires_at: DateTime<Utc>,
}

impl Session {
    #[must_use]
    pub fn new(user_id: UserId, now: DateTime<Utc>) -> Self {
        Self {
            user_id,
            expires_at: now + SESSION_TTL,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SessionPayload {
    user: Uuid,
    expires_at: DateTime<Utc>,
}

pub struct SessionCodec {
    key: Key,
    secure: bool,
}

impl SessionCodec {
    #[must_use]
    pub fn new(key: Key, secure: bool) -> Self {
        Self { key, secure }
    }

    #[must_use]
    pub fn encode_login(&self, state: &LoginState) -> Cookie<'static> {
        self.cookie(LOGIN_COOKIE, state.0.clone())
            .max_age(LOGIN_TTL)
            .build()
    }

    #[must_use]
    pub fn decode_login(jar: &PrivateCookieJar) -> Option<LoginState> {
        jar.get(LOGIN_COOKIE)
            .map(|cookie| LoginState(cookie.value().to_owned()))
    }

    #[must_use]
    pub fn encode_session(&self, session: &Session) -> Cookie<'static> {
        let payload = SessionPayload {
            user: session.user_id.0,
            expires_at: session.expires_at,
        };
        let expires = OffsetDateTime::from_unix_timestamp(session.expires_at.timestamp()).ok();
        self.cookie(SESSION_COOKIE, serde_json::json!(payload).to_string())
            .expires(expires)
            .build()
    }

    #[must_use]
    pub fn decode_session(jar: &PrivateCookieJar, now: DateTime<Utc>) -> Option<Session> {
        let cookie = jar.get(SESSION_COOKIE)?;
        let payload: SessionPayload = serde_json::from_str(cookie.value()).ok()?;
        (payload.expires_at > now).then_some(Session {
            user_id: UserId(payload.user),
            expires_at: payload.expires_at,
        })
    }

    pub(crate) fn key(&self) -> Key {
        self.key.clone()
    }

    pub(crate) fn login_removal() -> Cookie<'static> {
        Cookie::build(LOGIN_COOKIE).path("/").build()
    }

    pub(crate) fn session_removal() -> Cookie<'static> {
        Cookie::build(SESSION_COOKIE).path("/").build()
    }

    fn cookie(&self, name: &'static str, value: String) -> CookieBuilder<'static> {
        Cookie::build((name, value))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(self.secure)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::http::header::{COOKIE, SET_COOKIE};
    use axum::http::{HeaderMap, HeaderValue};
    use axum::response::IntoResponse;
    use axum_extra::extract::cookie::PrivateCookieJar;
    use chrono::{TimeDelta, TimeZone, Utc};
    use cookie::{Cookie, Key};
    use corral_core::domain::UserId;
    use corral_core::ports::LoginState;
    use uuid::Uuid;

    use super::{Session, SessionCodec};

    fn codec(key: &Key) -> SessionCodec {
        SessionCodec::new(key.clone(), true)
    }

    fn session() -> Session {
        Session::new(
            UserId(Uuid::new_v4()),
            Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap(),
        )
    }

    type Wire = BTreeMap<String, String>;

    fn sent(key: &Key, cookies: Vec<Cookie<'static>>) -> Wire {
        let jar = cookies
            .into_iter()
            .fold(PrivateCookieJar::new(key.clone()), PrivateCookieJar::add);
        jar.into_response()
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|header| {
                let cookie = Cookie::parse(header.to_str().unwrap().to_owned()).unwrap();
                (cookie.name().to_owned(), cookie.value().to_owned())
            })
            .collect()
    }

    fn received(key: &Key, cookies: &Wire) -> PrivateCookieJar {
        let line = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(&line).unwrap());
        PrivateCookieJar::from_headers(&headers, key.clone())
    }

    #[test]
    fn sessions_and_login_states_survive_the_round_trip() {
        let key = Key::generate();
        let session = session();
        let login = LoginState("pkce-verifier-and-nonce".to_owned());
        let wire = sent(
            &key,
            vec![
                codec(&key).encode_session(&session),
                codec(&key).encode_login(&login),
            ],
        );
        assert_eq!(
            wire.keys().collect::<Vec<_>>(),
            ["corral_login", "corral_session"]
        );
        assert!(
            wire.values()
                .all(|value| !value.contains("pkce") && !value.contains("expires")),
            "{wire:?}"
        );
        let jar = received(&key, &wire);
        assert_eq!(
            SessionCodec::decode_session(&jar, session.expires_at - TimeDelta::seconds(1)),
            Some(session)
        );
        assert_eq!(SessionCodec::decode_login(&jar), Some(login));
    }

    #[test]
    fn expired_session_is_rejected() {
        let key = Key::generate();
        let session = session();
        let jar = received(
            &key,
            &sent(&key, vec![codec(&key).encode_session(&session)]),
        );
        assert_eq!(SessionCodec::decode_session(&jar, session.expires_at), None);
        assert_eq!(
            SessionCodec::decode_session(&jar, session.expires_at + TimeDelta::days(30)),
            None
        );
    }

    #[test]
    fn tampered_cookie_is_rejected() {
        let key = Key::generate();
        let session = session();
        let before = session.expires_at - TimeDelta::hours(1);
        let wire = sent(
            &key,
            vec![
                codec(&key).encode_session(&session),
                codec(&key).encode_login(&LoginState("state".to_owned())),
            ],
        );
        assert!(SessionCodec::decode_session(&received(&key, &wire), before).is_some());

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
            serde_json::json!({"user": session.user_id.0, "expires_at": session.expires_at})
                .to_string(),
        )]);
        for forged in [flipped, swapped, plain] {
            let jar = received(&key, &forged);
            assert_eq!(
                SessionCodec::decode_session(&jar, before),
                None,
                "{forged:?}"
            );
        }
        let other_key = received(&Key::generate(), &wire);
        assert_eq!(SessionCodec::decode_session(&other_key, before), None);
        assert_eq!(SessionCodec::decode_login(&other_key), None);
    }
}
