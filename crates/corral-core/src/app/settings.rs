use std::fmt::Display;
use std::sync::Arc;

use async_trait::async_trait;

use super::{Catalog, CatalogDefaults, ImageOption, NamedImages, SizePreset, Source};
use crate::domain::{Admin, ImageKey, Sandbox, SandboxImage, SizeKey, User, UserId};
use crate::ports::{RunningLimit, SandboxRepo, SettingsRepo, UserRepo};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_sandboxes_per_member: usize,
    pub max_running: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_sandboxes_per_member: 5,
            max_running: 6,
        }
    }
}

/// A registry Corral pulls sandbox images from or reads their sizes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry {
    /// The registry's host, with its port if it has one, such as `registry.example.lan:5000`.
    pub registry: String,
    pub username: Option<String>,
    /// The environment variable holding the password or token, which settings never hold.
    pub password_env: Option<String>,
    /// Whether Corral reads image sizes from it over plain HTTP.
    pub plain_http: bool,
    pub source: Source,
}

impl Registry {
    fn check(&self) -> Result<()> {
        if self.registry.is_empty() {
            return Err(Error::Invalid("a registry needs a host".to_owned()));
        }
        if self.username.is_some() != self.password_env.is_some() {
            return Err(Error::Invalid(format!(
                "registry \"{}\" needs both a username and a password variable, or neither",
                self.registry
            )));
        }
        Ok(())
    }
}

/// How many sandboxes one member may have, in place of the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowance {
    pub max_sandboxes: usize,
}

/// What the config file sets, which settings show locked.
#[derive(Clone, Debug, Default)]
pub struct Configured {
    pub images: Vec<ImageOption>,
    pub sizes: Vec<SizePreset>,
    pub defaults: Option<CatalogDefaults>,
    pub named_images: Option<NamedImages>,
    pub max_sandboxes_per_member: Option<usize>,
    pub max_running: Option<usize>,
    pub registries: Vec<Registry>,
}

/// The database part of the catalog, as the admin saves it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogChange {
    pub images: Vec<ImageOption>,
    pub sizes: Vec<SizePreset>,
    pub defaults: Option<CatalogDefaults>,
    pub named_images: Option<NamedImages>,
}

