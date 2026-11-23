use std::collections::BTreeSet;
use std::num::NonZeroU64;

use serde::Deserialize;

use crate::domain::{ImageKey, ImageReference, Resources, SizeKey};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageOption {
    pub key: ImageKey,
    pub label: String,
    pub reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizePreset {
    pub key: SizeKey,
    pub label: String,
    pub resources: Resources,
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

/// The images and sizes a sandbox is made from. The first of each is the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    images: Vec<ImageOption>,
    sizes: Vec<SizePreset>,
    named: Option<NamedImages>,
}

impl Catalog {
    pub fn new(images: Vec<ImageOption>, sizes: Vec<SizePreset>) -> Result<Self> {
        check_keys("image", images.iter().map(|image| image.key.as_str()))?;
        check_keys("size", sizes.iter().map(|size| size.key.as_str()))?;
        Ok(Self {
            images,
            sizes,
            named: None,
        })
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

    pub fn image(&self, key: Option<&ImageKey>) -> Result<&ImageOption> {
        pick("image", &self.images, key, |image| &image.key)
    }

    pub fn size(&self, key: Option<&SizeKey>) -> Result<&SizePreset> {
        pick("size", &self.sizes, key, |size| &size.key)
    }
}

fn pick<'a, T, K: PartialEq + std::fmt::Display>(
    kind: &str,
    options: &'a [T],
    key: Option<&K>,
    key_of: impl Fn(&T) -> &K,
) -> Result<&'a T> {
    let Some(key) = key else {
        return Ok(&options[0]);
    };
    options
        .iter()
        .find(|option| key_of(option) == key)
        .ok_or_else(|| Error::Invalid(format!("unknown {kind} \"{key}\"")))
}

fn check_keys<'a>(kind: &str, keys: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            return Err(Error::Invalid(format!("{kind} {key:?} is listed twice")));
        }
    }
    if seen.is_empty() {
        return Err(Error::Invalid(format!("the catalog lists no {kind}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Catalog;
    use crate::Error;
    use crate::domain::{ImageKey, SizeKey};
    use crate::fakes::catalog;

    #[test]
    fn new_requires_unique_keys_and_at_least_one_of_each() {
        let c = catalog();
        let (images, sizes) = (c.images().to_vec(), c.sizes().to_vec());
        let images_twice = [images.clone(), images.clone()].concat();
        let sizes_twice = [sizes.clone(), sizes.clone()].concat();
        for (images, sizes) in [
            (images_twice, sizes.clone()),
            (images.clone(), sizes_twice),
            (vec![], sizes.clone()),
            (images.clone(), vec![]),
        ] {
            assert!(
                matches!(Catalog::new(images, sizes), Err(Error::Invalid(_))),
                "accepted"
            );
        }
    }

    #[test]
    fn image_and_size_default_to_the_first_and_reject_unknown_keys() {
        let c = catalog();
        assert_eq!(
            c.image(None).unwrap().reference,
            "ghcr.io/adamaq01/corral-sandbox:1"
        );
        let image = |key: &str| ImageKey::parse(key).unwrap();
        let size = |key: &str| SizeKey::parse(key).unwrap();
        assert_eq!(
            c.image(Some(&image("tools"))).unwrap().reference,
            "ghcr.io/adamaq01/corral-tools:2"
        );
        assert_eq!(c.size(None).unwrap().key, size("small"));
        assert_eq!(c.size(Some(&size("large"))).unwrap().key, size("large"));
        assert!(matches!(
            c.image(Some(&image("nope"))),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            c.size(Some(&size("nope"))),
            Err(Error::Invalid(_))
        ));
    }
}
