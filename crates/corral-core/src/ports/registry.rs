use async_trait::async_trait;
use secrecy::SecretString;

use crate::Result;
use crate::domain::ImageReference;

/// What Corral reads of an image in its registry, with the registry credentials Corral holds.
#[async_trait]
pub trait ImageRegistry: Send + Sync {
    /// What the image takes to download: its layers, compressed, for one platform.
    async fn download_size(&self, image: &ImageReference) -> Result<u64>;
}

/// The registries Corral holds credentials for, read at each use.
#[async_trait]
pub trait RegistryLogins: Send + Sync {
    async fn registries(&self) -> Result<Vec<RegistryAccess>>;
}

/// How Corral reaches a registry an admin configured: its credentials, which users never see, and
/// whether it serves plain HTTP.
#[derive(Clone, Debug)]
pub struct RegistryAccess {
    /// The registry's host, with its port if it has one, such as `registry.example.lan:5000`.
    pub registry: String,
    pub login: Option<RegistryLogin>,
    pub plain_http: bool,
}

#[derive(Clone, Debug)]
pub struct RegistryLogin {
    pub username: String,
    pub password: SecretString,
}

impl RegistryAccess {
    /// The access configured for the registry `image` is in, if any.
    #[must_use]
    pub fn of<'a>(all: &'a [Self], image: &ImageReference) -> Option<&'a Self> {
        all.iter()
            .find(|access| access.registry.eq_ignore_ascii_case(image.registry()))
    }
}
