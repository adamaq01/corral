use chrono::{DateTime, Utc};
use corral_core::app::{Catalog, SandboxView};
use corral_core::domain::{
    CredentialName, Handle, ImageKey, Role, SandboxName, SandboxStatus, SizeKey, SshKey,
    SshUsername, User, UserStatus,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::SshEndpoint;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewKey {
    pub label: String,
    pub public_key: String,
}

#[derive(Deserialize)]
pub(crate) struct CredentialValue {
    pub value: SecretString,
}

#[derive(Serialize)]
pub(crate) struct Me {
    handle: Handle,
    email: Option<String>,
    role: Role,
}

impl From<&User> for Me {
    fn from(user: &User) -> Self {
        Self {
            handle: user.handle.clone(),
            email: user.email.clone(),
            role: user.role,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct Choice {
    key: String,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogDto {
    images: Vec<Choice>,
    sizes: Vec<Choice>,
    default_image: String,
    default_size: String,
    ssh: SshEndpoint,
}

impl CatalogDto {
    pub(crate) fn new(catalog: &Catalog, ssh: &SshEndpoint) -> Self {
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
            default_image: images[0].key.clone(),
            default_size: sizes[0].key.clone(),
            images,
            sizes,
            ssh: ssh.clone(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct SandboxSsh {
    username: SshUsername,
    #[serde(flatten)]
    endpoint: SshEndpoint,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SandboxDto {
    name: SandboxName,
    image: ImageKey,
    size: SizeKey,
    is_default: bool,
    #[serde(flatten)]
    status: SandboxStatus,
    ssh: SandboxSsh,
}

impl SandboxDto {
    pub(crate) fn new(view: SandboxView, ssh: &SshEndpoint) -> Self {
        let SandboxView {
            sandbox,
            owner,
            is_default,
            status,
        } = view;
        Self {
            ssh: SandboxSsh {
                username: SshUsername {
                    handle: owner,
                    sandbox: Some(sandbox.name.clone()),
                },
                endpoint: ssh.clone(),
            },
            name: sandbox.name,
            image: sandbox.image,
            size: sandbox.size,
            is_default,
            status,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct OwnedSandbox {
    owner: Handle,
    #[serde(flatten)]
    sandbox: SandboxDto,
}

impl OwnedSandbox {
    pub(crate) fn new(view: SandboxView, ssh: &SshEndpoint) -> Self {
        Self {
            owner: view.owner.clone(),
            sandbox: SandboxDto::new(view, ssh),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct Wake {
    pub ready: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize)]
pub(crate) struct CredentialDto {
    name: CredentialName,
}

impl From<CredentialName> for CredentialDto {
    fn from(name: CredentialName) -> Self {
        Self { name }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserDto {
    id: Uuid,
    handle: Handle,
    email: Option<String>,
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
            status: user.status,
            created_at: user.created_at,
        }
    }
}
