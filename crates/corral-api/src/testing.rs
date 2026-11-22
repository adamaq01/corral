//! A browser for driving the API in tests, an `Api` over a core test `World`, and a provider
//! that signs in whoever it is told.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::post;
use cookie::{Cookie, Key};
use corral_core::domain::{PictureUrl, Role};
use corral_core::fakes::World;
use corral_core::ports::Identity;
use serde::Deserialize;
use serde_json::Value;
use tower::ServiceExt;

use crate::error::ApiError;
use crate::extract::JsonBody;
use crate::provider::{
    Identities, LoginKind, ProviderDescription, ProviderRoutes, SignedIn, provider_path,
};
use crate::state::{Api, SshEndpoint};

/// An API over the world's services, with plain-HTTP cookies and no trusted proxy.
#[must_use]
pub fn api(world: &World) -> Api {
    Api {
        accounts: world.accounts.clone(),
        keys: world.keys.clone(),
        credentials: world.credentials.clone(),
        sandboxes: world.sandboxes.clone(),
        ssh: SshEndpoint {
            host: "corral.example.com".to_owned(),
            port: 48291,
        },
        session_key: Key::generate(),
        secure_cookies: false,
        trust_forwarded_for: false,
    }
}

#[derive(Deserialize)]
struct Trusted {
    subject: String,
    username: String,
    role: Option<Role>,
    #[serde(default)]
    picture: Option<PictureUrl>,
}

/// A provider whose `POST /login` signs in whoever `{"subject", "username", "role"}` names,
/// with a null role for a person outside every role, and any `"picture"`.
#[must_use]
pub fn trusting_provider(id: &str) -> ProviderRoutes {
    ProviderRoutes {
        id: id.to_owned(),
        description: ProviderDescription {
            label: "Trusting".to_owned(),
            kind: LoginKind::Password,
            login: format!("{}/login", provider_path(id)),
            accounts: None,
        },
        routes: Router::new().route("/login", post(trusted_login)),
    }
}

async fn trusted_login(
    identities: Identities,
    JsonBody(trusted): JsonBody<Trusted>,
) -> Result<SignedIn, ApiError> {
    identities
        .sign_in(Identity {
            subject: trusted.subject,
            username: trusted.username,
            email: None,
            picture: trusted.picture,
            role: trusted.role,
        })
        .await
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    #[must_use]
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.text()))
    }

    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    #[must_use]
    pub fn set_cookie(&self, name: &str) -> Cookie<'static> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| Cookie::parse(value.to_str().unwrap().to_owned()).unwrap())
            .find(|cookie| cookie.name() == name)
            .unwrap_or_else(|| panic!("no {name} cookie in {:?}", self.headers))
    }
}

/// Sends requests from one peer address and keeps the cookies it is sent, whatever their path.
pub struct Browser {
    pub app: Router,
    pub peer: SocketAddr,
    pub cookies: BTreeMap<String, String>,
}

impl Browser {
    #[must_use]
    pub fn new(app: &Router, peer: &str) -> Self {
        Self {
            app: app.clone(),
            peer: peer.parse().unwrap(),
            cookies: BTreeMap::new(),
        }
    }

    pub async fn send(&mut self, mut request: Request<Body>) -> Reply {
        if !self.cookies.is_empty() {
            let line = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            request
                .headers_mut()
                .insert(header::COOKIE, line.parse().unwrap());
        }
        request.extensions_mut().insert(ConnectInfo(self.peer));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        for value in parts.headers.get_all(header::SET_COOKIE) {
            let cookie = Cookie::parse(value.to_str().unwrap().to_owned()).unwrap();
            if cookie.value().is_empty() {
                self.cookies.remove(cookie.name());
            } else {
                self.cookies
                    .insert(cookie.name().to_owned(), cookie.value().to_owned());
            }
        }
        Reply {
            status: parts.status,
            headers: parts.headers,
            body: axum::body::to_bytes(body, usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        }
    }

    /// A JSON request; `POST` and `PUT` carry the JSON content type even without a body.
    pub async fn call(&mut self, method: &str, uri: &str, body: Option<Value>) -> Reply {
        let mut request = Request::builder().method(method).uri(uri);
        if matches!(method, "POST" | "PUT") {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        self.send(request.body(body).unwrap()).await
    }

    pub async fn get(&mut self, uri: &str) -> Reply {
        self.call("GET", uri, None).await
    }

    pub async fn post(&mut self, uri: &str, body: Value) -> Reply {
        self.call("POST", uri, Some(body)).await
    }
}
