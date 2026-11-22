use std::num::NonZeroU64;

use anyhow::{Context, Result};
use corral_core::app::{Catalog, ImageOption, SizePreset};
use corral_core::domain::{Allocation, Resources};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use super::quantity;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Entries {
    images: Vec<ImageOption>,
    sizes: Vec<SizeEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeEntry {
    key: String,
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
}

impl Default for Entries {
    fn default() -> Self {
        Self {
            images: vec![ImageOption {
                key: "ubuntu".to_owned(),
                label: "Ubuntu 24.04".to_owned(),
                reference: "ghcr.io/adamaq01/corral-sandbox".to_owned(),
            }],
            sizes: vec![SizeEntry {
                key: "small".to_owned(),
                label: "Small: 2 CPUs, 4 GiB".to_owned(),
                cpu_request: const { NonZeroU64::new(500).unwrap() },
                cpu_limit: const { NonZeroU64::new(2000).unwrap() },
                memory_request: const { NonZeroU64::new(1 << 30).unwrap() },
                memory_limit: const { NonZeroU64::new(4 << 30).unwrap() },
                home_size: const { NonZeroU64::new(10 << 30).unwrap() },
            }],
        }
    }
}

impl Entries {
    fn catalog(self) -> Result<Catalog> {
        let sizes = self
            .sizes
            .into_iter()
            .map(|size| {
                let key = size.key.clone();
                size.preset()
                    .with_context(|| format!("size {key:?} is invalid"))
            })
            .collect::<Result<_>>()?;
        Ok(Catalog::new(self.images, sizes)?)
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
        })
    }
}

pub(super) fn default() -> Catalog {
    Entries::default()
        .catalog()
        .expect("the default catalog is valid")
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Catalog, D::Error> {
    Entries::deserialize(deserializer)?
        .catalog()
        .map_err(|error| D::Error::custom(format!("{error:#}")))
}
