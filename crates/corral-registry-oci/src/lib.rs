//! Reads images from their registries over the OCI distribution API, as `oci-client` speaks it.

use std::sync::Arc;

use async_trait::async_trait;
use corral_core::domain::ImageReference;
use corral_core::ports::{ImageRegistry, RegistryAccess, RegistryLogins};
use corral_core::{Error, Result};
use oci_client::client::{ClientConfig, ClientProtocol};
use oci_client::secrets::RegistryAuth;
use oci_client::{Client, Reference};
use secrecy::ExposeSecret;

pub struct OciRegistry {
    logins: Arc<dyn RegistryLogins>,
}

impl OciRegistry {
    /// Reaches each registry `logins` names as it says at the time of the read, and every other
    /// one anonymously over HTTPS. An image index resolves to Corral's own platform.
    #[must_use]
    pub fn new(logins: Arc<dyn RegistryLogins>) -> Self {
        Self { logins }
    }
}

// Made for each read, which only a named image's creation needs: `Client::new` panics where the
// system has no CA certificates, and Corral must start there all the same.
fn client(registries: &[RegistryAccess]) -> Result<Client> {
    let plain_http = registries
        .iter()
        .filter(|access| access.plain_http)
        .map(|access| access.registry.clone())
        .collect();
    let config = ClientConfig {
        protocol: ClientProtocol::HttpsExcept(plain_http),
        ..ClientConfig::default()
    };
    Client::try_from(config).map_err(|err| Error::internal("make a registry client", err))
}

#[async_trait]
impl ImageRegistry for OciRegistry {
    async fn download_size(&self, image: &ImageReference) -> Result<u64> {
        let reference: Reference = image
            .as_str()
            .parse()
            .map_err(|err| Error::internal(format!("read the reference {image}"), err))?;
        let registries = self.logins.registries().await?;
        let auth = match RegistryAccess::of(&registries, image).and_then(|a| a.login.as_ref()) {
            Some(login) => RegistryAuth::Basic(
                login.username.clone(),
                login.password.expose_secret().to_owned(),
            ),
            None => RegistryAuth::Anonymous,
        };
        let (manifest, _) = client(&registries)?
            .pull_image_manifest(&reference, &auth)
            .await
            .map_err(|err| {
                Error::Invalid(format!("cannot read {image} from its registry: {err}"))
            })?;
        let size: i64 = manifest
            .layers
            .iter()
            .map(|layer| layer.size)
            .chain([manifest.config.size])
            .sum();
        u64::try_from(size)
            .map_err(|_| Error::internal(format!("the manifest of {image}"), "has a negative size"))
    }
}
