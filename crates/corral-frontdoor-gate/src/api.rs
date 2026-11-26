//! The internal API between the gate and the server, on the server's internal listener. Every
//! request carries the shared secret as a bearer token.

use std::net::SocketAddr;

use corral_core::domain::Shared;
use serde::{Deserialize, Serialize};

pub(crate) const AUTHENTICATE: &str = "/gate/authenticate";
pub(crate) const ARRIVE: &str = "/gate/arrive";
pub(crate) const WORKED: &str = "/gate/worked";
pub(crate) const CLIENT_STATE: &str = "/gate/client-state";
pub(crate) const LOBBY: &str = "/gate/lobby";
pub(crate) const CHECK_IMAGE: &str = "/gate/check-image";
pub(crate) const CREATE: &str = "/gate/create";

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
    /// Wakes a sleeping sandbox, and makes a missing one; without it, a sleeping sandbox is
    /// left asleep and arrives `Asleep`.
    pub wake: bool,
    /// The client adapter whose kept state comes back with the sandbox.
    pub client: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Arrived {
    #[serde(rename_all = "camelCase")]
    Ready {
        sandbox: String,
        name: String,
        client_state: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Waking { name: String },
    #[serde(rename_all = "camelCase")]
    Asleep {
        sandbox: String,
        name: String,
        client_state: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Refused { reason: String },
}

/// Sandboxes a person or a client worked in through the gate since its last report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worked {
    pub sandboxes: Vec<String>,
}

/// What a client adapter keeps for a sandbox, in place of what it kept before.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientState {
    pub sandbox: String,
    pub client: String,
    pub state: String,
}

/// A login with a terminal, which may land in the lobby.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Enter {
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Entered {
    /// The login names a sandbox that exists: arriving goes on as usual.
    Sandbox,
    Lobby {
        offer: Offer,
    },
    Refused {
        reason: String,
    },
}

/// What a new sandbox may be made from, with the owner's defaults. Values never leave the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub handle: String,
    /// The name the login gives the new sandbox, if it gives one.
    pub name: Option<String>,
    pub images: Vec<ImageChoice>,
    /// Absent when sandboxes run only the catalog's images.
    pub named_images: Option<NamedImages>,
    pub sizes: Vec<SizeChoice>,
    pub default_image: String,
    pub default_size: String,
    pub entries: Vec<OfferedEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageChoice {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedImages {
    pub registries: Vec<String>,
    pub max_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeChoice {
    pub key: String,
    pub label: String,
    pub cpu_millis: u64,
    pub memory_bytes: u64,
    pub home_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Variable,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferedEntry {
    pub id: String,
    pub kind: EntryKind,
    /// The variable's name, or the file's path in the home.
    pub name: String,
    pub secret: bool,
    pub template: bool,
    pub apply_to_new: bool,
    /// How the admin shares it with everyone, for an entry that is not the member's own.
    pub shared: Option<Shared>,
}

impl OfferedEntry {
    /// Whether every sandbox gets it, whatever is chosen.
    pub(crate) fn required(&self) -> bool {
        self.shared == Some(Shared::Required)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckImage {
    pub reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageChecked {
    /// Why no sandbox can be made from the image, if none can.
    pub problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Create {
    /// `<sandbox>.<handle>`, naming a sandbox that does not exist yet.
    pub username: String,
    pub spec: Spec,
}

/// How a new sandbox is made, as `corral-new` reads it. Without an image or a size, the owner's
/// default; without `setup`, the entries that apply to new sandboxes; without `repository`,
/// nothing is cloned.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Spec {
    /// A catalog image's key, or else an image reference.
    pub image: Option<String>,
    pub size: Option<String>,
    /// Entries by id, or by the variable or file they set.
    pub setup: Option<Vec<String>>,
    pub repository: Option<RepositorySpec>,
}

/// A git repository the sandbox's first start clones, and the branch to check out, else its
/// default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositorySpec {
    pub url: String,
    pub branch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Created {
    Made { made: Made },
    Refused { reason: String },
}

/// A sandbox the lobby made, as `corral-new` prints it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Made {
    pub name: String,
    pub ssh_username: String,
    /// A catalog image's key, or the image reference it was made from.
    pub image: String,
    pub size: String,
    pub setup: Vec<MadeEntry>,
    pub repository: Option<RepositorySpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MadeEntry {
    pub id: String,
    pub kind: EntryKind,
    pub name: String,
}
