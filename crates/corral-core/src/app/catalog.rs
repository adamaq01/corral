use std::collections::BTreeSet;
use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::domain::{Allocation, ImageKey, ImageReference, Resources, SizeKey};
use crate::{Error, Result};

/// Who owns a catalog entry: the config file, or the admin through the web UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Config,
    Database,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageOption {
    pub key: ImageKey,
    pub label: String,
    pub reference: String,
    pub description: String,
    pub enabled: bool,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizePreset {
    pub key: SizeKey,
    pub label: String,
    pub resources: Resources,
    pub enabled: bool,
    pub source: Source,
}

/// Images users may name themselves, beside the catalog's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedImages {
    /// Registries, such as `ghcr.io`, or repository paths in one, such as `ghcr.io/acme`.
    pub registries: Vec<String>,
    /// The most an image may take to download, its layers compressed.
    pub max_bytes: NonZeroU64,
}

impl NamedImages {
    /// Refuses an image from none of the registries.
    pub fn check(&self, image: &ImageReference) -> Result<()> {
        if self.registries.iter().any(|place| image.is_in(place)) {
            return Ok(());
        }
        Err(Error::Invalid(format!(
            "{image} is from none of the registries sandboxes may use: {}",
            self.registries.join(", ")
        )))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogDefaults {
    pub image: ImageKey,
    pub size: SizeKey,
}

/// The images and sizes a sandbox is made from. A disabled entry is not offered for new
/// sandboxes, but the sandboxes made from it still resolve it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    images: Vec<ImageOption>,
    sizes: Vec<SizePreset>,
    defaults: CatalogDefaults,
    named: Option<NamedImages>,
}

impl Catalog {
    pub fn new(
        images: Vec<ImageOption>,
        sizes: Vec<SizePreset>,
        defaults: CatalogDefaults,
    ) -> Result<Self> {
        check_entries(
            "image",
            images
                .iter()
                .map(|image| (image.key.as_str(), image.enabled)),
            defaults.image.as_str(),
        )?;
        check_entries(
            "size",
            sizes.iter().map(|size| (size.key.as_str(), size.enabled)),
            defaults.size.as_str(),
        )?;
        Ok(Self {
            images,
            sizes,
            defaults,
            named: None,
        })
    }

    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn builtin() -> Self {
        let image = ImageKey::parse("ubuntu").expect("a valid key");
        let size = SizeKey::parse("small").expect("a valid key");
        let resources = Resources {
            cpu_millis: Allocation::new(500, 2000).expect("a valid allocation"),
            memory_bytes: Allocation::new(1 << 30, 4 << 30).expect("a valid allocation"),
            home_bytes: const { NonZeroU64::new(10 << 30).unwrap() },
        };
        Self::new(
            vec![ImageOption {
                key: image.clone(),
                label: "Ubuntu 24.04".to_owned(),
                reference: "ghcr.io/adamaq01/corral-sandbox".to_owned(),
                description: String::new(),
                enabled: true,
                source: Source::Config,
            }],
            vec![SizePreset {
                key: size.clone(),
                label: "Small: 2 CPUs, 4 GiB".to_owned(),
                resources,
                enabled: true,
                source: Source::Config,
            }],
            CatalogDefaults { image, size },
        )
        .expect("the built-in catalog is valid")
    }

    /// The same catalog, letting users name images too.
    #[must_use]
    pub fn with_named_images(self, named: NamedImages) -> Self {
        Self {
            named: Some(named),
            ..self
        }
    }

    /// Whether and which images users may name, beside the catalog's.
    #[must_use]
    pub fn named_images(&self) -> Option<&NamedImages> {
        self.named.as_ref()
    }

    #[must_use]
    pub fn images(&self) -> &[ImageOption] {
        &self.images
    }

    #[must_use]
    pub fn sizes(&self) -> &[SizePreset] {
        &self.sizes
    }

    pub fn offered_images(&self) -> impl Iterator<Item = &ImageOption> {
        self.images.iter().filter(|image| image.enabled)
    }

    pub fn offered_sizes(&self) -> impl Iterator<Item = &SizePreset> {
        self.sizes.iter().filter(|size| size.enabled)
    }

    #[must_use]
    pub fn defaults(&self) -> &CatalogDefaults {
        &self.defaults
    }

    pub fn image(&self, key: Option<&ImageKey>) -> Result<&ImageOption> {
        pick(
            "image",
            &self.images,
            key.unwrap_or(&self.defaults.image),
            |image| &image.key,
        )
    }

