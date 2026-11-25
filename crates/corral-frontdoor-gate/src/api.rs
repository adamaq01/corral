//! The internal API between the gate and the server, on the server's internal listener. Every
//! request carries the shared secret as a bearer token.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

pub(crate) const AUTHENTICATE: &str = "/gate/authenticate";
pub(crate) const ARRIVE: &str = "/gate/arrive";

/// A credential presented for a login. Unproven, it is a key offered before its signature, which
/// only says whether the client may go on to prove it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Authenticate {
    pub username: String,
    /// A public key or a certificate, as `<type> <base64>`.
    pub credential: String,
    pub proven: bool,
    pub client_address: SocketAddr,
    /// The same for every attempt of one connection, which counts once against its network.
    pub connection: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Authenticated {
    pub accepted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arrive {
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Arrived {
    #[serde(rename_all = "camelCase")]
    Ready { sandbox: String, name: String },
    #[serde(rename_all = "camelCase")]
    Waking { name: String },
    #[serde(rename_all = "camelCase")]
    Refused { reason: String },
}
