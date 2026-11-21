use std::collections::BTreeSet;

use crate::ports::{IdlePolicy, Resources};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub images: Vec<ImageOption>,
    pub default_image: String,
    pub sizes: Vec<SizePreset>,
    pub default_size: String,
    pub ssh: SshEndpoint,
    pub idle: IdlePolicy,
}

impl Catalog {
    pub fn validate(&self) -> Result<()> {
        check_keys(
            "image",
            self.images.iter().map(|image| image.key.as_str()),
            &self.default_image,
        )?;
        check_keys(
            "size",
            self.sizes.iter().map(|size| size.key.as_str()),
            &self.default_size,
        )
    }

    pub fn image(&self, key: Option<&str>) -> Result<&ImageOption> {
        let key = key.unwrap_or(&self.default_image);
        self.images
            .iter()
            .find(|image| image.key == key)
            .ok_or_else(|| unknown("image", key))
    }

    pub fn size(&self, key: Option<&str>) -> Result<&SizePreset> {
        let key = key.unwrap_or(&self.default_size);
        self.sizes
            .iter()
            .find(|size| size.key == key)
            .ok_or_else(|| unknown("size", key))
    }
}

fn check_keys<'a>(kind: &str, keys: impl Iterator<Item = &'a str>, default: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            return Err(Error::Invalid(format!("{kind} {key:?} is listed twice")));
        }
    }
    if seen.contains(default) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "default {kind} {default:?} is not in the catalog"
        )))
    }
}

fn unknown(kind: &str, key: &str) -> Error {
    Error::Invalid(format!("unknown {kind} {key:?}"))
}

#[cfg(test)]
mod tests {
    use super::Catalog;
    use crate::Error;
    use crate::testing::catalog;

    #[test]
    fn validate_requires_listed_defaults_and_unique_keys() {
        assert!(catalog().validate().is_ok());
        let with_duplicate_image = {
            let mut c = catalog();
            c.images.push(c.images[0].clone());
            c
        };
        let with_duplicate_size = {
            let mut c = catalog();
            c.sizes.push(c.sizes[1].clone());
            c
        };
        for broken in [
            Catalog {
                default_image: "missing".to_owned(),
                ..catalog()
            },
            Catalog {
                default_size: "missing".to_owned(),
                ..catalog()
            },
            Catalog {
                sizes: vec![],
                ..catalog()
            },
            with_duplicate_image,
            with_duplicate_size,
        ] {
            assert!(
                matches!(broken.validate(), Err(Error::Invalid(_))),
                "{broken:?}"
            );
        }
    }

    #[test]
    fn image_and_size_fall_back_to_defaults_and_reject_unknown_keys() {
        let c = catalog();
        assert_eq!(
            c.image(None).unwrap().reference,
            "ghcr.io/adamaq01/corral-sandbox:1"
        );
        assert_eq!(
            c.image(Some("tools")).unwrap().reference,
            "ghcr.io/adamaq01/corral-tools:2"
        );
        assert_eq!(c.size(None).unwrap().resources.memory_limit, "2Gi");
        assert_eq!(c.size(Some("large")).unwrap().resources.memory_limit, "8Gi");
        assert!(matches!(c.image(Some("nope")), Err(Error::Invalid(_))));
        assert!(matches!(c.size(Some("nope")), Err(Error::Invalid(_))));
    }
}