    pub fn size(&self, key: Option<&SizeKey>) -> Result<&SizePreset> {
        pick(
            "size",
            &self.sizes,
            key.unwrap_or(&self.defaults.size),
            |size| &size.key,
        )
    }
}

fn pick<'a, T, K: PartialEq + std::fmt::Display>(
    kind: &str,
    options: &'a [T],
    key: &K,
    key_of: impl Fn(&T) -> &K,
) -> Result<&'a T> {
    options
        .iter()
        .find(|option| key_of(option) == key)
        .ok_or_else(|| Error::Invalid(format!("unknown {kind} \"{key}\"")))
}

fn check_entries<'a>(
    kind: &str,
    entries: impl Iterator<Item = (&'a str, bool)>,
    default: &str,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut enabled = BTreeSet::new();
    for (key, is_enabled) in entries {
        if !seen.insert(key) {
            return Err(Error::Invalid(format!("{kind} {key:?} is listed twice")));
        }
        if is_enabled {
            enabled.insert(key);
        }
    }
    if enabled.is_empty() {
        return Err(Error::Invalid(format!(
            "the catalog offers no enabled {kind}"
        )));
    }
    if !enabled.contains(default) {
        return Err(Error::Invalid(format!(
            "the default {kind} {default:?} is not an enabled {kind}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Catalog, CatalogDefaults, ImageOption, SizePreset};
    use crate::Error;
    use crate::domain::{ImageKey, SizeKey};
    use crate::fakes::catalog;

    fn image(key: &str) -> ImageKey {
        ImageKey::parse(key).unwrap()
    }

    fn size(key: &str) -> SizeKey {
        SizeKey::parse(key).unwrap()
    }

    #[test]
    fn new_requires_unique_keys_an_enabled_entry_of_each_and_enabled_defaults() {
        let c = catalog();
        let (images, sizes, defaults) = (c.images().to_vec(), c.sizes().to_vec(), c.defaults());
        let disabled_images: Vec<_> = images
            .iter()
            .map(|image| ImageOption {
                enabled: false,
                ..image.clone()
            })
            .collect();
        let large_disabled: Vec<_> = sizes
            .iter()
            .map(|preset| SizePreset {
                enabled: preset.key != size("large"),
                ..preset.clone()
            })
            .collect();
        let defaulting = |image_key: &str, size_key: &str| CatalogDefaults {
            image: image(image_key),
            size: size(size_key),
        };
        for (images, sizes, defaults, expected) in [
            (
                [images.clone(), images.clone()].concat(),
                sizes.clone(),
                defaults.clone(),
                "image \"base\" is listed twice",
            ),
            (
                images.clone(),
                [sizes.clone(), sizes.clone()].concat(),
                defaults.clone(),
                "size \"small\" is listed twice",
            ),
            (
                disabled_images,
                sizes.clone(),
                defaults.clone(),
                "the catalog offers no enabled image",
            ),
            (
                images.clone(),
                large_disabled,
                defaulting("base", "large"),
                "the default size \"large\" is not an enabled size",
            ),
            (
                images.clone(),
                sizes.clone(),
                defaulting("nope", "small"),
                "the default image \"nope\" is not an enabled image",
            ),
        ] {
            match Catalog::new(images, sizes, defaults) {
                Err(Error::Invalid(message)) => assert_eq!(message, expected),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_disabled_entry_still_resolves_by_key_but_is_not_offered() {
        let c = catalog();
        assert!(c.image(Some(&image("tools"))).is_ok());
        assert_eq!(
            c.offered_images()
                .map(|offered| offered.key.as_str())
                .collect::<Vec<_>>(),
            ["base"]
        );
        assert_eq!(c.image(None).unwrap().key.as_str(), "base");
        assert_eq!(
            c.offered_sizes()
                .map(|offered| offered.key.as_str())
                .collect::<Vec<_>>(),
            ["small", "large"]
        );
        assert!(matches!(
            c.image(Some(&image("nope"))),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn the_builtin_catalog_offers_ubuntu_small() {
        let c = Catalog::builtin();
        assert_eq!(
            c.image(None).unwrap().reference,
            "ghcr.io/adamaq01/corral-sandbox"
        );
        assert_eq!(c.size(None).unwrap().label, "Small: 2 CPUs, 4 GiB");
    }
}
