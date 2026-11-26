use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::lock;
use crate::Result;
use crate::app::{
    Allowance, CatalogChange, ImageOption, LimitsChange, Registry, SizePreset, Source,
};
use crate::domain::UserId;
use crate::ports::SettingsRepo;

#[derive(Default)]
pub struct MemorySettingsRepo {
    catalog: Mutex<Option<CatalogChange>>,
    limits: Mutex<LimitsChange>,
    registries: Mutex<Vec<Registry>>,
    allowances: Mutex<BTreeMap<UserId, Allowance>>,
}

impl MemorySettingsRepo {
    #[must_use]
    pub fn holding(catalog: &CatalogChange, limits: LimitsChange) -> Self {
        Self {
            catalog: Mutex::new(Some(stored(catalog))),
            limits: Mutex::new(limits),
            ..Self::default()
        }
    }
}

fn stored(change: &CatalogChange) -> CatalogChange {
    CatalogChange {
        images: change
            .images
            .iter()
            .map(|image| ImageOption {
                source: Source::Database,
                ..image.clone()
            })
            .collect(),
        sizes: change
            .sizes
            .iter()
            .map(|size| SizePreset {
                source: Source::Database,
                ..size.clone()
            })
            .collect(),
        defaults: change.defaults.clone(),
        named_images: change.named_images.clone(),
    }
}

#[async_trait]
impl SettingsRepo for MemorySettingsRepo {
    async fn catalog(&self) -> Result<Option<CatalogChange>> {
        Ok(lock(&self.catalog).clone())
    }

    async fn set_catalog(&self, change: &CatalogChange) -> Result<()> {
        *lock(&self.catalog) = Some(stored(change));
        Ok(())
    }

    async fn limits(&self) -> Result<LimitsChange> {
        Ok(*lock(&self.limits))
    }

    async fn set_limits(&self, limits: &LimitsChange) -> Result<()> {
        *lock(&self.limits) = *limits;
        Ok(())
    }

    async fn registries(&self) -> Result<Vec<Registry>> {
        Ok(lock(&self.registries).clone())
    }

    async fn set_registries(&self, registries: &[Registry]) -> Result<()> {
        *lock(&self.registries) = registries
            .iter()
            .map(|row| Registry {
                source: Source::Database,
                ..row.clone()
            })
            .collect();
        Ok(())
    }

    async fn allowance(&self, user: UserId) -> Result<Option<Allowance>> {
        Ok(lock(&self.allowances).get(&user).copied())
    }

    async fn set_allowance(&self, user: UserId, allowance: Option<&Allowance>) -> Result<()> {
        let mut allowances = lock(&self.allowances);
        match allowance {
            Some(allowance) => allowances.insert(user, *allowance),
            None => allowances.remove(&user),
        };
        Ok(())
    }
}
