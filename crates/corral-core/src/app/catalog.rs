use std::collections::BTreeSet;

use crate::domain::Resources;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageOption {
    pub key: String,
    pub label: String,
    pub reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizePreset {
    pub key: String,
    pub label: String,
    pub resources: Resources,
}

/// The images and sizes a sandbox is made from. The first of each is the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    images: Vec<ImageOption>,
    sizes: Vec<SizePreset>,
}

impl Catalog {
    pub fn new(images: Vec<ImageOption>, sizes: Vec<SizePreset>) -> Result<Self> {
        check_keys("image", images.iter().map(|image| image.key.as_str()))?;
        check_keys("size", sizes.iter().map(|size| size.key.as_str()))?;
        Ok(Self { images, sizes })
    }

    #[must_use]
    pub fn images(&self) -> &[ImageOption] {
        &self.images
    }

    #[must_use]
    pub fn sizes(&self) -> &[SizePreset] {
        &self.sizes
    }

    pub fn image(&self, key: Option<&str>) -> Result<&ImageOption> {
        pick("image", &self.images, key, |image| &image.key)
    }

    pub fn size(&self, key: Option<&str>) -> Result<&SizePreset> {
        pick("size", &self.sizes, key, |size| &size.key)
    }
}

fn pick<'a, T>(
    kind: &str,
    options: &'a [T],
    key: Option<&str>,
    key_of: impl Fn(&T) -> &String,
) -> Result<&'a T> {
    let Some(key) = key else {
        return Ok(&options[0]);
    };
    options
        .iter()
        .find(|option| key_of(option) == key)
        .ok_or_else(|| Error::Invalid(format!("unknown {kind} {key:?}")))
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
    use crate::testing::catalog;

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
        assert_eq!(
            c.image(Some("tools")).unwrap().reference,
            "ghcr.io/adamaq01/corral-tools:2"
        );
        assert_eq!(c.size(None).unwrap().key, "small");
        assert_eq!(c.size(Some("large")).unwrap().key, "large");
        assert!(matches!(c.image(Some("nope")), Err(Error::Invalid(_))));
        assert!(matches!(c.size(Some("nope")), Err(Error::Invalid(_))));
    }
}
