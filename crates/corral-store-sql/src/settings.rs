use std::num::NonZeroU64;

use async_trait::async_trait;
use chrono::Utc;
use corral_core::app::{
    Allowance, CatalogChange, CatalogDefaults, ImageOption, LimitsChange, NamedImages, Registry,
    SizePreset, Source,
};
use corral_core::domain::{Allocation, ImageKey, Resources, SizeKey, UserId};
use corral_core::ports::SettingsRepo;
use corral_core::{Error, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Store;
use corral_sql::{Pool, db_error, on_pool, parsed};

#[derive(Serialize, Deserialize)]
struct CatalogDoc {
    images: Vec<ImageDoc>,
    sizes: Vec<SizeDoc>,
    defaults: Option<DefaultsDoc>,
    named_images: Option<NamedDoc>,
}

#[derive(Serialize, Deserialize)]
struct ImageDoc {
    key: ImageKey,
    label: String,
    reference: String,
    description: String,
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
struct SizeDoc {
    key: SizeKey,
    label: String,
    cpu_request_millis: u64,
    cpu_limit_millis: u64,
    memory_request_bytes: u64,
    memory_limit_bytes: u64,
    home_bytes: NonZeroU64,
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
struct DefaultsDoc {
    image: ImageKey,
    size: SizeKey,
}

#[derive(Serialize, Deserialize)]
struct NamedDoc {
    registries: Vec<String>,
    max_bytes: NonZeroU64,
}

#[derive(Serialize, Deserialize)]
struct LimitsDoc {
    max_sandboxes_per_member: Option<u64>,
    max_running: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct RegistryDoc {
    registry: String,
    username: Option<String>,
    password_env: Option<String>,
    plain_http: bool,
}

impl From<&CatalogChange> for CatalogDoc {
    fn from(change: &CatalogChange) -> Self {
        Self {
            images: change.images.iter().map(ImageDoc::from).collect(),
            sizes: change.sizes.iter().map(SizeDoc::from).collect(),
            defaults: change.defaults.as_ref().map(|defaults| DefaultsDoc {
                image: defaults.image.clone(),
                size: defaults.size.clone(),
            }),
            named_images: change.named_images.as_ref().map(|named| NamedDoc {
                registries: named.registries.clone(),
                max_bytes: named.max_bytes,
            }),
        }
    }
}

impl TryFrom<CatalogDoc> for CatalogChange {
    type Error = Error;

    fn try_from(doc: CatalogDoc) -> Result<Self> {
        Ok(Self {
            images: doc.images.into_iter().map(ImageOption::from).collect(),
            sizes: doc
                .sizes
                .into_iter()
                .map(SizePreset::try_from)
                .collect::<Result<_>>()?,
            defaults: doc.defaults.map(|defaults| CatalogDefaults {
                image: defaults.image,
                size: defaults.size,
            }),
            named_images: doc.named_images.map(|named| NamedImages {
                registries: named.registries,
                max_bytes: named.max_bytes,
            }),
        })
    }
}

impl From<&ImageOption> for ImageDoc {
    fn from(image: &ImageOption) -> Self {
        Self {
            key: image.key.clone(),
            label: image.label.clone(),
            reference: image.reference.clone(),
            description: image.description.clone(),
            enabled: image.enabled,
        }
    }
}

impl From<ImageDoc> for ImageOption {
    fn from(doc: ImageDoc) -> Self {
        Self {
            key: doc.key,
            label: doc.label,
            reference: doc.reference,
            description: doc.description,
            enabled: doc.enabled,
            source: Source::Database,
        }
    }
}

impl From<&SizePreset> for SizeDoc {
    fn from(size: &SizePreset) -> Self {
        let Resources {
            cpu_millis,
            memory_bytes,
            home_bytes,
        } = size.resources;
        Self {
            key: size.key.clone(),
            label: size.label.clone(),
            cpu_request_millis: cpu_millis.request(),
            cpu_limit_millis: cpu_millis.limit(),
            memory_request_bytes: memory_bytes.request(),
            memory_limit_bytes: memory_bytes.limit(),
            home_bytes,
            enabled: size.enabled,
        }
    }
}

impl TryFrom<SizeDoc> for SizePreset {
    type Error = Error;

    fn try_from(doc: SizeDoc) -> Result<Self> {
        Ok(Self {
            key: doc.key,
            label: doc.label,
            resources: Resources {
                cpu_millis: parsed(
                    "catalog",
                    Allocation::new(doc.cpu_request_millis, doc.cpu_limit_millis),
                )?,
                memory_bytes: parsed(
                    "catalog",
                    Allocation::new(doc.memory_request_bytes, doc.memory_limit_bytes),
                )?,
                home_bytes: doc.home_bytes,
            },
            enabled: doc.enabled,
            source: Source::Database,
        })
    }
}

impl From<&LimitsChange> for LimitsDoc {
    fn from(limits: &LimitsChange) -> Self {
        let widen = |count: usize| u64::try_from(count).unwrap_or(u64::MAX);
        Self {
            max_sandboxes_per_member: limits.max_sandboxes_per_member.map(widen),
            max_running: limits.max_running.map(widen),
        }
    }
}

impl TryFrom<LimitsDoc> for LimitsChange {
    type Error = Error;

    fn try_from(doc: LimitsDoc) -> Result<Self> {
        let narrow = |count: u64| parsed("limits", usize::try_from(count));
        Ok(Self {
            max_sandboxes_per_member: doc.max_sandboxes_per_member.map(narrow).transpose()?,
            max_running: doc.max_running.map(narrow).transpose()?,
        })
    }
}

impl From<&Registry> for RegistryDoc {
    fn from(row: &Registry) -> Self {
        Self {
            registry: row.registry.clone(),
            username: row.username.clone(),
            password_env: row.password_env.clone(),
            plain_http: row.plain_http,
        }
    }
}

impl From<RegistryDoc> for Registry {
    fn from(doc: RegistryDoc) -> Self {
        Self {
            registry: doc.registry,
            username: doc.username,
            password_env: doc.password_env,
            plain_http: doc.plain_http,
            source: Source::Database,
        }
    }
}

async fn document<T: DeserializeOwned>(pool: &Pool, key: &str) -> Result<Option<T>> {
    let value: Option<String> = on_pool!(pool, |pool| {
        sqlx::query_scalar("select value from settings where key = $1")
            .bind(key)
            .fetch_optional(pool)
            .await
    })
    .map_err(db_error)?;
    value
        .map(|value| parsed(key, serde_json::from_str(&value)))
        .transpose()
}

async fn set_document(pool: &Pool, key: &str, document: &(impl Serialize + Sync)) -> Result<()> {
    let value = serde_json::to_string(document)
        .map_err(|err| Error::internal(format!("storing {key}"), err))?;
    on_pool!(pool, |pool| {
        sqlx::query(
            "insert into settings (key, value, updated_at) values ($1, $2, $3) \
             on conflict (key) do update set value = excluded.value, \
             updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(&value)
        .bind(Utc::now())
        .execute(pool)
        .await
        .map(drop)
    })
    .map_err(db_error)
}

#[async_trait]
impl SettingsRepo for Store {
    async fn catalog(&self) -> Result<Option<CatalogChange>> {
        document::<CatalogDoc>(&self.pool, "catalog")
            .await?
            .map(CatalogChange::try_from)
            .transpose()
    }

    async fn set_catalog(&self, change: &CatalogChange) -> Result<()> {
        set_document(&self.pool, "catalog", &CatalogDoc::from(change)).await
    }

    async fn limits(&self) -> Result<LimitsChange> {
        document::<LimitsDoc>(&self.pool, "limits")
            .await?
            .map_or(Ok(LimitsChange::default()), LimitsChange::try_from)
    }

    async fn set_limits(&self, limits: &LimitsChange) -> Result<()> {
        set_document(&self.pool, "limits", &LimitsDoc::from(limits)).await
    }

    async fn registries(&self) -> Result<Vec<Registry>> {
        let rows: Vec<RegistryDoc> = document(&self.pool, "registries")
            .await?
            .unwrap_or_default();
        Ok(rows.into_iter().map(Registry::from).collect())
    }

    async fn set_registries(&self, registries: &[Registry]) -> Result<()> {
        let rows: Vec<RegistryDoc> = registries.iter().map(RegistryDoc::from).collect();
        set_document(&self.pool, "registries", &rows).await
    }

    async fn allowance(&self, user: UserId) -> Result<Option<Allowance>> {
        let max_sandboxes: Option<i64> = on_pool!(self.pool, |pool| {
            sqlx::query_scalar("select max_sandboxes from member_allowances where user_id = $1")
                .bind(user.0)
                .fetch_optional(pool)
                .await
        })
        .map_err(db_error)?;
        let Some(max_sandboxes) = max_sandboxes else {
            return Ok(None);
        };
        Ok(Some(Allowance {
            max_sandboxes: parsed("max_sandboxes", usize::try_from(max_sandboxes))?,
        }))
    }

    async fn set_allowance(&self, user: UserId, allowance: Option<&Allowance>) -> Result<()> {
        match allowance {
            Some(allowance) => {
                let max_sandboxes = i64::try_from(allowance.max_sandboxes).unwrap_or(i64::MAX);
                on_pool!(self.pool, |pool| {
                    sqlx::query(
                        "insert into member_allowances (user_id, max_sandboxes) \
                         select id, $2 from users where id = $1 \
                         on conflict (user_id) do update set max_sandboxes = excluded.max_sandboxes",
                    )
                    .bind(user.0)
                    .bind(max_sandboxes)
                    .execute(pool)
                    .await
                    .map(drop)
                })
            }
            None => on_pool!(self.pool, |pool| {
                sqlx::query("delete from member_allowances where user_id = $1")
                    .bind(user.0)
                    .execute(pool)
                    .await
                    .map(drop)
            }),
        }
        .map_err(db_error)
    }
}