/// A value and who owns it. The admin changes only what the database owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locked<T> {
    pub value: T,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogView {
    pub catalog: Catalog,
    pub defaults: Locked<CatalogDefaults>,
    /// The stored defaults name an image or a size the catalog does not offer, and its first
    /// stands in.
    pub defaults_fell_back: bool,
    pub named_images: Locked<Option<NamedImages>>,
    /// Stored entries whose keys the config also sets, so the catalog resolves the config's.
    pub shadowed_images: Vec<ImageKey>,
    pub shadowed_sizes: Vec<SizeKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitsView {
    pub max_sandboxes_per_member: Locked<usize>,
    pub max_running: Locked<usize>,
}

/// The database part of the limits. `None` leaves the built-in default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LimitsChange {
    pub max_sandboxes_per_member: Option<usize>,
    pub max_running: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistriesView {
    pub registries: Vec<Registry>,
    /// Hosts of stored rows the config also sets.
    pub shadowed: Vec<String>,
}

/// What the service offers: what the config file sets, locked, merged with what the admin keeps
/// in the database, which is read on every use so a change applies at once.
#[derive(Clone)]
pub struct Settings {
    repo: Arc<dyn SettingsRepo>,
    sandboxes: Arc<dyn SandboxRepo>,
    users: Arc<dyn UserRepo>,
    configured: Arc<Configured>,
}

impl Settings {
    #[must_use]
    pub fn new(
        repo: Arc<dyn SettingsRepo>,
        sandboxes: Arc<dyn SandboxRepo>,
        users: Arc<dyn UserRepo>,
        configured: Configured,
    ) -> Self {
        Self {
            repo,
            sandboxes,
            users,
            configured: Arc::new(configured),
        }
    }

    pub async fn catalog(&self) -> Result<Catalog> {
        Ok(self.catalog_view().await?.catalog)
    }

    pub async fn catalog_view(&self) -> Result<CatalogView> {
        CatalogMerge::of(&self.configured, self.repo.catalog().await?)
            .view()
            .map_err(|error| Error::internal("merged catalog", error))
    }

    /// Refuses what the config sets, and removing an entry a sandbox was made from.
    pub async fn set_catalog(&self, _: &Admin, change: CatalogChange) -> Result<()> {
        let configured = &self.configured;
        refuse_configured(&configured.images, &change.images)?;
        refuse_configured(&configured.sizes, &change.sizes)?;
        if configured.defaults.is_some() && change.defaults.is_some() {
            return Err(Error::Invalid("the defaults are set in config".to_owned()));
        }
        if configured.named_images.is_some() && change.named_images.is_some() {
            return Err(Error::Invalid("named images are set in config".to_owned()));
        }
        let before = CatalogMerge::of(configured, self.repo.catalog().await?);
        let after = CatalogMerge::of(configured, Some(change.clone()));
        self.refuse_removing_used(&before, &after).await?;
        let defaults = change
            .defaults
            .clone()
            .unwrap_or_else(|| after.defaults.value.clone());
        Catalog::new(after.images, after.sizes, defaults)?;
        self.repo.set_catalog(&change).await
    }

    pub async fn limits(&self) -> Result<Limits> {
        let view = self.limits_view().await?;
        Ok(Limits {
            max_sandboxes_per_member: view.max_sandboxes_per_member.value,
            max_running: view.max_running.value,
        })
    }

    pub async fn limits_view(&self) -> Result<LimitsView> {
        let stored = self.repo.limits().await?;
        let builtin = Limits::default();
        Ok(LimitsView {
            max_sandboxes_per_member: locked(
                self.configured.max_sandboxes_per_member,
                stored.max_sandboxes_per_member,
                builtin.max_sandboxes_per_member,
            ),
            max_running: locked(
                self.configured.max_running,
                stored.max_running,
                builtin.max_running,
            ),
        })
    }

    pub async fn set_limits(&self, _: &Admin, change: LimitsChange) -> Result<()> {
        for (name, configured, changed) in [
            (
                "max_sandboxes_per_member",
                self.configured.max_sandboxes_per_member,
                change.max_sandboxes_per_member,
            ),
            (
                "max_running",
                self.configured.max_running,
                change.max_running,
            ),
        ] {
            if changed == Some(0) {
                return Err(Error::Invalid("limits must be at least 1".to_owned()));
            }
            if configured.is_some() && changed.is_some() {
                return Err(Error::Invalid(format!("{name} is set in config")));
            }
        }
        self.repo.set_limits(&change).await
    }

    pub async fn registries(&self) -> Result<Vec<Registry>> {
        Ok(self.registries_view().await?.registries)
    }

    pub async fn registries_view(&self) -> Result<RegistriesView> {
        let configured = &self.configured.registries;
        let (shadowed, stored): (Vec<_>, Vec<_>) = self
            .repo
            .registries()
            .await?
            .into_iter()
            .partition(|row| lists(configured, &row.registry));
        Ok(RegistriesView {
            registries: configured
                .iter()
                .cloned()
                .map(|row| owned_by(row, Source::Config))
                .chain(
                    stored
                        .into_iter()
                        .map(|row| owned_by(row, Source::Database)),
                )
                .collect(),
            shadowed: shadowed.into_iter().map(|row| row.registry).collect(),
        })
    }

    /// Refuses a host the config sets, or one listed twice, in any case.
    pub async fn set_registries(&self, _: &Admin, registries: Vec<Registry>) -> Result<()> {
        for (index, row) in registries.iter().enumerate() {
            row.check()?;
            if lists(&self.configured.registries, &row.registry) {
                return Err(Error::Invalid(format!(
                    "registry \"{}\" is set in config",
                    row.registry
                )));
            }
            if lists(&registries[..index], &row.registry) {
                return Err(Error::Invalid(format!(
                    "registry \"{}\" is listed twice",
                    row.registry
                )));
            }
        }
        self.repo.set_registries(&registries).await
    }

    pub async fn allowance(&self, user: UserId) -> Result<Option<Allowance>> {
        self.repo.allowance(user).await
    }

    /// The user it is set for.
    pub async fn set_allowance(
        &self,
        _: &Admin,
        user: UserId,
        allowance: Option<Allowance>,
    ) -> Result<User> {
        if allowance.is_some_and(|allowance| allowance.max_sandboxes == 0) {
            return Err(Error::Invalid("allowances must be at least 1".to_owned()));
        }
        let user = self
            .users
            .get(user)
            .await?
            .ok_or_else(|| Error::NotFound(format!("user {}", user.0)))?;
        self.repo.set_allowance(user.id, allowance.as_ref()).await?;
        Ok(user)
    }

    pub async fn max_sandboxes_for(&self, user: UserId) -> Result<usize> {
        match self.repo.allowance(user).await? {
            Some(allowance) => Ok(allowance.max_sandboxes),
            None => Ok(self.limits().await?.max_sandboxes_per_member),
        }
    }

    async fn refuse_removing_used(
        &self,
        before: &CatalogMerge,
        after: &CatalogMerge,
    ) -> Result<()> {
        let sandboxes = self.sandboxes.list_all().await?;
        let Some((entry, used)) = removed_in_use(&before.images, &after.images, &sandboxes)
            .or_else(|| removed_in_use(&before.sizes, &after.sizes, &sandboxes))
        else {
            return Ok(());
        };
        let mut names = Vec::with_capacity(used.len());
        for sandbox in used {
            let owner = self.users.get(sandbox.owner).await?.ok_or_else(|| {
                Error::internal("store", format!("sandbox {} has no owner", sandbox.id.0))
            })?;
            names.push(format!("{}.{}", sandbox.name, owner.handle));
        }
        Err(Error::Invalid(format!(
            "{entry} is used by {}; disable it instead of removing it",
            names.join(", ")
        )))
    }
}

#[async_trait]
impl RunningLimit for Settings {
    async fn max_running(&self) -> Result<usize> {
        Ok(self.limits().await?.max_running)
    }
}

/// The catalog the config and the database make together, before `Catalog::new` checks it.
struct CatalogMerge {
    images: Vec<ImageOption>,
    sizes: Vec<SizePreset>,
    defaults: Locked<CatalogDefaults>,
    defaults_fell_back: bool,
    named_images: Locked<Option<NamedImages>>,
    shadowed_images: Vec<ImageKey>,
    shadowed_sizes: Vec<SizeKey>,
}

impl CatalogMerge {
    /// Configured entries first, then stored ones whose keys the config does not set, and the
    /// built-in entries of a kind neither has. The defaults and named images are the config's
    /// where it sets them.
    fn of(configured: &Configured, stored: Option<CatalogChange>) -> Self {
        let builtin = Catalog::builtin();
        let stored = stored.unwrap_or_default();
        let (images, shadowed_images) =
            merge_entries(&configured.images, stored.images, builtin.images());
        let (sizes, shadowed_sizes) =
            merge_entries(&configured.sizes, stored.sizes, builtin.sizes());
        let (defaults, defaults_fell_back) = if let Some(defaults) = &configured.defaults {
            (
                Locked {
                    value: defaults.clone(),
                    source: Source::Config,
                },
                false,
            )
        } else {
            let (stored_image, stored_size) = stored
                .defaults
                .map(|defaults| (defaults.image, defaults.size))
                .unzip();
            let (image, image_fell_back) =
                default_key(&images, stored_image, &builtin.defaults().image);
            let (size, size_fell_back) = default_key(&sizes, stored_size, &builtin.defaults().size);
            (
                Locked {
                    value: CatalogDefaults { image, size },
                    source: Source::Database,
                },
                image_fell_back || size_fell_back,
            )
        };
        let named_images = match &configured.named_images {
            Some(named) => Locked {
                value: Some(named.clone()),
                source: Source::Config,
            },
            None => Locked {
                value: stored.named_images,
                source: Source::Database,
            },
        };
        Self {
            images,
            sizes,
            defaults,
            defaults_fell_back,
            named_images,
            shadowed_images,
            shadowed_sizes,
        }
    }

    fn view(self) -> Result<CatalogView> {
        let catalog = Catalog::new(self.images, self.sizes, self.defaults.value.clone())?;
        let catalog = match &self.named_images.value {
            Some(named) => catalog.with_named_images(named.clone()),
            None => catalog,
        };
        Ok(CatalogView {
            catalog,
            defaults: self.defaults,
            defaults_fell_back: self.defaults_fell_back,
            named_images: self.named_images,
            shadowed_images: self.shadowed_images,
            shadowed_sizes: self.shadowed_sizes,
        })
    }
}

trait CatalogEntry: Clone {
    type Key: Clone + PartialEq + Display;
    const KIND: &'static str;
    fn key(&self) -> &Self::Key;
    fn enabled(&self) -> bool;
    fn owned_by(self, source: Source) -> Self;
    fn made(&self, sandbox: &Sandbox) -> bool;
}

impl CatalogEntry for ImageOption {
    type Key = ImageKey;
    const KIND: &'static str = "image";

    fn key(&self) -> &ImageKey {
        &self.key
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn owned_by(self, source: Source) -> Self {
        Self { source, ..self }
    }

    fn made(&self, sandbox: &Sandbox) -> bool {
        matches!(&sandbox.image, SandboxImage::Catalog(key) if *key == self.key)
    }
}

impl CatalogEntry for SizePreset {
    type Key = SizeKey;
    const KIND: &'static str = "size";

    fn key(&self) -> &SizeKey {
        &self.key
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn owned_by(self, source: Source) -> Self {
        Self { source, ..self }
    }

    fn made(&self, sandbox: &Sandbox) -> bool {
        sandbox.size == self.key
    }
}

fn has<T: CatalogEntry>(entries: &[T], key: &T::Key) -> bool {
    entries.iter().any(|entry| entry.key() == key)
}

/// The merged entries, and the keys of stored entries the config shadows.
fn merge_entries<T: CatalogEntry>(
    configured: &[T],
    stored: Vec<T>,
    builtin: &[T],
) -> (Vec<T>, Vec<T::Key>) {
    let (shadowed, stored): (Vec<T>, Vec<T>) = stored
        .into_iter()
        .partition(|entry| has(configured, entry.key()));
    let mut merged: Vec<T> = configured
        .iter()
        .cloned()
        .map(|entry| entry.owned_by(Source::Config))
        .chain(
            stored
                .into_iter()
                .map(|entry| entry.owned_by(Source::Database)),
        )
        .collect();
    if merged.is_empty() {
        merged = builtin.to_vec();
    }
    let shadowed = shadowed.iter().map(|entry| entry.key().clone()).collect();
    (merged, shadowed)
}

/// The stored default where the entries offer it, else the first they offer, and whether it fell
/// back. Entries offering nothing fail `Catalog::new` whatever the default, so `otherwise` only
/// fills the place.
fn default_key<T: CatalogEntry>(
    entries: &[T],
    stored: Option<T::Key>,
    otherwise: &T::Key,
) -> (T::Key, bool) {
    let offered = |key: &T::Key| {
        entries
            .iter()
            .any(|entry| entry.enabled() && entry.key() == key)
    };
    match stored {
        Some(key) if offered(&key) => (key, false),
        stored => {
            let first = entries
                .iter()
                .find(|entry| entry.enabled())
                .map_or(otherwise, CatalogEntry::key);
            (first.clone(), stored.is_some())
        }
    }
}

fn refuse_configured<T: CatalogEntry>(configured: &[T], change: &[T]) -> Result<()> {
    match change.iter().find(|entry| has(configured, entry.key())) {
        Some(entry) => Err(Error::Invalid(format!(
            "{} \"{}\" is set in config",
            T::KIND,
            entry.key()
        ))),
        None => Ok(()),
    }
}

/// The first entry the change removes that a sandbox was made from, as `image "key"`, with those
/// sandboxes.
fn removed_in_use<'a, T: CatalogEntry>(
    before: &[T],
    after: &[T],
    sandboxes: &'a [Sandbox],
) -> Option<(String, Vec<&'a Sandbox>)> {
    before
        .iter()
        .filter(|entry| !has(after, entry.key()))
        .map(|entry| {
            let used: Vec<_> = sandboxes
                .iter()
                .filter(|sandbox| entry.made(sandbox))
                .collect();
            (format!("{} \"{}\"", T::KIND, entry.key()), used)
        })
        .find(|(_, used)| !used.is_empty())
}

fn locked<T>(configured: Option<T>, stored: Option<T>, builtin: T) -> Locked<T> {
    match configured {
        Some(value) => Locked {
            value,
            source: Source::Config,
        },
        None => Locked {
            value: stored.unwrap_or(builtin),
            source: Source::Database,
        },
    }
}

fn lists(registries: &[Registry], host: &str) -> bool {
    registries
        .iter()
        .any(|row| row.registry.eq_ignore_ascii_case(host))
}

fn owned_by(row: Registry, source: Source) -> Registry {
    Registry { source, ..row }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::sync::Arc;

    use uuid::Uuid;

    use super::{
        Allowance, CatalogChange, Configured, Limits, LimitsChange, Locked, Registry, Settings,
    };
    use crate::app::{Catalog, CatalogDefaults, ImageOption, Source};
    use crate::domain::{ImageKey, SizeKey, UserId};
    use crate::fakes::{
        MAX_SANDBOXES, MemorySettingsRepo, World, admin_proof, catalog, catalog_change,
        image_option,
    };
    use crate::{Error, Result};

    fn image(key: &str) -> ImageKey {
        ImageKey::parse(key).unwrap()
    }

    fn size(key: &str) -> SizeKey {
        SizeKey::parse(key).unwrap()
    }

    fn invalid<T: Debug>(result: Result<T>) -> String {
        match result {
            Err(Error::Invalid(message)) => message,
            other => panic!("{other:?}"),
        }
    }

    fn over(w: &World, configured: Configured) -> Settings {
        Settings::new(
            w.settings_repo.clone(),
            w.sandbox_repo.clone(),
            w.users.clone(),
            configured,
        )
    }

    fn registry(host: &str) -> Registry {
        Registry {
            registry: host.to_owned(),
            username: None,
            password_env: None,
            plain_http: false,
            source: Source::Database,
        }
    }

    fn disabled(image: ImageOption) -> ImageOption {
        ImageOption {
            enabled: false,
            ..image
        }
    }

    #[tokio::test]
    async fn nothing_configured_or_stored_is_the_builtin_catalog_and_default_limits() {
        let w = World::new();
        let settings = Settings::new(
            Arc::new(MemorySettingsRepo::default()),
            w.sandbox_repo.clone(),
            w.users.clone(),
            Configured::default(),
        );
        assert_eq!(settings.catalog().await.unwrap(), Catalog::builtin());
        assert_eq!(
            settings.limits().await.unwrap(),
            Limits {
                max_sandboxes_per_member: 5,
                max_running: 6,
            }
        );
    }

    #[tokio::test]
    async fn config_entries_come_first_and_are_locked() {
        let w = World::with_configured(Configured {
            images: vec![image_option("ubuntu", "docker.io/library/ubuntu:24.04")],
            ..Configured::default()
        });
        let root = admin_proof(&w.admin("root").await);
        let storing = |images| CatalogChange {
            images,
            sizes: catalog().sizes().to_vec(),
            defaults: None,
            named_images: None,
        };
        assert_eq!(
            invalid(
                w.settings
                    .set_catalog(&root, storing(vec![image_option("ubuntu", "ubuntu:25.04")]))
                    .await
            ),
            "image \"ubuntu\" is set in config"
        );
        w.settings
            .set_catalog(
                &root,
                storing(vec![image_option("debian", "docker.io/library/debian:13")]),
            )
            .await
            .unwrap();
        let merged = w.settings.catalog().await.unwrap();
        assert_eq!(
            merged
                .images()
                .iter()
                .map(|image| (image.key.as_str(), image.source))
                .collect::<Vec<_>>(),
            [("ubuntu", Source::Config), ("debian", Source::Database)]
        );
    }

    #[tokio::test]
    async fn a_config_entry_shadows_a_stored_entry_with_its_key() {
        let w = World::new();
        let root = admin_proof(&w.admin("root").await);
        let mut change = catalog_change(&catalog());
        change
            .images
            .push(image_option("debian", "registry.lan/debian:1"));
        w.settings.set_catalog(&root, change).await.unwrap();
        assert_eq!(w.settings.catalog_view().await.unwrap().shadowed_images, []);

        let configured = over(
            &w,
            Configured {
                images: vec![image_option("debian", "registry.lan/debian:2")],
                ..Configured::default()
            },
        );
        assert_eq!(
            configured
                .catalog()
                .await
                .unwrap()
                .image(Some(&image("debian")))
                .unwrap()
                .reference,
            "registry.lan/debian:2"
        );
        assert_eq!(
            configured.catalog_view().await.unwrap().shadowed_images,
            [image("debian")]
        );
    }

    #[tokio::test]
    async fn a_stored_default_that_names_a_vanished_key_falls_back() {
        let w = World::new();
        let root = admin_proof(&w.admin("root").await);
        let mut change = catalog_change(&catalog());
        change
            .images
            .push(image_option("debian", "registry.lan/debian:1"));
        change.defaults = Some(CatalogDefaults {
            image: image("debian"),
            size: size("small"),
        });
        w.settings.set_catalog(&root, change).await.unwrap();
        let stored = w.settings.catalog_view().await.unwrap();
        assert_eq!(stored.catalog.image(None).unwrap().key, image("debian"));
        assert!(!stored.defaults_fell_back);

        let configured = over(
            &w,
            Configured {
                images: vec![disabled(image_option("debian", "registry.lan/debian:2"))],
                ..Configured::default()
            },
        );
        assert_eq!(
            configured.catalog().await.unwrap().image(None).unwrap().key,
            image("base")
        );
        let view = configured.catalog_view().await.unwrap();
        assert!(view.defaults_fell_back);
        assert_eq!(
            view.defaults,
            Locked {
                value: CatalogDefaults {
                    image: image("base"),
                    size: size("small"),
                },
                source: Source::Database,
            }
        );

        let mut naming_disabled = catalog_change(&catalog());
        naming_disabled.defaults = Some(CatalogDefaults {
            image: image("tools"),
            size: size("small"),
        });
        assert_eq!(
            invalid(w.settings.set_catalog(&root, naming_disabled).await),
            "the default image \"tools\" is not an enabled image"
        );
    }

    #[tokio::test]
    async fn a_merge_with_nothing_enabled_is_an_internal_error() {
        let w = World::new();
        let settings = Settings::new(
            Arc::new(MemorySettingsRepo::default()),
            w.sandbox_repo.clone(),
            w.users.clone(),
            Configured {
                images: vec![disabled(image_option("ubuntu", "ubuntu:24.04"))],
                ..Configured::default()
            },
        );
        assert!(matches!(
            settings.catalog().await,
            Err(Error::Internal { .. })
        ));
        assert!(matches!(
            settings.catalog_view().await,
            Err(Error::Internal { .. })
        ));
    }

    #[tokio::test]
    async fn set_catalog_refuses_removing_what_a_sandbox_uses() {
        let w = World::new();
        let root = admin_proof(&w.admin("root").await);
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        w.sandbox(&adam, "main").await;
        w.sandbox(&eve, "docs").await;
        let fakes = catalog();
        let base = fakes.image(Some(&image("base"))).unwrap().clone();
        let tools = ImageOption {
            enabled: true,
            ..fakes.image(Some(&image("tools"))).unwrap().clone()
        };
        let storing = |images| CatalogChange {
            images,
            sizes: fakes.sizes().to_vec(),
            defaults: Some(CatalogDefaults {
                image: image("tools"),
                size: size("small"),
            }),
            named_images: None,
        };
        assert_eq!(
            invalid(
                w.settings
                    .set_catalog(&root, storing(vec![tools.clone()]))
                    .await
            ),
            "image \"base\" is used by main.adam, docs.eve; disable it instead of removing it"
        );
        w.settings
            .set_catalog(&root, storing(vec![disabled(base), tools]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn limits_lock_per_field() {
        let w = World::with_configured(Configured {
            max_running: Some(3),
            ..Configured::default()
        });
        let root = admin_proof(&w.admin("root").await);
        assert_eq!(
            w.settings.limits_view().await.unwrap().max_running,
            Locked {
                value: 3,
                source: Source::Config,
            }
        );
        assert_eq!(
            invalid(
                w.settings
                    .set_limits(
                        &root,
                        LimitsChange {
                            max_sandboxes_per_member: None,
                            max_running: Some(9),
                        },
                    )
                    .await
            ),
            "max_running is set in config"
        );
        assert_eq!(
            invalid(
                w.settings
                    .set_limits(
                        &root,
                        LimitsChange {
                            max_sandboxes_per_member: Some(0),
                            max_running: None,
                        },
                    )
                    .await
            ),
            "limits must be at least 1"
        );
        w.settings
            .set_limits(
                &root,
                LimitsChange {
                    max_sandboxes_per_member: Some(2),
                    max_running: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            w.settings.limits().await.unwrap(),
            Limits {
                max_sandboxes_per_member: 2,
                max_running: 3,
            }
        );
        assert_eq!(
            w.settings
                .limits_view()
                .await
                .unwrap()
                .max_sandboxes_per_member,
            Locked {
                value: 2,
                source: Source::Database,
            }
        );
        w.settings
            .set_limits(&root, LimitsChange::default())
            .await
            .unwrap();
        assert_eq!(
            w.settings.limits().await.unwrap().max_sandboxes_per_member,
            5
        );
    }

    #[tokio::test]
    async fn an_allowance_overrides_the_limit_for_one_member() {
        let w = World::new();
        let root = admin_proof(&w.admin("root").await);
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let seven = Allowance { max_sandboxes: 7 };
        let set_for = w.settings.set_allowance(&root, adam.id, Some(seven)).await;
        assert_eq!(set_for.unwrap(), adam);
        assert_eq!(w.settings.allowance(adam.id).await.unwrap(), Some(seven));
        assert_eq!(w.settings.max_sandboxes_for(adam.id).await.unwrap(), 7);
        assert_eq!(
            w.settings.max_sandboxes_for(eve.id).await.unwrap(),
            MAX_SANDBOXES
        );
        w.settings
            .set_allowance(&root, adam.id, None)
            .await
            .unwrap();
        assert_eq!(
            w.settings.max_sandboxes_for(adam.id).await.unwrap(),
            MAX_SANDBOXES
        );
        assert!(matches!(
            w.settings
                .set_allowance(&root, UserId(Uuid::new_v4()), Some(seven))
                .await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(
            invalid(
                w.settings
                    .set_allowance(&root, adam.id, Some(Allowance { max_sandboxes: 0 }))
                    .await
            ),
            "allowances must be at least 1"
        );
    }

    #[tokio::test]
    async fn registries_merge_by_host_and_config_wins() {
        let w = World::with_configured(Configured {
            registries: vec![registry("ghcr.io")],
            ..Configured::default()
        });
        let root = admin_proof(&w.admin("root").await);
        assert_eq!(
            invalid(
                w.settings
                    .set_registries(&root, vec![registry("GHCR.io")])
                    .await
            ),
            "registry \"GHCR.io\" is set in config"
        );
        assert_eq!(
            invalid(
                w.settings
                    .set_registries(
                        &root,
                        vec![registry("registry.lan:5000"), registry("REGISTRY.lan:5000")],
                    )
                    .await
            ),
            "registry \"REGISTRY.lan:5000\" is listed twice"
        );
        w.settings
            .set_registries(&root, vec![registry("registry.lan:5000")])
            .await
            .unwrap();
        assert_eq!(
            w.settings
                .registries()
                .await
                .unwrap()
                .iter()
                .map(|row| (row.registry.as_str(), row.source))
                .collect::<Vec<_>>(),
            [
                ("ghcr.io", Source::Config),
                ("registry.lan:5000", Source::Database)
            ]
        );

        let later = over(
            &w,
            Configured {
                registries: vec![registry("Registry.lan:5000")],
                ..Configured::default()
            },
        );
        let view = later.registries_view().await.unwrap();
        assert_eq!(view.shadowed, ["registry.lan:5000"]);
        assert_eq!(
            view.registries
                .iter()
                .map(|row| (row.registry.as_str(), row.source))
                .collect::<Vec<_>>(),
            [("Registry.lan:5000", Source::Config)]
        );
    }
}
