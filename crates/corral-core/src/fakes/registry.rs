use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::lock;
use crate::domain::ImageReference;
use crate::ports::{ImageRegistry, RegistryAccess, RegistryLogins};
use crate::{Error, Result};

/// Registries holding the images given to `publish`, and nothing else, with no logins.
#[derive(Default)]
pub struct FakeRegistry {
    sizes: Mutex<BTreeMap<String, u64>>,
    reads: Mutex<Vec<String>>,
}

impl FakeRegistry {
    pub fn publish(&self, image: &str, download_size: u64) {
        let image = ImageReference::parse(image).unwrap().to_string();
        lock(&self.sizes).insert(image, download_size);
    }

    /// The images read, in full, oldest first.
    #[must_use]
    pub fn reads(&self) -> Vec<String> {
        lock(&self.reads).clone()
    }
}

#[async_trait]
impl ImageRegistry for FakeRegistry {
    async fn download_size(&self, image: &ImageReference) -> Result<u64> {
        lock(&self.reads).push(image.to_string());
        lock(&self.sizes)
            .get(image.as_str())
            .copied()
            .ok_or_else(|| Error::Invalid(format!("no image {image} in its registry")))
    }
}

#[async_trait]
impl RegistryLogins for FakeRegistry {
    async fn registries(&self) -> Result<Vec<RegistryAccess>> {
        Ok(Vec::new())
    }
}
