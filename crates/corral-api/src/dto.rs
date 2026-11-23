use chrono::{DateTime, Utc};
use corral_core::app::{Catalog, SandboxView};
use corral_core::domain::{
    CredentialName, Handle, ImageKey, PictureUrl, Role, SandboxDefaults, SandboxName,
    SandboxStatus, SizeKey, SshKey, SshUsername, User, UserStatus,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::state::SshEndpoint;

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewKey {
    pub label: String,
    pub public_key: String,
}

#[derive(Deserialize, TS)]
pub(crate) struct CredentialValue {
    #[ts(type = "string")]
    pub value: SecretString,
}

#[derive(Serialize, TS)]
pub(crate) struct Me {
    handle: Handle,
    email: Option<String>,
    picture: Option<PictureUrl>,
    role: Role,
}

impl From<&User> for Me {
    fn from(user: &User) -> Self {
        Self {
            handle: user.handle.clone(),
            email: user.email.clone(),
            picture: user.picture.clone(),
            role: user.role,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct Choice {
    key: String,
    label: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "Catalog")]
pub(crate) struct CatalogDto {
    images: Vec<Choice>,
    sizes: Vec<Choice>,
    /// The signed-in user's default image.
    default_image: ImageKey,
    /// The signed-in user's default size.
    default_size: SizeKey,
    max_sandboxes: usize,
    ssh: SshEndpoint,
}

impl CatalogDto {
    pub(crate) fn new(
        catalog: &Catalog,
        defaults: SandboxDefaults,
        max_sandboxes: usize,
        ssh: &SshEndpoint,
    ) -> Self {
        let choice = |key: &str, label: &str| Choice {
            key: key.to_owned(),
            label: label.to_owned(),
        };
        let images: Vec<Choice> = catalog
            .images()
            .iter()
            .map(|image| choice(image.key.as_str(), &image.label))
            .collect();
        let sizes: Vec<Choice> = catalog
            .sizes()
            .iter()
            .map(|size| choice(size.key.as_str(), &size.label))
            .collect();
        Self {
            default_image: defaults.image,
            default_size: defaults.size,
            images,
            sizes,
            max_sandboxes,
            ssh: ssh.clone(),
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct SandboxSsh {
    username: SshUsername,
    #[serde(flatten)]
    endpoint: SshEndpoint,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "Sandbox")]
pub(crate) struct SandboxDto {
    name: SandboxName,
    image: ImageKey,
    size: SizeKey,
    is_default: bool,
    #[serde(flatten)]
    status: SandboxStatus,
    ssh: SandboxSsh,
    created_at: DateTime<Utc>,
    /// Runs without its owner's latest credential change, which a restart applies.
    stale_credentials: bool,
}

impl SandboxDto {
    pub(crate) fn new(view: SandboxView, ssh: &SshEndpoint) -> Self {
        let SandboxView {
            sandbox,
            owner,
            is_default,
            status,
            stale_credentials,
        } = view;
        Self {
            ssh: SandboxSsh {
                username: SshUsername {
                    handle: owner.handle,
                    sandbox: Some(sandbox.name.clone()),
                },
                endpoint: ssh.clone(),
            },
            name: sandbox.name,
            image: sandbox.image,
            size: sandbox.size,
            is_default,
            status,
            created_at: sandbox.created_at,
            stale_credentials,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct Owner {
    handle: Handle,
    picture: Option<PictureUrl>,
}

#[derive(Serialize, TS)]
pub(crate) struct OwnedSandbox {
    owner: Owner,
    #[serde(flatten)]
    sandbox: SandboxDto,
}

impl OwnedSandbox {
    pub(crate) fn new(view: SandboxView, ssh: &SshEndpoint) -> Self {
        Self {
            owner: Owner {
                handle: view.owner.handle.clone(),
                picture: view.owner.picture.clone(),
            },
            sandbox: SandboxDto::new(view, ssh),
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct Wake {
    pub ready: bool,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SshKey")]
pub(crate) struct KeyDto {
    id: Uuid,
    label: String,
    public_key: String,
    fingerprint: String,
    created_at: DateTime<Utc>,
}

impl From<SshKey> for KeyDto {
    fn from(key: SshKey) -> Self {
        Self {
            id: key.id,
            label: key.label,
            fingerprint: key.public_key.fingerprint().to_owned(),
            public_key: key.public_key.as_str().to_owned(),
            created_at: key.created_at,
        }
    }
}

#[derive(Serialize, TS)]
#[ts(rename = "Credential")]
pub(crate) struct CredentialDto {
    name: CredentialName,
}

impl From<CredentialName> for CredentialDto {
    fn from(name: CredentialName) -> Self {
        Self { name }
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "User")]
pub(crate) struct UserDto {
    id: Uuid,
    handle: Handle,
    email: Option<String>,
    picture: Option<PictureUrl>,
    role: Role,
    status: UserStatus,
    created_at: DateTime<Utc>,
}

impl From<User> for UserDto {
    fn from(user: User) -> Self {
        Self {
            id: user.id.0,
            role: user.role,
            handle: user.handle,
            email: user.email,
            picture: user.picture,
            status: user.status,
            created_at: user.created_at,
        }
    }
}
