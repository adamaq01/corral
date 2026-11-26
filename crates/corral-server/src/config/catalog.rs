use std::collections::BTreeSet;
use std::num::NonZeroU64;

use anyhow::{Context, Result, bail, ensure};
use corral_core::app::{CatalogDefaults, ImageOption, NamedImages, SizePreset, Source};
use corral_core::domain::{Allocation, ImageKey, Resources, SizeKey};
use serde::Deserialize;

use super::quantity;

/// What `[catalog]` sets, which settings show locked. Settings fill in the rest.
#[derive(Default, Deserialize)]
#[serde(try_from = "Entries")]
pub struct ConfiguredCatalog {
    pub images: Vec<ImageOption>,
    pub sizes: Vec<SizePreset>,
    pub defaults: Option<CatalogDefaults>,
    pub named_images: Option<NamedImages>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Entries {
    images: Vec<ImageEntry>,
    sizes: Vec<SizeEntry>,
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

impl TryFrom<Entries> for ConfiguredCatalog {
    type Error = String;

    fn try_from(entries: Entries) -> Result<Self, String> {
        entries.configured().map_err(|error| format!("{error:#}"))
    }
}

impl Entries {
    /// Refuses only what no entry the database adds could make valid, since a configured entry
    /// shadows a stored one with its key.
    fn configured(self) -> Result<ConfiguredCatalog> {
        let images: Vec<ImageOption> = self.images.into_iter().map(ImageEntry::option).collect();
        let sizes = self
            .sizes
            .into_iter()
            .map(|size| {
                let key = size.key.clone();
                size.preset()
                    .with_context(|| format!("size {key:?} is invalid"))
            })
            .collect::<Result<Vec<_>>>()?;
        once("image", images.iter().map(|image| image.key.as_str()))?;
        once("size", sizes.iter().map(|size| size.key.as_str()))?;
        let defaults = match (self.default_image, self.default_size) {
            (Some(image), Some(size)) => {
                ensure!(
                    !images
                        .iter()
                        .any(|entry| entry.key == image && !entry.enabled),
                    "the default image \"{image}\" is not an enabled image"
                );
                ensure!(
                    !sizes
                        .iter()
                        .any(|entry| entry.key == size && !entry.enabled),
                    "the default size \"{size}\" is not an enabled size"
                );
                Some(CatalogDefaults { image, size })
            }
            (None, None) => None,
            _ => bail!("default_image and default_size go together"),
        };
        let named_images = match self.named_images {
            Some(named) => {
                ensure!(
                    !named.registries.is_empty(),
                    "catalog.named_images lists no registry"
                );
                Some(NamedImages {
                    registries: named.registries,
                    max_bytes: named.max_size,
                })
            }
            None => None,
        };
        Ok(ConfiguredCatalog {
            images,
            sizes,
            defaults,
            named_images,
        })
    }
}

fn once<'a>(kind: &str, keys: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for key in keys {
        ensure!(seen.insert(key), "{kind} {key:?} is listed twice");
    }
    Ok(())
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
