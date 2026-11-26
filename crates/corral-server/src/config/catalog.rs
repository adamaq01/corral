use std::num::NonZeroU64;

use anyhow::{Context, Result, ensure};
use corral_core::app::{Catalog, CatalogDefaults, ImageOption, NamedImages, SizePreset, Source};
use corral_core::domain::{Allocation, ImageKey, Resources, SizeKey};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use super::quantity;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Entries {
    images: Option<Vec<ImageEntry>>,
    sizes: Option<Vec<SizeEntry>>,
    default_image: Option<ImageKey>,
    default_size: Option<SizeKey>,
    /// Without it, sandboxes run the catalog's images alone.
    named_images: Option<NamedImagesEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedImagesEntry {
    registries: Vec<String>,
    #[serde(deserialize_with = "quantity::bytes")]
    max_size: NonZeroU64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageEntry {
    key: ImageKey,
    label: String,
    reference: String,
    #[serde(default)]
    description: String,
    #[serde(default = "enabled")]
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeEntry {
    key: SizeKey,
    label: String,
    #[serde(deserialize_with = "quantity::cpu_millis")]
    cpu_request: NonZeroU64,
    #[serde(deserialize_with = "quantity::cpu_millis")]
    cpu_limit: NonZeroU64,
    #[serde(deserialize_with = "quantity::bytes")]
    memory_request: NonZeroU64,
    #[serde(deserialize_with = "quantity::bytes")]
    memory_limit: NonZeroU64,
    #[serde(deserialize_with = "quantity::bytes")]
    home_size: NonZeroU64,
    #[serde(default = "enabled")]
    enabled: bool,
}

fn enabled() -> bool {
    true
}

impl Entries {
    fn catalog(self) -> Result<Catalog> {
        let builtin = Catalog::builtin();
        let images = match self.images {
            Some(images) => images.into_iter().map(ImageEntry::option).collect(),
            None => builtin.images().to_vec(),
        };
        let sizes = match self.sizes {
            Some(sizes) => sizes
                .into_iter()
                .map(|size| {
                    let key = size.key.clone();
                    size.preset()
                        .with_context(|| format!("size {key:?} is invalid"))
                })
                .collect::<Result<_>>()?,
            None => builtin.sizes().to_vec(),
        };
        let defaults = CatalogDefaults {
            image: match self.default_image {
                Some(key) => key,
                None => images
                    .iter()
                    .find(|image| image.enabled)
                    .map(|image| image.key.clone())
                    .context("the catalog offers no enabled image")?,
            },
            size: match self.default_size {
                Some(key) => key,
                None => sizes
                    .iter()
                    .find(|size| size.enabled)
                    .map(|size| size.key.clone())
                    .context("the catalog offers no enabled size")?,
            },
        };
        let catalog = Catalog::new(images, sizes, defaults)?;
        let Some(named) = self.named_images else {
            return Ok(catalog);
        };
        ensure!(
            !named.registries.is_empty(),
            "catalog.named_images lists no registry"
        );
        Ok(catalog.with_named_images(NamedImages {
            registries: named.registries,
            max_bytes: named.max_size,
        }))
    }
}

impl ImageEntry {
    fn option(self) -> ImageOption {
        ImageOption {
            key: self.key,
            label: self.label,
            reference: self.reference,
            description: self.description,
            enabled: self.enabled,
            source: Source::Config,
        }
    }
}

impl SizeEntry {
    fn preset(self) -> Result<SizePreset> {
        Ok(SizePreset {
            key: self.key,
            label: self.label,
            resources: Resources {
                cpu_millis: Allocation::new(self.cpu_request.get(), self.cpu_limit.get())?,
                memory_bytes: Allocation::new(self.memory_request.get(), self.memory_limit.get())?,
                home_bytes: self.home_size,
            },
            enabled: self.enabled,
            source: Source::Config,
        })
    }
}

pub(super) fn default() -> Catalog {
    Catalog::builtin()
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Catalog, D::Error> {
    Entries::deserialize(deserializer)?
        .catalog()
        .map_err(|error| D::Error::custom(format!("{error:#}")))
}
