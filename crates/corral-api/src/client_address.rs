use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::request::Parts;

use crate::error::ApiError;
use crate::state::AppState;

pub(crate) struct ClientAddress(pub IpAddr);

impl FromRequestParts<AppState> for ClientAddress {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        let forwarded = state
            .trust_forwarded_for
            .then(|| rightmost_forwarded_for(&parts.headers))
            .flatten();
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(peer)| peer.ip());
        std::future::ready(forwarded.or(peer).map(Self).ok_or_else(|| {
            ApiError::internal(&"no peer address: serve with into_make_service_with_connect_info")
        }))
    }
}

// The rightmost entry is the one the trusted proxy appended; anything left of it is whatever
// the client chose to send.
fn rightmost_forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .next_back()?
        .to_str()
        .ok()?
        .rsplit(',')
        .next()?
        .trim()
        .parse()
        .ok()
}
